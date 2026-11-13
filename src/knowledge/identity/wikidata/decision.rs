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
use crate::knowledge::text::{content_tokens, near_surface};
use crate::model;

use super::ranking::{
    CLAIM_BAR, CLAIM_MARGIN, FIT_MARGIN, HANDLE_CONFIDENCE, HINT_CEILING, HINT_MIN_SUPPORT,
    LONE_LABEL_CONFIDENCE, LOOKUP_CONFIDENCE, NAME_WEIGHT, Ranking, SUPPORT_WEIGHT,
    answer_confidence, blend_within, exact_label, fit_within, hint_tokens,
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
/// **Its score is not [`blend`].** `choose` ranks by IDF-weighted description fit; this ranks by the
/// proportion of hint tokens a candidate's description accounts for, and it deliberately assumes full
/// name credit. That is not an oversight: this function runs only after the name test failed, and the
/// hint *is* the evidence in that case — weighting name agreement here would double-count the failure
/// that brought us here. Measured 2026-11-12: switching it to the real blend lost five identities, all
/// `search_hint`. The ceiling is what keeps the two scales from competing.
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
    use crate::fetch::{Answer, Outcome};
    use crate::test_support::fixtures::stub_entity;
    use serde_json::json;

    // ---- the fixture -------------------------------------------------------------------------

    /// The entity a case asks about: one field per input `decide` reads.
    #[derive(Clone, Copy)]
    struct Probe {
        name: &'static str,
        description: Option<&'static str>,
        entity_type: &'static str,
        hint: Option<&'static str>,
    }

    fn probe(name: &'static str) -> Probe {
        Probe {
            name,
            description: None,
            entity_type: "unknown",
            hint: None,
        }
    }

    impl Probe {
        fn described(self, description: &'static str) -> Self {
            Self {
                description: Some(description),
                ..self
            }
        }

        fn typed(self, entity_type: &'static str) -> Self {
            Self {
                entity_type,
                ..self
            }
        }

        /// The capture's disambiguation hint, which only the fallbacks may read.
        fn hinted(self, hint: &'static str) -> Self {
            Self {
                hint: Some(hint),
                ..self
            }
        }

        fn entity(self) -> model::knowledge_entity::Model {
            let mut entity = stub_entity(1, self.name, self.entity_type);
            entity.description = self.description.map(str::to_string);
            entity.search_query = self.hint.map(str::to_string);
            entity
        }
    }

    fn found(answer: serde_json::Value) -> Answer<Vec<WikidataCandidate>> {
        Answer {
            outcome: Outcome::Found,
            body: Some(serde_json::from_value(answer).expect("fixture candidates")),
        }
    }

    /// What an answer must produce, named as the corpus will see it.
    struct Claim {
        qid: &'static str,
        method: &'static str,
        confidence: Option<f32>,
    }

    /// A claim whose confidence the rule pins.
    fn claim(qid: &'static str, method: &'static str, confidence: f32) -> Claim {
        Claim {
            qid,
            method,
            confidence: Some(confidence),
        }
    }

    /// A claim whose confidence is the calibration's business, not this table's.
    fn ranked(qid: &'static str, method: &'static str) -> Claim {
        Claim {
            qid,
            method,
            confidence: None,
        }
    }

    /// One row of the specification: the question, the answer, and what `decide` must make of it.
    struct Case {
        /// The rule this row pins, in the words someone would use to explain it.
        why: &'static str,
        probe: Probe,
        answer: serde_json::Value,
        claim: Option<Claim>,
    }

    impl Case {
        fn run(self, kind: WikidataKind) {
            match (
                decide(&self.probe.entity(), kind, &found(self.answer)),
                self.claim,
            ) {
                (Ok(identity), Some(claim)) => {
                    assert_eq!(identity.external_id, claim.qid, "{}", self.why);
                    assert_eq!(
                        identity.method.as_deref(),
                        Some(claim.method),
                        "{}",
                        self.why
                    );
                    if let Some(expected) = claim.confidence {
                        let actual = identity.confidence.expect("a claim carries a confidence");
                        assert!((actual - expected).abs() < 1e-3, "{}: {actual}", self.why);
                    }
                }
                (Err(reason), None) => {
                    assert!(!reason.is_empty(), "{}: a refusal says why", self.why)
                }
                (Ok(identity), None) => panic!("{}: claimed {identity:?}", self.why),
                (Err(reason), Some(_)) => panic!("{}: refused with {reason:?}", self.why),
            }
        }
    }

    // ---- the gates ---------------------------------------------------------------------------

    /// Before any rung decides: an answer that never arrived proves nothing, a menu is not a
    /// referent, and the right name can still be the wrong kind of thing. The last gate is
    /// one-sided on purpose — an unrecognised class never rejects.
    #[test]
    fn the_gates() {
        let movie = probe("Dogma").typed("movie").entity();
        let label = probe("Dogma").entity();

        for (why, raw) in [
            (
                "an answer that never came back cannot be admitted",
                Answer::nothing(Outcome::Failed),
            ),
            (
                "a disambiguation page is a menu, whatever else it says",
                found(json!([{ "id": "Q1", "label": "Dogma", "classes": ["Q4167410"] }])),
            ),
            (
                "…and the human-name subclass is a menu too",
                found(json!([{ "id": "Q1", "label": "Dogma", "classes": ["Q22808320"] }])),
            ),
            (
                "…and an unstored menu is caught by its description",
                found(
                    json!([{ "id": "Q1", "label": "Dogma", "description": "Wikimedia disambiguation page" }]),
                ),
            ),
        ] {
            assert!(admit(&movie, &raw).is_err(), "{why}");
        }

        let concept = found(json!([{ "id": "Q1", "label": "Dogma", "classes": ["Q151885"] }]));
        assert!(admit(&movie, &concept).is_err(), "a film is not a concept");

        let unknown = found(json!([{ "id": "Q1", "label": "Dogma", "classes": ["Q99999999"] }]));
        assert!(
            admit(&label, &unknown).is_ok(),
            "an unrecognised class never rejects — the table may be incomplete, but never wrong"
        );
    }

    // ---- handle ------------------------------------------------------------------------------

    /// A property match *is* the identity: the handle space belongs to the property, so there is
    /// nothing to rank and nothing to corroborate.
    #[test]
    fn the_handle_rung() {
        Case {
            why: "the account's item is the account",
            probe: probe("Tsoding"),
            answer: json!([{ "id": "Q1234", "label": "Tsoding" }]),
            claim: Some(claim("Q1234", "P2002", 0.95)),
        }
        .run(WikidataKind::Handle("P2002"));
    }

    // ---- label -------------------------------------------------------------------------------

    /// The label rung asked for *every* holder of the string, so a lone exact label is uniqueness
    /// evidence and needs no corroboration — while ambiguity arrives as a second candidate.
    #[test]
    fn the_label_rung() {
        for case in [
            Case {
                why: "a unique label is the name's own evidence",
                probe: probe("Train Dreams"),
                answer: json!([{ "id": "Q185971", "label": "Train Dreams", "description": "novella" }]),
                claim: Some(claim("Q185971", "exact_label", 0.75)),
            },
            Case {
                why: "…even when the descriptions share no word: 'film director' and 'filmmaker' are one person",
                probe: probe("Alfred Hitchcock").described("English film director and producer"),
                answer: json!([{
                    "id": "Q7374",
                    "label": "Alfred Hitchcock",
                    "description": "British filmmaker (1899-1980)",
                }]),
                claim: Some(claim("Q7374", "exact_label", 0.75)),
            },
            Case {
                why: "two holders of the string: the name cannot decide",
                probe: probe("Django"),
                answer: json!([
                    { "id": "Q1", "label": "Django", "description": "web framework" },
                    { "id": "Q2", "label": "Django", "description": "2021 video game" },
                ]),
                claim: None,
            },
            Case {
                why: "a misspelling is not the label — this rung does not rank",
                probe: probe("Sunset Boulevarde"),
                answer: json!([{ "id": "Q1", "label": "Sunset Boulevard" }]),
                claim: None,
            },
            Case {
                why: "several share the label; the description picks the film",
                probe: probe("Sunset Boulevard")
                    .described("1950 American film noir directed by Billy Wilder about a faded silent-screen star"),
                answer: json!([
                    { "id": "Q1", "label": "Sunset Boulevard", "description": "street in Los Angeles" },
                    { "id": "Q2", "label": "Sunset Boulevard", "description": "1950 film noir directed by Billy Wilder" },
                    { "id": "Q3", "label": "Sunset Boulevard", "description": "musical by Andrew Lloyd Webber" },
                ]),
                claim: Some(ranked("Q2", "exact_label")),
            },
            Case {
                why: "no description to arbitrate, so the tie is honest",
                probe: probe("Dune"),
                answer: json!([
                    { "id": "Q1", "label": "Dune", "description": "1965 novel by Frank Herbert" },
                    { "id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve" },
                ]),
                claim: None,
            },
            Case {
                why: "…and with one it resolves",
                probe: probe("Dune").described("1965 science fiction novel by Frank Herbert"),
                answer: json!([
                    { "id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert" },
                    { "id": "Q2", "label": "Dune", "description": "2021 film by Denis Villeneuve" },
                ]),
                claim: Some(ranked("Q1", "exact_label")),
            },
            Case {
                why: "nothing corroborates, so the entity's own hint decides",
                probe: probe("Stagecoach").described("a western").hinted("Stagecoach 1939 film"),
                answer: json!([
                    { "id": "Q1", "label": "Stagecoach", "description": "1939 film directed by John Ford" },
                    { "id": "Q2", "label": "Stagecoach", "description": "1986 film" },
                    { "id": "Q3", "label": "Stagecoach", "description": "a horse-drawn vehicle" },
                ]),
                claim: Some(ranked("Q1", "search_hint")),
            },
            Case {
                why: "a hint never breaks a tie — it is our own text echoing back",
                probe: probe("Dune")
                    .described("1965 science fiction novel by Frank Herbert")
                    .hinted("Dune novel Frank Herbert"),
                answer: json!([
                    { "id": "Q1", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert" },
                    { "id": "Q2", "label": "Dune", "description": "1965 science fiction novel by Frank Herbert" },
                ]),
                claim: None,
            },
        ] {
            case.run(WikidataKind::Label);
        }
    }

    // ---- title -------------------------------------------------------------------------------

    /// A title's query is our exact name, so the answer has to be about that name — and because a
    /// page maps to one item by construction, an exact surface proves nothing on its own.
    #[test]
    fn the_title_rung() {
        for case in [
            Case {
                why: "a redirect lands on another subject: 'The Bounty Hunter Wars' asks for Boba Fett",
                probe: probe("The Bounty Hunter Wars").described("a trilogy of Star Wars novels"),
                answer: json!([{
                    "id": "Q51790",
                    "label": "Boba Fett",
                    "description": "fictional character from Star Wars",
                }]),
                claim: None,
            },
            Case {
                why: "the article is the concept and the book is a different page, so one exact surface is not enough",
                probe: probe("Orthodoxy")
                    .described("A 1908 philosophical and apologetic book by G.K. Chesterton"),
                answer: json!([{
                    "id": "Q383258",
                    "label": "Orthodoxy",
                    "description": "adherence to the actual accepted belief, especially in religion",
                }]),
                claim: None,
            },
            Case {
                why: "a label-less item is a real answer shape, and the description still has to corroborate",
                probe: probe("James Cameron").described(
                    "Canadian filmmaker who served as executive producer on Point Break",
                ),
                answer: json!([{
                    "id": "Q42574",
                    "label": null,
                    "description": "Canadian filmmaker and explorer (born 1954)",
                }]),
                claim: Some(ranked("Q42574", "wikipedia_title")),
            },
            Case {
                why: "the article title carries a qualifier the item's label does not",
                probe: probe("Symphony No. 7 (Sibelius)")
                    .described("The final published symphony by Finnish composer Jean Sibelius"),
                answer: json!([{
                    "id": "Q261122",
                    "label": "Symphony No. 7",
                    "description": "symphony by Jean Sibelius",
                }]),
                claim: Some(ranked("Q261122", "wikipedia_title")),
            },
            Case {
                why: "a disambiguation page is a menu, not a referent",
                probe: probe("John Grant").described("American singer-songwriter"),
                answer: json!([{
                    "id": "Q632837",
                    "label": "John Grant",
                    "description": "Wikimedia disambiguation page",
                    "classes": ["Q4167410"],
                }]),
                claim: None,
            },
        ] {
            case.run(WikidataKind::Title);
        }
    }

    // ---- search ------------------------------------------------------------------------------

    /// A search result set is text search: one exact surface counts only with corroboration, a
    /// respelling is ranked, and an item whose label merely *contains* our name is another thing.
    #[test]
    fn the_search_rung() {
        for case in [
            Case {
                why: "a punctuation variant is the same name",
                probe: probe("J.R.R. Tolkien").described("English author, philologist, and academic"),
                answer: json!([
                    { "id": "Q892", "label": "J. R. R. Tolkien", "description": "English writer and philologist" },
                    { "id": "Q3805090", "label": "J.R.R. Tolkien's The Lord of the Rings, Vol. I", "description": "DOS game" },
                ]),
                claim: Some(ranked("Q892", "search_lookup")),
            },
            Case {
                why: "a transliteration is a spelling of the same name, so the description ranks it",
                probe: probe("Fyodor Dostoevsky")
                    .described("19th-century Russian novelist and philosopher renowned for psychological mastery"),
                answer: json!([
                    { "id": "Q991", "label": "Fyodor Dostoyevsky", "description": "Russian novelist and philosopher (1821–1881)" },
                    { "id": "Q136199557", "label": "Fyodor Dostoevsky–Darkness and Dawn", "description": "book, 2024" },
                ]),
                claim: Some(ranked("Q991", "search_ranked")),
            },
            Case {
                why: "an item that embeds our name is another referent, however well the text overlaps",
                probe: probe("Royal Navy").described("the United Kingdom's naval warfare force"),
                answer: json!([{
                    "id": "Q1",
                    "label": "Royal Navy fleet aircraft carrier fleet",
                    "description": "something else",
                }]),
                claim: None,
            },
            Case {
                why: "…and a suffix is enough to make it another thing",
                probe: probe("Clawdbot").described("an open-source personal AI agent harness"),
                answer: json!([{
                    "id": "Q1",
                    "label": "Clawdbot Pro",
                    "description": "something else",
                }]),
                claim: None,
            },
        ] {
            case.run(WikidataKind::Search);
        }
    }

    // ---- the deferred surface rule -----------------------------------------------------------

    /// The last resort has no corroboration and no ranking, so its guards are what protect it: one
    /// candidate, and a label that folds to our name. Its method is the title rung's, not a new one.
    #[test]
    fn the_surface_rule() {
        let entity = stub_entity(1, "Pokemon", "unknown");
        let one = vec![WikidataCandidate {
            id: "Q1".to_string(),
            label: Some("Pokémon".to_string()),
            description: None,
            classes: Vec::new(),
        }];

        let identity =
            surface_only(&entity, &one).expect("an accent is a spelling, not an article");
        assert_eq!(
            identity.method.as_deref(),
            Some(method_for(&WikidataKind::Title, true)),
            "the surface path records the title rung's method"
        );

        assert!(
            surface_only(&entity, &[]).is_none(),
            "no candidate, no surface"
        );
        let two = [one[0].clone(), one[0].clone()];
        assert!(
            surface_only(&entity, &two).is_none(),
            "several candidates are ambiguity, which this rule cannot resolve"
        );
    }

    // ---- the shared choice -------------------------------------------------------------------

    /// `choose` orders by the evidence it margins, so the runner-up it compares against is the
    /// closest by that same measure. A blend-ordered comparison once produced a negative margin and
    /// refused a claim the evidence supported.
    #[test]
    fn the_runner_up_is_the_closest_by_the_measure_that_margins() {
        let mut entity = stub_entity(1, "Dune", "book");
        entity.description = Some("1969 novel by Frank Herbert, sequel to Dune".to_string());
        let candidates = vec![
            WikidataCandidate {
                id: "Q191536".to_string(),
                label: Some("Dune".to_string()),
                description: Some("1965 science fiction novel by Frank Herbert".to_string()),
                classes: Vec::new(),
            },
            WikidataCandidate {
                id: "Q207189".to_string(),
                label: Some("Dune Messiah".to_string()),
                description: Some("1969 novel by Frank Herbert, sequel to Dune".to_string()),
                classes: Vec::new(),
            },
        ];

        let identity = choose(&entity, &candidates, WikidataKind::Search)
            .expect("the evidence supports a claim, not a refusal");

        assert_eq!(identity.external_id, "Q207189");
        assert_eq!(identity.method.as_deref(), Some("search_ranked"));
    }

    /// A terse, generic description must not win by being short: the accepted false negative is the
    /// president, and the invariant is that the painting never carries the claim.
    #[test]
    fn a_short_generic_description_never_wins() {
        let entity = probe("George Washington")
            .described(
                "An American military general and statesman who served as the first president of the United States",
            )
            .entity();
        let candidates = found(json!([
            { "id": "Q79035760", "label": "George Washington", "description": "painting by American" },
            { "id": "Q23", "label": "George Washington", "description": "Founding Father and first U.S. president (1732–1799)" },
        ]));

        // The correct candidate sits below the bar today, which is the accepted false negative; what
        // must never happen is the painting carrying the claim.
        if let Ok(identity) = decide(&entity, WikidataKind::Label, &candidates) {
            assert_ne!(identity.external_id, "Q79035760");
        }
    }
}
