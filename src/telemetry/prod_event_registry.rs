//! The production event registry: the stable names behind Cloud Logging metrics and alerts.
//!
//! This file is an operational contract, not plumbing. A log-based metric filters on an event name,
//! so renaming a constant here silently stops the metric matching; changing the *shape* of its
//! fields silently breaks a label extractor or an alert condition. Treat edits here the way you
//! treat the runbook (`plan/prod-deploy.md`) and the metric definitions it documents.
//!
//! Names are lowercase, dotted and specific; the variable data lives in sibling fields, never in the
//! message text.
//!
//! ```ignore
//! tracing::warn!(
//!     event = crate::telemetry::prod_event_registry::PROVIDER_RATE_LIMITED,
//!     provider,
//!     kind = %kind,
//!     retry_after_secs = retry_after_secs.unwrap_or_default(),
//!     "provider rate limited"
//! );
//! ```
//!
//! [`GCloudLoggingFormat`](crate::telemetry) puts `event` and every sibling field in
//! `jsonPayload`, so a metric reads `jsonPayload.event="provider.rate_limited"` and can break down
//! by `jsonPayload.provider` without parsing prose.

/// A provider answered `429`. The run latches it and asks that provider nothing further.
pub const PROVIDER_RATE_LIMITED: &str = "provider.rate_limited";

/// Every event this process emits — the vocabulary a reader or a metric filter can rely on.
#[cfg(test)]
const ALL: &[&str] = &[PROVIDER_RATE_LIMITED];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_are_lowercase_dotted_and_unique() {
        for name in ALL {
            assert!(
                name.starts_with(char::is_lowercase),
                "event {name} must start lowercase"
            );
            assert!(name.contains('.'), "event {name} must be dotted");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'),
                "event {name} must be lowercase snake, separated by dots"
            );
        }

        let mut names = ALL.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL.len(), "event names must be unique");
    }
}
