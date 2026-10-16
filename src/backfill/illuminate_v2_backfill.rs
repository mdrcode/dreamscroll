use sea_orm::{EntityTrait, QuerySelect, QueryTrait, sea_query::SelectStatement};

use crate::{illumination, logic, model};

use super::definition::{Backfill, BackfillTarget, TargetColumn};

/// Change this ID when changing the definition. Definition identity is developer-managed.
pub const ILLUMINATE_V2_BACKFILL_ID: &str = "illuminate-v2-all-captures-2026-10-16";

pub(super) struct IlluminateV2Backfill;

impl Backfill for IlluminateV2Backfill {
    type Task = logic::illuminate::IlluminationTask;

    fn id(&self) -> &'static str {
        ILLUMINATE_V2_BACKFILL_ID
    }

    fn targets(&self) -> SelectStatement {
        // All captures, including archived and already illuminated ones.
        model::capture::Entity::find()
            .select_only()
            .column_as(model::capture::Column::Id, TargetColumn::EntityId)
            .column_as(model::capture::Column::UserId, TargetColumn::OwnerId)
            .into_query()
    }

    fn make_task(&self, target: BackfillTarget) -> Self::Task {
        logic::illuminate::IlluminationTask::new(
            target.entity_id,
            target.owner_id,
            "gemini-3.8-flash",
            illumination::IlluminationVersion::V2,
        )
    }
}
