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
/// Every property here was verified against Wikidata on 2026-10-31. Unmapped platforms (`threads`,
/// `reddit`, `other`) are deliberate: a wrong property would produce confident, fabricated
/// identities, which is worse than leaving the entity unresolved.
pub(super) fn handle_property(platform: &str) -> Option<&'static str> {
    match platform.to_ascii_lowercase().as_str() {
        "x" | "twitter" | "x_twitter" => Some("P2002"),
        "instagram" => Some("P2003"),
        "tiktok" => Some("P7085"),
        "youtube" => Some("P2397"),
        "github" => Some("P2037"),
        "mastodon" => Some("P4033"),
        _ => None,
    }
}

/// A rung of the ladder, and which decision applies to its answer.
///
/// One definition, used by the resolver and by both eval replays: a rung that exists in one and not
/// the others is how `eval` came to report claims the resolver would not make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rung {
    /// Handle, title and exact label — corroborating evidence, ranked by description.
    Corroborating,
    /// Full-text search over the hint and then the bare label.
    Search,
    /// The article bearing this exact name, after every rung above has refused. Claims through
    /// [`super::policy::surface_only`], not through the ranking the other rungs use.
    SurfaceOnly,
}

/// The whole ladder for one entity, in the order the resolver climbs it.
pub(crate) fn ladder(entity: &model::knowledge_entity::Model) -> Vec<(Rung, Request<Kind>)> {
    stage(Rung::Corroborating, entity)
        .into_iter()
        .map(|request| (Rung::Corroborating, request))
        .chain(
            stage(Rung::Search, entity)
                .into_iter()
                .map(|request| (Rung::Search, request)),
        )
        .chain(
            stage(Rung::SurfaceOnly, entity)
                .into_iter()
                .map(|request| (Rung::SurfaceOnly, request)),
        )
        .collect()
}

/// One rung's requests, for a caller that batches a whole entity set per rung.
pub(crate) fn stage(rung: Rung, entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    match rung {
        Rung::Corroborating => for_entity(entity),
        Rung::Search => fallback(entity),
        Rung::SurfaceOnly => title(entity).into_iter().collect(),
    }
}

/// The first pass: a handle, then a title, then an exact label. Pure, so the order is testable
/// without a network — and reachable without one: the eval replays re-run the whole decision over
/// cached payloads.
pub(crate) fn for_entity(entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    let mut requests = Vec::new();
    if let (Some(platform), Some(handle)) = (&entity.platform, &entity.platform_handle) {
        // A capture that names a platform but no handle is a platform we cannot ask about: an
        // empty `VALUES` literal would be a wasted call and a cached negative about nothing.
        let handle = handle.trim();
        if !handle.is_empty()
            && let Some(property) = handle_property(platform)
        {
            requests.push(request(Kind::Handle(property), handle));
        }
    }
    if let Some(title) = title(entity) {
        // Title before label: a capture's name is often the *article* title
        // (`Symphony No. 7 (Sibelius)`), and the item is labelled `Symphony No. 7`. The label
        // lookup cannot see that; the title can.
        requests.push(title);
        requests.push(request(Kind::Label, &entity.name));
    }
    requests
}

/// The article whose exact name this is, when the name is not empty.
///
/// Two callers: the second rung of the first pass, and the last resort after every corroborating
/// question has been refused.
pub(crate) fn title(entity: &model::knowledge_entity::Model) -> Option<Request<Kind>> {
    let label = entity.name.trim();
    (!label.is_empty()).then(|| request(Kind::Title, label))
}

/// The second pass: search for the **hint**, then the **bare label**, for entities the first pass
/// left unresolved.
///
/// The hint goes first because it is the phrase that disambiguates, and until now it was only a
/// *scoring* signal — consulted against candidates some other question had already produced. It is
/// a lookup, though, and `wbsearchentities` matches labels and aliases rather than text, so a long
/// hint (`Commission of Tears Antonio Lobo Antunes`) usually returns nothing. That is a limit of
/// the endpoint, not a reason to withhold the question: over-fetching is the resolver's job, and
/// refusing a wrong candidate is the decider's.
///
/// The bare label stays because it is a lookup fallback, not a ranking signal. Wikidata labels
/// `J. R. R. Tolkien` and `D. H. Lawrence` *with spaces*; our exact-label SPARQL sends
/// `J.R.R. Tolkien` and gets nothing, while search finds Q892 immediately. Measured on 20 random
/// empty-label entities: 4 recoverable this way.
///
/// The pass boundary lives with the ladder, not in the engine: no other authority knows that search
/// is the expensive route.
pub(crate) fn fallback(entity: &model::knowledge_entity::Model) -> Vec<Request<Kind>> {
    let label = entity.name.trim();
    if label.is_empty() {
        return Vec::new();
    }
    let mut requests = Vec::new();
    if let Some(hint) = entity.search_query.as_deref() {
        let hint = hint.trim();
        // A hint that only repeats the name is not a second request.
        if !hint.is_empty() && normalize::comparable(hint) != normalize::comparable(label) {
            requests.push(request(Kind::Search, hint));
        }
    }
    requests.push(request(Kind::Search, label));
    requests
}
