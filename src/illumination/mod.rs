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
