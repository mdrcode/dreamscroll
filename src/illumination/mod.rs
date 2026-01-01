use std::sync::Arc;

use crate::{common::AppError, controller, database::DbHandle};

mod gemini;
mod grok;
mod loremipsum;
mod simpleworker;

pub use gemini::GeminiIlluminator;
pub use grok::GrokIlluminator;
pub use loremipsum::LoremIpsumIlluminator;

#[async_trait::async_trait]
pub trait Illuminator: dyn_clone::DynClone + Send + Sync {
    async fn illuminate(&self, capture: controller::CaptureInfo) -> anyhow::Result<String>;
}

dyn_clone::clone_trait_object!(Illuminator);

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
