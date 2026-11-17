//! Identity annotations: the canonical document a provider holds for an entity, and the answer it
//! gave.
//!
//! - a [`PlatformLink`](crate::illumination::v2::PlatformLink) is the illuminator's optional
//!   platform/handle (plus display name and URL). It is not an identity; [`platform_link`] reads it
//!   off an observation, and [`Identity`] is what it becomes when it has a handle.
//! - an [`IdentityKey`] is the durable name — `(scheme, external_id)`, both folded — that survives a
//!   rebuild.
//! - an [`Identity`] is one provider's annotation of one entity with that key: the metadata an
//!   identity resolver answered with, and the observation that carried it when a capture showed it.
//!
//! The store is [`Evidence`] for the capture/link path: a capture appends one, and a veto deletes
//! one. A resolver's replacement is the v2 `knowledge2::resolution` producer, which emits graph
//! actions. Nothing brokers the relation.
//!
//! A key is a canonical document, not a unique name: one Wikidata item can name an album and cover
//! its songs, or a film and its characters. Holders of one key are candidates for `proximity`, not
//! duplicates. This module reports them and leaves "are these one thing?" to `proximity`; differing
//! referents are `distinct`, and `veto_identity` is only for a document that does not describe the
//! entity at all.

/// The provider id for identities carried by an observation's platform link.
pub const PLATFORM_LINK_PROVIDER_ID: &str = "illuminator";

pub mod identity;
pub mod store;

pub use identity::{Identity, IdentityKey, MissingHandle, platform_link};
pub use store::{Evidence, IdentityWrite};
