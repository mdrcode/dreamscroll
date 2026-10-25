//! Best-effort server-sent update primitives.
//!
//! Event payload types, PostgreSQL notification publishing, PostgreSQL
//! listening, and HTTP (SSE) delivery are kept in separate modules to keep this
//! module root declarative.

pub mod event;
pub mod http;
pub mod listener;
pub mod notifier;

pub use event::*;
pub use listener::{ServerEventListener, spawn_local_fanout};
pub use notifier::{PostgresNotifier, ServerEventNotifier};
