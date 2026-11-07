//! The blocker that reads the database instead of holding a copy of it.
//!
//! `NameBlocker` loads the whole graph into a map per process, so it knows only this process's
//! writes and pays a full scan whenever it is built. This one loads nothing: `knowledge_index` is
//! already folded, indexed, and kept current by `project`, so a probe is one `&&` against a GIN.
//!
//! The candidate set is the same by construction — both read the same two key generations, folded
//! the same way — and `eval blocking` measures that rather than assuming it.

use sea_orm::{DatabaseBackend, EntityTrait, Statement};

use crate::database::DbHandle;
use crate::knowledge::blocker::Blocker;
use crate::knowledge::normalize;
use crate::model;

/// Reads `knowledge_index` for every candidate.
#[derive(Clone)]
pub struct SqlBlocker {
    db: DbHandle,
}

impl SqlBlocker {
    pub fn new(db: &DbHandle) -> Self {
        Self { db: db.clone() }
    }
}

/// Candidates wear a key from either generation. The casts are on the *parameter*: the columns are
/// `varchar[]` (SeaORM maps `Vec<String>` that way), and casting the indexed side instead would hide
/// both GIN indexes from the planner.
const CANDIDATES: &str = "
SELECT e.* FROM knowledge_entities e
  JOIN knowledge_index i ON i.knowledge_entity_id = e.id
 WHERE e.merged_into_id IS NULL
   AND (i.surfaces && $1::varchar[] OR i.word_keys && $2::varchar[])
 ORDER BY e.id";

#[async_trait::async_trait]
impl Blocker for SqlBlocker {
    /// Candidates for one observation, ordered by id so ties break exactly as they do in memory.
    ///
    /// Pure and complete for the keys it stores: same corpus, same answer. Nothing is held between
    /// calls, which is why `changed` and `forget` keep their no-op defaults.
    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> Vec<model::knowledge_entity::Model> {
        let folded = normalize::comparable(&observation.name);
        if folded.is_empty() {
            return Vec::new();
        }
        let words = normalize::word_key(&folded);
        match model::knowledge_entity::Entity::find()
            .from_raw_sql(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                CANDIDATES,
                [vec![folded].into(), vec![words].into()],
            ))
            .all(&self.db.conn)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                // A blocker has no channel for errors, and an empty set is a decision (create).
                // Log loudly: silence here looks exactly like "nothing matched".
                tracing::error!(%error, observation = %observation.name, "sql blocker query failed");
                Vec::new()
            }
        }
    }
}

/// The ids this blocker would offer, for tests and for `eval`.
pub async fn candidate_ids(
    db: &DbHandle,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<Vec<i32>> {
    Ok(SqlBlocker::new(db)
        .candidates(observation)
        .await
        .into_iter()
        .map(|row| row.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::knowledge::heuristic::NameBlocker;
    use crate::test_support::{db_or_skip, fixtures};

    /// The same observation, answered by both blockers. The index has to have been written, which
    /// `project` does when the entity is created.
    #[tokio::test]
    async fn the_sql_blocker_matches_the_in_memory_one() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::Knowledge::new(&db).update_projection(id).await.unwrap();

        let oracle = NameBlocker::new(&db).await.unwrap();
        let sql = SqlBlocker::new(&db);
        for name in ["Train Dreams", "Dreams Train", "train dreams", "Dune"] {
            let observation = fixtures::stub_observation(1, name, "book");
            let expected: Vec<i32> = oracle
                .candidates(&observation)
                .await
                .into_iter()
                .map(|row| row.id)
                .collect();
            let got: Vec<i32> = sql
                .candidates(&observation)
                .await
                .into_iter()
                .map(|row| row.id)
                .collect();
            assert_eq!(got, expected, "observation {name:?}");
        }
    }

    /// The word generation is what connects the two spellings; the folded surface alone does not.
    #[tokio::test]
    async fn a_word_order_difference_is_found_by_the_second_generation() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::Knowledge::new(&db).update_projection(id).await.unwrap();

        let observation = fixtures::stub_observation(1, "Dreams Train", "book");
        assert_eq!(candidate_ids(&db, &observation).await.unwrap(), vec![id]);
    }

    /// An observation whose name folds to nothing asks for nothing; it must not return the corpus.
    #[tokio::test]
    async fn an_empty_name_yields_no_candidates() {
        db_or_skip!(db);
        fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        assert!(candidate_ids(&db, &fixtures::stub_observation(1, "   ", "book"))
            .await
            .unwrap()
            .is_empty());
    }
}
