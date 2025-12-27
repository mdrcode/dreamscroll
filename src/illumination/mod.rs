use std::sync::Arc;

use crate::{common, database};

pub mod grok;

mod simple;

#[async_trait::async_trait]
pub trait Illumination: Clone + Send + Sync {
    async fn illuminate(&self, capture_id: i32) -> String;
}

#[async_trait::async_trait]
pub trait Illuminator: Send + Sync {
    async fn run(&self) -> anyhow::Result<()>;
}

pub fn make<I: Illumination + 'static>(db: Arc<database::DbHandle>, ill: I) -> Box<dyn Illuminator> {
    Box::new(simple::SimpleIlluminator {
        db,
        queue: Arc::new(common::OneShotQueue::new()),
        illumination: ill,
    })
}
