use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{EntityTrait, QueryOrder, QuerySelect};
use serde::Serialize;

use crate::common::*;
use crate::database::DbHandle;
use crate::model::*;

#[derive(Serialize)]
pub struct IlluminationInfo;

impl IlluminationInfo {
    pub async fn insert(
        db: &DbHandle,
        capture_id: i32,
        provider: &str,
        content: &str,
    ) -> anyhow::Result<(), AppError> {
        let new_illumination = illumination::ActiveModel {
            capture_id: Set(capture_id),
            provider: Set(provider.to_string()),
            content: Set(content.to_string()),
            ..Default::default()
        };

        illumination::Entity::insert(new_illumination)
            .exec(&db.conn)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }
}
