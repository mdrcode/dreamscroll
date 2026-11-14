//! The decision: which ranked candidate becomes the identity, or why none does.
//!
//! `ranking` produces the order; this applies the rules that turn an order into a verdict — who is
//! eligible at all, whether the best has enough evidence on its own, and whether it wins by a
//! margin. A refusal is a first-class answer with a reason, not an empty one.

use crate::knowledge::evidence::Identity;
use crate::model;

use super::gather::CandidateSet;
use super::ranking::{self, Ranked};

/// What a candidate from a guessing question must look like at minimum: a prefix, not a
/// rearrangement. Exact queries (handle, title, label) are held to a lower bar — one shared word —
/// because the question itself already named the thing.
const SEARCH_FLOOR: f32 = 0.7;
/// The floor for a candidate an exact query produced: the same words at minimum.
const EXACT_FLOOR: f32 = 0.6;
/// Two candidates that fit must be separated by this much, or neither is chosen.
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
    // A label match is a name, not a fact: it needs corroboration besides the string itself —
    // a description that overlaps ours, or an article whose exact title is our name. Search
    // agreement was tried as a third source and rejected by measurement: it admits the homonyms
    // the rule exists for, and two eligible same-label candidates then tie on the margin and
    // refuse a referent that was previously found (`Alan Watts`).
    let eligible = |r: &Ranked| {
        let named = r.label_less && r.tier < 2
            || r.surface
                >= if r.tier == 2 {
                    SEARCH_FLOOR
                } else {
                    EXACT_FLOOR
                };
        named && (r.tier != 1 || r.fit > 0.0 || r.titled || r.label_less)
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
    Ok(candidate.identity(
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
    use crate::knowledge::identity::wikidata::gather::Aggregate;
    use crate::knowledge::identity::wikidata::{WikidataCandidate, plan};

    fn candidate(id: &str, label: &str, description: &str) -> Aggregate {
        Aggregate {
            candidate: WikidataCandidate {
                id: id.to_string(),
                label: Some(label.to_string()),
                description: Some(description.to_string()),
                classes: Vec::new(),
            },
            strategies: vec![plan::Strategy::SearchLabel],
            title_only: false,
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
        // The title answer names one page, and that page is this item: an exact label is then a
        // name, and the title vouches for it without description corroboration.
        let mut distractor = candidate(
            "Q4787186",
            "Buildings of Scotland in the Industrial Revolution",
            "Buildings of Scotland in the Industrial Revolution",
        );
        distractor.strategies = vec![plan::Strategy::SearchLabel];
        let mut titled = candidate(
            "Q2269",
            "Industrial Revolution",
            "transition to new manufacturing processes in Europe and the United States",
        );
        titled.strategies = vec![plan::Strategy::Title];
        titled.title_only = true;
        let candidate_set = vec![distractor, titled];
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

    /// A title answer whose page title is our exact name vouches for an item labelled differently:
    /// the article for "Fall of Singapore" is the item labelled "Battle of Singapore".
    #[test]
    fn a_title_answer_vouches_for_a_synonym_label() {
        let entity = entity(
            "Fall of Singapore",
            "The February 1942 military engagement in which Imperial Japanese forces captured the British stronghold of Singapore.",
            Some("Fall of Singapore 1942"),
        );
        let mut titled = candidate("Q296754", "Battle of Singapore", "1942 World War II battle");
        titled.strategies = vec![plan::Strategy::Title];
        titled.title_only = true;

        let identity = decide(&entity, &vec![titled]).expect("the title answer named it");
        assert_eq!(identity.external_id, "Q296754");
    }

    /// Search agreement is **not** corroboration: it re-admits homonyms, and two eligible
    /// same-label candidates then tie on the margin. `Alan Watts` is the measured case — the
    /// author (title answer) and a meteorologist of the same name both became eligible, and the
    /// correct one was refused with "two candidates fit and neither wins".
    #[test]
    fn a_label_match_with_only_search_agreement_stays_ineligible() {
        let entity = entity(
            "Jake Blues",
            "The charismatic lead vocalist of The Blues Brothers.",
            None,
        );
        let mut agreed = candidate("Q88538218", "Jake Blues", "fictional musician");
        agreed.strategies = vec![plan::Strategy::Label, plan::Strategy::SearchLabel];

        let error = decide(&entity, &vec![agreed]).expect_err("search is not corroboration");
        assert!(error.contains("no candidate bears our name"), "{error}");
    }

    /// One question naming an item is a string, not a fact: a lone exact label with no
    /// corroboration is still refused. This is the gate that must survive the change.
    #[test]
    fn a_lonely_exact_label_is_not_enough() {
        let entity = entity("Blue", "A hue between violet and green.", None);
        let mut lonely = candidate("Q1088", "Blue", "colour");
        lonely.strategies = vec![plan::Strategy::Label];

        let error = decide(&entity, &vec![lonely]).expect_err("nothing corroborates the label");
        assert!(error.contains("no candidate bears our name"), "{error}");
    }

    /// A title answer does not outrank an item literally labelled our name. `Swann's Way` is the
    /// volume; the article redirects to the parent work, and crediting that alias made the two
    /// tie on the margin and refused both.
    #[test]
    fn a_title_alias_does_not_outrank_the_exact_label() {
        let entity = entity("Swann's Way", "The first novel by Proust.", None);
        let mut parent = candidate("Q464928", "In Search of Lost Time", "novel by Marcel Proust");
        parent.strategies = vec![plan::Strategy::Title];
        parent.title_only = true;
        let mut volume = candidate("Q2700307", "Swann's Way", "novel by Marcel Proust");
        volume.strategies = vec![plan::Strategy::Label];

        let identity = decide(&entity, &vec![parent, volume]).expect("the labelled volume wins");
        assert_eq!(identity.external_id, "Q2700307");
    }
}
