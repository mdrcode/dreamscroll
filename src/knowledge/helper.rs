//! The illumination composition: what to link and annotate, and the corpus-wide resolve.
//!
//! [`KnowledgeGraph::link`] knows only about a batch of observations, and
//! [`HeuristicResolver::resolve`](crate::knowledge2::heuristic::resolver::HeuristicResolver::resolve) knows only about
//! a collection of entities. Everything that knows they arrive one illumination at a time lives
//! here: the batch is one illumination's observations, and a capture's identities are written with
//! them. The per-illumination resolve is not here: the read that turns an illumination into its
//! entity batch is the service API's join, and the task that owns that read hands the batch straight
//! to the engine.
//!
//! [`link_all_pending`] and
//! [`HeuristicResolver::resolve_all_pending`](crate::knowledge2::heuristic::resolver::HeuristicResolver::resolve_all_pending) are the same
//! compositions over the whole outstanding set — the rebuild steps, and the only places that decide
//! what a corpus run touches.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::knowledge::graph::{KnowledgeGraph, LinkReport};
use crate::model;

/// Link one illumination: its observations become entities, then their handles become identities.
///
/// The app's unit. [`KnowledgeGraph::link`] is what makes it atomic.
pub async fn link_illumination(
    graph: &KnowledgeGraph,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<LinkReport> {
    // An unloaded relation would read as an illumination with no observations, and linking nothing
    // would look exactly like success.
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

    graph.link(&observations).await
}

/// What a batch of links did, summed over every illumination it was handed.
///
/// The per-illumination [`LinkReport`] answers "what did this reading write"; this answers the run:
/// how many observations were considered, and how the totals add up.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkAllReport {
    /// Observations in the illuminations that still had work.
    pub considered: usize,
    /// Of those, the ones already linked before the run.
    pub already_linked: usize,
    pub attached: usize,
    pub created: usize,
    /// Pending observations the linker returned no decision for.
    pub left_pending: usize,
    /// Identity annotations a standing veto refused.
    pub vetoed: usize,
}

impl LinkAllReport {
    fn add(&mut self, part: LinkReport) {
        self.attached += part.attached;
        self.created += part.created;
        self.left_pending += part.left_pending;
        self.vetoed += part.vetoed;
    }
}

/// Link every illumination that still has unlinked observations.
///
/// After a reset this rebuilds the entire graph. The order is by illumination id, because linking
/// one changes what a later one finds, so the order decides who owns a contested entity.
pub async fn link_all_pending(db: &DbHandle) -> anyhow::Result<LinkAllReport> {
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

    let graph = KnowledgeGraph::with_heuristic_linking(db);
    let mut report = LinkAllReport::default();
    for illumination in &illuminations {
        for observation in illumination.observations.iter() {
            report.considered += 1;
            if observation.knowledge_entity_id.is_some() {
                report.already_linked += 1;
            }
        }
        report.add(link_illumination(&graph, illumination).await?);
    }
    Ok(report)
}


