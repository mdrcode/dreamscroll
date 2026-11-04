//! Claims and identities: what a capture asserted, what an authority answered, and who holds it.
//!
//! - an [`Identity`] is the thing the world names: `(scheme, external_id)` plus whoever answered.
//! - a [`Claim`] is an entity's assertion of it — the edge of the many-to-many, carrying what the
//!   question was about ([`ClaimBasis`]) and which capture asked.
//!
//! An identity may have several holders: one Wikidata item can name an album and cover its songs.
//! This module reports them and leaves "are these one thing?" to `reconcile`.

use std::collections::HashSet;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

use crate::knowledge::normalize;
use crate::knowledge::score::Worth;
use crate::model;

/// A platform link, as the capture showed it. `link` transcribes it; nothing infers anything here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The platform, canonicalised by the illumination lib: `x_twitter`, `reddit`, …
    pub scheme: String,
    /// The identifier in its comparable form, so two spellings agree.
    pub external_id: String,
}

impl Claim {
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

    /// The key a platform presence answers to: `scheme:handle`, both halves in comparable form. One
    /// function for the two callers — the identity index and the blocker's key — because when they
    /// folded the halves differently, a handle was held by one entity and blocked under another.
    pub fn key(platform: &str, handle: &str) -> String {
        format!(
            "{}:{}",
            normalize::comparable(platform),
            normalize::comparable(handle)
        )
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
pub struct Identity {
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

impl Identity {
    /// An identity known only from a capture: the handle, and nothing about it.
    pub fn from_capture(claim: &Claim) -> Self {
        Self {
            scheme: claim.scheme.clone(),
            external_id: claim.external_id.clone(),
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

/// Why a proximity was raised.
///
/// A named type rather than a string at the call sites: the value is stored, compared, and will grow a
/// second case (entities that look alike without sharing an identity), so a typo should not become a
/// third basis that every query silently misses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Two entities hold the same identity.
    SharedId,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedId => "shared_id",
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
    pub async fn identity(&self, identity: &Identity) -> anyhow::Result<i32> {
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
            let Some(claim) = Claim::of(observation) else {
                continue;
            };
            // Two observations asserting one handle are one identity; the second upsert would be the
            // row the first just wrote.
            if !seen.insert(claim.identity()) {
                continue;
            }
            self.identity(&Identity::from_capture(&claim)).await?;
        }
        Ok(())
    }

    /// The identity this claim cites, if a row exists. Reads only; importing is the caller's job.
    async fn identity_id(&self, claim: &Claim) -> anyhow::Result<Option<i32>> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT id FROM knowledge_identities WHERE scheme = $1 AND external_id = $2",
                [
                    claim.scheme.clone().into(),
                    claim.external_id.clone().into(),
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
        match Claim::of(observation) {
            Some(claim) => self.identity_id(&claim).await,
            None => Ok(None),
        }
    }

    /// Record a proximity: two entities close enough to ask about. A pair is recorded once, for ever —
    /// `ON CONFLICT DO NOTHING` is what makes a settled verdict stick. Returns 1 when a row was added.
    pub async fn raise(
        &self,
        holder_id: i32,
        counterpart_id: i32,
        knowledge_identity_id: Option<i32>,
        basis: Basis,
        score: Option<f32>,
    ) -> anyhow::Result<usize> {
        let result = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO knowledge_proximities \
                   (holder_id, counterpart_id, knowledge_identity_id, basis, score) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT DO NOTHING",
                [
                    holder_id.into(),
                    counterpart_id.into(),
                    knowledge_identity_id.into(),
                    basis.as_str().to_string().into(),
                    score.into(),
                ],
            ))
            .await?;
        Ok(result.rows_affected() as usize)
    }

    /// The entities holding this identity, most substantiated first: the reconcile pass keeps the
    /// first and asks about the rest, and the linker attaches to the first.
    ///
    /// The ordering is the policy's, asked here rather than written into the SQL: `merge` picks its
    /// survivor with the same comparison, and two spellings of "most substantiated" disagreed on
    /// `NULL` for as long as they existed.
    pub async fn held_by_identity(
        &self,
        identity_id: i32,
        worth: &dyn Worth,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>> {
        use sea_orm::{ColumnTrait, QueryFilter};
        let rows = self
            .conn
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT c.knowledge_entity_id \
                   FROM knowledge_claims c \
                   JOIN knowledge_entities e ON e.id = c.knowledge_entity_id \
                  WHERE c.knowledge_identity_id = $1 AND c.retracted_at IS NULL \
                    AND e.merged_into_id IS NULL \
                  ORDER BY c.knowledge_entity_id",
                [identity_id.into()],
            ))
            .await?;
        let ids: Vec<i32> = rows
            .iter()
            .map(|row| row.try_get_by_index::<i32>(0))
            .collect::<Result<_, _>>()?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut entities = model::knowledge_entity::Entity::find()
            .filter(model::knowledge_entity::Column::Id.is_in(ids))
            .all(&self.conn)
            .await?;
        entities.sort_by(|a, b| {
            worth
                .order(b.confidence, a.confidence)
                .then(a.id.cmp(&b.id))
        });
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
    /// are one thing is the reconcile pass's question, not this call's.
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
    use super::*;

    fn observation(platform: &str, handle: &str) -> model::user_observed_entity::Model {
        model::user_observed_entity::Model {
            id: 1,
            user_id: 1,
            capture_id: 1,
            illumination_id: 1,
            position: 0,
            name: "Someone".to_string(),
            entity_type: "real_person".to_string(),
            description: None,
            search_query: None,
            platform: Some(platform.to_string()),
            platform_handle: Some(handle.to_string()),
            platform_display_name: None,
            knowledge_entity_id: None,
            created_at: chrono::Utc::now(),
        }
    }

    /// The identity the index enforces and the key the blocker indexes must be the same string, or
    /// one entity holds a handle another entity is blocked under.
    #[test]
    fn the_claim_key_is_the_claim_identity() {
        for (platform, handle) in [
            ("x_twitter", "tsoding"),
            ("x_twitter", "@tsoding"),
            ("reddit", "Some_User"),
            ("  x_twitter  ", "tsoding."),
        ] {
            let claim = Claim::of(&observation(platform, handle)).expect("a handle");
            assert_eq!(
                claim.identity(),
                Claim::key(platform, handle),
                "{platform}/{handle}"
            );
        }
    }
}
