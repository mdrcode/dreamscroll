use crate::{api::*, database::DbHandle, ignition, model};

pub async fn insert_spark(
    db: &DbHandle,
    user_id: i32,
    spark: ignition::SparkResponse,
) -> Result<(), ApiError> {
    let mut builder = model::spark::ActiveModel::builder().set_user_id(user_id);

    for cluster in &spark.clusters {
        let mut cluster_builder = model::spark_cluster::ActiveModel::builder()
            .set_user_id(user_id)
            .set_title(&cluster.title)
            .set_summary(&cluster.summary);

        for capture_id in &cluster.capture_ids {
            cluster_builder.spark_cluster_refs.push(
                model::spark_cluster_ref::ActiveModel::builder()
                    .set_user_id(user_id)
                    .set_capture_id(*capture_id),
            );
        }

        for link in &cluster.recommended_links {
            cluster_builder.spark_links.push(
                model::spark_link::ActiveModel::builder()
                    .set_user_id(user_id)
                    .set_url(&link.url)
                    .set_commentary(&link.commentary),
            );
        }

        builder.spark_clusters.push(cluster_builder);
    }

    builder.save(&db.conn).await?;

    Ok(())
}
