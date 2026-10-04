use std::sync::Arc;

use crate::{logic, task};

pub struct WebhookState {
    pub logic: logic::LogicState,
    pub task_master: Arc<task::TaskMaster>,
}
