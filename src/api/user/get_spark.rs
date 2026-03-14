use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

pub async fn get_sparks(
    db: &DbHandle,
    context: &auth::Context,
    spark_ids: Option<Vec<i32>>,
) -> Result<Vec<model::spark::ModelEx>, ApiError> {
    let mut loader =
        model::spark::Entity::load().filter(model::spark::Column::UserId.eq(context.user_id()));

    if let Some(ids) = &spark_ids {
        loader = loader.filter(model::spark::Column::Id.is_in(ids.clone()));
    }

    let loader = loader
        .with(model::spark_cluster::Entity)
        .with((model::spark_cluster::Entity, model::spark_link::Entity))
        .with((
            model::spark_cluster::Entity,
            model::spark_cluster_ref::Entity,
        ))
        .all(&db.conn)
        .await?;

    Ok(loader)
}
