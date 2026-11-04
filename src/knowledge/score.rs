//! Scoring: the verbs a policy provides, and the vocabulary it is asked in.
//!
//! Three verbs, because they are calibrated against one another and mean nothing apart:
//!
//! - [`Similarity`] — how alike are these two things, with every other candidate in view?
//! - [`Judge`] — is that likeness a decision? One pair; enough evidence; a big enough lead.
//! - [`Worth`] — what is an answer worth, and which of two substantiated things wins?
//!
//! Who asks, and why this is not a field of the linker: the linker asks [`Similarity`] and [`Judge`],
//! `reconcile` asks all three, `project` asks [`Worth::combine`], and `resolve` asks [`Judge`] and
//! [`Worth::value`]. Four modules consume the policy, so the store owns one [`Scoring`] and hands it
//! to each of them — a linker that carried its own would leave the projection and the merge pass on
//! a different policy, silently.
//!
//! The one implementation is `heuristic_score::Heuristic`. Swapping it means building a [`Scoring`]
//! and handing it to [`Knowledge::with_scoring`](super::Knowledge::with_scoring); the linker is a
//! procedure over the traits, not the policy itself.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::model;

use super::evidence::ClaimBasis;

/// One side of a comparison: a stored row, flattened.
///
/// Everything here is what the *row* holds, not what today's policy happens to read. That is the
/// point of the type: a policy is a parameter, so the vocabulary must be the graph's, and hiding a
/// field is a silent veto on every future policy. The heuristic reads the first three and ignores
/// the rest; an embedding policy reads the text, a hint-aware one reads `search_query`, a policy
/// that weighs provenance reads the presence.
///
/// The two constructors differ in one honest way: a capture makes one claim, so an observation has
/// no alternatives — `of_observation` leaves those lists empty rather than inventing a `[name]`.
#[derive(Debug, Clone, Copy)]
pub struct Subject<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub entity_type: &'a str,
    /// The illumination's disambiguating words, when the subject carried one.
    pub search_query: Option<&'a str>,
    /// The presence a capture showed: the platform, the handle, and the name it displayed.
    pub platform: Option<&'a str>,
    pub platform_handle: Option<&'a str>,
    pub platform_display_name: Option<&'a str>,
    /// What this entity's *other* members were called. Empty for an observation.
    pub alt_names: &'a [String],
    pub alt_types: &'a [String],
    pub alt_descriptions: &'a [String],
    pub alt_search_queries: &'a [String],
}

impl<'a> Subject<'a> {
    pub fn of_observation(row: &'a model::user_observed_entity::Model) -> Self {
        Self {
            name: &row.name,
            description: row.description.as_deref(),
            entity_type: &row.entity_type,
            search_query: row.search_query.as_deref(),
            platform: row.platform.as_deref(),
            platform_handle: row.platform_handle.as_deref(),
            platform_display_name: row.platform_display_name.as_deref(),
            alt_names: &[],
            alt_types: &[],
            alt_descriptions: &[],
            alt_search_queries: &[],
        }
    }

    /// An entity scored as if it were an observation: its selected values stand in for what a
    /// capture said, and what its members accumulated becomes the alternatives. This is how
    /// `reconcile` asks about two entities.
    pub fn of_entity(row: &'a model::knowledge_entity::Model) -> Self {
        Self {
            name: &row.name,
            description: row.description.as_deref(),
            entity_type: &row.entity_type,
            search_query: row.search_query.as_deref(),
            platform: row.platform.as_deref(),
            platform_handle: row.platform_handle.as_deref(),
            platform_display_name: row.platform_display_name.as_deref(),
            alt_names: &row.alt_names,
            alt_types: &row.alt_types,
            alt_descriptions: &row.alt_descriptions,
            alt_search_queries: &row.alt_search_queries,
        }
    }
}

/// What one candidate earns against one subject, per feature and in total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Features {
    pub name: f32,
    pub description: f32,
    pub type_match: f32,
    pub total: f32,
}

/// Verbs 1–3: measure, and weigh what is measured.
pub trait Similarity: Send + Sync {
    /// Score `subject` against every candidate, one `Features` each, in `candidates` order.
    ///
    /// The whole set is handed over rather than one candidate at a time because a policy may weigh
    /// terms by how rare they are *among these candidates* — a prior no single pair can compute.
    fn fit(
        &self,
        subject: &Subject<'_>,
        candidates: &[model::knowledge_entity::Model],
    ) -> Vec<Features>;

    /// One pair, for the callers that have no set to compare against.
    fn pair(
        &self,
        subject: &Subject<'_>,
        candidate: &model::knowledge_entity::Model,
    ) -> Features {
        self.fit(subject, std::slice::from_ref(candidate))
            .into_iter()
            .next()
            .expect("fit returns one entry per candidate")
    }
}

/// Verbs 4–6. One trait, because a bar, an ordering and a margin are one calibration: raising the
/// floor without moving the margin changes which pairs win, not just how many.
pub trait Judge: Send + Sync {
    /// Do these features say the two are one thing?
    fn same(&self, features: &Features) -> bool;

    /// Is this much similarity evidence at all? A floor, not a verdict — "there is something here
    /// worth asking about", as opposed to "this is the answer".
    fn supported(&self, similarity: f32) -> bool;

    /// Is the best candidate's lead over the runner-up enough to decide? A near-tie is not evidence,
    /// however high both sit.
    fn leads(&self, gap: f32) -> bool;
}

/// How an answer was reached, on the policy's scale.
///
/// The `method` string stored on a claim stays the resolver's own record of the question it asked;
/// this is the *kind* of answer the policy is asked to price. Two records because they answer
/// different questions: `search_ranked` is for us to read, [`Answer::Ranked`] is what the weights see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// An authority's property matched a handle: the item *is* the account.
    Handle,
    /// One exact label and nothing competing for it — identity, with nothing corroborating it.
    LoneLabel,
    /// A search answer accepted because exactly one candidate's label is our own surface.
    Lookup,
    /// The illumination's own disambiguating words, as a last resort.
    Hint,
    /// A ranked answer: the description decided.
    Ranked,
}

/// Verbs 7–9.
pub trait Worth: Send + Sync {
    /// What a claim of this kind, from this score, is worth. `score` is ignored by answers the
    /// policy does not score (a handle match, a lone label).
    fn value(&self, answer: Answer, score: f32) -> f32;

    /// How substantiated an entity is, from the claims it holds. `None` when it holds none.
    fn combine(&self, claims: &[(ClaimBasis, Option<f32>)]) -> Option<f32>;

    /// Which of two substantiated things wins: `Greater` when `a` is the more substantiated.
    /// `None` is least. Ties are the caller's business — this compares worth only.
    fn order(&self, a: Option<f32>, b: Option<f32>) -> Ordering;
}

/// The three verbs, from one policy.
#[derive(Clone)]
pub struct Scoring {
    pub similarity: Arc<dyn Similarity>,
    pub judge: Arc<dyn Judge>,
    pub worth: Arc<dyn Worth>,
}

impl Scoring {
    /// The heuristic: today's only policy, and the default for every store that does not choose.
    pub fn heuristic() -> Self {
        let policy = Arc::new(super::heuristic_score::Heuristic);
        Self {
            similarity: policy.clone(),
            judge: policy.clone(),
            worth: policy,
        }
    }
}

impl Default for Scoring {
    fn default() -> Self {
        Self::heuristic()
    }
}
