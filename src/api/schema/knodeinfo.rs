use serde::{Deserialize, Serialize};

use crate::model;

fn normalize_entity_type_label(raw: &str) -> String {
    // TODO: Remove this mapping once upstream entity typing stops emitting "unknown"
    // for user-facing data and we have a canonical display label policy.
    // Temporary presentation hack: avoid exposing raw "unknown" in UI.
    if raw.trim().eq_ignore_ascii_case("unknown") {
        return "entity".to_string();
    }

    raw.to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
            k_type: normalize_entity_type_label(&mx.k_type),
        }
    }
}
