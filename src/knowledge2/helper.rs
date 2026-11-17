//! The first end-to-end use case: link illuminations into the v2 graph.
//!
//! The graph has no linker and no blocker. This helper is the external executor: it hands the
//! pending observations to a v2 [`Linker`] and applies the actions. The graph projects natively,
//! including the blocking index, so the next illumination's SQL blocker sees the same graph v1 would
//! have built.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, QueryFilter, QueryOrder, Statement, Value,
};

use crate::database::DbHandle;
use crate::knowledge::blocker::{Blocker as V1Blocker, SqlBlocker};
use crate::knowledge2::action::KnowledgeAction;
use crate::knowledge2::graph::KnowledgeGraph2;
use crate::knowledge2::ids::{EntityOriginId, ObservationId};
use crate::knowledge2::linker::{Blocker, HeuristicPlanner, IdentityHolder, Linker};
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

/// The external executor: planner and graph.
#[derive(Clone)]
pub struct LinkHelper {
    db: DbHandle,
    graph: KnowledgeGraph2,
    blocker: SqlBlocker,
    linker: HeuristicPlanner,
}

impl LinkHelper {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            graph: KnowledgeGraph2::new(db),
            blocker: SqlBlocker::new(db),
            linker: HeuristicPlanner::new(),
        }
    }

    /// Link one illumination: decide, apply, project.
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
        let considered = observations.len();
        let pending: Vec<model::user_observed_entity::Model> = observations
            .into_iter()
            .filter(|row| row.knowledge_entity_id.is_none())
            .collect();
        let already_linked = considered - pending.len();

        let actions = self.linker.plan(&self.blocker, self, &pending).await?;
        let created = actions
            .iter()
            .filter(|action| matches!(action, KnowledgeAction::CreateEntity { .. }))
            .count();
        let attached = actions.len() - created;

        if !actions.is_empty() {
            let report = self.graph.proc(&actions).await?;
            anyhow::ensure!(
                report.refused.is_none(),
                "linking refused: {:?}",
                report.refused
            );
        }

        Ok(LinkReport {
            considered,
            already_linked,
            created,
            attached,
            left_pending: pending.len().saturating_sub(actions.len()),
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
}

/// The v1 SQL blocker is the recall implementation; the v2 planner sees it through v2's trait.
#[async_trait::async_trait]
impl Blocker for SqlBlocker {
    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        V1Blocker::candidates(self, observation).await
    }
}

const HOLDER: &str = r#"
SELECT e.origin_observation_id
  FROM knowledge_identities i
  JOIN knowledge_entities e ON e.id = i.knowledge_entity_id
 WHERE e.merged_into_id IS NULL
   AND i.scheme = $1
   AND i.external_id = $2
 ORDER BY e.confidence DESC NULLS LAST, e.id
 LIMIT 1
"#;

/// The one read the planner asks, answered from v2's own tables.
///
/// The ordering is the resolver's rule — most substantiated first, `NULL` last, then id — so the
/// decision matches the one knowledge1's `Evidence` would have given.
#[async_trait::async_trait]
impl IdentityHolder for LinkHelper {
    async fn holder(
        &self,
        scheme: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<EntityOriginId>> {
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
        let Some(row) = row else {
            return Ok(None);
        };
        let origin: Option<i32> = row.try_get_by_index(0)?;
        Ok(origin.map(|id| EntityOriginId(ObservationId(id))))
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
    use sea_orm::{EntityTrait, PaginatorTrait};

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

        assert_eq!(
            crate::knowledge2::shape::snapshot(&left).await.unwrap(),
            crate::knowledge2::shape::snapshot(&right).await.unwrap(),
        );
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

}
