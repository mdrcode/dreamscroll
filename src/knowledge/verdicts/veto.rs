//! Standing vetoes: an entity is *not* an identity.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::knowledge::graph::current_generation;
use crate::knowledge::graph::KnowledgeGraph;
use crate::model;
use crate::model::knowledge_verdicts::Kind;

use super::Cause;
use super::store::VerdictStore;

impl VerdictStore {
    /// Record that an entity is *not* an identity.
    ///
    /// Idempotent per subject and identity, and deliberately not per generation: a fold recurs every
    /// build, but a veto is a standing decision about the graph.
    pub async fn veto(
        &self,
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
            anyhow::bail!("entity {entity_id} has no origin observation, so no verdict can name it");
        };
        let generation = current_generation(conn).await?;
        let existing = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
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
            kind: Set(Kind::Veto),
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

    /// Lift a veto. Nothing is un-retracted: lifting stops suppressing a decision, and the next
    /// resolve re-derives the claim as a new row, because claims are append-only.
    pub async fn unveto(&self, verdict_id: i32) -> anyhow::Result<bool> {
        let deleted = model::knowledge_verdicts::Entity::delete_many()
            .filter(model::knowledge_verdicts::Column::Id.eq(verdict_id))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
            .exec(&self.conn)
            .await?
            .rows_affected;
        Ok(deleted > 0)
    }

    /// The `(entity, scheme, external_id)` triples a live veto forbids.
    ///
    /// Admission asks this before a claim is written: a claim that is never written cannot leave a
    /// window where the graph asserts something an editor has rejected, and does not need a replay to
    /// undo.
    pub async fn vetoed(
        &self,
    ) -> anyhow::Result<std::collections::HashSet<(i32, String, String)>> {
        let mut out = std::collections::HashSet::new();
        let vetoes = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
            .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
            .all(&self.conn)
            .await?;
        for verdict in vetoes {
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

    /// Retract the live claim holding a vetoed identity for its entity, then reproject it.
    ///
    /// The veto names a **pairing**, so the retraction does too: there is at most one live claim for
    /// `(entity, identity)`, whatever route produced it. `None` when the subject origin no longer
    /// names a live entity; `Some(0)` when the decision already holds and there is nothing to
    /// retract.
    ///
    /// The retraction and the projection are one act: `status` and `confidence` are functions of the
    /// live claims, so withdrawing a claim without reprojecting leaves the entity `identified` on
    /// nothing — and that is the row the API serves and `proximity auto_merge` ranks.
    pub(super) async fn withdraw(
        &self,
        graph: &KnowledgeGraph,
        verdict: &model::knowledge_verdicts::Model,
    ) -> anyhow::Result<Option<usize>> {
        let conn = &self.conn;
        let (Some(subject), Some(scheme), Some(external)) = (
            verdict.subject_origin,
            verdict.identity_scheme.as_deref(),
            verdict.identity_external_id.as_deref(),
        ) else {
            return Ok(Some(0));
        };
        let Some(row) = self.entity_with_origin(subject).await? else {
            return Ok(None);
        };
        let Some(entity_id) = self.canonical(row.id).await? else {
            return Ok(None);
        };
        let Some(identity) = model::knowledge_identity::Entity::find()
            .filter(model::knowledge_identity::Column::Scheme.eq(scheme))
            .filter(model::knowledge_identity::Column::ExternalId.eq(external))
            .one(conn)
            .await?
        else {
            return Ok(Some(0));
        };
        let retracted = model::knowledge_claims::Entity::update_many()
            .col_expr(
                model::knowledge_claims::Column::RetractedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(entity_id))
            .filter(model::knowledge_claims::Column::KnowledgeIdentityId.eq(identity.id))
            .filter(model::knowledge_claims::Column::RetractedAt.is_null())
            .exec(conn)
            .await?
            .rows_affected as usize;
        if retracted > 0 {
            graph.update_projection(entity_id).await?;
        }
        Ok(Some(retracted))
    }
}
