use crate::{api, database::DbHandle, illumination::*, model};

// Is this still needed?
pub async fn insert_illumination(
    db: &DbHandle,
    capture_id: i32,
    illumination: Illumination,
) -> Result<(), api::ApiError> {
    let raw_content = format!("{}\n{}", illumination.summary, illumination.details);

    let mut builder = model::illumination::ActiveModel::builder()
        .set_capture_id(capture_id)
        .set_provider_name(illumination.meta.provider_name)
        .set_summary(illumination.summary)
        .set_details(illumination.details)
        .set_raw_content(raw_content);

    for entity in illumination.entities {
        let k_node_builder = model::k_node::ActiveModel::builder()
            .set_name(entity.name)
            .set_description(entity.description)
            .set_k_type(entity.entity_type.to_string());
        builder.k_nodes.push(k_node_builder);
    }

    for ss in illumination.suggested_searches {
        let x_query_builder = model::x_query::ActiveModel::builder().set_query(ss);
        builder.x_queries.push(x_query_builder);
    }

    builder.save(&db.conn).await?;

    Ok(())
}
