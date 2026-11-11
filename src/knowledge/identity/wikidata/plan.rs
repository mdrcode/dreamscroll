//! What to ask: the ladder's requests, as data.
//!
//! Pure and stateless — no client, no `self`. The ladder in `resolver` decides *when* to ask and
//! what an answer means; this decides only *what*.

use super::shape;
use super::Kind;
use super::ID;
use crate::fetch::Request;
use crate::knowledge::normalize;
use crate::model;

/// One request, carrying the shape that produced it.
///
/// The only place requests are built here, so a kind and its shape cannot disagree — the failure the
/// per-kind `shape` was fixed to prevent.
fn request(kind: Kind, query: &str) -> Request<Kind> {
    Request {
        provider: ID,
        shape: shape::of(&kind),
        kind,
        query: query.trim().to_string(),
    }
}

/// A platform account is an identity, and each platform is a different property. Adding one is a row
/// in this table, not a new provider.
///
/// Every property here was verified against Wikidata on 2026-11-10. `other` stays unmapped: a wrong
/// property would produce confident, fabricated identities, which is worse than leaving the entity
/// unresolved. Reddit has two properties — a username (`P4265`) and a subreddit (`P3984`) — chosen by
/// the handle's `u/`/`r/` prefix in [`handle`].
pub(super) fn handle_property(platform: &str) -> Option<&'static str> {
    match platform.to_ascii_lowercase().as_str() {
        "x" | "twitter" | "x_twitter" => Some("P2002"),
        "instagram" => Some("P2003"),
        "tiktok" => Some("P7085"),
        "youtube" => Some("P2397"),
        "github" => Some("P2037"),
        "mastodon" => Some("P4033"),
        "facebook" => Some("P2013"),
        "linkedin" => Some("P6634"),
        "threads" => Some("P11892"),
        "bluesky" => Some("P12361"),
        "reddit" => Some("P4265"),
        _ => None,
    }
}

/// One step of the ladder, and which decision applies to its answer.
///
/// The steps are request kinds, not bundled passes: the resolver asks one step across every
/// unresolved entity, parses it, and only then plans the next. A title that resolves an entity
/// removes its label and search questions entirely.
///
/// One definition, used by the resolver and by the eval replay: a step that exists in one and not
/// the other is how `eval` came to report claims the resolver would not make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// A platform account property (`P2002`, …).
    Handle,
    /// A Wikipedia article title.
    Title,
    /// An exact Wikidata label.
    Label,
    /// Full-text search over the disambiguation hint.
    SearchHint,
    /// Full-text search over the bare label.
    SearchLabel,
    /// The article bearing this exact name, after every step above has refused. Claims through
    /// [`super::parse::surface_only`], not through the ranking the other steps use.
    SurfaceTitle,
}

impl Step {
    /// The ladder's order. `SurfaceTitle` reuses the title answer after every other step refused.
    pub(crate) const ORDER: [Self; 6] = [
        Self::Handle,
        Self::Title,
        Self::Label,
        Self::SearchHint,
        Self::SearchLabel,
        Self::SurfaceTitle,
    ];
}

/// The whole ladder for one entity, in the order the resolver climbs it.
pub(crate) fn ladder(entity: &model::knowledge_entity::Model) -> Vec<(Step, Request<Kind>)> {
    Step::ORDER
        .into_iter()
        .flat_map(|s| {
            step(s, entity)
                .into_iter()
                .map(move |request| (s, request))
        })
        .collect()
}

/// One step's requests for one entity. Pure, so the order is testable without a network.
pub(crate) fn step(step: Step, entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    match step {
        Step::Handle => {
            let Some(handle) = handle(entity) else {
                return Vec::new();
            };
            vec![handle]
        }
        Step::Title | Step::SurfaceTitle => title(entity).into_iter().collect(),
        Step::Label => label(entity).into_iter().collect(),
        Step::SearchHint => search_hint(entity).into_iter().collect(),
        Step::SearchLabel => search_label(entity).into_iter().collect(),
    }
}

fn handle(entity: &model::knowledge_entity::Model) -> Option<Request<Kind>> {
    let (platform, handle) = (entity.platform.as_deref()?, entity.platform_handle.as_deref()?);
    // A capture that names a platform but no handle is a platform we cannot ask about: an empty
    // `VALUES` literal would be a wasted call and a cached negative about nothing.
    let platform = platform.to_ascii_lowercase();
    let handle = handle.trim();
    if handle.is_empty() {
        return None;
    }
    let (property, query) = if platform == "reddit" {
        if let Some(subreddit) = handle.strip_prefix("r/") {
            ("P3984", subreddit)
        } else {
            ("P4265", handle.strip_prefix("u/").unwrap_or(handle))
        }
    } else {
        (handle_property(&platform)?, handle)
    };
    if query.is_empty() {
        return None;
    }
    Some(request(Kind::Handle(property), query))
}

fn label(entity: &model::knowledge_entity::Model) -> Option<Request<Kind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(Kind::Label, label))
}

/// The bare label as a search fallback: Wikidata labels `J. R. R. Tolkien` with spaces, while the
/// exact-label SPARQL is sent the capture's punctuation.
fn search_label(entity: &model::knowledge_entity::Model) -> Option<Request<Kind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(Kind::Search, label))
}

/// The disambiguating hint, when it says something the name does not.
///
/// Search is a lookup, so the hint runs before the bare label. A hint that only repeats the name is
/// not a second request.
fn search_hint(entity: &model::knowledge_entity::Model) -> Option<Request<Kind>> {
    let label = entity.name.trim();
    let hint = entity.search_query.as_deref()?.trim();
    if label.is_empty()
        || hint.is_empty()
        || normalize::comparable(hint) == normalize::comparable(label)
    {
        return None;
    }
    Some(request(Kind::Search, hint))
}

/// The old bundled first pass, kept for the planning tests.
#[cfg(test)]
pub(crate) fn for_entity(entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    [Step::Handle, Step::Title, Step::Label]
        .into_iter()
        .flat_map(|s| step(s, entity))
        .collect()
}

/// The article whose exact name this is, when the name is not empty.
///
/// Two steps: the title lookup, and the surface-only last resort that reuses its cached answer.
pub(crate) fn title(entity: &model::knowledge_entity::Model) -> Option<Request<Kind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(Kind::Title, label))
}

/// The old bundled search pass, kept for the planning tests.
#[cfg(test)]
pub(crate) fn fallback(entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    [Step::SearchHint, Step::SearchLabel]
        .into_iter()
        .flat_map(|s| step(s, entity))
        .collect()
}
