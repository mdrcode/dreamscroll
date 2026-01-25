use std::sync::Arc;

use crate::{api, database::DbHandle};

use super::{illuminator::*, simpleworker};

#[async_trait::async_trait]
pub trait IlluminatorWorker: Send + Sync {
    async fn run(&self) -> anyhow::Result<(), api::ApiError>;
}

pub fn make_worker<I>(db: Arc<DbHandle>, ill: I) -> Box<dyn IlluminatorWorker>
where
    I: Illuminator + 'static,
{
    Box::new(simpleworker::SimpleWorker::new(db, ill))
}
