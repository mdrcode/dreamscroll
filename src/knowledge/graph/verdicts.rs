//! Verdict reads: the durable decisions the graph replays and lists.
//!
//! A sub-view of [`GraphSelector`](super::GraphSelector). Verdicts are a different aggregate from
//! entities and identities, so their queries live here instead of on the main selector.

use std::collections::HashSet;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use super::selection::resolve_live_entity;
use crate::knowledge::ids::EntityOriginId;
use crate::model;

/// A borrowed read view over `knowledge_verdicts`, created by
/// [`GraphSelector::verdicts`](super::GraphSelector::verdicts).
#[derive(Clone, Copy)]
pub struct VerdictSelector<'a> {
    conn: &'a DatabaseConnection,
}

impl<'a> VerdictSelector<'a> {
    pub(super) fn new(conn: &'a DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Durable repair merges, oldest first.
    pub async fn merges(&self) -> anyhow::Result<Vec<model::knowledge_verdicts::Model>> {
        Ok(model::knowledge_verdicts::Entity::find()
            .filter(
                model::knowledge_verdicts::Column::Cause
                    .eq(model::knowledge_verdicts::Cause::Repair.as_str()),
            )
            .filter(
                model::knowledge_verdicts::Column::Kind
                    .eq(model::knowledge_verdicts::Kind::Merge.as_str()),
            )
            .order_by_asc(model::knowledge_verdicts::Column::Id)
            .all(self.conn)
            .await?)
    }

    /// Durable repair vetoes, oldest first.
    pub async fn vetoes(&self) -> anyhow::Result<Vec<model::knowledge_verdicts::Model>> {
        Ok(model::knowledge_verdicts::Entity::find()
            .filter(
                model::knowledge_verdicts::Column::Cause
                    .eq(model::knowledge_verdicts::Cause::Repair.as_str()),
            )
            .filter(
                model::knowledge_verdicts::Column::Kind
                    .eq(model::knowledge_verdicts::Kind::VetoIdentity.as_str()),
            )
            .order_by_asc(model::knowledge_verdicts::Column::Id)
            .all(self.conn)
            .await?)
    }

    /// Every live veto as `(live entity id, scheme, external_id)`, for admission and dry runs.
    pub async fn vetoed_keys(&self) -> anyhow::Result<HashSet<(i32, String, String)>> {
        let rows = model::knowledge_verdicts::Entity::find()
            .filter(
                model::knowledge_verdicts::Column::Kind
                    .eq(model::knowledge_verdicts::Kind::VetoIdentity.as_str()),
            )
            .order_by_asc(model::knowledge_verdicts::Column::Id)
            .all(self.conn)
            .await?;

        let mut out = HashSet::new();
        for row in rows {
            let (Some(origin), Some(scheme), Some(external)) = (
                row.subject_origin,
                row.identity_scheme,
                row.identity_external_id,
            ) else {
                continue;
            };
            let Some(entity) = resolve_live_entity(self.conn, origin).await? else {
                continue;
            };
            out.insert((entity, scheme, external));
        }
        Ok(out)
    }

    /// Whether a `distinct` verdict names this unordered pair of origins.
    pub async fn kept_apart(&self, a: EntityOriginId, b: EntityOriginId) -> anyhow::Result<bool> {
        let (a, b) = (a.0.0, b.0.0);
        let rows = model::knowledge_verdicts::Entity::find()
            .filter(model::knowledge_verdicts::Column::Kind.eq("distinct"))
            .filter(model::knowledge_verdicts::Column::WinnerOrigin.is_in([a, b]))
            .filter(model::knowledge_verdicts::Column::LoserOrigin.is_in([a, b]))
            .all(self.conn)
            .await?;
        Ok(rows.iter().any(|row| {
            matches!(
                (row.winner_origin, row.loser_origin),
                (Some(winner), Some(loser))
                    if (winner == a && loser == b) || (winner == b && loser == a)
            )
        }))
    }
}
