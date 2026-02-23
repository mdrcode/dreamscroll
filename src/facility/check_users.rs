use sea_orm::prelude::*;

use crate::{database::DbHandle, model};

pub async fn check_users(db: &DbHandle) -> anyhow::Result<()> {
    let user_count = model::user::Entity::find().count(&db.conn).await?;
    if user_count == 0 {
        tracing::error!(
            "No users found in db! Create first user with `dreamscroll_util check_first_user`."
        );
    } else {
        tracing::info!("Found {} users in database", user_count);
    }

    Ok(())
}
