use super::Illuminator;
use crate::controller;

#[derive(Clone, Default)]
pub struct GrokIlluminator;

#[async_trait::async_trait]
impl Illuminator for GrokIlluminator {
    async fn illuminate(&self, capture: controller::CaptureInfo) -> String {
        tracing::info!("GrokIlluminator: Illuminating capture ID {}", capture.id);
        let s = format!("Here is the illumination for capture {}!!", capture.id);
        s
    }
}
