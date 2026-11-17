//! Verdict reads: the durable decisions the graph replays and lists.
//!
//! A sub-view of [`GraphSelector`](super::GraphSelector). Verdicts are a different aggregate from
//! entities and identities, so their queries live here instead of on the main selector.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

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
}
