use serde::{Deserialize, Serialize};

use crate::{api, auth};

#[derive(Debug, Deserialize)]
pub(super) struct ContentSpec {
    #[serde(default)]
    pub query: String,
    pub limit: Option<u64>,
    pub content: Option<FeedContent>,
}

impl ContentSpec {
    pub(super) fn search_query(&self) -> &str {
        self.query.trim()
    }

    pub(super) fn is_search(&self) -> bool {
        !self.search_query().is_empty()
    }

    pub(super) fn limit(&self) -> u64 {
        self.limit.unwrap_or(50)
    }

    pub(super) fn content_mode(&self) -> FeedContent {
        self.content.unwrap_or(FeedContent::Blend)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum FeedContent {
    Blend,
    Captures,
    Sparks,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Card {
    Capture(CaptureCard),
    Spark(SparkCard),
}

impl Card {
    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        match self {
            Card::Capture(capture_card) => capture_card.capture.created_at,
            Card::Spark(spark_card) => spark_card.spark.created_at,
        }
    }
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
    pub capture_previews: Vec<api::CapturePreviewInfo>,
}

pub(super) async fn render_content(
    user_api: &api::UserApiClient,
    context_user: &auth::Context,
    spec: &ContentSpec,
) -> Result<Vec<Card>, api::ApiError> {
    if spec.is_search() {
        render_search(user_api, context_user, spec.search_query(), spec.limit()).await
    } else {
        render_timeline(user_api, context_user, spec.content_mode(), spec.limit()).await
    }
}

async fn render_search(
    user_api: &api::UserApiClient,
    context: &auth::Context,
    q: &str,
    limit: u64,
) -> Result<Vec<Card>, api::ApiError> {
    let capture_infos = user_api.search(context, q, Some(limit)).await?;
    Ok(cards_from_captures(capture_infos))
}

async fn render_timeline(
    user_api: &api::UserApiClient,
    context_user: &auth::Context,
    feed_mix: FeedContent,
    limit: u64,
) -> Result<Vec<Card>, api::ApiError> {
    match feed_mix {
        FeedContent::Sparks => {
            let sparks = user_api.get_timeline_sparks(context_user, limit).await?;
            Ok(cards_from_sparks(sparks))
        }
        FeedContent::Captures => {
            let capture_infos = user_api.get_timeline_captures(context_user, limit).await?;
            Ok(cards_from_captures(capture_infos))
        }
        FeedContent::Blend => {
            let capture_infos = user_api.get_timeline_captures(context_user, limit).await?;
            let capture_cards = cards_from_captures(capture_infos);
            let sparks = user_api.get_timeline_sparks(context_user, limit).await?;
            let spark_cards = cards_from_sparks(sparks);
            Ok(blend(capture_cards, spark_cards))
        }
    }
}

fn cards_from_captures(captures: Vec<api::CaptureInfo>) -> Vec<Card> {
    captures
        .into_iter()
        .map(|capture| Card::Capture(CaptureCard { capture }))
        .collect()
}

fn cards_from_sparks(sparks: Vec<api::SparkInfo>) -> Vec<Card> {
    sparks
        .into_iter()
        .map(|spark| {
            let clusters = spark
                .spark_clusters
                .iter()
                .map(|cluster| SparkClusterCard {
                    cluster: cluster.clone(),
                    capture_previews: cluster.capture_previews.clone(),
                })
                .collect();

            Card::Spark(SparkCard { spark, clusters })
        })
        .collect()
}

fn blend(capture_cards: Vec<Card>, spark_cards: Vec<Card>) -> Vec<Card> {
    let mut cards = Vec::with_capacity(capture_cards.len() + spark_cards.len());
    cards.extend(capture_cards);
    cards.extend(spark_cards);
    cards.sort_by_key(|card| std::cmp::Reverse(card.created_at()));
    cards
}
