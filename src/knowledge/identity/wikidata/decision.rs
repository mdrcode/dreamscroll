//! How an answer becomes an identity: which candidates may be considered, which one wins, and what
//! the win is called.
//!
//! [`decide`] is the only entry. It runs three stages, and a later stage may not re-implement an
//! earlier one:
//!
//! 1. **Gates** ([`admit`]) — a menu is not a referent, and the right name can be the wrong kind of
//!    thing. Shared by every rung, and by the deferred `surface_only` pass.
//! 2. **Admission**, per rung — [`title`] and [`search`] each decide whether a candidate relates to
//!    our name at all; [`handle`] and [`label`] need no rule. The two that do sit next to each other
//!    on purpose: they are the pair that once diverged.
//! 3. **Choice** ([`choose`]) — corroboration, margin, hint fallback. Shared by every rung, so
//!    tuning it moves them together.
//!
//! Each rung has exactly one early exit, matched to what its query guarantees. `label` enumerates
//! every holder of the string, so a lone exact label is identity. `search` may return anything, so
//! one exact surface counts only with corroboration. `title` *is* our exact name, so an exact surface
//! proves nothing there and it has no early exit at all — it must corroborate, which is why
//! `Orthodoxy` cannot claim on its article alone. `handle` needs no decision: the property match is
//! the identity.
//!
//! The two name tests differ, and the difference is measured rather than accidental. A title's query
//! is an exact page title, so the *article* can be our name while the *item* is labelled something
//! else: `Colonel Sanders` is the article and `Harland Sanders` the item; `Ali Khamraev` and `Ali
//! Hamroyev` are one director; `20th Century Fox` became `20th Century Studios`. Any shared content
//! word therefore has to be enough there — tightening `title` to the search rule cost 17 claims, 16 of
//! them correct (measured 2026-11-12).
//!
//! A search hit has no such guarantee: its result set is text search, where a shared word is not
//! evidence. Loosening `search` to the title rule nets six claims but *reassigns twelve referents*,
//! which is a change to judge against a labelled sample, not a tidy-up. So `title` admits generously
//! and `search` strictly, on purpose.
//!
//! Admission follows the same logic. `title` admits on any shared content word because its query is
//! already our exact name; `search` demands near-surface, a qualifier suffix or a content-word subset
//! because its result set is text search and will happily embed our name in something else
//! (`Royal Navy fleet aircraft carrier fleet`).
//!
//! [`surface_only`] is outside the ladder: the deferred exact-surface rule, applied by the resolver
//! after every rung refused. Everything here is pure, a refusal records *which* stage closed so
//! `identity inspect` reports it rather than guessing, and the numbers live in `ranking.rs`.

use crate::fetch::Answer;
use crate::knowledge::evidence::Identity;
use crate::knowledge::normalize;
use crate::knowledge::scoring::Rank;
use crate::model;

use super::ranking::{
    CLAIM_BAR, CLAIM_MARGIN, FIT_MARGIN, HANDLE_CONFIDENCE, HINT_CEILING, HINT_MIN_SUPPORT,
    LONE_LABEL_CONFIDENCE, LOOKUP_CONFIDENCE, NAME_WEIGHT, Ranking, SUPPORT_WEIGHT,
    answer_confidence, blend_within, content_tokens, exact_label, fit_within, hint_tokens,
    near_surface,
};
use super::{WikidataCandidate, WikidataKind, answer, types};

/// Which rung produced an identity. Recorded on the identity, so it must name the rung that answered and
/// not the shortcut that got there.
fn method_for(kind: &WikidataKind, exact_label: bool) -> &'static str {
    match (kind, exact_label) {
        (WikidataKind::Title, _) => "wikipedia_title",
        (WikidataKind::Search, true) => "search_lookup",
        (WikidataKind::Search, false) => "search_ranked",
        (_, true) => "exact_label",
        (_, false) => "ranked",
    }
}

/// The one identity this answer supports for this rung, or the reason there is none — the module's
/// single entry.
///
/// Three stages, in order: [`admit`] (a menu is not a referent, the right name can be the wrong kind
/// of thing), the rung's own admission, then [`choose`]. The `Err` is the refusal reason, recorded at
/// the stage that closed so `identity inspect` reports it rather than guessing.
pub(crate) fn decide(
    entity: &model::knowledge_entity::Model,
    kind: WikidataKind,
    raw: &Answer<Vec<WikidataCandidate>>,
) -> Result<Identity, &'static str> {
    let candidates = admit(entity, raw)?;
    match kind {
        WikidataKind::Handle(property) => Ok(handle(&candidates, property)),
        WikidataKind::Label => label(entity, &candidates),
        WikidataKind::Title => title(entity, &candidates),
        WikidataKind::Search => search(entity, &candidates),
    }
}

/// A property match *is* the identity: an item claiming this handle is the account itself. No
/// admission rule, no ranking — the handle space belongs to the property, and it answered.
///
/// `candidates` is non-empty because [`admit`] refuses an answer that carried nothing, so there is
/// no "no item" branch to write.
fn handle(candidates: &[WikidataCandidate], property: &'static str) -> Identity {
    answer::identity(&candidates[0], property, HANDLE_CONFIDENCE)
}

/// No admission rule on purpose: the label query enumerates *every* holder of the string, so the
/// answer set is already the whole field. That is also what makes a lone exact label evidence here
/// and nowhere else.
fn label(
    entity: &model::knowledge_entity::Model,
    candidates: &[WikidataCandidate],
) -> Result<Identity, &'static str> {
    // One result means the name is unique on Wikidata, so it needs no corroboration: two true
    // descriptions of one person can share no content word, and demanding fit here refuses correct
    // answers. Ambiguity is what the shortcut must not survive, and this rung reports it as more
    // than one candidate.
    if let [candidate] = candidates
        && exact_label(entity, candidate)
    {
        return Ok(answer::identity(
            candidate,
            method_for(&WikidataKind::Label, true),
            LONE_LABEL_CONFIDENCE,
        ));
    }
    choose(entity, candidates, WikidataKind::Label)
}

/// A title maps to one article. A disambiguation page maps to none: it is a menu, and taking
/// it is the trap that produced `Royal Navy` → an aircraft-carrier fleet.
fn title(
    entity: &model::knowledge_entity::Model,
    candidates: &[WikidataCandidate],
) -> Result<Identity, &'static str> {
    // A title is a *name* lookup, so the answer has to be about that name. Wikipedia redirects
    // otherwise: asking for `The Bounty Hunter Wars` lands on the article for `Boba Fett`, and
    // "one title, one article" then reads as identity. Sharing no word with our name is not an
    // identity, whatever the title said.
    let ours = content_tokens(&entity.name);
    let ours_key = normalize::comparable(&entity.name);
    let titled: Vec<WikidataCandidate> = candidates
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
        return Err("the title is about another subject");
    }
    choose(entity, &titled, WikidataKind::Title)
}

/// A search answer: one corroborated surface is identity, anything else is a ranker question.
fn search(
    entity: &model::knowledge_entity::Model,
    candidates: &[WikidataCandidate],
) -> Result<Identity, &'static str> {
    let ours = normalize::comparable(&entity.name);
    let same: Vec<&WikidataCandidate> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .label
                .as_deref()
                .is_some_and(|label| normalize::comparable(label) == ours)
        })
        .collect();
    // A matching surface is not enough — the description has to corroborate it, exactly as on
    // the label path. Without this, `Audi` (a car brand) matches a *family name* and
    // `A Pattern Language` matches a 2011 master's thesis, both of which match the surface.
    //
    // Anything short of one corroborated surface falls through to the ranker: *zero* is a
    // spelling difference (`J.R.R.` vs `J. R. R.`), *several* is a shared title where only the
    // description can pick the referent.
    if let [candidate] = same.as_slice()
        && fit_within(entity, candidate, candidates) > 0.0
    {
        return Ok(answer::identity(
            candidate,
            method_for(&WikidataKind::Search, true),
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
    // Admission is not an identification. The description must still corroborate — a non-zero fit — and beat the
    // runner-up, which is what keeps `Royal Navy fleet aircraft carrier fleet` off `Royal Navy`.
    let mine = content_tokens(&entity.name);
    let admitted: Vec<WikidataCandidate> = candidates
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
        // The name nominated nothing, but the hint may still: `Sten Mk IIS suppressed` is a model
        // whose label is just `Sten`, and no name rule will connect the two. Admission is a name
        // test, so the hint path has to be reached before it turns the answer down.
        return by_hint(entity, candidates);
    }
    choose(entity, &admitted, WikidataKind::Search)
}

/// The gates every answer passes before a rung decides: a menu is not a referent, and the right
/// name can still be the wrong kind of thing. Returns the surviving candidates, or the reason the
/// answer was rejected outright. Only recognised classes reject — an unknown one passes (see
/// `types`), so the table being incomplete cannot lose a candidate.
///
/// Split out because the last-resort pass needs the same gates without a rung's decision.
pub(crate) fn admit(
    entity: &model::knowledge_entity::Model,
    raw: &Answer<Vec<WikidataCandidate>>,
) -> Result<Vec<WikidataCandidate>, &'static str> {
    if !raw.outcome.answered() {
        return Err("the answer carried no candidate");
    }
    let mut candidates = raw.body.clone().unwrap_or_default();
    // A menu is not a referent, on *every* rung. This used to be checked in `parse_title` only,
    // and the label rung — which is where all 34 such annotations in the corpus came from — never
    // looked. Filtering at the gate makes the rule a property of an answer rather than of one
    // caller.
    candidates.retain(|candidate| !answer::is_menu(candidate));
    if candidates.is_empty() {
        return Err("the answer carried only a disambiguation page");
    }
    // `Dogma` the film against the concept, `Micah` the prophet against the given name.
    candidates.retain(|candidate| !types::incompatible(&entity.entity_type, &candidate.classes));
    if candidates.is_empty() {
        return Err("every candidate is a different kind of thing");
    }
    Ok(candidates)
}

/// Choose a referent from the candidates, or say why none fits. Shared by every rung: the only
/// difference between them is which candidates reach it, so `kind` is needed here only to name the
/// method that won.
fn choose(
    entity: &model::knowledge_entity::Model,
    candidates: &[WikidataCandidate],
    kind: WikidataKind,
) -> Result<Identity, &'static str> {
    // The verb: the candidates whose description corroborates, best fit first.
    let ranked = Ranking.rank(entity, candidates);
    let Some(&(best, best_fit)) = ranked.first() else {
        return by_hint(entity, candidates);
    };

    // `Rank` orders by `fit`, so the winner and the runner-up are measured in one order and the
    // subtraction below is a gap between comparable quantities. It used to be ordered by the blend
    // while this margin read `fit`, so a runner-up with *more* description evidence than the winner
    // produced a negative gap — and the identity was refused with "neither wins" while holding a winner.
    let runner_up = ranked.get(1).map(|(_, fit)| *fit).unwrap_or(0.0);

    // Otherwise the winner must beat the runner-up. Ambiguity is not a reason to give up —
    // `Sunset Boulevard` has eleven candidates and the description picks the film — but a near-tie
    // is.
    if best_fit - runner_up < FIT_MARGIN {
        return Err("two candidates fit and neither wins");
    }

    let best = &candidates[best];
    Ok(answer::identity(
        best,
        method_for(&kind, exact_label(entity, best)),
        answer_confidence(blend_within(entity, best, candidates)),
    ))
}

/// The entity's own disambiguation hint, consulted only after the name rules and the descriptions
/// have both refused — so it can never overrule a description that resolved the case. It is our own
/// text echoing back, so its identities are capped lower ([`HINT_CEILING`]). The `Sunset Boulevard`
/// hint once lifted the musical above the 1950 film; the ordering is what prevents a repeat.
///
/// **Its score is not the blend.** [`choose`] ranks by IDF-weighted description fit; this ranks by
/// the proportion of hint tokens a candidate's description accounts for — a different quantity on a
/// different range. The ceiling is what keeps them from competing, not comparability.
///
/// `candidates` is non-empty. From [`choose`] it is the admitted set; from [`search`] it is the
/// *unadmitted* one, because the hint is scored against descriptions and the name test must not be
/// allowed to veto it.
fn by_hint(
    entity: &model::knowledge_entity::Model,
    candidates: &[WikidataCandidate],
) -> Result<Identity, &'static str> {
    let Some(hint) = hint_tokens(entity) else {
        return Err("no disambiguation hint to fall back on");
    };
    // A candidate, its blended score, and the proportion of the hint it accounts for. Not `Ranked`:
    // that type's `fit` means "description evidence", and an earlier version smuggled the support
    // proportion into it, so the same field meant two things on two paths.
    let mut scored: Vec<(&WikidataCandidate, f32, f32)> = candidates
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
        return Err("the hint did not clear its bar");
    }
    Ok(answer::identity(
        best,
        "search_hint",
        answer_confidence(best_score).min(HINT_CEILING),
    ))
}

/// The article that bears this exact name, offered only after every question that could corroborate
/// was asked and refused. A title maps one article to one name, so a lone answer says nothing about
/// ambiguity — which is why this spoke last: as a first answer it made `Dogma` the doctrine and
/// `Orthodoxy` the concept.
pub(crate) fn surface_only(
    entity: &model::knowledge_entity::Model,
    candidates: &[WikidataCandidate],
) -> Option<Identity> {
    // No rung check: the only caller is the deferred title step, whose request is a title request by
    // construction. A guard here would be a comment that cannot fail.
    if candidates.len() != 1 {
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
        method_for(&WikidataKind::Title, true),
        LONE_LABEL_CONFIDENCE,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Request;
    use crate::test_support::fixtures::stub_entity;
    use serde_json::json;

    /// The ladder hands over a request; tests say the same thing with the query spelled out.
    fn identify(
        entity: &model::knowledge_entity::Model,
        request: &Request<WikidataKind>,
        raw: &Answer<Vec<WikidataCandidate>>,
    ) -> Result<Identity, &'static str> {
        decide(entity, request.kind, raw)
    }

    fn candidate(id: &str, label: &str, description: Option<&str>) -> WikidataCandidate {
        WikidataCandidate {
            id: id.to_string(),
            label: Some(label.to_string()),
            description: description.map(str::to_string),
            classes: Vec::new(),
        }
    }

    fn request(kind: WikidataKind, query: &str) -> Request<WikidataKind> {
        Request {
            provider: "wikidata",
            kind,
            shape: "test".to_string(),
            query: query.to_string(),
        }
    }

    fn found(candidates: serde_json::Value) -> crate::fetch::Answer<Vec<WikidataCandidate>> {
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
        assert!(
            surface_only(&entity, &candidates).is_some(),
            "a spelling difference is not a different article"
        );
    }

    /// The surface path records the title method through `method_for`, so it cannot drift from the
    /// name a decided title writes. Both halves are asserted: the link, and the stored vocabulary.
    #[test]
    fn the_surface_path_records_the_title_method() {
        let entity = stub_entity(1, "Pokemon", "unknown");
        let candidates = vec![candidate("Q1", "Pokémon", None)];

        let identity = surface_only(&entity, &candidates).expect("an identity");

        assert_eq!(
            identity.method.as_deref(),
            Some(method_for(&WikidataKind::Title, true))
        );
        assert_eq!(identity.method.as_deref(), Some("wikipedia_title"));
    }

    /// The winner and the runner-up must be measured in one order.
    ///
    /// This is the case that exposed the split: the exact-label candidate wins the *blend* on its
    /// name (0.820) even though the runner-up fits the description better (0.842 against 0.700).
    /// Comparing a blend-ordered winner against an evidence-ordered runner-up made the margin
    /// negative and refused an identity the evidence supports.
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
        let identity = choose(&entity, &candidates, WikidataKind::Search);

        assert!(
            identity.is_ok(),
            "the evidence supports an identity, not a refusal: {identity:?}"
        );
        assert_eq!(
            identity.as_ref().expect("an identity").external_id,
            "Q207189"
        );
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("search_ranked")
        );
    }

    // The gates are asserted through `identify` — the composition the resolver calls — so a
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
        let identity = identify(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(WikidataKind::Title, "The Bounty Hunter Wars"),
            &raw,
        );
        assert!(
            identity.is_err(),
            "a redirect is not an identity: {identity:?}"
        );

        // …while a title whose label is our name still identifies.
        let ok = found(json!([{
            "id": "Q16010557",
            "label": "The Bounty Hunter Wars",
            "description": "novel trilogy by K. W. Jeter",
        }]));
        let identity = identify(
            &entity(
                "The Bounty Hunter Wars",
                Some("a trilogy of Star Wars novels"),
            ),
            &request(WikidataKind::Title, "The Bounty Hunter Wars"),
            &ok,
        );
        assert!(identity.is_ok());
        assert_eq!(
            identity.as_ref().expect("an identity").external_id,
            "Q16010557"
        );

        // A title answer with no label is not a mismatch — the description still has to corroborate.
        let unlabelled = found(json!([{
            "id": "Q42574",
            "label": null,
            "description": "Canadian filmmaker and explorer (born 1954)",
        }]));
        let identity = identify(
            &entity(
                "James Cameron",
                Some("Canadian filmmaker who served as executive producer on Point Break"),
            ),
            &request(WikidataKind::Title, "James Cameron"),
            &unlabelled,
        );
        assert!(identity.is_ok(), "a label-less answer can still be right");
        assert_eq!(
            identity.as_ref().expect("an identity").external_id,
            "Q42574"
        );
    }

    #[test]
    fn an_exact_label_match_is_claimed() {
        let identity = identify(
            &entity("Train Dreams", None),
            &request(WikidataKind::Label, "Train Dreams"),
            &found(json!([{"id": "Q185971", "label": "Train Dreams", "description": "novella"}])),
        );
        assert!(identity.is_ok());
        assert_eq!(
            identity.as_ref().expect("an identity").external_id,
            "Q185971"
        );
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("exact_label")
        );
        // A unique exact label is identity even with nothing to corroborate it — but it does not
        // reach the confidence a description-backed identity gets.
        assert!(
            (identity
                .as_ref()
                .expect("an identity")
                .confidence
                .expect("an identity carries confidence")
                - 0.75)
                .abs()
                < 1e-6,
            "{:?}",
            identity
        );
    }

    /// A title answer always carries one candidate, so "only one" is not evidence there: the
    /// `Orthodoxy` article names the concept, and the book is a different page.
    #[test]
    fn a_title_cannot_claim_on_an_exact_label_alone() {
        let identity = identify(
            &entity(
                "Orthodoxy",
                Some("A 1908 philosophical and apologetic book by G.K. Chesterton"),
            ),
            &request(WikidataKind::Title, "Orthodoxy"),
            &found(json!([{
                "id": "Q383258",
                "label": "Orthodoxy",
                "description": "adherence to the actual accepted belief, especially in religion",
            }])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    /// On the label rung, one exact match *is* evidence: the query enumerated every holder of the
    /// string. With nothing to corroborate it — no description on either side — that is identity.
    #[test]
    fn a_lone_label_with_nothing_to_contradict_it_claims() {
        let identity = identify(
            &entity("Tsoding", None),
            &request(WikidataKind::Label, "Tsoding"),
            &found(json!([{"id": "Q1", "label": "Tsoding"}])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("exact_label")
        );
    }

    /// …and a description that shares no content word does not refute it: "British filmmaker" and
    /// "English film director" describe one person without agreeing on a single token.
    #[test]
    fn a_lone_label_claims_even_when_the_descriptions_share_no_word() {
        let identity = identify(
            &entity(
                "Alfred Hitchcock",
                Some("English film director and producer"),
            ),
            &request(WikidataKind::Label, "Alfred Hitchcock"),
            &found(json!([{
                "id": "Q7374",
                "label": "Alfred Hitchcock",
                "description": "British filmmaker (1899-1980)",
            }])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("exact_label")
        );
    }

    #[test]
    fn an_ambiguous_label_is_not_claimed() {
        // The Django case: two items share the surface, so the name cannot decide.
        let identity = identify(
            &entity("Django", None),
            &request(WikidataKind::Label, "Django"),
            &found(json!([
                {"id": "Q1", "label": "Django", "description": "web framework"},
                {"id": "Q2", "label": "Django", "description": "2021 video game"}
            ])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    /// Not the same surface: no identity, however close the spelling. The label stage does not rank.
    #[test]
    fn a_misspelt_label_is_not_claimed() {
        let identity = identify(
            &entity("Sunset Boulevarde", None),
            &request(WikidataKind::Label, "Sunset Boulevarde"),
            &found(json!([{"id": "Q1", "label": "Sunset Boulevard"}])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    #[test]
    fn a_handle_match_is_stronger_than_a_label_match() {
        let identity = identify(
            &entity("Tsoding", None),
            &request(WikidataKind::Handle("P2002"), "tsoding"),
            &found(json!([{"id": "Q1234", "label": "Tsoding"}])),
        );
        assert!(identity.is_ok());
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("P2002")
        );
        assert_eq!(
            identity.as_ref().expect("an identity").confidence,
            Some(0.95)
        );
    }

    /// The case the conservative rule used to throw away: eleven candidates share the label, and the
    /// entity's own description picks the 1950 film.
    #[test]
    fn a_description_picks_one_of_many_same_label_candidates() {
        let candidates = json!([
            {"id": "Q1", "label": "Sunset Boulevard", "description": "street in Los Angeles"},
            {"id": "Q2", "label": "Sunset Boulevard", "description": "1950 film noir directed by Billy Wilder"},
            {"id": "Q3", "label": "Sunset Boulevard", "description": "musical by Andrew Lloyd Webber"},
            {"id": "Q4", "label": "Sunset Boulevard", "description": "film directed by someone else"}
        ]);
        let identity = identify(
            &entity(
                "Sunset Boulevard",
                Some(
                    "1950 American film noir directed by Billy Wilder about a faded silent-screen star",
                ),
            ),
            &request(WikidataKind::Label, "Sunset Boulevard"),
            &found(candidates),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(identity.as_ref().expect("an identity").external_id, "Q2");
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("exact_label")
        );
        assert!(
            identity
                .as_ref()
                .expect("an identity")
                .confidence
                .expect("confidence")
                >= 0.80,
            "{:?}",
            identity
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
        let identity = identify(
            &entity(
                "George Washington",
                Some(
                    "An American military general and statesman who served as the first president of the United States",
                ),
            ),
            &request(WikidataKind::Label, "George Washington"),
            &found(json!([
                {"id": "Q79035760", "label": "George Washington", "description": "painting by American"},
                {"id": "Q23", "label": "George Washington", "description": "Founding Father and first U.S. president (1732–1799)"}
            ])),
        );
        assert!(
            identity
                .as_ref()
                .map_or(true, |found| found.external_id != "Q79035760"),
            "a one-word match must not carry an identity: {identity:?}"
        );
    }

    /// With no candidate fitting the description, the illumination's hint decides.
    #[test]
    fn the_hint_decides_when_nothing_fits_the_description() {
        let mut view = entity("Stagecoach", Some("a western"));
        view.search_query = Some("Stagecoach 1939 film".to_string());
        let identity = identify(
            &view,
            &request(WikidataKind::Label, "Stagecoach"),
            &found(json!([
                {"id": "Q1", "label": "Stagecoach", "description": "1939 film directed by John Ford"},
                {"id": "Q2", "label": "Stagecoach", "description": "1986 film"},
                {"id": "Q3", "label": "Stagecoach", "description": "a horse-drawn vehicle"}
            ])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(identity.as_ref().expect("an identity").external_id, "Q1");
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("search_hint")
        );
        assert!(
            identity
                .as_ref()
                .expect("an identity")
                .confidence
                .expect("confidence")
                <= 0.85,
            "hint identities are capped lower"
        );
    }

    /// Two candidates fit our description equally: the hint must not break that tie. A hint that
    /// could pick one is still our own text echoing back, not independent evidence.
    #[test]
    fn the_hint_does_not_break_a_tie_between_qualifying_candidates() {
        let mut view = entity("Dune", Some("1965 science fiction novel by Frank Herbert"));
        view.search_query = Some("Dune novel Frank Herbert".to_string());
        let identity = identify(
            &view,
            &request(WikidataKind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"}
            ])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    /// No description to arbitrate: the tie is honest, so nothing is claimed.
    #[test]
    fn a_near_tie_claims_nothing() {
        let identity = identify(
            &entity("Dune", None),
            &request(WikidataKind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    /// …but with a description it resolves, which is the whole point of holding the text.
    #[test]
    fn a_description_breaks_the_dune_tie() {
        let identity = identify(
            &entity("Dune", Some("1965 science fiction novel by Frank Herbert")),
            &request(WikidataKind::Label, "Dune"),
            &found(json!([
                {"id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert"},
                {"id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve"}
            ])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(identity.as_ref().expect("an identity").external_id, "Q1");
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("exact_label")
        );
    }

    /// Wikidata labels `J. R. R. Tolkien` with spaces; our label has none. Search finds it, and the
    /// surface comparison ignores the difference.
    #[test]
    fn a_search_answer_is_accepted_on_a_punctuation_variant() {
        let identity = identify(
            &entity(
                "J.R.R. Tolkien",
                Some("English author, philologist, and academic"),
            ),
            &request(WikidataKind::Search, "J.R.R. Tolkien"),
            &found(json!([
                {"id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist"},
                {"id": "Q3805090", "label": "J.R.R. Tolkien's The Lord of the Rings, Vol. I", "description": "DOS game"}
            ])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(identity.as_ref().expect("an identity").external_id, "Q892");
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("search_lookup")
        );
    }

    /// A transliteration difference is a spelling of the same name, not a different name:
    /// `Dostoyevsky` is how Wikidata spells him, `Dostoevsky` is how the entity did.
    #[test]
    fn a_search_answer_with_a_misspelt_label_is_ranked() {
        let identity = identify(
            &entity(
                "Fyodor Dostoevsky",
                Some(
                    "19th-century Russian novelist and philosopher renowned for psychological mastery",
                ),
            ),
            &request(WikidataKind::Search, "Fyodor Dostoevsky"),
            &found(json!([
                {"id": "Q991", "label": "Fyodor Dostoyevsky", "description": "Russian novelist and philosopher (1821–1881)"},
                {"id": "Q136199557", "label": "Fyodor Dostoevsky–Darkness and Dawn", "description": "book, 2024"}
            ])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(identity.as_ref().expect("an identity").external_id, "Q991");
    }

    /// The trap: an item whose *label* merely contains our name is a different referent, and its
    /// description overlaps ours precisely because it is about ours. A weak lookup must not become a
    /// weak identity — the search stage accepts a surface match or nothing.
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
            let identity = identify(
                &entity(name, Some(description)),
                &request(WikidataKind::Search, name),
                &found(json!([{"id": "Q1", "label": label, "description": "something else"}])),
            );
            assert!(identity.is_err(), "{label}: {identity:?}");
        }
    }

    /// A Wikipedia title reaches an item whose Wikidata label is different: the item is
    /// `Symphony No. 7`, the article is `Symphony No. 7 (Sibelius)`.
    #[test]
    fn a_title_reaches_an_item_whose_label_differs() {
        let identity = identify(
            &entity(
                "Symphony No. 7 (Sibelius)",
                Some("The final published symphony by Finnish composer Jean Sibelius"),
            ),
            &request(WikidataKind::Title, "Symphony No. 7 (Sibelius)"),
            &found(json!([{
                "id": "Q261122",
                "label": "Symphony No. 7",
                "description": "symphony by Jean Sibelius",
                "classes": ["Q97363528"]
            }])),
        );
        assert!(identity.is_ok(), "{identity:?}");
        assert_eq!(
            identity.as_ref().expect("an identity").external_id,
            "Q261122"
        );
        assert_eq!(
            identity.as_ref().expect("an identity").method.as_deref(),
            Some("wikipedia_title")
        );
    }

    /// A title that lands on a disambiguation page is a menu, not a referent.
    #[test]
    fn a_title_that_is_a_disambiguation_page_is_refused() {
        let identity = identify(
            &entity("John Grant", Some("American singer-songwriter")),
            &request(WikidataKind::Title, "John Grant"),
            &found(json!([{
                "id": "Q632837",
                "label": "John Grant",
                "description": "Wikimedia disambiguation page",
                "classes": ["Q4167410"]
            }])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    /// The bug this guard exists for: a *label* answer carrying a human-name disambiguation page was
    /// claimed, because the menu check lived in `parse_title` and the label rung never looked.
    #[test]
    fn a_label_answer_that_is_a_menu_is_refused() {
        let identity = identify(
            &entity("Robert Chandler", Some("British literary translator")),
            &request(WikidataKind::Label, "Robert Chandler"),
            &found(json!([{
                "id": "Q7342857",
                "label": "Robert Chandler",
                "description": "Wikimedia human name disambiguation page",
                "classes": ["Q22808320"]
            }])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }

    /// The subclass is not in every stored class list, so the description has to carry the rule too.
    #[test]
    fn a_menu_with_no_classes_is_refused_by_its_description() {
        let identity = identify(
            &entity(
                "Rogue Heroes",
                Some("British historical drama television series"),
            ),
            &request(WikidataKind::Label, "Rogue Heroes"),
            &found(json!([{
                "id": "Q115697563",
                "label": "Rogue Heroes",
                "description": "Wikimedia disambiguation page",
                "classes": []
            }])),
        );
        assert!(identity.is_err(), "{identity:?}");
    }
}
