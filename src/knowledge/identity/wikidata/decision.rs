//! Which candidates are referents at all — the only filter the aggregate ranking keeps.
//!
//! A menu is not a thing to identify, and an answer that carried only one proves nothing. Everything
//! else about a candidate — which name it bears, which kind it is, which question produced it — is
//! the field's question, answered by scoring in [`super::aggregate`].
//!
//! The ladder's per-rung admission, its early exits and its type gate were removed with the ladder.
//! They decided *before* the ranking, which is how `BBC` had `Q9531` at the top of two answers and
//! still claimed a division: the right item was excluded, not outranked.

use crate::fetch::Answer;

use super::{WikidataCandidate, answer};

/// The candidates that are referents at all.
pub(crate) fn referents(
    raw: &Answer<Vec<WikidataCandidate>>,
) -> Result<Vec<WikidataCandidate>, &'static str> {
    if !raw.outcome.answered() {
        return Err("the answer carried no candidate");
    }
    let mut candidates = raw.body.clone().unwrap_or_default();
    candidates.retain(|candidate| !answer::is_menu(candidate));
    if candidates.is_empty() {
        return Err("the answer carried only a disambiguation page");
    }
    Ok(candidates)
}
