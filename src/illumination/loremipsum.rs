use crate::api;

use super::Illuminator;

#[derive(Clone, Default)]
pub struct LoremIpsumIlluminator;

#[async_trait::async_trait]
impl Illuminator for LoremIpsumIlluminator {
    fn model_name(&self) -> &'static str {
        "loremipsum"
    }

    async fn illuminate(&self, capture: api::CaptureInfo) -> anyhow::Result<String> {
        let s = format!("Lorem ipsum illumination for capture ID {}", capture.id);
        std::thread::sleep(std::time::Duration::from_millis(500));
        Ok(s)
    }
}
