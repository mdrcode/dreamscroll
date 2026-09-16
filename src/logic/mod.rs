// Pure, non-trivial business logic for background task execution.
//
// This module is deliberately independent of `webhook` — it defines the
// concrete task types and the code paths that execute them, with no knowledge
// of the HTTP transport (Cloud Tasks / Pub/Sub / local). The webhook layer
// deserializes a task and calls `logic::<task>::exec`.
pub mod illuminate;
pub mod search_index;
pub mod spark;
