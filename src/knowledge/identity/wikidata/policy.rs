//! The gates: could one of these candidates be the thing the capture is about?
//!
//! Everything here is pure. Candidates and what the entity already says go in; either one claim or
//! a refusal with a reason comes out. The order matters and is the design: the description decides
//! first, the hint is only a fallback, and a refusal records *which* gate closed, so `eval inspect`
//! reports it rather than guessing.
//!
//! Numbers live in `ranking.rs`; this file owns only the order and the admission rules.

use super::answer;
use super::ranking::{
    CLAIM_BAR, CLAIM_MARGIN, FIT_MARGIN, HINT_CEILING, HINT_MIN_SUPPORT, LONE_LABEL_CONFIDENCE,
    LOOKUP_CONFIDENCE, NAME_WEIGHT, Ranking, SUPPORT_WEIGHT, answer_confidence, blend_within,
    content_tokens, exact_label, fit_within, hint_tokens, near_surface,
};
use super::{Candidate, Kind};
use crate::fetch::Request;
use crate::knowledge::evidence::Claim;
use crate::knowledge::normalize;
use crate::knowledge::scoring::Rank;
use crate::model;

/// Last resort: the illumination's own disambiguation, consulted *only* when the description
/// cannot decide. It is weaker evidence than the capture's description, so its claims are capped
/// lower, and it can never overrule a description that resolved the case — which is what keeps
/// the `Sunset Boulevard` failure (the hint lifting the musical adaptation above the film) from
/// recurring, because there the description decides first.
pub(super) fn by_hint(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    reason: &mut &'static str,
) -> Vec<Claim> {
    if candidates.is_empty() {
        *reason = "the answer carried no candidate";
        return Vec::new();
    }
    let Some(hint) = hint_tokens(entity) else {
        *reason = "no disambiguation hint to fall back on";
        return Vec::new();
    };
    // A candidate, its blended score, and the proportion of the hint it accounts for. Not `Ranked`:
    // that type's `fit` means "description evidence", and an earlier version smuggled the support
    // proportion into it, so the same field meant two things on two paths.
    let mut scored: Vec<(&Candidate, f32, f32)> = candidates
        .iter()
        .map(|candidate| {
            let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
            // How much of the *disambiguator* this candidate accounts for. A proportion, not an
            // overlap: the hint is short, and every word of it should be findable.
            let matched = hint.iter().filter(|token| theirs.contains(token)).count();
            let support = matched as f32 / hint.len() as f32;
            (candidate, NAME_WEIGHT + SUPPORT_WEIGHT * support, support)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, best_score, best_support) = scored[0];
    let runner_up = scored.get(1).map(|other| other.1).unwrap_or(0.0);
    // Half the disambiguating words, not a third. One of three cleared the old bar, which is how
    // `Suspicion` (a Matsumoto novel) claimed a Dürrenmatt one on the word "novel" alone.
    if best_support < HINT_MIN_SUPPORT
        || best_score < CLAIM_BAR
        || best_score - runner_up < CLAIM_MARGIN
    {
        *reason = "the hint did not clear its bar";
        return Vec::new();
    }
    vec![answer::claim(
        best,
        "search_hint",
        answer_confidence(best_score).min(HINT_CEILING),
    )]
}
/// Which rung produced a claim. Recorded on the claim, so it must name the rung that answered and
/// not the shortcut that got there.
fn method_for(kind: &Kind, exact_label: bool) -> &'static str {
    match (kind, exact_label) {
        (Kind::Title, _) => "wikipedia_title",
        (Kind::Search, true) => "search_lookup",
        (Kind::Search, false) => "search_ranked",
        (_, true) => "exact_label",
        (_, false) => "ranked",
    }
}

/// Choose a referent from the candidates, or none. Shared by the label path and the search
/// fallback — the only difference between them is which candidates are admitted.
pub(super) fn decide(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<Kind>,
    reason: &mut &'static str,
) -> Vec<Claim> {
    // A lone exact label is evidence *only on the label rung*, because that query enumerates every
    // item with the string: one result means the name is unique on Wikidata. A title answer always
    // carries exactly one candidate — a page maps to an item by construction — so counting it there
    // says nothing, which is how `Orthodoxy` claimed the concept instead of the Chesterton book.
    //
    // Scoped by rung, not weakened by description: two true descriptions of one person — "British
    // filmmaker (1899–1980)" and "English film director" — can share no content word, so demanding
    // fit here refuses correct answers. Ambiguity is what the shortcut must not survive, and the
    // label rung already reports it as more than one candidate. This runs before `Rank`, whose job
    // is the corroborated candidates; a lone label needs no corroboration.
    if request.kind == Kind::Label
        && candidates.len() == 1
        && exact_label(entity, &candidates[0])
    {
        return vec![answer::claim(
            &candidates[0],
            method_for(&request.kind, true),
            LONE_LABEL_CONFIDENCE,
        )];
    }

    // The verb: the candidates whose description corroborates, best fit first.
    let ranked = Ranking.rank(entity, candidates);
    let Some(&(best, best_fit)) = ranked.first() else {
        *reason = "the description does not corroborate";
        return by_hint(entity, candidates, reason);
    };

    // `Rank` orders by `fit`, so the winner and the runner-up are measured in one order and the
    // subtraction below is a gap between comparable quantities. It used to be ordered by the blend
    // while this margin read `fit`, so a runner-up with *more* description evidence than the winner
    // produced a negative gap — and the claim was refused with "neither wins" while holding a winner.
    let runner_up = ranked.get(1).map(|(_, fit)| *fit).unwrap_or(0.0);

    // Otherwise a claim must beat the runner-up. Ambiguity is not a reason to give up —
    // `Sunset Boulevard` has eleven candidates and the description picks the film — but a near-tie
    // is.
    if best_fit - runner_up < FIT_MARGIN {
        *reason = "two candidates fit and neither wins";
        return Vec::new();
    }

    let best = &candidates[best];
    vec![answer::claim(
        best,
        method_for(&request.kind, exact_label(entity, best)),
        answer_confidence(blend_within(entity, best, candidates)),
    )]
}

/// The article that bears this exact name, offered only after every question that could corroborate
/// was asked and refused. A title maps one article to one name, so a lone answer says nothing about
/// ambiguity — which is why this spoke last: as a first answer it made `Dogma` the doctrine and
/// `Orthodoxy` the concept.
pub(super) fn surface_only(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<Kind>,
) -> Option<Claim> {
    if request.kind != Kind::Title || candidates.len() != 1 {
        return None;
    }
    let candidate = &candidates[0];
    // The article has to be about our name: Wikipedia redirects otherwise (`The Bounty Hunter Wars`
    // lands on `Boba Fett`), and this is the last place that can be noticed.
    if let Some(label) = candidate.label.as_deref() {
        if normalize::comparable(label) != normalize::comparable(&entity.name) {
            return None;
        }
    }
    Some(answer::claim(
        candidate,
        "wikipedia_title",
        LONE_LABEL_CONFIDENCE,
    ))
}

/// A title maps to one article. A disambiguation page maps to none: it is a menu, and claiming
/// it is the trap that produced `Royal Navy` → an aircraft-carrier fleet.
pub(super) fn parse_title(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<Kind>,
    reason: &mut &'static str,
) -> Vec<Claim> {
    // A title is a *name* lookup, so the answer has to be about that name. Wikipedia redirects
    // otherwise: asking for `The Bounty Hunter Wars` lands on the article for `Boba Fett`, and
    // "one title, one article" then reads as identity. Sharing no word with our name is not an
    // identity, whatever the title said.
    let ours = content_tokens(&entity.name);
    let ours_key = normalize::comparable(&entity.name);
    let titled: Vec<Candidate> = candidates
        .iter()
        .filter(|candidate| {
            // No label is not a mismatch: Wikidata sometimes answers with a description only,
            // and `Q42574 · "Canadian filmmaker and explorer"` is exactly the right item for
            // `James Cameron`. `decide` still requires the description to corroborate.
            let Some(label) = candidate.label.as_deref() else {
                return true;
            };
            if near_surface(&normalize::comparable(label), &ours_key) {
                return true;
            }
            content_tokens(label)
                .iter()
                .any(|token| ours.contains(token))
        })
        .cloned()
        .collect();
    if titled.is_empty() {
        *reason = "the title is about another subject";
        return Vec::new();
    }
    decide(entity, &titled, request, reason)
}

/// A search answer: one corroborated surface is identity, anything else is a ranker question.
pub(super) fn parse_search(
    entity: &model::knowledge_entity::Model,
    candidates: Vec<Candidate>,
    request: &Request<Kind>,
    reason: &mut &'static str,
) -> Vec<Claim> {
    let ours = normalize::comparable(&entity.name);
    let same: Vec<&Candidate> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .label
                .as_deref()
                .is_some_and(|label| normalize::comparable(label) == ours)
        })
        .collect();
    // A matching surface is not enough — the description has to corroborate it, exactly as on
    // the label path. Without this, `Audi` (a car brand) claims a *family name* and
    // `A Pattern Language` claims a 2011 master's thesis, both of which match the surface.
    //
    // Anything short of one corroborated surface falls through to the ranker: *zero* is a
    // spelling difference (`J.R.R.` vs `J. R. R.`), *several* is a shared title where only the
    // description can pick the referent.
    if let [candidate] = same.as_slice()
        && fit_within(entity, candidate, &candidates) > 0.0
    {
        return vec![answer::claim(candidate, "search_lookup", LOOKUP_CONFIDENCE)];
    }
    // A candidate is admissible when it *is* our name, a trivial respelling of it, or one of the
    // two is a superset of the other's content words: `Tomb of Napoleon Bonaparte` vs `Tomb of
    // Napoleon`, `Andriivka, Donetsk Oblast` vs `Andriivka`, `The Justice of Emperor Otto III`
    // vs `Justice of Emperor Otto III: The Beheading…`. The ladder used to admit only the first
    // two, so a name carrying a qualifier never let its description speak — measured on the
    // local corpus, 51 undecided entities had the right item in hand and were refused here.
    //
    // Admission is not a claim. The description must still corroborate — a non-zero fit — and beat the
    // runner-up, which is what keeps `Royal Navy fleet aircraft carrier fleet` off `Royal Navy`.
    let mine = content_tokens(&entity.name);
    let admitted: Vec<Candidate> = candidates
        .iter()
        .filter(|candidate| {
            let Some(label) = candidate.label.as_deref() else {
                return false;
            };
            if near_surface(&normalize::comparable(label), &ours) {
                return true;
            }
            // Our name is the label plus a qualifier the authority keeps elsewhere: `Andriivka,
            // Donetsk Oblast`, `Pennsylvania Station (1910–1963)`. The qualifier is ours to
            // carry, not the authority's — and refusing here means the description never
            // gets to separate the six villages that share one name.
            let (name_flat, label_flat) = (
                normalize::comparable(&entity.name),
                normalize::comparable(label),
            );
            if let Some(rest) = name_flat.strip_prefix(&label_flat) {
                let rest = rest.trim_start();
                if rest.starts_with(',') || rest.starts_with('(') {
                    return true;
                }
            }
            // A one-word name is still a name: `Rare` against `Rare Ltd.`, `Bash` against
            // `GNU Bash`. Requiring both sides to carry two content words excluded exactly the
            // short names, and the description is what decides anyway.
            let theirs = content_tokens(label);
            if theirs.is_empty() || mine.is_empty() {
                return false;
            }
            let (small, large) = if theirs.len() <= mine.len() {
                (&theirs, &mine)
            } else {
                (&mine, &theirs)
            };
            small.iter().all(|token| large.contains(token))
        })
        .cloned()
        .collect();
    if admitted.is_empty() {
        *reason = "nothing in the answer relates to the name";
        // The name nominated nothing, but the hint may still: `Sten Mk IIS suppressed` is a
        // model whose label is just `Sten`, and no name rule will connect the two. Admission is
        // a name test, so the hint path has to be reached before it turns the answer down.
        return by_hint(entity, &candidates, reason);
    }
    decide(entity, &admitted, request, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixtures::stub_entity;

    fn candidate(id: &str, label: &str, description: Option<&str>) -> Candidate {
        Candidate {
            id: id.to_string(),
            label: Some(label.to_string()),
            description: description.map(str::to_string),
            classes: Vec::new(),
        }
    }

    fn search(query: &str) -> Request<Kind> {
        Request {
            provider: "wikidata",
            kind: Kind::Search,
            shape: "test".to_string(),
            query: query.to_string(),
        }
    }

    /// The shared fold is the one that decides what "the same name" means: an accent is a spelling,
    /// not a different article. `surface_key` used to keep the accent and miss this.
    #[test]
    fn the_shared_fold_sees_through_an_accent() {
        let entity = stub_entity(1, "Pokemon", "unknown");
        let candidates = vec![candidate("Q1", "Pokémon", None)];
        let request = Request {
            provider: "wikidata",
            kind: Kind::Title,
            shape: "test".to_string(),
            query: "Pokemon".to_string(),
        };
        assert!(
            surface_only(&entity, &candidates, &request).is_some(),
            "a spelling difference is not a different article"
        );
    }

    /// The winner and the runner-up must be measured in one order.
    ///
    /// This is the case that exposed the split: the exact-label candidate wins the *blend* on its
    /// name (0.820) even though the runner-up fits the description better (0.842 against 0.700).
    /// Comparing a blend-ordered winner against an evidence-ordered runner-up made the margin
    /// negative and refused a claim the evidence supports.
    ///
    /// The description describes the second candidate because that is how a graph reaches this
    /// shape: `our_tokens` takes the best fit over every `alt_description`, not only the one that
    /// arrived with the name.
    #[test]
    fn the_runner_up_is_measured_in_the_same_order_as_the_winner() {
        let mut entity = stub_entity(1, "Dune", "book");
        entity.description = Some("1969 novel by Frank Herbert, sequel to Dune".to_string());
        let candidates = vec![
            candidate(
                "Q191536",
                "Dune",
                Some("1965 science fiction novel by Frank Herbert"),
            ),
            candidate(
                "Q207189",
                "Dune Messiah",
                Some("1969 novel by Frank Herbert, sequel to Dune"),
            ),
        ];
        let mut reason = "";

        let claims = decide(&entity, &candidates, &search("Dune"), &mut reason);

        assert_eq!(
            claims.len(),
            1,
            "the evidence supports a claim, not a refusal: {reason:?}"
        );
        assert_eq!(claims[0].external_id, "Q207189");
        assert_eq!(claims[0].method, "search_ranked");
    }
}
