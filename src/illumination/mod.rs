use std::sync::Arc;

use crate::{common, database::DbHandle};

mod grok;
mod simpleworker;

pub use grok::GrokIllumination;

#[async_trait::async_trait]
pub trait Illumination: Clone + Send + Sync {
    async fn illuminate(&self, capture_id: i32) -> String;
}

#[async_trait::async_trait]
pub trait IlluminationWorker: Send + Sync {
    async fn run(&self) -> anyhow::Result<()>;
}

pub fn make_worker<I>(db: Arc<DbHandle>, ill: I) -> Box<dyn IlluminationWorker>
where
    I: Illumination + 'static,
{
    Box::new(simpleworker::SimpleWorker {
        db,
        queue: Arc::new(common::OneShotQueue::new()),
        illumination: ill,
    })
}
