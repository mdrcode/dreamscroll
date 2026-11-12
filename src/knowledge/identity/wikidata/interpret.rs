//! What an answer means: the gates every candidate passes, and the rung that speaks for it.
//!
//! Pure: the same entity plus the same answer yields the same claim, so the whole decision can be
//! re-run over cached payloads with no transport. Nothing here knows which wire produced the answer
//! — `transport` already parsed the JSON into [`Candidate`]s, which is why nothing here is called
//! `parse`.

use super::answer;
use super::policy;
use super::ranking::HANDLE_CONFIDENCE;
use super::types;
use super::{Candidate, RequestKind};
use crate::fetch::{Answer, Request};
use crate::knowledge::evidence::Identity;
use crate::model;

/// The one claim this answer supports, if any. Pure: same entity plus same answer, same claim.
///
/// At most one, always — every rung answers about a single referent, so the `Vec` this used to
/// return pretended the question was plural.
pub(crate) fn claim(
    entity: &model::knowledge_entity::Model,
    request: &Request<RequestKind>,
    raw: &Answer<Vec<Candidate>>,
) -> Option<Identity> {
    let mut reason = "";
    claim_explained(entity, request, raw, &mut reason)
}

/// The gates every answer passes before a rung decides: a menu is not a referent, and the right
/// name can still be the wrong kind of thing. Only recognised classes reject — an unknown one
/// passes (see `types`), so the table being incomplete cannot lose a candidate.
///
/// Split out because the last-resort pass needs the same gates without a rung's decision.
pub(crate) fn admit(
    entity: &model::knowledge_entity::Model,
    raw: &Answer<Vec<Candidate>>,
) -> Result<Vec<Candidate>, &'static str> {
    if !raw.outcome.answered() {
        return Err("the answer carried no candidate");
    }
    let mut candidates = raw.body.clone().unwrap_or_default();
    // A menu is not a referent, on *every* rung. This used to be checked in `parse_title` only,
    // and the label rung — which is where all 34 such claims in the corpus came from — never
    // looked. Filtering at the gate makes the rule a property of an answer rather than of one
    // caller.
    candidates.retain(|candidate| !answer::is_menu(candidate));
    if candidates.is_empty() {
        return Err("the answer carried only a disambiguation page");
    }
    // `Dogma` the film against the concept, `Micah` the prophet against the given name.
    candidates.retain(|candidate| !types::incompatible(&entity.entity_type, &candidate.classes));
    if candidates.is_empty() {
        return Err("every candidate is a different kind of thing");
    }
    Ok(candidates)
}

/// The last resort: the article bearing this exact name, when every corroborating rung refused.
///
/// Not a ranking decision — a surface this exact is better than nothing — which is why it is a
/// separate entry point rather than an arm of [`claim_explained`].
pub(crate) fn surface_only(
    entity: &model::knowledge_entity::Model,
    candidates: &[Candidate],
    request: &Request<RequestKind>,
) -> Option<Identity> {
    policy::surface_only(entity, candidates, request)
}

/// [`claim`], and *why* it refused. `reason` is empty when a claim came back.
///
/// The refusal is recorded where it happens rather than inferred afterwards: an inspector that
/// guessed at the gate would be a second implementation of the gates.
pub(crate) fn claim_explained(
    entity: &model::knowledge_entity::Model,
    request: &Request<RequestKind>,
    raw: &Answer<Vec<Candidate>>,
    reason: &mut &'static str,
) -> Option<Identity> {
    // `admit` is the one gate, and it already refuses an answer that carried nothing — the reason
    // it records is the reason reported here.
    let candidates = match admit(entity, raw) {
        Ok(candidates) => candidates,
        Err(gate) => {
            *reason = gate;
            return None;
        }
    };
    match request.kind {
        // A property match *is* the identity: an item claiming this handle is the account itself.
        RequestKind::Handle(property) => match candidates.first() {
            Some(candidate) => Some(answer::identity(candidate, property, HANDLE_CONFIDENCE)),
            None => {
                *reason = "the handle answer carried no item";
                None
            }
        },
        RequestKind::Title => policy::parse_title(entity, &candidates, request, reason),
        RequestKind::Search => policy::parse_search(entity, &candidates, request, reason),
        // A label is a ranking question.
        RequestKind::Label => policy::decide(entity, &candidates, request, reason),
    }
}
