//! The heuristic: the default policy, and everything we guess, in one file.
//!
//! This module is the *entire* ad hoc policy that turns observations into links. Nothing outside it
//! knows what a name similarity is, which weights apply, or where a bar sits — the core exposes
//! `Blocker` and `LinkDecider`, and this file implements both. Delete it and the graph has no
//! linking policy at all, which is the point: the guess is replaceable, the contracts are not.
//!
//! Three parts, in the order the executor runs them:
//!
//! - [`NameBlocker`] — normalized names and account handles. Recall-oriented, over-generates.
//! - [`score`] — pure features: name, description, type, weighted into a total.
//! - [`HeuristicLinker`] — take the best candidate that clears the bar, else create.

use std::collections::{BTreeSet, HashMap};

use sea_orm::{EntityTrait, QueryOrder};

use crate::model;

use super::blocker::Blocker;
use super::evidence::{Claim, Evidence};
use super::link_decider::{Action, Decision, LinkDecider};
use super::normalize;

pub const WEIGHT_NAME: f32 = 0.60;
pub const WEIGHT_DESCRIPTION: f32 = 0.30;
/// Deliberately small: with both types model-assigned, disagreement says more about the extractor
/// than about the referent (111 of 179 prod collisions are the same thing typed twice).
pub const WEIGHT_TYPE: f32 = 0.10;

/// The bar for a provisional → provisional merge, which is the only regime that exists until
/// resolution gives us identifiers.
pub const MERGE_BAR: f32 = 0.60;

/// A merge without an identifier needs a name match to be in the running at all.
pub const NAME_FLOOR: f32 = 0.90;

/// …and the anchor must agree about *something else* as well.
pub const AGREEMENT_FLOOR: f32 = 0.50;

/// Same name once word order and articles are folded.
const NAME_TOKEN_MATCH: f32 = 0.9;

/// Two types agreeing when one is `unknown`: an absence of information, not a disagreement.
const TYPE_UNKNOWN_MATCH: f32 = 0.5;

/// What an extra independent input is worth on top of the best claim's score.
pub const CORROBORATION_BONUS: f32 = 0.1;
pub const CONFIDENCE_CEILING: f32 = 1.0;

/// A ranked authority answer, mapped into [0.5, `ANSWER_CEILING`].
const ANSWER_BASE: f32 = 0.50;
const ANSWER_SPREAD: f32 = 0.45;
pub const ANSWER_CEILING: f32 = 0.95;
/// A guess from the disambiguating words alone is worth less than an exact label.
pub const HINT_CEILING: f32 = 0.85;
/// One exact label with nothing competing for it is identity, but nothing corroborates it either.
pub const LONE_LABEL_CONFIDENCE: f32 = 0.75;
/// A search answer accepted because exactly one candidate's label is our own surface.
pub const LOOKUP_CONFIDENCE: f32 = 0.85;
/// An authority asked about a handle: a property match names the account itself.
pub const HANDLE_CONFIDENCE: f32 = 0.95;

/// What one candidate earns against one observation, and the weighted total of it all.
#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    pub name: f32,
    pub description: f32,
    pub type_match: f32,
    pub total: f32,
}

/// The observation being linked, with its normalized forms precomputed.
#[derive(Debug, Clone)]
pub struct NormalizedObservation {
    pub name_key: String,
    pub token_key: String,
    pub description_tokens: Vec<String>,
    pub entity_type: String,
}

impl NormalizedObservation {
    pub fn new(name: &str, description: Option<&str>, entity_type: &str) -> Self {
        let name_key = normalize::comparable(name);
        let token_key = token_key_of(&tokens_of(&name_key));
        Self {
            token_key,
            name_key,
            description_tokens: description.map(tokens).unwrap_or_default(),
            entity_type: entity_type.to_string(),
        }
    }
}

impl NormalizedObservation {
    /// An entity scored as an observation: its selected name, description and type.
    pub fn of_entity(entity: &model::knowledge_entity::Model) -> Self {
        Self::new(
            &entity.name,
            entity.description.as_deref(),
            &entity.entity_type,
        )
    }
}

impl From<&model::user_observed_entity::Model> for NormalizedObservation {
    fn from(observation: &model::user_observed_entity::Model) -> Self {
        Self::new(
            &observation.name,
            observation.description.as_deref(),
            &observation.entity_type,
        )
    }
}

/// A candidate entity, with the label work precomputed so scoring is cheap in a loop.
#[derive(Debug, Clone)]
pub struct NormalizedCandidate {
    pub id: i32,
    pub alt_name_keys: Vec<String>,
    pub alt_name_token_keys: Vec<String>,
    /// One token set per accumulated description; a candidate has to fit only one of them.
    pub description_tokens: Vec<Vec<String>>,
    /// Every type this entity's members asserted. Agreement with any one of them counts.
    pub entity_types: Vec<String>,
}

impl NormalizedCandidate {
    /// `name` is the entity's selected surface and must be indexed alongside the alternatives.
    /// `project` deliberately excludes it from `alt_names`, so indexing `alt_names` alone leaves a
    /// persisted entity invisible to blocking on its own name — every later observation that names
    /// it then creates a duplicate instead of attaching.
    pub fn new(
        id: i32,
        name: &str,
        alt_names: &[String],
        descriptions: &[&str],
        entity_types: &[String],
    ) -> Self {
        let mut alt_name_keys = Vec::with_capacity(alt_names.len() + 1);
        let mut alt_name_token_keys = Vec::with_capacity(alt_names.len() + 1);
        for surface in std::iter::once(name).chain(alt_names.iter().map(String::as_str)) {
            if let Some((key, blocking)) = block_keys(surface) {
                alt_name_keys.push(key);
                alt_name_token_keys.push(blocking);
            }
        }

        Self {
            id,
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
        Self::new(
            entity.id,
            &entity.name,
            &entity.alt_names,
            &descriptions,
            &types,
        )
    }
}

/// Whether the features justify a merge. A weight alone is not enough: the observed Django failure
/// is one exact name, two referents, and a description sharing only enough tokens to lift the total
/// over the bar. Without an identifier, a name match has to be corroborated by the type or the text.
pub fn worth_merging(features: &Features) -> bool {
    features.name >= NAME_FLOOR
        && features.total >= MERGE_BAR
        && (features.type_match >= AGREEMENT_FLOOR || features.description >= AGREEMENT_FLOOR)
}

/// How substantiated an entity is: its best claim's score, plus a bonus per extra independent input
/// (`text` and `handle` are the two). `None` when it holds no claim.
pub fn entity_confidence(claims: &[(super::evidence::ClaimBasis, Option<f32>)]) -> Option<f32> {
    if claims.is_empty() {
        return None;
    }
    let bases: BTreeSet<&str> = claims.iter().map(|claim| claim.0.as_str()).collect();
    let best = claims
        .iter()
        .map(|claim| claim.1.unwrap_or(0.0))
        .fold(0.0f32, f32::max);
    let extra = bases.len().saturating_sub(1) as f32 * CORROBORATION_BONUS;
    Some((best + extra).min(CONFIDENCE_CEILING))
}

/// What a ranked authority answer is worth, before corroboration.
pub fn answer_confidence(score: f32) -> f32 {
    let mapped = ANSWER_BASE + ANSWER_SPREAD * score;
    mapped.min(ANSWER_CEILING)
}

/// Score one entity against another with the linker's own weights.
pub fn score_pair(
    observation_of: &model::knowledge_entity::Model,
    candidate: &model::knowledge_entity::Model,
) -> Features {
    score(
        &NormalizedObservation::of_entity(observation_of),
        &NormalizedCandidate::from(candidate),
    )
}

/// Whether `a` is the more substantiated side. Ties go to `b` (the caller passes the lower id first).
pub fn wins(a: &model::knowledge_entity::Model, b: &model::knowledge_entity::Model) -> bool {
    a.confidence.unwrap_or(0.0) > b.confidence.unwrap_or(0.0)
}

/// Similarity of one observation to one candidate: per feature, and weighted into a total.
pub fn score(observation: &NormalizedObservation, candidate: &NormalizedCandidate) -> Features {
    let name = name_similarity(observation, candidate);
    // Best fit across every description the candidate has accumulated, never their union: a union
    // would dilute a good match with text from captures that had nothing to do with this observation.
    let description = candidate
        .description_tokens
        .iter()
        .map(|tokens| normalize::jaccard(&observation.description_tokens, tokens))
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

/// 1.0 on the same normalized surface, 0.9 when only word order or an article differs, else 0.0.
/// A name never decides on its own — [`worth_merging`] demands corroboration.
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

/// Blocks on normalized names and the account a capture showed.
///
/// Both keys are cheap and generous: a shared word-order-insensitive name, or a `(scheme, handle)`
/// an observation carries, puts an entity in front of the linker. Nothing here knows about identity
/// or type — those narrow, they do not find.
#[derive(Debug, Default)]
pub struct NameBlocker {
    /// One lock over both maps: `blocks` naming an entity `entities` does not hold is a candidate
    /// the linker would score against nothing, so the two only ever move together. `std` rather
    /// than `tokio` because no method waits while holding it — the whole index is in memory.
    index: std::sync::Mutex<Index>,
}

#[derive(Debug, Default)]
struct Index {
    /// Block key -> entity ids. `BTreeSet` so candidates come back in a stable order.
    blocks: HashMap<String, BTreeSet<i32>>,
    /// The entities themselves. Candidates are handed back whole, because the linker scores them
    /// and an id it cannot resolve is a query the executor would otherwise have to run.
    entities: HashMap<i32, model::knowledge_entity::Model>,
}

impl NameBlocker {
    /// Index every live entity. The blocker is the corpus, so it loads its own.
    pub async fn new(db: &crate::database::DbHandle) -> anyhow::Result<Self> {
        let entities = model::knowledge_entity::Entity::find()
            .order_by_asc(model::knowledge_entity::Column::Id)
            .all(&db.conn)
            .await?
            .into_iter()
            .filter(|row| row.merged_into_id.is_none());
        let blocker = Self::default();
        for entity in entities {
            blocker.changed(&entity);
        }
        Ok(blocker)
    }

    /// Build the index from a corpus already in hand.
    #[cfg(test)]
    pub fn build_entities(entities: &[model::knowledge_entity::Model]) -> Self {
        let blocker = Self::default();
        for entity in entities {
            blocker.changed(entity);
        }
        blocker
    }

    /// The index, locked. A poisoned lock is still a usable one: the maps are updated cell by cell,
    /// so a panic mid-update cannot leave them structurally broken.
    fn index(&self) -> std::sync::MutexGuard<'_, Index> {
        self.index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn keys_of(entity: &model::knowledge_entity::Model) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for surface in entity.surfaces() {
            if let Some((key, blocking)) = block_keys(surface) {
                keys.extend([key, blocking]);
            }
        }
        if let (Some(platform), Some(handle)) = (&entity.platform, &entity.platform_handle) {
            keys.push(Claim::key(platform, handle));
        }
        keys
    }
}

#[async_trait::async_trait]
impl Blocker for NameBlocker {
    fn forget(&self, knowledge_entity_id: i32) {
        let mut index = self.index();
        index.entities.remove(&knowledge_entity_id);
        index.blocks.retain(|_, ids| {
            ids.remove(&knowledge_entity_id);
            !ids.is_empty()
        });
    }

    fn changed(&self, entity: &model::knowledge_entity::Model) {
        let mut index = self.index();
        for key in Self::keys_of(entity) {
            index.blocks.entry(key).or_default().insert(entity.id);
        }
        index.entities.insert(entity.id, entity.clone());
    }

    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> Vec<model::knowledge_entity::Model> {
        let view = NormalizedObservation::from(observation);
        let index = self.index();
        let mut found: BTreeSet<i32> = BTreeSet::new();
        for block in [&view.name_key, &view.token_key] {
            if let Some(hits) = index.blocks.get(block) {
                found.extend(hits.iter().copied());
            }
        }
        found
            .into_iter()
            .filter_map(|id| index.entities.get(&id).cloned())
            .collect()
    }
}

/// Take the highest-scoring candidate that clears the bar; otherwise start a new entity.
///
/// A name alone never decides — [`worth_merging`] requires corroboration — so this is a thin shell
/// over pure functions. It exists so that "what to do" has exactly one home.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicLinker;

#[async_trait::async_trait]
impl LinkDecider for HeuristicLinker {
    /// Give the illumination's observations their entities.
    ///
    /// A held identity settles the question before any scoring: an entity that already claims this
    /// platform link *is* the thing, and asking a name-similarity model to second-guess that is how a
    /// handle and a real name end up as two entities. Several entities may hold it, so the most
    /// substantiated one wins and the rest are left to reconcile, which is where "are these one
    /// thing?" is asked.
    ///
    /// An identity nothing holds yet does *not* settle it: the capture asserted a handle, which is
    /// strong evidence about the handle and weak evidence about the referent. So the observation
    /// falls through to similarity with its identity carried along — a name match may attach it to an
    /// entity that has the name and was missing the handle.
    ///
    /// Each observation is decided on its own, against the corpus as it stood before this
    /// illumination was linked: nothing is set aside for a sibling, and two observations may name the
    /// same entity. Whether that is right is the extraction's question, not the linker's.
    async fn decide_link(
        &self,
        blocker: &dyn Blocker,
        evidence: &Evidence,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<Vec<Decision>> {
        // An unloaded relation would read as an illumination with no observations, and linking
        // nothing would look exactly like success.
        if !illumination.observations.is_loaded() {
            anyhow::bail!(
                "the illumination's observations are not loaded; fetch it with Entity::load().with(…) "
            );
        }
        // `ModelEx` rather than `Model`: the relation is loaded, and the linker wants plain rows.
        let mut observations: Vec<model::user_observed_entity::Model> = illumination
            .observations
            .iter()
            .map(|row| row.clone().into())
            .collect();
        // The order the capture emitted them, which decides who wins a contested entity.
        observations.sort_by_key(|observation| (observation.position, observation.id));

        let mut decisions = Vec::new();

        for observation in observations {
            if observation.knowledge_entity_id.is_some() {
                continue;
            }
            // Already a row: the executor imported this illumination's identities before deciding.
            let identity = evidence.identity_of(&observation).await?;

            // `held_by_identity` returns the most substantiated first; the rest go to reconcile.
            if let Some(identity_id) = identity
                && let Some(best) = evidence.held_by_identity(identity_id).await?.into_iter().next()
            {
                decisions.push(Decision {
                    observation,
                    action: Action::Attach(best.id),
                    identity,
                });
                continue;
            }

            let view = NormalizedObservation::from(&observation);
            // (entity id, total, clears the bar) for the best candidate: highest total wins, and
            // whether it is worth merging decides between attaching and creating.
            // Highest total wins; ties keep the first candidate, i.e. the lowest entity id.
            let mut best: Option<(i32, f32, bool)> = None;

            for candidate in blocker.candidates(&observation).await {
                let features = score(&view, &NormalizedCandidate::from(&candidate));
                let entry = (candidate.id, features.total, worth_merging(&features));
                if best.is_none_or(|winner| entry.1 > winner.1) {
                    best = Some(entry);
                }
            }

            let action = match best {
                Some((entity, _, true)) => Action::Attach(entity),
                _ => Action::Create,
            };
            decisions.push(Decision {
                observation,
                action,
                identity,
            });
        }

        Ok(decisions)
    }
}

// ---------------------------------------------------------------------------------------------
// The string plumbing the scoring needs. `normalize::comparable` is the core's; these three are the
// guess's, and nothing outside this file asks for them.
// ---------------------------------------------------------------------------------------------

/// The two keys one surface answers to: its comparable form, and the order-insensitive token key.
/// `None` for a surface that folds to nothing.
fn block_keys(surface: &str) -> Option<(String, String)> {
    let key = normalize::comparable(surface);
    if key.is_empty() {
        return None;
    }
    let blocking = token_key_of(&tokens_of(&key));
    Some((key, blocking))
}

/// Dropped from the *token* key only. They are noise for blocking, and never an identity decision.
const ARTICLES: [&str; 3] = ["the", "a", "an"];

/// The words of a surface, in order, folded the same way `normalize::comparable` folds it.
fn tokens(surface: &str) -> Vec<String> {
    tokens_of(&normalize::comparable(surface))
}

/// The words of a surface already in its comparable form, so a caller that has one does not
/// normalize it twice.
fn tokens_of(comparable: &str) -> Vec<String> {
    comparable.split_whitespace().map(str::to_string).collect()
}

/// A word-order- and article-insensitive key, for blocking only. Never an identity key.
fn token_key_of(words: &[String]) -> String {
    let mut words: Vec<&str> = words.iter().map(String::as_str).collect();
    if words.len() > 1 && ARTICLES.contains(&words[0]) {
        words.remove(0);
    }
    words.sort_unstable();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(name: &str, description: Option<&str>, ty: &str) -> NormalizedObservation {
        NormalizedObservation::new(name, description, ty)
    }

    fn entity(name: &str, description: Option<&str>, ty: &str) -> NormalizedCandidate {
        let descriptions: Vec<&str> = description.into_iter().collect();
        NormalizedCandidate::new(1, name, &[], &descriptions, &[ty.to_string()])
    }

    /// `brand` and `organization` are the same referent described twice — the taxonomy does not have
    /// to choose for a member to agree.
    #[test]
    fn any_asserted_type_counts_as_agreement() {
        let entity = NormalizedCandidate::new(
            1,
            "Nintendo",
            &[],
            &[],
            &["organization".to_string(), "brand".to_string()],
        );
        let typed = |ty: &str| score(&observation("Nintendo", None, ty), &entity).type_match;
        assert_eq!(typed("brand"), 1.0);
        assert_eq!(typed("organization"), 1.0);
        assert_eq!(typed("movie"), 0.0);
    }

    #[test]
    fn identical_name_and_type_clears_the_bar() {
        let features = score(
            &observation("Train Dreams", Some("a novella by Denis Johnson"), "book"),
            &entity("Train Dreams", Some("a novella by Denis Johnson"), "book"),
        );
        assert_eq!(features.name, 1.0);
        assert!(worth_merging(&features), "{features:?}");
    }

    #[test]
    fn article_or_word_order_alone_still_clears() {
        let features = score(
            &observation("The Train Dreams", None, "book"),
            &entity("Train Dreams", None, "book"),
        );
        assert_eq!(features.name, 0.9);
        assert!(worth_merging(&features), "{features:?}");
    }

    #[test]
    fn a_type_mismatch_without_name_agreement_does_not_clear() {
        let features = score(
            &observation("Django", Some("a Python web framework"), "software"),
            &entity("Django", Some("a 2021 video game"), "product"),
        );
        assert_eq!(features.name, 1.0);
        assert_eq!(features.type_match, 0.0);
        // One exact name and a description that shares only stopword-ish tokens: this is the case
        // that made the resolver claim the video game, so it must not merge on weights alone.
        assert!(!worth_merging(&features), "{features:?}");
    }

    #[test]
    fn different_names_do_not_clear() {
        let features = score(
            &observation("Train Dreams", None, "book"),
            &entity("Gravity's Rainbow", None, "book"),
        );
        assert_eq!(features.name, 0.0);
        assert!(!worth_merging(&features), "{features:?}");
    }

    #[test]
    fn unknown_type_is_weak_not_negative() {
        let features = score(
            &observation("Wojak", None, "unknown"),
            &entity("Wojak", None, "concept"),
        );
        assert_eq!(features.type_match, 0.5);
    }

    fn entity_row(id: i32, name: &str, ty: &str) -> model::knowledge_entity::Model {
        crate::test_support::fixtures::stub_entity(id, name, ty)
    }

    fn observation_row(name: &str, ty: &str) -> model::user_observed_entity::Model {
        crate::test_support::fixtures::stub_observation(1, name, ty)
    }

    /// Link one illumination: build the blocker from the corpus and run the linker over it.
    ///
    /// Needs a database now, because the linker asks the evidence store whether a claim is already
    /// held. Observations with no platform link never reach that call.
    async fn link(
        db: &crate::database::DbHandle,
        entities: &[model::knowledge_entity::Model],
        observations: &[model::user_observed_entity::Model],
    ) -> Vec<Decision> {
        let blocker = NameBlocker::build_entities(entities);
        let evidence = Evidence::from_handle(db);
        evidence
            .import_identities(observations)
            .await
            .expect("importing the assertions should not fail");
        let mut illumination = crate::test_support::fixtures::stub_illumination(1);
        illumination.observations = sea_orm::compound::HasMany::Loaded(
            observations.iter().map(|row| row.clone().into()).collect(),
        );
        HeuristicLinker
            .decide_link(&blocker, &evidence, &illumination)
            .await
            .expect("the linker should not fail")
    }

    fn ids(candidates: &[model::knowledge_entity::Model]) -> Vec<i32> {
        candidates.iter().map(|entity| entity.id).collect()
    }

    /// The blocking key, spelled the way a caller spells it: fold, split, sort.
    fn blocking(surface: &str) -> String {
        token_key_of(&tokens_of(&normalize::comparable(surface)))
    }

    #[test]
    fn token_key_ignores_order_and_articles() {
        assert_eq!(blocking("The Train Dreams"), blocking("train dreams"));
        assert_eq!(blocking("Dreams Train"), blocking("Train Dreams"));
        assert_ne!(blocking("C"), blocking("C#"));
    }

    /// The counterpart to `changed`: a held blocker is only ever added to, so an entity that is
    /// discarded or tombstoned has to be told, or it keeps being offered as a candidate.
    #[tokio::test]
    async fn a_forgotten_entity_is_no_longer_proposed() {
        let blocker = NameBlocker::build_entities(&[
            entity_row(7, "Train Dreams", "book"),
            entity_row(8, "Dune", "book"),
        ]);

        assert_eq!(
            ids(&blocker
                .candidates(&observation_row("Train Dreams", "book"))
                .await),
            vec![7]
        );

        blocker.forget(7);

        assert!(
            blocker
                .candidates(&observation_row("Train Dreams", "book"))
                .await
                .is_empty(),
            "the name must not resolve to a deleted entity"
        );
        assert_eq!(
            ids(&blocker.candidates(&observation_row("Dune", "book")).await),
            vec![8],
            "forgetting one entity leaves the others alone"
        );
    }

    /// The bug that made every later capture duplicate an existing name: `project` keeps the
    /// selected surface out of `alt_names`, so an entity with no alternatives must still be
    /// findable by its own name.
    #[tokio::test]
    async fn the_selected_name_is_a_block_key_on_its_own() {
        let blocker =
            NameBlocker::build_entities(&[entity_row(7, "Francis Ford Coppola", "real_person")]);

        let candidates = blocker
            .candidates(&observation_row("Francis Ford Coppola", "real_person"))
            .await;
        assert_eq!(ids(&candidates), vec![7]);
    }

    #[tokio::test]
    async fn word_order_alone_still_blocks() {
        let blocker = NameBlocker::build_entities(&[entity_row(7, "Train Dreams", "book")]);

        let candidates = blocker
            .candidates(&observation_row("Dreams Train", "book"))
            .await;
        assert_eq!(
            ids(&candidates),
            vec![7],
            "token-set equality is a block key"
        );
    }

    #[tokio::test]
    async fn distinct_names_propose_nothing() {
        let blocker = NameBlocker::build_entities(&[entity_row(7, "Train Dreams", "book")]);

        assert!(
            blocker
                .candidates(&observation_row("Apocalypse Now", "movie"))
                .await
                .is_empty()
        );
    }

    /// The mid-run path: an entity built after the corpus is findable by it.
    #[tokio::test]
    async fn an_entity_added_mid_run_is_findable() {
        let blocker = NameBlocker::build_entities(&[entity_row(7, "Train Dreams", "book")]);
        blocker.changed(&entity_row(9, "Apocalypse Now", "movie"));

        let candidates = blocker
            .candidates(&observation_row("Apocalypse Now", "movie"))
            .await;
        assert_eq!(ids(&candidates), vec![9]);
    }

    #[tokio::test]
    async fn the_linker_takes_the_best_candidate_that_clears_the_bar() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let weak = entity_row(1, "Train Dreams", "book");
        let strong = entity_row(2, "Train Dreams", "book");
        let observation = observation_row("Train Dreams", "book");

        let result = link(&db, &[weak, strong], &[observation]).await;

        match result.as_slice() {
            [decision] => match &decision.action {
                Action::Attach(entity) => assert!(*entity == 1 || *entity == 2),
                other => panic!("expected an attach, got {other:?}"),
            },
            other => panic!("expected one decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_candidates_creates_with_nothing_to_report() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let result = link(&db, &[], &[observation_row("Anything", "unknown")]).await;
        match result.as_slice() {
            [decision] => match &decision.action {
                Action::Create => {}
                other => panic!("expected a create, got {other:?}"),
            },
            other => panic!("expected one decision, got {other:?}"),
        }
    }
}
