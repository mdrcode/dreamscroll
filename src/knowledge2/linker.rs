//! The planner seam: pending observations in, graph actions out.
//!
//! The blocker is recall; the holder is the one evidence read. A policy implements [`Linker`] and
//! emits [`KnowledgeAction`]s; [`HeuristicPlanner`](crate::knowledge2::heuristic::HeuristicPlanner) is
//! today's.

use crate::model;

use super::action::KnowledgeAction;
use super::blocker::Blocker;
use super::ids::EntityOriginId;

/// The one evidence read: which live entity already carries `(scheme, external_id)`, by origin?
#[async_trait::async_trait]
pub trait IdentityHolder: Send + Sync {
    async fn holder(
        &self,
        scheme: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<EntityOriginId>>;
}

/// A policy: the pending batch in, one action per observation out.
///
/// The caller selects the batch — observations already linked are not the planner's problem. The
/// unit is one illumination, because a reading is decided as one.
#[async_trait::async_trait]
pub trait Linker: Send + Sync {
    async fn plan(
        &self,
        blocker: &dyn Blocker,
        holder: &dyn IdentityHolder,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<Vec<KnowledgeAction>>;
}
