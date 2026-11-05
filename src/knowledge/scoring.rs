//! Scoring: two verbs.
//!
//! - [`Similarity`] measures one pair: how strongly the evidence says these two are one thing.
//! - [`Rank`] says which of a candidate set is *this*, best first — and only the ones worth choosing.
//!
//! The filter belongs to `Rank` and cannot be a threshold on the score: the rule is a conjunction —
//! a name match has to be corroborated — and a number cannot carry its components. A pair that fails
//! it still reports its score, so "same name, different type: 0.74" stays visible.
//!
//! One implementation covers the graph (`heuristic::scoring::Heuristic`); an authority implements
//! `Rank` for its own candidate type, because its measure is corpus-aware and a pair comparison
//! cannot see the corpus.
//!
//! Not verbs: the aggregate over claims is arithmetic (`heuristic::scoring::combine`), and
//! [`by_confidence`] is a convention about `NULL`.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::model;

/// The graph's own side of a comparison.
pub(crate) type Entity = model::knowledge_entity::Model;
/// What a capture asserted: one claim, no alternatives.
pub(crate) type Observation = model::user_observed_entity::Model;

/// How strongly the evidence says these two are one thing, in `[0, 1]`.
///
/// A measurement, never a verdict: the same number is stored on a proximity row, used to order
/// candidates, and priced into a confidence, so it must mean the same thing to all three.
pub trait Similarity<A, B>: Send + Sync {
    fn score(&self, a: &A, b: &B) -> f32;
}

/// Which of these candidates is *this*, best first — and only the ones worth choosing.
///
/// Two duties in one verb because they cannot be separated: ordering, and the threshold that turns a
/// candidate into an answer. The filter is why an empty list means "nothing here is this".
pub trait Rank<A, B>: Send + Sync {
    fn rank(&self, a: &A, candidates: &[B]) -> Vec<(usize, f32)>;
}

/// Which of two substantiated things wins. Not policy: `None` is least, ties stay ties.
///
/// One definition, asked by the merge survivor choice and by the identity-holder order — the two
/// places that used to disagree about `NULL`.
pub fn by_confidence(a: Option<f32>, b: Option<f32>) -> Ordering {
    a.unwrap_or(f32::NEG_INFINITY)
        .total_cmp(&b.unwrap_or(f32::NEG_INFINITY))
}

/// The two verbs, from one policy, for the pairs the graph asks about.
///
/// Three fields because a trait object fixes its type parameters: the same policy is asked three
/// questions — a capture against entities, two entities as a verdict, two entities as a number.
#[derive(Clone)]
pub struct Scoring {
    /// A capture's claim against the graph's entities: empty means start a new entity.
    pub link: Arc<dyn Rank<Observation, Entity>>,
    /// Two entities, as a verdict. A set of one is the pair question.
    pub merge: Arc<dyn Rank<Entity, Entity>>,
    /// Two entities, as the number a proximity row stores.
    pub pair: Arc<dyn Similarity<Entity, Entity>>,
}

impl Scoring {
    /// One policy, asked the three questions the store needs answered.
    ///
    /// Generic over the policy rather than taking a choice of implementation: whoever composes the
    /// store picks the concrete type, and this module stays a contract. The bounds are the whole
    /// requirement — a policy that cannot answer one of the three does not compile here.
    pub fn new<P>(policy: Arc<P>) -> Self
    where
        P: Rank<Observation, Entity> + Rank<Entity, Entity> + Similarity<Entity, Entity> + 'static,
    {
        Self {
            link: policy.clone(),
            merge: policy.clone(),
            pair: policy,
        }
    }
}
