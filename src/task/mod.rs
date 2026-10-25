// Task and TaskRun, the core concepts
mod task_def;
pub use task_def::*;

// TaskMaster is the primary entry point for starting a new Run
// of a Task. It owns the queues and the database handle, and is responsible
// for enforcing the "one run at a time" rule for each logical task.
mod taskmaster;
pub use taskmaster::*;

// TaskRunStatus models progress of a background Run (invocation) of a Task
mod taskrunstatus;
pub use taskrunstatus::*;

// StatusCatchup (replay) and StatusPublisher (live) are the two SSE-facing
// adapters over task-run state, kept out of TaskMaster's lifecycle API.
mod statuscatchup;
pub use statuscatchup::StatusCatchup;
mod statuspublisher;

// TaskRunTracker is TaskMaster's private persistence component for task-run
// status. Callers go through TaskMaster so lifecycle policy and status writes
// remain encapsulated together.
mod taskruntracker;

// Task timing aggregates used to estimate how long queued work will take.
pub mod timing;

// TaskQueue trait which abstracts over the underlying queue backends.
mod taskqueue;
pub use taskqueue::*;
mod taskqueue_cloudtask;
pub use taskqueue_cloudtask::*;
mod taskqueue_local;
pub use taskqueue_local::*;

mod maker;
pub use maker::*;
