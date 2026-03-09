use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, webhook};

use super::*;

/// Webhook POST route for illumination task invoked by PubSub.
///
/// There is NO AUTHENTICATION here, it should be enforced externally by GCloud ADC.
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(body): Json<pubsub::PushBody>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task = pubsub::decode_message_data::<webhook::logic::illuminate::IlluminationTask>(
        &body.message.data,
    )
    .map_err(|err| {
        tracing::error!(error = ?err, "Failed to decode Pub/Sub message task");
        api::ApiError::bad_request(err)
    })?;

    webhook::logic::illuminate::exec(&state.service_api, &state.illuminator, task).await?;

    Ok(StatusCode::NO_CONTENT)
}
