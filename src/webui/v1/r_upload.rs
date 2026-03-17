use std::sync::Arc;

use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use axum_login::{AuthSession, AuthUser};

use crate::{api, auth};

use super::WebState;

pub async fn post(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    multipart: Multipart,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let user_id = user.id();
    let context_user = user.into();

    tracing::info!("Processing upload for user ID {}", user_id);
    let cap = crate::webui::upload::insert_capture_from_multipart(
        &state.user_api,
        &context_user,
        multipart,
    )
    .await?;
    tracing::info!("Capture {} inserted via upload", cap.id);

    // Redirect to home page to show the timeline
    Ok(Redirect::to("/v1").into_response())
}
