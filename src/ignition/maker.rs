use crate::facility;

use super::*;

pub fn make_firestarter(config: &facility::Config) -> anyhow::Result<Box<dyn Firestarter>> {
    match config.firestarter.as_str() {
        "grok" => {
            let api_key = config
                .xai_api_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("XAI_API_KEY required but missing from config"))?
                .to_string();
            Ok(Box::new(grok::GrokFirestarter::new(api_key)))
        }
        "gemini" => {
            let api_key = config
                .gemini_api_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY required but missing from config"))?
                .to_string();
            Ok(Box::new(gemini::GeminiFirestarter::new(api_key)))
        }
        other => Err(anyhow::anyhow!(
            "Unknown firestarter model '{}' for webhook Spark inference. Supported: grok, gemini",
            other
        )),
    }
}
