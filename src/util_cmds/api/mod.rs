mod state;
pub use state::*;

/// Print a command response as pretty JSON. This is the `--raw-json`
/// convention: terminal output by default, the underlying response on demand.
pub fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Parse capture ID tokens, expanding ranges like `205-209`.
pub fn parse_capture_ids(tokens: &[String]) -> anyhow::Result<Vec<i32>> {
    let mut parsed_ids = Vec::new();

    for token in tokens {
        let trimmed = token.trim();

        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("Capture ID argument cannot be empty."));
        }

        if let Some((start_raw, end_raw)) = trimmed.split_once('-') {
            let start = start_raw.parse::<i32>().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid range start '{}'. Expected an integer in '{}'.",
                    start_raw,
                    token
                )
            })?;
            let end = end_raw.parse::<i32>().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid range end '{}'. Expected an integer in '{}'.",
                    end_raw,
                    token
                )
            })?;

            if start <= end {
                parsed_ids.extend(start..=end);
            } else {
                parsed_ids.extend((end..=start).rev());
            }
        } else {
            let id = trimmed.parse::<i32>().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid capture ID '{}'. Use an integer or range like 205-209.",
                    token
                )
            })?;
            parsed_ids.push(id);
        }
    }

    Ok(parsed_ids)
}

pub mod token_cache;

pub mod backfill;
pub mod captures;
pub mod change_password;
pub mod clear_token;
pub mod entities;
pub mod export_digest;
pub mod illuminate;
pub mod import_digest;
pub mod search;
pub mod search_similar;
pub mod spark;
pub mod throughput;
