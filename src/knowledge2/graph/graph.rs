//! The facade: one transaction per call, one match per action.
//!
//! It has no linker, no blocker, no resolver. Callers decide; this applies. The only derived write
//! is the identity projection, and it runs inside the same transaction.

use std::collections::BTreeSet;

use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::database::DbHandle;
use crate::knowledge2::action::KnowledgeAction;

use super::report::{Outcome, Report};
use super::{project, sql};

/// The graph, as the outside world sees it.
#[derive(Clone)]
pub struct KnowledgeGraph2 {
    conn: DatabaseConnection,
}

impl KnowledgeGraph2 {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            conn: db.conn.clone(),
        }
    }

    /// Apply every action in one transaction.
    ///
    /// Actions run in canonical order, not caller order. A refusal rolls the whole call back:
    /// nothing is half-applied. `NoChange` is success.
    pub async fn proc(&self, actions: &[KnowledgeAction]) -> anyhow::Result<Report> {
        let ordered = order(actions);
        let txn = self.conn.begin().await?;
        let mut report = Report::default();
        let mut touched = BTreeSet::new();

        for action in ordered {
            match sql::apply(&txn, action).await? {
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
            project::refresh(&txn, entity).await?;
        }

        txn.commit().await?;
        Ok(report)
    }
}

/// Dependency order, not preference:
///
/// 1. lift a standing veto before evidence it would block
/// 2. create, so later actions can name what it made
/// 3. membership
/// 4. fold, so evidence lands on the survivor
/// 5. evidence
/// 6. assert a standing denial last
fn order(actions: &[KnowledgeAction]) -> Vec<&KnowledgeAction> {
    let rank = |action: &KnowledgeAction| match action {
        KnowledgeAction::UnvetoIdentity { .. } => 0,
        KnowledgeAction::CreateEntity { .. } => 1,
        KnowledgeAction::AttachObservation { .. } | KnowledgeAction::DetachObservation { .. } => 2,
        KnowledgeAction::MergeEntity { .. } => 3,
        KnowledgeAction::AttestIdentity { .. } | KnowledgeAction::RetractIdentity { .. } => 4,
        KnowledgeAction::VetoIdentity { .. } | KnowledgeAction::MarkDistinct { .. } => 5,
    };

    let mut ordered: Vec<&KnowledgeAction> = actions.iter().collect();
    ordered.sort_by_key(|action| rank(action));
    ordered
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    use super::*;
    use crate::knowledge2::action::Cause;
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
            .proc(&[KnowledgeAction::CreateEntity {
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
                KnowledgeAction::CreateEntity {
                    from: ObservationId(first),
                    with: None,
                },
                KnowledgeAction::AttachObservation {
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
                KnowledgeAction::CreateEntity {
                    from: ObservationId(left),
                    with: None,
                },
                KnowledgeAction::CreateEntity {
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
            .proc(&[KnowledgeAction::MergeEntity {
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
}
