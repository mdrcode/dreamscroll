use std::sync::Arc;

use axum::{extract::State, response::Html};
use sea_orm::{QueryOrder, entity::prelude::*};
use tera::Context;

use crate::entity::{capture, media};
use crate::webui::{WebState, prelude::*};

pub async fn index(State(state): State<Arc<WebState>>) -> Html<String> {
    let captures = capture::Entity::find()
        .order_by(capture::Column::CreatedAt, sea_orm::Order::Desc)
        .find_with_related(media::Entity)
        .all(&state.db.conn)
        .await
        .expect("Failed to capture fetches from db.");

    let capture_medias = captures
        .into_iter()
        .map(|(capture, medias)| CaptureInfo { capture, medias })
        .collect::<Vec<_>>();

    let mut context = Context::new();
    context.insert("capture_medias", &capture_medias);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered)
}
