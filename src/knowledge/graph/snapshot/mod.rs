//! Snapshotting the projected graph: take a copy, empty the live one, and diff two copies.
//!
//! [reset] is the whole loop — snapshot, then empty. [compare] answers what a change to the linker
//! actually did, over two snapshots. [Snapshot] is the copy itself, and where its tables live.
//!
//! Observations are never touched: reset only unlinks them, which is what makes two runs
//! comparable at all — the observation ids survive, the entity ids do not.

mod compare;
mod record;
mod reset;

pub use compare::{compare, Kind};
pub use record::Snapshot;
pub use reset::reset;
