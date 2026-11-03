//! Linking one illumination's observations into entities.
//!
//! No network. The blocker is the index the store holds, the evidence store answers exactly one
//! question — does an entity already hold this claim — and what to do about the answer belongs to the
//! [`LinkDecider`] the store was built with. Every write is idempotent: a second run finds nothing to
//! do because every observation already has a `knowledge_entity_id`.
//!
//! Callers reach this through [`Knowledge::link_illumination`]. The rest is `pub(super)` or private:
//! `merge` and `project` do not link, and nothing outside `knowledge` does either.

use crate::model;

use super::{Knowledge, evidence::ClaimOutcome, link_decider};

// Linking: the one write path that decides before it writes.
// ---------------------------------------------------------------------------------------------

/// Link one reading into the graph: cluster its observations, record their claims, and project every
/// entity it touched.
///
/// One entry point, because the blocker is the store's: a caller linking a thousand illuminations and
/// a caller linking one take the same path, and both benefit from what the store has been told so far.
pub(super) async fn link_illumination(
    knowledge: &Knowledge,
    illumination: &model::illumination::ModelEx,
) -> anyhow::Result<LinkStats> {
    let (Some(decider), Some(blocker)) = (&knowledge.decider, &knowledge.blocker) else {
        anyhow::bail!(
            "this Knowledge was built without a linker; use Knowledge::with_linking to link \
             illuminations"
        );
    };

    // The caller's job, checked here: an illumination loaded as a row and nothing else is a reading
    // with no observations, and a decider would happily decide about it. The decider cannot be relied
    // on to notice — it is injectable.
    anyhow::ensure!(
        illumination.observations.is_loaded(),
        "illumination {} was handed to link_illumination with no observations loaded; fetch it \
         with Entity::load().with(model::user_observed_entity::Entity)",
        illumination.id
    );

    // No guard is held across the decision: the blocker synchronizes itself, so the notifications
    // that `apply_decision` and projection make cannot deadlock against this call.
    let decisions = decider
        .decide_link(&**blocker, &knowledge.evidence, illumination)
        .await?;

    let mut stats = LinkStats::default();
    for decision in &decisions {
        let applied = apply_decision(knowledge, decision).await?;
        // Projection is a function of the members, so it runs after the writes that changed them,
        // and it returns the row it wrote — which is exactly what the blocker needs to re-index.
        // An entity two observations landed on is projected twice; a wasted read is cheaper than
        // remembering which ones were already done.
        knowledge.update_projection(applied.entity).await?;
        stats.record(decision, applied);
    }

    Ok(stats)
}

/// What one illumination's decisions actually did.
///
/// Deliberately not a report: it does not count what the caller already knows — how many observations
/// were considered, which illuminations were superseded. It says only what the executor wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub attached: usize,
    /// The part of `attached` where a claim named the entity rather than a score.
    pub attached_by_claim: usize,
    pub created: usize,
    /// Created, then discarded: another entity turned out to hold the claim after all, so the row
    /// that was just made owned nothing. Not a merge — nothing was folded into anything.
    pub discarded_on_conflict: usize,
    /// The same-illumination veto firing.
    pub vetoed: u32,
}

impl LinkStats {
    /// Fold one decision and what came of it into the totals.
    ///
    /// Both halves live here because they are one question: what did this reading do, and why.
    fn record(&mut self, decision: &link_decider::Decision, applied: Applied) {
        match &decision.action {
            link_decider::Action::Attach { by_claim: true, .. } => self.attached_by_claim += 1,
            link_decider::Action::Create { vetoed: true, .. } => self.vetoed += 1,
            _ => {}
        }
        match applied.outcome {
            Outcome::Attached => self.attached += 1,
            Outcome::Created => self.created += 1,
            Outcome::DiscardedOnConflict => self.discarded_on_conflict += 1,
        }
    }
}

/// What applying one decision did.
struct Applied {
    /// The entity the observation ended up on — the one to update the projection for.
    entity: i32,
    outcome: Outcome,
}

enum Outcome {
    /// It joined the entity the decision named.
    Attached,
    /// A new entity was created for it.
    Created,
    /// A new entity was created and then discarded, because another one already held the claim.
    DiscardedOnConflict,
}

/// Apply a linking decision against the knowledge_entities table.
/// Apply one decision against the graph.
async fn apply_decision(
    knowledge: &Knowledge,
    decision: &link_decider::Decision,
) -> anyhow::Result<Applied> {
    let observation = &decision.observation;
    if observation.knowledge_entity_id.is_some() {
        anyhow::bail!(
            "the linker decided about observation {}, which is already linked",
            observation.id
        );
    }

    // The entity this observation ends up on, resolved once: the one the decision named, or a new
    // one it asked for.
    let mut target_entity = match &decision.action {
        link_decider::Action::Attach { entity, .. } => knowledge.get(*entity).await?,
        link_decider::Action::Create { .. } => knowledge.create(observation).await?,
    };
    let created = target_entity.id;
    knowledge
        .attach_observation(observation.id, target_entity.id)
        .await?;

    // The observation may carry its own claim — a handle we inferred from the capture. If another
    // entity already holds it, the entity just created owns nothing: discard it and point the
    // observation at the one that does. Usually this is two observations of one illumination that
    // both asked for a new entity carrying the same claim; occasionally it is another worker.
    let mut discarded_on_conflict = false;
    if let Some(claim) = &decision.claim
        && let ClaimOutcome::HeldBy(owner) = knowledge
            .evidence
            .record(target_entity.id, claim, observation)
            .await?
    {
        if matches!(decision.action, link_decider::Action::Attach { .. }) {
            anyhow::bail!(
                "entities {} and {owner} both hold {}:{}; merging is not implemented yet \
             (plan/knowledge-canonical-entities.md §Repair)",
                target_entity.id,
                claim.scheme,
                claim.id
            );
        }
        // Re-point before discarding: the observation still references the entity being deleted,
        // and that foreign key is `NO ACTION`.
        target_entity = knowledge.get(owner).await?;
        knowledge.attach_observation(observation.id, owner).await?;
        knowledge.discard_empty(created).await?;
        discarded_on_conflict = true;
    }

    let outcome = if discarded_on_conflict {
        Outcome::DiscardedOnConflict
    } else if matches!(decision.action, link_decider::Action::Attach { .. }) {
        Outcome::Attached
    } else {
        Outcome::Created
    };
    Ok(Applied {
        entity: target_entity.id,
        outcome,
    })
}

#[cfg(test)]
mod unit {
    use std::sync::Arc;

    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    use super::*;
    use crate::database::DbHandle;
    use crate::knowledge::blocker::Blocker;
    use crate::knowledge::evidence::Evidence;
    use crate::knowledge::link_decider::LinkDecider;
    use crate::model::knowledge_entity as entity;
    use crate::test_support::{fixtures, test_db};

    /// The shape `link_illumination` needs: one illumination with its observations loaded.
    async fn load_illumination(db: &DbHandle, id: i32) -> model::illumination::ModelEx {
        model::illumination::Entity::load()
            .filter(model::illumination::Column::Id.eq(id))
            .with(model::user_observed_entity::Entity)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
    }

    /// The store with a linker and a fresh blocker, for the tests that actually link.
    async fn linking_store(db: &DbHandle, decider: Arc<dyn LinkDecider>) -> Knowledge {
        Knowledge::with_linking(
            db,
            Box::new(
                crate::knowledge::heuristic::NameBlocker::new(db)
                    .await
                    .unwrap(),
            ),
            decider,
        )
    }

    /// A decider that decides nothing. Exists to prove the policy is the caller's: if this store
    /// still named `HeuristicLinker` somewhere inside, this test could not pass.
    struct NoDecisions;

    #[async_trait::async_trait]
    impl LinkDecider for NoDecisions {
        async fn decide_link(
            &self,
            _blocker: &dyn Blocker,
            _evidence: &Evidence,
            _illumination: &model::illumination::ModelEx,
        ) -> anyhow::Result<Vec<link_decider::Decision>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn the_linking_policy_is_the_one_it_was_built_with() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "policy").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        let loaded = load_illumination(&db, illumination).await;
        let inert = linking_store(&db, Arc::new(NoDecisions)).await;
        let stats = inert.link_illumination(&loaded).await.unwrap();
        assert_eq!(
            stats,
            LinkStats::default(),
            "nothing decided, nothing written"
        );
        assert_eq!(
            entity::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "an inert policy must not create entities"
        );

        let loaded = load_illumination(&db, illumination).await;
        let linking =
            linking_store(&db, Arc::new(crate::knowledge::heuristic::HeuristicLinker)).await;
        let stats = linking.link_illumination(&loaded).await.unwrap();
        assert_eq!(stats.created, 1, "the linker it was built with decides");
    }

    /// A store built for writing has no policy, and linking on it says so rather than pretending.
    #[tokio::test]
    async fn a_store_without_a_linker_refuses_to_link() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "no_linker").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;
        let loaded = load_illumination(&db, illumination).await;

        let error = Knowledge::new(&db)
            .link_illumination(&loaded)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("without a linker"), "{error}");
    }

    /// The executor owns this precondition, not the decider: the decider here decides nothing and
    /// checks nothing, so an error naming the missing observations can only come from the store.
    #[tokio::test]
    async fn an_illumination_without_observations_is_refused() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "unloaded").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        // The row and nothing hanging off it.
        let unloaded = model::illumination::Entity::load()
            .filter(model::illumination::Column::Id.eq(illumination))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(!unloaded.observations.is_loaded());

        let error = linking_store(&db, Arc::new(NoDecisions))
            .await
            .link_illumination(&unloaded)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no observations loaded"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod corpus {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

    use crate::database;
    use crate::knowledge::evidence::{ClaimOutcome, Evidence};
    use crate::knowledge::{STATUS_IDENTIFIED, STATUS_PROVISIONAL};
    use crate::model::{
        knowledge_entity as entity, knowledge_evidence as evidence,
        user_observed_entity as observation,
    };
    use crate::test_support::fixtures;

    struct Capture {
        user_id: i32,
        capture_id: i32,
        illumination_id: i32,
    }

    async fn capture(db: &database::DbHandle, tag: &str) -> Capture {
        let user_id = fixtures::user(db, tag).await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        Capture {
            user_id,
            capture_id,
            illumination_id,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn observe(
        db: &database::DbHandle,
        capture: &Capture,
        position: i32,
        name: &str,
        description: Option<&str>,
        entity_type: &str,
        platform: Option<&str>,
        handle: Option<&str>,
    ) -> i32 {
        observation::Entity::insert(observation::ActiveModel {
            user_id: Set(capture.user_id),
            capture_id: Set(capture.capture_id),
            illumination_id: Set(capture.illumination_id),
            position: Set(position),
            name: Set(name.to_string()),
            entity_type: Set(entity_type.to_string()),
            description: Set(description.map(str::to_string)),
            platform: Set(platform.map(str::to_string)),
            platform_handle: Set(handle.map(str::to_string)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .expect("observation should insert")
        .last_insert_id as i32
    }

    async fn entity_count(db: &database::DbHandle) -> u64 {
        entity::Entity::find().count(&db.conn).await.unwrap()
    }

    #[tokio::test]
    async fn one_capture_never_folds_two_observations_together() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let same = capture(&db, "link_same_capture").await;
        observe(
            &db,
            &same,
            0,
            "Dune",
            Some("a 1965 novel by Frank Herbert"),
            "book",
            None,
            None,
        )
        .await;
        observe(
            &db,
            &same,
            1,
            "Dune",
            Some("a 1965 novel by Frank Herbert"),
            "book",
            None,
            None,
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        assert_eq!(entity_count(&db).await, 2);
    }

    /// Two observations of one illumination carrying the same, previously unseen handle. Decisions
    /// are made before any writes, so neither can see the other's claim: both are told to create.
    /// The second discovers the first already recorded it and folds — re-pointing the observation
    /// first, because the row it created cannot be deleted while an observation still references it.
    #[tokio::test]
    async fn two_observations_sharing_a_new_handle_fold_into_one_entity() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let same = capture(&db, "link_fold").await;
        observe(
            &db,
            &same,
            0,
            "Alexey Kutepov",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        observe(
            &db,
            &same,
            1,
            "Tsoding",
            None,
            "social_media_account",
            Some("x"),
            Some("Tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("the second must fold, not trip the foreign key");

        assert_eq!(entity_count(&db).await, 1, "one referent, not two");
        let linked: Vec<Option<i32>> = observation::Entity::find()
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.knowledge_entity_id)
            .collect();
        assert_eq!(linked.len(), 2);
        assert!(linked.iter().all(|id| *id == linked[0] && id.is_some()));
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "one claim, on the entity that survived"
        );
    }

    #[tokio::test]
    async fn a_second_run_does_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_idempotent").await;
        observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a novella"),
            "book",
            None,
            None,
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        crate::test_support::link::link_all(&db)
            .await
            .expect("second build should succeed");

        assert_eq!(
            entity_count(&db).await,
            1,
            "one entity, and a second run adds none"
        );
        let unlinked = observation::Entity::find()
            .filter(observation::Column::KnowledgeEntityId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 0, "the second run had nothing left to link");
    }

    /// Every illumination is linked. An illumination can be wrong and the correct illumination is often the
    /// older one, so the graph keeps them all — currency is decided at the view boundary
    /// (`api::user::get_entity`), not here.
    #[tokio::test]
    async fn a_superseded_illumination_is_linked_too() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user_id = fixtures::user(&db, "link_superseded").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let old = fixtures::illumination(&db, user_id, capture_id).await;
        let live = fixtures::illumination(&db, user_id, capture_id).await;

        for (illumination_id, name) in [(old, "Old Run"), (live, "Train Dreams")] {
            observation::Entity::insert(observation::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(0),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap();
        }

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let unlinked = observation::Entity::find()
            .filter(observation::Column::KnowledgeEntityId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 0, "history is linked, not discarded");
    }

    /// `project` keeps the selected name out of `alt_names`, so an observation is only findable on a
    /// later run if its own name is indexed as well. Without that, every capture that repeated a
    /// name created a duplicate — the local corpus had ten such pairs.
    #[tokio::test]
    async fn a_shared_handle_links_across_captures_and_names() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_persona_first").await;
        let second = capture(&db, "link_persona_second").await;
        observe(
            &db,
            &first,
            0,
            "Alexey Kutepov",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        observe(
            &db,
            &second,
            0,
            "Tsoding",
            None,
            "social_media_account",
            Some("x"),
            Some("Tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        assert_eq!(
            entity_count(&db).await,
            1,
            "the claim discarded the duplicate"
        );
        assert_eq!(evidence::Entity::find().count(&db.conn).await.unwrap(), 1);
    }

    /// The concurrency guard, tested directly: a second entity asking for an identifier that is
    /// already claimed must be told who owns it, not silently ignored.
    #[tokio::test]
    async fn an_identifier_claimed_elsewhere_reports_the_owner() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_claim").await;
        observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let owner_core = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        let duplicate = entity::Entity::insert(entity::ActiveModel {
            name: Set("Tsoding".to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set(STATUS_PROVISIONAL.to_string()),
            alt_names: Set(vec!["tsoding".to_string()]),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;

        let observation = observation::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let claim = crate::knowledge::evidence::Claim::of(&observation).expect("a handle");
        let outcome = Evidence::from_handle(&db)
            .record(duplicate, &claim, &observation)
            .await
            .expect("claim write should succeed");

        assert_eq!(outcome, ClaimOutcome::HeldBy(owner_core.id));
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "the duplicate must not create a second identity row"
        );
    }

    #[tokio::test]
    async fn a_handle_identified_core_is_not_provisional() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_status").await;
        observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let entity = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        assert_eq!(entity.status, STATUS_IDENTIFIED);
    }
}
