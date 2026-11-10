//! Blocking replay: would the blocker have offered the entity an observation ended up in?
//!
//! The final graph cannot answer this by inspection — every entity is findable by its own members'
//! surfaces, so a probe with an entity's own observation is a tautology. The question is temporal:
//! *at the moment this observation arrived, was its eventual home among the candidates?*
//!
//! So observations are replayed in capture order, and a candidate only counts if the entity already
//! had an earlier member — anything created by a later observation is the future and is excluded.
//! The index is the current one, so surfaces contributed by later members remain visible: the recall
//! measured here is an **upper bound**. A miss is real; a hit may be slightly flattered.

use std::collections::{BTreeMap, HashMap, HashSet};

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::knowledge::blocker::Blocker;
use crate::knowledge::normalize;
use crate::knowledge::blocker::SqlBlocker;
use crate::model;

/// One eligible observation whose true entity was not offered.
pub struct Miss {
    pub observation: i32,
    pub name: String,
    pub expected: i32,
    pub expected_name: String,
    pub offered: usize,
    /// A merge verdict says a person later decided these are one thing. The blocker was never going
    /// to see that: the merge happened after the observation arrived. Not a blocking failure.
    pub merged: bool,
}

#[derive(Default)]
pub struct Report {
    /// Observations that already belong to an entity.
    pub observations: usize,
    /// First member of its entity: nothing existed to find, so blocking is not asked.
    pub new_entity: usize,
    /// A later member: the entity existed and should have been offered.
    pub eligible: usize,
    pub hit: usize,
    /// Eligible, and the blocker offered nothing at all.
    pub starved: usize,
    /// Eligible, and more than one entity was offered — the decider's real workload.
    pub ambiguous: usize,
    /// Distribution of candidate-set size over eligible observations.
    pub sizes: BTreeMap<usize, usize>,
    pub misses: Vec<Miss>,
    /// Observations skipped because a newer illumination for their capture exists. A superseded
    /// reading is not the surface the graph is asked about any more.
    pub superseded: usize,
    /// Recall against the surfaces of *earlier members only* — the honest number. The index above
    /// carries every member's surface, so the live probe finds an entity by the very observation
    /// being tested. This one reconstructs what the entity looked like at that moment.
    pub prior_hit: usize,
    pub prior_misses: Vec<Miss>,
    /// Misses explained by a post-hoc merge verdict — excluded from the honest recall figure.
    pub merged: usize,
}

impl Report {
    /// Recall against the current index: an upper bound, because it can see surfaces later members
    /// contributed.
    pub fn live_recall(&self) -> f32 {
        if self.eligible == 0 {
            0.0
        } else {
            self.hit as f32 * 100.0 / self.eligible as f32
        }
    }

    /// Eligible observations whose entity already existed when they arrived.
    pub fn attributable(&self) -> usize {
        self.eligible.saturating_sub(self.merged)
    }

    /// Recall against earlier members only, with post-hoc merges excluded — the honest figure.
    pub fn attributable_recall(&self) -> f32 {
        let attributable = self.attributable();
        if attributable == 0 {
            100.0
        } else {
            self.prior_hit as f32 * 100.0 / attributable as f32
        }
    }

    /// Eligible observations the earlier-member probe missed.
    pub fn misses(&self) -> usize {
        self.eligible.saturating_sub(self.prior_hit)
    }

    /// Misses a post-hoc merge does not explain.
    pub fn unexplained_misses(&self) -> usize {
        self.misses().saturating_sub(self.merged)
    }
}

pub async fn replay(db: &DbHandle) -> anyhow::Result<Report> {
    let observations = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
        .order_by_asc(model::user_observed_entity::Column::CreatedAt)
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(&db.conn)
        .await?;

    // Only a capture's newest illumination is its current reading; re-illuminating appends rather
    // than replaces, so the older occurrences are history, not questions for the blocker.
    let mut newest: HashMap<i32, i32> = HashMap::new();
    for observation in &observations {
        newest
            .entry(observation.capture_id)
            .and_modify(|current| *current = (*current).max(observation.illumination_id))
            .or_insert(observation.illumination_id);
    }
    let superseded = observations
        .iter()
        .filter(|observation| newest[&observation.capture_id] != observation.illumination_id)
        .count();
    let observations: Vec<_> = observations
        .into_iter()
        .filter(|observation| newest[&observation.capture_id] == observation.illumination_id)
        .collect();

    let names: HashMap<i32, String> = model::knowledge_entity::Entity::find()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|entity| (entity.id, entity.name))
        .collect();

    // Position of each entity's first member: the point it came into existence for the replay.
    let mut first: HashMap<i32, usize> = HashMap::new();
    for (position, observation) in observations.iter().enumerate() {
        let entity_id = observation.knowledge_entity_id.expect("filtered");
        first.entry(entity_id).or_insert(position);
    }

    // Every entity's members in capture order: the surfaces it would have worn at any point.
    let mut members: HashMap<i32, Vec<(usize, String, Option<String>)>> = HashMap::new();
    for (position, observation) in observations.iter().enumerate() {
        members
            .entry(observation.knowledge_entity_id.expect("filtered"))
            .or_default()
            .push((
                position,
                observation.name.clone(),
                observation.platform_handle.clone(),
            ));
    }

    // The same two key generations the blocker matches on.
    let keys = |name: &str, handle: Option<&str>| -> HashSet<String> {
        let mut set = HashSet::new();
        for surface in [Some(name), handle].into_iter().flatten() {
            let folded = normalize::comparable(surface);
            if !folded.is_empty() {
                set.insert(folded.clone());
                let words = normalize::word_key(&folded);
                // The same rule as the index and the probe: no words, no word key.
                if !words.is_empty() {
                    set.insert(words);
                }
            }
        }
        set
    };

    // Merge verdicts, folded the same way the index folds: (winner, loser), either direction.
    let merges: HashSet<(String, String)> = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Kind.eq("merge"))
        .all(&db.conn)
        .await?
        .into_iter()
        .filter_map(|verdict| {
            Some((
                normalize::comparable(verdict.winner_label.as_deref()?),
                normalize::comparable(verdict.loser_label.as_deref()?),
            ))
        })
        .collect();

    let blocker = SqlBlocker::new(db);
    let mut report = Report {
        observations: observations.len(),
        superseded,
        ..Default::default()
    };
    let mut exists: HashSet<i32> = HashSet::new();

    for (position, observation) in observations.iter().enumerate() {
        let expected = observation.knowledge_entity_id.expect("filtered");
        let eligible = first[&expected] < position;

        let offered: Vec<i32> = blocker
            .candidates(observation)
            .await
            .into_iter()
            .map(|candidate| candidate.id)
            .filter(|id| exists.contains(id))
            .collect();

        if eligible {
            report.eligible += 1;
            *report.sizes.entry(offered.len()).or_default() += 1;
            if offered.is_empty() {
                report.starved += 1;
            }
            if offered.len() > 1 {
                report.ambiguous += 1;
            }
            // What the entity wore *before* this observation: earlier members only.
            let ours = keys(&observation.name, observation.platform_handle.as_deref());
            let expected_name = names.get(&expected).cloned().unwrap_or_default();
            // A merge verdict names the entity and a name that belongs with it — which may be an
            // earlier member, or the observation under test (the member it was renamed after).
            let folded = normalize::comparable(&expected_name);
            let mut candidates: Vec<&str> = vec![observation.name.as_str()];
            candidates.extend(
                members[&expected]
                    .iter()
                    .filter(|(earlier, _, _)| *earlier < position)
                    .map(|(_, name, _)| name.as_str()),
            );
            let merged = candidates.iter().any(|name| {
                let other = normalize::comparable(name);
                merges.contains(&(folded.clone(), other.clone()))
                    || merges.contains(&(other, folded.clone()))
            });
            let prior: HashSet<String> = members[&expected]
                .iter()
                .filter(|(earlier, _, _)| *earlier < position)
                .flat_map(|(_, name, handle)| keys(name, handle.as_deref()))
                .collect();
            if !ours.is_disjoint(&prior) {
                report.prior_hit += 1;
            } else {
                if merged {
                    report.merged += 1;
                }
                report.prior_misses.push(Miss {
                    observation: observation.id,
                    name: observation.name.clone(),
                    expected,
                    expected_name: expected_name.clone(),
                    offered: prior.len(),
                    merged,
                });
            }
            if offered.contains(&expected) {
                report.hit += 1;
            } else {
                report.misses.push(Miss {
                    observation: observation.id,
                    name: observation.name.clone(),
                    expected,
                    expected_name: expected_name.clone(),
                    offered: offered.len(),
                    merged,
                });
            }
        } else {
            report.new_entity += 1;
        }

        exists.insert(expected);
    }

    Ok(report)
}
