//! Standing vetoes: an entity is *not* an identity.
//!
//! A veto is keyed by the subject's `origin_observation_id` plus the identity, so it outlives the
//! rows a rebuild renumbers. `vetoed` answers admission before a claim is written, and `withdraw`
//! retracts the one live claim for the pairing — whatever route produced it. The merge side of the
//! same table is written by `merge` itself, where the fold happens.
//!
//! `entity_with_origin` and `canonical` are the readers this module and `replay` share to turn an
//! origin into the entity it names now.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::model;

use super::current_generation;
use super::merge::Cause;
use super::KnowledgeGraph;

/// Which statement a verdict row makes lives with the column it fills; this re-export is how this
/// module names it.
pub(crate) use crate::model::knowledge_verdicts::Kind;

/// The unordered pair two origins name, as a key.
pub(crate) fn pair_key(a: i32, b: i32) -> (i32, i32) {
    (a.min(b), a.max(b))
}

/// Record that an entity is *not* an identity.
///
/// Idempotent per subject and identity, and deliberately not per generation: a fold recurs every
/// build, but a veto is a standing decision about the graph.
pub async fn veto(
    graph: &KnowledgeGraph,
    entity_id: i32,
    scheme: &str,
    external_id: &str,
    note: Option<&str>,
) -> anyhow::Result<bool> {
    let Some(entity) = model::knowledge_entity::Entity::find_by_id(entity_id)
        .one(graph.conn())
        .await?
    else {
        anyhow::bail!("no knowledge entity {entity_id}");
    };
    let Some(subject) = entity.origin_observation_id else {
        anyhow::bail!("entity {entity_id} has no origin observation, so no verdict can name it");
    };
    let generation = current_generation(graph.conn()).await?;
    let existing = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .filter(model::knowledge_verdicts::Column::SubjectOrigin.eq(subject))
        .filter(model::knowledge_verdicts::Column::IdentityScheme.eq(scheme))
        .filter(model::knowledge_verdicts::Column::IdentityExternalId.eq(external_id))
        .one(graph.conn())
        .await?;
    if existing.is_some() {
        return Ok(false);
    }
    model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
        generation: Set(generation),
        cause: Set(Cause::Repair),
        kind: Set(Kind::Veto),
        subject_origin: Set(Some(subject)),
        identity_scheme: Set(Some(scheme.to_string())),
        identity_external_id: Set(Some(external_id.to_string())),
        note: Set(note.map(str::to_string)),
        ..Default::default()
    })
    .exec(graph.conn())
    .await?;
    Ok(true)
}

/// Lift a veto. Nothing is un-retracted: lifting stops suppressing a decision, and the next resolve
/// re-derives the claim as a new row, because claims are append-only.
pub async fn unveto(graph: &KnowledgeGraph, verdict_id: i32) -> anyhow::Result<bool> {
    let deleted = model::knowledge_verdicts::Entity::delete_many()
        .filter(model::knowledge_verdicts::Column::Id.eq(verdict_id))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .exec(graph.conn())
        .await?
        .rows_affected;
    Ok(deleted > 0)
}

/// The `(entity, scheme, external_id)` triples a live veto forbids.
///
/// Admission asks this before a claim is written: a claim that is never written cannot leave a window
/// where the graph asserts something an editor has rejected, and does not need a replay to undo.
pub async fn vetoed(
    graph: &KnowledgeGraph,
) -> anyhow::Result<std::collections::HashSet<(i32, String, String)>> {
    let mut out = std::collections::HashSet::new();
    let vetoes = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .all(graph.conn())
        .await?;
    for verdict in vetoes {
        let (Some(subject), Some(scheme), Some(external)) = (
            verdict.subject_origin,
            verdict.identity_scheme.clone(),
            verdict.identity_external_id.clone(),
        ) else {
            continue;
        };
        let Some(row) = entity_with_origin(graph.conn(), subject).await? else {
            continue;
        };
        let Some(entity_id) = canonical(graph.conn(), row.id).await? else {
            continue;
        };
        out.insert((entity_id, scheme, external));
    }
    Ok(out)
}

/// Retract the live claim holding a vetoed identity for its entity, then reproject it.
///
/// The veto names a **pairing**, so the retraction does too: there is at most one live claim for
/// `(entity, identity)`, whatever route produced it. `None` when the subject origin no longer names a
/// live entity; `Some(0)` when the decision already holds and there is nothing to retract.
///
/// The retraction and the projection are one act: `status` and `confidence` are functions of the
/// live claims, so withdrawing a claim without reprojecting leaves the entity `identified` on
/// nothing — and that is the row the API serves and `proximity auto_merge` ranks.
pub(crate) async fn withdraw(
    graph: &KnowledgeGraph,
    verdict: &model::knowledge_verdicts::Model,
) -> anyhow::Result<Option<usize>> {
    let (Some(subject), Some(scheme), Some(external)) = (
        verdict.subject_origin,
        verdict.identity_scheme.as_deref(),
        verdict.identity_external_id.as_deref(),
    ) else {
        return Ok(Some(0));
    };
    let Some(row) = entity_with_origin(graph.conn(), subject).await? else {
        return Ok(None);
    };
    let Some(entity_id) = canonical(graph.conn(), row.id).await? else {
        return Ok(None);
    };
    let Some(identity) = model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::Scheme.eq(scheme))
        .filter(model::knowledge_identity::Column::ExternalId.eq(external))
        .one(graph.conn())
        .await?
    else {
        return Ok(Some(0));
    };
    let retracted = model::knowledge_claims::Entity::update_many()
        .col_expr(
            model::knowledge_claims::Column::RetractedAt,
            sea_orm::sea_query::Expr::current_timestamp(),
        )
        .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity_id))
        .filter(model::knowledge_claims::Column::KnowledgeIdentityId.eq(identity.id))
        .filter(model::knowledge_claims::Column::RetractedAt.is_null())
        .exec(graph.conn())
        .await?
        .rows_affected as usize;
    if retracted > 0 {
        graph.update_projection(entity_id).await?;
    }
    Ok(Some(retracted))
}

/// The entity this origin names now, live or tombstoned.
pub(crate) async fn entity_with_origin(
    conn: &DatabaseConnection,
    origin: i32,
) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
    Ok(model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::OriginObservationId.eq(origin))
        .one(conn)
        .await?)
}

/// Follow tombstones to the live entity a row ends at. Bounded: a cycle is not expressible in the
/// schema, but the reader must not spin if one somehow appears.
pub(crate) async fn canonical(
    conn: &DatabaseConnection,
    id: i32,
) -> anyhow::Result<Option<i32>> {
    let mut current = id;
    for _ in 0..8 {
        let Some(row) = model::knowledge_entity::Entity::find_by_id(current)
            .one(conn)
            .await?
        else {
            return Ok(None);
        };
        match row.merged_into_id {
            Some(next) => current = next,
            None => return Ok(Some(current)),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use crate::knowledge::evidence::ClaimBasis;
    use crate::knowledge::replay::replay;

    use crate::test_support::{db_or_skip, fixtures};

    use super::*;

    /// Seed an entity claiming one identity, as `resolve` would have written it.
    async fn seed_claim(
        db: &crate::database::DbHandle,
        name: &str,
        external_id: &str,
    ) -> i32 {
        let entity_id = fixtures::knowledge_entity_with_member(db, name).await;
        let identity = match model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::Scheme.eq("wikidata"))
            .filter(model::knowledge_identity::Column::ExternalId.eq(external_id))
            .one(&db.conn)
            .await
            .unwrap()
        {
            Some(identity) => identity,
            None => model::knowledge_identity::Entity::insert(
                model::knowledge_identity::ActiveModel {
                    scheme: Set("wikidata".to_string()),
                    external_id: Set(external_id.to_string()),
                    asserted_by: Set("wikidata".to_string()),
                    classes: Set(Vec::new()),
                    ..Default::default()
                },
            )
            .exec_with_returning(&db.conn)
            .await
            .unwrap(),
        };
        model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
            knowledge_entity_id: Set(entity_id),
            knowledge_identity_id: Set(identity.id),
            basis: Set(ClaimBasis::Text),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
        entity_id
    }

    async fn live_claims(db: &crate::database::DbHandle, entity_id: i32) -> usize {
        model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity_id))
            .filter(model::knowledge_claims::Column::RetractedAt.is_null())
            .all(&db.conn)
            .await
            .unwrap()
            .len()
    }

    /// A veto withdraws the one pairing it names — not the identity, and not the entity.
    #[tokio::test]
    async fn a_veto_withdraws_only_the_pairing_it_names() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let vetoed_entity = seed_claim(&db, "Q", "Q131472411").await;
        // The same item, legitimately claimed by a different entity: the veto must not reach it.
        let other = seed_claim(&db, "Q (Star Trek)", "Q131472411").await;

        assert!(
            veto(&store, vetoed_entity, "wikidata", "Q131472411", Some("a short fiction"))
                .await
                .unwrap()
        );
        assert!(
            !veto(&store, vetoed_entity, "wikidata", "Q131472411", None)
                .await
                .unwrap(),
            "recording the same veto twice is a no-op"
        );

        let report = replay(&db.conn, &store).await.unwrap();
        assert_eq!(report.withdrawn, 1, "{report:?}");
        assert_eq!(live_claims(&db, vetoed_entity).await, 0);
        assert_eq!(live_claims(&db, other).await, 1, "the pair, not the identity");
    }

    /// A veto names a pairing, so it retracts the claim whatever route produced it — including one an
    /// authority reached through a handle (P2002), which a `basis = 'text'` filter used to miss.
    #[tokio::test]
    async fn a_veto_retracts_a_claim_that_arrived_by_handle() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = fixtures::knowledge_entity_with_member(&db, "Tsoding").await;
        fixtures::claim(
            &db,
            entity_id,
            "wikidata",
            "Q123737256",
            "wikidata",
            Some("P2002"),
            Some(0.9),
            ClaimBasis::Handle,
        )
        .await;
        store.update_projection(entity_id).await.unwrap();
        assert_eq!(
            store.get(entity_id).await.unwrap().status,
            crate::knowledge::Status::Identified
        );

        veto(&store, entity_id, "wikidata", "Q123737256", None)
            .await
            .unwrap();
        let report = replay(&db.conn, &store).await.unwrap();
        assert_eq!(report.withdrawn, 1, "{report:?}");

        let entity = store.get(entity_id).await.unwrap();
        assert_eq!(
            entity.status,
            crate::knowledge::Status::Provisional,
            "the identity went, so the identification went"
        );
        assert_eq!(entity.confidence, None);

        // A second replay: the decision holds, and says so instead of reporting a zero withdrawal.
        let report = replay(&db.conn, &store).await.unwrap();
        assert_eq!(report.settled, 1, "{report:?}");
        assert_eq!(report.withdrawn, 0, "{report:?}");
    }

    /// The claim is re-derived every build, so the veto has to outlive the rows it withdrew.
    #[tokio::test]
    async fn a_veto_survives_a_rebuild_and_withdraws_the_new_claim() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = seed_claim(&db, "Q", "Q131472411").await;
        let subject = fixtures::origin(&db, entity_id).await;
        veto(&store, entity_id, "wikidata", "Q131472411", None)
            .await
            .unwrap();
        assert_eq!(replay(&db.conn, &store).await.unwrap().withdrawn, 1);

        // A rebuild: entities, identities and claims are recreated and the parser claims again.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn = seed_claim(&db, "Q", "Q131472411").await;
        fixtures::set_origin(&db, reborn, subject).await;

        let report = replay(&db.conn, &store).await.unwrap();
        assert_eq!(report.withdrawn, 1, "against the reborn claim: {report:?}");
        assert_eq!(live_claims(&db, reborn).await, 0);
        assert_eq!(vetoed(&store).await.unwrap().len(), 1, "admission still knows it");
    }

    /// Lifting is deletion. Nothing is un-retracted: the claim returns by being re-derived.
    #[tokio::test]
    async fn lifting_a_veto_stops_suppressing_and_restores_nothing() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = seed_claim(&db, "Los sorias", "Q122707294").await;
        veto(&store, entity_id, "wikidata", "Q122707294", Some("the edition, not the work"))
            .await
            .unwrap();
        replay(&db.conn, &store).await.unwrap();
        assert_eq!(live_claims(&db, entity_id).await, 0);

        let id = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .id;
        assert!(unveto(&store, id).await.unwrap());
        assert!(!unveto(&store, id).await.unwrap(), "already gone");
        assert!(vetoed(&store).await.unwrap().is_empty());
        assert_eq!(
            live_claims(&db, entity_id).await,
            0,
            "lifting is passive; the next resolve re-derives"
        );
        assert_eq!(replay(&db.conn, &store).await.unwrap().withdrawn, 0);
    }

    /// Status and confidence are functions of the live claims. Retracting one without reprojecting
    /// leaves a row the API serves as `identified` and `proximity auto-merge` ranks by a score it no
    /// longer earns.
    #[tokio::test]
    async fn a_veto_reprojects_the_entity_it_empties() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = fixtures::knowledge_entity_with_member(&db, "Q").await;
        fixtures::claim(
            &db,
            entity_id,
            "wikidata",
            "Q131472411",
            "wikidata",
            Some("exact_label"),
            Some(0.9),
            ClaimBasis::Text,
        )
        .await;
        store.update_projection(entity_id).await.unwrap();
        assert_eq!(
            store.get(entity_id).await.unwrap().status,
            crate::knowledge::Status::Identified
        );

        veto(&store, entity_id, "wikidata", "Q131472411", None)
            .await
            .unwrap();
        assert_eq!(replay(&db.conn, &store).await.unwrap().withdrawn, 1);

        let entity = store.get(entity_id).await.unwrap();
        assert_eq!(
            entity.status,
            crate::knowledge::Status::Provisional,
            "no live claim, no identification"
        );
        assert_eq!(entity.confidence, None);
    }

    /// The projection recomputes from what is left: the withdrawn claim's score and its corroboration
    /// bonus go, the other input stays.
    #[tokio::test]
    async fn a_veto_drops_the_score_that_claim_earned_and_no_more() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let entity_id = fixtures::knowledge_entity_with_member(&db, "Nintendo").await;
        fixtures::claim(
            &db,
            entity_id,
            "wikidata",
            "Q8093",
            "wikidata",
            Some("exact_label"),
            Some(0.9),
            ClaimBasis::Text,
        )
        .await;
        fixtures::claim(
            &db,
            entity_id,
            "x_twitter",
            "nintendo",
            "platform",
            None,
            Some(0.7),
            ClaimBasis::Handle,
        )
        .await;
        store.update_projection(entity_id).await.unwrap();
        // Best claim 0.9, plus one corroborating basis.
        assert!(
            (store.get(entity_id).await.unwrap().confidence.unwrap() - 1.0).abs() < 1e-6
        );

        veto(&store, entity_id, "wikidata", "Q8093", None)
            .await
            .unwrap();
        replay(&db.conn, &store).await.unwrap();

        let entity = store.get(entity_id).await.unwrap();
        assert_eq!(
            entity.status,
            crate::knowledge::Status::Identified,
            "the handle still identifies it"
        );
        assert!(
            (entity.confidence.unwrap() - 0.7).abs() < 1e-6,
            "no stale bonus: {entity:?}"
        );
    }
}
