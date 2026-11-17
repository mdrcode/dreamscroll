//! Projection: the derived columns of a knowledge entity, and its retrieval material.
//!
//! One function writes `name`, `alt_names`, `description`, `alt_descriptions`, `search_query`,
//! `alt_search_queries`, `platform`, `platform_handle`, `platform_display_name`, `status` and
//! `confidence`, and the entity's `knowledge_blocking_index` row. Nothing else writes them after
//! create.
//!
//! The entity **wears the values of its origin** — the member it was created from — and every other
//! member contributes alternatives. That is what makes a merge safe: the winner keeps its own surface
//! and text while the loser's arrive as `alt_*`.

use std::collections::{HashMap, HashSet};

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};

use crate::model;

/// Recompute the columns that are a function of the members and of the evidence.
pub(super) async fn project<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let row = get(conn, knowledge_entity_id).await?;

    // Ordered by member id, so the alternatives come out in the order the members joined.
    let members = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(conn)
        .await?;

    // Re-illuminating a capture appends a reading without removing the old one, so a member can be a
    // superseded occurrence. Project from a member of its capture's *newest* illumination.
    let mut newest: HashMap<i32, i32> = HashMap::new();
    for member in &members {
        newest
            .entry(member.capture_id)
            .and_modify(|current| *current = (*current).max(member.illumination_id))
            .or_insert(member.illumination_id);
    }
    let current = |member: &model::user_observed_entity::Model| {
        newest.get(&member.capture_id).copied() == Some(member.illumination_id)
    };

    // An entity that predates the column, or whose origin is gone, falls back to its first member —
    // but not to a superseded reading while a current one exists.
    let recorded = row
        .origin_observation_id
        .and_then(|id| members.iter().find(|member| member.id == id));
    let origin = recorded
        .filter(|member| current(member))
        .or_else(|| members.iter().find(|member| current(member)))
        .or(recorded)
        .or_else(|| members.first());

    let surfaces = gather(
        &members,
        |member| surfaces(member).collect(),
        crate::knowledge::text::comparable,
    );
    let types = gather(
        &members,
        |member| vec![member.entity_type.as_str()],
        str::to_string,
    );
    let descriptions = gather(
        &members,
        |member| member.description.as_deref().into_iter().collect(),
        str::to_string,
    );
    let queries = gather(
        &members,
        |member| member.search_query.as_deref().into_iter().collect(),
        str::to_string,
    );

    let (name, alt_names) = select(origin.map(|member| member.name.trim()), &surfaces);
    let (entity_type, alt_types) = select(origin.map(|member| member.entity_type.trim()), &types);

    // When no member offers text, keep what is there: an identity resolver-sourced description must
    // not be wiped by a member list that says nothing.
    let (description, alt_descriptions) = match select(
        origin.and_then(|member| member.description.as_deref()),
        &descriptions,
    ) {
        (Some(text), alt) => (Some(text), alt),
        (None, _) => (row.description.clone(), row.alt_descriptions.clone()),
    };
    let (search_query, alt_search_queries) = match select(
        origin.and_then(|member| member.search_query.as_deref()),
        &queries,
    ) {
        (Some(text), alt) => (Some(text), alt),
        (None, _) => (row.search_query.clone(), row.alt_search_queries.clone()),
    };

    // The presence the resolver asks by: the origin's if it showed one, else the first member's.
    let presence = origin
        .filter(|member| member.platform.is_some())
        .or_else(|| members.iter().find(|member| member.platform.is_some()));
    let (platform, platform_handle, platform_display_name) = match presence {
        Some(member) => (
            member.platform.clone(),
            member.platform_handle.clone(),
            member.platform_display_name.clone(),
        ),
        None => (
            row.platform.clone(),
            row.platform_handle.clone(),
            row.platform_display_name.clone(),
        ),
    };

    let identities: Vec<Option<f32>> = model::knowledge_identity::Entity::find()
        .filter(model::knowledge_identity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .select_only()
        .column(model::knowledge_identity::Column::Confidence)
        .into_tuple::<Option<f32>>()
        .all(conn)
        .await?;
    let status = model::knowledge_entity::Status::of(identities.len());
    let confidence = best_confidence(&identities);

    let updated = model::knowledge_entity::Entity::update(model::knowledge_entity::ActiveModel {
        id: Set(knowledge_entity_id),
        name: Set(name.unwrap_or_else(|| row.name.clone())),
        alt_names: Set(alt_names),
        entity_type: Set(entity_type.unwrap_or_else(|| row.entity_type.clone())),
        alt_types: Set(alt_types),
        description: Set(description),
        alt_descriptions: Set(alt_descriptions),
        search_query: Set(search_query),
        alt_search_queries: Set(alt_search_queries),
        platform: Set(platform),
        platform_handle: Set(platform_handle),
        platform_display_name: Set(platform_display_name),
        status: Set(status),
        confidence: Set(confidence),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    })
    .exec(conn)
    .await?;

    // The index row is a function of these columns and the current fold, so it moves with them.
    crate::knowledge2::blocker::index::write(conn, &updated).await?;
    Ok(updated)
}

async fn get<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("knowledge entity {knowledge_entity_id} does not exist"))
}

fn surfaces(observation: &model::user_observed_entity::Model) -> impl Iterator<Item = &str> {
    [
        Some(observation.name.as_str()),
        observation.platform_handle.as_deref(),
        observation.platform_display_name.as_deref(),
    ]
    .into_iter()
    .flatten()
}

/// Every distinct, non-empty value the members offer, in member order.
fn gather<'a>(
    members: &'a [model::user_observed_entity::Model],
    values: impl Fn(&'a model::user_observed_entity::Model) -> Vec<&'a str>,
    key: impl Fn(&str) -> String,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut distinct = Vec::new();
    for member in members {
        for value in values(member) {
            let text = value.trim();
            if !text.is_empty() && seen.insert(key(text)) {
                distinct.push(text.to_string());
            }
        }
    }
    distinct
}

/// Keep `preferred` while the members still offer it; otherwise take the first. Everything else
/// becomes the alternative list.
fn select(preferred: Option<&str>, candidates: &[String]) -> (Option<String>, Vec<String>) {
    let selected = preferred
        .filter(|text| candidates.iter().any(|candidate| candidate == text))
        .map(str::to_string)
        .or_else(|| candidates.first().cloned());
    let alternatives = candidates
        .iter()
        .filter(|candidate| Some(candidate.as_str()) != selected.as_deref())
        .cloned()
        .collect();
    (selected, alternatives)
}

/// Which entities a reprojection pass walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Live entities holding no identity annotation — what a deletion can leave stale.
    Stale,
    /// Every live entity — what a change to the projection rule needs.
    All,
}

/// Recompute the projection for a scope, and report how many entities it wrote.
pub async fn reproject(
    conn: &sea_orm::DatabaseConnection,
    scope: Scope,
) -> anyhow::Result<usize> {
    use sea_orm::sea_query::{Expr, ExprTrait, Query};

    use crate::model::knowledge_entity as entity;
    use crate::model::knowledge_identity as identities;

    let mut query = entity::Entity::find().filter(entity::Column::MergedIntoId.is_null());
    if scope == Scope::Stale {
        let annotated = Query::select()
            .distinct()
            .column((identities::Entity, identities::Column::KnowledgeEntityId))
            .from(identities::Entity)
            .to_owned();
        query = query.filter(Expr::col(entity::Column::Id).not_in_subquery(annotated));
    }
    let rows = query.all(conn).await?;
    let total = rows.len();
    for row in &rows {
        project(conn, row.id).await?;
    }
    Ok(total)
}

/// Confidence is the best identity's claim, not a sum or a bonus; one with no score adds nothing.
fn best_confidence(identities: &[Option<f32>]) -> Option<f32> {
    identities
        .iter()
        .filter_map(|identity| *identity)
        .max_by(f32::total_cmp)
}
