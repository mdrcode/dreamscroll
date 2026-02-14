use anyhow::anyhow;
use sea_orm::TryIntoModel;

use crate::{api, auth, database::DbHandle, model};

/// Generates a random base36 string (a-z, 0-9) of num_chars length for use as
/// a user's storage shard prefix. Uses UUID v4 bytes as the entropy source.
/// Collisions will happen when number of users is low millions. If too many
/// collisions, just increase num_chars.
fn generate_storage_shard(num_chars: usize) -> String {
    if num_chars > 12 {
        panic!("generate_storage_shard: num_chars too large for current entropy source, max is 12");
    }
    let bytes = uuid::Uuid::new_v4();
    let mut value = u64::from_le_bytes(bytes.as_bytes()[..8].try_into().unwrap());
    let charset = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut result = String::with_capacity(num_chars);
    for _ in 0..num_chars {
        result.push(charset[(value % 36) as usize] as char);
        value /= 36;
    }
    result
}

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
        let candidate = generate_storage_shard(8);
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
