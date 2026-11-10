//! Where a claim becomes a row, and every way it changes.
//!
//! A capture's claim and an authority's claim are the same edge: one insert into `knowledge_claims`
//! citing an identity row. Keeping every change to that table here — insert, reassign on a fold,
//! retract on a veto — means the veto check, the uniqueness rule and the basis column are one thing
//! to read, and the place a second authority inherits all three.

use std::collections::HashSet;

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};

use super::claim::{ClaimDraft, Stored};
use super::identity::{DraftIdentity, IdentityKey};
use crate::knowledge::scoring::TotalCmp;
use crate::model;

/// The unique index that makes one entity's hold on one identity a single live claim. Both writers
/// ask the same question, so the name is here once rather than in two match arms.
const CLAIMS_UNIQUE: &str = "idx-knowledge_claims_entity_identity";

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

    /// The identity's row, created if this is the first time we have seen it.
    ///
    /// Upserts by filling gaps rather than overwriting: a handle first read off a capture has no
    /// label, and the first authority that answers for it supplies one. Nothing here clears a field
    /// a previous answer set — an identity is a running record, not a latest-value cache.
    pub async fn upsert_identity(&self, identity: &DraftIdentity) -> anyhow::Result<i32> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO knowledge_identities \
                   (scheme, external_id, label, description, classes, asserted_by, method, confidence) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (scheme, external_id) DO UPDATE SET \
                   label = COALESCE(EXCLUDED.label, knowledge_identities.label), \
                   description = COALESCE(EXCLUDED.description, knowledge_identities.description), \
                   classes = CASE WHEN cardinality(EXCLUDED.classes) > 0 \
                                  THEN EXCLUDED.classes ELSE knowledge_identities.classes END, \
                   method = COALESCE(EXCLUDED.method, knowledge_identities.method), \
                   confidence = COALESCE(EXCLUDED.confidence, knowledge_identities.confidence), \
                   asserted_by = CASE WHEN EXCLUDED.label IS NOT NULL \
                                 THEN EXCLUDED.asserted_by ELSE knowledge_identities.asserted_by END \
                 RETURNING id",
                identity_values(identity),
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("identity insert returned no row"))?;
        Ok(row.try_get_by_index::<i32>(0)?)
    }

    /// Create the identity if it is new; leave an existing row exactly as it is.
    ///
    /// The no-op `DO UPDATE` is what lets one statement return the id either way. This is the only
    /// writer a capture uses: a screenshot knows a handle exists, not what the thing is.
    async fn create_identity(&self, identity: &DraftIdentity) -> anyhow::Result<i32> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO knowledge_identities \
                   (scheme, external_id, label, description, classes, asserted_by, method, confidence) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (scheme, external_id) DO UPDATE SET scheme = EXCLUDED.scheme \
                 RETURNING id",
                identity_values(identity),
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("identity insert returned no row"))?;
        Ok(row.try_get_by_index::<i32>(0)?)
    }

    /// The identity this capture link asserts, created if this is the first sighting.
    ///
    /// A capture may create an identity, never enrich one: it knows a handle exists, not what the
    /// thing is. Idempotent, so a re-link is free.
    pub async fn identity_for_link(&self, key: &IdentityKey) -> anyhow::Result<i32> {
        self.create_identity(&DraftIdentity::from_capture(key))
            .await
    }

    /// Entities no configured authority has claimed yet, in the order a run should ask them.
    ///
    /// The mirror of [`Self::held_by_identity`]: that asks who holds an identity, this asks which
    /// entities still have none. The filter is *authority*, not *any evidence* — an entity identified
    /// only by a platform handle is still a candidate, because a QID is what corroboration needs.
    ///
    /// Ordering and limit are the caller's policy, passed in: ascending is a rebuild, descending is
    /// "what just arrived", and a finite budget has to reach one of them deterministically.
    pub async fn awaiting_authority(
        &self,
        sources: &[&str],
        limit: Option<usize>,
        only: Option<i32>,
        newest_first: bool,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        use model::knowledge_entity as entity;
        use sea_orm::sea_query::{Expr, ExprTrait, Query};

        // Asked already: holds a claim an authority made, rather than only the capture's handle.
        let claimed = Query::select()
            .distinct()
            .column((
                model::knowledge_claims::Entity,
                model::knowledge_claims::Column::KnowledgeEntityId,
            ))
            .from(model::knowledge_claims::Entity)
            .inner_join(
                model::knowledge_identity::Entity,
                Expr::col((
                    model::knowledge_claims::Entity,
                    model::knowledge_claims::Column::KnowledgeIdentityId,
                ))
                .equals((
                    model::knowledge_identity::Entity,
                    model::knowledge_identity::Column::Id,
                )),
            )
            .and_where(
                Expr::col((
                    model::knowledge_claims::Entity,
                    model::knowledge_claims::Column::RetractedAt,
                ))
                .is_null(),
            )
            .and_where(
                Expr::col((
                    model::knowledge_identity::Entity,
                    model::knowledge_identity::Column::AssertedBy,
                ))
                .is_in(
                    sources
                        .iter()
                        .map(|source| source.to_string())
                        .collect::<Vec<_>>(),
                ),
            )
            .to_owned();

        let mut query = entity::Entity::find()
            .filter(entity::Column::MergedIntoId.is_null())
            .filter(Expr::col(entity::Column::Id).not_in_subquery(claimed))
            .order_by(
                entity::Column::Id,
                if newest_first {
                    sea_orm::Order::Desc
                } else {
                    sea_orm::Order::Asc
                },
            );
        if let Some(knowledge_entity_id) = only {
            query = query.filter(entity::Column::Id.eq(knowledge_entity_id));
        }
        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }
        Ok(query.all(&self.conn).await?)
    }

    /// The identity this claim cites, if a row exists. Reads only; importing is the caller's job.
    pub(crate) async fn identity_id(
        &self,
        scheme: &str,
        external_id: &str,
    ) -> anyhow::Result<Option<i32>> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT id FROM knowledge_identities WHERE scheme = $1 AND external_id = $2",
                [scheme.into(), external_id.into()],
            ))
            .await?;
        Ok(match row {
            Some(row) => Some(row.try_get_by_index::<i32>(0)?),
            None => None,
        })
    }

    /// The identity this observation asserts, if it carries one and it has been imported.
    pub async fn identity_of(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<Option<i32>> {
        match IdentityKey::from_platform_link(observation) {
            Some(key) => self.identity_id(&key.scheme, &key.external_id).await,
            None => Ok(None),
        }
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

    /// Write one edge: `holder` asserts `identity`. Phase two, shared by both callers.
    ///
    /// The veto is asked here, at the one place a claim becomes a row, so every writer inherits it.
    /// Idempotent per entity: asserting an identity twice is not a duplicate, and another entity
    /// holding it is not an error either — claims are many-to-one, and whether the holders are one
    /// thing is `proximity`'s question, not this call's.
    pub async fn assert(
        &self,
        draft: ClaimDraft<'_>,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<Stored> {
        if vetoes.contains(&(
            draft.holder,
            draft.identity.scheme.clone(),
            draft.identity.external_id.clone(),
        )) {
            return Ok(Stored::Vetoed);
        }
        let insert =
            model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
                knowledge_entity_id: sea_orm::Set(draft.holder),
                knowledge_identity_id: sea_orm::Set(draft.identity_id),
                basis: sea_orm::Set(draft.basis),
                surface: sea_orm::Set(draft.capture.and_then(|row| row.platform_handle.clone())),
                user_observed_entity_id: sea_orm::Set(draft.capture.map(|row| row.id)),
                source_user_id: sea_orm::Set(draft.capture.map(|row| row.user_id)),
                ..Default::default()
            })
            .exec(&self.conn)
            .await;

        match insert {
            Ok(_) => Ok(Stored::Written),
            Err(error) => match unique_violation(&error) {
                Some(CLAIMS_UNIQUE) => Ok(Stored::AlreadyClaimed),
                _ => Err(error.into()),
            },
        }
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
               AND l.knowledge_identity_id = w.knowledge_identity_id",
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
fn identity_values(identity: &DraftIdentity) -> Vec<sea_orm::Value> {
    vec![
        sea_orm::Value::String(Some(identity.scheme.clone())),
        sea_orm::Value::String(Some(identity.external_id.clone())),
        sea_orm::Value::String(identity.label.clone()),
        sea_orm::Value::String(identity.description.clone()),
        // `classes` is an array column; bind it as one, the way the blocking index binds its
        // key arrays.
        identity.classes.clone().into(),
        sea_orm::Value::String(Some(identity.asserted_by.clone())),
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
    use crate::test_support::{db_or_skip, fixtures};

    /// Two entities may hold one identity — that is what the scan asks about — while each holds it
    /// once. The second assertion is the unique index's path: a repeat is a no-op, not an error.
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
        let key = IdentityKey::from_platform_link(&observation).expect("a handle");
        let identity = evidence
            .identity_for_link(&key)
            .await
            .expect("the capture asserted a handle");

        let vetoes = HashSet::new();
        let claim = |holder| ClaimDraft::capture(holder, &observation, &key, identity);
        assert_eq!(
            evidence.assert(claim(first), &vetoes).await.unwrap(),
            Stored::Written
        );
        assert_eq!(
            evidence.assert(claim(first), &vetoes).await.unwrap(),
            Stored::AlreadyClaimed,
            "the same holder again is one claim, not a violation"
        );
        assert_eq!(
            evidence.assert(claim(second), &vetoes).await.unwrap(),
            Stored::Written,
            "a second holder is not an error"
        );

        assert_eq!(
            evidence.held_by_identity(identity).await.unwrap().len(),
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
            .upsert_identity(&DraftIdentity {
                scheme: "x_twitter".to_string(),
                external_id: "tsoding".to_string(),
                asserted_by: "wikidata".to_string(),
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
        evidence
            .identity_for_link(&IdentityKey::from_platform_link(&observation).expect("a handle"))
            .await
            .unwrap();

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
        assert_eq!(row.asserted_by, "wikidata");
        assert_eq!(row.label.as_deref(), Some("Alexey Kutepov"));
    }

    /// `classes` is an array column, and the round trip has to survive more than one element: every
    /// other test supplies none, which is exactly how an encoding bug hides.
    #[tokio::test]
    async fn identity_classes_round_trip_and_a_later_answer_does_not_erase_them() {
        db_or_skip!(db);
        let evidence = Evidence::from_handle(&db);
        let draft = DraftIdentity {
            scheme: "wikidata".to_string(),
            external_id: "Q42".to_string(),
            asserted_by: "wikidata".to_string(),
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
            .upsert_identity(&DraftIdentity {
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
