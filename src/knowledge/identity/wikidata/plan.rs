//! What to ask: the ladder's requests, as data.
//!
//! Pure and stateless — no client, no `self`. The ladder in `resolver` decides *when* to ask and
//! what an answer means; this decides only *what*.

use super::ID;
use super::RequestKind;
use super::property_query::platform_link_queries;
use super::shape;
use crate::fetch::Request;
use crate::illumination::v2::PlatformLink;
use crate::knowledge::normalize;
use crate::model;

/// One request, carrying the shape that produced it.
///
/// The only place requests are built here, so a kind and its shape cannot disagree — the failure the
/// per-kind `shape` was fixed to prevent.
fn request(kind: RequestKind, query: &str) -> Request<RequestKind> {
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
/// One definition, used by the resolver and by `inspect`: a step that exists in one and not the
/// other is what lets a diagnostic report claims the resolver would not make.
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
    /// **Not a question — a decision wearing a step's clothes.** It plans the *identical* `Request`
    /// as [`Title`](Self::Title), so it asks nothing new: the cached answer is decided a second way,
    /// taking the article bearing this exact name without ranking. It is a step only so the
    /// deferral is visible as data; a `Step` should not decide anything, and this one does.
    ///
    /// HACK, kept deliberately until the ladder's shape is settled: it costs a branch in
    /// `resolver::ladder` and a matching one in `inspect`. The shape it wants to become is a
    /// post-pass over the title answer after every real question has been asked — see R6 in
    /// `plan/wikidata-cleanup-20261112.md`. Claims through [`super::interpret::surface_only`], not
    /// through the ranking the other steps use.
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
pub(crate) fn ladder(entity: &model::knowledge_entity::Model) -> Vec<(Step, Request<RequestKind>)> {
    Step::ORDER
        .into_iter()
        .flat_map(|rung| {
            step(rung, entity)
                .into_iter()
                .map(move |request| (rung, request))
        })
        .collect()
}

/// One step's requests for one entity. Pure, so the order is testable without a network.
pub(crate) fn step(
    step: Step,
    entity: &model::knowledge_entity::Model,
) -> Vec<Request<RequestKind>> {
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
fn handle(entity: &model::knowledge_entity::Model) -> Vec<Request<RequestKind>> {
    let Some(link) = PlatformLink::from_columns(
        entity.platform.clone(),
        entity.platform_handle.clone(),
        entity.platform_display_name.clone(),
    ) else {
        return Vec::new();
    };
    platform_link_queries(&link)
        .into_iter()
        .map(|query| request(RequestKind::Handle(query.property.as_str()), &query.value))
        .collect()
}

fn label(entity: &model::knowledge_entity::Model) -> Option<Request<RequestKind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(RequestKind::Label, label))
}

/// The bare label as a search fallback: Wikidata labels `J. R. R. Tolkien` with spaces, while the
/// exact-label SPARQL is sent the capture's punctuation.
fn search_label(entity: &model::knowledge_entity::Model) -> Option<Request<RequestKind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(RequestKind::Search, label))
}

/// The disambiguating hint, when it says something the name does not.
///
/// Search is a lookup, so the hint runs before the bare label. A hint that only repeats the name is
/// not a second request.
fn search_hint(entity: &model::knowledge_entity::Model) -> Option<Request<RequestKind>> {
    let label = entity.name.trim();
    let hint = entity.search_query.as_deref()?.trim();
    if label.is_empty()
        || hint.is_empty()
        || normalize::comparable(hint) == normalize::comparable(label)
    {
        return None;
    }
    Some(request(RequestKind::Search, hint))
}

/// The article whose exact name this is, when the name is not empty.
///
/// Two steps: the title lookup, and the surface-only last resort that reuses its cached answer.
fn title(entity: &model::knowledge_entity::Model) -> Option<Request<RequestKind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(RequestKind::Title, label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixtures::stub_entity;

    fn entity(label: &str) -> model::knowledge_entity::Model {
        stub_entity(1, label, "unknown")
    }

    /// The order is the policy: a handle is stronger than a name, a title is unique where a label is
    /// not, and the last resort re-asks the title question so it can be answered from the cache.
    #[test]
    fn the_ladder_asks_a_handle_then_a_title_then_a_label_then_search() {
        let mut entity = stub_entity(1, "Alexey Kutepov", "real_person");
        entity.search_query = Some("Tsoding".to_string());
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("tsoding".to_string());

        let asked: Vec<(Step, RequestKind, String)> = ladder(&entity)
            .into_iter()
            .map(|(step, request)| (step, request.kind, request.query))
            .collect();

        assert_eq!(
            asked,
            vec![
                (
                    Step::Handle,
                    RequestKind::Handle("P2002"),
                    "tsoding".to_string()
                ),
                (
                    Step::Title,
                    RequestKind::Title,
                    "Alexey Kutepov".to_string()
                ),
                (
                    Step::Label,
                    RequestKind::Label,
                    "Alexey Kutepov".to_string()
                ),
                (Step::SearchHint, RequestKind::Search, "Tsoding".to_string()),
                (
                    Step::SearchLabel,
                    RequestKind::Search,
                    "Alexey Kutepov".to_string()
                ),
                (
                    Step::SurfaceTitle,
                    RequestKind::Title,
                    "Alexey Kutepov".to_string()
                ),
            ]
        );
    }

    /// A capture can name the platform and not the handle. Asking `VALUES { "" }` spends a call to
    /// cache a negative about nobody, so the rung is skipped and the name rungs still run.
    #[test]
    fn a_platform_without_a_handle_asks_nothing() {
        let mut entity = stub_entity(1, "Alexey Kutepov", "real_person");
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("   ".to_string());

        assert!(step(Step::Handle, &entity).is_empty());
        assert_eq!(step(Step::Title, &entity).len(), 1);
        assert_eq!(step(Step::Label, &entity).len(), 1);
    }

    /// The database string is the seam that fails silently: a platform spelled differently parses to
    /// `Platform::Other` and asks nothing at all, which is indistinguishable from "not on Wikidata".
    /// The property and its value form are `property_query`'s; this is the string reaching it.
    #[test]
    fn every_platform_string_reaches_the_property_that_holds_its_handle() {
        for (platform, handle, property) in [
            ("x_twitter", "tsoding", RequestKind::Handle("P2002")),
            ("youtube", "@MeatCanyon", RequestKind::Handle("P11245")),
            (
                "youtube",
                "UC_x5XG1OV2P6uZZ5FSM9Ttw",
                RequestKind::Handle("P2397"),
            ),
            ("reddit", "r/NFCWestMemeWar", RequestKind::Handle("P3984")),
            ("reddit", "u/spez", RequestKind::Handle("P4265")),
            ("threads", "@ada", RequestKind::Handle("P11892")),
            (
                "mastodon",
                "@user@mastodon.social",
                RequestKind::Handle("P4033"),
            ),
        ] {
            let mut entity = entity("someone");
            entity.platform = Some(platform.to_string());
            entity.platform_handle = Some(handle.to_string());

            let asked = step(Step::Handle, &entity);
            assert!(
                asked.iter().any(|request| request.kind == property),
                "{platform}/{handle}: {asked:?}"
            );
        }

        // An unmapped platform asks nothing rather than borrowing another platform's property.
        let mut unknown = entity("someone");
        unknown.platform = Some("myspace".to_string());
        unknown.platform_handle = Some("tom".to_string());
        assert!(step(Step::Handle, &unknown).is_empty());
    }

    /// The hint is a *query*, not only a scoring signal: `Commission of Tears` alone finds meniscal
    /// tears, and the author in the hint is what would find the novel.
    #[test]
    fn the_hint_is_searched_before_the_bare_label() {
        let mut target = entity("Commission of Tears");
        target.search_query = Some("Commission of Tears Antonio Lobo Antunes".to_string());

        let hints = step(Step::SearchHint, &target);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].query, "Commission of Tears Antonio Lobo Antunes");
        assert_eq!(
            step(Step::SearchLabel, &target)[0].query,
            "Commission of Tears"
        );
    }

    /// A hint that only repeats the name is not a second request.
    #[test]
    fn a_hint_equal_to_the_label_is_not_asked_twice() {
        let mut target = entity("Tsoding");
        target.search_query = Some("  TSODING ".to_string());

        assert!(step(Step::SearchHint, &target).is_empty());
        assert_eq!(step(Step::SearchLabel, &target)[0].query, "Tsoding");
    }
}
