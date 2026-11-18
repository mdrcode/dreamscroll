//! The identity vocabulary: who asserts a key, the key itself, and the assertion.
//!
//! The caller is the policy, so the assertion carries its provider. The identity key is the stable
//! pair `(scheme, external_id)`; metadata belongs to the provider that answered.

use serde::{Deserialize, Serialize};

use crate::illumination::v2::PlatformLink;

use super::ids::ObservationId;
use super::text;

/// Who asserted an identity: `illuminator`, `wikidata`, `model`, `user`, `operator`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Provider(pub String);

/// Provider ids that name our own pipeline, not an authority.
pub const RESERVED_PROVIDER_IDS: &[&str] =
    &["illuminator", "platform", "model", "operator", "user"];

/// The durable name of an identity: `(scheme, external_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityKey {
    pub scheme: String,
    pub external_id: String,
}

/// One provider's assertion of one identity about one entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub provider: Provider,
    pub scheme: String,
    pub external_id: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub classes: Vec<String>,
    /// The observation that carried it, when a capture showed it.
    #[serde(default)]
    pub observation_id: Option<ObservationId>,
}

impl Identity {
    /// The platform link as an identity, when it carries a handle.
    ///
    /// The caller decides which link, if any, to promote; this only folds the link's key.
    pub fn from_platform_link(link: &PlatformLink) -> Option<Self> {
        let handle = link.handle()?;
        Some(Self {
            provider: Provider("illuminator".into()),
            scheme: text::comparable(link.platform_name()),
            external_id: text::comparable(handle),
            method: Some("platform_link".into()),
            confidence: None,
            label: link.display_name.clone(),
            description: None,
            classes: Vec::new(),
            observation_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::illumination::v2::Platform;

    #[test]
    fn a_platform_link_display_name_becomes_the_identity_label() {
        let link = PlatformLink {
            platform: Platform::XTwitter,
            handle: Some("@Bandy1803".into()),
            display_name: Some("Bandy".into()),
            url: None,
        };

        let identity = Identity::from_platform_link(&link).expect("the link has a handle");
        assert_eq!(identity.label.as_deref(), Some("Bandy"));
        assert_eq!(identity.external_id, "bandy1803");
    }

    #[test]
    fn a_platform_link_without_a_display_name_leaves_the_label_empty() {
        let link = PlatformLink {
            platform: Platform::XTwitter,
            handle: Some("Bandy1803".into()),
            display_name: None,
            url: None,
        };

        let identity = Identity::from_platform_link(&link).expect("the link has a handle");
        assert_eq!(identity.label, None);
    }
}
