use serde::{Deserialize, Serialize};

use crate::model;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkLinkInfo {
    pub id: i32,
    pub url: String,
    pub commentary: String,
}

impl From<model::spark_link::ModelEx> for SparkLinkInfo {
    fn from(m: model::spark_link::ModelEx) -> Self {
        Self {
            id: m.id,
            url: m.url,
            commentary: m.commentary,
        }
    }
}
