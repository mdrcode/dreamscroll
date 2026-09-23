//! Best-effort server-sent update primitives.
//!
//! Event payload types, PostgreSQL notification publishing, and PostgreSQL
//! listening are kept in separate modules to keep this module root declarative.

pub mod event;
pub mod listener;
pub mod notifier;

pub use event::*;
pub use listener::{ReceivedServerEvent, ServerEventListener, spawn_local_fanout};
pub use notifier::ServerEventNotifier;
