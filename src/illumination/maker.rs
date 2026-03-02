use crate::{facility, storage};

use super::*;

pub fn make_illuminator(
    config: &facility::Config,
    storage: Box<dyn storage::StorageProvider>,
) -> Box<dyn Illuminator> {
    match config.illuminator.as_str() {
        "gemini" => Box::new(gemini::legacy::GeminiIlluminator::new(storage)),
        "geminipublicapi" => Box::new(gemini::GeminiPublicApiIlluminator::new(
            config
                .gemini_api_key
                .as_deref()
                .expect("GEMINI_API_KEY required but missing from config."),
            "gemini-3-flash-preview",
            storage,
        )),
        "geminivertexapi" => Box::new(gemini::GeminiVertexApiIlluminator::new(
            &config.gcloud_project_id,
            "gemini-3-flash-preview",
            config.gemini_payload_method,
            storage,
        )),
        "grok" => Box::new(grok::GrokIlluminator::default()),
        "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator::default()),
        other => unimplemented!(
            "Unknown illuminator model '{}'. Supported: grok, gemini, geminipublicapi, geminivertexapi, loremipsum.",
            other
        ),
    }
}
