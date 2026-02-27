// core traits
mod illuminator;
pub use illuminator::*;
// illuminator implementations
pub mod gemini;
pub mod grok;
pub mod loremipsum;

pub fn make_illuminator(
    config: &crate::facility::Config,
    storage: Box<dyn crate::storage::StorageProvider>,
) -> Box<dyn Illuminator> {
    match config.illuminator.as_str() {
        "gemini" => Box::new(gemini::legacy::GeminiIlluminator::new(storage)),
        "geminipublicapi" => Box::new(gemini::GeminiPublicApiIlluminator::new(
            config
                .gemini_api_key
                .as_deref()
                .expect("GEMINI_API_KEY required but missing from config."),
            storage,
        )),
        "geminivertexapi" => Box::new(gemini::GeminiVertexApiIlluminator::new(
            &config.gcloud_project_id,
            "gemini-3-flash-preview",
            storage,
        )),
        "grok" => Box::new(grok::GrokIlluminator::default()),
        "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator::default()),
        other => unimplemented!(
            "Unknown illuminator model '{}'. Supported: grok, gemini, geminipublicapi, loremipsum.",
            other
        ),
    }
}
