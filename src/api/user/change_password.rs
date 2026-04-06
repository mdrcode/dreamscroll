use anyhow::anyhow;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

use crate::{api::ApiError, auth, database::DbHandle, model};

const MIN_PASSWORD_LENGTH: usize = 8;

pub async fn change_password(
    db: &DbHandle,
    context: &auth::Context,
    current_password: String,
    new_password: String,
) -> Result<(), ApiError> {
    if new_password.trim().is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "New password cannot be empty"
        )));
    }

    if new_password.len() < MIN_PASSWORD_LENGTH {
        return Err(ApiError::bad_request(anyhow!(
            "New password must be at least {} characters",
            MIN_PASSWORD_LENGTH
        )));
    }

    if current_password == new_password {
        return Err(ApiError::bad_request(anyhow!(
            "New password must be different from current password"
        )));
    }

    let db_user = model::user::Entity::find_by_id(context.user_id())
        .one(&db.conn)
        .await?
        .ok_or_else(|| ApiError::unauthorized(anyhow!("Authenticated user not found")))?;

    let current_ok = auth::password::verify_hash(&db_user.password_hash, &current_password)
        .map_err(|e| ApiError::internal(anyhow!("Current password verification failed: {}", e)))?;

    if !current_ok {
        return Err(ApiError::unauthorized(anyhow!("Invalid current password")));
    }

    let new_hash = auth::password::hash(&new_password)
        .map_err(|e| ApiError::internal(anyhow!("Password hashing failed: {}", e)))?;

    let mut active: model::user::ActiveModel = db_user.into();
    active.password_hash = Set(new_hash);
    active.update(&db.conn).await?;

    Ok(())
}
