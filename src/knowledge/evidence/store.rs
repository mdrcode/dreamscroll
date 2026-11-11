//! Where a claim becomes a row, and every way it changes.
//!
//! An observation's claim and a provider's claim are the same edge: one insert into
//! `knowledge_claims` citing an identity row. Keeping every change to that table here — insert,
//! reassign on a fold, retract on a veto — means the veto check, the uniqueness rule and the basis
//! column are one thing to read, and the place a second authority inherits all three.

use std::collections::HashSet;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement};

use super::claim::Claim;
use super::identity::{IdentityDraft, IdentityKey};
use super::selection::ClaimSelection;
use crate::knowledge::scoring::TotalCmp;
use crate::model;

/// The unique index that makes one entity's hold on one identity a single live claim. Both writers
/// ask the same question, so the name is here once rather than in two match arms.
const CLAIMS_UNIQUE: &str = "idx-knowledge_claims_entity_identity_provider";

/// What became of a claim write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The edge was written.
    Written { identity_id: i32, claim_id: i32 },
    /// An editor withdrew this claim.
    Vetoed { identity_id: i32 },
    /// This entity already claims the identity. A repeat is not a conflict: whether both holders are
    /// one thing is `proximity`'s question.
    AlreadyClaimed { identity_id: i32, claim_id: i32 },
}

/// Claims and the identities they cite.
///
/// Holds its own clone of the connection rather than borrowing one: `DatabaseConnection` is a pool
/// handle behind an `Arc`, so cloning is a pointer copy, and the type can be built once and kept in
/// a long-lived state — which is the point of it being a parameter.
#[derive(Clone)]
pub struct Evidence {
    conn: sea_orm::DatabaseConnection,
}

impl Evidence {
    pub fn from_connection(conn: sea_orm::DatabaseConnection) -> Self {
        Self { conn }
    }

    pub fn from_handle(db: &crate::database::DbHandle) -> Self {
        Self::from_connection(db.conn.clone())
    }

    /// Candidate selection over the same pool. Reads only; kept out of the write surface.
    pub(crate) fn selection(&self) -> ClaimSelection<'_> {
        ClaimSelection::from_connection(&self.conn)
    }

    /// The identity's row, created if this is the first time we have seen it.
    ///
    /// Upserts by filling gaps rather than overwriting: a handle first read off an observation has no
    /// label, and the first authority that answers for it supplies one. Nothing here clears a field
    /// a previous answer set — an identity is a running record, not a latest-value cache. An
    /// observation answer never downgrades `method` or `provider`, so one upsert serves both sources.
    pub async fn upsert_identity(&self, identity: &IdentityDraft) -> anyhow::Result<i32> {
        // Bind order matches the INSERT column list.
        let values = vec![
            sea_orm::Value::String(Some(identity.scheme.clone())),
            sea_orm::Value::String(Some(identity.external_id.clone())),
            sea_orm::Value::String(identity.label.clone()),
            sea_orm::Value::String(identity.description.clone()),
            // `classes` is an array column; bind it as one.
            identity.classes.clone().into(),
            sea_orm::Value::String(Some(identity.provider.clone())),
            sea_orm::Value::String(identity.method.clone()),
            sea_orm::Value::Float(identity.confidence),
        ];
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO knowledge_identities \
                   (scheme, external_id, label, description, classes, provider, method, confidence) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (scheme, external_id) DO UPDATE SET \
                   label = COALESCE(EXCLUDED.label, knowledge_identities.label), \
                   description = COALESCE(EXCLUDED.description, knowledge_identities.description), \
                   classes = CASE WHEN cardinality(EXCLUDED.classes) > 0 \
                                  THEN EXCLUDED.classes ELSE knowledge_identities.classes END, \
                   method = CASE WHEN EXCLUDED.method = 'observation' \
                                 THEN knowledge_identities.method \
                                 ELSE COALESCE(EXCLUDED.method, knowledge_identities.method) END, \
                   confidence = COALESCE(EXCLUDED.confidence, knowledge_identities.confidence), \
                   provider = CASE WHEN EXCLUDED.method IS DISTINCT FROM 'observation' \
                                 THEN EXCLUDED.provider ELSE knowledge_identities.provider END \
                 RETURNING id",
                values,
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("identity insert returned no row"))?;
        Ok(row.try_get_by_index::<i32>(0)?)
    }

    /// The identity row for this durable key, if it exists. Reads only; creating is the caller's job.
    pub(crate) async fn find_identity_id(&self, key: &IdentityKey) -> anyhow::Result<Option<i32>> {
        let row = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::Scheme.eq(&key.scheme))
            .filter(model::knowledge_identity::Column::ExternalId.eq(&key.external_id))
            .one(&self.conn)
            .await?;
        Ok(row.map(|row| row.id))
    }

    /// The entities holding this identity, most substantiated first: the scan keeps the
    /// first and asks about the rest, and the linker attaches to the first.
    ///
    /// The ordering is [`TotalCmp`], asked here rather than written into the SQL: `merge` picks its
    /// survivor with the same function, and two spellings of "most substantiated" disagreed on
    /// `NULL` for as long as they existed.
    pub async fn holders_of_identity(
        &self,
        identity_id: i32,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        use sea_orm::sea_query::{Expr, ExprTrait, Query};
        use sea_orm::{ColumnTrait, QueryFilter};

        // The holders, as a subquery: a live claim on this identity, from a live entity.
        let claimants = Query::select()
            .distinct()
            .column((
                model::knowledge_claims::Entity,
                model::knowledge_claims::Column::KnowledgeEntityId,
            ))
            .from(model::knowledge_claims::Entity)
            .and_where(
                Expr::col((
                    model::knowledge_claims::Entity,
                    model::knowledge_claims::Column::KnowledgeIdentityId,
                ))
                .eq(identity_id),
            )
            .and_where(
                Expr::col((
                    model::knowledge_claims::Entity,
                    model::knowledge_claims::Column::RetractedAt,
                ))
                .is_null(),
            )
            .to_owned();

        let mut entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .filter(Expr::col(model::knowledge_entity::Column::Id).in_subquery(claimants))
            .all(&self.conn)
            .await?;
        entities.sort_by(|a, b| b.confidence.total_cmp(&a.confidence).then(a.id.cmp(&b.id)));
        Ok(entities)
    }

    /// Settle an identity and write the claim edge in one operation.
    ///
    /// The two rows remain separate: an identity is shared and durable, a claim is this entity's
    /// edge to it. The claim already names its holder and identity; this asserts they agree with the
    /// identity being settled.
    pub async fn claim_identity(
        &self,
        identity_draft: IdentityDraft,
        claim: Claim,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<ClaimOutcome> {
        anyhow::ensure!(
            claim.identity == identity_draft.key(),
            "claim identity `{}:{}` does not match the settled identity",
            claim.identity.scheme,
            claim.identity.external_id
        );
        let identity_id = self.upsert_identity(&identity_draft).await?;
        self.write_claim(identity_id, claim, vetoes).await
    }

    /// Write one edge. The identity id must already be resolved.
    ///
    /// The veto is asked here, at the one place a claim becomes a row, so every writer inherits it.
    /// Idempotent per entity: claiming an identity twice is not a duplicate, and another entity
    /// holding it is not an error either — claims are many-to-one, and whether the holders are one
    /// thing is `proximity`'s question, not this call's.
    async fn write_claim(
        &self,
        identity_id: i32,
        claim: Claim,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<ClaimOutcome> {
        if vetoes.contains(&(
            claim.holder,
            claim.identity.scheme.clone(),
            claim.identity.external_id.clone(),
        )) {
            return Ok(ClaimOutcome::Vetoed { identity_id });
        }
        let insert =
            model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
                knowledge_entity_id: sea_orm::Set(claim.holder),
                knowledge_identity_id: sea_orm::Set(identity_id),
                basis: sea_orm::Set(claim.basis),
                provider: sea_orm::Set(claim.provider.to_string()),
                method: sea_orm::Set(Some(claim.method.clone())),
                user_observed_entity_id: sea_orm::Set(claim.observation_id),
                ..Default::default()
            })
            .exec_with_returning(&self.conn)
            .await;

        match insert {
            Ok(row) => Ok(ClaimOutcome::Written {
                identity_id,
                claim_id: row.id,
            }),
            Err(error) => match unique_violation(&error) {
                Some(CLAIMS_UNIQUE) => {
                    let claim_id = self
                        .find_claim_id(claim.holder, identity_id, claim.provider)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("unique violation without a live claim"))?;
                    Ok(ClaimOutcome::AlreadyClaimed {
                        identity_id,
                        claim_id,
                    })
                }
                _ => Err(error.into()),
            },
        }
    }

    /// The live claim id for one `(holder, identity, provider)`, after the unique index refused a
    /// second insert.
    async fn find_claim_id(
        &self,
        holder: i32,
        identity_id: i32,
        provider: &str,
    ) -> anyhow::Result<Option<i32>> {
        let row = model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(holder))
            .filter(model::knowledge_claims::Column::KnowledgeIdentityId.eq(identity_id))
            .filter(model::knowledge_claims::Column::Provider.eq(provider))
            .filter(model::knowledge_claims::Column::RetractedAt.is_null())
            .one(&self.conn)
            .await?;
        Ok(row.map(|row| row.id))
    }

    /// Re-point every claim from `loser` to `winner`, dropping the duplicates first.
    ///
    /// A fold runs this on its transaction, so the claims move with the observations and the
    /// tombstone. Evidence that duplicates the winner's would violate the identity index on the way
    /// over, so it is deleted rather than moved: it is the same fact stated twice, and the winner's
    /// copy is the one kept.
    pub(crate) async fn move_claims<C: ConnectionTrait>(
        &self,
        conn: &C,
        loser: i32,
        winner: i32,
    ) -> anyhow::Result<()> {
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM knowledge_claims l USING knowledge_claims w \
             WHERE l.knowledge_entity_id = $1 AND w.knowledge_entity_id = $2 \
               AND w.retracted_at IS NULL \
               AND l.knowledge_identity_id = w.knowledge_identity_id \
               AND l.provider = w.provider",
            [loser.into(), winner.into()],
        ))
        .await?;
        model::knowledge_claims::Entity::update_many()
            .col_expr(
                model::knowledge_claims::Column::KnowledgeEntityId,
                Expr::value(Some(winner)),
            )
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(loser))
            .exec(conn)
            .await?;
        Ok(())
    }

    /// Retract this provider's live claims on `entity` that the run did not keep.
    ///
    /// `keep` is the exact set the provider got accepted this run. Its schemes are the schemes the
    /// provider answered: absence from a scheme it never asked is not evidence.
    pub(crate) async fn retract_unproposed(
        &self,
        entity_id: i32,
        provider: &str,
        keep: &HashSet<(String, String)>,
    ) -> anyhow::Result<usize> {
        let schemes: HashSet<&str> = keep.iter().map(|(scheme, _)| scheme.as_str()).collect();
        if schemes.is_empty() {
            return Ok(0);
        }
        let rows = model::knowledge_claims::Entity::find()
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity_id))
            .filter(model::knowledge_claims::Column::Provider.eq(provider))
            .filter(model::knowledge_claims::Column::RetractedAt.is_null())
            .find_with_related(model::knowledge_identity::Entity)
            .all(&self.conn)
            .await?;
        let mut retracted = 0;
        for (claim, identities) in rows {
            let Some(identity) = identities.into_iter().next() else {
                continue;
            };
            if !schemes.contains(identity.scheme.as_str())
                || keep.contains(&(identity.scheme.clone(), identity.external_id.clone()))
            {
                continue;
            }
            model::knowledge_claims::Entity::update_many()
                .col_expr(
                    model::knowledge_claims::Column::RetractedAt,
                    Expr::current_timestamp(),
                )
                .filter(model::knowledge_claims::Column::Id.eq(claim.id))
                .exec(&self.conn)
                .await?;
            retracted += 1;
        }
        Ok(retracted)
    }

    /// Retract the live claim for `(entity, identity)`, if there is one.
    ///
    /// A veto names a pairing, so the retraction does too: there is at most one live claim for
    /// `(entity, identity)`, whatever route produced it. Claims are append-only, so this marks the
    /// row rather than deleting it — the caller reprojects the entity, because `status` and
    /// `confidence` are functions of the live claims.
    pub(crate) async fn retract(
        &self,
        knowledge_entity_id: i32,
        identity_id: i32,
    ) -> anyhow::Result<usize> {
        Ok(model::knowledge_claims::Entity::update_many()
            .col_expr(
                model::knowledge_claims::Column::RetractedAt,
                Expr::current_timestamp(),
            )
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(knowledge_entity_id))
            .filter(model::knowledge_claims::Column::KnowledgeIdentityId.eq(identity_id))
            .filter(model::knowledge_claims::Column::RetractedAt.is_null())
            .exec(&self.conn)
            .await?
            .rows_affected as usize)
    }
}

/// The constraint a unique violation broke, if it was one. Postgres reports `23505`.
fn unique_violation(error: &sea_orm::DbErr) -> Option<&str> {
    use sea_orm::RuntimeErr;
    match error {
        sea_orm::DbErr::Exec(RuntimeErr::SqlxError(error))
        | sea_orm::DbErr::Query(RuntimeErr::SqlxError(error)) => error
            .as_database_error()
            .filter(|error| error.code().as_deref() == Some("23505"))
            .and_then(|error| error.constraint()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::PaginatorTrait;

    use super::*;
    use crate::knowledge::evidence::platform_link;
    use crate::test_support::{db_or_skip, fixtures};

    /// Two entities may hold one identity — that is what the scan asks about — while each holds it
    /// once. The second claim write is the unique index's path: a repeat is a no-op, not an error.
    #[tokio::test]
    async fn one_identity_many_holders_one_claim_each() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "evidence_holders").await;
        let observation = fixtures::observe(
            &db,
            &capture,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        let first = fixtures::knowledge_entity_with_member(&db, "Tsoding").await;
        let second = fixtures::knowledge_entity_with_member(&db, "Alexey Kutepov").await;

        let evidence = Evidence::from_handle(&db);
        let observation = model::user_observed_entity::Entity::find_by_id(observation)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let link = platform_link(&observation).expect("a platform link");
        let identity = IdentityDraft::try_from(&link).expect("a handle");
        let key = identity.key();

        let vetoes = HashSet::new();
        let first_claim = evidence
            .claim_identity(
                identity.clone(),
                Claim::from_observation(first, &observation, key.clone()),
                &vetoes,
            )
            .await
            .unwrap();
        let (identity_id, claim_id) = match first_claim {
            ClaimOutcome::Written {
                identity_id,
                claim_id,
            } => (identity_id, claim_id),
            other => panic!("expected Written, got {other:?}"),
        };

        let repeat = evidence
            .claim_identity(
                identity.clone(),
                Claim::from_observation(first, &observation, key.clone()),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(
            repeat,
            ClaimOutcome::AlreadyClaimed {
                identity_id,
                claim_id
            },
            "the same holder again is one claim, not a violation"
        );

        let second_claim = evidence
            .claim_identity(
                identity,
                Claim::from_observation(second, &observation, key),
                &vetoes,
            )
            .await
            .unwrap();
        assert!(
            matches!(second_claim, ClaimOutcome::Written { .. }),
            "a second holder is not an error: {second_claim:?}"
        );

        assert_eq!(
            evidence
                .holders_of_identity(identity_id)
                .await
                .unwrap()
                .len(),
            2,
            "both entities hold it, and the scan decides whether they are one thing"
        );
        assert_eq!(
            crate::model::knowledge_claims::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            2,
            "one claim per holder"
        );
    }

    /// A capture may create an identity; it may never overwrite one an answer established.
    #[tokio::test]
    async fn ingesting_a_capture_does_not_downgrade_an_answered_identity() {
        use sea_orm::{ColumnTrait, QueryFilter};

        db_or_skip!(db);
        let evidence = Evidence::from_handle(&db);
        evidence
            .upsert_identity(&IdentityDraft {
                scheme: "x_twitter".to_string(),
                external_id: "tsoding".to_string(),
                provider: "wikidata".to_string(),
                method: Some("P2002".to_string()),
                confidence: Some(0.9),
                label: Some("Alexey Kutepov".to_string()),
                description: None,
                classes: Vec::new(),
            })
            .await
            .expect("an answer creates the identity");

        let capture = fixtures::Capture::new(&db, "ingest_precedence").await;
        let observation = fixtures::observe(
            &db,
            &capture,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x_twitter"),
            Some("tsoding"),
        )
        .await;
        let observation = model::user_observed_entity::Entity::find_by_id(observation)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let link = platform_link(&observation).expect("a platform link");
        let identity = IdentityDraft::try_from(&link).expect("a handle");
        evidence.upsert_identity(&identity).await.unwrap();

        let row = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::Scheme.eq("x_twitter"))
            .filter(model::knowledge_identity::Column::ExternalId.eq("tsoding"))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.method.as_deref(),
            Some("P2002"),
            "not downgraded to capture"
        );
        assert_eq!(row.provider, "wikidata");
        assert_eq!(row.label.as_deref(), Some("Alexey Kutepov"));
    }

    /// An authority's answer settles the identity and returns the claim id; a repeat returns the same
    /// id instead of writing a second edge.
    #[tokio::test]
    async fn an_answer_claim_returns_the_written_and_existing_id() {
        db_or_skip!(db);
        let entity = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let evidence = Evidence::from_handle(&db);
        let identity = IdentityDraft {
            scheme: "wikidata".to_string(),
            external_id: "Q_CLAIM_ID".to_string(),
            provider: "wikidata".to_string(),
            method: Some("exact_label".to_string()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: Vec::new(),
        };
        let vetoes = HashSet::new();
        let key = identity.key();

        let written = evidence
            .claim_identity(
                identity.clone(),
                Claim::from_provider(
                    entity,
                    key.clone(),
                    crate::model::knowledge_claims::ClaimBasis::Text,
                    "wikidata",
                    "exact_label".to_string(),
                ),
                &vetoes,
            )
            .await
            .unwrap();
        let (identity_id, claim_id) = match written {
            ClaimOutcome::Written {
                identity_id,
                claim_id,
            } => (identity_id, claim_id),
            other => panic!("expected Written, got {other:?}"),
        };

        let repeat = evidence
            .claim_identity(
                identity,
                Claim::from_provider(
                    entity,
                    key,
                    crate::model::knowledge_claims::ClaimBasis::Text,
                    "wikidata",
                    "exact_label".to_string(),
                ),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(
            repeat,
            ClaimOutcome::AlreadyClaimed {
                identity_id,
                claim_id
            }
        );
    }

    /// The provider follows the answer, not merely the presence of a label: a model guess with no
    /// label is replaced by the authority that actually answered.
    #[tokio::test]
    async fn an_answer_owns_the_provider_even_without_a_label() {
        db_or_skip!(db);
        let evidence = Evidence::from_handle(&db);
        let id = evidence
            .upsert_identity(&IdentityDraft {
                scheme: "wikidata".to_string(),
                external_id: "Q_PROVIDER_TEST".to_string(),
                provider: "model".to_string(),
                method: Some("guess".to_string()),
                confidence: None,
                label: None,
                description: Some("a guess".to_string()),
                classes: Vec::new(),
            })
            .await
            .expect("a guess creates the identity");
        evidence
            .upsert_identity(&IdentityDraft {
                scheme: "wikidata".to_string(),
                external_id: "Q_PROVIDER_TEST".to_string(),
                provider: "wikidata".to_string(),
                method: Some("exact_label".to_string()),
                confidence: Some(0.8),
                label: None,
                description: None,
                classes: Vec::new(),
            })
            .await
            .expect("the answer enriches it");

        let row = model::knowledge_identity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.provider, "wikidata");
        assert_eq!(row.description.as_deref(), Some("a guess"));
    }

    /// `classes` is an array column, and the round trip has to survive more than one element: every
    /// other test supplies none, which is exactly how an encoding bug hides.
    #[tokio::test]
    async fn identity_classes_round_trip_and_a_later_answer_does_not_erase_them() {
        db_or_skip!(db);
        let evidence = Evidence::from_handle(&db);
        let draft = IdentityDraft {
            scheme: "wikidata".to_string(),
            external_id: "Q42".to_string(),
            provider: "wikidata".to_string(),
            method: Some("exact_label".to_string()),
            confidence: Some(0.9),
            label: Some("Douglas Adams".to_string()),
            description: None,
            classes: vec!["Q5".to_string(), "Q36180".to_string()],
        };
        let id = evidence.upsert_identity(&draft).await.unwrap();

        let row = model::knowledge_identity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.classes,
            vec!["Q5".to_string(), "Q36180".to_string()],
            "two classes, not one joined string"
        );

        // A later answer that carried no classes leaves the accumulated ones alone.
        evidence
            .upsert_identity(&IdentityDraft {
                classes: Vec::new(),
                label: None,
                method: None,
                confidence: None,
                ..draft
            })
            .await
            .unwrap();
        let row = model::knowledge_identity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.classes, vec!["Q5".to_string(), "Q36180".to_string()]);
        assert_eq!(row.label.as_deref(), Some("Douglas Adams"));
    }
}
