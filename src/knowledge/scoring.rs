//! Scoring: the verbs a measure answers.
//!
//! - [`Similarity`] measures one pair — a number, never a verdict.
//! - [`Policy`] measures a *set*: the evidence for each candidate, which of them qualifies, and how
//!   they order. The set is the corpus, which is why a corpus-aware measure cannot be a `Similarity`.
//! - [`Rank`] is the object-safe form of a `Policy`: qualifying candidates, best first. Every `Policy`
//!   gets it for free, so the filter-and-order procedure is written once here rather than per policy.
//!
//! The filter cannot be a threshold on a score: the graph's rule is a conjunction — a name match has
//! to be corroborated — and a number cannot carry its components. A policy that fails it still
//! reports its evidence, so "same name, different type: 0.74" stays visible.
//!
//! Not verbs: a verdict belongs to the domain that can use it (an index to link, an identity to
//! annotate), confidence is that domain's calibration, and the aggregate over a set is arithmetic.
//! Ordering of confidences is [`TotalCmp`].

use std::cmp::Ordering;
use std::sync::Arc;

use crate::model;

/// The graph's own side of a comparison.
pub(crate) type Entity = model::knowledge_entity::Model;
/// What a capture asserted: one observation, no alternatives.
pub(crate) type Observation = model::user_observed_entity::Model;

/// How strongly the evidence says these two are one thing, in `[0, 1]`.
///
/// Corpus-free by definition: the same pair scores the same wherever it is asked. A measure that
/// depends on the candidate set is not a `Similarity` — it is a [`Policy`].
///
/// A measurement, never a verdict: the number is stored on a proximity row and shown to a human, so
/// it must mean the same thing in both places.
pub trait Similarity<A, B>: Send + Sync {
    fn score(&self, a: &A, b: &B) -> f32;
}

/// How one policy measures a candidate set, and what it does with the measurement.
///
/// Three methods because they are three views of one policy, not three policies: the components that
/// decide qualification are the same components the order key is built from.
pub trait Policy<A, B>: Send + Sync {
    /// The policy's own components — `Features` for the graph, `fit` and its blend for an identity resolver.
    type Evidence;

    /// The evidence for every candidate, in candidate order. Corpus-aware by construction: the slice
    /// *is* the corpus, which is what separates this from [`Similarity`].
    fn evaluate(&self, subject: &A, candidates: &[B]) -> Vec<Self::Evidence>;

    /// Is this evidence enough to answer with? The conjunction lives here.
    fn qualifies(&self, evidence: &Self::Evidence) -> bool;

    /// Where this candidate sorts. Higher is better on both fields; equal keys keep candidate order.
    fn order(&self, evidence: &Self::Evidence) -> Order;
}

/// A candidate's place in the order: one number that decides, one that only separates equals.
///
/// Two fields because two policies need two kinds of separation: the graph's judgement is a single
/// total, while an identity resolver orders by evidence and uses its price to break ties — otherwise two
/// candidates with identical evidence would fall back on the order the caller happened to supply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Order {
    /// Decides the order.
    pub primary: f32,
    /// Consulted only when `primary` ties.
    pub tiebreak: f32,
}

impl Order {
    /// A key with nothing to break a tie with.
    pub fn of(primary: f32) -> Self {
        Self {
            primary,
            tiebreak: 0.0,
        }
    }
}

/// Which of these candidates is *this*, best first — and only the ones the policy qualifies.
///
/// The object-safe projection of a [`Policy`], and the only form the store holds as a trait object.
/// Its filter is the policy's, so an empty list means exactly what [`Policy::qualifies`] says it
/// means — nothing here decides on its own.
pub trait Rank<A, B>: Send + Sync {
    fn rank(&self, a: &A, candidates: &[B]) -> Vec<(usize, f32)>;
}

/// Every `Policy` gets this for free — there is no other implementation, so the filter-and-order
/// procedure exists once.
impl<A, B, P> Rank<A, B> for P
where
    P: Policy<A, B>,
{
    fn rank(&self, subject: &A, candidates: &[B]) -> Vec<(usize, f32)> {
        let mut ranked: Vec<(usize, Order)> = self
            .evaluate(subject, candidates)
            .into_iter()
            .enumerate()
            .filter_map(|(index, evidence)| {
                self.qualifies(&evidence)
                    .then(|| (index, self.order(&evidence)))
            })
            .collect();
        // Stable: equal keys keep the order the caller supplied.
        ranked.sort_by(|a, b| {
            b.1.primary
                .total_cmp(&a.1.primary)
                .then_with(|| b.1.tiebreak.total_cmp(&a.1.tiebreak))
        });
        ranked
            .into_iter()
            .map(|(index, order)| (index, order.primary))
            .collect()
    }
}

/// `f32::total_cmp`, for `Option<f32>`, with `None` least.
///
/// The method std would have if `Option<f32>` were `Ord` — `f32` is not, because of NaN, so the
/// comparison is [`f32::total_cmp`] over `NEG_INFINITY`-widened values.
pub trait TotalCmp {
    fn total_cmp(&self, other: &Self) -> Ordering;
}

impl TotalCmp for Option<f32> {
    fn total_cmp(&self, other: &Self) -> Ordering {
        self.unwrap_or(f32::NEG_INFINITY)
            .total_cmp(&other.unwrap_or(f32::NEG_INFINITY))
    }
}

/// The verbs, from one policy, for the pairs the graph asks about.
///
/// Three fields because a trait object fixes its type parameters: the same policy is asked three
/// questions — a capture against entities, two entities as a verdict, two entities as a number.
#[derive(Clone)]
pub struct Scoring {
    /// A capture's observation against the graph's entities: empty means start a new entity.
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
        P: Policy<Observation, Entity>
            + Policy<Entity, Entity>
            + Similarity<Entity, Entity>
            + 'static,
    {
        Self {
            link: policy.clone(),
            merge: policy.clone(),
            pair: policy,
        }
    }
}
