mod client;
pub use client::{
    GeminiInferenceClient, GeminiInputPart, GeminiInteractionRequest, GeminiInteractionResponse,
};

mod illuminator;
pub use illuminator::GeminiIlluminator;

mod prompts;
mod response;
