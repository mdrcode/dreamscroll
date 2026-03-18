use sea_orm::{
    EntityTrait, ExprTrait, QueryFilter, QueryOrder, QuerySelect,
    prelude::*,
    sea_query::{Expr, Func},
};

use crate::{api, auth, database::DbHandle, model};

pub async fn search_by_illuminations(
    db: &DbHandle,
    user_context: &auth::Context,
    query: &str,
    limit: Option<u64>,
) -> anyhow::Result<Vec<model::capture::ModelEx>, api::ApiError> {
    let num_captures = limit.unwrap_or(100);
    if num_captures == 0 {
        return Ok(vec![]);
    }

    if query.is_empty() {
        return Ok(vec![]);
    }

    let capture_rows: Vec<(i32, chrono::DateTime<chrono::Utc>)> =
        model::search_index::Entity::find()
            .filter(model::search_index::Column::UserId.eq(user_context.user_id()))
            .filter(
                Expr::expr(Func::lower(Expr::col(model::search_index::Column::Content)))
                    .like(format!("%{}%", query.to_lowercase())),
            )
            .inner_join(model::capture::Entity)
            .filter(model::capture::Column::UserId.eq(user_context.user_id()))
            .select_only()
            .column(model::search_index::Column::CaptureId)
            .column(model::capture::Column::CreatedAt)
            .distinct()
            .order_by(model::capture::Column::CreatedAt, sea_orm::Order::Desc)
            .limit(num_captures)
            .into_tuple::<(i32, chrono::DateTime<chrono::Utc>)>()
            .all(&db.conn)
            .await?;

    let capture_ids = capture_rows
        .into_iter()
        .map(|(capture_id, _)| capture_id)
        .collect::<Vec<i32>>();

    if capture_ids.is_empty() {
        return Ok(vec![]);
    }

    let captures = super::get_captures(&db, user_context, capture_ids).await?;

    Ok(captures)
}
