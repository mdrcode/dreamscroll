//! The blocker: candidates read out of `knowledge_blocking_index` instead of a copy of the graph.
//!
//! Nothing is loaded and nothing is held between probes. `knowledge_blocking_index` is already folded,
//! indexed and kept current by `project`, so a probe is one `&&` against a GIN — and a second
//! writer's commits are visible immediately, which an in-memory map could not promise.

use sea_orm::{
    ColumnTrait, EntityTrait, ExprTrait, JoinType, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait, Value, sea_query::Expr,
};

use super::Blocker;
use crate::database::DbHandle;
use crate::knowledge::text;
use crate::model;

/// Reads `knowledge_blocking_index` for every candidate.
#[derive(Clone)]
pub struct SqlBlocker {
    db: DbHandle,
}

impl SqlBlocker {
    pub fn new(db: &DbHandle) -> Self {
        Self { db: db.clone() }
    }
}

/// The one thing the SeaORM query builder cannot say: Postgres's array-overlap operator.
///
/// The cast is on the *parameter*. The columns are `varchar[]`, and casting the indexed side instead
/// would hide the GIN indexes from the planner. Two of them because the index stores two key
/// generations: the folded surface, and its order- and article-insensitive word key.
///
/// `$1`, not `?`: sea-query substitutes the placeholder *style of the backend*, so Postgres wants
/// `$n` here. The number indexes this expression's own values, not the statement's — each predicate
/// is rendered with its own bind, whatever the surrounding query numbers it.
const OVERLAP_SURFACES: &str = "\"knowledge_blocking_index\".\"surfaces\" && $1::varchar[]";
const OVERLAP_WORDS: &str = "\"knowledge_blocking_index\".\"word_keys\" && $1::varchar[]";

#[async_trait::async_trait]
impl Blocker for SqlBlocker {
    /// Candidates for one observation, ordered by id so ties break exactly as they do in memory.
    ///
    /// A capture can name the same thing two ways — a display name in one reading, the handle in
    /// another — and the entity side stores both, so the probe asks for both. Measured 2026-11-07:
    /// 380 of 2,663 observations carry a handle and **3** find an entity by it that the name misses.
    ///
    /// Recall, not a decision: the decider still decides. `Andra` (`real_person`,
    /// `x_twitter:bioavailablend`) and `@bioavailablend` (`social_media_account`,
    /// `instagram:bioavailablend`) are now offered to each other and stay two entities — the pair
    /// differs in *type*, which is a question about the graph's vocabulary, not about blocking.
    async fn candidates(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> Vec<model::knowledge_entity::Model> {
        self.probe([
            observation.name.as_str(),
            observation.platform_handle.as_deref().unwrap_or_default(),
        ])
        .await
    }
}

impl SqlBlocker {
    /// Candidates for a bare surface, so an inspector can ask without inventing an observation.
    pub async fn candidates_for_name(&self, name: &str) -> Vec<model::knowledge_entity::Model> {
        self.probe([name]).await
    }

    /// One probe: fold each surface, then ask the index for either key generation.
    async fn probe<'a>(
        &self,
        surfaces: impl IntoIterator<Item = &'a str>,
    ) -> Vec<model::knowledge_entity::Model> {
        let mut keys: Vec<String> = surfaces
            .into_iter()
            .map(text::comparable)
            .filter(|key| !key.is_empty())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        if keys.is_empty() {
            return Vec::new();
        }
        let words: Vec<String> = keys
            .iter()
            .map(|key| text::word_key(key))
            // An empty key would be `ARRAY['']`, which overlaps every row that stored one.
            .filter(|key| !key.is_empty())
            .collect();
        let query = model::knowledge_entity::Entity::find()
            .join(
                JoinType::InnerJoin,
                model::knowledge_blocking_index::Relation::KnowledgeEntity
                    .def()
                    .rev(),
            )
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .filter(
                Expr::cust_with_values(OVERLAP_SURFACES, [Value::from(keys)])
                    .or(Expr::cust_with_values(OVERLAP_WORDS, [Value::from(words)])),
            )
            .order_by_asc(model::knowledge_entity::Column::Id);
        match query.all(&self.db.conn).await {
            Ok(rows) => rows,
            Err(error) => {
                // A blocker has no channel for errors, and an empty set is a decision (create).
                // Log loudly: silence here looks exactly like "nothing matched".
                tracing::error!(%error, "sql blocker query failed");
                Vec::new()
            }
        }
    }
}

/// The ids this blocker would offer, for tests and for `eval`.
#[cfg(test)]
pub(crate) async fn candidate_ids(
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

    use sea_orm::ConnectionTrait;

    use crate::test_support::{db_or_skip, fixtures};

    /// The word generation is what connects the two spellings; the folded surface alone does not.
    #[tokio::test]
    async fn a_word_order_difference_is_found_by_the_second_generation() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        crate::knowledge::KnowledgeGraph::new(&db)
            .update_projection(id)
            .await
            .unwrap();

        let observation = fixtures::stub_observation(1, "Dreams Train", "book");
        assert_eq!(candidate_ids(&db, &observation).await.unwrap(), vec![id]);
    }

    /// A capture can name an account by its handle where another named it by display name. Both
    /// keys are indexed, so the probe must ask for both: `Andra` and `@bioavailablend` were one
    /// account split into two entities, and no proximity could catch it — they share no identity.
    #[tokio::test]
    async fn a_handle_finds_an_entity_named_differently() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Andra").await;
        // The handle reaches the index through projection: `entity.surfaces()` is name + alt_names,
        // so the key is only there because `project` gathered it from the member observation.
        db.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE user_observed_entities SET platform = 'x_twitter', platform_handle = 'BioavailableNd' \
                  WHERE knowledge_entity_id = $1",
                [id.into()],
            ))
            .await
            .unwrap();
        crate::knowledge::KnowledgeGraph::new(&db)
            .update_projection(id)
            .await
            .unwrap();

        // A surname is the probe: `Andra T.` would now reach it by name, because the fold drops
        // initials, and that is intended.
        let mut observation = fixtures::stub_observation(1, "Andra Thompson", "real_person");
        observation.platform = Some("x_twitter".to_string());
        observation.platform_handle = Some("bioavailablend".to_string());

        let blocker = SqlBlocker::new(&db);
        assert!(
            blocker
                .candidates_for_name("Andra Thompson")
                .await
                .is_empty(),
            "the name alone does not reach it"
        );
        // …while an initial does, which is the point of dropping them.
        assert_eq!(
            blocker
                .candidates_for_name("Andra T.")
                .await
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            vec![id]
        );
        let found = blocker.candidates(&observation).await;
        assert_eq!(found.iter().map(|row| row.id).collect::<Vec<_>>(), vec![id]);
    }

    /// An observation whose name folds to nothing asks for nothing; it must not return the corpus.
    #[tokio::test]
    async fn an_empty_name_yields_no_candidates() {
        db_or_skip!(db);
        fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        assert!(
            candidate_ids(&db, &fixtures::stub_observation(1, "   ", "book"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// A row written before the index stopped storing empty word keys still holds one. The probe
    /// must not ask for it: `ARRAY['']` overlaps every such row, so two dot-only names that fold
    /// differently (`.` and `..`) would be offered to each other with no evidence.
    #[tokio::test]
    async fn a_wordless_probe_does_not_match_a_stored_empty_key() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "..").await;
        crate::knowledge::KnowledgeGraph::new(&db)
            .update_projection(id)
            .await
            .unwrap();
        // The state an older build left behind.
        db.conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE knowledge_blocking_index SET word_keys = ARRAY[''] \
                  WHERE knowledge_entity_id = $1",
                [id.into()],
            ))
            .await
            .unwrap();

        // `...` folds to `..`, the stored surface; only the empty word key could connect them.
        let observation = fixtures::stub_observation(1, "...", "unknown");
        assert!(
            candidate_ids(&db, &observation).await.unwrap().is_empty(),
            "no words on either side is no evidence"
        );
    }
}
