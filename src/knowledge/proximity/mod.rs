//! Proximities: the candidates the last scan found, and the passes that produce and answer them.
//!
//! Sharing an identity is a candidate, not a verdict. An album's article is the identity of the album
//! *and* covers each of its songs, so two holders may be one thing, or may be a work and the things
//! inside it. Claims are many-to-one precisely so both readings can be stored.
//!
//! [`scan`] derives the table and replaces it wholesale, so what it holds is always "the candidates
//! as of the last scan" — no status, no history, no rows to settle. [`auto_merge`] answers them: it
//! folds the pairs that score as one thing and leaves the rest, which are the questions still owed to
//! a person.
//!
//! Both passes take what they act on — the connection and the
//! [`Heuristic`] to measure with, and, for [`auto_merge`], the store it folds into — rather than
//! reading them off a facade.
//!
//! [`Heuristic`]: crate::knowledge::linker::heuristic::Heuristic

mod auto_merge;
mod scan;

pub use auto_merge::{AutoMergeReport, auto_merge};
pub use scan::scan;

/// Why a proximity was raised.
///
/// A named type rather than a string at the call sites: the value is stored, compared, and will grow a
/// second case (entities that look alike without sharing an identity), so a typo should not become a
/// third basis that every query silently misses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Two entities hold the same identity.
    SharedId,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedId => "shared_id",
        }
    }
}

#[cfg(test)]
mod tests;
