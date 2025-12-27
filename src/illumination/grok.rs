use super::Illumination;

#[derive(Clone)]
pub struct GrokIllumination;

#[async_trait::async_trait]
impl Illumination for GrokIllumination {
    async fn illuminate(&self, capture_id: i32) -> String {
        println!("Grok illuminating capture ID {}", capture_id);
        format!("Grok illumination for capture ID {}", capture_id)
    }
}
