use serde::{Deserialize, Serialize};

/// Version selected for one illumination task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IlluminationVersion {
    #[default]
    V1,
    V2,
}

impl IlluminationVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_versions_as_lowercase_labels() {
        assert_eq!(IlluminationVersion::V1.as_str(), "v1");
        assert_eq!(IlluminationVersion::V2.as_str(), "v2");
        assert_eq!(
            serde_json::to_value(IlluminationVersion::V1).unwrap(),
            serde_json::json!(IlluminationVersion::V1.as_str())
        );
        assert_eq!(
            serde_json::to_value(IlluminationVersion::V2).unwrap(),
            serde_json::json!(IlluminationVersion::V2.as_str())
        );
    }
}
