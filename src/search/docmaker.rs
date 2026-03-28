use crate::api;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HybridSearchDocument {
    pub capture_id: i32,
    pub illumination_id: i32,
    pub text: String,
}

pub fn make_text_doc(capture: &api::CaptureInfo) -> anyhow::Result<HybridSearchDocument> {
    let illumination = capture.illuminations.first().ok_or_else(|| {
        anyhow::anyhow!("Capture has no illumination; search embedding requires text context")
    })?;

    let text = format_hybrid_document_text(illumination);

    if text.trim().is_empty() {
        anyhow::bail!(
            "Illumination text is empty for capture {}; cannot build search document",
            capture.id
        );
    }

    Ok(HybridSearchDocument {
        capture_id: capture.id,
        illumination_id: illumination.id,
        text,
    })
}

/// Generates the document-side text for retrieval embedding. This format follows
/// Gemini Embeddings guidance for asymmetric retrieval use-cases.
pub fn format_hybrid_document_text(illumination: &api::IlluminationInfo) -> String {
    let knodes = illumination
        .k_nodes
        .iter()
        .map(|k| k.name.clone())
        .collect::<Vec<_>>()
        .join(", ");

    let social_handles = illumination
        .social_medias
        .iter()
        .map(|s| format!("{} {}", s.display_name, s.handle))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "task: search result | title: capture-summary | text: {}\n\n{}\n\nEntities: {}\nSuggested queries: {}\nSocial: {}",
        illumination.summary,
        illumination.details,
        knodes,
        illumination.x_queries.join(", "),
        social_handles,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_hybrid_text_contains_main_fields() {
        let info = api::IlluminationInfo {
            id: 10,
            capture_id: 9,
            summary: "summary".to_string(),
            details: "details".to_string(),
            x_queries: vec!["a".to_string(), "b".to_string()],
            k_nodes: vec![],
            social_medias: vec![],
        };

        let text = format_hybrid_document_text(&info);
        assert!(text.contains("task: search result"));
        assert!(text.contains("summary"));
        assert!(text.contains("details"));
        assert!(text.contains("a, b"));
    }
}
