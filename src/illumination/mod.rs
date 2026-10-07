// core traits
mod illuminator;
pub use illuminator::*;

pub mod v1;

// illuminator implementations
mod illuminator_gemini;
pub use illuminator_gemini::GeminiIlluminator;
