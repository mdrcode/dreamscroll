use sea_orm::{
    EntityTrait, ExprTrait, QueryFilter, QuerySelect,
    prelude::*,
    sea_query::{Expr, Func},
};

use crate::{api, auth, database::DbHandle, model};

pub async fn search_by_illuminations(
    db: &DbHandle,
    user_context: &auth::Context,
    query: &str,
) -> anyhow::Result<Vec<model::capture::ModelEx>, api::ApiError> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    let iids_matching = model::search_index::Entity::find()
        .filter(model::search_index::Column::UserId.eq(user_context.user_id()))
        .filter(
            Expr::expr(Func::lower(Expr::col(model::search_index::Column::Content)))
                .like(format!("%{}%", query.to_lowercase())),
        )
        .column(model::search_index::Column::IlluminationId)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|si| si.illumination_id)
        .collect::<Vec<i32>>();

    let capture_ids = model::illumination::Entity::find()
        .filter(model::illumination::Column::Id.is_in(iids_matching.clone()))
        .column(model::illumination::Column::CaptureId)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|illum| illum.capture_id)
        .collect::<Vec<i32>>();

    let captures = super::get_captures(&db, user_context, Some(capture_ids)).await?;

    Ok(captures)
}
