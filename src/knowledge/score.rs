//! The merge decision, as a pure function.
//!
//! Two questions share this code: "is this candidate this referent?" (merge) and, later, "is this
//! authority item what the capture meant?" (resolution). Keeping one implementation means one set
//! of weights and one meaning for a threshold.
//!
//! Weights and bars are provisional. They must be calibrated against the labelled sheet before
//! anything leans on them (see `plan/knowledge-canonical-entities.md`).

use super::normalize;

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

/// A candidate core, with the label work precomputed so scoring is cheap in a loop.
#[derive(Debug, Clone)]
pub struct Core {
    pub id: i32,
    pub alt_label_keys: Vec<String>,
    pub alt_label_token_keys: Vec<String>,
    /// One token set per accumulated description; a candidate has to fit only one of them.
    pub description_tokens: Vec<Vec<String>>,
    /// Every type this core's members asserted. Agreement with any one of them counts.
    pub entity_types: Vec<String>,
}

impl Core {
    pub fn new(
        id: i32,
        alt_labels: &[String],
        descriptions: &[&str],
        entity_types: &[String],
    ) -> Self {
        Self {
            id,
            alt_label_keys: alt_labels.iter().map(|a| normalize::key(a)).collect(),
            alt_label_token_keys: alt_labels.iter().map(|a| normalize::token_key(a)).collect(),
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

pub fn score(observation: &Observation, core: &Core) -> Features {
    let name = name_similarity(observation, core);
    // Best fit across every description the core has accumulated, never their union: a union would
    // dilute a good match with text from captures that had nothing to do with this observation.
    let description = core
        .description_tokens
        .iter()
        .map(|tokens| jaccard(&observation.description_tokens, tokens))
        .fold(0.0f32, f32::max);
    // Any agreement counts. A core whose members were called both `brand` and `organization` must
    // accept either, or half its own members could not have joined it.
    let type_match = core
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
fn name_similarity(observation: &Observation, core: &Core) -> f32 {
    if core.alt_label_keys.contains(&observation.name_key) {
        return 1.0;
    }
    if core.alt_label_token_keys.contains(&observation.token_key) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(name: &str, description: Option<&str>, ty: &str) -> Observation {
        Observation::new(name, description, ty)
    }

    fn core(name: &str, description: Option<&str>, ty: &str) -> Core {
        let descriptions: Vec<&str> = description.into_iter().collect();
        Core::new(1, &[normalize::key(name)], &descriptions, &[ty.to_string()])
    }

    /// `brand` and `organization` are the same referent described twice — the taxonomy does not have
    /// to choose for a member to agree.
    #[test]
    fn any_asserted_type_counts_as_agreement() {
        let core = Core::new(
            1,
            &[normalize::key("Nintendo")],
            &[],
            &["organization".to_string(), "brand".to_string()],
        );
        let typed = |ty: &str| score(&observation("Nintendo", None, ty), &core).type_match;
        assert_eq!(typed("brand"), 1.0);
        assert_eq!(typed("organization"), 1.0);
        assert_eq!(typed("movie"), 0.0);
    }

    #[test]
    fn identical_name_and_type_clears_the_bar() {
        let features = score(
            &observation("Train Dreams", Some("a novella by Denis Johnson"), "book"),
            &core("Train Dreams", Some("a novella by Denis Johnson"), "book"),
        );
        assert_eq!(features.name, 1.0);
        assert!(worth_merging(&features), "{features:?}");
    }

    #[test]
    fn article_or_word_order_alone_still_clears() {
        let features = score(
            &observation("The Train Dreams", None, "book"),
            &core("Train Dreams", None, "book"),
        );
        assert_eq!(features.name, 0.9);
        assert!(worth_merging(&features), "{features:?}");
    }

    #[test]
    fn a_type_mismatch_without_name_agreement_does_not_clear() {
        let features = score(
            &observation("Django", Some("a Python web framework"), "software"),
            &core("Django", Some("a 2021 video game"), "product"),
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
            &core("Gravity's Rainbow", None, "book"),
        );
        assert_eq!(features.name, 0.0);
        assert!(!worth_merging(&features), "{features:?}");
    }

    #[test]
    fn unknown_type_is_weak_not_negative() {
        let features = score(
            &observation("Wojak", None, "unknown"),
            &core("Wojak", None, "concept"),
        );
        assert_eq!(features.type_match, 0.5);
    }
}
