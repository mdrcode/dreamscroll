//! The illumination composition: what to link, and what to assert.
//!
//! [`KnowledgeGraph::link`] knows only about a collection of observations. Everything that knows they
//! arrive one illumination at a time lives here: the batch is one illumination's observations, and a
//! capture's claims are written after its entities exist, because a claim names both the identity the
//! observation asserted and the entity it landed on.

use std::collections::HashMap;

use crate::knowledge::evidence::{ClaimDraft, IdentityKey, Stored};
use crate::knowledge::graph::{KnowledgeGraph, LinkReport};
use crate::model;

/// Link one illumination: its observations become entities, then their handles become claims.
pub async fn link_illumination(
    graph: &KnowledgeGraph,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<LinkReport> {
    // An unloaded relation would read as an illumination with no observations, and linking nothing
    // would look exactly like success.
    anyhow::ensure!(
        illumination.observations.is_loaded(),
        "illumination {} was handed to link_illumination with no observations loaded; fetch it \
         with Entity::load().with(model::user_observed_entity::Entity)",
        illumination.id
    );
    let observations: Vec<model::user_observed_entity::Model> = illumination
        .observations
        .iter()
        .map(|row| row.clone().into())
        .collect();

    let (mut report, linked) = graph.link(&observations).await?;
    report.vetoed = assert_claims(graph, &observations, &linked).await?;
    Ok(report)
}

/// Assert the handle each linked observation showed, and project where it landed.
///
/// The only place a capture's identity and claim are written. Returns how many a standing veto
/// refused.
async fn assert_claims(
    graph: &KnowledgeGraph,
    observations: &[model::user_observed_entity::Model],
    linked: &HashMap<i32, i32>,
) -> anyhow::Result<usize> {
    let vetoes = graph.vetoed().await?;
    let mut vetoed = 0;
    for observation in observations {
        let Some(entity) = linked.get(&observation.id).copied() else {
            continue;
        };
        if let Some(key) = IdentityKey::from_platform_link(observation) {
            let identity = graph.identity_for_link(&key).await?;
            if graph
                .assert(
                    ClaimDraft::capture(entity, observation, &key, identity),
                    &vetoes,
                )
                .await?
                == Stored::Vetoed
            {
                vetoed += 1;
            }
        }
        // After the claims: status and confidence are functions of them.
        graph.update_projection(entity).await?;
    }
    Ok(vetoed)
}
