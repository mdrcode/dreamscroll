use serde::Deserialize;

/// Shared `?hours=` selector for windowed reports: comma-separated widths in
/// hours (e.g. `1,24,168`); absent means the default 24-hour window.
#[derive(Debug, Deserialize)]
pub struct HoursQuery {
    pub hours: Option<String>,
}
