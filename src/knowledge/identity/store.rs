//! Where an authority's claim becomes a row: one identity upsert, one edge insert.
//!
//! The only writer of `knowledge_claims` from a provider. Keeping it alone makes the veto check and
//! the uniqueness rule one thing to read — and the place a second authority inherits both.

use sea_orm::{EntityTrait, Set};

use super::contract::Claim;
use super::engine::Report;
use crate::{database, model};

/// Store one claim as an edge to its identity. Returns whether a new claim was written; an identity
/// another entity already holds is not a conflict, because claims are many-to-one — whether the
/// holders are one thing is [`crate::knowledge::proximity`]'s question.
pub(crate) async fn store_claim(
    db: &database::DbHandle,
    asserted_by: &str,
    knowledge_entity_id: i32,
    claim: Claim,
    vetoes: &std::collections::HashSet<(i32, String, String)>,
    report: &mut Report,
) -> anyhow::Result<bool> {
    // Asked at the one place an authority claim becomes a row, so every provider inherits it without
    // knowing it exists.
    if vetoes.contains(&(knowledge_entity_id, claim.scheme.clone(), claim.id.clone())) {
        report.vetoed += 1;
        return Ok(false);
    }
    let evidence = crate::knowledge::evidence::Evidence::new(db.conn.clone());
    let identity_id = evidence
        .upsert_identity(&crate::knowledge::evidence::DraftIdentity {
            scheme: claim.scheme,
            external_id: claim.id,
            asserted_by: asserted_by.to_string(),
            method: Some(claim.method),
            confidence: Some(claim.confidence),
            label: claim.label,
            description: claim.description,
            classes: claim.classes,
        })
        .await?;

    let insert = model::knowledge_claims::Entity::insert(model::knowledge_claims::ActiveModel {
        knowledge_entity_id: Set(knowledge_entity_id),
        knowledge_identity_id: Set(identity_id),
        basis: Set(claim.basis.as_str().to_string()),
        ..Default::default()
    })
    .exec(&db.conn)
    .await;

    match insert {
        Ok(_) => {
            report.claims += 1;
            Ok(true)
        }
        // This entity already asserts the identity.
        Err(error) => match crate::knowledge::evidence::unique_violation(&error) {
            Some("idx-knowledge_claims_entity_identity") => Ok(false),
            _ => Err(error.into()),
        },
    }
}
