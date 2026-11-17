//! The first end-to-end use case: link illuminations into the v2 graph.
//!
//! The graph has no linker and no blocker. This helper is the external executor: it asks knowledge1's
//! [`SqlBlocker`] for candidates and its [`HeuristicLinker`] for decisions, turns those decisions into
//! [`KnowledgeAction`]s, and hands them to [`KnowledgeGraph2::proc`].
//!
//! It also does the job the graph refuses to do: after each committed call it runs knowledge1's
//! projection, which fills the entity's derived columns and its `knowledge_blocking_index` row. That
//! keeps the next illumination's blocker looking at the same graph v1 would have built.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder, Statement,
    Value,
};

use crate::database::DbHandle;
use crate::knowledge::KnowledgeGraph;
use crate::knowledge::blocker::SqlBlocker;
use crate::knowledge::linker::heuristic::HeuristicLinker;
use crate::knowledge::linker::{Action, IdentityHolder, Linker};
use crate::knowledge2::action::KnowledgeAction;
use crate::knowledge2::graph::KnowledgeGraph2;
use crate::knowledge2::identity::Identity;
use crate::knowledge2::ids::{EntityOriginId, ObservationId};
use crate::model;

/// One illumination's link.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkReport {
    pub considered: usize,
    pub already_linked: usize,
    pub created: usize,
    pub attached: usize,
    pub left_pending: usize,
}

/// Every illumination that still had work.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LinkAllReport {
    pub illuminations: usize,
    pub considered: usize,
    pub already_linked: usize,
    pub created: usize,
    pub attached: usize,
    pub left_pending: usize,
}

/// The external executor: blocker, linker, graph, and the shared projection pass.
#[derive(Clone)]
pub struct LinkHelper {
    db: DbHandle,
    graph: KnowledgeGraph2,
    projector: KnowledgeGraph,
    blocker: SqlBlocker,
    linker: HeuristicLinker,
}

impl LinkHelper {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            graph: KnowledgeGraph2::new(db),
            // knowledge1, used only as the shared projection/index pass so v1 and v2 are comparable.
            projector: KnowledgeGraph::new(db),
            blocker: SqlBlocker::new(db),
            linker: HeuristicLinker::new(),
        }
    }

    /// Link one illumination: decide, convert, apply, project.
    pub async fn link_illumination(
        &self,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<LinkReport> {
        // An unloaded relation would read as an illumination with no observations, and linking
        // nothing would look exactly like success.
        anyhow::ensure!(
            illumination.observations.is_loaded(),
            "illumination {} was handed to link_illumination with no observations loaded; fetch it \
             with Entity::load().with(model::user_observed_entity::Entity)",
            illumination.id
        );

        let observations: Vec<model::user_observed_entity::Model> = illumination
            .observations
            .iter()
            .map(|row| row.clone().into())
            .collect();
        let pending = observations
            .iter()
            .filter(|row| row.knowledge_entity_id.is_none())
            .count();

        let decisions = self
            .linker
            .decide_link(&self.blocker, self, &observations)
            .await?;

        let mut actions = Vec::with_capacity(decisions.len());
        let mut created = 0;
        let mut attached = 0;
        for decision in &decisions {
            let with = Identity::from_observation(&decision.observation);
            match decision.action {
                Action::Create => {
                    created += 1;
                    actions.push(KnowledgeAction::CreateEntity {
                        from: ObservationId(decision.observation.id),
                        with,
                    });
                }
                Action::Attach(entity) => {
                    attached += 1;
                    actions.push(KnowledgeAction::AttachObservation {
                        observation: ObservationId(decision.observation.id),
                        entity: self.origin_of(entity).await?,
                        with,
                        why: None,
                    });
                }
            }
        }

        if !actions.is_empty() {
            let report = self.graph.proc(&actions).await?;
            anyhow::ensure!(
                report.refused.is_none(),
                "linking refused: {:?}",
                report.refused
            );
            for entity in &report.touched {
                self.projector.update_projection(*entity).await?;
            }
        }

        Ok(LinkReport {
            considered: observations.len(),
            already_linked: observations.len() - pending,
            created,
            attached,
            left_pending: pending.saturating_sub(decisions.len()),
        })
    }

    /// Link every illumination that still has an unlinked observation.
    ///
    /// One illumination, one `proc`, one projection pass — the same boundary as v1. The order is by
    /// illumination id, because linking one changes what a later one finds.
    pub async fn link_all_pending(&self) -> anyhow::Result<LinkAllReport> {
        let unlinked = Query::select()
            .column(model::user_observed_entity::Column::IlluminationId)
            .from(model::user_observed_entity::Entity)
            .and_where(model::user_observed_entity::Column::KnowledgeEntityId.is_null())
            .to_owned();
        let illuminations = model::illumination::Entity::load()
            .filter(Expr::col(model::illumination::Column::Id).in_subquery(unlinked))
            .with(model::user_observed_entity::Entity)
            .order_by_asc(model::illumination::Column::Id)
            .all(&self.db.conn)
            .await?;

        let mut report = LinkAllReport::default();
        for illumination in &illuminations {
            let part = self.link_illumination(illumination).await?;
            report.illuminations += 1;
            report.considered += part.considered;
            report.already_linked += part.already_linked;
            report.created += part.created;
            report.attached += part.attached;
            report.left_pending += part.left_pending;
        }
        Ok(report)
    }

    /// The linker speaks entity ids; the graph speaks origins.
    async fn origin_of(&self, entity: i32) -> anyhow::Result<EntityOriginId> {
        let row = model::knowledge_entity::Entity::find_by_id(entity)
            .one(&self.db.conn)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the linker chose entity {entity}, which does not exist"))?;
        let origin = row
            .origin_observation_id
            .ok_or_else(|| anyhow::anyhow!("entity {entity} has no origin_observation_id"))?;
        Ok(EntityOriginId(ObservationId(origin)))
    }
}

const HOLDER: &str = r#"
SELECT i.knowledge_entity_id
  FROM knowledge_identities i
  JOIN knowledge_entities e ON e.id = i.knowledge_entity_id
 WHERE e.merged_into_id IS NULL
   AND i.scheme = $1
   AND i.external_id = $2
 ORDER BY e.confidence DESC NULLS LAST, e.id
 LIMIT 1
"#;

/// The one read the linker asks, answered from v2's own tables.
///
/// The ordering is the resolver's rule — most substantiated first, `NULL` last, then id — so the
/// decision matches the one knowledge1's `Evidence` would have given.
#[async_trait::async_trait]
impl IdentityHolder for LinkHelper {
    async fn holder(&self, scheme: &str, external_id: &str) -> anyhow::Result<Option<i32>> {
        let row = self
            .db
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                HOLDER,
                vec![
                    Value::String(Some(scheme.to_string())),
                    Value::String(Some(external_id.to_string())),
                ],
            ))
            .await?;
        Ok(row
            .map(|row| row.try_get_by_index::<i32>(0))
            .transpose()?)
    }
}

/// Link one illumination through a fresh helper. The v1-shaped entry point.
pub async fn link_illumination(
    db: &DbHandle,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<LinkReport> {
    LinkHelper::new(db).link_illumination(illumination).await
}

/// Link every pending illumination through one helper. The v1-shaped entry point.
pub async fn link_all_pending(db: &DbHandle) -> anyhow::Result<LinkAllReport> {
    LinkHelper::new(db).link_all_pending().await
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;

    use super::*;
    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn links_two_illuminations_that_share_a_handle() {
        db_or_skip!(db);
        let first = fixtures::Capture::new(&db, "k2_link_first").await;
        let second = fixtures::Capture::new(&db, "k2_link_second").await;

        fixtures::observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            Some("instagram"),
            Some("traindreams"),
        )
        .await;
        fixtures::observe(
            &db,
            &second,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            Some("instagram"),
            Some("traindreams"),
        )
        .await;

        let report = link_all_pending(&db).await.unwrap();

        assert_eq!(report.illuminations, 2);
        assert_eq!(report.created, 1);
        assert_eq!(report.attached, 1);
        assert_eq!(report.left_pending, 0);
        assert_eq!(
            model::knowledge_entity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            model::user_observed_entity::Entity::find()
                .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
                .count(&db.conn)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn the_projection_pass_keeps_the_blocker_current() {
        db_or_skip!(db);
        let first = fixtures::Capture::new(&db, "k2_index_first").await;
        let second = fixtures::Capture::new(&db, "k2_index_second").await;

        fixtures::observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            None,
            None,
        )
        .await;
        fixtures::observe(
            &db,
            &second,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            None,
            None,
        )
        .await;

        let report = link_all_pending(&db).await.unwrap();

        assert_eq!(report.created, 1);
        assert_eq!(report.attached, 1);
        assert_eq!(
            model::knowledge_entity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1
        );
    }

    /// The point of the helper: same fixtures, v1 and v2, same shape.
    #[tokio::test]
    async fn v1_and_v2_build_the_same_graph() {
        let Some(v1) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let Some(v2) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let left = v1.handle();
        let right = v2.handle();

        seed(&left).await;
        seed(&right).await;

        crate::knowledge::link_all_pending(&left).await.unwrap();
        super::link_all_pending(&right).await.unwrap();

        assert_eq!(shape(&left).await, shape(&right).await);
    }

    async fn seed(db: &DbHandle) {
        let first = fixtures::Capture::new(db, "compare_first").await;
        let second = fixtures::Capture::new(db, "compare_second").await;
        fixtures::observe(
            db,
            &first,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            Some("instagram"),
            Some("traindreams"),
        )
        .await;
        fixtures::observe(
            db,
            &second,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            Some("instagram"),
            Some("traindreams"),
        )
        .await;
    }

    /// Everything that should be equal across two isolated schemas. Ids, timestamps, and user
    /// ownership are deliberately excluded; the graph's shape is what we are comparing.
    async fn shape(db: &DbHandle) -> (Vec<String>, Vec<String>, Vec<Vec<String>>) {
        let mut entities: Vec<String> = model::knowledge_entity::Entity::find()
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|entity| {
                format!(
                    "name={:?} type={:?} alt_names={:?} alt_types={:?} \
                     description={:?} alt_descriptions={:?} search={:?} alt_search={:?} \
                     status={:?} confidence={} platform={:?} handle={:?} display={:?}",
                    entity.name,
                    entity.entity_type,
                    entity.alt_names,
                    entity.alt_types,
                    entity.description,
                    entity.alt_descriptions,
                    entity.search_query,
                    entity.alt_search_queries,
                    entity.status,
                    entity.confidence.is_some(),
                    entity.platform,
                    entity.platform_handle,
                    entity.platform_display_name,
                )
            })
            .collect();
        entities.sort();

        let mut identities: Vec<String> = model::knowledge_identity::Entity::find()
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|identity| {
                format!(
                    "{:?}",
                    (
                        identity.scheme,
                        identity.external_id,
                        identity.provider,
                        identity.method,
                        identity.confidence.is_some(),
                        identity.label,
                        identity.description,
                        identity.classes,
                        identity.user_observed_entity_id.is_some(),
                    )
                )
            })
            .collect();
        identities.sort();

        let mut groups: std::collections::BTreeMap<i32, Vec<String>> =
            std::collections::BTreeMap::new();
        for observation in model::user_observed_entity::Entity::find()
            .all(&db.conn)
            .await
            .unwrap()
        {
            if let Some(entity) = observation.knowledge_entity_id {
                groups.entry(entity).or_default().push(observation.name);
            }
        }
        let mut memberships: Vec<Vec<String>> = groups
            .into_values()
            .map(|mut names| {
                names.sort();
                names
            })
            .collect();
        memberships.sort();

        (entities, identities, memberships)
    }
}
