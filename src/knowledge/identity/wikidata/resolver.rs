//! The v1 `IdentityResolver` adapter over `knowledge2::heuristic::resolution`.

use crate::fetch::Gateway;
use crate::knowledge::evidence::Identity;
use crate::knowledge::identity::IdentityResolver;
use crate::knowledge2::heuristic::resolution::Wikidata as Inner;
use crate::knowledge2::identity::Identity as Identity2;
use crate::knowledge2::wikidata::ID;
use crate::model;

pub struct Wikidata(Inner);

impl Wikidata {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self(Inner::new()?))
    }
}

#[async_trait::async_trait]
impl IdentityResolver for Wikidata {
    fn id(&self) -> &'static str {
        ID
    }

    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)> {
        self.0
            .resolve(entities, gateway)
            .await
            .into_iter()
            .map(|(entity, identity)| (entity, into_v1(identity)))
            .collect()
    }
}

/// v2 identities carry their provider; the v1 writer supplies it from `id()`.
fn into_v1(identity: Identity2) -> Identity {
    Identity {
        scheme: identity.scheme,
        external_id: identity.external_id,
        method: identity.method,
        confidence: identity.confidence,
        label: identity.label,
        description: identity.description,
        classes: identity.classes,
        observation_id: identity.observation_id.map(|id| id.0),
    }
}
