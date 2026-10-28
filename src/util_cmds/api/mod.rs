mod state;
pub use state::*;

/// Print a command response as pretty JSON. This is the `--raw-json`
/// convention: terminal output by default, the underlying response on demand.
pub fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub mod token_cache;

pub mod backfill;
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
