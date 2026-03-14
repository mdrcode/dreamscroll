use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::{Deserialize, Serialize};

use crate::{api, auth};

use super::WebState;

#[derive(Debug, Deserialize)]
pub struct SparksQuery {
    pub id: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
struct SparkCaptureThumb {
    id: i32,
    url: String,
    summary: String,
}

#[derive(Debug, Clone, Serialize)]
struct SparkClusterView {
    id: i32,
    title: String,
    summary: String,
    referenced_capture_ids: Vec<i32>,
    spark_links: Vec<api::SparkLinkInfo>,
    capture_thumbs: Vec<SparkCaptureThumb>,
}

#[derive(Debug, Clone, Serialize)]
struct SparkView {
    id: i32,
    spark_clusters: Vec<SparkClusterView>,
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

    let selected_spark_view = if let Some(spark) = selected_spark {
        let referenced_capture_ids = spark
            .spark_clusters
            .iter()
            .flat_map(|cluster| cluster.referenced_capture_ids.iter().copied())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let captures = if referenced_capture_ids.is_empty() {
            vec![]
        } else {
            state
                .user_api
                .get_captures(&user_context, Some(referenced_capture_ids))
                .await?
        };

        let capture_thumb_map = captures
            .into_iter()
            .filter_map(|capture| {
                let summary = capture
                    .illuminations
                    .first()
                    .map(|illum| illum.summary.clone())
                    .unwrap_or_else(|| "No capture summary available.".to_string());

                capture.medias.first().map(|media| {
                    (
                        capture.id,
                        SparkCaptureThumb {
                            id: capture.id,
                            url: media.url.clone(),
                            summary,
                        },
                    )
                })
            })
            .collect::<std::collections::HashMap<_, _>>();

        Some(SparkView {
            id: spark.id,
            spark_clusters: spark
                .spark_clusters
                .into_iter()
                .map(|cluster| SparkClusterView {
                    id: cluster.id,
                    title: cluster.title,
                    summary: cluster.summary,
                    referenced_capture_ids: cluster.referenced_capture_ids.clone(),
                    spark_links: cluster.spark_links,
                    capture_thumbs: cluster
                        .referenced_capture_ids
                        .into_iter()
                        .filter_map(|capture_id| capture_thumb_map.get(&capture_id).cloned())
                        .collect(),
                })
                .collect(),
        })
    } else {
        None
    };

    let mut context = state.template_context();
    context.insert("sparks", &sparks);
    context.insert("selected_spark", &selected_spark_view);
    context.insert("selected_spark_id", &selected_spark_id);

    let rendered = state
        .tera
        .render("sparks.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
