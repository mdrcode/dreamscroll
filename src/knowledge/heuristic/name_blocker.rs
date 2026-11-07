//! The blocker: recall, and only recall.
//!
//! `NameBlocker` puts an entity in front of the linker when a name, or the account a capture showed,
//! could plausibly be the same thing. It over-generates by design — precision is the linker's job —
//! and it never scores anything: the candidate set it returns is what
//! [`Similarity`](crate::knowledge::scoring::Similarity) is asked about.
//!
//! Both keys are cheap and generous: a shared word-order-insensitive name, or a `(scheme, handle)`
//! an observation carries. Nothing here knows about identity or type — those narrow, they do not
//! find.

use std::collections::{BTreeSet, HashMap};

use sea_orm::{EntityTrait, QueryOrder};

use crate::model;

use crate::knowledge::blocker::Blocker;
use crate::knowledge::normalize;
use crate::knowledge::evidence::Claim;


/// Blocks on normalized names and the account a capture showed.
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
            let key = normalize::comparable(surface);
            if !key.is_empty() {
                keys.extend([normalize::word_key(&key), key]);
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
        let key = normalize::comparable(&observation.name);
        if key.is_empty() {
            return Vec::new();
        }
        let blocking = normalize::word_key(&key);
        let index = self.index();
        let mut found: BTreeSet<i32> = BTreeSet::new();
        for block in [&key, &blocking] {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entity_row(id: i32, name: &str, ty: &str) -> model::knowledge_entity::Model {
        crate::test_support::fixtures::stub_entity(id, name, ty)
    }

    fn observation_row(name: &str, ty: &str) -> model::user_observed_entity::Model {
        crate::test_support::fixtures::stub_observation(1, name, ty)
    }

    fn ids(candidates: &[model::knowledge_entity::Model]) -> Vec<i32> {
        candidates.iter().map(|entity| entity.id).collect()
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
}
