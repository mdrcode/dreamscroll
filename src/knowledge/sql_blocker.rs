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

/// Which key generations a probe consults.
///
/// `Both` is the shipping behaviour. `Surfaces` exists so the word generation can be A/B'd against
/// the folded surface alone — it is a weaker claim about identity and has to earn its keep
/// (plan/backlog.md), not inherit it from parity with the old blocker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generations {
    Both,
    Surfaces,
}

/// Reads `knowledge_index` for every candidate.
#[derive(Clone)]
pub struct SqlBlocker {
    db: DbHandle,
    generations: Generations,
}

impl SqlBlocker {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            generations: Generations::Both,
        }
    }

    /// Folded surfaces only: an order-only or leading-article difference does not match.
    pub fn surfaces_only(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            generations: Generations::Surfaces,
        }
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

/// The same, without the word generation.
const CANDIDATES_SURFACES: &str = "
SELECT e.* FROM knowledge_entities e
  JOIN knowledge_index i ON i.knowledge_entity_id = e.id
 WHERE e.merged_into_id IS NULL
   AND i.surfaces && $1::varchar[]
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
        self.candidates_for_name(&observation.name).await
    }
}

impl SqlBlocker {
    /// Candidates for a bare surface, so an inspector can ask without inventing an observation.
    pub async fn candidates_for_name(&self, name: &str) -> Vec<model::knowledge_entity::Model> {
        let folded = normalize::comparable(name);
        if folded.is_empty() {
            return Vec::new();
        }
        let (sql, values) = match self.generations {
            Generations::Both => (
                CANDIDATES,
                vec![
                    sea_orm::Value::from(vec![folded.clone()]),
                    sea_orm::Value::from(vec![normalize::word_key(&folded)]),
                ],
            ),
            Generations::Surfaces => (CANDIDATES_SURFACES, vec![sea_orm::Value::from(vec![folded])]),
        };
        match model::knowledge_entity::Entity::find()
            .from_raw_sql(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                sql,
                values,
            ))
            .all(&self.db.conn)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                // A blocker has no channel for errors, and an empty set is a decision (create).
                // Log loudly: silence here looks exactly like "nothing matched".
                tracing::error!(%error, surface = name, "sql blocker query failed");
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
    use sea_orm::ConnectionTrait;

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

    /// The generation's effect on a *decision*, in the only situation it can have one: an existing
    /// entity, and a later observation that names it differently. A full rebuild never revisits an
    /// observation, which is why the corpus A/B shows zero — the difference is incremental.
    #[tokio::test]
    async fn the_word_generation_can_change_the_decision() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        // The fixture's member reads as `unknown`, and `worth_merging` wants a score above 0.60:
        // name 0.9 alone plus a 0.5 type lands at 0.59. Give it a real type so the only difference
        // between the two arms is the key generation.
        db.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE user_observed_entities SET entity_type = 'book' WHERE knowledge_entity_id = $1",
                [id.into()],
            ))
            .await
            .unwrap();
        crate::knowledge::Knowledge::new(&db).update_projection(id).await.unwrap();

        let scoring = crate::knowledge::heuristic::Heuristic::scoring();
        let observation = fixtures::stub_observation(1, "Dreams Train", "book");

        let both = SqlBlocker::new(&db).candidates(&observation).await;
        assert_eq!(
            scoring.link.rank(&observation, &both).len(),
            1,
            "the word generation finds it, so the linker can attach"
        );

        let surfaces = SqlBlocker::surfaces_only(&db).candidates(&observation).await;
        assert!(
            scoring.link.rank(&observation, &surfaces).is_empty(),
            "without it there is no candidate, and the decision is Create"
        );
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
