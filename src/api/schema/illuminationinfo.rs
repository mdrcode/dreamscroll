use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IlluminationInfo {
    pub id: i32,
    pub capture_id: i32,
    pub summary: String,
    pub details: String,
    pub suggested_searches: Vec<String>,
    pub knowledge_entities: Vec<KnowledgeEntityInfo>,
}

impl IlluminationInfo {
    pub fn make_text(&self) -> String {
        let mut sections = Vec::new();
        push_section(&mut sections, "Summary", &self.summary);
        push_section(&mut sections, "Details", &self.details);

        let entity_lines = self
            .knowledge_entities
            .iter()
            .map(|entity| {
                let mut parts = vec![entity.name.clone(), format!("type: {}", entity.entity_type)];
                if let Some(description) = &entity.description {
                    parts.push(format!("description: {description}"));
                }
                if let Some(search_query) = &entity.search_query {
                    parts.push(format!("canonical search: {search_query}"));
                }
                if let Some(link) = &entity.platform_link {
                    parts.push(format!("platform: {}", link.platform));
                    if let Some(display_name) = &link.display_name {
                        parts.push(format!("display name: {display_name}"));
                    }
                    parts.push(format!("handle: {}", link.handle));
                }
                format!("- {}", parts.join(" | "))
            })
            .collect::<Vec<_>>();
        if !entity_lines.is_empty() {
            sections.push(format!("Entities\n{}", entity_lines.join("\n")));
        }

        let search_lines = self
            .suggested_searches
            .iter()
            .map(|search| search.trim())
            .filter(|search| !search.is_empty())
            .map(|search| format!("- {search}"))
            .collect::<Vec<_>>();
        if !search_lines.is_empty() {
            sections.push(format!("Suggested searches\n{}", search_lines.join("\n")));
        }

        sections.join("\n\n")
    }
}

fn push_section(sections: &mut Vec<String>, header: &str, body: &str) {
    let body = body.trim();
    if !body.is_empty() {
        sections.push(format!("{header}\n{body}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_text_includes_unified_entity_details_and_suggested_searches() {
        let info = IlluminationInfo {
            id: 10,
            capture_id: 9,
            summary: "summary".to_string(),
            details: "details".to_string(),
            suggested_searches: vec!["related topic".to_string()],
            knowledge_entities: vec![KnowledgeEntityInfo {
                id: 1,
                capture_id: 9,
                illumination_id: 10,
                capture_archived_at: None,
                name: "Ada Lovelace".to_string(),
                entity_type: "real_person".to_string(),
                description: Some("Pioneer of computing".to_string()),
                search_query: Some("Ada Lovelace mathematician".to_string()),
                platform_link: Some(PlatformLinkInfo {
                    platform: "x_twitter".to_string(),
                    handle: "@ada".to_string(),
                    display_name: Some("Ada".to_string()),
                }),
            }],
        };
        let payload = serde_json::to_value(&info).unwrap();
        assert_eq!(
            payload["suggested_searches"],
            serde_json::json!(["related topic"])
        );
        assert!(payload.get("x_queries").is_none());

        let text = info.make_text();
        assert!(text.contains("Summary\nsummary"));
        assert!(text.contains("Details\ndetails"));
        assert!(text.contains("Entities"));
        assert!(text.contains("type: real_person"));
        assert!(text.contains("description: Pioneer of computing"));
        assert!(text.contains("canonical search: Ada Lovelace mathematician"));
        assert!(text.contains("handle: @ada"));
        assert!(text.contains("Suggested searches\n- related topic"));
    }

    #[test]
    fn make_text_omits_empty_sections() {
        let info = IlluminationInfo {
            id: 10,
            capture_id: 9,
            summary: "summary".to_string(),
            details: "  ".to_string(),
            suggested_searches: vec![],
            knowledge_entities: vec![],
        };

        let text = info.make_text();
        assert!(text.contains("Summary"));
        assert!(!text.contains("Details"));
        assert!(!text.contains("Entities"));
        assert!(!text.contains("Suggested searches"));
    }
}
