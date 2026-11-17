//! A canonical, id-independent view of the graph, for v1/v2 comparison and diffs.
//!
//! Row ids, timestamps, and user ownership are excluded. An entity is keyed by its origin
//! observation, which survives a rebuild; every section is a sorted list of lines, so two runs are
//! a plain text diff.

use std::collections::HashMap;

use sea_orm::EntityTrait;

use crate::database::DbHandle;
use crate::model;

/// One graph, in four sorted sections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shape {
    pub entities: Vec<String>,
    pub identities: Vec<String>,
    pub memberships: Vec<String>,
    pub index: Vec<String>,
}

impl Shape {
    pub fn render(&self) -> String {
        let mut out = String::new();
        section(&mut out, "entities", &self.entities);
        section(&mut out, "identities", &self.identities);
        section(&mut out, "memberships", &self.memberships);
        section(&mut out, "index", &self.index);
        out
    }
}

fn section(out: &mut String, name: &str, lines: &[String]) {
    out.push_str(&format!("# {name} ({})\n", lines.len()));
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
}

pub async fn snapshot(db: &DbHandle) -> anyhow::Result<Shape> {
    let entities = model::knowledge_entity::Entity::find()
        .all(&db.conn)
        .await?;
    let origins: HashMap<i32, Option<i32>> = entities
        .iter()
        .map(|entity| (entity.id, entity.origin_observation_id))
        .collect();
    let origin = |entity_id: i32| origins.get(&entity_id).copied().flatten();

    let mut shape = Shape::default();

    shape.entities = entities
        .iter()
        .map(|entity| {
            format!(
                "entity origin={:?} name={:?} type={:?} alt_names={:?} alt_types={:?} \
                 description={:?} alt_descriptions={:?} search={:?} alt_search={:?} \
                 status={:?} confidence={} platform={:?} handle={:?} display={:?} \
                 merged_into={:?}",
                entity.origin_observation_id,
                entity.name,
                entity.entity_type,
                entity.alt_names,
                entity.alt_types,
                entity.description,
                entity.alt_descriptions,
                entity.search_query,
                entity.alt_search_queries,
                entity.status,
                entity.confidence.is_some(),
                entity.platform,
                entity.platform_handle,
                entity.platform_display_name,
                entity.merged_into_id.map(origin),
            )
        })
        .collect();
    shape.entities.sort();

    shape.identities = model::knowledge_identity::Entity::find()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|identity| {
            format!(
                "identity origin={:?} provider={:?} scheme={:?} external={:?} method={:?} \
                 confidence={} label={:?} description={:?} classes={:?} provenance={:?}",
                origin(identity.knowledge_entity_id),
                identity.provider,
                identity.scheme,
                identity.external_id,
                identity.method,
                identity.confidence.is_some(),
                identity.label,
                identity.description,
                identity.classes,
                identity.user_observed_entity_id,
            )
        })
        .collect();
    shape.identities.sort();

    shape.memberships = model::user_observed_entity::Entity::find()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|observation| {
            format!(
                "member origin={:?} observation={} name={:?}",
                observation.knowledge_entity_id.map(origin),
                observation.id,
                observation.name,
            )
        })
        .collect();
    shape.memberships.sort();

    shape.index = model::knowledge_blocking_index::Entity::find()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|row| {
            format!(
                "index origin={:?} surfaces={:?} word_keys={:?} text={:?} built_with={:?}",
                origin(row.knowledge_entity_id),
                row.surfaces,
                row.word_keys,
                row.text,
                row.built_with,
            )
        })
        .collect();
    shape.index.sort();

    Ok(shape)
}
