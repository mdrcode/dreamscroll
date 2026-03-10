use crate::api;

#[async_trait::async_trait]
pub trait Firestarter {
    fn name(&self) -> &str;
    async fn spark(&self, captures: Vec<api::CaptureInfo>) -> anyhow::Result<String>;
}
