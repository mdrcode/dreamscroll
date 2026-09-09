use crate::{config, storage};

use super::*;

pub fn make_illuminator(
    cfg: &config::Config,
    storage: Box<dyn storage::StorageProvider>,
) -> Box<dyn Illuminator> {
    match cfg.illuminator.as_str() {
        "gemini" => Box::new(gemini::legacy::GeminiIlluminator::new(storage)),
        "geminipublicapi" => Box::new(gemini::GeminiPublicApiIlluminator::new(
            cfg.gemini_api_key
                .as_deref()
                .expect("GEMINI_API_KEY required but missing from config."),
            cfg.gemini_model_id
                .as_deref()
                .expect("GEMINI_MODEL_ID required but missing from config."),
            storage,
        )),
        "geminivertexapi" => Box::new(gemini::GeminiVertexApiIlluminator::new(
            &cfg.gcloud_project_id,
            cfg.gemini_model_id
                .as_deref()
                .expect("GEMINI_MODEL_ID required but missing from config."),
            cfg.gemini_payload_method,
            storage,
        )),
        "grok" => Box::new(grok::GrokIlluminator::default()),
        "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator),
        other => unimplemented!(
            "Unknown illuminator model '{}'. Supported: grok, gemini, geminipublicapi, geminivertexapi, loremipsum.",
            other
        ),
    }
}
