//! Telemetry wiring: logging + OpenTelemetry tracing for GCP.
//!
//! This module exists because three independent ecosystems don't fit together
//! out of the box:
//!   - `tracing`     — Rust's structured logging/span framework (no JSON, no GCP).
//!   - `opentelemetry` — the trace model (`tracing-opentelemetry` bridges spans).
//!   - GCP Cloud Logging/Trace — expect a specific JSON shape + severity vocab
//!     (`WARNING` not `WARN`, no `TRACE`) and magic `logging.googleapis.com/*`
//!     correlation keys.
//!
//! None of these crates depends on the others, so the glue lives here:
//!   - `gcloud_logging_format` — turns `tracing::Event` into Cloud Logging JSON
//!   - `axum_propagation`      — seeds spans from Cloud Run's `traceparent` header
//!   - `init`                  — assembles the subscriber (layers = traces + logs)
//!   - `util`                  — small helpers

mod axum_propagation;
pub use axum_propagation::*;

mod gcloud_logging_format;

mod init;
pub use init::*;

mod util;
pub use util::*;
