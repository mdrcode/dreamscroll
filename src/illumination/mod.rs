// core traits
mod illuminator;
pub use illuminator::*;

mod maker;
pub use maker::*;

// illuminator implementations
pub mod gemini;
pub mod grok;
pub mod loremipsum;
