use std::sync::Arc;

use crate::{api, auth, database::DbHandle};

use super::{illuminator::*, simpleworker};

#[async_trait::async_trait]
pub trait IlluminatorWorker: Send + Sync {
    async fn run(&self) -> anyhow::Result<(), api::ApiError>;
}

pub fn make_worker(
    db: Arc<DbHandle>,
    context: auth::Context,
    ill: Box<dyn Illuminator>,
) -> Box<dyn IlluminatorWorker> {
    Box::new(simpleworker::SimpleWorker::new(db, context, ill))
}
