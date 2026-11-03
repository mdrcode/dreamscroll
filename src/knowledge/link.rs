//! Link one illumination's observations to entities.
//!
//! No network. The blocker is an in-memory index the caller built (`~2k` rows, so a scan beats a
//! query plan), the evidence store answers exactly one question — does an entity already hold this
//! claim — and what to do about the answer belongs to a [`LinkDecider`]. Every write is idempotent: a
//! second run finds nothing to do because every observation already has a `knowledge_entity_id`.

use crate::{database, model};

use super::{
    blocking::Blocker,
    entity,
    evidence::{ClaimOutcome, Evidence},
    link_decider::{self, LinkDecider},
};

impl LinkStats {
    /// Fold one decision and what came of it into the totals.
    ///
    /// Both halves live here because they are one question: what did this reading do, and why.
    fn record(&mut self, decision: &link_decider::Decision, applied: Applied) {
        match &decision.action {
            link_decider::Action::Attach { by_claim: true, .. } => self.by_claim += 1,
            link_decider::Action::Create { vetoed: true, .. } => self.vetoed += 1,
            _ => {}
        }
        match applied.outcome {
            Outcome::Attached => self.attached += 1,
            Outcome::Created => self.created += 1,
            Outcome::Folded => self.folded += 1,
        }
    }
}

/// What applying one decision did.
struct Applied {
    /// The entity the observation ended up on — the one to re-project.
    entity: i32,
    outcome: Outcome,
}

enum Outcome {
    /// It joined the entity the decision named.
    Attached,
    /// A new entity was created for it.
    Created,
    /// A new entity was created and then discarded, because another one already held the claim.
    Folded,
}

/// What one illumination's decisions actually did.
///
/// Deliberately not a report: there is no `considered`, no `stale`, no notion of a run. Those are
/// questions the caller asks of its own inputs; this says only what the executor wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub attached: usize,
    /// Attached because a claim named the entity rather than a score.
    pub by_claim: usize,
    pub created: usize,
    /// Created, then dropped: another entity turned out to hold the claim after all.
    pub folded: usize,
    /// The same-illumination veto firing.
    pub vetoed: u32,
}

/// Link one illumination's observed entities into the knowledge graph.
///
/// The blocker and the evidence store are passed in because they are the expensive, shareable things:
/// a caller linking more than one illumination builds them once. This function queries nothing to decide
/// anything — the reads it does are read-modify-write on the entities it is about to change.
pub async fn link_illumination(
    db: &database::DbHandle,
    blocker: &mut dyn Blocker,
    evidence: &Evidence,
    decider: &dyn LinkDecider,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<LinkStats> {
    let decisions = decider.decide_link(blocker, evidence, illumination).await?;

    let mut stats = LinkStats::default();
    for decision in &decisions {
        let applied = apply_decision(db, blocker, evidence, decision).await?;
        // Projection is a function of the members, so it runs after the writes that changed them.
        // An entity two observations landed on is projected twice; a wasted read is cheaper than
        // remembering which ones were already done.
        entity::update_projection(&db.conn, applied.entity).await?;
        stats.record(decision, applied);
    }

    Ok(stats)
}

/// Carry out one decision: give the observation its entity, then record what the capture asserted
/// against it.
async fn apply_decision(
    db: &database::DbHandle,
    blocker: &mut dyn Blocker,
    evidence: &Evidence,
    decision: &link_decider::Decision,
) -> anyhow::Result<Applied> {
    let observation = &decision.observation;
    if observation.knowledge_entity_id.is_some() {
        anyhow::bail!(
            "the linker decided about observation {}, which is already linked",
            observation.id
        );
    }

    // The entity this observation ends up on, resolved once: the one the decision named, or a new
    // one it asked for.
    let mut entity = match &decision.action {
        link_decider::Action::Attach { entity, .. } => entity::get(&db.conn, *entity).await?,
        link_decider::Action::Create { .. } => entity::create(&db.conn, observation).await?,
    };
    entity::attach(&db.conn, observation.id, entity.id).await?;

    // A claim held by another entity turns a *created* row into a duplicate: it owns nothing yet, so
    // it is discarded and the observation joins the entity that got there first. An attach that hits
    // the same wall is a contradiction, not something to paper over.
    let mut folded = false;
    if let Some(claim) = &decision.claim
        && let ClaimOutcome::HeldBy(owner) = evidence.record(entity.id, claim, observation).await?
    {
        if matches!(decision.action, link_decider::Action::Attach { .. }) {
            anyhow::bail!(
                "entities {} and {owner} both hold {}:{}; merging is not implemented yet \
                 (plan/knowledge-canonical-entities.md §Repair)",
                entity.id,
                claim.scheme,
                claim.id
            );
        }
        entity::discard(&db.conn, entity.id).await?;
        entity = entity::get(&db.conn, owner).await?;
        entity::attach(&db.conn, observation.id, owner).await?;
        folded = true;
    }

    // Fold what the capture said into whichever entity won, and let the blocker see the result.
    entity::absorb_observation(&db.conn, &mut entity, observation).await?;
    blocker.changed(&entity);

    let outcome = if folded {
        Outcome::Folded
    } else if matches!(decision.action, link_decider::Action::Attach { .. }) {
        Outcome::Attached
    } else {
        Outcome::Created
    };
    Ok(Applied {
        entity: entity.id,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

    use super::*;
    use crate::knowledge::entity::{STATUS_IDENTIFIED, STATUS_PROVISIONAL};
    use crate::model::{
        knowledge_entity as entity, knowledge_evidence as evidence,
        user_observed_entity as observation,
    };
    use crate::test_support::fixtures;

    struct Capture {
        user_id: i32,
        capture_id: i32,
        illumination_id: i32,
    }

    async fn capture(db: &database::DbHandle, tag: &str) -> Capture {
        let user_id = fixtures::user(db, tag).await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        Capture {
            user_id,
            capture_id,
            illumination_id,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn observe(
        db: &database::DbHandle,
        capture: &Capture,
        position: i32,
        name: &str,
        description: Option<&str>,
        entity_type: &str,
        platform: Option<&str>,
        handle: Option<&str>,
    ) -> i32 {
        observation::Entity::insert(observation::ActiveModel {
            user_id: Set(capture.user_id),
            capture_id: Set(capture.capture_id),
            illumination_id: Set(capture.illumination_id),
            position: Set(position),
            name: Set(name.to_string()),
            entity_type: Set(entity_type.to_string()),
            description: Set(description.map(str::to_string)),
            platform: Set(platform.map(str::to_string)),
            platform_handle: Set(handle.map(str::to_string)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .expect("observation should insert")
        .last_insert_id as i32
    }

    async fn entity_count(db: &database::DbHandle) -> u64 {
        entity::Entity::find().count(&db.conn).await.unwrap()
    }

    #[tokio::test]
    async fn one_capture_never_folds_two_observations_together() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let same = capture(&db, "link_same_capture").await;
        observe(
            &db,
            &same,
            0,
            "Dune",
            Some("a 1965 novel by Frank Herbert"),
            "book",
            None,
            None,
        )
        .await;
        observe(
            &db,
            &same,
            1,
            "Dune",
            Some("a 1965 novel by Frank Herbert"),
            "book",
            None,
            None,
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        assert_eq!(entity_count(&db).await, 2);
    }

    #[tokio::test]
    async fn a_second_run_does_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_idempotent").await;
        observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a novella"),
            "book",
            None,
            None,
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        crate::test_support::link::link_all(&db)
            .await
            .expect("second build should succeed");

        assert_eq!(
            entity_count(&db).await,
            1,
            "one entity, and a second run adds none"
        );
        let unlinked = observation::Entity::find()
            .filter(observation::Column::KnowledgeEntityId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 0, "the second run had nothing left to link");
    }

    /// Every illumination is linked. An illumination can be wrong and the correct illumination is often the
    /// older one, so the graph keeps them all — currency is decided at the view boundary
    /// (`knowledge::current`), not here.
    #[tokio::test]
    async fn a_superseded_illumination_is_linked_too() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user_id = fixtures::user(&db, "link_superseded").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let old = fixtures::illumination(&db, user_id, capture_id).await;
        let live = fixtures::illumination(&db, user_id, capture_id).await;

        for (illumination_id, name) in [(old, "Old Run"), (live, "Train Dreams")] {
            observation::Entity::insert(observation::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(0),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap();
        }

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let unlinked = observation::Entity::find()
            .filter(observation::Column::KnowledgeEntityId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 0, "history is linked, not discarded");
    }

    /// `project` keeps the selected name out of `alt_names`, so an observation is only findable on a
    /// later run if its own name is indexed as well. Without that, every capture that repeated a
    /// name created a duplicate — the local corpus had ten such pairs.
    #[tokio::test]
    async fn a_shared_handle_links_across_captures_and_names() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_persona_first").await;
        let second = capture(&db, "link_persona_second").await;
        observe(
            &db,
            &first,
            0,
            "Alexey Kutepov",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        observe(
            &db,
            &second,
            0,
            "Tsoding",
            None,
            "social_media_account",
            Some("x"),
            Some("Tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        assert_eq!(entity_count(&db).await, 1, "the claim folded the duplicate");
        assert_eq!(evidence::Entity::find().count(&db.conn).await.unwrap(), 1);
    }

    /// The concurrency guard, tested directly: a second entity asking for an identifier that is
    /// already claimed must be told who owns it, not silently ignored.
    #[tokio::test]
    async fn an_identifier_claimed_elsewhere_reports_the_owner() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_claim").await;
        observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let owner_core = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        let duplicate = entity::Entity::insert(entity::ActiveModel {
            name: Set("Tsoding".to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set(STATUS_PROVISIONAL.to_string()),
            alt_names: Set(vec!["tsoding".to_string()]),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;

        let observation = observation::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let claim = crate::knowledge::evidence::Claim::of(&observation).expect("a handle");
        let outcome = Evidence::from_handle(&db)
            .record(duplicate, &claim, &observation)
            .await
            .expect("claim write should succeed");

        assert_eq!(outcome, ClaimOutcome::HeldBy(owner_core.id));
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "the duplicate must not create a second identity row"
        );
    }

    #[tokio::test]
    async fn a_handle_identified_core_is_not_provisional() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_status").await;
        observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let entity = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        assert_eq!(entity.status, STATUS_IDENTIFIED);
    }
}
