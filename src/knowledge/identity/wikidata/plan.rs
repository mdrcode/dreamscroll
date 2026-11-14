//! What to ask: the plan's requests, as data.
//!
//! Pure and stateless — no client, no `self`. `resolver` decides *when* to ask and what an answer
//! means; this decides only *what*.

use super::ID;
use super::WikidataKind;
use super::property_query::platform_link_queries;
use crate::fetch::Request;
use crate::illumination::v2::PlatformLink;
use crate::knowledge::text;
use crate::model;

/// One request, carrying the shape that produced it. Only place requests are built here.
fn request(kind: WikidataKind, query: &str) -> Request<WikidataKind> {
    Request {
        provider: ID,
        shape: kind.shape(),
        kind,
        query: query.trim().to_string(),
    }
}

/// One strategy of the plan, and which decision applies to its answer.
///
/// The strategies are ways of asking, not bundled passes: the resolver applies every strategy to
/// every entity in one batch, and decides once over all the answers. A strategy that finds nothing
/// costs its answer and nothing else, so the plan stays complete even when a strategy is redundant.
///
/// One definition, used by the resolver and by `inspect`: a strategy that exists in one and not the
/// other is what lets a diagnostic report identities the resolver would not make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Strategy {
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
}

impl Strategy {
    /// Every strategy, in the order of its authority. The resolver applies all of them — this
    /// array is enumeration, not a sequence — and the order documents what each way of asking can
    /// prove: a platform property and an article title name the item, a label is a name, and search
    /// is a guess.
    pub(crate) const ALL: [Self; 5] = [
        Self::Handle,
        Self::Title,
        Self::Label,
        Self::SearchHint,
        Self::SearchLabel,
    ];
}

/// Every strategy's requests for one entity, in [`Strategy::ALL`] — a flattening, not a one-to-one
/// map: one strategy can yield several requests (a platform link asks a query per value it may
/// hold).
///
/// The resolver asks every strategy for every entity in one batch; this per-entity view is what
/// `inspect` prints.
pub(crate) fn asks(
    entity: &model::knowledge_entity::Model,
) -> Vec<(Strategy, Request<WikidataKind>)> {
    Strategy::ALL
        .into_iter()
        .flat_map(|each| {
            requests(each, entity)
                .into_iter()
                .map(move |request| (each, request))
        })
        .collect()
}

/// One strategy's requests for one entity. Pure, so the order is testable without a network.
pub(crate) fn requests(
    strategy: Strategy,
    entity: &model::knowledge_entity::Model,
) -> Vec<Request<WikidataKind>> {
    match strategy {
        Strategy::Handle => handle(entity),
        Strategy::Title => title(entity).into_iter().collect(),
        Strategy::Label => label(entity).into_iter().collect(),
        Strategy::SearchHint => search_hint(entity).into_iter().collect(),
        Strategy::SearchLabel => search_label(entity).into_iter().collect(),
    }
}

/// The entity's projected platform link, one request per value the link's property expects.
///
/// `project` copies one member observation's `PlatformLink` onto the entity, so this reads a single
/// presence: the resolver never sees a second member's link, and one presence names one referent.
fn handle(entity: &model::knowledge_entity::Model) -> Vec<Request<WikidataKind>> {
    let Some(link) = PlatformLink::from_columns(
        entity.platform.clone(),
        entity.platform_handle.clone(),
        entity.platform_display_name.clone(),
    ) else {
        return Vec::new();
    };
    platform_link_queries(&link)
        .into_iter()
        .map(|query| request(WikidataKind::Handle(query.property), &query.value))
        .collect()
}

fn label(entity: &model::knowledge_entity::Model) -> Option<Request<WikidataKind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(WikidataKind::Label, label))
}

/// The bare label as a search fallback: Wikidata labels `J. R. R. Tolkien` with spaces, while the
/// exact-label SPARQL is sent the entity's punctuation.
fn search_label(entity: &model::knowledge_entity::Model) -> Option<Request<WikidataKind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(WikidataKind::Search, label))
}

/// The disambiguating hint, when it says something the name does not.
///
/// Search is a lookup, so the hint runs before the bare label. A hint that only repeats the name is
/// not a second request.
fn search_hint(entity: &model::knowledge_entity::Model) -> Option<Request<WikidataKind>> {
    let label = entity.name.trim();
    let hint = entity.search_query.as_deref()?.trim();
    if label.is_empty() || hint.is_empty() || text::comparable(hint) == text::comparable(label) {
        return None;
    }
    Some(request(WikidataKind::Search, hint))
}

/// The article whose exact name this is, when the name is not empty.
///
/// Two steps: the title lookup, and the surface-only last resort that reuses its cached answer.
fn title(entity: &model::knowledge_entity::Model) -> Option<Request<WikidataKind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(WikidataKind::Title, label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::identity::wikidata::property_query::PropertyId;
    use crate::test_support::fixtures::stub_entity;

    fn entity(label: &str) -> model::knowledge_entity::Model {
        stub_entity(1, label, "unknown")
    }

    /// The array documents authority, not a sequence: a handle is stronger than a name, a title is
    /// unique where a label is not, and search is the guess that runs beside them, not after them.
    #[test]
    fn the_strategies_name_a_handle_a_title_a_label_then_search() {
        let mut entity = stub_entity(1, "Alexey Kutepov", "real_person");
        entity.search_query = Some("Tsoding".to_string());
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("tsoding".to_string());

        let asked: Vec<(Strategy, WikidataKind, String)> = asks(&entity)
            .into_iter()
            .map(|(strategy, request)| (strategy, request.kind, request.query))
            .collect();

        assert_eq!(
            asked,
            vec![
                (
                    Strategy::Handle,
                    WikidataKind::Handle(PropertyId::X_USERNAME),
                    "tsoding".to_string()
                ),
                (
                    Strategy::Title,
                    WikidataKind::Title,
                    "Alexey Kutepov".to_string()
                ),
                (
                    Strategy::Label,
                    WikidataKind::Label,
                    "Alexey Kutepov".to_string()
                ),
                (
                    Strategy::SearchHint,
                    WikidataKind::Search,
                    "Tsoding".to_string()
                ),
                (
                    Strategy::SearchLabel,
                    WikidataKind::Search,
                    "Alexey Kutepov".to_string()
                ),
            ]
        );
    }

    /// An entity can carry a platform with no handle. Asking `VALUES { "" }` spends a call to
    /// cache a negative about nobody, so that strategy is skipped and the name strategies still run.
    #[test]
    fn a_platform_without_a_handle_asks_nothing() {
        let mut entity = stub_entity(1, "Alexey Kutepov", "real_person");
        entity.platform = Some("x_twitter".to_string());
        entity.platform_handle = Some("   ".to_string());

        assert!(requests(Strategy::Handle, &entity).is_empty());
        assert_eq!(requests(Strategy::Title, &entity).len(), 1);
        assert_eq!(requests(Strategy::Label, &entity).len(), 1);
    }

    /// The database string is the seam that fails silently: a platform spelled differently parses to
    /// `Platform::Other` and asks nothing at all, which is indistinguishable from "not on Wikidata".
    /// The property and its value form are `property_query`'s; this is the string reaching it.
    #[test]
    fn every_platform_string_reaches_the_property_that_holds_its_handle() {
        for (platform, handle, property) in [
            (
                "x_twitter",
                "tsoding",
                WikidataKind::Handle(PropertyId::X_USERNAME),
            ),
            (
                "youtube",
                "@MeatCanyon",
                WikidataKind::Handle(PropertyId::YOUTUBE_HANDLE),
            ),
            (
                "youtube",
                "UC_x5XG1OV2P6uZZ5FSM9Ttw",
                WikidataKind::Handle(PropertyId::YOUTUBE_CHANNEL),
            ),
            (
                "reddit",
                "r/NFCWestMemeWar",
                WikidataKind::Handle(PropertyId::SUBREDDIT),
            ),
            (
                "reddit",
                "u/spez",
                WikidataKind::Handle(PropertyId::REDDIT_USERNAME),
            ),
            (
                "threads",
                "@ada",
                WikidataKind::Handle(PropertyId::THREADS_USERNAME),
            ),
            (
                "mastodon",
                "@user@mastodon.social",
                WikidataKind::Handle(PropertyId::MASTODON_ADDRESS),
            ),
        ] {
            let mut entity = entity("someone");
            entity.platform = Some(platform.to_string());
            entity.platform_handle = Some(handle.to_string());

            let asked = requests(Strategy::Handle, &entity);
            assert!(
                asked.iter().any(|request| request.kind == property),
                "{platform}/{handle}: {asked:?}"
            );
        }

        // An unmapped platform asks nothing rather than borrowing another platform's property.
        let mut unknown = entity("someone");
        unknown.platform = Some("myspace".to_string());
        unknown.platform_handle = Some("tom".to_string());
        assert!(requests(Strategy::Handle, &unknown).is_empty());
    }

    /// The hint is a *query*, not only a scoring signal: `Commission of Tears` alone finds meniscal
    /// tears, and the author in the hint is what would find the novel.
    #[test]
    fn the_hint_is_searched_before_the_bare_label() {
        let mut target = entity("Commission of Tears");
        target.search_query = Some("Commission of Tears Antonio Lobo Antunes".to_string());

        let hints = requests(Strategy::SearchHint, &target);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].query, "Commission of Tears Antonio Lobo Antunes");
        assert_eq!(
            requests(Strategy::SearchLabel, &target)[0].query,
            "Commission of Tears"
        );
    }

    /// A hint that only repeats the name is not a second request.
    #[test]
    fn a_hint_equal_to_the_label_is_not_asked_twice() {
        let mut target = entity("Tsoding");
        target.search_query = Some("  TSODING ".to_string());

        assert!(requests(Strategy::SearchHint, &target).is_empty());
        assert_eq!(requests(Strategy::SearchLabel, &target)[0].query, "Tsoding");
    }
}
