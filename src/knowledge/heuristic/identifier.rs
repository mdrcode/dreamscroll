//! The heuristic identifier: ask Wikidata for a batch of identities.

use crate::fetch::Gateway;
use crate::knowledge::identity::Identity;
use crate::model;

use super::identification::Wikidata;

/// The heuristic provider over Wikidata.
pub struct HeuristicIdentifier {
    wikidata: Wikidata,
    #[cfg(test)]
    test_identity: Option<Identity>,
}

impl HeuristicIdentifier {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            wikidata: Wikidata::new()?,
            #[cfg(test)]
            test_identity: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_test_identity(identity: Identity) -> anyhow::Result<Self> {
        Ok(Self {
            wikidata: Wikidata::new()?,
            test_identity: Some(identity),
        })
    }

    /// The identities Wikidata asserts for this batch.
    pub(crate) async fn identify_batch(
        &self,
        batch: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)> {
        #[cfg(test)]
        if let Some(identity) = &self.test_identity {
            return batch
                .iter()
                .map(|entity| (entity.id, identity.clone()))
                .collect();
        }
        self.wikidata.identify(batch, gateway).await
    }
}
