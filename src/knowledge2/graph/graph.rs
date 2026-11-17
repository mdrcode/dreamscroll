//! The facade: one transaction per call, one match per mutation.
//!
//! It has no linker, no blocker, no resolver. Callers decide; this applies. The only derived write
//! is projection — the entity's columns and its blocking-index row — and it runs inside the same
//! transaction.

use std::collections::BTreeSet;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, Statement, TransactionTrait,
};

use crate::database::DbHandle;
use crate::knowledge2::mutation::GraphMutation;
use crate::model;

use super::report::{Outcome, Report};
use super::{project, sql};

/// The graph, as the outside world sees it.
#[derive(Clone)]
pub struct KnowledgeGraph2 {
    conn: DatabaseConnection,
}

/// What a merge would do, without doing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergePreview {
    pub observations: u64,
    pub identities: u64,
    /// Illuminations both entities appear in. Nonzero means the merge is refused.
    pub shared_illuminations: i64,
    /// A `distinct` verdict names this pair, so the merge is refused in either direction.
    pub standing_verdict: bool,
}

impl KnowledgeGraph2 {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            conn: db.conn.clone(),
        }
    }

    /// Apply every mutation in one transaction.
    ///
    /// Mutations run in canonical order, not caller order. A refusal rolls the whole call back:
    /// nothing is half-applied. `NoChange` is success.
    pub async fn proc(&self, mutations: &[GraphMutation]) -> anyhow::Result<Report> {
        let ordered = order(mutations);
        let txn = self.conn.begin().await?;
        let mut report = Report::default();
        let mut touched = BTreeSet::new();

        for mutation in ordered {
            match sql::apply(&txn, mutation).await? {
                Outcome::Applied(entities) => {
                    report.applied += 1;
                    touched.extend(entities);
                }
                Outcome::NoChange => report.unchanged += 1,
                Outcome::Refused(refusal) => {
                    txn.rollback().await?;
                    // Nothing was applied; a refusal is the whole call's outcome.
                    return Ok(Report {
                        refused: Some(refusal),
                        ..Report::default()
                    });
                }
            }
        }

        report.touched = touched.iter().copied().collect();
        for entity in touched {
            project::project(&txn, entity).await?;
        }

        txn.commit().await?;
        Ok(report)
    }

    /// Single-entity maintenance. `proc` already projects every entity it touches; this exists for
    /// tests that insert fixture rows directly.
    #[cfg(test)]
    pub(crate) async fn update_projection(
        &self,
        entity: i32,
    ) -> anyhow::Result<crate::model::knowledge_entity::Model> {
        project::project(&self.conn, entity).await
    }

    /// Batch maintenance for out-of-band changes and projection migrations. Normal writes project
    /// through `proc`; this is the only path that recomputes entities no mutation touched.
    pub async fn reproject(&self, scope: super::ReprojectScope) -> anyhow::Result<usize> {
        project::reproject(&self.conn, scope).await
    }

    /// What a merge would do, without doing it.
    pub async fn merge_preview(
        &self,
        loser: i32,
        winner: i32,
    ) -> anyhow::Result<MergePreview> {
        let observations = model::user_observed_entity::Entity::find()
            .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(loser))
            .count(&self.conn)
            .await?;
        let identities = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(loser))
            .count(&self.conn)
            .await?;
        let shared_illuminations = shared_illuminations(&self.conn, loser, winner).await?;

        let selector = self.selector();
        let standing_verdict =
            match (selector.origin_of(loser).await?, selector.origin_of(winner).await?) {
                (Some(loser), Some(winner)) => {
                    selector.verdicts().kept_apart(loser, winner).await?
                }
                _ => false,
            };

        Ok(MergePreview {
            observations,
            identities,
            shared_illuminations,
            standing_verdict,
        })
    }

    /// A read view over the graph. Writes and projection stay here; selection lives on the selector.
    pub fn selector(&self) -> super::GraphSelector<'_> {
        super::GraphSelector::new(&self.conn)
    }
}

/// Dependency order, not preference:
///
/// 1. lift a standing veto before evidence it would block
/// 2. create, so later mutations can name what it made
/// 3. membership
/// 4. fold, so evidence lands on the survivor
/// 5. evidence
/// 6. assert a standing denial last
async fn shared_illuminations<C: ConnectionTrait>(
    conn: &C,
    loser: i32,
    winner: i32,
) -> anyhow::Result<i64> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(DISTINCT l.illumination_id) FROM user_observed_entities l \
             JOIN user_observed_entities w ON w.illumination_id = l.illumination_id \
             WHERE l.knowledge_entity_id = $1 AND w.knowledge_entity_id = $2",
            [loser.into(), winner.into()],
        ))
        .await?;
    Ok(row
        .map(|row| row.try_get_by_index::<i64>(0))
        .transpose()?
        .unwrap_or(0))
}

fn order(mutations: &[GraphMutation]) -> Vec<&GraphMutation> {
    let rank = |mutation: &GraphMutation| match mutation {
        GraphMutation::UnvetoIdentity { .. } => 0,
        GraphMutation::CreateEntity { .. } => 1,
        GraphMutation::AttachObservation { .. } | GraphMutation::DetachObservation { .. } => 2,
        GraphMutation::MergeEntity { .. } => 3,
        GraphMutation::Identify { .. } | GraphMutation::RetractIdentity { .. } => 4,
        GraphMutation::VetoIdentity { .. } | GraphMutation::MarkDistinct { .. } => 5,
    };

    let mut ordered: Vec<&GraphMutation> = mutations.iter().collect();
    ordered.sort_by_key(|mutation| rank(mutation));
    ordered
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    use super::*;
    use crate::knowledge2::mutation::Cause;
    use crate::knowledge2::identity::{Identity, Provider};
    use crate::knowledge2::ids::{EntityOriginId, ObservationId};
    use crate::model;
    use crate::test_support::{db_or_skip, fixtures};

    fn identity(scheme: &str, external_id: &str) -> Identity {
        Identity {
            provider: Provider("model".into()),
            scheme: scheme.into(),
            external_id: external_id.into(),
            method: Some("test".into()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: vec!["film".into()],
            observation_id: None,
        }
    }

    #[tokio::test]
    async fn create_writes_the_entity_and_its_identity() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "k2_create").await;
        let observation = fixtures::observe(
            &db,
            &capture,
            0,
            "Train Dreams",
            Some("a film"),
            "film",
            None,
            None,
        )
        .await;
        let graph = KnowledgeGraph2::new(&db);

        let report = graph
            .proc(&[GraphMutation::CreateEntity {
                from: ObservationId(observation),
                with: Some(identity("wikidata", "Q1")),
            }])
            .await
            .unwrap();

        assert_eq!(report.applied, 1);
        assert!(report.refused.is_none());

        let entity = model::knowledge_entity::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("one entity");
        assert_eq!(entity.origin_observation_id, Some(observation));
        assert_eq!(entity.name, "Train Dreams");
        assert_eq!(entity.entity_type, "film");
        assert_eq!(entity.status, model::knowledge_entity::Status::Identified);
        assert!(entity.confidence.is_some());

        let linked = model::user_observed_entity::Entity::find_by_id(observation)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the observation");
        assert_eq!(linked.knowledge_entity_id, Some(entity.id));

        let annotation = model::knowledge_identity::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .expect("one identity");
        assert_eq!(annotation.classes, vec!["film"]);
    }

    #[tokio::test]
    async fn a_refusal_rolls_back_the_whole_call() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "k2_rollback").await;
        let first =
            fixtures::observe(&db, &capture, 0, "Train Dreams", None, "film", None, None).await;
        let second =
            fixtures::observe(&db, &capture, 1, "Train Dreams", None, "film", None, None).await;
        let graph = KnowledgeGraph2::new(&db);

        let report = graph
            .proc(&[
                GraphMutation::CreateEntity {
                    from: ObservationId(first),
                    with: None,
                },
                GraphMutation::AttachObservation {
                    observation: ObservationId(second),
                    entity: EntityOriginId(ObservationId(999_999)),
                    with: None,
                    why: None,
                },
            ])
            .await
            .unwrap();

        assert!(report.is_refused());
        assert_eq!(
            model::knowledge_entity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            model::user_observed_entity::Entity::find()
                .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
                .count(&db.conn)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn merge_moves_members_and_tombstones_the_loser() {
        db_or_skip!(db);
        let left_capture = fixtures::Capture::new(&db, "k2_merge_left").await;
        let right_capture = fixtures::Capture::new(&db, "k2_merge_right").await;
        let left = fixtures::observe(
            &db,
            &left_capture,
            0,
            "Train Dreams",
            None,
            "film",
            None,
            None,
        )
        .await;
        let right = fixtures::observe(
            &db,
            &right_capture,
            0,
            "Train Dreams",
            None,
            "film",
            None,
            None,
        )
        .await;
        let graph = KnowledgeGraph2::new(&db);

        graph
            .proc(&[
                GraphMutation::CreateEntity {
                    from: ObservationId(left),
                    with: None,
                },
                GraphMutation::CreateEntity {
                    from: ObservationId(right),
                    with: None,
                },
            ])
            .await
            .unwrap();

        let entities = model::knowledge_entity::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        let left_entity = entities
            .iter()
            .find(|entity| entity.origin_observation_id == Some(left))
            .expect("left entity");
        let right_entity = entities
            .iter()
            .find(|entity| entity.origin_observation_id == Some(right))
            .expect("right entity");

        let report = graph
            .proc(&[GraphMutation::MergeEntity {
                loser: EntityOriginId(ObservationId(left)),
                winner: EntityOriginId(ObservationId(right)),
                cause: Cause::Model,
                why: "the same film".into(),
            }])
            .await
            .unwrap();
        assert_eq!(report.applied, 1);
        assert!(report.refused.is_none());

        let moved = model::user_observed_entity::Entity::find_by_id(left)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the moved observation");
        assert_eq!(moved.knowledge_entity_id, Some(right_entity.id));

        let loser = model::knowledge_entity::Entity::find_by_id(left_entity.id)
            .one(&db.conn)
            .await
            .unwrap()
            .expect("the loser");
        assert_eq!(loser.merged_into_id, Some(right_entity.id));
    }

    #[tokio::test]
    async fn merge_preview_counts_members_and_standing_distinct() {
        db_or_skip!(db);
        let left_capture = fixtures::Capture::new(&db, "k2_preview_left").await;
        let right_capture = fixtures::Capture::new(&db, "k2_preview_right").await;
        let left = fixtures::observe(
            &db,
            &left_capture,
            0,
            "Train Dreams",
            None,
            "film",
            None,
            None,
        )
        .await;
        let right = fixtures::observe(
            &db,
            &right_capture,
            0,
            "Train Dreams",
            None,
            "film",
            None,
            None,
        )
        .await;
        let graph = KnowledgeGraph2::new(&db);

        graph
            .proc(&[
                GraphMutation::CreateEntity {
                    from: ObservationId(left),
                    with: Some(identity("wikidata", "Q1")),
                },
                GraphMutation::CreateEntity {
                    from: ObservationId(right),
                    with: None,
                },
            ])
            .await
            .unwrap();

        let entities = model::knowledge_entity::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        let loser = entities
            .iter()
            .find(|entity| entity.origin_observation_id == Some(left))
            .expect("left entity");
        let winner = entities
            .iter()
            .find(|entity| entity.origin_observation_id == Some(right))
            .expect("right entity");

        let preview = graph.merge_preview(loser.id, winner.id).await.unwrap();
        assert_eq!(preview.observations, 1);
        assert_eq!(preview.identities, 1);
        assert_eq!(preview.shared_illuminations, 0);
        assert!(!preview.standing_verdict);

        graph
            .proc(&[GraphMutation::MarkDistinct {
                a: EntityOriginId(ObservationId(left)),
                b: EntityOriginId(ObservationId(right)),
                cause: Cause::Repair,
                why: "two different works".into(),
            }])
            .await
            .unwrap();

        let preview = graph.merge_preview(loser.id, winner.id).await.unwrap();
        assert!(preview.standing_verdict);
    }
}
