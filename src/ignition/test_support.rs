use super::{Firestarter, SparkResult};

pub(crate) struct UnusedFirestarter;

#[async_trait::async_trait]
impl Firestarter for UnusedFirestarter {
    fn name(&self) -> &str {
        "test"
    }

    async fn spark(&self, _captures: Vec<crate::api::CaptureInfo>) -> anyhow::Result<SparkResult> {
        anyhow::bail!("spark is not used by the illumination worker test")
    }
}
