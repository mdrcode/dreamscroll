use crate::facility;

use super::*;

pub fn make_firestarter(config: &facility::Config) -> Box<dyn Firestarter> {
    match config.firestarter.as_str() {
        "grok" => Box::new(grok::GrokFirestarter::new(
            config
                .xai_api_key
                .as_deref()
                .expect("XAI_API_KEY required but missing from config.")
                .to_string(),
        )),
        "gemini" => Box::new(gemini::GeminiFirestarter::new(
            config
                .gemini_api_key
                .as_deref()
                .expect("GEMINI_API_KEY required but missing from config.")
                .to_string(),
        )),
        other => unimplemented!(
            "Unknown firestarter model '{}' for webhook Spark inference. Supported: grok, gemini.",
            other
        ),
    }
}
