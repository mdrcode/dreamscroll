//! One assertion about an entity's identity: what was said, how sure, and what was asked.
//!
//! The vocabulary every writer of `knowledge_claims` shares. A capture's observation and an
//! authority's answer are both claims; only [`ClaimBasis`] and who filled the other fields differ.

/// One identity a provider is willing to assert about an entity.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub scheme: String,
    pub id: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub method: String,
    pub confidence: f32,
    /// What the provider was asked about. The resolver knows, because it chose the question.
    pub basis: ClaimBasis,
}

/// What the question was about, for one claim: the identifier, or the text.
///
/// Corroboration counts independent *inputs*, and independence is this, not who answered. A capture's
/// handle and an authority asked about that handle are one chain however many schemes they name; a
/// name and a handle are two. Recorded when the claim is written, because that is the only moment
/// anything knows which question produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimBasis {
    /// A handle: a capture showed one, or the authority was asked about one.
    Handle,
    /// A name, title or description.
    Text,
}

impl ClaimBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Handle => "handle",
            Self::Text => "text",
        }
    }

    /// The stored form, back. Anything that is not `handle` reads as text: the column only ever holds
    /// these two.
    pub fn from_db(value: &str) -> Self {
        if value == Self::Handle.as_str() {
            Self::Handle
        } else {
            Self::Text
        }
    }
}
