use anyhow::anyhow;
use sea_orm::TryIntoModel;

use crate::{api, auth, database::DbHandle, model};

pub async fn create_user(
    db: &DbHandle,
    context: &auth::Context,
    username: String,
    password: String,
    email: String,
) -> anyhow::Result<api::UserInfo, api::ApiError> {
    // Only allow admin users to create new users
    if !context.is_admin() {
        return Err(api::ApiError::forbidden(anyhow!(
            "Only admin users can create new users"
        )));
    }

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

    // Create new user in the database
    let user_new = model::user::ActiveModel::builder()
        .set_username(username.clone())
        .set_password_hash(password_hash)
        .set_email(email.clone())
        .save(&db.conn)
        .await?;

    let user_saved = user_new.try_into_model()?;

    Ok(api::UserInfo::from(user_saved))
}
