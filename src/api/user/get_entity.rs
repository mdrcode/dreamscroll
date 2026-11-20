use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{QueryOrder, QuerySelect};

use crate::{api::*, auth, database::DbHandle, model};

/// Fetch one of the user's unarchived knowledge entities by occurrence ID.
///
/// Ids are only ever handed out from current views, but a stale id can still be presented — so the
/// occurrence is checked against the capture's current illumination before it is returned. The user
/// never sees an illumination their capture no longer stands behind.
pub async fn get_knowledge_entity(
    db: &DbHandle,
    context: &auth::Context,
    id: i32,
) -> Result<model::user_observed_entity::ModelEx, ApiError> {
    let entity = model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::user_observed_entity::Column::Id.eq(id))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .one(&db.conn)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow!("Knowledge entity with id {id} not found")))?;

    Ok(entity.into())
}

/// The newest illumination of every capture, as a set of ids.
///
/// The graph keeps **every** illumination of a capture — an illumination can be wrong, and the
/// correct one is often the older reading — while the view shows exactly one. Illuminations are only
/// ever appended, so the highest id is the latest, and deriving that keeps a stored flag from
/// drifting.
///
/// Both reads below filter on this, so a capture that has been re-read surfaces only its latest
/// reading, whether through the list or through a stale id.
fn current_illumination() -> sea_orm::sea_query::SelectStatement {
    use model::illumination::Column;
    use sea_orm::sea_query::{Alias, Query};
    Query::select()
        .expr_as(Expr::col(Column::Id).max(), Alias::new("id"))
        .from(model::illumination::Entity)
        .group_by_col(Column::CaptureId)
        .to_owned()
}

/// The live knowledge entities behind a batch of observation foreign keys.
///
/// One query per page, not per occurrence: callers collect the ids from models they already hold.
/// A tombstone is excluded rather than resolved — `merge` repoints observations, so a pointer at a
/// merged row means some write bypassed the graph, and serving it would show the absorbed surface
/// as if it were current.
pub async fn knowledge_entity_infos(
    db: &DbHandle,
    ids: impl IntoIterator<Item = i32>,
) -> Result<HashMap<i32, KnowledgeEntityInfo>, ApiError> {
    let mut ids: Vec<i32> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let entities: Vec<model::knowledge_entity::Model> = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::Id.is_in(ids.clone()))
        .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
        .all(&db.conn)
        .await?;

    let mut by_entity: HashMap<i32, Vec<IdentityInfo>> = HashMap::new();
    for identity in model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.is_in(ids))
        .all(&db.conn)
        .await?
    {
        by_entity
            .entry(identity.knowledge_entity_id)
            .or_default()
            .push(IdentityInfo::new(identity));
    }

    // Most confident first; the tie-breaks keep a page stable across requests.
    for identities in by_entity.values_mut() {
        identities.sort_by(|a, b| {
            b.confidence
                .unwrap_or(f32::MIN)
                .partial_cmp(&a.confidence.unwrap_or(f32::MIN))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.scheme.cmp(&b.scheme))
                .then_with(|| a.external_id.cmp(&b.external_id))
        });
    }

    Ok(entities
        .into_iter()
        .map(|entity| {
            let mut info = KnowledgeEntityInfo::new(entity);
            if let Some(identities) = by_entity.remove(&info.id) {
                info.identities = identities;
            }
            (info.id, info)
        })
        .collect())
}

/// The capture an observation belongs to when the observation is real but superseded.
///
/// The entity view only serves a capture's current illumination. After a re-read, links to the old
/// reading land here: send the caller to the capture, which shows the updated entities, rather than
/// guessing which new observation corresponds to the old one.
pub async fn superseded_capture_id(
    db: &DbHandle,
    context: &auth::Context,
    id: i32,
) -> Result<Option<i32>, ApiError> {
    let capture_id = model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .select_only()
        .column(model::user_observed_entity::Column::CaptureId)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::user_observed_entity::Column::Id.eq(id))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .not_in_subquery(current_illumination()),
        )
        .into_tuple::<i32>()
        .one(&db.conn)
        .await?;

    Ok(capture_id)
}

/// Every knowledge entity that shares a reading with one of the caller's observations of this entity.
///
/// One hop: the union over all current member readings of the entity, across the caller's captures.
/// The opened capture's siblings are handled separately by the page (they come first); this returns
/// the rest, one observation per knowledge entity, sorted by name. Excluded ids are entities already
/// shown as current-capture siblings.
pub async fn entity_co_occurrences(
    db: &DbHandle,
    context: &auth::Context,
    knowledge_entity_id: i32,
    excluded_knowledge_entity_ids: &[i32],
) -> Result<Vec<ObservedEntityInfo>, ApiError> {
    let member_readings: Vec<i32> = model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .select_only()
        .column(model::user_observed_entity::Column::IlluminationId)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .into_tuple()
        .all(&db.conn)
        .await?;

    if member_readings.is_empty() {
        return Ok(Vec::new());
    }

    // Newest observation per knowledge entity, so a merged-away pointer settles on what the view
    // currently serves. The member reading's own observation is excluded: `$ke` is the subject.
    let rows: Vec<(i32, Option<i32>)> = model::user_observed_entity::Entity::find()
        .select_only()
        .column(model::user_observed_entity::Column::Id)
        .column(model::user_observed_entity::Column::KnowledgeEntityId)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::user_observed_entity::Column::IlluminationId.is_in(member_readings))
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.ne(knowledge_entity_id))
        .order_by_desc(model::user_observed_entity::Column::Id)
        .into_tuple()
        .all(&db.conn)
        .await?;

    let excluded: HashSet<i32> = excluded_knowledge_entity_ids.iter().copied().collect();
    let mut representatives: HashMap<i32, i32> = HashMap::new();
    for (observation_id, related_entity_id) in rows {
        let Some(related_entity_id) = related_entity_id else {
            continue;
        };
        if excluded.contains(&related_entity_id) {
            continue;
        }
        representatives
            .entry(related_entity_id)
            .or_insert(observation_id);
    }

    if representatives.is_empty() {
        return Ok(Vec::new());
    }

    let observations = model::user_observed_entity::Entity::find()
        .filter(
            model::user_observed_entity::Column::Id
                .is_in(representatives.values().copied().collect::<Vec<_>>()),
        )
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .all(&db.conn)
        .await?;

    let knowledge = knowledge_entity_infos(
        db,
        observations
            .iter()
            .filter_map(|observation| observation.knowledge_entity_id),
    )
    .await?;

    let mut infos: Vec<ObservedEntityInfo> = observations
        .into_iter()
        .map(|observation| {
            let knowledge_entity = observation
                .knowledge_entity_id
                .and_then(|id| knowledge.get(&id))
                .cloned();
            ObservedEntityInfo::new(observation.into(), None, knowledge_entity)
        })
        .collect();
    infos.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(infos)
}

/// The caller's current observations of one entity, one per capture, clicked first.
///
/// Pairs rather than wire rows: the card needs a capture preview, which needs storage, so the
/// caller hydrates. Ordering is the clicked occurrence, then newest observation first; a capture
/// whose reading names the entity twice contributes once.
pub async fn entity_occurrence_pairs(
    db: &DbHandle,
    context: &auth::Context,
    knowledge_entity_id: i32,
    clicked_observation_id: i32,
) -> Result<Vec<(i32, i32)>, ApiError> {
    let rows: Vec<(i32, i32)> = model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .select_only()
        .column(model::user_observed_entity::Column::Id)
        .column(model::user_observed_entity::Column::CaptureId)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .order_by_desc(model::user_observed_entity::Column::Id)
        .into_tuple()
        .all(&db.conn)
        .await?;

    let (clicked, rest): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|(observation, _)| *observation == clicked_observation_id);

    let mut seen = HashSet::new();
    Ok(clicked
        .into_iter()
        .chain(rest)
        .filter(|(_, capture)| seen.insert(*capture))
        .collect())
}

/// Fetch the user's unarchived knowledge entities of the requested type.
///
/// Scoped to each capture's current illumination: the graph keeps every reading, the view keeps one.
pub async fn get_knowledge_entities_by_type(
    db: &DbHandle,
    context: &auth::Context,
    entity_type: &str,
) -> Result<Vec<model::user_observed_entity::Model>, ApiError> {
    Ok(model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(model::user_observed_entity::Column::EntityType.eq(entity_type))
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(&db.conn)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{db_or_skip, fixtures};

    /// One active and one archived capture, each with one entity named after
    /// its capture's state.
    async fn seed(db: &crate::database::DbHandle) -> (i32, Vec<i32>) {
        let user = fixtures::user(db, "entity_get").await;
        let active = fixtures::capture(db, user).await;
        let archived = fixtures::archived_capture(db, user).await;

        let mut ids = Vec::new();
        for (capture_id, name) in [(active, "active entity"), (archived, "archived entity")] {
            let illumination = fixtures::illumination(db, user, capture_id).await;
            ids.push(fixtures::entity(db, user, capture_id, illumination, 0, name).await);
        }

        (user, ids)
    }

    /// A knowledge entity inserted directly, with the optional fields these tests vary.
    async fn knowledge_entity(
        db: &crate::database::DbHandle,
        name: &str,
        platform: Option<(&str, &str)>,
        merged_into: Option<i32>,
    ) -> i32 {
        let (platform, handle) = match platform {
            Some((platform, handle)) => (Some(platform.to_string()), Some(handle.to_string())),
            None => (None, None),
        };
        model::knowledge_entity::Entity::insert(model::knowledge_entity::ActiveModel {
            name: sea_orm::Set(name.to_string()),
            entity_type: sea_orm::Set("unknown".to_string()),
            status: sea_orm::Set(model::knowledge_entity::Status::Provisional),
            platform: sea_orm::Set(platform),
            platform_handle: sea_orm::Set(handle),
            merged_into_id: sea_orm::Set(merged_into),
            // This helper inserts before the observation it will attach; the tests do not read the
            // origin. A real writer always supplies one.
            origin_observation_id: sea_orm::Set(0),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32
    }

    #[tokio::test]
    async fn entity_type_listing_excludes_archived_captures() {
        db_or_skip!(db);
        let (user, _) = seed(&db).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let entities = get_knowledge_entities_by_type(&db, &context, "unknown")
            .await
            .unwrap();
        assert_eq!(
            entities
                .into_iter()
                .map(|entity| entity.name)
                .collect::<Vec<_>>(),
            vec!["active entity"]
        );
    }

    /// The referent is served through the observation the caller owns, and carries the projected
    /// platform presence — the column the resolver now reads.
    #[tokio::test]
    async fn the_linked_entity_is_served_with_its_platform_presence() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "entity_presence").await;

        let knowledge_entity_id =
            knowledge_entity(&db, "active entity", Some(("x_twitter", "tsoding")), None).await;
        let observation_id = fixtures::attached(
            &db,
            &capture,
            0,
            "active entity",
            None,
            "unknown",
            knowledge_entity_id,
        )
        .await;

        let context =
            auth::Context::from(auth::DreamscrollAuthUser::new_test_session(capture.user_id));
        let observation = get_knowledge_entity(&db, &context, observation_id)
            .await
            .unwrap();
        let id = observation
            .knowledge_entity_id
            .expect("the observation is linked");
        let info = knowledge_entity_infos(&db, [id])
            .await
            .unwrap()
            .remove(&id)
            .expect("the observation is linked");
        assert_eq!(info.platform.as_deref(), Some("x_twitter"));
        assert_eq!(info.platform_handle.as_deref(), Some("tsoding"));
        assert_eq!(info.name, "active entity");
    }

    #[tokio::test]
    async fn entity_lookup_excludes_archived_captures() {
        db_or_skip!(db);
        let (user, entity_ids) = seed(&db).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let active = get_knowledge_entity(&db, &context, entity_ids[0])
            .await
            .unwrap();
        assert_eq!(active.name, "active entity");

        let archived = get_knowledge_entity(&db, &context, entity_ids[1])
            .await
            .unwrap_err();
        assert_eq!(archived.http_status_code, axum::http::StatusCode::NOT_FOUND);
    }

    /// The capture is re-read. Both readings are in the graph — `link` keeps history — but the user
    /// only ever sees the current one, whether they list by type or hold a stale id.
    #[tokio::test]
    async fn only_the_current_reading_is_visible() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "entity_current").await;
        let capture = fixtures::capture(&db, user).await;

        let stale_illumination = fixtures::illumination(&db, user, capture).await;
        let stale = fixtures::entity(
            &db,
            user,
            capture,
            stale_illumination,
            0,
            "superseded reading",
        )
        .await;
        let current_illumination = fixtures::illumination(&db, user, capture).await;
        let current = fixtures::entity(
            &db,
            user,
            capture,
            current_illumination,
            0,
            "current reading",
        )
        .await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));

        let listed = get_knowledge_entities_by_type(&db, &context, "unknown")
            .await
            .unwrap();
        assert_eq!(
            listed
                .into_iter()
                .map(|entity| entity.name)
                .collect::<Vec<_>>(),
            vec!["current reading"],
            "a superseded reading must never reach the listing"
        );

        let superseded = get_knowledge_entity(&db, &context, stale)
            .await
            .unwrap_err();
        assert_eq!(
            superseded.http_status_code,
            axum::http::StatusCode::NOT_FOUND
        );

        let live = get_knowledge_entity(&db, &context, current).await.unwrap();
        assert_eq!(live.name, "current reading");
    }

    /// A merged entity is a tombstone kept for history; the observation's consumer must get no
    /// referent rather than the absorbed entity's stale surface.
    #[tokio::test]
    async fn a_merged_entity_is_not_served() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "entity_merged").await;
        let survivor = knowledge_entity(&db, "survivor", None, None).await;
        let absorbed = knowledge_entity(&db, "absorbed", None, Some(survivor)).await;
        let _observation_id =
            fixtures::attached(&db, &capture, 0, "absorbed", None, "unknown", absorbed).await;

        assert!(
            knowledge_entity_infos(&db, [absorbed])
                .await
                .unwrap()
                .is_empty(),
            "a merged entity must not be served as the observation's referent"
        );
    }

    #[tokio::test]
    async fn knowledge_infos_skip_tombstones_and_empty_batches() {
        db_or_skip!(db);
        let live = knowledge_entity(&db, "live", None, None).await;
        let survivor = knowledge_entity(&db, "survivor", None, None).await;
        let absorbed = knowledge_entity(&db, "absorbed", None, Some(survivor)).await;

        let map = knowledge_entity_infos(&db, [live, absorbed, live])
            .await
            .unwrap();
        assert_eq!(map.len(), 1, "one live row, deduped by id");
        assert_eq!(map[&live].name, "live");
        assert!(!map.contains_key(&absorbed));

        assert!(
            knowledge_entity_infos(&db, Vec::<i32>::new())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_superseded_observation_redirects_to_its_capture() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "superseded_redirect").await;
        let capture = fixtures::capture(&db, user).await;
        let first = fixtures::illumination(&db, user, capture).await;
        let old = fixtures::entity(&db, user, capture, first, 0, "old reading").await;
        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));

        assert_eq!(
            superseded_capture_id(&db, &context, old).await.unwrap(),
            None,
            "the only reading is current"
        );

        fixtures::illumination(&db, user, capture).await;
        assert_eq!(
            superseded_capture_id(&db, &context, old).await.unwrap(),
            Some(capture),
            "a newer reading sends the old link to the capture"
        );

        let other = fixtures::user(&db, "superseded_other").await;
        let other_context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(other));
        assert_eq!(
            superseded_capture_id(&db, &other_context, old)
                .await
                .unwrap(),
            None,
            "another user's observation is not theirs to follow"
        );
    }

    #[tokio::test]
    async fn an_archived_capture_does_not_redirect() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "superseded_archived").await;
        let capture = fixtures::archived_capture(&db, user).await;
        let first = fixtures::illumination(&db, user, capture).await;
        let old = fixtures::entity(&db, user, capture, first, 0, "old reading").await;
        fixtures::illumination(&db, user, capture).await;
        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));

        assert_eq!(
            superseded_capture_id(&db, &context, old).await.unwrap(),
            None,
            "archived captures keep the existing not-found behavior"
        );
    }

    #[tokio::test]
    async fn knowledge_infos_carry_identities() {
        db_or_skip!(db);
        let linked = knowledge_entity(&db, "linked", None, None).await;
        let plain = knowledge_entity(&db, "plain", None, None).await;
        fixtures::identity(
            &db,
            linked,
            "x_twitter",
            "handle",
            "illuminator",
            Some("observation"),
            Some(0.7),
        )
        .await;
        fixtures::identity(
            &db,
            linked,
            "wikidata",
            "Q42",
            "wikidata",
            Some("exact_label"),
            Some(0.9),
        )
        .await;

        let map = knowledge_entity_infos(&db, [linked, plain]).await.unwrap();
        let identities = &map[&linked].identities;
        assert_eq!(identities.len(), 2);
        assert_eq!(
            identities[0].external_id, "Q42",
            "the most confident identity leads"
        );
        assert_eq!(identities[0].scheme, "wikidata");
        assert_eq!(identities[0].method.as_deref(), Some("exact_label"));
        assert_eq!(identities[1].external_id, "handle");
        assert!(map[&plain].identities.is_empty());
    }

    #[tokio::test]
    async fn occurrences_lead_with_the_clicked_capture_and_dedupe() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "occurrences").await;
        let shared = knowledge_entity(&db, "shared", None, None).await;

        let first = fixtures::capture(&db, user).await;
        let first_illumination = fixtures::illumination(&db, user, first).await;
        let first_capture = fixtures::Capture {
            user_id: user,
            capture_id: first,
            illumination_id: first_illumination,
        };
        fixtures::attached(&db, &first_capture, 0, "shared", None, "unknown", shared).await;
        let first_later = fixtures::attached(
            &db,
            &first_capture,
            1,
            "shared again",
            None,
            "unknown",
            shared,
        )
        .await;

        let second = fixtures::capture(&db, user).await;
        let second_illumination = fixtures::illumination(&db, user, second).await;
        let second_capture = fixtures::Capture {
            user_id: user,
            capture_id: second,
            illumination_id: second_illumination,
        };
        let clicked =
            fixtures::attached(&db, &second_capture, 0, "shared", None, "unknown", shared).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let pairs = entity_occurrence_pairs(&db, &context, shared, clicked)
            .await
            .unwrap();

        assert_eq!(
            pairs,
            vec![(clicked, second), (first_later, first)],
            "the clicked occurrence leads, one card per capture, newest reading within it"
        );
    }

    #[tokio::test]
    async fn co_occurrences_union_the_member_readings() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "co_occurrences").await;
        let subject = knowledge_entity(&db, "Subject", None, None).await;
        let alpha = knowledge_entity(&db, "Alpha", None, None).await;
        let beta = knowledge_entity(&db, "Beta", None, None).await;
        let gamma = knowledge_entity(&db, "Gamma", None, None).await;

        let first = fixtures::capture(&db, user).await;
        let first_illumination = fixtures::illumination(&db, user, first).await;
        let first_reading = fixtures::Capture {
            user_id: user,
            capture_id: first,
            illumination_id: first_illumination,
        };
        fixtures::attached(&db, &first_reading, 0, "Subject", None, "unknown", subject).await;
        fixtures::attached(&db, &first_reading, 1, "Alpha", None, "unknown", alpha).await;

        let second = fixtures::capture(&db, user).await;
        let second_illumination = fixtures::illumination(&db, user, second).await;
        let second_reading = fixtures::Capture {
            user_id: user,
            capture_id: second,
            illumination_id: second_illumination,
        };
        fixtures::attached(&db, &second_reading, 0, "Subject", None, "unknown", subject).await;
        fixtures::attached(&db, &second_reading, 1, "Beta", None, "unknown", beta).await;

        // Another user's capture must not leak into the caller's union.
        let stranger = fixtures::user(&db, "co_occurrences_other").await;
        let stranger_capture = fixtures::capture(&db, stranger).await;
        let stranger_illumination = fixtures::illumination(&db, stranger, stranger_capture).await;
        let stranger_reading = fixtures::Capture {
            user_id: stranger,
            capture_id: stranger_capture,
            illumination_id: stranger_illumination,
        };
        fixtures::attached(
            &db,
            &stranger_reading,
            0,
            "Subject",
            None,
            "unknown",
            subject,
        )
        .await;
        fixtures::attached(&db, &stranger_reading, 1, "Gamma", None, "unknown", gamma).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let union = entity_co_occurrences(&db, &context, subject, &[])
            .await
            .unwrap();
        assert_eq!(
            union
                .iter()
                .map(|info| info.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta"],
            "both of the caller's readings contribute, sorted by name"
        );
        assert!(union.iter().all(|info| info.knowledge_entity.is_some()));

        let remaining = entity_co_occurrences(&db, &context, subject, &[alpha])
            .await
            .unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|info| info.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Beta"],
            "entities already shown as current-capture siblings are excluded"
        );
    }
}
