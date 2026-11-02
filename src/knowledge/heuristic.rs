//! The heuristic: everything we guess, in one file.
//!
//! This module is the *entire* ad hoc policy that turns observations into links. Nothing outside it
//! knows what a name similarity is, which weights apply, or where a bar sits — the core exposes
//! `Blocker`, `Linker` and the views, and this file is one implementation of both traits. Delete it
//! and the graph has no linking policy at all, which is the point: the guess is replaceable, the
//! contracts are not.
//!
//! Three parts, in the order the executor runs them:
//!
//! - [`NameBlocker`] — normalized names and account handles. Recall-oriented, over-generates.
//! - [`score`] — pure features: name, description, type, weighted into a total.
//! - [`HeuristicLinker`] — take the best candidate that clears the bar, else create.

use std::collections::{BTreeSet, HashMap};

use super::blocking::Blocker;
use super::linker::{Decision, Linker};
use super::normalize;
use crate::model;


pub const W_NAME: f32 = 0.60;
pub const W_DESCRIPTION: f32 = 0.30;
/// Deliberately small: with both types model-assigned, disagreement says more about the extractor
/// than about the referent (111 of 179 prod collisions are the same thing typed twice).
pub const W_TYPE: f32 = 0.10;

/// The bar for a provisional → provisional merge, which is the only regime that exists until
/// resolution gives us identifiers.
pub const MERGE_BAR: f32 = 0.60;

/// A merge without an identifier needs a name match to be in the running at all.
pub const NAME_FLOOR: f32 = 0.90;

/// …and the anchor must agree about *something else* as well.
pub const AGREEMENT_FLOOR: f32 = 0.50;

#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    pub name: f32,
    pub description: f32,
    pub type_match: f32,
    pub total: f32,
}

/// The observation being linked, with its normalized forms precomputed.
#[derive(Debug, Clone)]
pub struct Observation {
    pub name_key: String,
    pub name_tokens: Vec<String>,
    pub token_key: String,
    pub description_tokens: Vec<String>,
    pub entity_type: String,
}

impl From<&model::user_observed_entity::Model> for Observation {
    fn from(observation: &model::user_observed_entity::Model) -> Self {
        Self::new(
            &observation.name,
            observation.description.as_deref(),
            &observation.entity_type,
        )
    }
}

impl From<&model::knowledge_entity::Model> for Scored {
    fn from(entity: &model::knowledge_entity::Model) -> Self {
        let descriptions: Vec<&str> = entity
            .description
            .iter()
            .map(String::as_str)
            .chain(entity.alt_descriptions.iter().map(String::as_str))
            .collect();
        let types: Vec<String> = entity.types().map(str::to_string).collect();
        Self::new(entity.id, &entity.name, &entity.alt_names, &descriptions, &types)
    }
}

impl Observation {
    pub fn new(name: &str, description: Option<&str>, entity_type: &str) -> Self {
        Self {
            name_key: normalize::key(name),
            name_tokens: normalize::tokens(name),
            token_key: normalize::token_key(name),
            description_tokens: description.map(normalize::tokens).unwrap_or_default(),
            entity_type: entity_type.to_string(),
        }
    }
}

/// A candidate entity, with the label work precomputed so scoring is cheap in a loop.
#[derive(Debug, Clone)]
pub struct Scored {
    pub id: i32,
    pub alt_name_keys: Vec<String>,
    pub alt_name_token_keys: Vec<String>,
    /// One token set per accumulated description; a candidate has to fit only one of them.
    pub description_tokens: Vec<Vec<String>>,
    /// Every type this entity's members asserted. Agreement with any one of them counts.
    pub entity_types: Vec<String>,
}

impl Scored {
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
        let mut names: Vec<String> = Vec::with_capacity(alt_names.len() + 1);
        names.push(name.to_string());
        names.extend_from_slice(alt_names);
        Self {
            id,
            alt_name_keys: names
                .iter()
                .map(|a| normalize::key(a))
                .filter(|key| !key.is_empty())
                .collect(),
            alt_name_token_keys: names
                .iter()
                .map(|a| normalize::token_key(a))
                .filter(|key| !key.is_empty())
                .collect(),
            description_tokens: descriptions
                .iter()
                .map(|description| normalize::tokens(description))
                .filter(|tokens| !tokens.is_empty())
                .collect(),
            entity_types: entity_types.to_vec(),
        }
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

pub fn score(observation: &Observation, entity: &Scored) -> Features {
    let name = name_similarity(observation, entity);
    // Best fit across every description the entity has accumulated, never their union: a union would
    // dilute a good match with text from captures that had nothing to do with this observation.
    let description = entity
        .description_tokens
        .iter()
        .map(|tokens| jaccard(&observation.description_tokens, tokens))
        .fold(0.0f32, f32::max);
    // Any agreement counts. A entity whose members were called both `brand` and `organization` must
    // accept either, or half its own members could not have joined it.
    let type_match = entity
        .entity_types
        .iter()
        .map(|entity_type| type_similarity(&observation.entity_type, entity_type))
        .fold(0.0f32, f32::max);
    let total = W_NAME * name + W_DESCRIPTION * description + W_TYPE * type_match;
    Features {
        name,
        description,
        type_match,
        total,
    }
}

/// 1.0 on the same normalized surface, 0.9 when only word order or an article differs, else the
/// best token overlap. A name never decides on its own — see the vetoes in `link`.
fn name_similarity(observation: &Observation, entity: &Scored) -> f32 {
    if entity.alt_name_keys.contains(&observation.name_key) {
        return 1.0;
    }
    if entity.alt_name_token_keys.contains(&observation.token_key) {
        return 0.9;
    }
    0.0
}

fn type_similarity(a: &str, b: &str) -> f32 {
    match (a, b) {
        (x, y) if x == y => 1.0,
        // An unknown type is an absence of information, not a disagreement.
        ("unknown", _) | (_, "unknown") => 0.5,
        _ => 0.0,
    }
}

pub fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a: std::collections::BTreeSet<&String> = a.iter().collect();
    let b: std::collections::BTreeSet<&String> = b.iter().collect();
    let intersection = a.intersection(&b).count() as f32;
    let union = a.union(&b).count() as f32;
    intersection / union
}

/// Blocks on normalized names and the account a capture showed.
///
/// Both keys are cheap and generous: a shared word-order-insensitive name, or a `(scheme, handle)`
/// an observation carries, puts an entity in front of the linker. Nothing here knows about identity
/// or type — those narrow, they do not find.
#[derive(Debug, Default)]
pub struct NameBlocker {
    /// Block key -> entity ids. `BTreeSet` so candidates come back in a stable order.
    blocks: HashMap<String, BTreeSet<i32>>,
}

impl NameBlocker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the index from the corpus. Called where the corpus is known, before the run.
    pub fn build_entities(entities: &[model::knowledge_entity::Model]) -> Self {
        let mut blocker = Self::default();
        for entity in entities {
            blocker.changed(entity);
        }
        blocker
    }

    fn keys_of(entity: &model::knowledge_entity::Model) -> Vec<String> {
        let mut keys: Vec<String> = entity
            .surfaces()
            .flat_map(|surface| [normalize::key(surface), normalize::token_key(surface)])
            .filter(|key| !key.is_empty())
            .collect();
        if let (Some(platform), Some(handle)) = (&entity.platform, &entity.platform_handle) {
            keys.push(normalize::link_key(platform, handle));
        }
        keys
    }
}

#[async_trait::async_trait]
impl Blocker for NameBlocker {
    fn changed(&mut self, entity: &model::knowledge_entity::Model) {
        for key in Self::keys_of(entity) {
            self.blocks.entry(key).or_default().insert(entity.id);
        }
    }

    async fn candidates(&self, observation: &model::user_observed_entity::Model) -> Vec<i32> {
        let observation = Observation::from(observation);
        let mut found: BTreeSet<i32> = BTreeSet::new();
        for block in [&observation.name_key, &observation.token_key] {
            if let Some(hits) = self.blocks.get(block) {
                found.extend(hits.iter().copied());
            }
        }
        found.into_iter().collect()
    }
}

/// Take the highest-scoring candidate that clears the bar; otherwise start a new entity.
///
/// A name alone never decides — [`worth_merging`] requires corroboration — so this is a thin shell
/// over pure functions. It exists so that "what to do" has exactly one home.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicLinker;

impl Linker for HeuristicLinker {
    fn decide(
        &self,
        observation: &model::user_observed_entity::Model,
        candidates: &[&model::knowledge_entity::Model],
    ) -> Decision {
        let observation = Observation::from(observation);
        let best = candidates
            .iter()
            .map(|entity| (*entity, score(&observation, &Scored::from(*entity))))
            .max_by(|a, b| a.1.total.total_cmp(&b.1.total));

        match best {
            Some((entity, features)) if worth_merging(&features) => Decision::Attach {
                entity: entity.id,
                score: features.total,
            },
            Some((entity, features)) => Decision::Create {
                best: Some((entity.id, features.total)),
            },
            None => Decision::Create { best: None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(name: &str, description: Option<&str>, ty: &str) -> Observation {
        Observation::new(name, description, ty)
    }

    fn entity(name: &str, description: Option<&str>, ty: &str) -> Scored {
        let descriptions: Vec<&str> = description.into_iter().collect();
        Scored::new(1, name, &[], &descriptions, &[ty.to_string()])
    }

    /// `brand` and `organization` are the same referent described twice — the taxonomy does not have
    /// to choose for a member to agree.
    #[test]
    fn any_asserted_type_counts_as_agreement() {
        let entity = Scored::new(
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

    fn entity_view(id: i32, name: &str, ty: &str) -> model::knowledge_entity::Model {
        crate::knowledge::entity::scratch(id, name, ty)
    }

    fn observation_view(name: &str, ty: &str) -> model::user_observed_entity::Model {
        crate::knowledge::entity::scratch_observation(1, name, ty)
    }

    /// The bug that made every later capture duplicate an existing name: `project` keeps the
    /// selected surface out of `alt_names`, so an entity with no alternatives must still be
    /// findable by its own name.
    #[tokio::test]
    async fn the_selected_name_is_a_block_key_on_its_own() {
        let blocker =
            NameBlocker::build_entities(&[entity_view(7, "Francis Ford Coppola", "real_person")]);

        let candidates = blocker
            .candidates(&observation_view("Francis Ford Coppola", "real_person"))
            .await;
        assert_eq!(candidates, vec![7]);
    }

    #[tokio::test]
    async fn word_order_alone_still_blocks() {
        let blocker = NameBlocker::build_entities(&[entity_view(7, "Train Dreams", "book")]);

        let candidates = blocker
            .candidates(&observation_view("Dreams Train", "book"))
            .await;
        assert_eq!(candidates, vec![7], "token-set equality is a block key");
    }

    #[tokio::test]
    async fn distinct_names_propose_nothing() {
        let blocker = NameBlocker::build_entities(&[entity_view(7, "Train Dreams", "book")]);

        assert!(blocker
            .candidates(&observation_view("Apocalypse Now", "movie"))
            .await
            .is_empty());
    }

    /// The mid-run path: an entity built after the corpus is findable by it.
    #[tokio::test]
    async fn an_entity_added_mid_run_is_findable() {
        let mut blocker = NameBlocker::build_entities(&[entity_view(7, "Train Dreams", "book")]);
        blocker.changed(&entity_view(9, "Apocalypse Now", "movie"));

        let candidates = blocker
            .candidates(&observation_view("Apocalypse Now", "movie"))
            .await;
        assert_eq!(candidates, vec![9]);
    }

    #[test]
    fn the_linker_takes_the_best_candidate_that_clears_the_bar() {
        let weak = entity_view(1, "Train Dreams", "book");
        let strong = entity_view(2, "Train Dreams", "book");
        let observation = observation_view("Train Dreams", "book");

        match HeuristicLinker.decide(&observation, &[&weak, &strong]) {
            Decision::Attach { entity, score } => {
                assert!(entity == 1 || entity == 2);
                assert!(score >= 0.6, "{score}");
            }
            other => panic!("expected an attach, got {other:?}"),
        }
    }

    #[test]
    fn the_linker_creates_when_nothing_clears_the_bar() {
        let mut other = entity_view(1, "Apocalypse Now", "movie");
        other.description = Some("a 1979 film by Francis Ford Coppola".to_string());

        match HeuristicLinker.decide(&observation_view("Train Dreams", "book"), &[&other]) {
            Decision::Create { best } => assert!(best.is_some(), "the rejected candidate is kept"),
            other => panic!("expected a create, got {other:?}"),
        }
    }

    #[test]
    fn no_candidates_creates_with_nothing_to_report() {
        match HeuristicLinker.decide(&observation_view("Anything", "unknown"), &[]) {
            Decision::Create { best } => assert!(best.is_none()),
            other => panic!("expected a create, got {other:?}"),
        }
    }
}
