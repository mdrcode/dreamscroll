use anyhow;

pub trait DataObject: Send + Sync {
    fn data_object_id(&self) -> String;
    fn data_object_json(&self) -> anyhow::Result<serde_json::Map<String, serde_json::Value>>;
}
