use std::collections::HashMap;

use sea_orm::prelude::*;
use sea_orm::{QueryOrder, QuerySelect};

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
        .with(model::spark_input_ref::Entity)
        .with(model::spark_meta::Entity)
        .with((model::spark_cluster::Entity, model::spark_link::Entity))
        .with((
            model::spark_cluster::Entity,
            model::spark_output_ref::Entity,
        ))
        .all(&db.conn)
        .await?;

    Ok(loader)
}

pub async fn get_timeline_sparks(
    db: &DbHandle,
    context: &auth::Context,
    limit: u64,
) -> Result<Vec<model::spark::ModelEx>, ApiError> {
    if limit == 0 {
        return Ok(vec![]);
    }

    let spark_ids = model::spark::Entity::find()
        .filter(model::spark::Column::UserId.eq(context.user_id()))
        .order_by(model::spark::Column::CreatedAt, sea_orm::Order::Desc)
        .limit(limit)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|spark| spark.id)
        .collect::<Vec<i32>>();

    if spark_ids.is_empty() {
        return Ok(vec![]);
    }

    let sparks = get_sparks(db, context, Some(spark_ids.clone())).await?;
    let mut sparks_by_id: HashMap<i32, model::spark::ModelEx> =
        sparks.into_iter().map(|spark| (spark.id, spark)).collect();

    let ordered = spark_ids
        .into_iter()
        .filter_map(|spark_id| sparks_by_id.remove(&spark_id))
        .collect();

    Ok(ordered)
}
