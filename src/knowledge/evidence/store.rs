//! Where a claim becomes a row, and every way it changes.
//!
//! A capture's claim and an authority's claim are the same edge: one insert into `knowledge_claims`
//! citing an identity row. Keeping every change to that table here — insert, reassign on a fold,
//! retract on a veto — means the veto check, the uniqueness rule and the basis column are one thing
//! to read, and the place a second authority inherits all three.

use std::collections::HashSet;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement};

use super::claim::ClaimDraft;
use super::identity::{IdentityDraft, IdentityKey};
use super::selection::ClaimSelection;
use crate::knowledge::scoring::TotalCmp;
use crate::model;

/// The unique index that makes one entity's hold on one identity a single live claim. Both writers
/// ask the same question, so the name is here once rather than in two match arms.
const CLAIMS_UNIQUE: &str = "idx-knowledge_claims_entity_identity_provider";

/// What one claim write returned.
///
/// `claim_id` is present when the row was written or already existed; it is absent when a veto
/// refused the edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimWrite {
    pub identity_id: i32,
    pub claim_id: Option<i32>,
    pub outcome: ClaimOutcome,
}

/// What became of a claim write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The edge was written.
    Written,
    /// An editor withdrew this claim.
    Vetoed,
    /// This entity already claims the identity. A repeat is not a conflict: whether both holders are
    /// one thing is `proximity`'s question.
    AlreadyClaimed,
}

/// How an identity row enters the store: create-only or fill-gaps.
///
/// The choice is the caller's; the store only knows the two storage behaviours.
#[derive(Debug, Clone, Copy)]
pub enum IdentitySettlement<'a> {
    CreateOnly(&'a IdentityDraft),
    Enrich(&'a IdentityDraft),
}

impl IdentitySettlement<'_> {
    /// The durable key this settlement names.
    pub fn key(&self) -> IdentityKey {
        match self {
            Self::CreateOnly(draft) | Self::Enrich(draft) => draft.key(),
        }
    }
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
    pub fn new(conn: sea_orm::DatabaseConnection) -> Self {
        Self { conn }
    }

    pub fn from_handle(db: &crate::database::DbHandle) -> Self {
        Self::new(db.conn.clone())
    }

    /// Candidate selection over the same pool. Reads only; kept out of the write surface.
    pub(crate) fn selection(&self) -> ClaimSelection<'_> {
        ClaimSelection::new(&self.conn)
    }

    /// The identity's row, created if this is the first time we have seen it.
    ///
    /// Upserts by filling gaps rather than overwriting: a handle first read off a capture has no
    /// label, and the first authority that answers for it supplies one. Nothing here clears a field
    /// a previous answer set — an identity is a running record, not a latest-value cache.
    pub async fn enrich_identity(&self, identity: &IdentityDraft) -> anyhow::Result<i32> {
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
                   method = COALESCE(EXCLUDED.method, knowledge_identities.method), \
                   confidence = COALESCE(EXCLUDED.confidence, knowledge_identities.confidence), \
                   provider = CASE WHEN EXCLUDED.method IS DISTINCT FROM 'capture' \
                                 THEN EXCLUDED.provider ELSE knowledge_identities.provider END \
                 RETURNING id",
                identity_values(identity),
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("identity insert returned no row"))?;
        Ok(row.try_get_by_index::<i32>(0)?)
    }

    /// The identity row for this draft, created if this is the first sighting.
    ///
    /// Create-only: an existing row is left exactly as it is. Idempotent, so a re-link is free. The
    /// no-op `DO UPDATE` is what lets one statement return the id whether the row was inserted or
    /// already present.
    pub async fn ensure_identity(&self, identity: &IdentityDraft) -> anyhow::Result<i32> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO knowledge_identities \
                   (scheme, external_id, label, description, classes, provider, method, confidence) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (scheme, external_id) DO UPDATE SET scheme = EXCLUDED.scheme \
                 RETURNING id",
                identity_values(identity),
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
    pub async fn held_by_identity(
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
    /// edge to it. What callers no longer do is orchestrate the pair themselves.
    pub async fn claim_identity(
        &self,
        holder: i32,
        identity: IdentitySettlement<'_>,
        claim: ClaimDraft<'_>,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<ClaimWrite> {
        let key = identity.key();
        let identity_id = match identity {
            IdentitySettlement::CreateOnly(draft) => self.ensure_identity(draft).await?,
            IdentitySettlement::Enrich(draft) => self.enrich_identity(draft).await?,
        };
        self.write_claim(holder, identity_id, &key, claim, vetoes)
            .await
    }

    /// Write one edge: `holder` claims `identity`. The identity id must already be resolved.
    ///
    /// The veto is asked here, at the one place a claim becomes a row, so every writer inherits it.
    /// Idempotent per entity: claiming an identity twice is not a duplicate, and another entity
    /// holding it is not an error either — claims are many-to-one, and whether the holders are one
    /// thing is `proximity`'s question, not this call's.
    async fn write_claim(
        &self,
        holder: i32,
        identity_id: i32,
        identity: &IdentityKey,
        draft: ClaimDraft<'_>,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<ClaimWrite> {
        if vetoes.contains(&(
            holder,
            identity.scheme.clone(),
            identity.external_id.clone(),
        )) {
            return Ok(ClaimWrite {
                identity_id,
                claim_id: None,
                outcome: ClaimOutcome::Vetoed,
            });
        }
        let insert =
            model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
                knowledge_entity_id: sea_orm::Set(holder),
                knowledge_identity_id: sea_orm::Set(identity_id),
                basis: sea_orm::Set(draft.basis),
                provider: sea_orm::Set(draft.provider.to_string()),
                method: sea_orm::Set(Some(draft.method.clone())),
                surface: sea_orm::Set(draft.capture.and_then(|row| row.platform_handle.clone())),
                user_observed_entity_id: sea_orm::Set(draft.capture.map(|row| row.id)),
                source_user_id: sea_orm::Set(draft.capture.map(|row| row.user_id)),
                ..Default::default()
            })
            .exec_with_returning(&self.conn)
            .await;

        match insert {
            Ok(row) => Ok(ClaimWrite {
                identity_id,
                claim_id: Some(row.id),
                outcome: ClaimOutcome::Written,
            }),
            Err(error) => match unique_violation(&error) {
                Some(CLAIMS_UNIQUE) => {
                    let claim_id = self
                        .find_claim_id(holder, identity_id, draft.provider)
                        .await?;
                    Ok(ClaimWrite {
                        identity_id,
                        claim_id,
                        outcome: ClaimOutcome::AlreadyClaimed,
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

/// The binds an identity insert or upsert needs, in column order.
fn identity_values(identity: &IdentityDraft) -> Vec<sea_orm::Value> {
    vec![
        sea_orm::Value::String(Some(identity.scheme.clone())),
        sea_orm::Value::String(Some(identity.external_id.clone())),
        sea_orm::Value::String(identity.label.clone()),
        sea_orm::Value::String(identity.description.clone()),
        // `classes` is an array column; bind it as one, the way the blocking index binds its
        // key arrays.
        identity.classes.clone().into(),
        sea_orm::Value::String(Some(identity.provider.clone())),
        sea_orm::Value::String(identity.method.clone()),
        sea_orm::Value::Float(identity.confidence),
    ]
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

        let vetoes = HashSet::new();
        let first_claim = evidence
            .claim_identity(
                first,
                IdentitySettlement::CreateOnly(&identity),
                ClaimDraft::capture(&observation),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(first_claim.outcome, ClaimOutcome::Written);
        assert!(first_claim.claim_id.is_some());

        let repeat = evidence
            .claim_identity(
                first,
                IdentitySettlement::CreateOnly(&identity),
                ClaimDraft::capture(&observation),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(
            repeat.outcome,
            ClaimOutcome::AlreadyClaimed,
            "the same holder again is one claim, not a violation"
        );
        assert_eq!(repeat.claim_id, first_claim.claim_id);

        let second_claim = evidence
            .claim_identity(
                second,
                IdentitySettlement::CreateOnly(&identity),
                ClaimDraft::capture(&observation),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(
            second_claim.outcome,
            ClaimOutcome::Written,
            "a second holder is not an error"
        );

        assert_eq!(
            evidence
                .held_by_identity(first_claim.identity_id)
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
            .enrich_identity(&IdentityDraft {
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
        evidence.ensure_identity(&identity).await.unwrap();

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

        let written = evidence
            .claim_identity(
                entity,
                IdentitySettlement::Enrich(&identity),
                ClaimDraft::authority(
                    crate::model::knowledge_claims::ClaimBasis::Text,
                    "wikidata",
                    "exact_label".to_string(),
                ),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(written.outcome, ClaimOutcome::Written);
        assert!(written.claim_id.is_some());

        let repeat = evidence
            .claim_identity(
                entity,
                IdentitySettlement::Enrich(&identity),
                ClaimDraft::authority(
                    crate::model::knowledge_claims::ClaimBasis::Text,
                    "wikidata",
                    "exact_label".to_string(),
                ),
                &vetoes,
            )
            .await
            .unwrap();
        assert_eq!(repeat.outcome, ClaimOutcome::AlreadyClaimed);
        assert_eq!(repeat.claim_id, written.claim_id);
    }

    /// The provider follows the answer, not merely the presence of a label: a model guess with no
    /// label is replaced by the authority that actually answered.
    #[tokio::test]
    async fn an_answer_owns_the_provider_even_without_a_label() {
        db_or_skip!(db);
        let evidence = Evidence::from_handle(&db);
        let id = evidence
            .enrich_identity(&IdentityDraft {
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
            .enrich_identity(&IdentityDraft {
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
        let id = evidence.enrich_identity(&draft).await.unwrap();

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
            .enrich_identity(&IdentityDraft {
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
