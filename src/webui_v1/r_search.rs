use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use tera::Context;

use crate::common::AppError;
use crate::controller::CaptureInfo;
use crate::model::illumination;
use crate::webui_v1::WebState;

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    q: String,
}

pub async fn search(
    State(state): State<Arc<WebState>>,
    Query(params): Query<SearchParams>,
) -> Result<Response, AppError> {
    let query = params.q.trim();
    let mut context = Context::new();
    context.insert("query", query);

    let capture_infos: Vec<CaptureInfo> = if !query.is_empty() {
        // Find illuminations that contain the search query
        let matching_illuminations = illumination::Entity::find()
            .filter(illumination::Column::Content.contains(query))
            .all(&state.db.conn)
            .await
            .map_err(|e| AppError::internal(anyhow!("DB error searching illuminations: {}", e)))?;

        // Get unique capture IDs
        let capture_ids: Vec<i32> = matching_illuminations
            .iter()
            .map(|i| i.capture_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Fetch full capture info for each matching capture
        let mut results = Vec::new();
        for capture_id in capture_ids {
            if let Ok(info) = CaptureInfo::fetch_by_id(&state.db, capture_id).await {
                results.push(info);
            }
        }
        results
    } else {
        vec![]
    };

    context.insert("capture_infos", &capture_infos);
    context.insert("result_count", &capture_infos.len());

    let rendered = state
        .tera
        .render("search.html.tera", &context)
        .map_err(|e| AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
