//! The illumination composition: what to link, and what to assert.
//!
//! [`KnowledgeGraph::link`] knows only about a collection of observations. Everything that knows they
//! arrive one illumination at a time lives here: the batch is one illumination's observations, and a
//! capture's claims are written after its entities exist, because a claim names both the identity the
//! observation asserted and the entity it landed on.
//!
//! [`link_all_pending`] is the same composition over every illumination that still has work — the
//! rebuild step, and the only place that decides which illuminations a run links.

use std::collections::HashMap;

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::knowledge::evidence::{ClaimDraft, IdentityKey, Stored};
use crate::knowledge::graph::{KnowledgeGraph, LinkReport};
use crate::model;

/// Link one illumination: its observations become entities, then their handles become claims.
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

    let (mut report, linked) = graph.link(&observations).await?;
    report.vetoed = assert_claims(graph, &observations, &linked).await?;
    Ok(report)
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
    /// Claims a standing veto refused.
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

/// Assert the handle each linked observation showed, and project where it landed.
///
/// The only place a capture's identity and claim are written. Returns how many a standing veto
/// refused.
async fn assert_claims(
    graph: &KnowledgeGraph,
    observations: &[model::user_observed_entity::Model],
    linked: &HashMap<i32, i32>,
) -> anyhow::Result<usize> {
    let vetoes = graph.vetoed().await?;
    let mut vetoed = 0;
    for observation in observations {
        let Some(entity) = linked.get(&observation.id).copied() else {
            continue;
        };
        if let Some(key) = IdentityKey::from_platform_link(observation) {
            let identity = graph.identity_for_link(&key).await?;
            if graph
                .assert(
                    ClaimDraft::capture(entity, observation, &key, identity),
                    &vetoes,
                )
                .await?
                == Stored::Vetoed
            {
                vetoed += 1;
            }
        }
        // After the claims: status and confidence are functions of them.
        graph.update_projection(entity).await?;
    }
    Ok(vetoed)
}
