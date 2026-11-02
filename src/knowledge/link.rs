//! Link every observation to an entity, creating one when nothing is close enough.
//!
//! No network. Blocking is an in-memory index over the entities (`~2k` rows per user, so a scan is
//! cheaper than a query plan), identifiers are a decisive lookup, the decision is [`score`], and
//! every write is idempotent: a second run finds nothing to do because every observation already has
//! a `knowledge_entity_id`.

use std::collections::HashSet;

use anyhow::anyhow;

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, DbErr, EntityTrait,
    QueryFilter, QueryOrder, RuntimeErr, Set, Statement,
};

use super::{
    blocking::Blocker,
    heuristic::{HeuristicLinker, NameBlocker},
    linker::{self, Linker},
    normalize,
};
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
    /// Observations whose best candidate was already taken by a sibling from the same illumination.
    pub vetoed: u32,
    pub created: usize,
}

impl Report {
    /// Fold one run's counts into this one. Derived rather than stored (`duplicate_rate`), and the
    /// two entry points differ only in how many runs there are.
    pub fn update(&mut self, part: Report) {
        self.considered += part.considered;
        self.already_linked += part.already_linked;
        self.stale += part.stale;
        self.linked_by_identifier += part.linked_by_identifier;
        self.linked += part.linked;
        self.created += part.created;
        self.vetoed += part.vetoed;
    }
}

/// Add an observed type. Returns whether anything changed: an entity that has accepted `brand`
/// must accept it for the rest of the run.
fn absorb_type(entity: &mut model::knowledge_entity::Model, entity_type: &str) -> bool {
    if entity_type.is_empty() || entity.alt_types.iter().any(|t| t == entity_type) {
        return false;
    }
    entity.alt_types.push(entity_type.to_string());
    true
}

/// Add a surface. Returns whether anything changed, so a caller can skip the write.
fn absorb_surface(entity: &mut model::knowledge_entity::Model, surface: &str) -> bool {
    let key = normalize::key(surface);
    if key.is_empty() || entity.surfaces().any(|seen| normalize::key(seen) == key) {
        return false;
    }
    entity.alt_names.push(surface.trim().to_string());
    true
}

/// The `(scheme, value)` an observation carries, if it carries one. `scheme` is the platform; the
/// same shape covers `wikidata` → `Q…`, `imdb` → `tt…`, and every authority after them.
fn identifier_of(observation: &model::user_observed_entity::Model) -> Option<(String, String)> {
    let platform = observation.platform.as_deref()?;
    let handle = observation.platform_handle.as_deref()?;
    Some((platform.to_lowercase(), normalize::key(handle)))
}

/// Link one illumination: its observations, against the whole graph.
///
/// Self-contained — it loads the corpus it needs — so a worker holding one illumination can call it
/// without a rebuilding pass around it.
/// Link one illumination against the graph.
///
/// The blocker is passed in because it *is* the corpus: building it costs a pass over every live
/// entity, so a caller doing more than one illumination builds it once. A worker handling a single
/// illumination builds one and calls this — [`link_db`] is the same call in a loop.
pub async fn link(
    db: &database::DbHandle,
    blocker: &mut dyn Blocker,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<Report> {
    // Which reading of this capture is current, for the report only. Earlier readings are linked
    // too: an illumination can be wrong — a re-read of one image once described a different film —
    // so the graph holds what every reading said.
    let current = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::CaptureId.eq(illumination.capture_id))
        .order_by_desc(model::user_observed_entity::Column::IlluminationId)
        .one(&db.conn)
        .await?
        .map(|row| row.illumination_id);

    // The relation, materialised by `Entity::load().with(observation::Entity)`.
    let mut observations: Vec<model::user_observed_entity::Model> = illumination
        .observations
        .iter()
        .map(|row| row.clone().into())
        .collect();
    let mut report = Report {
        considered: observations.len(),
        ..Default::default()
    };
    for observation in &observations {
        if observation.knowledge_entity_id.is_some() {
            report.already_linked += 1;
            continue;
        }
        if current != Some(observation.illumination_id) {
            report.stale += 1;
        }
    }

    // Entities this reading changes, and so must re-project before the next reading sees them.
    let mut touched: HashSet<i32> = HashSet::new();

    // Identity first: decisive, and no scoring. Marking the observation as linked is also how the
    // linker learns the entity is spoken for in this reading.
    for observation in observations.iter_mut() {
        if observation.knowledge_entity_id.is_some() {
            continue;
        }
        let Some((scheme, value)) = identifier_of(observation) else {
            continue;
        };
        let Some(knowledge_entity_id) = identity_owner(&db.conn, &scheme, &value).await? else {
            continue;
        };
        attach(&db.conn, observation.id, knowledge_entity_id).await?;
        observation.knowledge_entity_id = Some(knowledge_entity_id);
        if let Some(entity) = absorb_observation(&db.conn, knowledge_entity_id, observation).await?
        {
            blocker.changed(&entity);
            touched.insert(knowledge_entity_id);
        }
        report.linked_by_identifier += 1;
    }

    // The reading as a set, already-linked siblings included: they are what makes the veto
    // expressible, and what lets a linker see more than one thing at a time.
    let reading: model::illumination::Model = illumination.clone().into();
    let result = HeuristicLinker
        .link_illuminated_observations(blocker, &reading, &observations)
        .await;
    report.vetoed += result.vetoed;

    for (observation_id, decision) in result.decisions {
        let observation = observations
            .iter()
            .find(|row| row.id == observation_id)
            .ok_or_else(|| {
                anyhow!("the linker decided about observation {observation_id}, which is not in this reading")
            })?;
        if observation.knowledge_entity_id.is_some() {
            anyhow::bail!(
                "the linker decided about observation {observation_id}, which is already linked"
            );
        }

        match decision {
            linker::Decision::Attach { entity, .. } => {
                attach(&db.conn, observation.id, entity).await?;
                if let Some(absorbed) = absorb_observation(&db.conn, entity, observation).await? {
                    blocker.changed(&absorbed);
                }
                let has_identity = identifier_of(observation).is_some();
                if let Some(owner) = remember_identifier(&db.conn, entity, observation).await? {
                    anyhow::bail!(
                        "entities {entity} and {owner} both claim the same identifier; \
                         merging is not implemented yet (plan/knowledge-canonical-entities.md §Repair)"
                    );
                }
                if has_identity {
                    // The entity may have been created without an identity and is only now
                    // acquiring one. `status` is a projection of evidence, so it moves with it.
                    mark_identified(&db.conn, entity).await?;
                }
                touched.insert(entity);
                report.linked += 1;
            }
            linker::Decision::Create { .. } => {
                let entity = create(&db.conn, observation).await?;
                let knowledge_entity_id = entity.id;

                // A concurrent worker may have claimed this identifier since the map was read. The
                // new entity is then a duplicate that owns nothing yet: link to the real owner and
                // forget it.
                if let Some(owner) =
                    remember_identifier(&db.conn, knowledge_entity_id, observation).await?
                {
                    attach(&db.conn, observation.id, owner).await?;
                    model::knowledge_entity::Entity::delete_by_id(knowledge_entity_id)
                        .exec(&db.conn)
                        .await?;
                    if let Some(absorbed) = absorb_observation(&db.conn, owner, observation).await?
                    {
                        blocker.changed(&absorbed);
                    }
                    touched.insert(owner);
                    report.linked_by_identifier += 1;
                    continue;
                }

                blocker.changed(&entity);
                attach(&db.conn, observation.id, knowledge_entity_id).await?;
                touched.insert(knowledge_entity_id);
                report.created += 1;
            }
        }
    }

    // Projection is a function of the members, so it runs after this reading's writes.
    for knowledge_entity_id in &touched {
        crate::knowledge::project::project(&db.conn, *knowledge_entity_id).await?;
    }

    Ok(report)
}

/// Link every illumination that still has unlinked observations: the whole graph, rebuilt.
pub async fn link_db(db: &database::DbHandle) -> anyhow::Result<Report> {
    let mut blocker = NameBlocker::new(db).await?;

    let unlinked_obs = Query::select()
        .column(model::user_observed_entity::Column::IlluminationId)
        .from(model::user_observed_entity::Entity)
        .and_where(model::user_observed_entity::Column::KnowledgeEntityId.is_null())
        .to_owned();

    let illuminations = model::illumination::Entity::load()
        .filter(Expr::col(model::illumination::Column::Id).in_subquery(unlinked_obs))
        .with(model::user_observed_entity::Entity)
        .all(&db.conn)
        .await?;

    let mut report = Report::default();
    for illumination in &illuminations {
        let part = link(db, &mut blocker, illumination).await?;

        report.update(part);
    }

    Ok(report)
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

/// Fold everything one observation contributes into the entity it was linked to: its surfaces (so a
/// later observation can find the entity by any of them) and its type. Descriptions accumulate in
/// `project`, which runs once at the end of a build rather than per observation.
/// Fold what an observation knows into its entity. Returns the updated row when anything changed,
/// so the caller can hand it to the blocker.
///
/// Read here rather than passed in: the blocker owns the corpus, so the executor keeps no parallel
/// copy of the entities and cannot hold one stale.
async fn absorb_observation(
    conn: &sea_orm::DatabaseConnection,
    knowledge_entity_id: i32,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
    let Some(mut entity) = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?
    else {
        return Ok(None);
    };

    let mut changed = false;
    for surface in surfaces(observation) {
        changed |= absorb_surface(&mut entity, surface);
    }
    changed |= absorb_type(&mut entity, &observation.entity_type);
    if !changed {
        return Ok(None);
    }
    model::knowledge_entity::ActiveModel {
        id: Set(entity.id),
        alt_names: Set(entity.alt_names.clone()),
        alt_types: Set(entity.alt_types.clone()),
        ..Default::default()
    }
    .update(conn)
    .await?;
    Ok(Some(entity))
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
    knowledge_entity_id: i32,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<Option<i32>> {
    let Some((scheme, value)) = identifier_of(observation) else {
        return Ok(None);
    };
    // Asked rather than cached: the claim is one row in one index, and a map held across a rebuild
    // is the same data with a staler answer.
    if let Some(existing) = identity_owner(conn, &scheme, &value).await? {
        return Ok((existing != knowledge_entity_id).then_some(existing));
    }

    // The partial unique index refuses a second live claim to the same (scheme, id). Under a
    // concurrent writer that surfaces as a unique violation, so the run stops rather than leaving
    // two entities for one referent; turning it into a merge is phase 3 (§Repair).
    let insert =
        model::knowledge_evidence::Entity::insert(model::knowledge_evidence::ActiveModel {
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
        })
        .exec(conn)
        .await;

    match insert {
        Ok(_) => Ok(None),
        // Another entity claimed this identity between the map read and the insert. The index said so;
        // report the owner so the caller can drop the duplicate instead of leaving two referents.
        Err(err) if is_unique_violation(&err) => {
            let owner = identity_owner(conn, &scheme, &value).await?;
            match owner {
                Some(owner) if owner != knowledge_entity_id => Ok(Some(owner)),
                Some(_) => Ok(None),
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
) -> anyhow::Result<model::knowledge_entity::Model> {
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
    Ok(model::knowledge_entity::Entity::insert(model)
        .exec_with_returning(conn)
        .await?)
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

        let report = link_db(&db).await.expect("build should succeed");

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

        let first_run = link_db(&db).await.expect("build should succeed");
        let second_run = link_db(&db).await.expect("second build should succeed");

        assert_eq!(first_run.created, 1, "{first_run:?}");
        assert_eq!(second_run.created, 0, "{second_run:?}");
        assert_eq!(second_run.linked, 0, "{second_run:?}");
        assert_eq!(
            second_run.considered, 0,
            "no illumination still needs linking: {second_run:?}"
        );
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

        let report = link_db(&db).await.expect("build should succeed");

        assert_eq!(report.stale, 1, "the older reading is counted: {report:?}");
        assert_eq!(
            report.created, 2,
            "both readings reach the graph: {report:?}"
        );

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

        let report = link_db(&db).await.expect("build should succeed");

        assert_eq!(report.created, 1, "{report:?}");
        assert_eq!(report.linked_by_identifier, 1, "{report:?}");
        assert_eq!(entity_count(&db).await, 1);
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
        link_db(&db).await.expect("build should succeed");

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
        let claimed = remember_identifier(&db.conn, duplicate, &observation)
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

        link_db(&db).await.expect("build should succeed");

        let entity = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        assert_eq!(entity.status, STATUS_IDENTIFIED);
    }
}
