/// Gemini Interactions reasoning effort; supported levels and defaults depend on the model.
///
/// Only the levels our configured models accept are represented. `minimal` (still valid on
/// `gemini-3.6-flash`, the `*-flash-lite` models, and `gemini-3-flash-preview`) and `xhigh`
/// (Gemini 4 Hydrogen only) are deliberately absent: both are rejected by `gemini-3.8-flash`.
/// See <https://ai.google.dev/gemini-api/docs/thinking#thinking-levels>.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ThinkingLevel {
    Low,
    Medium,
    High,
}
