use crate::api;

use super::*;

const DETAILS: &str = r#"
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore
et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut
aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse
cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa
qui officia deserunt mollit anim id est laborum.
"#;

#[derive(Clone, Default)]
pub struct LoremIpsumIlluminator;

#[async_trait::async_trait]
impl Illuminator for LoremIpsumIlluminator {
    fn name(&self) -> &'static str {
        "geministructured"
    }

    async fn illuminate(&self, capture: &api::CaptureInfo) -> anyhow::Result<Illumination> {
        std::thread::sleep(std::time::Duration::from_millis(500));

        let meta = IlluminationMeta {
            provider_name: "loremipsum".to_string(),
        };

        Ok(Illumination {
            meta,
            summary: format!("Lorem ipsum illumination for capture ID {}", capture.id),
            details: DETAILS.trim().to_string(),
            suggested_searches: vec![],
            entities: vec![],
            social_media_accounts: vec![],
        })
    }
}
