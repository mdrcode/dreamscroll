use serde::{Deserialize, Serialize};

use super::ThinkingLevel;

/// Gemini-specific knobs for one inference request.
///
/// Provider-neutral tools stay in `InferenceCapability`; the flags here are the
/// Gemini request policy and generation controls that produce them.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GeminiInferenceOptions {
    /// `None` delegates to the model default. See
    /// <https://ai.google.dev/gemini-api/docs/thinking#thinking-levels>.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// Persist the interaction (`store`). Debugging only; retains prompt and media.
    #[serde(default)]
    pub store: bool,
    /// Ask for human-readable thought summaries. Debugging only.
    #[serde(default)]
    pub thinking_summaries: bool,
    /// Enable the `google_search` grounding tool. Billed per grounded request.
    #[serde(default)]
    pub web_search: bool,
}
