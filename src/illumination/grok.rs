use super::Illumination;

#[derive(Clone)]
pub struct GrokIllumination;

#[async_trait::async_trait]
impl Illumination for GrokIllumination {
    async fn illuminate(&self, capture_id: i32) -> String {
        let s = format!("Grok illumination for capture ID {}", capture_id);
        println!("{}", s);
        s
    }
}
