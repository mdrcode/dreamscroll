use crate::config;

use super::*;

pub fn make_firestarter(cfg: &config::Config) -> anyhow::Result<Box<dyn Firestarter>> {
    match cfg.firestarter.as_str() {
        "grok" => {
            let api_key = cfg
                .xai_api_key
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("XAI_API_KEY required but missing from config"))?
                .to_string();
            Ok(Box::new(grok::GrokFirestarter::new(api_key)))
        }
        "gemini" => {
            let api_key = cfg
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
