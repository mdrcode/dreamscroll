//! Link every observation to an entity, creating one when nothing is close enough.
//!
//! No network. Blocking is an in-memory index over the entities (`~2k` rows per user, so a scan is
//! cheaper than a query plan), identifiers are a decisive lookup, the decision is [`score`], and
//! every write is idempotent: a second run finds nothing to do because every observation already has
//! a `knowledge_entity_id`.

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
    /// Observations that already had an entity.
    pub already_linked: usize,
    /// Observations linked from a superseded illumination of their capture. Counted, not skipped:
    /// the graph holds every reading, and the view decides which one a capture currently shows.
    pub stale: usize,
    /// Linked by an identifier rather than by similarity.
    pub linked_by_identifier: usize,
    pub linked: usize,
    pub created: usize,
}

struct EntityState {
    scored: score::Scored,
    /// Verbatim surfaces; `scored.alt_name_keys` holds the normalized forms derived from them.
    alt_names: Vec<String>,
    /// Types other members asserted. Kept in step with `scored.entity_types`, which is what the
    /// scorer consults — an entity that has seen `brand` must accept `brand` for the rest of the run.
    alt_types: Vec<String>,
}

impl EntityState {
    fn from_model(row: model::knowledge_entity::Model) -> Self {
        let mut descriptions: Vec<&str> = row.description.iter().map(String::as_str).collect();
        descriptions.extend(row.alt_descriptions.iter().map(String::as_str));
        let mut entity_types = vec![row.entity_type.clone()];
        entity_types.extend(row.alt_types.clone());
        Self {
            scored: score::Scored::new(row.id, &row.name, &row.alt_names, &descriptions, &entity_types),
            alt_names: row.alt_names,
            alt_types: row.alt_types,
        }
    }

    /// Add an observed type. Returns whether anything changed; the scorer accepts agreement with
    /// any asserted type, so this widens the gate for later observations in the same run.
    fn add_type(&mut self, entity_type: &str) -> bool {
        if entity_type.is_empty() || self.scored.entity_types.iter().any(|t| t == entity_type) {
            return false;
        }
        self.alt_types.push(entity_type.to_string());
        self.scored.entity_types.push(entity_type.to_string());
        true
    }

    /// Add a surface. Returns whether anything changed, so a caller can skip the write.
    fn add_name(&mut self, surface: &str) -> bool {
        let key = normalize::key(surface);
        if key.is_empty() || self.scored.alt_name_keys.contains(&key) {
            return false;
        }
        self.alt_names.push(surface.trim().to_string());
        self.scored.alt_name_keys.push(key);
        self.scored
            .alt_name_token_keys
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
fn identifier_of(observation: &model::user_observed_entity::Model) -> Option<(String, String)> {
    let platform = observation.platform.as_deref()?;
    let handle = observation.platform_handle.as_deref()?;
    Some((platform.to_lowercase(), normalize::key(handle)))
}

pub async fn link(db: &database::DbHandle) -> anyhow::Result<Report> {
    let observations = model::user_observed_entity::Entity::find()
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(&db.conn)
        .await?;

    // Which illumination is current, for the report only. Earlier readings are linked too: an
    // illumination can be wrong (a re-read of one image once described a different film), and the
    // graph should hold what every reading said. Currency is a view concern, not a linking one.
    let mut latest: HashMap<i32, i32> = HashMap::new();
    for observation in &observations {
        let current = latest
            .entry(observation.capture_id)
            .or_insert(observation.illumination_id);
        *current = (*current).max(observation.illumination_id);
    }

    let mut entities: Vec<EntityState> = model::knowledge_entity::Entity::find()
        .order_by_asc(model::knowledge_entity::Column::Id)
        .all(&db.conn)
        .await?
        .into_iter()
        .filter(|row| row.merged_into_id.is_none())
        .map(EntityState::from_model)
        .collect();

    let mut positions: HashMap<i32, usize> = entities
        .iter()
        .enumerate()
        .map(|(position, entity)| (entity.scored.id, position))
        .collect();

    let mut index: HashMap<String, BTreeSet<usize>> = HashMap::new();
    for (position, entity) in entities.iter().enumerate() {
        index_core(&mut index, position, entity);
    }

    // Any authority, one lookup. Identity is a claim like any other, so it is read from evidence:
    // `(scheme, id)` over every live identity row, whatever the authority.
    let mut identifiers: HashMap<(String, String), i32> = HashMap::new();
    for row in model::knowledge_evidence::Entity::find()
        .filter(model::knowledge_evidence::Column::Kind.eq("claim"))
        .filter(model::knowledge_evidence::Column::RetractedAt.is_null())
        .all(&db.conn)
        .await?
    {
        if let Some(key) = identity_key_of(&row.value) {
            identifiers.insert(key, row.knowledge_entity_id);
        }
    }

    // (entity, illumination) pairs already linked: one *extraction* never folds two of its own
    // observations into one entity, because a single read emitting the same string twice means two
    // things. The unit is the illumination, not the capture — re-reading an image is a separate
    // reading, and its names may legitimately join the same observation. A shared identifier overrides
    // this anyway: the same account is the same referent.
    let mut seen: HashSet<(i32, i32)> = HashSet::new();

    let mut report = Report {
        considered: entities.len(),
        ..Default::default()
    };

    for observation in observations {
        if observation.knowledge_entity_id.is_some() {
            report.already_linked += 1;
            continue;
        }
        if latest.get(&observation.capture_id) != Some(&observation.illumination_id) {
            report.stale += 1;
        }

        // Identity first: an identifier is decisive and needs no scoring.
        if let Some(knowledge_entity_id) = identifier_of(&observation)
            .and_then(|key| identifiers.get(&key).copied())
            .filter(|knowledge_entity_id| positions.contains_key(knowledge_entity_id))
        {
            let position = positions[&knowledge_entity_id];
            attach(&db.conn, observation.id, knowledge_entity_id).await?;
            seen.insert((knowledge_entity_id, observation.illumination_id));
            if absorb_names(&db.conn, &mut entities[position], &observation).await? {
                index_core(&mut index, position, &entities[position]);
            }
            report.linked_by_identifier += 1;
            continue;
        }

        let view = score::Observation::new(
            &observation.name,
            observation.description.as_deref(),
            &observation.entity_type,
        );

        let mut candidates: BTreeSet<usize> = BTreeSet::new();
        for block in [&view.name_key, &view.token_key] {
            if let Some(hits) = index.get(block) {
                candidates.extend(hits.iter().copied());
            }
        }

        let best = candidates
            .into_iter()
            .filter(|position| {
                !seen.contains(&(entities[*position].scored.id, observation.illumination_id))
            })
            .map(|position| (position, score::score(&view, &entities[position].scored)))
            .max_by(|a, b| a.1.total.total_cmp(&b.1.total))
            .filter(|(_, features)| score::worth_merging(features));

        match best {
            Some((position, _)) => {
                let knowledge_entity_id = entities[position].scored.id;
                attach(&db.conn, observation.id, knowledge_entity_id).await?;
                seen.insert((knowledge_entity_id, observation.illumination_id));
                if absorb_names(&db.conn, &mut entities[position], &observation).await? {
                    index_core(&mut index, position, &entities[position]);
                }
                let claimed = identifier_of(&observation).is_some();
                if let Some(owner) =
                    remember_identifier(&db.conn, &mut identifiers, knowledge_entity_id, &observation).await?
                {
                    anyhow::bail!(
                        "entities {knowledge_entity_id} and {owner} both claim the same identifier; \
                         merging is not implemented yet (plan/knowledge-canonical-entities.md §Repair)"
                    );
                }
                if claimed {
                    // The entity may have been created without an identity and is only now acquiring
                    // one. `status` is a projection of evidence, so it moves with it.
                    mark_identified(&db.conn, knowledge_entity_id).await?;
                }
                report.linked += 1;
            }
            None => {
                let knowledge_entity_id = create(&db.conn, &observation).await?;

                // A concurrent worker may have claimed this identifier since the map was read.
                // The new entity is then a duplicate that owns nothing yet: link to the real owner
                // and forget it.
                if let Some(owner) =
                    remember_identifier(&db.conn, &mut identifiers, knowledge_entity_id, &observation).await?
                {
                    attach(&db.conn, observation.id, owner).await?;
                    model::knowledge_entity::Entity::delete_by_id(knowledge_entity_id)
                        .exec(&db.conn)
                        .await?;
                    seen.insert((owner, observation.illumination_id));
                    if let Some(&position) = positions.get(&owner)
                        && absorb_names(&db.conn, &mut entities[position], &observation).await?
                    {
                        index_core(&mut index, position, &entities[position]);
                    }
                    report.linked_by_identifier += 1;
                    continue;
                }

                let alt_names = names_for(&observation);
                entities.push(EntityState {
                    scored: score::Scored::new(
                        knowledge_entity_id,
                        &observation.name,
                        &alt_names,
                        &observation.description.iter().map(String::as_str).collect::<Vec<_>>(),
                        std::slice::from_ref(&observation.entity_type),
                    ),
                    alt_names,
                    alt_types: Vec::new(),
                });
                let position = entities.len() - 1;
                positions.insert(knowledge_entity_id, position);
                index_core(&mut index, position, entities.last().expect("just pushed"));
                attach(&db.conn, observation.id, knowledge_entity_id).await?;
                seen.insert((knowledge_entity_id, observation.illumination_id));
                report.created += 1;
            }
        }
    }

    // A entity is a function of its members, not of whichever observation happened to create it.
    // Projecting once at the end keeps the accumulated descriptions, status and confidence current,
    // and repairs rows written before projection covered those columns.
    for entity in model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
        .all(&db.conn)
        .await?
    {
        crate::knowledge::resolve::project(&db.conn, entity.id).await?;
    }

    Ok(report)
}

fn index_core(index: &mut HashMap<String, BTreeSet<usize>>, position: usize, entity: &EntityState) {
    for key in &entity.scored.alt_name_keys {
        index.entry(key.clone()).or_default().insert(position);
    }
    for key in &entity.scored.alt_name_token_keys {
        index.entry(key.clone()).or_default().insert(position);
    }
}

pub(crate) fn surfaces(
    observation: &model::user_observed_entity::Model,
) -> impl Iterator<Item = &str> {
    [
        Some(observation.name.as_str()),
        observation.platform_handle.as_deref(),
        observation.platform_display_name.as_deref(),
    ]
    .into_iter()
    .flatten()
}

fn names_for(observation: &model::user_observed_entity::Model) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for surface in surfaces(observation) {
        let label = surface.trim();
        let key = normalize::key(label);
        if key.is_empty() || keys.contains(&key) {
            continue;
        }
        keys.push(key);
        labels.push(label.to_string());
    }
    labels
}

/// `identified` is a claim about evidence, so it is true the moment an identity exists — including
/// when a provisional entity absorbs an observation that carries one. A no-op when already set.
async fn mark_identified(
    conn: &sea_orm::DatabaseConnection,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    model::knowledge_entity::Entity::update_many()
        .col_expr(
            model::knowledge_entity::Column::Status,
            Expr::value(STATUS_IDENTIFIED),
        )
        .filter(model::knowledge_entity::Column::Id.eq(knowledge_entity_id))
        .filter(model::knowledge_entity::Column::Status.ne(STATUS_IDENTIFIED))
        .exec(conn)
        .await?;
    Ok(())
}

async fn attach(
    conn: &sea_orm::DatabaseConnection,
    entity_id: i32,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    model::user_observed_entity::Entity::update_many()
        .col_expr(
            model::user_observed_entity::Column::KnowledgeEntityId,
            Expr::value(Some(knowledge_entity_id)),
        )
        .filter(model::user_observed_entity::Column::Id.eq(entity_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// Fold this observation's surfaces into the entity it was linked to. Descriptions accumulate in
/// `project`, which runs once at the end of a build rather than per observation.
async fn absorb_names(
    conn: &sea_orm::DatabaseConnection,
    entity: &mut EntityState,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<bool> {
    let mut changed = false;
    for surface in surfaces(observation) {
        changed |= entity.add_name(surface);
    }
    changed |= entity.add_type(&observation.entity_type);
    if changed {
        model::knowledge_entity::ActiveModel {
            id: Set(entity.scored.id),
            alt_names: Set(entity.alt_names.clone()),
            alt_types: Set(entity.alt_types.clone()),
            ..Default::default()
        }
        .update(conn)
        .await?;
    }
    Ok(changed)
}

/// Record the observation's identifier against its entity. Returns the *other* entity that already
/// claims it, if any — the caller decides what to do, because dropping the claim silently would
/// leave two entities for one referent and no trace of the race.
///
/// The read-back is what makes this race-safe: `ON CONFLICT DO NOTHING` leaves whichever row was
/// committed first, and comparing its owner is cheaper and clearer than matching a unique-violation
/// error code.
async fn remember_identifier(
    conn: &sea_orm::DatabaseConnection,
    known: &mut HashMap<(String, String), i32>,
    knowledge_entity_id: i32,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<Option<i32>> {
    let Some((scheme, value)) = identifier_of(observation) else {
        return Ok(None);
    };
    if let Some(&existing) = known.get(&(scheme.clone(), value.clone())) {
        return Ok((existing != knowledge_entity_id).then_some(existing));
    }

    // The partial unique index refuses a second live claim to the same (scheme, id). Under a
    // concurrent writer that surfaces as a unique violation, so the run stops rather than leaving
    // two entities for one referent; turning it into a merge is phase 3 (§Repair).
    let insert = model::knowledge_evidence::Entity::insert(
        model::knowledge_evidence::ActiveModel {
            knowledge_entity_id: Set(knowledge_entity_id),
            kind: Set("claim".to_string()),
            value: Set(serde_json::json!({
                "scheme": scheme,
                "id": value,
                "handle": observation.platform_handle,
            })),
            source: Set("platform".to_string()),
            method: Set(Some("capture".to_string())),
            user_observed_entity_id: Set(Some(observation.id)),
            source_user_id: Set(Some(observation.user_id)),
            ..Default::default()
        },
    )
    .exec(conn)
    .await;

    match insert {
        Ok(_) => {
            known.insert((scheme, value), knowledge_entity_id);
            Ok(None)
        }
        // Another entity claimed this identity between the map read and the insert. The index said so;
        // report the owner so the caller can drop the duplicate instead of leaving two referents.
        Err(err) if is_unique_violation(&err) => {
            let owner = identity_owner(conn, &scheme, &value).await?;
            match owner {
                Some(owner) if owner != knowledge_entity_id => {
                    known.insert((scheme, value), owner);
                    Ok(Some(owner))
                }
                Some(_) => {
                    known.insert((scheme, value), knowledge_entity_id);
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

pub(crate) async fn identity_owner(
    conn: &sea_orm::DatabaseConnection,
    scheme: &str,
    value: &str,
) -> anyhow::Result<Option<i32>> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT knowledge_entity_id FROM knowledge_evidence \
             WHERE kind = 'claim' AND retracted_at IS NULL \
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
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<i32> {
    let description = observation.description.clone();
    // A handle is an identifier, so the row is identified from the start; anything else is a
    // hypothesis until resolution supplies an authority identifier.
    let status = if identifier_of(observation).is_some() {
        STATUS_IDENTIFIED
    } else {
        STATUS_PROVISIONAL
    };
    let model = model::knowledge_entity::ActiveModel {
        name: Set(observation.name.trim().to_string()),
        entity_type: Set(observation.entity_type.clone()),
        alt_types: Set(Vec::new()),
        description: Set(description),
        alt_descriptions: Set(Vec::new()),
        status: Set(status.to_string()),
        confidence: Set(None),
        merged_into_id: Set(None),
        alt_names: Set(names_for(observation)),
        ..Default::default()
    };
    let inserted = model::knowledge_entity::Entity::insert(model)
        .exec(conn)
        .await?;
    Ok(inserted.last_insert_id as i32)
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;

    use super::*;
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
    async fn same_name_across_captures_shares_one_core() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_first").await;
        let second = capture(&db, "link_second").await;
        observe(&db, &first, 0, "Train Dreams", Some("a novella by Denis Johnson"), "book", None, None).await;
        observe(&db, &second, 0, "train dreams", Some("a novella by Denis Johnson"), "book", None, None).await;

        let report = link(&db).await.expect("build should succeed");

        assert_eq!(report.created, 1, "{report:?}");
        assert_eq!(report.linked, 1, "{report:?}");
        assert_eq!(entity_count(&db).await, 1);
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

        let report = link(&db).await.expect("build should succeed");

        assert_eq!(report.created, 2, "{report:?}");
        assert_eq!(entity_count(&db).await, 2);
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

        let report = link(&db).await.expect("build should succeed");

        assert_eq!(report.created, 2, "{report:?}");
        assert_eq!(entity_count(&db).await, 2);
    }

    #[tokio::test]
    async fn a_second_run_does_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_idempotent").await;
        observe(&db, &first, 0, "Train Dreams", Some("a novella"), "book", None, None).await;

        let first_run = link(&db).await.expect("build should succeed");
        let second_run = link(&db).await.expect("second build should succeed");

        assert_eq!(first_run.created, 1, "{first_run:?}");
        assert_eq!(second_run.created, 0, "{second_run:?}");
        assert_eq!(second_run.linked, 0, "{second_run:?}");
        assert_eq!(second_run.already_linked, 1, "{second_run:?}");
    }

    /// Every reading is linked. An illumination can be wrong and the correct reading is often the
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

        let report = link(&db).await.expect("build should succeed");

        assert_eq!(report.stale, 1, "the older reading is counted: {report:?}");
        assert_eq!(report.created, 2, "both readings reach the graph: {report:?}");

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
    async fn a_later_run_finds_an_entity_by_its_own_name() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let first = capture(&db, "link_name_first").await;
        observe(
            &db,
            &first,
            0,
            "Francis Ford Coppola",
            Some("An American film director, producer and screenwriter"),
            "real_person",
            None,
            None,
        )
        .await;
        let first_run = link(&db).await.expect("first link");
        assert_eq!(first_run.created, 1, "{first_run:?}");

        let second = capture(&db, "link_name_second").await;
        observe(
            &db,
            &second,
            0,
            "Francis Ford Coppola",
            Some("An American film director, producer and screenwriter"),
            "real_person",
            None,
            None,
        )
        .await;
        let second_run = link(&db).await.expect("second link");

        assert_eq!(
            second_run.created, 0,
            "a repeated name must attach, not duplicate: {second_run:?}"
        );
        assert_eq!(second_run.linked, 1, "{second_run:?}");
        assert_eq!(entity::Entity::find().count(&db.conn).await.unwrap(), 1);
    }

    /// The persona case: an account's own name is the identity, so two captures that name the same
    /// person differently still share an entity. Without the identifier lookup these would be two entities,
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

        let report = link(&db).await.expect("build should succeed");

        assert_eq!(report.created, 1, "{report:?}");
        assert_eq!(report.linked_by_identifier, 1, "{report:?}");
        assert_eq!(entity_count(&db).await, 1);
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1
        );
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
        observe(&db, &first, 0, "Tsoding", None, "real_person", Some("x"), Some("tsoding")).await;
        link(&db).await.expect("build should succeed");

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

        let observation = observation::Entity::find().one(&db.conn).await.unwrap().unwrap();
        let mut known = HashMap::new();
        let claimed = remember_identifier(&db.conn, &mut known, duplicate, &observation)
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

        link(&db).await.expect("build should succeed");

        let entity = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        assert_eq!(entity.status, STATUS_IDENTIFIED);
    }
}
