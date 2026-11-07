//! The gates: could one of these candidates be the thing the capture is about?
//!
//! Everything here is pure. Candidates and what the entity already says go in; either one claim or
//! a refusal with a reason comes out. The order matters and is the design: the description decides
//! first, the hint is only a fallback, and a refusal records *which* gate closed, so `eval inspect`
//! reports it rather than guessing.
//!
//! Numbers live in `ranking.rs`; this file owns only the order and the admission rules.

use super::answer::{self, Candidate};
use super::ranking::{
    CLAIM_BAR, CLAIM_MARGIN, FIT_FLOOR, FIT_MARGIN, HINT_CEILING, HINT_MIN_SUPPORT,
    LONE_LABEL_CONFIDENCE, LOOKUP_CONFIDENCE, NAME_WEIGHT, Ranked, SUPPORT_WEIGHT, Weights,
    answer_confidence, content_tokens, hint_tokens, near_surface, our_tokens, rank, surface_key,
};
use crate::knowledge::identity::{Claim, Request};
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
    let Some(hint) = hint_tokens(entity) else {
        *reason = "no disambiguation hint to fall back on";
        return Vec::new();
    };
    let mut ranked: Vec<Ranked> = candidates
        .iter()
        .map(|candidate| {
            let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
            // How much of the *disambiguator* this candidate accounts for. A proportion, not an
            // overlap: the hint is short, and every word of it should be findable.
            let matched = hint.iter().filter(|token| theirs.contains(token)).count();
            let support = matched as f32 / hint.len() as f32;
            Ranked {
                candidate,
                score: NAME_WEIGHT + SUPPORT_WEIGHT * support,
                fit: support,
                // The hint path gates on fit and support: not an exact-label decision.
                exact_label: false,
            }
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = &ranked[0];
    let runner_up = ranked.get(1).map(|other| other.score).unwrap_or(0.0);
    // Half the disambiguating words, not a third. One of three cleared the old bar, which is how
    // `Suspicion` (a Matsumoto novel) claimed a Dürrenmatt one on the word "novel" alone.
    if best.fit < HINT_MIN_SUPPORT
        || best.score < CLAIM_BAR
        || best.score - runner_up < CLAIM_MARGIN
    {
        *reason = "the hint did not clear its bar";
        return Vec::new();
    }
    vec![answer::claim(
        best.candidate,
        "search_hint",
        answer_confidence(best.score).min(HINT_CEILING),
    )]
}
pub(super) fn property_of(kind: &str) -> Option<&str> {
    kind.strip_prefix("handle:")
}

/// Choose a referent from the candidates, or none. Shared by the label path and the search
/// fallback — the only difference between them is which candidates are admitted.
pub(super) fn decide(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request,
    reason: &mut &'static str,
) -> Vec<Claim> {
    let mut ranked = rank(entity, candidates);
    if ranked.is_empty() {
        *reason = "no candidate survived the name test";
        return Vec::new();
    }

    // One exact label and nothing else claiming it: that is identity, even with no text to
    // corroborate it.
    if ranked.len() == 1 && ranked[0].exact_label {
        return vec![answer::claim(
            ranked[0].candidate,
            "exact_label",
            LONE_LABEL_CONFIDENCE,
        )];
    }

    let best = ranked.remove(0);
    let runner_up = ranked.first().map(|other| other.fit).unwrap_or(0.0);

    // Otherwise a claim must clear the bar *and* beat the runner-up. Ambiguity is not a reason to
    // give up — `Sunset Boulevard` has eleven candidates and the description picks the film — but
    // a near-tie is.
    // The decision is on the *evidence*, not the blend. The name is why this candidate is here at
    // all, so requiring a blended score quietly demanded a fixed amount of text overlap — which
    // real, terse Wikidata descriptions cannot supply ("German-born theoretical physicist" vs a
    // capture's six-word description of Einstein).
    if best.fit < FIT_FLOOR {
        *reason = "the description does not corroborate";
        return by_hint(entity, candidates, reason);
    }
    // Two candidates fit and neither wins: genuine ambiguity. The hint must not break this tie.
    if best.fit - runner_up < FIT_MARGIN {
        *reason = "two candidates fit and neither wins";
        return Vec::new();
    }
    let method = match (request.kind.as_str(), best.exact_label) {
        ("title", _) => "wikipedia_title",
        ("search", true) => "search_lookup",
        ("search", false) => "search_ranked",
        (_, true) => "exact_label",
        (_, false) => "ranked",
    };
    vec![answer::claim(
        best.candidate,
        method,
        answer_confidence(best.score),
    )]
}

/// A title maps to one article. A disambiguation page maps to none: it is a menu, and claiming
/// it is the trap that produced `Royal Navy` → an aircraft-carrier fleet.
pub(super) fn parse_title(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request,
    reason: &mut &'static str,
) -> Vec<Claim> {
    // A title is a *name* lookup, so the answer has to be about that name. Wikipedia redirects
    // otherwise: asking for `The Bounty Hunter Wars` lands on the article for `Boba Fett`, and
    // "one title, one article" then reads as identity. Sharing no word with our name is not an
    // identity, whatever the title said.
    let ours = content_tokens(&entity.name);
    let titled: Vec<Candidate> = candidates
        .iter()
        .filter(|candidate| {
            // No label is not a mismatch: Wikidata sometimes answers with a description only,
            // and `Q42574 · "Canadian filmmaker and explorer"` is exactly the right item for
            // `James Cameron`. `decide` still requires the description to corroborate.
            let Some(label) = candidate.label.as_deref() else {
                return true;
            };
            if near_surface(&surface_key(label), &surface_key(&entity.name)) {
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
    request: &Request,
    reason: &mut &'static str,
) -> Vec<Claim> {
    let ours = surface_key(&entity.name);
    let same: Vec<&Candidate> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .label
                .as_deref()
                .is_some_and(|label| surface_key(label) == ours)
        })
        .collect();
    // A matching surface is not enough — the description has to corroborate it, exactly as on
    // the label path. Without this, `Audi` (a car brand) claims a *family name* and
    // `A Pattern Language` claims a 2011 master's thesis, both of which match the surface.
    //
    // Anything short of one corroborated surface falls through to the ranker: *zero* is a
    // spelling difference (`J.R.R.` vs `J. R. R.`), *several* is a shared title where only the
    // description can pick the referent.
    if let [candidate] = same.as_slice() {
        let weights = Weights::over(&candidates);
        let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
        let ours = our_tokens(entity);
        let fit = ours
            .iter()
            .map(|tokens| weights.fit(&theirs, tokens))
            .fold(0.0f32, f32::max);
        if fit >= FIT_FLOOR {
            return vec![answer::claim(candidate, "search_lookup", LOOKUP_CONFIDENCE)];
        }
    }
    // A candidate is admissible when it *is* our name, a trivial respelling of it, or one of the
    // two is a superset of the other's content words: `Tomb of Napoleon Bonaparte` vs `Tomb of
    // Napoleon`, `Andriivka, Donetsk Oblast` vs `Andriivka`, `The Justice of Emperor Otto III`
    // vs `Justice of Emperor Otto III: The Beheading…`. The ladder used to admit only the first
    // two, so a name carrying a qualifier never let its description speak — measured on the
    // local corpus, 51 undecided entities had the right item in hand and were refused here.
    //
    // Admission is not a claim. The description must still clear `FIT_FLOOR` and beat the
    // runner-up, which is what keeps `Royal Navy fleet aircraft carrier fleet` off `Royal Navy`.
    let mine = content_tokens(&entity.name);
    let admitted: Vec<Candidate> = candidates
        .iter()
        .filter(|candidate| {
            let Some(label) = candidate.label.as_deref() else {
                return false;
            };
            if near_surface(&surface_key(label), &ours) {
                return true;
            }
            // Our name is the label plus a qualifier the authority keeps elsewhere: `Andriivka,
            // Donetsk Oblast`, `Pennsylvania Station (1910–1963)`. The qualifier is ours to
            // carry, not the authority's — and refusing here means the description never
            // gets to separate the six villages that share one name.
            let (name_flat, label_flat) = (
                entity.name.trim().to_lowercase(),
                label.trim().to_lowercase(),
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
