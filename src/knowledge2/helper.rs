//! The first end-to-end use case: link illuminations into the v2 graph.
//!
//! The graph has no linker and no blocker. This helper is the external executor: it hands the
//! pending observations to the v2 heuristic planner and applies the mutations. The graph projects natively,
//! including the blocking index, so the next illumination's SQL blocker sees the same graph v1 would
//! have built.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::knowledge2::mutation::GraphMutation;
use crate::knowledge2::blocker::SqlBlocker;
use crate::knowledge2::graph::KnowledgeGraph2;
use crate::knowledge2::heuristic::HeuristicLinker;
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

/// The external executor: linker and graph.
#[derive(Clone)]
pub struct LinkHelper {
    graph: KnowledgeGraph2,
    blocker: SqlBlocker,
    linker: HeuristicLinker,
}

impl LinkHelper {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            graph: KnowledgeGraph2::new(db),
            blocker: SqlBlocker::new(db),
            linker: HeuristicLinker::new(),
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

        let selector = self.graph.selector();
        let mutations = self
            .linker
            .plan(&self.blocker, &selector, &pending)
            .await?;
        let created = mutations
            .iter()
            .filter(|mutation| matches!(mutation, GraphMutation::CreateEntity { .. }))
            .count();
        let attached = mutations.len() - created;

        if !mutations.is_empty() {
            let report = self.graph.proc(&mutations).await?;
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
            left_pending: pending.len().saturating_sub(mutations.len()),
        })
    }

    /// Link every illumination that still has an unlinked observation.
    ///
    /// One illumination, one `proc`, one projection pass — the same boundary as v1. The order is by
    /// illumination id, because linking one changes what a later one finds.
    pub async fn link_all_pending(&self, db: &DbHandle) -> anyhow::Result<LinkAllReport> {
        let unlinked = Query::select()
            .column(model::user_observed_entity::Column::IlluminationId)
            .from(model::user_observed_entity::Entity)
            .and_where(model::user_observed_entity::Column::KnowledgeEntityId.is_null())
            .to_owned();
        let illuminations = model::illumination::Entity::load()
            .filter(Expr::col(model::illumination::Column::Id).in_subquery(unlinked))
            .with(model::user_observed_entity::Entity)
            .order_by_asc(model::illumination::Column::Id)
            .all(&db.conn)
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

/// Link one illumination through a fresh helper. The v1-shaped entry point.
pub async fn link_illumination(
    db: &DbHandle,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<LinkReport> {
    LinkHelper::new(db).link_illumination(illumination).await
}

/// Link every pending illumination through one helper. The v1-shaped entry point.
pub async fn link_all_pending(db: &DbHandle) -> anyhow::Result<LinkAllReport> {
    LinkHelper::new(db).link_all_pending(db).await
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

}
