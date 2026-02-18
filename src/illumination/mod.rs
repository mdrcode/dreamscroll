// core traits
mod illuminator;
pub use illuminator::*;


// illuminator implementations
pub mod gemini;
pub mod geministructured;
pub mod grok;
pub mod loremipsum;



pub fn make_illuminator(
    model_name: &str,
    storage: Box<dyn crate::storage::StorageProvider>,
) -> Box<dyn Illuminator> {
    match model_name {
        "grok" => Box::new(grok::GrokIlluminator::default()),
        "gemini" => Box::new(gemini::GeminiIlluminator::new(storage)),
        "geministructured" => Box::new(geministructured::GeminiStructuredIlluminator::new(storage)),
        "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator::default()),
        other => panic!(
            "Unknown illuminator model '{}'. Supported: grok, gemini, geministructured, loremipsum.",
            other
        ),
    }
}
