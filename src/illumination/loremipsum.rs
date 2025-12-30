use super::Illuminator;
use crate::controller;

#[derive(Clone)]
pub struct LoremIpsumIlluminator;

#[async_trait::async_trait]
impl Illuminator for LoremIpsumIlluminator {
    async fn illuminate(&self, capture: controller::CaptureInfo) -> String {
        let s = format!("Lorem ipsum illumination for capture ID {}", capture.id);
        std::thread::sleep(std::time::Duration::from_millis(500));
        s
    }
}
