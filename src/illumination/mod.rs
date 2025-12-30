use std::sync::Arc;

use crate::{common::AppError, controller, database::DbHandle};

mod grok;
mod loremipsum;
mod simpleworker;

pub use grok::GrokIlluminator;
pub use loremipsum::LoremIpsumIlluminator;

#[async_trait::async_trait]
pub trait Illuminator: Clone + Send + Sync {
    async fn illuminate(&self, capture: controller::CaptureInfo) -> String;
}

#[async_trait::async_trait]
pub trait IlluminatorWorker: Send + Sync {
    async fn run(&self) -> anyhow::Result<(), AppError>;
}

pub fn make_worker<I>(db: Arc<DbHandle>, ill: I) -> Box<dyn IlluminatorWorker>
where
    I: Illuminator + 'static,
{
    Box::new(simpleworker::SimpleWorker::new(db, ill))
}
