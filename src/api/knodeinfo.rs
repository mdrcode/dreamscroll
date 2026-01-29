use serde::Serialize;

use crate::model;

#[derive(Clone, Debug, Serialize)]
pub struct KNodeInfo {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub k_type: String,
}

impl From<model::knode::ModelEx> for KNodeInfo {
    fn from(mx: model::knode::ModelEx) -> Self {
        Self {
            id: mx.id,
            name: mx.name,
            description: mx.description,
            k_type: mx.k_type,
        }
    }
}
