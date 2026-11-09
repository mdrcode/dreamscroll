//! Claims and identities: what a capture asserted, what an authority answered, and who holds it.
//!
//! - a [`DraftIdentity`] is a row of `knowledge_identities` before it has an id: the thing the world
//!   names, `(scheme, external_id)`, plus whoever answered.
//! - a [`PlatformLink`] is a platform presence as a capture showed it — `(scheme, handle)`, both
//!   folded — before anything knows whether an entity holds it.
//! - a [`Claim`] is an entity's assertion of an identity — the edge of the many-to-many, carrying
//!   what the question was about ([`ClaimBasis`]) and which capture asked.
//!
//! Two writers, one table: [`Evidence::record`] writes the claim a capture's observation carries, and
//! [`store_claim`] writes an authority's answer. Both live in [`store`], so the veto, the uniqueness
//! rule and the basis column are read in one place instead of being rediscovered per provider.
//!
//! An identity may have several holders: one Wikidata item can name an album and cover its songs.
//! This module reports them and leaves "are these one thing?" to `proximity`.

pub mod claim;
pub mod store;

pub use claim::{Claim, ClaimBasis};
pub use store::{
    DraftIdentity, Evidence, PlatformLink, Stored, store_claim, unique_violation,
};
