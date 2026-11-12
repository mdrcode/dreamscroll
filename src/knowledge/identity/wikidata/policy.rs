//! The gates: could one of these candidates be the thing the capture is about?
//!
//! Everything here is pure. Candidates and what the entity already says go in; either one claim or
//! a refusal with a reason comes out. The order matters and is the design: the description decides
//! first, the hint is only a fallback, and a refusal records *which* gate closed, so `identity inspect`
//! reports it rather than guessing.
//!
//! Numbers live in `ranking.rs`; this file owns only the order and the admission rules.

use super::answer;
use super::ranking::{
    CLAIM_BAR, CLAIM_MARGIN, FIT_MARGIN, HINT_CEILING, HINT_MIN_SUPPORT, LONE_LABEL_CONFIDENCE,
    LOOKUP_CONFIDENCE, NAME_WEIGHT, Ranking, SUPPORT_WEIGHT, answer_confidence, blend_within,
    content_tokens, exact_label, fit_within, hint_tokens, near_surface,
};
use super::{Candidate, RequestKind};
use crate::fetch::Request;
use crate::knowledge::evidence::Identity;
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
) -> Option<Identity> {
    if candidates.is_empty() {
        *reason = "the answer carried no candidate";
        return None;
    }
    let Some(hint) = hint_tokens(entity) else {
        *reason = "no disambiguation hint to fall back on";
        return None;
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
        return None;
    }
    Some(answer::identity(
        best,
        "search_hint",
        answer_confidence(best_score).min(HINT_CEILING),
    ))
}
/// Which rung produced a claim. Recorded on the claim, so it must name the rung that answered and
/// not the shortcut that got there.
fn method_for(kind: &RequestKind, exact_label: bool) -> &'static str {
    match (kind, exact_label) {
        (RequestKind::Title, _) => "wikipedia_title",
        (RequestKind::Search, true) => "search_lookup",
        (RequestKind::Search, false) => "search_ranked",
        (_, true) => "exact_label",
        (_, false) => "ranked",
    }
}

/// Choose a referent from the candidates, or none. Shared by the label path and the search
/// fallback — the only difference between them is which candidates are admitted.
pub(super) fn decide(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<RequestKind>,
    reason: &mut &'static str,
) -> Option<Identity> {
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
    if request.kind == RequestKind::Label
        && candidates.len() == 1
        && exact_label(entity, &candidates[0])
    {
        return Some(answer::identity(
            &candidates[0],
            method_for(&request.kind, true),
            LONE_LABEL_CONFIDENCE,
        ));
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
        return None;
    }

    let best = &candidates[best];
    Some(answer::identity(
        best,
        method_for(&request.kind, exact_label(entity, best)),
        answer_confidence(blend_within(entity, best, candidates)),
    ))
}

/// The article that bears this exact name, offered only after every question that could corroborate
/// was asked and refused. A title maps one article to one name, so a lone answer says nothing about
/// ambiguity — which is why this spoke last: as a first answer it made `Dogma` the doctrine and
/// `Orthodoxy` the concept.
pub(super) fn surface_only(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<RequestKind>,
) -> Option<Identity> {
    if request.kind != RequestKind::Title || candidates.len() != 1 {
        return None;
    }
    let candidate = &candidates[0];
    // The article has to be about our name: Wikipedia redirects otherwise (`The Bounty Hunter Wars`
    // lands on `Boba Fett`), and this is the last place that can be noticed.
    if let Some(label) = candidate.label.as_deref()
        && normalize::comparable(label) != normalize::comparable(&entity.name)
    {
        return None;
    }
    Some(answer::identity(
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
    request: &Request<RequestKind>,
    reason: &mut &'static str,
) -> Option<Identity> {
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
        return None;
    }
    decide(entity, &titled, request, reason)
}

/// A search answer: one corroborated surface is identity, anything else is a ranker question.
pub(super) fn parse_search(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<RequestKind>,
    reason: &mut &'static str,
) -> Option<Identity> {
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
        && fit_within(entity, candidate, candidates) > 0.0
    {
        return Some(answer::identity(
            candidate,
            "search_lookup",
            LOOKUP_CONFIDENCE,
        ));
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
        return by_hint(entity, candidates, reason);
    }
    decide(entity, &admitted, request, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::identity::wikidata::interpret;
    use crate::test_support::fixtures::stub_entity;
    use serde_json::json;

    fn candidate(id: &str, label: &str, description: Option<&str>) -> Candidate {
        Candidate {
            id: id.to_string(),
            label: Some(label.to_string()),
            description: description.map(str::to_string),
            classes: Vec::new(),
        }
    }

    fn request(kind: RequestKind, query: &str) -> Request<RequestKind> {
        Request {
            provider: "wikidata",
            kind,
            shape: "test".to_string(),
            query: query.to_string(),
        }
    }

    fn found(candidates: serde_json::Value) -> crate::fetch::Answer<Vec<Candidate>> {
        crate::fetch::Answer {
            outcome: crate::fetch::Outcome::Found,
            body: Some(serde_json::from_value(candidates).expect("fixture candidates")),
        }
    }

    fn entity(label: &str, description: Option<&str>) -> model::knowledge_entity::Model {
        let mut entity = stub_entity(1, label, "unknown");
        entity.description = description.map(str::to_string);
        entity
    }

    /// The shared fold is the one that decides what "the same name" means: an accent is a spelling,
    /// not a different article. `surface_key` used to keep the accent and miss this.
    #[test]
    fn the_shared_fold_sees_through_an_accent() {
        let entity = stub_entity(1, "Pokemon", "unknown");
        let candidates = vec![candidate("Q1", "Pokémon", None)];
        let asked = request(RequestKind::Title, "Pokemon");
        assert!(
            surface_only(&entity, &candidates, &asked).is_some(),
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

        let claims = decide(
            &entity,
            &candidates,
            &request(RequestKind::Search, "Dune"),
            &mut reason,
        );

        assert!(
            claims.is_some(),
            "the evidence supports a claim, not a refusal: {reason:?}"
        );
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q207189");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("search_ranked")
        );
    }

    // The gates are asserted through `interpret::claim` — the composition the resolver calls — so a
    // rule that only holds when a rung is entered directly cannot pass here.

    /// A Wikipedia title can redirect to a different subject: asking for `The Bounty Hunter Wars`
    /// lands on `Boba Fett`. "One title, one article" then reads as identity, so the answer has to be
    /// about our name before it is even considered.
    #[test]
    fn a_title_about_another_subject_is_refused() {
        let raw = found(json!([{
            "id": "Q51790",
            "label": "Boba Fett",
            "description": "fictional character from Star Wars",
        }]));
        let claims = interpret::claim(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(RequestKind::Title, "The Bounty Hunter Wars"),
            &raw,
        );
        assert!(
            claims.is_none(),
            "a redirect is not an identity: {claims:?}"
        );

        // …while a title whose label is our name still claims.
        let ok = found(json!([{
            "id": "Q16010557",
            "label": "The Bounty Hunter Wars",
            "description": "novel trilogy by K. W. Jeter",
        }]));
        let claims = interpret::claim(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(RequestKind::Title, "The Bounty Hunter Wars"),
            &ok,
        );
        assert!(claims.is_some());
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q16010557");

        // A title answer with no label is not a mismatch — the description still has to corroborate.
        let unlabelled = found(json!([{
            "id": "Q42574",
            "label": null,
            "description": "Canadian filmmaker and explorer (born 1954)",
        }]));
        let claims = interpret::claim(
            &entity(
                "James Cameron",
                Some("Canadian filmmaker who served as executive producer on Point Break"),
            ),
            &request(RequestKind::Title, "James Cameron"),
            &unlabelled,
        );
        assert!(claims.is_some(), "a label-less answer can still be right");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q42574");
    }

    #[test]
    fn an_exact_label_match_is_claimed() {
        let claims = interpret::claim(
            &entity("Train Dreams", None),
            &request(RequestKind::Label, "Train Dreams"),
            &found(json!([{"id": "Q185971", "label": "Train Dreams", "description": "novella"}])),
        );
        assert!(claims.is_some());
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q185971");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("exact_label")
        );
        // A unique exact label is identity even with nothing to corroborate it — but it does not
        // reach the confidence a description-backed claim gets.
        assert!(
            (claims
                .as_ref()
                .expect("a claim")
                .confidence
                .expect("a claim carries confidence")
                - 0.75)
                .abs()
                < 1e-6,
            "{:?}",
            claims
        );
    }

    /// A title answer always carries one candidate, so "only one" is not evidence there: the
    /// `Orthodoxy` article names the concept, and the book is a different page.
    #[test]
    fn a_title_cannot_claim_on_an_exact_label_alone() {
        let claims = interpret::claim(
            &entity(
                "Orthodoxy",
                Some("A 1908 philosophical and apologetic book by G.K. Chesterton"),
            ),
            &request(RequestKind::Title, "Orthodoxy"),
            &found(json!([{
                "id": "Q383258",
                "label": "Orthodoxy",
                "description": "adherence to the actual accepted belief, especially in religion",
            }])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    /// On the label rung, one exact match *is* evidence: the query enumerated every holder of the
    /// string. With nothing to corroborate it — no description on either side — that is identity.
    #[test]
    fn a_lone_label_with_nothing_to_contradict_it_claims() {
        let claims = interpret::claim(
            &entity("Tsoding", None),
            &request(RequestKind::Label, "Tsoding"),
            &found(json!([{"id": "Q1", "label": "Tsoding"}])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("exact_label")
        );
    }

    /// …and a description that shares no content word does not refute it: "British filmmaker" and
    /// "English film director" describe one person without agreeing on a single token.
    #[test]
    fn a_lone_label_claims_even_when_the_descriptions_share_no_word() {
        let claims = interpret::claim(
            &entity(
                "Alfred Hitchcock",
                Some("English film director and producer"),
            ),
            &request(RequestKind::Label, "Alfred Hitchcock"),
            &found(json!([{
                "id": "Q7374",
                "label": "Alfred Hitchcock",
                "description": "British filmmaker (1899-1980)",
            }])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("exact_label")
        );
    }

    #[test]
    fn an_ambiguous_label_is_not_claimed() {
        // The Django case: two items share the surface, so the name cannot decide.
        let claims = interpret::claim(
            &entity("Django", None),
            &request(RequestKind::Label, "Django"),
            &found(json!([
                {"id": "Q1", "label": "Django", "description": "web framework"},
                {"id": "Q2", "label": "Django", "description": "2021 video game"}
            ])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    /// Not the same surface: no claim, however close the spelling. The label stage does not rank.
    #[test]
    fn a_misspelt_label_is_not_claimed() {
        let claims = interpret::claim(
            &entity("Sunset Boulevarde", None),
            &request(RequestKind::Label, "Sunset Boulevarde"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard"}])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    #[test]
    fn a_handle_match_is_stronger_than_a_label_match() {
        let claims = interpret::claim(
            &entity("Tsoding", None),
            &request(RequestKind::Handle("P2002"), "tsoding"),
            &found(json!([{"id": "Q1234", "label": "Tsoding"}])),
        );
        assert!(claims.is_some());
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("P2002")
        );
        assert_eq!(claims.as_ref().expect("a claim").confidence, Some(0.95));
    }

    /// The case the conservative rule used to throw away: eleven candidates share the label, and the
    /// capture's own description picks the 1950 film.
    #[test]
    fn a_description_picks_one_of_many_same_label_candidates() {
        let candidates = json!([
            {"id": "Q1", "label": "Sunset Boulevard", "description": "street in Los Angeles"},
            {"id": "Q2", "label": "Sunset Boulevard", "description": "1950 film noir directed by Billy Wilder"},
            {"id": "Q3", "label": "Sunset Boulevard", "description": "musical by Andrew Lloyd Webber"},
            {"id": "Q4", "label": "Sunset Boulevard", "description": "film directed by someone else"}
        ]);
        let claims = interpret::claim(
            &entity(
                "Sunset Boulevard",
                Some(
                    "1950 American film noir directed by Billy Wilder about a faded silent-screen star",
                ),
            ),
            &request(RequestKind::Label, "Sunset Boulevard"),
            &found(candidates),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q2");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("exact_label")
        );
        assert!(
            claims
                .as_ref()
                .expect("a claim")
                .confidence
                .expect("confidence")
                >= 0.80,
            "{:?}",
            claims
        );
    }

    /// A two-word description that shares one generic word must not win by being short.
    ///
    /// With today's conservative bar the correct candidate is rejected too (it scores 0.546 against
    /// 0.60, because Wikidata's description of George Washington shares only "first" and "president"
    /// with ours). That is a false negative we accept until the thresholds are calibrated against a
    /// labelled sample; the invariant worth locking now is that the painting never wins.
    #[test]
    fn a_short_generic_description_never_wins() {
        let claims = interpret::claim(
            &entity(
                "George Washington",
                Some(
                    "An American military general and statesman who served as the first president of the United States",
                ),
            ),
            &request(RequestKind::Label, "George Washington"),
            &found(json!([
                {"id": "Q79035760", "label": "George Washington", "description": "painting by American"},
                {"id": "Q23", "label": "George Washington", "description": "Founding Father and first U.S. president (1732–1799)"}
            ])),
        );
        assert!(
            claims
                .as_ref()
                .is_none_or(|claim| claim.external_id != "Q79035760"),
            "a one-word match must not carry a claim: {claims:?}"
        );
    }

    /// With no candidate fitting the description, the illumination's hint decides.
    #[test]
    fn the_hint_decides_when_nothing_fits_the_description() {
        let mut view = entity("Stagecoach", Some("a western"));
        view.search_query = Some("Stagecoach 1939 film".to_string());
        let claims = interpret::claim(
            &view,
            &request(RequestKind::Label, "Stagecoach"),
            &found(json!([
                {"id": "Q1", "label": "Stagecoach", "description": "1939 film directed by John Ford"},
                {"id": "Q2", "label": "Stagecoach", "description": "1986 film"},
                {"id": "Q3", "label": "Stagecoach", "description": "a horse-drawn vehicle"}
            ])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q1");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("search_hint")
        );
        assert!(
            claims
                .as_ref()
                .expect("a claim")
                .confidence
                .expect("confidence")
                <= 0.85,
            "hint claims are capped lower"
        );
    }

    /// Two candidates fit our description equally: the hint must not break that tie. A hint that
    /// could pick one is still our own text echoing back, not independent evidence.
    #[test]
    fn the_hint_does_not_break_a_tie_between_qualifying_candidates() {
        let mut view = entity("Dune", Some("1965 science fiction novel by Frank Herbert"));
        view.search_query = Some("Dune novel Frank Herbert".to_string());
        let claims = interpret::claim(
            &view,
            &request(RequestKind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"}
            ])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    /// No description to arbitrate: the tie is honest, so nothing is claimed.
    #[test]
    fn a_near_tie_claims_nothing() {
        let claims = interpret::claim(
            &entity("Dune", None),
            &request(RequestKind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    /// …but with a description it resolves, which is the whole point of holding the text.
    #[test]
    fn a_description_breaks_the_dune_tie() {
        let claims = interpret::claim(
            &entity("Dune", Some("1965 science fiction novel by Frank Herbert")),
            &request(RequestKind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q1");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("exact_label")
        );
    }

    /// Wikidata labels `J. R. R. Tolkien` with spaces; our label has none. Search finds it, and the
    /// surface comparison ignores the difference.
    #[test]
    fn a_search_answer_is_accepted_on_a_punctuation_variant() {
        let claims = interpret::claim(
            &entity(
                "J.R.R. Tolkien",
                Some("English author, philologist, and academic"),
            ),
            &request(RequestKind::Search, "J.R.R. Tolkien"),
            &found(json!([
                {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"},
                {"id": "Q3805090", "label": "J.R.R. Tolkien's The Lord of the Rings, Vol. I", "description": "DOS game"}
            ])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q892");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("search_lookup")
        );
    }

    /// A transliteration difference is a spelling of the same name, not a different name:
    /// `Dostoyevsky` is how Wikidata spells him, `Dostoevsky` is how the capture did.
    #[test]
    fn a_search_answer_with_a_misspelt_label_is_ranked() {
        let claims = interpret::claim(
            &entity(
                "Fyodor Dostoevsky",
                Some(
                    "19th-century Russian novelist and philosopher renowned for psychological mastery",
                ),
            ),
            &request(RequestKind::Search, "Fyodor Dostoevsky"),
            &found(json!([
                {"id": "Q991", "label": "Fyodor Dostoyevsky", "description": "Russian novelist and philosopher (1821–1881)"},
                {"id": "Q136199557", "label": "Fyodor Dostoevsky–Darkness and Dawn", "description": "book, 2024"}
            ])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q991");
    }

    /// The trap: an item whose *label* merely contains our name is a different referent, and its
    /// description overlaps ours precisely because it is about ours. A weak lookup must not become a
    /// weak claim — the search stage accepts a surface match or nothing.
    #[test]
    fn a_search_answer_that_only_mentions_our_name_is_refused() {
        for (name, description, label) in [
            (
                "Royal Navy",
                "the United Kingdom's naval warfare force",
                "Royal Navy fleet aircraft carrier fleet",
            ),
            (
                "Clawdbot",
                "an open-source personal AI agent harness",
                "Clawdbot Pro",
            ),
        ] {
            let claims = interpret::claim(
                &entity(name, Some(description)),
                &request(RequestKind::Search, name),
                &found(json!([{"id": "Q1", "label": label, "description": "something else"}])),
            );
            assert!(claims.is_none(), "{label}: {claims:?}");
        }
    }

    /// A Wikipedia title reaches an item whose Wikidata label is different: the item is
    /// `Symphony No. 7`, the article is `Symphony No. 7 (Sibelius)`.
    #[test]
    fn a_title_reaches_an_item_whose_label_differs() {
        let claims = interpret::claim(
            &entity(
                "Symphony No. 7 (Sibelius)",
                Some("The final published symphony by Finnish composer Jean Sibelius"),
            ),
            &request(RequestKind::Title, "Symphony No. 7 (Sibelius)"),
            &found(json!([{
                "id": "Q261122",
                "label": "Symphony No. 7",
                "description": "symphony by Jean Sibelius",
                "classes": ["Q97363528"]
            }])),
        );
        assert!(claims.is_some(), "{claims:?}");
        assert_eq!(claims.as_ref().expect("a claim").external_id, "Q261122");
        assert_eq!(
            claims.as_ref().expect("a claim").method.as_deref(),
            Some("wikipedia_title")
        );
    }

    /// A title that lands on a disambiguation page is a menu, not a referent.
    #[test]
    fn a_title_that_is_a_disambiguation_page_is_refused() {
        let claims = interpret::claim(
            &entity("John Grant", Some("American singer-songwriter")),
            &request(RequestKind::Title, "John Grant"),
            &found(json!([{
                "id": "Q632837",
                "label": "John Grant",
                "description": "Wikimedia disambiguation page",
                "classes": ["Q4167410"]
            }])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    /// The bug this guard exists for: a *label* answer carrying a human-name disambiguation page was
    /// claimed, because the menu check lived in `parse_title` and the label rung never looked.
    #[test]
    fn a_label_answer_that_is_a_menu_is_refused() {
        let claims = interpret::claim(
            &entity("Robert Chandler", Some("British literary translator")),
            &request(RequestKind::Label, "Robert Chandler"),
            &found(json!([{
                "id": "Q7342857",
                "label": "Robert Chandler",
                "description": "Wikimedia human name disambiguation page",
                "classes": ["Q22808320"]
            }])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }

    /// The subclass is not in every stored class list, so the description has to carry the rule too.
    #[test]
    fn a_menu_with_no_classes_is_refused_by_its_description() {
        let claims = interpret::claim(
            &entity(
                "Rogue Heroes",
                Some("British historical drama television series"),
            ),
            &request(RequestKind::Label, "Rogue Heroes"),
            &found(json!([{
                "id": "Q115697563",
                "label": "Rogue Heroes",
                "description": "Wikimedia disambiguation page",
                "classes": []
            }])),
        );
        assert!(claims.is_none(), "{claims:?}");
    }
}
