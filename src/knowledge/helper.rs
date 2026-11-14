//! The illumination composition: what to link and annotate, and the corpus-wide resolve.
//!
//! [`KnowledgeGraph::link`] knows only about a collection of observations, and
//! [`IdentityEngine::resolve`] knows only about a collection of entities. Everything that knows they
//! arrive one illumination at a time lives here: the batch is one illumination's observations, a
//! capture's identities are written after its entities exist. The per-illumination resolve is not
//! here: the read that turns an illumination into its entity batch is the service API's join, and
//! the task that owns that read hands the batch straight to the engine.
//!
//! [`link_all_pending`] and [`resolve_all_pending`] are the same compositions over the whole
//! outstanding set — the rebuild steps, and the only places that decide what a corpus run touches.

use std::collections::{BTreeMap, HashMap};

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::fetch::FetchCache;
use crate::knowledge::evidence::{
    Identity, IdentitySelection, PLATFORM_LINK_PROVIDER_ID, platform_link,
};
use crate::knowledge::graph::{KnowledgeGraph, LinkReport};
use crate::knowledge::identity::{IdentityEngine, ResolutionReport};
use crate::model;

/// Link one illumination: its observations become entities, then their handles become identities.
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
    report.vetoed = write_observation_identities(graph, &observations, &linked).await?;
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

/// Resolve every entity still pending for any configured provider.
///
/// The engine receives the whole pending set in one call, so the wavefront still fills its batches.
/// This is the rebuild/drain path; a live capture resolves through the service API's
/// illumination read, which also reports whether linking finished.
pub async fn resolve_all_pending(
    db: &DbHandle,
    engine: &IdentityEngine,
    cache: &FetchCache,
    allowance: Option<&crate::fetch::Allowance>,
    limit: Option<usize>,
    newest_first: bool,
) -> anyhow::Result<ResolutionReport> {
    let selection = IdentitySelection::from_handle(db);
    // One provider's pending set, then the union: an entity Wikidata answered is still pending for
    // IMDb. The limit is global, so a second resolver does not silently double the run.
    let mut pending: BTreeMap<i32, model::knowledge_entity::Model> = BTreeMap::new();
    for provider in engine.providers() {
        for entity in selection
            .awaiting_resolver(&[provider], None, newest_first)
            .await?
        {
            pending.entry(entity.id).or_insert(entity);
        }
    }
    let mut entities: Vec<model::knowledge_entity::Model> = if newest_first {
        pending.into_values().rev().collect()
    } else {
        pending.into_values().collect()
    };
    if let Some(limit) = limit {
        entities.truncate(limit);
    }

    // An online corpus pass is the maintenance moment: drop answers that aged out long ago. A
    // per-illumination resolve does not do this — it must stay quick.
    if allowance.is_some() {
        let pruned = cache.prune().await?;
        if pruned > 0 {
            tracing::info!(pruned, "Pruned expired fetch cache rows");
        }
    }

    engine.resolve(&entities, cache, allowance).await
}

/// Annotate each linked observation's handle, and project where it landed.
///
/// The only place a capture's identity is written. Returns how many a standing veto refused.
async fn write_observation_identities(
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
        if let Some(link) = platform_link(observation)
            && let Ok(mut identity) = Identity::try_from(&link)
        {
            identity.observation_id = Some(observation.id);
            vetoed += graph
                .add_identity(entity, PLATFORM_LINK_PROVIDER_ID, &identity, &vetoes)
                .await?
                .vetoed;
        }
        // After the annotations: status and confidence are functions of them.
        graph.update_projection(entity).await?;
    }
    Ok(vetoed)
}
