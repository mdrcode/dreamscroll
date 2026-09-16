use crate::database::DbHandle;

/// Watches `task_status` for changes and relays them to SSE subscribers.
///
/// This is one of only two structs allowed to query `task_status` directly
/// (the other is `TaskMaster`). It will own the Postgres `LISTEN`/`NOTIFY`
/// connection that wakes up on status transitions and re-reads rows to push
/// to clients.
///
/// **Not yet implemented** — wired in a later step (see
/// `_project/plans/sse-task-status.md` §12). This struct exists now to make
/// the two-owner contract explicit.
pub struct StatusListener {
    _db: DbHandle,
}

impl StatusListener {
    pub fn new(db: DbHandle) -> Self {
        Self { _db: db }
    }
}
