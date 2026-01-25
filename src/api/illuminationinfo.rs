use serde::Serialize;

use crate::model;

#[derive(Clone, Serialize)]
pub struct KNodeInfo {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub k_type: String,
}

impl From<model::k_node::ModelEx> for KNodeInfo {
    fn from(mx: model::k_node::ModelEx) -> Self {
        Self {
            id: mx.id,
            name: mx.name,
            description: mx.description,
            k_type: mx.k_type,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub summary: String,
    pub details: String,
    pub x_queries: Vec<String>,
    pub k_nodes: Vec<KNodeInfo>,
}

impl From<model::illumination::ModelEx> for IlluminationInfo {
    fn from(mx: model::illumination::ModelEx) -> Self {
        let x_queries = match mx.x_queries {
            sea_orm::prelude::HasMany::Unloaded => vec![],
            sea_orm::prelude::HasMany::Loaded(models) => {
                models.into_iter().map(|m| m.query).collect()
            }
        };

        let k_nodes = match mx.k_nodes {
            sea_orm::prelude::HasMany::Unloaded => vec![],
            sea_orm::prelude::HasMany::Loaded(models) => {
                models.into_iter().map(|m| KNodeInfo::from(m)).collect()
            }
        };

        Self {
            id: mx.id,
            capture_id: mx.capture_id,
            summary: mx.summary,
            details: mx.details,
            x_queries,
            k_nodes,
        }
    }
}
