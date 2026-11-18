//! What a call did, and the typed reasons it could not.

use serde::Serialize;

use crate::knowledge::ids::{EntityOriginId, ObservationId};

/// A call that could not be applied. Any refusal rolls the whole call back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    UnknownObservation(ObservationId),
    UnknownOrigin(EntityOriginId),
    AmbiguousOrigin(EntityOriginId),
    LinkedElsewhere(ObservationId),
    OriginClaimed(ObservationId),
    StandingDistinct,
    PendingMerge,
    SharedIllumination,
    Contradiction(&'static str),
    Unauthorized,
}

/// One call's outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub applied: usize,
    pub unchanged: usize,
    pub refused: Option<Refusal>,
    /// Live entity ids the call wrote, for the caller's derived views (projection, index).
    pub touched: Vec<i32>,
}

impl Report {
    pub fn is_refused(&self) -> bool {
        self.refused.is_some()
    }
}

/// What one mutation did, before the call is committed.
pub(super) enum Outcome {
    /// Applied, touching these entity ids (which will be reprojected).
    Applied(Vec<i32>),
    /// Nothing to do; not a failure.
    NoChange,
    /// The call cannot proceed.
    Refused(Refusal),
}
