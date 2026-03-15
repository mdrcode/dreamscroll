use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn append_captures_to_user_prompt(
    prompt: &str,
    captures: Vec<crate::api::CaptureInfo>,
) -> String {
    let captures_section = captures
        .iter()
        .enumerate()
        .filter(|(_, capture)| {
            if capture.illuminations.is_empty() {
                tracing::warn!(
                    capture_id = capture.id,
                    "Spark ignoring capture with no illuminations"
                );
                false
            } else {
                true
            }
        })
        .map(|(idx, capture)| {
            let illumination = capture.illuminations.first();
            let summary = illumination
                .map(|it| it.summary.as_str())
                .unwrap_or("(no summary available)");
            let details = illumination
                .map(|it| it.details.as_str())
                .unwrap_or("(no details available)");

            format!(
                "{}. Capture ID {}\nSummary: {}\nDetails: {}",
                idx + 1,
                capture.id,
                summary,
                details,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "{}\n\nRecent captures and summaries:\n\n{}",
        prompt, captures_section
    )
}

pub fn truncate_for_error(text: &str, max_chars: usize) -> String {
    let mut out = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[allow(dead_code)]
pub fn dump_raw_response_to_tmp(content: &str, file_stem: &str) -> anyhow::Result<PathBuf> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = std::env::temp_dir().join(format!("{}-{}.json", file_stem, ts));
    fs::write(&path, content)?;
    Ok(path)
}
