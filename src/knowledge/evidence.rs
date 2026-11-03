//! Claims: what a capture asserted, and who holds it.
//!
//! A claim is evidence of one kind — `kind = 'claim'` — and the identity index is what makes it
//! useful: no two entities may hold a *live* claim to the same `(scheme, id)`. Retracting frees the
//! slot, so a claim is a fact with a timestamp rather than a status.
//!
//! Two questions live here, and nothing else does:
//!
//! - **who holds this claim** ([`Evidence::owner`]) — one indexed lookup;
//! - **record it against an entity** ([`Evidence::record`]) — insert, or report who got there first.
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
    /// The handle in its comparable form, so two spellings agree.
    pub id: String,
    /// The handle as written, kept for the evidence payload.
    pub handle: Option<String>,
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
            id: normalize::comparable(handle),
            handle: Some(handle.to_string()),
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

    /// [`Self::key`] for a claim already built; equal to it for the claim that observation yields.
    pub fn identity(&self) -> String {
        format!("{}:{}", self.scheme, self.id)
    }
}

/// What happened when a claim was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Accepted,
    /// Another entity already holds this claim.
    HeldBy(i32),
}

/// The evidence table, for claims.
///
/// Holds its own clone of the connection rather than borrowing one: `DatabaseConnection` is a pool
/// handle behind an `Arc`, so cloning is a pointer copy, and the type can be built once and kept in
/// a long-lived state — which is the point of it being a parameter.
/// A handle, not state: cloning one clones the connection handle, which is what `Knowledge` stores so
/// that a caller linking thousands of illuminations does not build one per link.
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

    /// Which entity holds this claim, if any. The identity index serves this lookup.
    pub async fn owner(&self, claim: &Claim) -> anyhow::Result<Option<i32>> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT knowledge_entity_id FROM knowledge_evidence \
                 WHERE kind = 'claim' AND retracted_at IS NULL \
                   AND value->>'scheme' = $1 AND value->>'id' = $2 \
                 LIMIT 1",
                [claim.scheme.clone().into(), claim.id.clone().into()],
            ))
            .await?;
        Ok(match row {
            Some(row) => Some(row.try_get_by_index::<i32>(0)?),
            None => None,
        })
    }

    /// Record a claim against an entity.
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

        let insert =
            model::knowledge_evidence::Entity::insert(model::knowledge_evidence::ActiveModel {
                knowledge_entity_id: sea_orm::Set(knowledge_entity_id),
                kind: sea_orm::Set("claim".to_string()),
                value: sea_orm::Set(serde_json::json!({
                    "scheme": claim.scheme,
                    "id": claim.id,
                    "handle": claim.handle,
                })),
                source: sea_orm::Set("platform".to_string()),
                method: sea_orm::Set(Some("capture".to_string())),
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
