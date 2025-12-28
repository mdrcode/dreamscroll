use super::Illumination;
use crate::controller;

#[derive(Clone)]
pub struct GrokIllumination;

#[async_trait::async_trait]
impl Illumination for GrokIllumination {
    async fn illuminate(&self, capture: controller::CaptureInfo) -> String {
        let s = format!("Grok illumination for capture ID {}", capture.id);
        println!("{}", s);
        s
    }
}
