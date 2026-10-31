//! Link every observation to a core, creating one when nothing is close enough.
//!
//! No network. Blocking is an in-memory index over the cores (`~2k` rows per user, so a scan is
//! cheaper than a query plan), identifiers are a decisive lookup, the decision is [`score`], and
//! every write is idempotent: a second run finds nothing to do because every observation already has
//! a `core_id`.

use std::collections::{BTreeSet, HashMap, HashSet};

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DbErr, EntityTrait,
    QueryFilter, QueryOrder, RuntimeErr, Set, Statement,
};

use super::{normalize, score};
use crate::{database, model};

pub const STATUS_PROVISIONAL: &str = "provisional";
pub const STATUS_IDENTIFIED: &str = "identified";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub considered: usize,
    /// Observations that already had a core.
    pub already_linked: usize,
    /// Observations from a superseded illumination of their capture.
    pub superseded: usize,
    /// Linked by an identifier rather than by similarity.
    pub linked_by_identifier: usize,
    pub linked: usize,
    pub created: usize,
}

struct CoreState {
    scored: score::Core,
    aliases: Vec<String>,
}

impl CoreState {
    fn from_model(row: model::core_knowledge_entity::Model) -> Self {
        Self {
            scored: score::Core::new(
                row.id,
                &row.aliases,
                row.description.as_deref(),
                &row.entity_type,
            ),
            aliases: row.aliases,
        }
    }

    /// Add a normalized surface. Returns whether anything changed, so a caller can skip the write.
    fn add_alias(&mut self, surface: &str) -> bool {
        let key = normalize::key(surface);
        if key.is_empty() || self.aliases.contains(&key) {
            return false;
        }
        self.aliases.push(key.clone());
        self.scored.alias_keys.push(key);
        self.scored
            .alias_token_keys
            .push(normalize::token_key(surface));
        true
    }
}

/// `(scheme, id)` out of an identity evidence payload. The keys are a contract with the partial
/// unique index in `database::indexes::knowledge`.
fn identity_key_of(value: &serde_json::Value) -> Option<(String, String)> {
    let scheme = value.get("scheme")?.as_str()?;
    let id = value.get("id")?.as_str()?;
    Some((scheme.to_lowercase(), id.to_string()))
}

/// The `(scheme, value)` an observation carries, if it carries one. `scheme` is the platform; the
/// same shape covers `wikidata` → `Q…`, `imdb` → `tt…`, and every authority after them.
fn identifier_of(entity: &model::user_knowledge_entity::Model) -> Option<(String, String)> {
    let platform = entity.platform.as_deref()?;
    let handle = entity.platform_handle.as_deref()?;
    Some((platform.to_lowercase(), normalize::key(handle)))
}

pub async fn build_core(db: &database::DbHandle) -> anyhow::Result<Report> {
    let entities = model::user_knowledge_entity::Entity::find()
        .order_by_asc(model::user_knowledge_entity::Column::Id)
        .all(&db.conn)
        .await?;

    // Only the latest illumination of a capture is live; earlier runs are still rows.
    let mut latest: HashMap<i32, i32> = HashMap::new();
    for entity in &entities {
        let current = latest
            .entry(entity.capture_id)
            .or_insert(entity.illumination_id);
        *current = (*current).max(entity.illumination_id);
    }

    let mut cores: Vec<CoreState> = model::core_knowledge_entity::Entity::find()
        .order_by_asc(model::core_knowledge_entity::Column::Id)
        .all(&db.conn)
        .await?
        .into_iter()
        .filter(|row| row.superseded_by_id.is_none())
        .map(CoreState::from_model)
        .collect();

    let mut positions: HashMap<i32, usize> = cores
        .iter()
        .enumerate()
        .map(|(position, core)| (core.scored.id, position))
        .collect();

    let mut index: HashMap<String, BTreeSet<usize>> = HashMap::new();
    for (position, core) in cores.iter().enumerate() {
        index_core(&mut index, position, core);
    }

    // Any authority, one lookup. Identity is a claim like any other, so it is read from evidence:
    // `(scheme, id)` over every live identity row, whatever the authority.
    let mut identifiers: HashMap<(String, String), i32> = HashMap::new();
    for row in model::core_knowledge_evidence::Entity::find()
        .filter(model::core_knowledge_evidence::Column::Kind.is_in(["ontology", "platform_link"]))
        .filter(model::core_knowledge_evidence::Column::RetractedAt.is_null())
        .all(&db.conn)
        .await?
    {
        if let Some(key) = identity_key_of(&row.value) {
            identifiers.insert(key, row.core_id);
        }
    }

    // (core, capture) pairs already linked, so one capture never folds two of its own observations
    // into the same core: eight prod pairs prove the same string can mean two things in one capture.
    // A shared identifier overrides this — the same account is the same referent.
    let mut seen: HashSet<(i32, i32)> = HashSet::new();

    let mut report = Report {
        considered: entities.len(),
        ..Default::default()
    };

    for entity in entities {
        if entity.core_id.is_some() {
            report.already_linked += 1;
            continue;
        }
        if latest.get(&entity.capture_id) != Some(&entity.illumination_id) {
            report.superseded += 1;
            continue;
        }

        // Identity first: an identifier is decisive and needs no scoring.
        if let Some(core_id) = identifier_of(&entity)
            .and_then(|key| identifiers.get(&key).copied())
            .filter(|core_id| positions.contains_key(core_id))
        {
            let position = positions[&core_id];
            link(&db.conn, entity.id, core_id).await?;
            seen.insert((core_id, entity.capture_id));
            if absorb_aliases(&db.conn, &mut cores[position], &entity).await? {
                index_core(&mut index, position, &cores[position]);
            }
            report.linked_by_identifier += 1;
            continue;
        }

        let observation = score::Observation::new(
            &entity.name,
            entity.description.as_deref(),
            &entity.entity_type,
        );

        let mut candidates: BTreeSet<usize> = BTreeSet::new();
        for block in [&observation.name_key, &observation.token_key] {
            if let Some(hits) = index.get(block) {
                candidates.extend(hits.iter().copied());
            }
        }

        let best = candidates
            .into_iter()
            .filter(|position| {
                !seen.contains(&(cores[*position].scored.id, entity.capture_id))
            })
            .map(|position| (position, score::score(&observation, &cores[position].scored)))
            .max_by(|a, b| a.1.total.total_cmp(&b.1.total))
            .filter(|(_, features)| score::worth_merging(features));

        match best {
            Some((position, _)) => {
                let core_id = cores[position].scored.id;
                link(&db.conn, entity.id, core_id).await?;
                seen.insert((core_id, entity.capture_id));
                if absorb_aliases(&db.conn, &mut cores[position], &entity).await? {
                    index_core(&mut index, position, &cores[position]);
                }
                let claimed = identifier_of(&entity).is_some();
                if let Some(owner) =
                    remember_identifier(&db.conn, &mut identifiers, core_id, &entity).await?
                {
                    anyhow::bail!(
                        "cores {core_id} and {owner} both claim the same identifier; \
                         merging is not implemented yet (plan/knowledge-canonical-entities.md §Repair)"
                    );
                }
                if claimed {
                    // The core may have been created without an identity and is only now acquiring
                    // one. `status` is a projection of evidence, so it moves with it.
                    mark_identified(&db.conn, core_id).await?;
                }
                report.linked += 1;
            }
            None => {
                let core_id = create(&db.conn, &entity).await?;

                // A concurrent worker may have claimed this identifier since the map was read.
                // The new core is then a duplicate that owns nothing yet: link to the real owner
                // and forget it.
                if let Some(owner) =
                    remember_identifier(&db.conn, &mut identifiers, core_id, &entity).await?
                {
                    link(&db.conn, entity.id, owner).await?;
                    model::core_knowledge_entity::Entity::delete_by_id(core_id)
                        .exec(&db.conn)
                        .await?;
                    seen.insert((owner, entity.capture_id));
                    if let Some(&position) = positions.get(&owner)
                        && absorb_aliases(&db.conn, &mut cores[position], &entity).await?
                    {
                        index_core(&mut index, position, &cores[position]);
                    }
                    report.linked_by_identifier += 1;
                    continue;
                }

                let aliases = aliases_for(&entity);
                cores.push(CoreState {
                    scored: score::Core::new(
                        core_id,
                        &aliases,
                        entity.description.as_deref(),
                        &entity.entity_type,
                    ),
                    aliases,
                });
                let position = cores.len() - 1;
                positions.insert(core_id, position);
                index_core(&mut index, position, cores.last().expect("just pushed"));
                link(&db.conn, entity.id, core_id).await?;
                seen.insert((core_id, entity.capture_id));
                report.created += 1;
            }
        }
    }

    Ok(report)
}

fn index_core(index: &mut HashMap<String, BTreeSet<usize>>, position: usize, core: &CoreState) {
    for key in &core.scored.alias_keys {
        index.entry(key.clone()).or_default().insert(position);
    }
    for key in &core.scored.alias_token_keys {
        index.entry(key.clone()).or_default().insert(position);
    }
}

fn surfaces(
    entity: &model::user_knowledge_entity::Model,
) -> impl Iterator<Item = &str> {
    [
        Some(entity.name.as_str()),
        entity.platform_handle.as_deref(),
        entity.platform_display_name.as_deref(),
    ]
    .into_iter()
    .flatten()
}

fn aliases_for(entity: &model::user_knowledge_entity::Model) -> Vec<String> {
    let mut aliases = Vec::new();
    for surface in surfaces(entity) {
        let key = normalize::key(surface);
        if !key.is_empty() && !aliases.contains(&key) {
            aliases.push(key);
        }
    }
    aliases
}

/// `identified` is a claim about evidence, so it is true the moment an identity exists — including
/// when a provisional core absorbs an observation that carries one. A no-op when already set.
async fn mark_identified(
    conn: &sea_orm::DatabaseConnection,
    core_id: i32,
) -> anyhow::Result<()> {
    model::core_knowledge_entity::Entity::update_many()
        .col_expr(
            model::core_knowledge_entity::Column::Status,
            Expr::value(STATUS_IDENTIFIED),
        )
        .filter(model::core_knowledge_entity::Column::Id.eq(core_id))
        .filter(model::core_knowledge_entity::Column::Status.ne(STATUS_IDENTIFIED))
        .exec(conn)
        .await?;
    Ok(())
}

async fn link(
    conn: &sea_orm::DatabaseConnection,
    entity_id: i32,
    core_id: i32,
) -> anyhow::Result<()> {
    model::user_knowledge_entity::Entity::update_many()
        .col_expr(
            model::user_knowledge_entity::Column::CoreId,
            Expr::value(Some(core_id)),
        )
        .filter(model::user_knowledge_entity::Column::Id.eq(entity_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// Fold this observation's surfaces into the core it was linked to. This is the whole of `project`
/// for now: no authority evidence exists yet, so `label`, `entity_type` and `description` keep the
/// values the core was created with.
async fn absorb_aliases(
    conn: &sea_orm::DatabaseConnection,
    core: &mut CoreState,
    entity: &model::user_knowledge_entity::Model,
) -> anyhow::Result<bool> {
    let mut changed = false;
    for surface in surfaces(entity) {
        changed |= core.add_alias(surface);
    }
    if changed {
        model::core_knowledge_entity::ActiveModel {
            id: Set(core.scored.id),
            aliases: Set(core.aliases.clone()),
            ..Default::default()
        }
        .update(conn)
        .await?;
    }
    Ok(changed)
}

/// Record the observation's identifier against its core. Returns the *other* core that already
/// claims it, if any — the caller decides what to do, because dropping the claim silently would
/// leave two cores for one referent and no trace of the race.
///
/// The read-back is what makes this race-safe: `ON CONFLICT DO NOTHING` leaves whichever row was
/// committed first, and comparing its owner is cheaper and clearer than matching a unique-violation
/// error code.
async fn remember_identifier(
    conn: &sea_orm::DatabaseConnection,
    known: &mut HashMap<(String, String), i32>,
    core_id: i32,
    entity: &model::user_knowledge_entity::Model,
) -> anyhow::Result<Option<i32>> {
    let Some((scheme, value)) = identifier_of(entity) else {
        return Ok(None);
    };
    if let Some(&existing) = known.get(&(scheme.clone(), value.clone())) {
        return Ok((existing != core_id).then_some(existing));
    }

    // The partial unique index refuses a second live claim to the same (scheme, id). Under a
    // concurrent writer that surfaces as a unique violation, so the run stops rather than leaving
    // two cores for one referent; turning it into a merge is phase 3 (§Repair).
    let insert = model::core_knowledge_evidence::Entity::insert(
        model::core_knowledge_evidence::ActiveModel {
            core_id: Set(core_id),
            kind: Set("platform_link".to_string()),
            value: Set(serde_json::json!({
                "scheme": scheme,
                "id": value,
                "handle": entity.platform_handle,
            })),
            source: Set("platform".to_string()),
            method: Set(Some("capture".to_string())),
            user_entity_id: Set(Some(entity.id)),
            source_user_id: Set(Some(entity.user_id)),
            ..Default::default()
        },
    )
    .exec(conn)
    .await;

    match insert {
        Ok(_) => {
            known.insert((scheme, value), core_id);
            Ok(None)
        }
        // Another core claimed this identity between the map read and the insert. The index said so;
        // report the owner so the caller can drop the duplicate instead of leaving two referents.
        Err(err) if is_unique_violation(&err) => {
            let owner = identity_owner(conn, &scheme, &value).await?;
            match owner {
                Some(owner) if owner != core_id => {
                    known.insert((scheme, value), owner);
                    Ok(Some(owner))
                }
                Some(_) => {
                    known.insert((scheme, value), core_id);
                    Ok(None)
                }
                None => Err(err.into()),
            }
        }
        Err(err) => Err(err.into()),
    }
}

fn is_unique_violation(err: &DbErr) -> bool {
    match err {
        DbErr::Exec(RuntimeErr::SqlxError(error)) | DbErr::Query(RuntimeErr::SqlxError(error)) => {
            error
                .as_database_error()
                .and_then(|error| error.code())
                .is_some_and(|code| code == "23505")
        }
        _ => false,
    }
}

async fn identity_owner(
    conn: &sea_orm::DatabaseConnection,
    scheme: &str,
    value: &str,
) -> anyhow::Result<Option<i32>> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT core_id FROM core_knowledge_evidence \
             WHERE kind IN ('ontology', 'platform_link') AND retracted_at IS NULL \
               AND value->>'scheme' = $1 AND value->>'id' = $2 \
             LIMIT 1",
            [scheme.into(), value.into()],
        ))
        .await?;
    Ok(match row {
        Some(row) => Some(row.try_get_by_index::<i32>(0)?),
        None => None,
    })
}

async fn create(
    conn: &sea_orm::DatabaseConnection,
    entity: &model::user_knowledge_entity::Model,
) -> anyhow::Result<i32> {
    let description = entity.description.clone();
    // A handle is an identifier, so the row is identified from the start; anything else is a
    // hypothesis until resolution supplies an authority identifier.
    let status = if identifier_of(entity).is_some() {
        STATUS_IDENTIFIED
    } else {
        STATUS_PROVISIONAL
    };
    let model = model::core_knowledge_entity::ActiveModel {
        label: Set(entity.name.trim().to_string()),
        entity_type: Set(entity.entity_type.clone()),
        // The capture's own reading, recorded as such: authority text displaces it later.
        description_source: Set(description.as_ref().map(|_| "capture".to_string())),
        description: Set(description),
        status: Set(status.to_string()),
        confidence: Set(None),
        superseded_by_id: Set(None),
        aliases: Set(aliases_for(entity)),
        ..Default::default()
    };
    let inserted = model::core_knowledge_entity::Entity::insert(model)
        .exec(conn)
        .await?;
    Ok(inserted.last_insert_id as i32)
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;

    use super::*;
    use crate::model::{
        core_knowledge_entity as core, core_knowledge_evidence as evidence,
        user_knowledge_entity as observation,
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

    async fn core_count(db: &database::DbHandle) -> u64 {
        core::Entity::find().count(&db.conn).await.unwrap()
    }

    #[tokio::test]
    async fn same_name_across_captures_shares_one_core() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_first").await;
        let second = capture(&db, "link_second").await;
        observe(&db, &first, 0, "Train Dreams", Some("a novella by Denis Johnson"), "book", None, None).await;
        observe(&db, &second, 0, "train dreams", Some("a novella by Denis Johnson"), "book", None, None).await;

        let report = build_core(&db).await.expect("build should succeed");

        assert_eq!(report.created, 1, "{report:?}");
        assert_eq!(report.linked, 1, "{report:?}");
        assert_eq!(core_count(&db).await, 1);
    }

    #[tokio::test]
    async fn one_capture_never_folds_two_observations_together() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let same = capture(&db, "link_same_capture").await;
        observe(&db, &same, 0, "Dune", Some("a 1965 novel by Frank Herbert"), "book", None, None).await;
        observe(&db, &same, 1, "Dune", Some("a 1965 novel by Frank Herbert"), "book", None, None).await;

        let report = build_core(&db).await.expect("build should succeed");

        assert_eq!(report.created, 2, "{report:?}");
        assert_eq!(core_count(&db).await, 2);
    }

    #[tokio::test]
    async fn distinct_names_get_distinct_cores() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_distinct_first").await;
        let second = capture(&db, "link_distinct_second").await;
        observe(&db, &first, 0, "Train Dreams", None, "book", None, None).await;
        observe(&db, &second, 0, "Gravity's Rainbow", None, "book", None, None).await;

        let report = build_core(&db).await.expect("build should succeed");

        assert_eq!(report.created, 2, "{report:?}");
        assert_eq!(core_count(&db).await, 2);
    }

    #[tokio::test]
    async fn a_second_run_does_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_idempotent").await;
        observe(&db, &first, 0, "Train Dreams", Some("a novella"), "book", None, None).await;

        let first_run = build_core(&db).await.expect("build should succeed");
        let second_run = build_core(&db).await.expect("second build should succeed");

        assert_eq!(first_run.created, 1, "{first_run:?}");
        assert_eq!(second_run.created, 0, "{second_run:?}");
        assert_eq!(second_run.linked, 0, "{second_run:?}");
        assert_eq!(second_run.already_linked, 1, "{second_run:?}");
    }

    #[tokio::test]
    async fn superseded_illuminations_are_skipped() {
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

        let report = build_core(&db).await.expect("build should succeed");

        assert_eq!(report.superseded, 1, "{report:?}");
        assert_eq!(report.created, 1, "{report:?}");
    }

    /// The persona case: an account's own name is the identity, so two captures that name the same
    /// person differently still share a core. Without the identifier lookup these would be two cores,
    /// because the names share no tokens.
    #[tokio::test]
    async fn a_shared_handle_links_across_captures_and_names() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_persona_first").await;
        let second = capture(&db, "link_persona_second").await;
        observe(&db, &first, 0, "Alexey Kutepov", None, "real_person", Some("x"), Some("tsoding")).await;
        observe(&db, &second, 0, "Tsoding", None, "social_media_account", Some("x"), Some("Tsoding")).await;

        let report = build_core(&db).await.expect("build should succeed");

        assert_eq!(report.created, 1, "{report:?}");
        assert_eq!(report.linked_by_identifier, 1, "{report:?}");
        assert_eq!(core_count(&db).await, 1);
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1
        );
    }

    /// The concurrency guard, tested directly: a second core asking for an identifier that is
    /// already claimed must be told who owns it, not silently ignored.
    #[tokio::test]
    async fn an_identifier_claimed_elsewhere_reports_the_owner() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_claim").await;
        observe(&db, &first, 0, "Tsoding", None, "real_person", Some("x"), Some("tsoding")).await;
        build_core(&db).await.expect("build should succeed");

        let owner_core = core::Entity::find().one(&db.conn).await.unwrap().unwrap();
        let duplicate = core::Entity::insert(core::ActiveModel {
            label: Set("Tsoding".to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set(STATUS_PROVISIONAL.to_string()),
            aliases: Set(vec!["tsoding".to_string()]),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;

        let entity = observation::Entity::find().one(&db.conn).await.unwrap().unwrap();
        let mut known = HashMap::new();
        let claimed = remember_identifier(&db.conn, &mut known, duplicate, &entity)
            .await
            .expect("claim lookup should succeed");

        assert_eq!(claimed, Some(owner_core.id));
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
        observe(&db, &first, 0, "Tsoding", None, "real_person", Some("x"), Some("tsoding")).await;

        build_core(&db).await.expect("build should succeed");

        let core = core::Entity::find().one(&db.conn).await.unwrap().unwrap();
        assert_eq!(core.status, STATUS_IDENTIFIED);
    }
}
