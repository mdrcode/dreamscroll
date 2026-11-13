//! The heuristic: every number the graph guesses, in one file.
//!
//! Two verbs, implemented for the pairs the graph asks about:
//!
//! - [`Similarity`] — the weighted total of `name`, `description` and type agreement.
//! - [`Policy`] — the same features for a candidate set, the corroboration conjunction, and the
//!   total as the order key. `Rank` (qualifying candidates, best first) follows from it.
//!
//! The filter is the part a number could not carry: `same` is a conjunction — a name match *and*
//! something corroborating it — which is why [`Rank`] exists rather than a threshold on the score.
//! A pair that fails the conjunction still reports its total, so "same name, different type: 0.74"
//! stays visible on the proximity list even though it will never be folded.
//!
//! Not here: the confidence scale for resolver answers (that is the identity resolver's own calibration,
//! in `identity/wikidata/ranking.rs`), the best identity over a set (`best_confidence`), and the order
//! of two confidences (`scoring::TotalCmp`).

use crate::model;

use crate::knowledge::scoring::{Entity, Observation, Order, Policy, Scoring, Similarity};
use crate::knowledge::text;

pub const WEIGHT_NAME: f32 = 0.60;
pub const WEIGHT_DESCRIPTION: f32 = 0.30;
/// Deliberately small: with both types model-assigned, disagreement says more about the extractor
/// than about the referent (111 of 179 prod collisions are the same thing typed twice).
pub const WEIGHT_TYPE: f32 = 0.10;

/// A merge without an identifier needs a name match to be in the running at all.
const NAME_FLOOR: f32 = 0.90;

/// …and the anchor must agree about *something else* as well.
const AGREEMENT_FLOOR: f32 = 0.50;

/// Same name once word order and articles are folded.
const NAME_TOKEN_MATCH: f32 = 0.9;

/// Two types agreeing when one is `unknown`: an absence of information, not a disagreement.
const TYPE_UNKNOWN_MATCH: f32 = 0.5;

/// The heuristic policy, as one value. Stateless: every number is a constant above.
#[derive(Debug, Default, Clone, Copy)]
pub struct Heuristic;

/// The one instance: a unit struct has nothing to own, so borrowing it costs nothing.
static HEURISTIC: Heuristic = Heuristic;

impl Heuristic {
    /// This policy as the graph's three questions. Stateless, so one instance serves every caller.
    pub fn scoring() -> &'static dyn Scoring {
        &HEURISTIC
    }
}

impl Similarity<Observation, Entity> for Heuristic {
    fn score(&self, observation: &Observation, candidate: &Entity) -> f32 {
        features(observation, candidate).total
    }
}

impl Similarity<Entity, Entity> for Heuristic {
    fn score(&self, a: &Entity, b: &Entity) -> f32 {
        features_of_entities(a, b).total
    }
}

impl Policy<Observation, Entity> for Heuristic {
    type Evidence = Features;

    fn evaluate(&self, observation: &Observation, candidates: &[Entity]) -> Vec<Features> {
        candidates
            .iter()
            .map(|candidate| features(observation, candidate))
            .collect()
    }

    fn qualifies(&self, features: &Features) -> bool {
        features.qualifies()
    }

    fn order(&self, features: &Features) -> Order {
        features.order()
    }
}

impl Policy<Entity, Entity> for Heuristic {
    type Evidence = Features;

    fn evaluate(&self, a: &Entity, candidates: &[Entity]) -> Vec<Features> {
        candidates
            .iter()
            .map(|candidate| features_of_entities(a, candidate))
            .collect()
    }

    fn qualifies(&self, features: &Features) -> bool {
        features.qualifies()
    }

    fn order(&self, features: &Features) -> Order {
        features.order()
    }
}

/// What one candidate earns against one subject, per feature and in total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Features {
    pub name: f32,
    pub description: f32,
    pub type_match: f32,
    pub total: f32,
}

impl Features {
    /// Whether these features justify calling two things one.
    ///
    /// A weight alone is not enough: the observed Django failure is one exact name, two referents, and
    /// a description sharing only enough tokens to lift the total over the bar. Without an identifier, a
    /// name match has to be corroborated by the type or the text — and that conjunction is why this is
    /// a filter on a vector of components rather than a threshold on its sum.
    ///
    /// The total is deliberately not consulted. Under these weights every pair this accepts scores at
    /// least `0.6 * NAME_TOKEN_MATCH + 0.1 * TYPE_UNKNOWN_MATCH`, so a threshold on the sum can only
    /// contradict the conjunction — which is exactly what `MERGE_BAR = 0.60` did at that one point.
    pub fn qualifies(&self) -> bool {
        self.name >= NAME_FLOOR
            && (self.type_match >= AGREEMENT_FLOOR || self.description >= AGREEMENT_FLOOR)
    }

    /// Where these features sort: the total is the whole judgement, so there is no tie to break.
    pub fn order(&self) -> Order {
        Order::of(self.total)
    }
}

/// How substantiated an entity is: its best identity's score.
///
/// An identity without a confidence cannot substantiate anything; the best known confidence wins.
pub(in crate::knowledge) fn best_confidence(identities: &[Option<f32>]) -> Option<f32> {
    identities
        .iter()
        .filter_map(|identity| *identity)
        .max_by(f32::total_cmp)
}

/// The observation being compared, with its folded forms precomputed.
#[derive(Debug, Clone)]
struct NormalizedObservation {
    name_key: String,
    token_key: String,
    description_tokens: Vec<String>,
    entity_type: String,
}

impl NormalizedObservation {
    fn new(name: &str, description: Option<&str>, entity_type: &str) -> Self {
        let name_key = text::comparable(name);
        let token_key = text::word_key(&name_key);
        Self {
            token_key,
            name_key,
            description_tokens: description.map(tokens).unwrap_or_default(),
            entity_type: entity_type.to_string(),
        }
    }
}

/// A candidate entity, with the label work precomputed so scoring is cheap in a loop.
#[derive(Debug, Clone)]
struct NormalizedCandidate {
    alt_name_keys: Vec<String>,
    alt_name_token_keys: Vec<String>,
    /// One token set per accumulated description; a candidate has to fit only one of them.
    description_tokens: Vec<Vec<String>>,
    /// Every type this entity's members asserted. Agreement with any one of them counts.
    entity_types: Vec<String>,
}

impl NormalizedCandidate {
    /// `name` is the entity's selected surface and must be indexed alongside the alternatives.
    /// `project` deliberately excludes it from `alt_names`, so indexing `alt_names` alone leaves a
    /// persisted entity invisible to blocking on its own name — every later observation that names
    /// it then creates a duplicate instead of attaching.
    fn new(
        name: &str,
        alt_names: &[String],
        descriptions: &[&str],
        entity_types: &[String],
    ) -> Self {
        let mut alt_name_keys = Vec::with_capacity(alt_names.len() + 1);
        let mut alt_name_token_keys = Vec::with_capacity(alt_names.len() + 1);
        for surface in std::iter::once(name).chain(alt_names.iter().map(String::as_str)) {
            let key = text::comparable(surface);
            if key.is_empty() {
                continue;
            }
            // A surface with no words (`...` folds to `..`) has no word generation, and storing the
            // empty string would make it match every other wordless surface. Same rule as
            // `blocker::index::fold`, which is the one definition of the two generations.
            let token_key = text::word_key(&key);
            if !token_key.is_empty() {
                alt_name_token_keys.push(token_key);
            }
            alt_name_keys.push(key);
        }

        Self {
            alt_name_keys,
            alt_name_token_keys,
            description_tokens: descriptions
                .iter()
                .map(|description| tokens(description))
                .filter(|tokens| !tokens.is_empty())
                .collect(),
            entity_types: entity_types.to_vec(),
        }
    }
}

impl From<&model::knowledge_entity::Model> for NormalizedCandidate {
    fn from(entity: &model::knowledge_entity::Model) -> Self {
        let descriptions: Vec<&str> = entity
            .description
            .iter()
            .map(String::as_str)
            .chain(entity.alt_descriptions.iter().map(String::as_str))
            .collect();
        let types: Vec<String> = entity.types().map(str::to_string).collect();
        Self::new(&entity.name, &entity.alt_names, &descriptions, &types)
    }
}

/// Similarity of one observation to one candidate: per feature, and weighted into a total.
fn features(observation: &Observation, candidate: &Entity) -> Features {
    let view = NormalizedObservation::new(
        &observation.name,
        observation.description.as_deref(),
        &observation.entity_type,
    );
    score(&view, &NormalizedCandidate::from(candidate))
}

/// Two entities, each read as everything its members contributed.
///
/// Symmetric by construction: a pair has one score, whichever side asks. Reading the subject's
/// selected values only made its `alt_*` invisible, so the number depended on direction — and both
/// passes choose that direction from the projected confidence, so a confidence change (a later
/// resolve) could move the number, and with it the verdict, with no evidence changing.
fn features_of_entities(a: &Entity, b: &Entity) -> Features {
    let left = NormalizedCandidate::from(a);
    let right = NormalizedCandidate::from(b);

    let name = if left
        .alt_name_keys
        .iter()
        .any(|key| right.alt_name_keys.contains(key))
    {
        1.0
    } else if left
        .alt_name_token_keys
        .iter()
        .any(|key| right.alt_name_token_keys.contains(key))
    {
        NAME_TOKEN_MATCH
    } else {
        0.0
    };

    // Best fit across every description either side has accumulated, never their union: a union
    // would dilute a good match with text from captures that had nothing to do with this pair.
    let description = left
        .description_tokens
        .iter()
        .flat_map(|theirs| {
            right
                .description_tokens
                .iter()
                .map(move |ours| text::jaccard(ours, theirs))
        })
        .fold(0.0f32, f32::max);

    // Any agreement counts, in either direction.
    let type_match = left
        .entity_types
        .iter()
        .flat_map(|theirs| {
            right
                .entity_types
                .iter()
                .map(move |ours| type_similarity(ours, theirs))
        })
        .fold(0.0f32, f32::max);

    let total = WEIGHT_NAME * name + WEIGHT_DESCRIPTION * description + WEIGHT_TYPE * type_match;
    Features {
        name,
        description,
        type_match,
        total,
    }
}

/// Similarity of one folded observation to one candidate: per feature, weighted into a total.
fn score(observation: &NormalizedObservation, candidate: &NormalizedCandidate) -> Features {
    let name = name_similarity(observation, candidate);
    // Best fit across every description the candidate has accumulated, never their union: a union
    // would dilute a good match with text from captures that had nothing to do with this observation.
    let description = candidate
        .description_tokens
        .iter()
        .map(|tokens| text::jaccard(&observation.description_tokens, tokens))
        .fold(0.0f32, f32::max);
    // Any agreement counts. An entity whose members were called both `brand` and `organization` must
    // accept either, or half its own members could not have joined it.
    let type_match = candidate
        .entity_types
        .iter()
        .map(|entity_type| type_similarity(&observation.entity_type, entity_type))
        .fold(0.0f32, f32::max);
    let total = WEIGHT_NAME * name + WEIGHT_DESCRIPTION * description + WEIGHT_TYPE * type_match;
    Features {
        name,
        description,
        type_match,
        total,
    }
}

/// 1.0 on the same folded surface, 0.9 when only word order or an article differs, else 0.0.
/// A name never decides on its own — [`Features::qualifies`] demands corroboration.
fn name_similarity(observation: &NormalizedObservation, candidate: &NormalizedCandidate) -> f32 {
    if candidate.alt_name_keys.contains(&observation.name_key) {
        return 1.0;
    }
    if candidate
        .alt_name_token_keys
        .contains(&observation.token_key)
    {
        return NAME_TOKEN_MATCH;
    }
    0.0
}

/// 1.0 for the same type, `TYPE_UNKNOWN_MATCH` when either is unknown, else 0.
fn type_similarity(a: &str, b: &str) -> f32 {
    match (a, b) {
        (x, y) if x == y => 1.0,
        // An unknown type is an absence of information, not a disagreement.
        ("unknown", _) | (_, "unknown") => TYPE_UNKNOWN_MATCH,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------------------------
// The string plumbing the scoring and the blocker share. `text::comparable` is the core's;
// these are the guess's, and nothing outside `heuristic_scoring` asks for them.
// ---------------------------------------------------------------------------------------------

/// The words of a surface, in order, folded the same way `text::comparable` folds it.
fn tokens(surface: &str) -> Vec<String> {
    tokens_of(&text::comparable(surface))
}

/// The words of a surface already in its comparable form, so a caller that has one does not
/// fold it twice. Stopwords are dropped with `text::is_noise`, the resolver's own definition: `the`
/// and `of` never name a thing, and `The Big Short` must block against `Big Short`.
///
/// Deliberately *not* the full `content_tokens` yet: stemming and the short-run floor would move
/// link decisions much further, so they are a separate step.
fn tokens_of(comparable: &str) -> Vec<String> {
    comparable
        .split_whitespace()
        .filter(|token| !text::is_noise(token))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::knowledge::scoring::{Rank, Similarity};
    use crate::test_support::fixtures;

    fn observation(name: &str, description: Option<&str>, ty: &str) -> Observation {
        let mut row = fixtures::stub_observation(1, name, ty);
        row.description = description.map(str::to_string);
        row
    }

    fn entity(
        id: i32,
        name: &str,
        description: Option<&str>,
        ty: &str,
    ) -> model::knowledge_entity::Model {
        let mut row = fixtures::stub_entity(id, name, ty);
        row.description = description.map(str::to_string);
        row
    }

    /// One pair, two directions: the film's only evidence is in its alternatives, the novella's is
    /// its selected description.
    fn train_dreams_pair() -> (
        model::knowledge_entity::Model,
        model::knowledge_entity::Model,
    ) {
        let mut movie = entity(1, "Train Dreams", None, "movie");
        movie.alt_descriptions = vec!["a 2011 novella by Denis Johnson".to_string()];
        let book = entity(
            2,
            "Train Dreams",
            Some("a 2011 novella by Denis Johnson"),
            "book",
        );
        (movie, book)
    }

    /// `brand` and `organization` are the same referent described twice — the taxonomy does not have
    /// to choose for a member to agree.
    #[test]
    fn any_asserted_type_counts_as_agreement() {
        let mut candidate = fixtures::stub_entity(1, "Nintendo", "organization");
        candidate.alt_types = vec!["brand".to_string()];
        let typed = |ty: &str| features(&observation("Nintendo", None, ty), &candidate).type_match;
        assert_eq!(typed("brand"), 1.0);
        assert_eq!(typed("organization"), 1.0);
        assert_eq!(typed("movie"), 0.0);
    }

    /// Two entities each carry what their members said, so a pair has one measurement whichever
    /// side asks. Reading the subject's selected values only made its `alt_*` invisible, which is
    /// how the same pair scored differently in each direction.
    #[test]
    fn the_pair_score_is_symmetric() {
        let (a, b) = train_dreams_pair();

        let features = features_of_entities(&a, &b);
        assert_eq!(
            features_of_entities(&b, &a),
            features,
            "one pair, one number"
        );
        assert_eq!(features.name, 1.0);
        assert_eq!(features.description, 1.0);
    }

    /// The evidence lives in the subject's alternatives. Under the selected-only view that
    /// direction refused the pair outright while the reverse accepted it, so the outcome followed
    /// the confidence ordering rather than the evidence. Both directions must agree now.
    #[test]
    fn a_subject_alternative_corroborates_the_pair() {
        let (a, b) = train_dreams_pair();

        for (subject, candidate) in [(&a, &b), (&b, &a)] {
            let ranked = Heuristic.rank(subject, std::slice::from_ref(candidate));
            assert_eq!(
                ranked.len(),
                1,
                "the alternative corroborates from either side"
            );
        }
    }

    #[test]
    fn article_or_word_order_alone_still_clears() {
        let candidate = entity(1, "Train Dreams", None, "book");
        let ranked = Heuristic.rank(&observation("The Train Dreams", None, "book"), &[candidate]);
        assert_eq!(
            ranked.len(),
            1,
            "an article is not a different name: {ranked:?}"
        );
        // A word-key name match (0.9) plus an agreeing type (1.0).
        assert!((ranked[0].1 - 0.64).abs() < 1e-6, "{ranked:?}");
    }

    /// An `unknown` type is corroboration: the two floors accept this pair, so the gate must too.
    /// The weighted total is 0.59 — a threshold on it refused this, which is the bug this pins.
    #[test]
    fn a_token_name_match_corroborated_by_an_unknown_type_clears() {
        let candidate = entity(1, "Dr. Dominic Ng", None, "real_person");
        let probe = observation("Dominic Ng", None, "unknown");
        let features = features(&probe, &candidate);
        assert!((features.total - 0.59).abs() < 1e-6, "{features:?}");
        assert_eq!(
            Heuristic.rank(&probe, &[candidate]).len(),
            1,
            "the conjunction accepts what its own floors accept"
        );
    }

    /// The control: a name match and nothing else is still refused.
    #[test]
    fn a_name_match_without_corroboration_still_does_not_clear() {
        let candidate = entity(1, "Dominic Ng", None, "book");
        let probe = observation("Dominic Ng", None, "software");
        assert!(
            Heuristic.rank(&probe, &[candidate]).is_empty(),
            "same name, disagreeing type, no text"
        );
    }

    /// The pair the graph must never fold, and the number that says why: one exact name, no
    /// corroboration. A single threshold in this band would fold it — which is exactly why the
    /// filter is a conjunction and not a threshold on the total.
    #[test]
    fn a_type_mismatch_without_name_agreement_does_not_clear() {
        let django = entity(1, "Django", Some("a 2021 video game"), "product");
        let frame = observation("Django", Some("a Python web framework"), "software");
        assert!(
            Heuristic.score(&frame, &django) > 0.6,
            "the total alone would fold this pair"
        );
        assert!(Heuristic.rank(&frame, &[django]).is_empty());
    }

    /// The other arm of the corroboration: a shared description carries a name match on its own,
    /// even when the types disagree. No other test reaches the conjunction from this side — every
    /// one that does has a matching type too.
    #[test]
    fn a_matching_description_corroborates_a_disagreeing_type() {
        let candidate = entity(1, "Django", Some("a 2021 video game"), "product");
        let probe = observation("Django", Some("a 2021 video game"), "software");
        let features = features(&probe, &candidate);
        assert_eq!(features.type_match, 0.0, "the types disagree");
        assert!(
            features.description >= AGREEMENT_FLOOR,
            "the text agrees: {features:?}"
        );
        assert_eq!(Heuristic.rank(&probe, &[candidate]).len(), 1);
    }

    /// The extractor could not type either side: that is an absence of information, so an exact
    /// name still clears, and the unknown type still satisfies the "agree about something" anchor.
    #[test]
    fn an_unknown_type_does_not_disqualify_a_name() {
        let candidate = entity(1, "Wojak", None, "concept");
        let ranked = Heuristic.rank(&observation("Wojak", None, "unknown"), &[candidate]);
        assert_eq!(
            ranked.len(),
            1,
            "an unknown type is not a disagreement: {ranked:?}"
        );
    }

    /// The filter is a filter, not a reordering: the strongest candidate wins, and the ones that do
    /// not qualify are absent rather than last.
    #[test]
    fn rank_keeps_only_qualifying_candidates_best_first() {
        let weak = entity(1, "Train Dreams", None, "book");
        let strong = entity(
            2,
            "Train Dreams",
            Some("a novella by Denis Johnson"),
            "book",
        );
        let unrelated = entity(3, "Dune", None, "book");
        let ranked = Heuristic.rank(
            &observation("Train Dreams", Some("a novella by Denis Johnson"), "book"),
            &[weak, strong, unrelated],
        );
        assert_eq!(ranked.len(), 2, "{ranked:?}");
        assert_eq!(ranked[0].0, 1, "the corroborated one leads: {ranked:?}");
        assert!(ranked.iter().all(|(index, _)| *index != 2));
    }

    /// `None` is the least worth there is, and a tie is not a win for either side.
    #[test]
    fn cmp_confidence_puts_nothing_last() {
        use crate::knowledge::scoring::TotalCmp;
        let none: Option<f32> = None;
        assert_eq!(none.total_cmp(&Some(0.5)), std::cmp::Ordering::Less);
        assert_eq!(Some(0.5f32).total_cmp(&none), std::cmp::Ordering::Greater);
        assert_eq!(
            Some(0.5f32).total_cmp(&Some(0.5)),
            std::cmp::Ordering::Equal
        );
    }

    /// Confidence is the best claim, not a sum or a bonus.
    #[test]
    fn confidence_is_the_best_claim() {
        assert_eq!(best_confidence(&[]), None);
        assert_eq!(best_confidence(&[None]), None);
        assert_eq!(best_confidence(&[Some(0.8)]), Some(0.8));
        assert_eq!(best_confidence(&[Some(0.8), Some(0.9)]), Some(0.9));
        assert_eq!(best_confidence(&[Some(0.8), None, Some(0.5)]), Some(0.8));
    }

    /// A handle is settled *before* measuring; folding it into the name is how a handle and a real
    /// name become two entities.
    #[test]
    fn a_handle_does_not_move_the_score() {
        let plain = fixtures::stub_observation(1, "Alex Lei", "real_person");
        let mut present = plain.clone();
        present.platform = Some("x_twitter".to_string());
        present.platform_handle = Some("@alexL_E_I".to_string());
        present.platform_display_name = Some("alex lei".to_string());
        present.search_query = Some("Alex Lei photographer".to_string());

        let candidate = fixtures::stub_entity(9, "Alex Lei", "real_person");
        assert_eq!(
            Heuristic.score(&plain, &candidate),
            Heuristic.score(&present, &candidate),
        );
    }
}
