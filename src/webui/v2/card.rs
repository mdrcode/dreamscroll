use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::{api, auth};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedCard {
    Capture(CaptureCard),
    Spark(SparkCard),
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureCard {
    pub capture: api::CaptureInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct SparkCard {
    pub spark: api::SparkInfo,
    pub clusters: Vec<SparkClusterCard>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SparkClusterCard {
    pub cluster: api::SparkClusterInfo,
    pub capture_thumbs: Vec<SparkCaptureThumb>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SparkCaptureThumb {
    pub id: i32,
    pub url: String,
    pub summary: String,
}

pub fn cards_from_captures(captures: Vec<api::CaptureInfo>) -> Vec<FeedCard> {
    captures
        .into_iter()
        .map(|capture| FeedCard::Capture(CaptureCard { capture }))
        .collect()
}

pub async fn load_spark_cards(
    user_api: &api::UserApiClient,
    context: &auth::Context,
    limit: u64,
) -> Result<Vec<FeedCard>, api::ApiError> {
    let mut sparks = user_api.get_sparks(context, None).await?;
    sparks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let selected_sparks: Vec<api::SparkInfo> = sparks.into_iter().take(limit as usize).collect();

    let referenced_capture_ids: Vec<i32> = selected_sparks
        .iter()
        .flat_map(|spark| {
            spark
                .spark_clusters
                .iter()
                .flat_map(|cluster| cluster.referenced_capture_ids.iter().copied())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let captures = if referenced_capture_ids.is_empty() {
        vec![]
    } else {
        user_api
            .get_captures(context, Some(referenced_capture_ids))
            .await?
    };

    let capture_thumb_map: HashMap<i32, SparkCaptureThumb> = captures
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
        .collect();

    Ok(selected_sparks
        .into_iter()
        .map(|spark| {
            let clusters = spark
                .spark_clusters
                .iter()
                .map(|cluster| SparkClusterCard {
                    cluster: cluster.clone(),
                    capture_thumbs: cluster
                        .referenced_capture_ids
                        .iter()
                        .filter_map(|capture_id| capture_thumb_map.get(capture_id).cloned())
                        .collect(),
                })
                .collect();

            FeedCard::Spark(SparkCard { spark, clusters })
        })
        .collect())
}

pub fn blend_capture_and_spark_cards(
    capture_cards: Vec<FeedCard>,
    spark_cards: Vec<FeedCard>,
) -> Vec<FeedCard> {
    let mut cards = Vec::with_capacity(capture_cards.len() + spark_cards.len());
    cards.extend(capture_cards);
    cards.extend(spark_cards);
    cards.sort_by(|a, b| feed_card_created_at(b).cmp(&feed_card_created_at(a)));
    cards
}

fn feed_card_created_at(card: &FeedCard) -> chrono::DateTime<chrono::Utc> {
    match card {
        FeedCard::Capture(capture_card) => capture_card.capture.created_at,
        FeedCard::Spark(spark_card) => spark_card.spark.created_at,
    }
}
