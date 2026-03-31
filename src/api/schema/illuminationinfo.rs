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
        let mut sections: Vec<String> = Vec::new();

        push_section(&mut sections, "Summary", &self.summary);
        push_section(&mut sections, "Details", &self.details);

        let knode_lines = self
            .k_nodes
            .iter()
            .filter_map(|k| {
                let mut parts: Vec<String> = Vec::new();
                let name = k.name.trim();
                if !name.is_empty() {
                    parts.push(name.to_string());
                }

                let k_type = k.k_type.trim();
                if !k_type.is_empty() {
                    parts.push(format!("type: {}", k_type));
                }

                let description = k.description.trim();
                if !description.is_empty() {
                    parts.push(format!("description: {}", description));
                }

                if parts.is_empty() {
                    None
                } else {
                    Some(format!("- {}", parts.join(" | ")))
                }
            })
            .collect::<Vec<_>>();
        if !knode_lines.is_empty() {
            sections.push(format!("Knowledge Nodes\n{}", knode_lines.join("\n")));
        }

        let query_lines = self
            .x_queries
            .iter()
            .map(|q| q.trim())
            .filter(|q| !q.is_empty())
            .map(|q| format!("- {}", q))
            .collect::<Vec<_>>();
        if !query_lines.is_empty() {
            sections.push(format!("Suggested Queries\n{}", query_lines.join("\n")));
        }

        let social_lines = self
            .social_medias
            .iter()
            .filter_map(|s| {
                let mut parts: Vec<String> = Vec::new();

                let display_name = s.display_name.trim();
                if !display_name.is_empty() {
                    parts.push(display_name.to_string());
                }

                let platform = s.platform.trim();
                if !platform.is_empty() {
                    parts.push(format!("platform: {}", platform));
                }

                let handle = s.handle.trim();
                if !handle.is_empty() {
                    parts.push(format!("handle: {}", handle));
                }

                if parts.is_empty() {
                    None
                } else {
                    Some(format!("- {}", parts.join(" | ")))
                }
            })
            .collect::<Vec<_>>();
        if !social_lines.is_empty() {
            sections.push(format!("Social Profiles\n{}", social_lines.join("\n")));
        }

        sections.join("\n\n")
    }
}

fn push_section(sections: &mut Vec<String>, header: &str, body: &str) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    sections.push(format!("{}\n{}", header, body));
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
            k_nodes: vec![KNodeInfo {
                id: 1,
                name: "Ada Lovelace".to_string(),
                description: "Pioneer of computing".to_string(),
                k_type: "person".to_string(),
            }],
            social_medias: vec![SocialMediaInfo {
                id: 2,
                display_name: "Ada".to_string(),
                handle: "@ada".to_string(),
                platform: "x".to_string(),
            }],
        };

        let text = info.make_text();
        assert!(text.contains("Summary"));
        assert!(text.contains("summary"));
        assert!(text.contains("Details"));
        assert!(text.contains("details"));
        assert!(text.contains("Queries"));
        assert!(text.contains("- a"));
        assert!(text.contains("- b"));
        assert!(text.contains("Knowledge Nodes"));
        assert!(text.contains("type: person"));
        assert!(text.contains("description: Pioneer of computing"));
        assert!(text.contains("Social Profiles"));
        assert!(text.contains("platform: x"));
        assert!(text.contains("handle: @ada"));
    }

    #[test]
    fn make_text_omits_empty_sections() {
        let info = IlluminationInfo {
            id: 10,
            capture_id: 9,
            summary: "summary".to_string(),
            details: "  ".to_string(),
            x_queries: vec![],
            k_nodes: vec![],
            social_medias: vec![],
        };

        let text = info.make_text();
        assert!(text.contains("Summary"));
        assert!(!text.contains("Details"));
        assert!(!text.contains("Queries"));
        assert!(!text.contains("Knowledge Nodes"));
        assert!(!text.contains("Social Profiles"));
    }
}
