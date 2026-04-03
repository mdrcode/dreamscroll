use serde::{Deserialize, Serialize};

use crate::task;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IlluminationTask {
    pub capture_id: i32,
}

impl task::TaskId for IlluminationTask {
    fn id(&self) -> String {
        self.capture_id.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexTask {
    pub capture_id: i32,
}

impl task::TaskId for SearchIndexTask {
    fn id(&self) -> String {
        self.capture_id.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkTask {
    pub capture_ids: Vec<i32>,
}

impl task::TaskId for SparkTask {
    fn id(&self) -> String {
        self.capture_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("-")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestTask {
    pub capture_id: i32,
}

impl task::TaskId for IngestTask {
    fn id(&self) -> String {
        self.capture_id.to_string()
    }
}