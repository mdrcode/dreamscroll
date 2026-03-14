use crate::{api::*, database::DbHandle, ignition, model};

pub async fn insert_spark(
    db: &DbHandle,
    user_id: i32,
    spark: ignition::SparkResponse,
    input_capture_ids: Vec<i32>,
) -> Result<(), ApiError> {
    let spark_row = model::spark::ActiveModel::builder()
        .set_user_id(user_id)
        .insert(&db.conn)
        .await?;

    for (idx, capture_id) in input_capture_ids.iter().enumerate() {
        model::spark_input_ref::ActiveModel::builder()
            .set_user_id(user_id)
            .set_spark_id(spark_row.id)
            .set_capture_id(*capture_id)
            .set_position(idx as i32)
            .save(&db.conn)
            .await?;
    }

    for cluster in &spark.clusters {
        let cluster_row = model::spark_cluster::ActiveModel::builder()
            .set_user_id(user_id)
            .set_spark_id(spark_row.id)
            .set_title(&cluster.title)
            .set_summary(&cluster.summary)
            .insert(&db.conn)
            .await?;

        for capture_id in &cluster.capture_ids {
            model::spark_output_ref::ActiveModel::builder()
                .set_user_id(user_id)
                .set_spark_id(spark_row.id)
                .set_spark_cluster_id(cluster_row.id)
                .set_capture_id(*capture_id)
                .save(&db.conn)
                .await?;
        }

        for link in &cluster.recommended_links {
            model::spark_link::ActiveModel::builder()
                .set_user_id(user_id)
                .set_spark_cluster_id(cluster_row.id)
                .set_url(&link.url)
                .set_commentary(&link.commentary)
                .save(&db.conn)
                .await?;
        }
    }

    Ok(())
}
