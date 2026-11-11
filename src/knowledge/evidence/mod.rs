//! Claims and identities: what a capture claimed, what an authority answered, and who holds it.
//!
//! - a [`PlatformLink`] is the illuminator's optional platform/handle (plus display name and URL).
//!   It is not an identity; [`IdentityKey::try_from`] maps it to one when it has a handle.
//! - an [`IdentityKey`] is the durable name — `(scheme, external_id)`, both folded — that survives a
//!   rebuild.
//! - an [`IdentityDraft`] is a row of `knowledge_identities` before it has an id: that key, plus
//!   whoever answered and what they said.
//! - a [`Claim`] is an entity's claim to an identity — the edge of the many-to-many, carrying what
//!   the question was about ([`ClaimBasis`]) and which capture asked.
//!
//! Two phases, one operation, and both callers use it.
//!
//! 1. **settle** — [`Evidence::ensure_identity`] creates only; [`Evidence::enrich_identity`] fills
//!    gaps. [`IdentitySettlement`] names which behaviour the caller needs.
//! 2. **claim** — [`Evidence::claim_identity`] settles the identity, asks the veto, and writes the
//!    edge, returning [`ClaimWrite`] with the claim id. [`ClaimDraft`] carries the edge's provenance
//!    and basis; the operation supplies the resolved identity id.
//!
//! An identity may have several holders: one Wikidata item can name an album and cover its songs.
//! This module reports them and leaves "are these one thing?" to `proximity`.
//!
//! Every change to `knowledge_claims` is here: `claim_identity` writes an edge, `move_claims`
//! re-points one on a fold (and drops the duplicate the unique index would refuse), and `retract`
//! marks it when a veto withdraws it.

pub mod claim;
pub mod identity;
pub mod selection;
pub mod store;

pub use crate::illumination::v2::PlatformLink;
pub use claim::{Claim, ClaimBasis, ClaimDraft};
pub use identity::{ILLUMINATOR, IdentityDraft, IdentityKey, MissingHandle, platform_link};
pub use selection::ClaimSelection;
pub use store::{ClaimOutcome, ClaimWrite, Evidence, IdentitySettlement};
