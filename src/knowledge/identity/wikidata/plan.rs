//! What to ask: the ladder's requests, as data.
//!
//! Pure and stateless — no client, no `self`. The ladder in `resolver` decides *when* to ask and
//! what an answer means; this decides only *what*.

use super::property_query::platform_link_queries;
use super::shape;
use super::Kind;
use super::ID;
use crate::fetch::Request;
use crate::illumination::v2::PlatformLink;
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
    /// A platform link's handle, through the property that holds it (`P2002`, …).
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
        Step::Handle => handle(entity),
        Step::Title | Step::SurfaceTitle => title(entity).into_iter().collect(),
        Step::Label => label(entity).into_iter().collect(),
        Step::SearchHint => search_hint(entity).into_iter().collect(),
        Step::SearchLabel => search_label(entity).into_iter().collect(),
    }
}

/// The entity's projected platform link, one request per value the link's property expects.
///
/// `project` copies one member observation's `PlatformLink` onto the entity, so this reads a single
/// presence: the resolver never sees a second member's link, and one presence names one referent.
fn handle(entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    let Some(link) = PlatformLink::from_columns(
        entity.platform.clone(),
        entity.platform_handle.clone(),
        entity.platform_display_name.clone(),
    ) else {
        return Vec::new();
    };
    platform_link_queries(&link)
        .into_iter()
        .map(|query| request(Kind::Handle(query.property.as_str()), &query.value))
        .collect()
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
