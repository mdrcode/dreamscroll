//! The decision: which ranked candidate becomes the identity, or why none does.
//!
//! `ranking` produces the order; this applies the rules that turn an order into a verdict — who is
//! eligible at all, whether the best has enough evidence on its own, and whether it wins by a
//! margin. A refusal is a first-class answer with a reason, not an empty one.

use crate::knowledge::evidence::Identity;
use crate::model;

use super::answer;
use super::gather::CandidateSet;
use super::ranking::{self, Ranked};

/// What a candidate from a guessing question must look like at minimum: a prefix, not a
/// rearrangement. Exact queries (handle, title, label) are held to a lower bar — one shared word —
/// because the question itself already named the thing.
const SEARCH_FLOOR: f32 = 0.7;
/// The floor for a candidate an exact query produced: the same words at minimum.
const EXACT_FLOOR: f32 = 0.6;
/// When two candidates bear the same surface, their descriptions must separate them by this much.
const MARGIN: f32 = 0.05;
/// A claim needs a minimum of evidence, not just a lead over the runner-up: with a set of one,
/// there is no runner-up to lose to, and a surface-only candidate would assert itself.
const SCORE_FLOOR: f32 = 0.50;

/// The one identity the candidate set supports, or the reason there is none.
pub(crate) fn decide(
    entity: &model::knowledge_entity::Model,
    candidate_set: &CandidateSet,
) -> Result<Identity, &'static str> {
    // A profile is identified by its platform link, never by a name. Measured 2026-11-13: of 26
    // `social_media_account` entities carrying a Wikidata identity, one was right — the handle
    // match — and 25 were another bearer of the same word (`Bear` → a gay-culture term, `Blue` → a
    // colour, `Drake` → a botanist). A profile's description says what it posts, never who it is,
    // so no name-based question can answer for it. The handle strategy claims before the set is
    // asked, which is why refusing here leaves exactly the handle match standing.
    if account_only(entity) {
        return Err("a profile is identified by its platform link, not by a name");
    }

    let ranked = ranking::rank(entity, candidate_set);
    // A label-less candidate has no name to compare; the exact query is what vouches for it.
    //
    // A label match with no description corroboration is a name, not a fact: it is eligible only
    // when an article-title answer also produced it. That is the difference between `Industrial
    // Revolution` (a title and a label agree) and a homonym that merely shares the string.
    let eligible = |r: &Ranked| {
        let named = r.label_less && r.tier < 2
            || r.surface
                >= if r.tier == 2 {
                    SEARCH_FLOOR
                } else {
                    EXACT_FLOOR
                };
        named && (r.tier != 1 || r.fit > 0.0 || r.label_less)
    };
    let ranked: Vec<Ranked> = ranked.into_iter().filter(eligible).collect();
    let Some(best) = ranked.first().copied() else {
        return Err("no candidate bears our name");
    };
    if ranking::score(&best) < SCORE_FLOOR {
        return Err("the evidence is too thin to assert");
    }
    if let Some(second) = ranked.get(1)
        && ranking::score(&best) - ranking::score(second) < MARGIN
    {
        return Err("two candidates fit and neither wins");
    }
    let candidate = &candidate_set[best.index].candidate;
    Ok(answer::identity(
        candidate,
        ranking::method(best.strategy, best.surface),
        ranking::confidence(&best),
    ))
}

/// Whether every type the entity wears is `social_media_account`: a profile, which a name cannot
/// identify.
fn account_only(entity: &model::knowledge_entity::Model) -> bool {
    entity
        .types()
        .all(|entity_type| entity_type == "social_media_account")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::identity::wikidata::gather::{Aggregate, Source};
    use crate::knowledge::identity::wikidata::{WikidataCandidate, plan};

    fn candidate(id: &str, label: &str, description: &str) -> Aggregate {
        Aggregate {
            candidate: WikidataCandidate {
                id: id.to_string(),
                label: Some(label.to_string()),
                description: Some(description.to_string()),
                classes: Vec::new(),
            },
            sources: vec![Source {
                strategy: plan::Strategy::SearchLabel,
                position: 0,
            }],
        }
    }

    fn entity(name: &str, description: &str, hint: Option<&str>) -> model::knowledge_entity::Model {
        let mut entity = crate::test_support::fixtures::stub_entity(1, name, "unknown");
        entity.description = Some(description.to_string());
        entity.search_query = hint.map(str::to_string);
        entity
    }

    /// The failure this module exists for: the right item's label is the *expansion* of our
    /// acronym, so it shares no token with the name — but it is the hint verbatim. The division
    /// shares `bbc` with the name and must still lose.
    #[test]
    fn an_expansion_of_the_name_beats_a_division_that_shares_it() {
        let entity = entity(
            "BBC",
            "The public service broadcaster of the United Kingdom.",
            Some("British Broadcasting Corporation"),
        );
        let candidate_set = vec![
            candidate(
                "Q1160945",
                "BBC News",
                "news division of the British Broadcasting Corporation",
            ),
            candidate(
                "Q9531",
                "British Broadcasting Corporation",
                "British public service broadcaster",
            ),
        ];
        let identity = decide(&entity, &candidate_set).expect("the expansion is the name");
        assert_eq!(identity.external_id, "Q9531");
    }

    /// An exact label needs no description corroboration. `Industrial Revolution`'s description is
    /// phrased differently from ours, which used to score zero and hand the claim to a book *about*
    /// the revolution.
    #[test]
    fn an_exact_label_needs_no_description_overlap() {
        let entity = entity(
            "Industrial Revolution",
            "The period of global socioeconomic transformation from agrarian economies to mechanized production.",
            None,
        );
        // From the title answer: an exact label is a name, and a title answer vouches for it
        // without description corroboration. The same candidate found by search would not.
        let labelled = |id: &str, label: &str, description: &str| {
            let mut candidate = candidate(id, label, description);
            candidate.sources[0].strategy = plan::Strategy::Title;
            candidate
        };
        let candidate_set = vec![
            labelled(
                "Q4787186",
                "Buildings of Scotland in the Industrial Revolution",
                "Buildings of Scotland in the Industrial Revolution",
            ),
            labelled(
                "Q2269",
                "Industrial Revolution",
                "transition to new manufacturing processes in Europe and the United States",
            ),
        ];
        let identity = decide(&entity, &candidate_set).expect("the exact label wins");
        assert_eq!(identity.external_id, "Q2269");
    }

    /// Same surface on both sides: the description has to separate them.
    #[test]
    fn a_tie_on_surface_is_refused() {
        let entity = entity("Dune", "1965 science fiction novel by Frank Herbert", None);
        let candidate_set = vec![
            candidate("Q1", "Dune", "novel by Frank Herbert"),
            candidate("Q2", "Dune", "novel by Frank Herbert"),
        ];
        assert_eq!(
            decide(&entity, &candidate_set).unwrap_err(),
            "two candidates fit and neither wins"
        );
    }

    /// A profile may not be identified by a name, however well the words match — and an entity that
    /// is also a person is not only a profile.
    #[test]
    fn a_profile_needs_a_platform_link() {
        let mut account = entity("Bear", "An account on X discussing military history.", None);
        account.entity_type = "social_media_account".to_string();
        let candidate_set = vec![candidate("Q618804", "bear", "term in gay culture")];
        assert_eq!(
            decide(&account, &candidate_set).unwrap_err(),
            "a profile is identified by its platform link, not by a name"
        );

        let mut person = entity("Sama Hoole", "A researcher and commentator.", None);
        person.entity_type = "social_media_account".to_string();
        person.alt_types = vec!["real_person".to_string()];
        let candidate_set = vec![candidate("Q1", "Sama Hoole", "researcher")];
        assert!(
            decide(&person, &candidate_set).is_ok(),
            "a person is not only a profile"
        );
    }
}
