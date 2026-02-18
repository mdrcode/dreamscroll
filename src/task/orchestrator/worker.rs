use crate::{api, illumination};

use super::*;

#[async_trait::async_trait]
pub trait IlluminatorWorker: Send + Sync {
    async fn run(&self) -> anyhow::Result<(), api::ApiError>;
}

pub fn make_worker(
    service_api: api::ServiceApiClient,
    ill: Box<dyn illumination::Illuminator>,
) -> Box<dyn IlluminatorWorker> {
    Box::new(simpleworker::SimpleWorker::new(service_api, ill))
}
