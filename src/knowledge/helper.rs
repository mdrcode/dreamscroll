//! The illumination composition: what to link, what to claim, and what to resolve.
//!
//! [`KnowledgeGraph::link`] knows only about a collection of observations, and
//! [`ResolutionEngine::resolve`] knows only about a collection of entities. Everything that knows they
//! arrive one illumination at a time lives here: the batch is one illumination's observations, a
//! capture's claims are written after its entities exist, and the entities are then offered to the
//! authorities.
//!
//! [`link_all_pending`] and [`resolve_all_pending`] are the same compositions over the whole
//! outstanding set — the rebuild steps, and the only places that decide what a corpus run touches.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::fetch::FetchCache;
use crate::knowledge::evidence::{
    ClaimDraft, ClaimOutcome, ClaimSelection, IdentityDraft, IdentitySettlement, platform_link,
};
use crate::knowledge::graph::{KnowledgeGraph, LinkReport};
use crate::knowledge::identity::{ResolutionEngine, ResolutionReport};
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
    report.vetoed = write_capture_claims(graph, &observations, &linked).await?;
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

/// Resolve the entities one illumination references.
///
/// The illumination is already linked; an unlinked observation is a pipeline error, not a silent
/// skip. The engine receives one batch so a resolver's wavefront can span the whole illumination.
pub async fn resolve_illumination(
    db: &DbHandle,
    engine: &ResolutionEngine,
    illumination: &model::illumination::ModelEx,
    cache: &FetchCache<'_>,
    allowance: Option<&crate::fetch::Allowance>,
) -> anyhow::Result<ResolutionReport> {
    anyhow::ensure!(
        illumination.observations.is_loaded(),
        "illumination {} was handed to resolve_illumination with no observations loaded; fetch it \
         with Entity::load().with(model::user_observed_entity::Entity)",
        illumination.id
    );

    let ids: BTreeSet<i32> = illumination
        .observations
        .iter()
        .map(|observation| {
            observation.knowledge_entity_id.ok_or_else(|| {
                anyhow::anyhow!(
                    "illumination {} has unlinked observation {}; run link first",
                    illumination.id,
                    observation.id
                )
            })
        })
        .collect::<Result<_, _>>()?;

    if ids.is_empty() {
        return engine.resolve(&[], cache, allowance).await;
    }

    let entities = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::Id.is_in(ids.iter().copied()))
        .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
        .all(&db.conn)
        .await?;
    anyhow::ensure!(
        entities.len() == ids.len(),
        "illumination {} references {} live entities but {} were loaded; re-link before resolving",
        illumination.id,
        ids.len(),
        entities.len()
    );
    engine.resolve(&entities, cache, allowance).await
}

/// Resolve every entity still pending for any configured provider.
///
/// The engine receives the whole pending set in one call, so the wavefront still fills its batches.
/// This is the rebuild/drain path; a live capture should call [`resolve_illumination`].
pub async fn resolve_all_pending(
    db: &DbHandle,
    engine: &ResolutionEngine,
    cache: &FetchCache<'_>,
    allowance: Option<&crate::fetch::Allowance>,
    limit: Option<usize>,
    newest_first: bool,
) -> anyhow::Result<ResolutionReport> {
    let selection = ClaimSelection::from_handle(db);
    // One provider's pending set, then the union: an entity Wikidata answered is still pending for
    // IMDb. The limit is global, so a second resolver does not silently double the run.
    let mut pending: BTreeMap<i32, model::knowledge_entity::Model> = BTreeMap::new();
    for provider in engine.providers() {
        for entity in selection
            .awaiting_authority(&[provider], None, newest_first)
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

/// Claim the handle each linked observation showed, and project where it landed.
///
/// The only place a capture's identity and claim are written. Returns how many a standing veto
/// refused.
async fn write_capture_claims(
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
            && let Ok(identity) = IdentityDraft::try_from(&link)
        {
            let claimed = graph
                .claim_identity(
                    entity,
                    IdentitySettlement::CreateOnly(&identity),
                    ClaimDraft::capture(observation),
                    &vetoes,
                )
                .await?;
            if claimed.outcome == ClaimOutcome::Vetoed {
                vetoed += 1;
            }
        }
        // After the claims: status and confidence are functions of them.
        graph.update_projection(entity).await?;
    }
    Ok(vetoed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Gateway;
    use crate::knowledge::evidence::Claim;
    use crate::knowledge::identity::{Options, Resolver};
    use crate::test_support::{db_or_skip, fixtures};

    /// An authority that answers nothing: the helper tests are about scope, not claims.
    struct Empty;

    #[async_trait::async_trait]
    impl Resolver for Empty {
        fn id(&self) -> &'static str {
            "fake"
        }

        async fn resolve(
            &self,
            _entities: &[model::knowledge_entity::Model],
            _gateway: &Gateway<'_>,
        ) -> Vec<(i32, Claim)> {
            Vec::new()
        }
    }

    fn engine(db: &DbHandle) -> ResolutionEngine {
        ResolutionEngine::new(
            KnowledgeGraph::new(db),
            Options::default(),
            vec![Box::new(Empty)],
        )
        .expect("the fake provider is not reserved")
    }

    async fn loaded_illumination(db: &DbHandle, id: i32) -> model::illumination::ModelEx {
        model::illumination::Entity::load()
            .filter(model::illumination::Column::Id.eq(id))
            .with(model::user_observed_entity::Entity)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
    }

    /// The helper resolves exactly the entities the illumination linked, once per distinct entity.
    #[tokio::test]
    async fn resolve_illumination_scopes_to_its_linked_entities() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "resolve_illumination").await;
        fixtures::observe(
            &db,
            &capture,
            0,
            "Train Dreams",
            None,
            "book",
            Some("x_twitter"),
            Some("@td"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("link");

        let illumination = loaded_illumination(&db, capture.illumination_id).await;
        let cache = FetchCache::new(&db.conn);
        let allowance = crate::fetch::Allowance::new(10, std::time::Duration::ZERO);
        let report =
            resolve_illumination(&db, &engine(&db), &illumination, &cache, Some(&allowance))
                .await
                .expect("resolve");

        assert_eq!(report.considered, 1, "{report:?}");
        assert_eq!(report.offline, false);
    }

    /// An illumination handed over before `link` is a pipeline error, not an empty resolve.
    #[tokio::test]
    async fn resolve_illumination_refuses_unlinked_observations() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "resolve_unlinked").await;
        fixtures::observe(&db, &capture, 0, "Train Dreams", None, "book", None, None).await;

        let illumination = loaded_illumination(&db, capture.illumination_id).await;
        let cache = FetchCache::new(&db.conn);
        let allowance = crate::fetch::Allowance::new(10, std::time::Duration::ZERO);
        let error =
            resolve_illumination(&db, &engine(&db), &illumination, &cache, Some(&allowance))
                .await
                .expect_err("an unlinked observation must be refused");

        assert!(error.to_string().contains("unlinked"), "{error}");
    }

    /// The primitive resolves the slice it is given; it does not consult the pending selection.
    #[tokio::test]
    async fn resolve_takes_the_explicit_scope() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Explicit").await;
        let entity = model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        let cache = FetchCache::new(&db.conn);
        let allowance = crate::fetch::Allowance::new(10, std::time::Duration::ZERO);
        let report = engine(&db)
            .resolve(std::slice::from_ref(&entity), &cache, Some(&allowance))
            .await
            .expect("resolve");

        assert_eq!(report.considered, 1, "{report:?}");
    }
}
