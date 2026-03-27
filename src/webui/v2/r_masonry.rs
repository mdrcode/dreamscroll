use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;
use serde::Serialize;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, Clone, Serialize)]
struct MasonryImage {
    capture_id: i32,
    url: String,
}

#[derive(Debug, Deserialize)]
pub struct MasonrySpec {
    pub limit: Option<u64>,
}

impl MasonrySpec {
    fn limit(&self) -> u64 {
        self.limit.unwrap_or(100)
    }
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(spec): Query<MasonrySpec>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let captures = state
        .user_api
        .get_timeline_captures(&context_user, spec.limit())
        .await?;

    let images: Vec<MasonryImage> = captures
        .into_iter()
        .filter_map(|capture| {
            let capture_id = capture.id;
            capture.medias.into_iter().next().map(|media| MasonryImage {
                capture_id,
                url: media.url,
            })
        })
        .collect();

    let mut context = state.template_context();
    context.insert("images", &images);

    let rendered = state
        .tera
        .render("masonry.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
