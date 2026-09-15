use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};

use crate::{api, logic, webhook};

use super::*;

/// Webhook POST route for illumination task invoked by PubSub.
///
/// There is NO AUTHENTICATION here, it should be enforced externally by GCloud ADC.
pub async fn post(
    State(state): State<Arc<webhook::WebhookState>>,
    Json(body): Json<schema::PushBody>,
) -> Result<impl IntoResponse, api::ApiError> {
    let task =
        schema::decode_message_data::<logic::illuminate::IlluminationTask>(&body.message.data)
            .map_err(|err| {
                tracing::error!(error = ?err, "Failed to decode Pub/Sub message task");
                api::ApiError::bad_request(err)
            })?;

    logic::illuminate::exec(&state.service_api, state.illuminator.as_ref(), task).await?;

    Ok(StatusCode::NO_CONTENT)
}
