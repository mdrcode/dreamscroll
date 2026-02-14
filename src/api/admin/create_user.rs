use anyhow::anyhow;
use sea_orm::TryIntoModel;

use crate::{api, auth, database::DbHandle, model};

pub async fn create_user(
    db: &DbHandle,
    username: String,
    password: String,
    email: String,
) -> anyhow::Result<api::UserInfo, api::ApiError> {
    // Check if already exists
    let existing_user = model::user::Entity::find_by_username(username.clone())
        .one(&db.conn)
        .await
        .map_err(|e| {
            api::ApiError::internal(anyhow!(
                "DB error checking existing username {}: {}",
                username,
                e
            ))
        })?;

    if existing_user.is_some() {
        return Err(api::ApiError::conflict(anyhow!(
            "Username {} already exists",
            username
        )));
    }

    // Hash the password
    let password_hash = auth::password::hash(&password).map_err(|e| {
        api::ApiError::internal(anyhow!(
            "Password hashing failed for user {}: {}",
            username,
            e
        ))
    })?;

    // Generate a unique storage shard with collision retry
    const MAX_SHARD_RETRIES: usize = 10;
    let mut storage_shard = None;
    for attempt in 0..MAX_SHARD_RETRIES {
        let candidate = model::user::generate_storage_shard(8);
        let existing = model::user::Entity::find_by_storage_shard(candidate.clone())
            .one(&db.conn)
            .await
            .map_err(|e| {
                api::ApiError::internal(anyhow!(
                    "DB error checking storage shard uniqueness: {}",
                    e
                ))
            })?;
        if existing.is_none() {
            storage_shard = Some(candidate);
            break;
        }
        tracing::warn!(
            "Storage shard collision on '{}' (attempt {}/{}), retrying",
            candidate,
            attempt + 1,
            MAX_SHARD_RETRIES,
        );
    }
    let storage_shard = storage_shard.ok_or_else(|| {
        api::ApiError::internal(anyhow!(
            "Failed to generate unique storage shard after {} attempts",
            MAX_SHARD_RETRIES
        ))
    })?;

    // Create new user in the database
    let user_new = model::user::ActiveModel::builder()
        .set_username(username.clone())
        .set_password_hash(password_hash)
        .set_email(email.clone())
        .set_storage_shard(storage_shard)
        .save(&db.conn)
        .await?;

    let user_saved = user_new.try_into_model()?;

    Ok(api::UserInfo::from(user_saved))
}
