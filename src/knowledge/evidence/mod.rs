//! Claims and identities: what an observation claimed, what a provider answered, and who holds it.
//!
//! - a [`PlatformLink`](crate::illumination::v2::PlatformLink) is the illuminator's optional
//!   platform/handle (plus display name and URL).
//!   It is not an identity; [`IdentityKey::try_from`] maps it to one when it has a handle.
//! - an [`IdentityKey`] is the durable name — `(scheme, external_id)`, both folded — that survives a
//!   rebuild.
//! - an [`IdentityDraft`] is a row of `knowledge_identities` before it has an id: that key, plus
//!   whoever answered and what they said.
//! - a [`Claim`] is an entity's claim to an identity — the edge of the many-to-many, carrying who
//!   asserted it and which observation carried it.
//!
//! One operation: [`Evidence::claim_identity`] upserts the identity and writes the edge, returning
//! [`ClaimOutcome`] with the ids. The identity upsert is observation-safe, so there is no
//! create/enrich mode.
//!
//! An identity may have several holders: one Wikidata item can name an album and cover its songs.
//! This module reports them and leaves "are these one thing?" to `proximity`.
//!
//! Every change to `knowledge_claims` is here: `claim_identity` writes an edge, `move_claims`
//! re-points one on a fold (and drops the duplicate the unique index would refuse), and `retract`
//! marks it when a veto withdraws it.

/// The provider id for identities and claims carried by an observation's platform link.
pub const PLATFORM_LINK_PROVIDER_ID: &str = "illuminator";

pub mod claim;
pub mod identity;
pub mod selection;
pub mod store;

pub use claim::Claim;
pub use identity::{IdentityDraft, IdentityKey, MissingHandle, platform_link};
pub use selection::ClaimSelection;
pub use store::{ClaimOutcome, Evidence};
