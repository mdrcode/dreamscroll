//! Standing vetoes: an entity is *not* an identity.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::model;
use crate::model::knowledge_verdicts::Kind;

use super::Cause;
use super::store::VerdictStore;

/// What one veto did: the register row, and the annotation it withdrew.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct VetoReport {
    /// A new row was written; `false` when the same pairing was already recorded.
    pub recorded: bool,
    /// Identity annotations deleted. Zero is settled: the pairing already held, or no row carries it.
    pub withdrawn: usize,
}

impl VerdictStore {
    /// Record that an entity is *not* an identity.
    ///
    /// Idempotent per subject and identity, and deliberately not per generation: a fold recurs every
    /// build, but a veto is a standing decision about the graph.
    pub async fn veto(
        &self,
        generation: i32,
        entity_id: i32,
        scheme: &str,
        external_id: &str,
        note: Option<&str>,
    ) -> anyhow::Result<bool> {
        let conn = &self.conn;
        let Some(entity) = model::knowledge_entity::Entity::find_by_id(entity_id)
            .one(conn)
            .await?
        else {
            anyhow::bail!("no knowledge entity {entity_id}");
        };
        let Some(subject) = entity.origin_observation_id else {
            anyhow::bail!(
                "entity {entity_id} has no origin observation, so no verdict can name it"
            );
        };
        let existing = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::VetoIdentity.as_str()))
            .filter(model::knowledge_verdicts::Column::SubjectOrigin.eq(subject))
            .filter(model::knowledge_verdicts::Column::IdentityScheme.eq(scheme))
            .filter(model::knowledge_verdicts::Column::IdentityExternalId.eq(external_id))
            .one(conn)
            .await?;
        if existing.is_some() {
            return Ok(false);
        }
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(generation),
            cause: Set(Cause::Repair),
            kind: Set(Kind::VetoIdentity),
            subject_origin: Set(Some(subject)),
            identity_scheme: Set(Some(scheme.to_string())),
            identity_external_id: Set(Some(external_id.to_string())),
            note: Set(note.map(str::to_string)),
            ..Default::default()
        })
        .exec(conn)
        .await?;
        Ok(true)
    }

    /// Lift a veto. Nothing is undeleted: lifting stops suppressing a decision, and the next resolve
    /// recreates the identity annotation.
    pub async fn unveto(&self, verdict_id: i32) -> anyhow::Result<bool> {
        let deleted = model::knowledge_verdicts::Entity::delete_many()
            .filter(model::knowledge_verdicts::Column::Id.eq(verdict_id))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::VetoIdentity.as_str()))
            .exec(&self.conn)
            .await?
            .rows_affected;
        Ok(deleted > 0)
    }

    /// The `(entity, scheme, external_id)` triples a live veto forbids.
    ///
    /// Admission asks this before an annotation is written: one that is never written cannot leave a
    /// window where the graph asserts something an editor has rejected, and does not need a replay to
    /// undo.
    pub async fn vetoed(&self) -> anyhow::Result<std::collections::HashSet<(i32, String, String)>> {
        let mut out = std::collections::HashSet::new();
        for verdict in self.vetoes().await? {
            let (Some(subject), Some(scheme), Some(external)) = (
                verdict.subject_origin,
                verdict.identity_scheme.clone(),
                verdict.identity_external_id.clone(),
            ) else {
                continue;
            };
            let Some(row) = self.entity_with_origin(subject).await? else {
                continue;
            };
            let Some(entity_id) = self.canonical(row.id).await? else {
                continue;
            };
            out.insert((entity_id, scheme, external));
        }
        Ok(out)
    }
}
