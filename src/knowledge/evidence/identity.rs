//! The identity vocabulary: an identity's durable name, a platform link a capture showed, and the
//! draft an authority's answer becomes.
//!
//! The row itself lives in `model::knowledge_identity`; these are the shapes `Evidence` writes it
//! from, and the pair every durable reference uses.

use crate::illumination::v2::PlatformLink;
use crate::knowledge::normalize;
use crate::model;

use super::claim::Claim;

/// The provider of an identity a capture showed: the illuminator produced the link, not the
/// platform it names. Reserved — a resolver may never take this id.
pub const ILLUMINATOR: &str = "illuminator";

/// The platform link an observation's flattened columns describe, if it has a platform.
///
/// This is not an identity. It is the observation's optional link; only [`IdentityKey`]'s
/// conversion makes it one.
pub fn platform_link(observation: &model::user_observed_entity::Model) -> Option<PlatformLink> {
    PlatformLink::from_columns(
        observation.platform.clone(),
        observation.platform_handle.clone(),
        observation.platform_display_name.clone(),
    )
}

/// An identity's durable name: the pair a rebuild can reproduce.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentityKey {
    pub scheme: String,
    pub external_id: String,
}

/// A platform link can exist without a handle (a display name alone), but an identity key cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingHandle;

impl TryFrom<&PlatformLink> for IdentityKey {
    type Error = MissingHandle;

    fn try_from(link: &PlatformLink) -> Result<Self, Self::Error> {
        let handle = link.handle().ok_or(MissingHandle)?;
        Ok(Self {
            scheme: normalize::comparable(link.platform_name()),
            external_id: normalize::comparable(handle),
        })
    }
}

/// An identity and the answer whoever gave it.
///
/// The authority's label, description and classes are properties of the identity, not of any one
/// claim: an album's article describes the album whether one entity claims it or five do.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityDraft {
    pub scheme: String,
    pub external_id: String,
    /// Who supplied it: [`ILLUMINATOR`] for a handle a capture showed, else the resolver's id.
    pub provider: String,
    /// How it was answered, on the first answer only: `capture`, a Wikidata property (`P2002`, …) for
    /// a handle join, or the ladder step (`exact_label`, `search_ranked`, …).
    pub method: Option<String>,
    pub confidence: Option<f32>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
}

impl TryFrom<&PlatformLink> for IdentityDraft {
    type Error = MissingHandle;

    /// An identity known only from a platform link: the key, and nothing about the thing.
    fn try_from(link: &PlatformLink) -> Result<Self, Self::Error> {
        let key = IdentityKey::try_from(link)?;
        Ok(Self {
            scheme: key.scheme,
            external_id: key.external_id,
            provider: ILLUMINATOR.to_string(),
            method: Some("capture".to_string()),
            confidence: None,
            label: None,
            description: None,
            classes: Vec::new(),
        })
    }
}

impl IdentityDraft {
    /// The answer an authority gave: it knows what the thing is, not which capture asked.
    pub fn from_claim(claim: Claim, provider: &str) -> Self {
        Self {
            scheme: claim.scheme,
            external_id: claim.external_id,
            provider: provider.to_string(),
            method: Some(claim.method),
            confidence: Some(claim.confidence),
            label: claim.label,
            description: claim.description,
            classes: claim.classes,
        }
    }

    /// The durable name of the identity this draft describes.
    pub fn key(&self) -> IdentityKey {
        IdentityKey {
            scheme: self.scheme.clone(),
            external_id: self.external_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(
        platform: Option<&str>,
        handle: Option<&str>,
    ) -> model::user_observed_entity::Model {
        let mut observation =
            crate::test_support::fixtures::stub_observation(1, "Billy Bob", "real_person");
        observation.platform = platform.map(str::to_string);
        observation.platform_handle = handle.map(str::to_string);
        observation
    }

    #[test]
    fn a_platform_link_maps_to_a_folded_identity_key() {
        let observation = observation(Some("instagram"), Some("BillyBob"));
        let link = platform_link(&observation).expect("a platform link");
        let key = IdentityKey::try_from(&link).expect("a handle");
        assert_eq!(key.scheme, "instagram");
        assert_eq!(key.external_id, "billybob");
    }

    #[test]
    fn a_platform_is_required_for_a_link_and_a_handle_for_a_key() {
        assert!(platform_link(&observation(None, Some("billybob"))).is_none());

        let display_only = platform_link(&observation(Some("instagram"), None))
            .expect("a platform is enough for a link");
        assert_eq!(IdentityKey::try_from(&display_only), Err(MissingHandle));
    }
}
