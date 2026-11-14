//! Text: the fold, the keys derived from it, and the words a measure compares.
//!
//! One pipeline, four questions, one file each:
//!
//! - [`fold::comparable`] — *are these two surfaces the same string?* The answer is an identity key:
//!   `knowledge_entities` and `knowledge_blocking_index` store it, and two surfaces are the same
//!   thing exactly when their comparable forms are equal.
//! - [`key::word_key`] — the word-order-insensitive form of a folded surface: a guess about what
//!   *might* be the same thing, so it lives with blocking and the heuristic's name match.
//! - [`tokens`] — *do these two texts talk about the same thing?* Words, stems, grams and set
//!   overlap for a measure to price. Nothing here is ever stored.
//! - [`surface`] — is one surface a spelling of the other, or a whole-word prefix of it?
//!   [`surface::near_surface`] and [`surface::word_prefix`] both compare two *already comparable*
//!   surfaces, so their caller owns the fold.
//!
//! [`fingerprint::norm_fingerprint`] stamps the first two: any change to the fold or the word key
//! moves it, which is what makes a stale `knowledge_blocking_index` visible instead of silent.
//!
//! Nothing here knows what a platform is. A capture's `(platform, handle)` becomes an
//! [`IdentityKey`](crate::knowledge::evidence::IdentityKey) from these folds, and both halves are
//! treated like any surface. And nothing here rewrites the captured surface: the questions above are
//! asked about it, never stored in its place. Inner punctuation is preserved on purpose — `C`, `C#`
//! and `C++` are three different things.

mod fingerprint;
mod fold;
mod key;
mod surface;
mod tokens;

pub(crate) use fingerprint::norm_fingerprint;
pub(crate) use fold::comparable;
pub(crate) use key::word_key;
pub(crate) use surface::{near_surface, word_prefix};
pub use tokens::jaccard;
pub(crate) use tokens::{content_tokens, grams};
