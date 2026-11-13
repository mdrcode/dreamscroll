//! A platform link, resolved into what the authority should be asked.
//!
//! The property and the value are **one** decision, because the value's form depends on the
//! property: a YouTube channel ID cannot fill a handle property, `r/x` and `u/x` name different
//! referents, and a property whose own description says "do not include the '@' symbol" will never
//! match the decorated form a capture often writes.
//!
//! Every property and every serialization here was read off Wikidata on 2026-11-12, from the
//! properties' descriptions: `P2002`/`P7085`/`P11245` forbid the `@`; `P4033` is `user@server` with
//! no leading `@`; `P2013` is what follows `facebook.com/`; `P2397` is an opaque channel ID.
//! An unmapped platform asks nothing: a wrong property does not fail, it returns nothing and caches
//! the negative, which is indistinguishable from "this handle is not in Wikidata".
//!
//! Pure — no client, no `self`, no database. One `PlatformLink` in, the queries for it out.

use crate::illumination::v2::{Platform, PlatformLink};

/// A Wikidata external-identifier property: the `wdt:P…` predicate a handle is matched against.
///
/// A const per property we know, because the number is otherwise a bare string in a query, where a
/// typo is invisible: it returns nothing and caches the negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PropertyId(&'static str);

impl PropertyId {
    /// X (formerly Twitter) username; values carry no `@`.
    pub(super) const X_USERNAME: Self = Self("P2002");
    pub(super) const INSTAGRAM_USERNAME: Self = Self("P2003");
    /// GitHub account.
    pub(super) const GITHUB_ACCOUNT: Self = Self("P2037");
    /// A YouTube *channel* ID (`UC…`), not the channel's name.
    pub(super) const YOUTUBE_CHANNEL: Self = Self("P2397");
    /// A YouTube handle; values carry no `@`.
    pub(super) const YOUTUBE_HANDLE: Self = Self("P11245");
    pub(super) const TIKTOK_USERNAME: Self = Self("P7085");
    /// A Mastodon address, `user@server.domain`, without a leading `@`.
    pub(super) const MASTODON_ADDRESS: Self = Self("P4033");
    pub(super) const FACEBOOK_USERNAME: Self = Self("P2013");
    pub(super) const LINKEDIN_PROFILE: Self = Self("P6634");
    pub(super) const THREADS_USERNAME: Self = Self("P11892");
    pub(super) const BLUESKY_HANDLE: Self = Self("P12361");
    pub(super) const REDDIT_USERNAME: Self = Self("P4265");
    /// A Reddit forum's name; not a user, and therefore not a username.
    pub(super) const SUBREDDIT: Self = Self("P3984");

    pub(super) const fn as_str(self) -> &'static str {
        self.0
    }
}

/// One exact match to ask for: the property whose value space the handle lives in, and the literal
/// the SPARQL `VALUES` block compares against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PropertyQuery {
    pub(super) property: PropertyId,
    pub(super) value: String,
}

/// The property that can hold this link, and the value(s) it expects.
///
/// Empty when the platform is unmapped or the handle cannot fill a value space. Two queries only for
/// case spellings of one namespace — never for two properties, because one platform link names one
/// referent.
pub(super) fn platform_link_queries(link: &PlatformLink) -> Vec<PropertyQuery> {
    let Some(handle) = link.handle().map(str::trim) else {
        return Vec::new();
    };
    match link.platform {
        Platform::XTwitter => usernames(PropertyId::X_USERNAME, handle),
        Platform::Instagram => usernames(PropertyId::INSTAGRAM_USERNAME, handle),
        Platform::Tiktok => usernames(PropertyId::TIKTOK_USERNAME, handle),
        Platform::Threads => usernames(PropertyId::THREADS_USERNAME, handle),
        Platform::Bluesky => usernames(PropertyId::BLUESKY_HANDLE, handle),
        Platform::Github => usernames(PropertyId::GITHUB_ACCOUNT, handle),
        // A Mastodon address is `user@server`: the leading `@` goes, the inner one stays.
        Platform::Mastodon => usernames(PropertyId::MASTODON_ADDRESS, handle),
        Platform::Facebook => usernames(PropertyId::FACEBOOK_USERNAME, handle),
        Platform::Linkedin => usernames(PropertyId::LINKEDIN_PROFILE, handle),
        // A name cannot fill a channel-ID property, and a channel ID cannot fill a handle one.
        Platform::Youtube => match channel_id(handle) {
            Some(id) => vec![PropertyQuery {
                property: PropertyId::YOUTUBE_CHANNEL,
                value: id,
            }],
            None => usernames(PropertyId::YOUTUBE_HANDLE, handle),
        },
        // Two namespaces, two referents: the prefix selects the property, nothing is guessed.
        Platform::Reddit => match subreddit(handle) {
            Some(name) => queries(PropertyId::SUBREDDIT, name),
            None => usernames(PropertyId::REDDIT_USERNAME, reddit_user(handle)),
        },
        // A platform we have not taught a property. Borrowing another platform's would fabricate.
        Platform::Other => Vec::new(),
    }
}

/// A username-shaped value: decoration is not part of it, and a case-insensitive namespace is asked
/// for both spellings because Wikidata stores the canonical case while the `VALUES` match is exact.
fn usernames(property: PropertyId, handle: &str) -> Vec<PropertyQuery> {
    let value = handle.trim_start_matches(['@', '#']);
    if value.is_empty() {
        return Vec::new();
    }
    queries(property, value.to_string())
}

/// The value, plus its lowercase spelling when it differs.
fn queries(property: PropertyId, value: String) -> Vec<PropertyQuery> {
    let lower = value.to_lowercase();
    let mut queries = vec![PropertyQuery { property, value }];
    if lower != queries[0].value {
        queries.push(PropertyQuery {
            property,
            value: lower,
        });
    }
    queries
}

/// `u/<name>` or a bare name: the username, which is what `P4265` holds.
fn reddit_user(handle: &str) -> &str {
    let name = handle.trim_start_matches(['@', '#']);
    name.strip_prefix("u/")
        .or_else(|| name.strip_prefix("U/"))
        .unwrap_or(name)
}

/// `r/<name>`: the forum's name, which is what `P3984` holds. `u/` is a user, not a forum.
fn subreddit(handle: &str) -> Option<String> {
    let name = handle
        .strip_prefix("r/")
        .or_else(|| handle.strip_prefix("R/"))?;
    (!name.is_empty()).then(|| name.to_string())
}

/// `UC` and 22 characters: the shape of a channel ID, so a handle can never fill `P2397`.
fn channel_id(handle: &str) -> Option<String> {
    (handle.len() == 24 && handle.starts_with("UC")).then(|| handle.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queries_for(platform: Platform, handle: &str) -> Vec<(&'static str, String)> {
        let link = PlatformLink {
            platform,
            handle: Some(handle.to_string()),
            display_name: None,
            url: None,
        };
        platform_link_queries(&link)
            .into_iter()
            .map(|query| (query.property.as_str(), query.value))
            .collect()
    }

    fn expected(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs
            .iter()
            .map(|(property, value)| (*property, value.to_string()))
            .collect()
    }

    /// The property is selected by what the value *is*, and the value is serialized the way that
    /// property documents. Both halves are asserted here because they are one decision.
    #[test]
    fn a_platform_link_yields_the_property_that_can_hold_it() {
        for (platform, handle, want) in [
            // Decoration is not part of a username, and the namespace is case-insensitive.
            (
                Platform::XTwitter,
                "@stats_feed",
                &[("P2002", "stats_feed")][..],
            ),
            (
                Platform::XTwitter,
                "Tsoding",
                &[("P2002", "Tsoding"), ("P2002", "tsoding")][..],
            ),
            (Platform::XTwitter, "@", &[][..]),
            // YouTube splits: a channel ID is not a handle, and neither fills the other's property.
            (
                Platform::Youtube,
                "UC_x5XG1OV2P6uZZ5FSM9Ttw",
                &[("P2397", "UC_x5XG1OV2P6uZZ5FSM9Ttw")][..],
            ),
            (
                Platform::Youtube,
                "@MeatCanyon",
                &[("P11245", "MeatCanyon"), ("P11245", "meatcanyon")][..],
            ),
            (
                Platform::Youtube,
                "3blue1brown",
                &[("P11245", "3blue1brown")][..],
            ),
            // Reddit's two namespaces are different referents.
            (
                Platform::Reddit,
                "r/NFCWestMemeWar",
                &[("P3984", "NFCWestMemeWar"), ("P3984", "nfcwestmemewar")][..],
            ),
            (Platform::Reddit, "u/spez", &[("P4265", "spez")][..]),
            // A Mastodon address keeps its inner `@`.
            (
                Platform::Mastodon,
                "@user@mastodon.social",
                &[("P4033", "user@mastodon.social")][..],
            ),
            (Platform::Threads, "@ada", &[("P11892", "ada")][..]),
            // An unmapped platform asks nothing rather than borrowing another platform's property.
            (Platform::Other, "anything", &[][..]),
        ] {
            let context = format!("{platform:?} {handle:?}");
            assert_eq!(queries_for(platform, handle), expected(want), "{context}");
        }
    }

    /// A link with no handle asks nothing: an empty `VALUES` literal is a wasted call that caches a
    /// negative about nobody.
    #[test]
    fn a_link_without_a_handle_asks_nothing() {
        let link = PlatformLink {
            platform: Platform::XTwitter,
            handle: None,
            display_name: Some("Tsoding".to_string()),
            url: None,
        };
        assert!(platform_link_queries(&link).is_empty());
        assert!(queries_for(Platform::XTwitter, "   ").is_empty());
    }
}
