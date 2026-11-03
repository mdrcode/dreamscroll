//! Claims and identities: what a capture asserted, what an authority answered, and who holds it.
//!
//! Two rows, two jobs:
//!
//! - a [`Identity`] is the thing the world names — `(scheme, external_id)` plus the answer whoever
//!   gave it (label, description, classes, method, confidence). One row per identity, ever.
//! - a [`Claim`] is an entity's assertion of it, with the surface as a capture wrote it and which
//!   capture that was. The edge of the many-to-many.
//!
//! For now the graph is single-holder — a unique index keeps one live claim per identity, standing
//! in for "an identity is one thing" — and this module reports the holder so callers can fold or
//! refuse.
//!
//! What a caller does with a refusal is policy and stays with the caller: `link` declines to merge
//! and leaves the observation provisional; `resolve` folds or records a collision.

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};

use crate::knowledge::normalize;
use crate::model;

/// A platform link, as the capture showed it.
///
/// The illuminator read this off an image; `link` transcribes it. Nothing infers anything here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// The platform, canonicalised by the illumination lib: `x_twitter`, `reddit`, …
    pub scheme: String,
    /// The identifier in its comparable form, so two spellings agree.
    pub external_id: String,
    /// The identifier as written, when a capture carried it verbatim.
    pub surface: Option<String>,
}

impl Claim {
    /// The claim an observation carries, if it carries one.
    ///
    /// Deliberately permissive: a platform we have no mapping for still yields a claim, because the
    /// capture asserted it and losing that would be losing evidence. What can be *used* is another
    /// question, asked later by whatever resolves identities against an authority.
    pub fn of(observation: &model::user_observed_entity::Model) -> Option<Self> {
        let platform = observation.platform.as_deref()?;
        let handle = observation.platform_handle.as_deref()?;
        Some(Self {
            scheme: normalize::comparable(platform),
            external_id: normalize::comparable(handle),
            surface: Some(handle.to_string()),
        })
    }

    /// The key a platform presence answers to: `scheme:handle`, both halves compared in the same
    /// form.
    ///
    /// One function for the two callers that need it — the identity index ([`Self::of`] then
    /// [`Self::identity`]) and the blocker's key for an entity's projected presence — because when
    /// they normalized the halves differently, a handle could be held by one entity and blocked
    /// under another.
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
    /// Who answered: `wikidata`, `wikimedia`, `platform`.
    pub source: String,
    /// How: `exact_label`, `p2002`, `capture`.
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

    pub fn identity(&self) -> String {
        format!("{}:{}", self.scheme, self.external_id)
    }
}

/// What happened when a claim was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Accepted,
    /// Another entity already holds this claim.
    HeldBy(i32),
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

    /// Which entity holds this identity, if any. Unique while the graph is single-holder.
    pub async fn holder(&self, identity_id: i32) -> anyhow::Result<Option<i32>> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT knowledge_entity_id FROM knowledge_claims \
                 WHERE knowledge_identity_id = $1 AND retracted_at IS NULL LIMIT 1",
                [identity_id.into()],
            ))
            .await?;
        Ok(match row {
            Some(row) => Some(row.try_get_by_index::<i32>(0)?),
            None => None,
        })
    }

    /// Which entity holds this claim, if any. Unique while the graph is single-holder.
    pub async fn owner(&self, claim: &Claim) -> anyhow::Result<Option<i32>> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT c.knowledge_entity_id \
                   FROM knowledge_claims c \
                   JOIN knowledge_identities i ON i.id = c.knowledge_identity_id \
                  WHERE i.scheme = $1 AND i.external_id = $2 AND c.retracted_at IS NULL \
                  LIMIT 1",
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

    /// Record a claim against an entity, creating the identity if it is new.
    ///
    /// Held by this same entity already → `Accepted`: the claim is true either way, and a second
    /// observation asserting it is corroboration, not a duplicate to report.
    pub async fn record(
        &self,
        knowledge_entity_id: i32,
        claim: &Claim,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<ClaimOutcome> {
        if let Some(owner) = self.owner(claim).await? {
            return Ok(if owner == knowledge_entity_id {
                ClaimOutcome::Accepted
            } else {
                ClaimOutcome::HeldBy(owner)
            });
        }

        let identity_id = self.identity(&Identity::from_capture(claim)).await?;
        let insert =
            model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
                knowledge_entity_id: sea_orm::Set(knowledge_entity_id),
                knowledge_identity_id: sea_orm::Set(identity_id),
                surface: sea_orm::Set(claim.surface.clone()),
                user_observed_entity_id: sea_orm::Set(Some(observation.id)),
                source_user_id: sea_orm::Set(Some(observation.user_id)),
                ..Default::default()
            })
            .exec(&self.conn)
            .await;

        match insert {
            Ok(_) => Ok(ClaimOutcome::Accepted),
            // Another worker claimed it between the lookup and the insert. The index said so;
            // report the owner so the caller can fold instead of leaving two referents.
            Err(error) => match unique_violation(&error) {
                Some("idx-knowledge_claims_entity_identity") => Ok(ClaimOutcome::Accepted),
                Some(_) => match self.owner(claim).await? {
                    Some(owner) if owner != knowledge_entity_id => Ok(ClaimOutcome::HeldBy(owner)),
                    Some(_) => Ok(ClaimOutcome::Accepted),
                    None => Err(error.into()),
                },
                None => Err(error.into()),
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
