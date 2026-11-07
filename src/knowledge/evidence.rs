//! Claims and identities: what a capture asserted, what an authority answered, and who holds it.
//!
//! - a [`DraftIdentity`] is a row of `knowledge_identities` before it has an id: the thing the world
//!   names, `(scheme, external_id)`, plus whoever answered.
//! - a [`PlatformLink`] is a platform presence as a capture showed it — `(scheme, handle)`, both
//!   folded — before anything knows whether an entity holds it.
//! - a *claim* is an entity's assertion of an identity — the edge of the many-to-many, carrying what
//!   the question was about ([`ClaimBasis`]) and which capture asked. [`Evidence::record`] writes the
//!   capture's; `identity::store` writes a provider's.
//!
//! An identity may have several holders: one Wikidata item can name an album and cover its songs.
//! This module reports them and leaves "are these one thing?" to `proximity`.

use std::collections::HashSet;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

use crate::knowledge::normalize;
use crate::knowledge::scoring::TotalCmp;
use crate::model;

/// A platform link, as the capture showed it. `link` transcribes it; nothing infers anything here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformLink {
    /// The platform, canonicalised by the illumination lib: `x_twitter`, `reddit`, …
    pub scheme: String,
    /// The identifier in its comparable form, so two spellings agree.
    pub external_id: String,
}

impl PlatformLink {
    /// The claim an observation carries, if it carries one. Permissive: an unmapped platform still
    /// yields a claim, because the capture asserted it.
    pub fn of(observation: &model::user_observed_entity::Model) -> Option<Self> {
        let platform = observation.platform.as_deref()?;
        let handle = observation.platform_handle.as_deref()?;
        Some(Self {
            scheme: normalize::comparable(platform),
            external_id: normalize::comparable(handle),
        })
    }

    pub fn identity(&self) -> String {
        format!("{}:{}", self.scheme, self.external_id)
    }
}

/// An identity and the answer whoever gave it.
///
/// The authority's label, description and classes are properties of the identity, not of any one
/// claim: an album's article describes the album whether one entity claims it or five do.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftIdentity {
    pub scheme: String,
    pub external_id: String,
    /// Who answered: `wikidata`, `wikimedia`, `platform` (the resolver's own id).
    pub source: String,
    /// How it was answered, on the first answer only: `capture`, a Wikidata property (`P2002`, …) for
    /// a handle join, or the ladder step (`exact_label`, `search_ranked`, …).
    pub method: Option<String>,
    pub confidence: Option<f32>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
}

impl DraftIdentity {
    /// An identity known only from a capture: the handle, and nothing about it.
    pub fn from_capture(link: &PlatformLink) -> Self {
        Self {
            scheme: link.scheme.clone(),
            external_id: link.external_id.clone(),
            source: "platform".to_string(),
            method: Some("capture".to_string()),
            confidence: None,
            label: None,
            description: None,
            classes: Vec::new(),
        }
    }
}

/// What the question was about, for one claim: the identifier, or the text.
///
/// Corroboration counts independent *inputs*, and independence is this, not who answered. A capture's
/// handle and an authority asked about that handle are one chain however many schemes they name; a
/// name and a handle are two. Recorded when the claim is written, because that is the only moment
/// anything knows which question produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimBasis {
    /// A handle: a capture showed one, or the authority was asked about one.
    Handle,
    /// A name, title or description.
    Text,
}

impl ClaimBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Handle => "handle",
            Self::Text => "text",
        }
    }

    /// The stored form, back. Anything that is not `handle` reads as text: the column only ever holds
    /// these two.
    pub fn from_db(value: &str) -> Self {
        if value == Self::Handle.as_str() {
            Self::Handle
        } else {
            Self::Text
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
                   (scheme, external_id, label, description, classes, source, method, confidence) \
                 VALUES ($1, $2, $3, $4, string_to_array($5, chr(31)), $6, $7, $8) \
                 ON CONFLICT (scheme, external_id) DO UPDATE SET \
                   label = COALESCE(EXCLUDED.label, knowledge_identities.label), \
                   description = COALESCE(EXCLUDED.description, knowledge_identities.description), \
                   classes = CASE WHEN cardinality(EXCLUDED.classes) > 0 \
                                  THEN EXCLUDED.classes ELSE knowledge_identities.classes END, \
                   method = COALESCE(EXCLUDED.method, knowledge_identities.method), \
                   confidence = COALESCE(EXCLUDED.confidence, knowledge_identities.confidence), \
                   source = CASE WHEN EXCLUDED.label IS NOT NULL \
                                 THEN EXCLUDED.source ELSE knowledge_identities.source END \
                 RETURNING id",
                vec![
                    sea_orm::Value::String(Some(identity.scheme.clone())),
                    sea_orm::Value::String(Some(identity.external_id.clone())),
                    sea_orm::Value::String(identity.label.clone()),
                    sea_orm::Value::String(identity.description.clone()),
                    // `classes` is a text array; chr(31) is the unit separator, which no class
                    // name contains. An empty string splits to an empty array.
                    sea_orm::Value::String(Some(identity.classes.join(&31u8.to_string()))),
                    sea_orm::Value::String(Some(identity.source.clone())),
                    sea_orm::Value::String(identity.method.clone()),
                    sea_orm::Value::Float(identity.confidence),
                ],
            ))
            .await?
            .ok_or_else(|| anyhow::anyhow!("identity insert returned no row"))?;
        Ok(row.try_get_by_index::<i32>(0)?)
    }

    /// Import every identity this illumination's observations assert.
    ///
    /// The executor's first act, before deciding, and a contract rather than a type: the decider reads
    /// identities back with [`Self::identity_of`], so a caller that skips this leaves them unrecorded.
    /// Importing first is what makes [`Self::record`] an insert with nothing to create, and what lets
    /// a decision name an identity by id.
    ///
    /// Upserts, so repeating it is free. A link whose decision phase then fails leaves identity rows
    /// behind — harmless, because an identity is a fact about the world rather than a claim about the
    /// graph, and the next import finds them.
    pub async fn import_identities(
        &self,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<()> {
        let mut seen: HashSet<String> = HashSet::new();
        for observation in observations {
            let Some(link) = PlatformLink::of(observation) else {
                continue;
            };
            // Two observations asserting one handle are one identity; the second upsert would be the
            // row the first just wrote.
            if !seen.insert(link.identity()) {
                continue;
            }
            self.upsert_identity(&DraftIdentity::from_capture(&link)).await?;
        }
        Ok(())
    }

    /// The identity this claim cites, if a row exists. Reads only; importing is the caller's job.
    async fn identity_id(&self, link: &PlatformLink) -> anyhow::Result<Option<i32>> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT id FROM knowledge_identities WHERE scheme = $1 AND external_id = $2",
                [
                    link.scheme.clone().into(),
                    link.external_id.clone().into(),
                ],
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
        match PlatformLink::of(observation) {
            Some(claim) => self.identity_id(&claim).await,
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

    /// Record that an entity claims an identity the capture asserted.
    ///
    /// The identity id was handed over by [`Self::import_identities`], so there is nothing left to
    /// create or look up here: this is one insert of the edge, and the surface is whatever the
    /// capture wrote, read off the observation rather than reconstructed.
    ///
    /// Idempotent per entity: asserting an identity twice is corroboration, not a duplicate. Another
    /// entity holding it is not an error either — claims are many-to-one now, and whether the holders
    /// are one thing is `proximity`'s question, not this call's.
    pub async fn record(
        &self,
        knowledge_entity_id: i32,
        knowledge_identity_id: i32,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<()> {
        let insert =
            model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
                knowledge_entity_id: sea_orm::Set(knowledge_entity_id),
                knowledge_identity_id: sea_orm::Set(knowledge_identity_id),
                // A capture showed a handle: that is what the question was about.
                basis: sea_orm::Set(ClaimBasis::Handle.as_str().to_string()),
                surface: sea_orm::Set(observation.platform_handle.clone()),
                user_observed_entity_id: sea_orm::Set(Some(observation.id)),
                source_user_id: sea_orm::Set(Some(observation.user_id)),
                ..Default::default()
            })
            .exec(&self.conn)
            .await;

        match insert {
            Ok(_) => Ok(()),
            // This entity already asserts the identity.
            Err(error) => match unique_violation(&error) {
                Some("idx-knowledge_claims_entity_identity") => Ok(()),
                _ => Err(error.into()),
            },
        }
    }
}

/// The constraint a unique violation broke, if it was one. Postgres reports `23505`.
pub fn unique_violation(error: &sea_orm::DbErr) -> Option<&str> {
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
        evidence
            .import_identities(std::slice::from_ref(&observation))
            .await
            .expect("the capture asserted a handle");
        let identity = evidence
            .identity_of(&observation)
            .await
            .expect("the lookup should not fail")
            .expect("already a row");

        evidence.record(first, identity, &observation).await.expect("first");
        evidence
            .record(first, identity, &observation)
            .await
            .expect("the same holder again is one claim, not a violation");
        evidence
            .record(second, identity, &observation)
            .await
            .expect("a second holder is not an error");

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
}
