//! Contradictions inside one call, checked before the transaction opens.
//!
//! The executor applies mutations in canonical order. That is the right answer for independent
//! work, but it hides a plan that asks for both sides of one decision. This module owns the small
//! set of direct contradictions; everything else remains canonical-order behavior.

use std::collections::{HashMap, HashSet};

use crate::knowledge::ids::{EntityOriginId, ObservationId};
use crate::knowledge::mutation::GraphMutation;

use super::report::Refusal;

type EntityPair = (ObservationId, ObservationId);
type IdentityKey = (EntityOriginId, String, String);

fn pair(a: EntityOriginId, b: EntityOriginId) -> EntityPair {
    let (a, b) = (a.0, b.0);
    if a.0 <= b.0 { (a, b) } else { (b, a) }
}

fn identity(subject: EntityOriginId, scheme: &str, external_id: &str) -> IdentityKey {
    (subject, scheme.to_string(), external_id.to_string())
}

/// The first direct contradiction in the plan, if any.
pub(super) fn contradiction(mutations: &[GraphMutation]) -> Option<Refusal> {
    let mut merges = HashSet::new();
    let mut distincts = HashSet::new();
    let mut identified = HashSet::new();
    let mut vetoed = HashSet::new();
    let mut attached: HashMap<ObservationId, EntityOriginId> = HashMap::new();
    let mut detached = HashSet::new();

    for mutation in mutations {
        match mutation {
            GraphMutation::MergeEntity { loser, winner, .. } => {
                if loser == winner {
                    return Some(Refusal::Contradiction("an entity cannot merge into itself"));
                }
                let pair = pair(*loser, *winner);
                if distincts.contains(&pair) {
                    return Some(Refusal::Contradiction(
                        "cannot merge and mark distinct the same pair",
                    ));
                }
                merges.insert(pair);
            }
            GraphMutation::MarkDistinct { a, b, .. } => {
                if a == b {
                    return Some(Refusal::Contradiction(
                        "an entity cannot be distinct from itself",
                    ));
                }
                let pair = pair(*a, *b);
                if merges.contains(&pair) {
                    return Some(Refusal::Contradiction(
                        "cannot merge and mark distinct the same pair",
                    ));
                }
                distincts.insert(pair);
            }
            GraphMutation::Identify { subject, identity: id } => {
                let key = identity(*subject, &id.scheme, &id.external_id);
                if vetoed.contains(&key) {
                    return Some(Refusal::Contradiction(
                        "cannot identify and veto the same key",
                    ));
                }
                identified.insert(key);
            }
            GraphMutation::VetoIdentity { subject, key, .. } => {
                let key = identity(*subject, &key.scheme, &key.external_id);
                if identified.contains(&key) {
                    return Some(Refusal::Contradiction(
                        "cannot identify and veto the same key",
                    ));
                }
                vetoed.insert(key);
            }
            GraphMutation::AttachObservation {
                observation,
                entity,
                ..
            } => {
                if detached.contains(observation) {
                    return Some(Refusal::Contradiction(
                        "cannot attach and detach the same observation",
                    ));
                }
                if let Some(previous) = attached.get(observation) {
                    if previous != entity {
                        return Some(Refusal::Contradiction(
                            "cannot attach one observation to two entities",
                        ));
                    }
                } else {
                    attached.insert(*observation, *entity);
                }
            }
            GraphMutation::DetachObservation { observation, .. } => {
                if attached.contains_key(observation) {
                    return Some(Refusal::Contradiction(
                        "cannot attach and detach the same observation",
                    ));
                }
                detached.insert(*observation);
            }
            GraphMutation::CreateEntity { .. }
            | GraphMutation::RetractIdentity { .. }
            | GraphMutation::UnvetoIdentity { .. } => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::identity::{Identity, Provider};
    use crate::knowledge::mutation::Cause;

    fn origin(id: i32) -> EntityOriginId {
        EntityOriginId(ObservationId(id))
    }

    fn assertion(scheme: &str, external_id: &str) -> Identity {
        Identity {
            provider: Provider("wikidata".into()),
            scheme: scheme.into(),
            external_id: external_id.into(),
            method: Some("test".into()),
            confidence: Some(0.9),
            label: None,
            description: None,
            classes: Vec::new(),
            observation_id: None,
        }
    }

    #[test]
    fn merge_and_distinct_conflict_in_either_order() {
        let merge = GraphMutation::MergeEntity {
            loser: origin(1),
            winner: origin(2),
            cause: Cause::Repair,
            why: "same".into(),
            record_verdict: true,
        };
        let distinct = GraphMutation::MarkDistinct {
            a: origin(2),
            b: origin(1),
            cause: Cause::Repair,
            why: "different".into(),
        };

        assert!(contradiction(&[merge.clone(), distinct.clone()]).is_some());
        assert!(contradiction(&[distinct, merge]).is_some());
    }

    #[test]
    fn identify_and_veto_conflict_for_one_key() {
        let identify = GraphMutation::Identify {
            subject: origin(1),
            identity: assertion("wikidata", "Q1"),
        };
        let veto = GraphMutation::VetoIdentity {
            subject: origin(1),
            key: crate::knowledge::IdentityKey {
                scheme: "wikidata".into(),
                external_id: "Q1".into(),
            },
            cause: Cause::Repair,
            why: "wrong".into(),
            record_verdict: true,
        };

        assert!(contradiction(&[identify, veto]).is_some());
    }

    #[test]
    fn attach_and_detach_conflict_for_one_observation() {
        let attach = GraphMutation::AttachObservation {
            observation: ObservationId(3),
            entity: origin(1),
            with: None,
            why: None,
        };
        let detach = GraphMutation::DetachObservation {
            observation: ObservationId(3),
            why: None,
        };

        assert!(contradiction(&[attach, detach]).is_some());
    }

    #[test]
    fn one_observation_cannot_attach_to_two_entities() {
        let first = GraphMutation::AttachObservation {
            observation: ObservationId(3),
            entity: origin(1),
            with: None,
            why: None,
        };
        let second = GraphMutation::AttachObservation {
            observation: ObservationId(3),
            entity: origin(2),
            with: None,
            why: None,
        };

        assert!(contradiction(&[first, second]).is_some());
    }

    #[test]
    fn independent_mutations_are_not_contradictory() {
        let mutations = [
            GraphMutation::MergeEntity {
                loser: origin(1),
                winner: origin(2),
                cause: Cause::Repair,
                why: "same".into(),
                record_verdict: true,
            },
            GraphMutation::Identify {
                subject: origin(3),
                identity: assertion("wikidata", "Q3"),
            },
            GraphMutation::AttachObservation {
                observation: ObservationId(4),
                entity: origin(1),
                with: None,
                why: None,
            },
        ];

        assert!(contradiction(&mutations).is_none());
    }
}
