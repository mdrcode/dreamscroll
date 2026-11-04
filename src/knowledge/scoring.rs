//! Scoring: two verbs.
//!
//! - [`Similarity`] measures one pair: how strongly the evidence says these two are one thing.
//! - [`Rank`] answers one question: which of these candidates is *this*, best first.
//!
//! Everything else a graph needs from scoring is not a verb:
//!
//! - **A threshold** is [`Rank`]'s filter. It cannot be a constant on the score, because the rule
//!   that separates "same thing" from "similar" is a conjunction over what was compared — a name
//!   match has to be corroborated — and a number cannot carry its own components.
//! - **An aggregate** over several claims about one entity is arithmetic, not a judgement; it is
//!   `heuristic_scoring::combine`, one function, written once.
//! - **An ordering** between two substantiated things is [`by_worth`]: a convention about `None`,
//!   with no policy in it.
//!
//! One implementation covers the graph — `heuristic_scoring::Heuristic` — and implements both traits
//! for each pair it is asked about. An authority implements [`Rank`] for its *own* candidate type,
//! because its measure is corpus-aware (it weighs terms by rarity among the candidates it got back)
//! and a pair comparison cannot see the corpus.
//!
//! Note the asymmetry this admits: [`Similarity`] reports how alike a pair is *even when it is not
//! one thing*. That is deliberate — "same name, different type" at 0.74 is how a person spots the
//! merge that must never happen, and a score gated by its own threshold could not say it.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::model;

/// One side of a comparison: a stored row, flattened and owned, so it can cross a trait object.
///
/// Everything here is what the *row* holds, not what today's policy reads. A policy that prices a
/// handle, weighs accumulated surfaces, or consults the illumination's hint gets them without a
/// trait change. An observation has no alternatives — a capture makes one claim — so those lists
/// are empty rather than a list of one.
#[derive(Debug, Clone)]
pub struct Subject {
    pub name: String,
    pub description: Option<String>,
    pub entity_type: String,
    pub search_query: Option<String>,
    pub platform: Option<String>,
    pub platform_handle: Option<String>,
    pub platform_display_name: Option<String>,
    pub alt_names: Vec<String>,
    pub alt_types: Vec<String>,
    pub alt_descriptions: Vec<String>,
    pub alt_search_queries: Vec<String>,
}

impl Subject {
    pub fn of_observation(row: &model::user_observed_entity::Model) -> Self {
        Self {
            name: row.name.clone(),
            description: row.description.clone(),
            entity_type: row.entity_type.clone(),
            search_query: row.search_query.clone(),
            platform: row.platform.clone(),
            platform_handle: row.platform_handle.clone(),
            platform_display_name: row.platform_display_name.clone(),
            alt_names: Vec::new(),
            alt_types: Vec::new(),
            alt_descriptions: Vec::new(),
            alt_search_queries: Vec::new(),
        }
    }

    /// An entity scored as if it were an observation: its selected values stand in for what a
    /// capture said, and what its members accumulated becomes the alternatives.
    pub fn of_entity(row: &model::knowledge_entity::Model) -> Self {
        Self {
            name: row.name.clone(),
            description: row.description.clone(),
            entity_type: row.entity_type.clone(),
            search_query: row.search_query.clone(),
            platform: row.platform.clone(),
            platform_handle: row.platform_handle.clone(),
            platform_display_name: row.platform_display_name.clone(),
            alt_names: row.alt_names.clone(),
            alt_types: row.alt_types.clone(),
            alt_descriptions: row.alt_descriptions.clone(),
            alt_search_queries: row.alt_search_queries.clone(),
        }
    }
}

/// How strongly the evidence says these two are one thing, in `[0, 1]`.
///
/// A measurement, never a verdict. The same number is stored on a proximity row, used to order
/// candidates, and priced into a confidence — so it has to mean the same thing in all three, and
/// it cannot be pre-filtered by its consumer.
pub trait Similarity<A, B>: Send + Sync {
    fn score(&self, a: &A, b: &B) -> f32;
}

/// Which of these candidates is *this*, best first — and only the ones worth choosing.
///
/// Two duties in one verb because they cannot be separated: ordering, and the threshold that turns
/// a candidate into an answer. The filter is why an empty list means "nothing here is this", and it
/// is where a component-shaped rule lives, since a number cannot carry its own components.
pub trait Rank<A, B>: Send + Sync {
    fn rank(&self, a: &A, candidates: &[B]) -> Vec<(usize, f32)>;
}

/// Which of two substantiated things wins. Not policy: `None` is least, ties stay ties.
///
/// One definition, asked by the merge survivor choice and by the identity-holder order — the two
/// places that used to disagree about `NULL`.
pub fn by_worth(a: Option<f32>, b: Option<f32>) -> Ordering {
    a.unwrap_or(f32::NEG_INFINITY)
        .total_cmp(&b.unwrap_or(f32::NEG_INFINITY))
}

type Entity = model::knowledge_entity::Model;

/// The two verbs, from one policy, for the pairs the graph asks about.
///
/// Three fields because a trait object fixes its type parameters: the same policy is asked three
/// questions — a capture against entities, two entities as a verdict, two entities as a number.
#[derive(Clone)]
pub struct Scoring {
    /// One capture's claim against the graph's entities: `None` means start a new entity.
    pub link: Arc<dyn Rank<Subject, Entity>>,
    /// Two entities, for the merge question. A set of one is the pair question.
    pub merge: Arc<dyn Rank<Entity, Entity>>,
    /// Two entities, as the number a proximity row stores.
    pub pair: Arc<dyn Similarity<Entity, Entity>>,
}

impl Scoring {
    /// The heuristic: today's only policy, and the default for every store that does not choose.
    pub fn heuristic() -> Self {
        let policy = Arc::new(super::heuristic_scoring::Heuristic);
        Self {
            link: policy.clone(),
            merge: policy.clone(),
            pair: policy,
        }
    }
}

impl Default for Scoring {
    fn default() -> Self {
        Self::heuristic()
    }
}
