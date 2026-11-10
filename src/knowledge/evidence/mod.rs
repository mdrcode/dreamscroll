//! Claims and identities: what a capture asserted, what an authority answered, and who holds it.
//!
//! - a [`DraftIdentity`] is a row of `knowledge_identities` before it has an id: the thing the world
//!   names, `(scheme, external_id)`, plus whoever answered.
//! - a [`PlatformLink`] is a platform presence as a capture showed it — `(scheme, handle)`, both
//!   folded — before anything knows whether an entity holds it.
//! - a [`Claim`] is an entity's assertion of an identity — the edge of the many-to-many, carrying
//!   what the question was about ([`ClaimBasis`]) and which capture asked.
//!
//! Two phases, one implementation of each, and both callers use both.
//!
//! 1. **ingest** — [`Evidence::identity_for_link`] from a capture, [`Evidence::upsert_identity`]
//!    from an answer.
//!    A capture creates an identity and never enriches one; only an answer knows what a thing is.
//! 2. **assert** — [`Evidence::assert`] writes the edge, asking the veto at the one place a claim
//!    becomes a row. It is the same call for a capture's handle and an authority's cross-reference;
//!    what differs is a value ([`ClaimDraft`]'s provenance and basis), not a second code path.
//!
//! An identity may have several holders: one Wikidata item can name an album and cover its songs.
//! This module reports them and leaves "are these one thing?" to `proximity`.
//!
//! Every change to `knowledge_claims` is here: `assert` inserts an edge, `move_claims` re-points one
//! on a fold (and drops the duplicate the unique index would refuse), and `retract` marks it when a
//! veto withdraws it.

pub mod claim;
pub mod identity;
pub mod store;

pub use claim::{Claim, ClaimBasis, ClaimDraft, Stored};
pub use identity::{DraftIdentity, IdentityKey, PlatformLink};
pub use store::Evidence;
