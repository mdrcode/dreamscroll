use serde::Serialize;

use crate::api;

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
    pub spark_id: i32,
    pub title: String,
    pub summary: String,
    pub cluster_count: usize,
    pub input_capture_count: usize,
    pub sparked_by: Option<String>,
}

pub fn cards_from_captures(captures: Vec<api::CaptureInfo>) -> Vec<FeedCard> {
    captures
        .into_iter()
        .map(|capture| FeedCard::Capture(CaptureCard { capture }))
        .collect()
}

pub fn cards_from_sparks(mut sparks: Vec<api::SparkInfo>, limit: usize) -> Vec<FeedCard> {
    sparks.sort_by(|a, b| b.id.cmp(&a.id));

    sparks
        .into_iter()
        .take(limit)
        .map(|spark| {
            let title = format!("Spark {}", spark.id);
            let summary = spark
                .spark_clusters
                .first()
                .map(|cluster| cluster.summary.clone())
                .unwrap_or_else(|| {
                    "Spark is available, but no summary has been generated yet.".to_string()
                });
            let sparked_by = spark.meta.map(|meta| meta.provider_name);

            FeedCard::Spark(SparkCard {
                spark_id: spark.id,
                title,
                summary,
                cluster_count: spark.spark_clusters.len(),
                input_capture_count: spark.input_capture_ids.len(),
                sparked_by,
            })
        })
        .collect()
}

pub fn blend_capture_and_spark_cards(
    capture_cards: Vec<FeedCard>,
    spark_cards: Vec<FeedCard>,
) -> Vec<FeedCard> {
    if spark_cards.is_empty() {
        return capture_cards;
    }

    let mut blended = Vec::with_capacity(capture_cards.len() + spark_cards.len());
    let mut spark_iter = spark_cards.into_iter();

    for (index, card) in capture_cards.into_iter().enumerate() {
        blended.push(card);

        // Keep the blend predictable: one spark card after every 4 capture cards.
        if (index + 1) % 4 == 0 {
            if let Some(spark_card) = spark_iter.next() {
                blended.push(spark_card);
            }
        }
    }

    blended.extend(spark_iter);
    blended
}
