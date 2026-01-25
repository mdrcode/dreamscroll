// core traits
mod illuminator;
pub use illuminator::*;
mod worker;
pub use worker::*;

// simple local worker implementation
mod simpleworker;

// illuminator implementations
pub mod gemini;
pub mod geministructured;
pub mod grok;
pub mod loremipsum;

pub fn make(model_name: &str) -> Box<dyn Illuminator> {
    match model_name {
        "grok" => Box::new(grok::GrokIlluminator::default()),
        "gemini" => Box::new(gemini::GeminiIlluminator::default()),
        "geministructured" => Box::new(geministructured::GeminiStructuredIlluminator::default()),
        "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator::default()),
        other => panic!(
            "Unknown illuminator model '{}'. Supported: grok, gemini, geministructured, loremipsum.",
            other
        ),
    }
}
