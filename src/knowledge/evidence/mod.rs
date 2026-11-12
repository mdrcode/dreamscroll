//! Identity annotations: what an entity is called, and by whom.
//!
//! - a [`PlatformLink`](crate::illumination::v2::PlatformLink) is the illuminator's optional
//!   platform/handle (plus display name and URL). It is not an identity; [`Identity::from_platform_link`]
//!   maps it to one when it has a handle.
//! - an [`IdentityKey`] is the durable name — `(scheme, external_id)`, both folded — that survives a
//!   rebuild.
//! - an [`Identity`] is one provider's annotation of one entity with that key: the metadata an
//!   authority answered with, and the observation that carried it when a capture showed it.
//!
//! The store is [`Evidence`]. A resolver's run replaces its provider's complete annotation set for
//! an entity; a capture appends one; a veto deletes one. Nothing brokers the relation.
//!
//! A key may have several holders: one Wikidata item can name an album and cover its songs. This
//! module reports them and leaves "are these one thing?" to `proximity`.

/// The provider id for identities carried by an observation's platform link.
pub const PLATFORM_LINK_PROVIDER_ID: &str = "illuminator";

pub mod identity;
pub mod selection;
pub mod store;

pub use identity::{Identity, IdentityKey, MissingHandle, platform_link};
pub use selection::IdentitySelection;
pub(crate) use store::filter_identities;
pub use store::{Evidence, IdentityWrite};
