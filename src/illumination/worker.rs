use crate::{api, auth};

use super::{illuminator::*, simpleworker};

#[async_trait::async_trait]
pub trait IlluminatorWorker: Send + Sync {
    async fn run(&self) -> anyhow::Result<(), api::ApiError>;
}

pub fn make_worker(
    user_api: api::UserApiClient,
    context: auth::Context,
    ill: Box<dyn Illuminator>,
) -> Box<dyn IlluminatorWorker> {
    Box::new(simpleworker::SimpleWorker::new(user_api, context, ill))
}
