use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, Deserialize)]
pub struct SparksQuery {
    pub id: Option<i32>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<SparksQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let user_context = user.into();

    let mut sparks = state.user_api.get_sparks(&user_context, None).await?;
    sparks.sort_by(|a, b| b.id.cmp(&a.id));

    let selected_spark_id = match query.id {
        Some(id) => Some(id),
        None => sparks.first().map(|s| s.id),
    };

    let selected_spark = selected_spark_id
        .and_then(|id| sparks.iter().find(|s| s.id == id))
        .cloned();

    let mut context = state.template_context();
    context.insert("sparks", &sparks);
    context.insert("selected_spark", &selected_spark);
    context.insert("selected_spark_id", &selected_spark_id);

    let rendered = state
        .tera
        .render("sparks.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
