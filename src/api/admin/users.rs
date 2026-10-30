use sea_orm::QueryOrder;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

/// Export users, one `ExportRow<UserInfo>` per line.
///
/// Admin only. Lives under the export routes rather than `/admin/users` so it
/// cannot be confused with the administrative user actions (create, change
/// password) that share that path.
pub async fn export_users(
    db: &DbHandle,
    context: &auth::Context,
    user_id: Option<i32>,
) -> Result<Vec<schema::ExportRow<schema::UserInfo>>, ApiError> {
    super::client::ensure_admin(context)?;

    let mut query = model::user::Entity::find().order_by_asc(model::user::Column::Id);
    if let Some(user_id) = user_id {
        query = query.filter(model::user::Column::Id.eq(user_id));
    }

    Ok(query
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|user| schema::ExportRow {
            user_id: user.id,
            row: schema::UserInfo::from(model::user::ModelEx::from(user)),
        })
        .collect())
}
