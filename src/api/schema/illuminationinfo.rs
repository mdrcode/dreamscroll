use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub summary: String,
    pub details: String,
    pub x_queries: Vec<String>,
    pub k_nodes: Vec<KNodeInfo>,
    pub social_medias: Vec<SocialMediaInfo>,
}

impl IlluminationInfo {
    pub fn make_text(&self) -> String {
        let knodes = self
            .k_nodes
            .iter()
            .map(|k| k.name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        let social_handles = self
            .social_medias
            .iter()
            .map(|s| format!("{} {}", s.display_name, s.handle))
            .collect::<Vec<_>>()
            .join(", ");

        let text = format!(
            "task: search result | title: capture-summary | text: {}\n\n{}\n\nEntities: {}\nSuggested queries: {}\nSocial: {}",
            self.summary,
            self.details,
            knodes,
            self.x_queries.join(", "),
            social_handles,
        );

        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_text_contains_main_fields() {
        let info = IlluminationInfo {
            id: 10,
            capture_id: 9,
            summary: "summary".to_string(),
            details: "details".to_string(),
            x_queries: vec!["a".to_string(), "b".to_string()],
            k_nodes: vec![],
            social_medias: vec![],
        };

        let text = info.make_text();
        assert!(text.contains("task: search result"));
        assert!(text.contains("summary"));
        assert!(text.contains("details"));
        assert!(text.contains("a, b"));
    }
}
