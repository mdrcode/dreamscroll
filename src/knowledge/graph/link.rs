//! The linking executor: a linker's decisions applied to a collection of observations.
//!
//! No network, no identities, no claims. The [`Linker`] decides, the executor attaches, and each
//! entity is projected so it exists with its surfaces. Asserting what a capture showed is the
//! caller's job — see [`crate::knowledge::helper`], which knows that the batch is one illumination.

use std::collections::{HashMap, HashSet};

use crate::knowledge::linker::{Action, Decision};
use crate::model;

use super::KnowledgeGraph;

// Linking: the one write path that decides before it applies.
// ---------------------------------------------------------------------------------------------

/// What one batch of decisions actually did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub attached: usize,
    pub created: usize,
    /// Pending observations the Linker returned no decision for.
    pub left_pending: usize,
    /// Claims a veto "verdict" refused — [`crate::knowledge::graph::verdicts::vetoed`]
    pub vetoed: usize,
}

impl LinkStats {
    fn record(&mut self, decision: &Decision) {
        match decision.action {
            Action::Attach(_) => self.attached += 1,
            Action::Create => self.created += 1,
        }
    }
}

/// Link a collection of observations: decide, attach, and project each entity.
///
/// Returns the stats and the `observation → entity` mapping, so the caller can assert what each
/// observation showed. One entry point, because the blocker is the store's: a caller linking a
/// thousand observations and a caller linking one take the same path.
pub(crate) async fn link(
    graph: &KnowledgeGraph,
    observations: &[model::user_observed_entity::Model],
) -> anyhow::Result<(LinkStats, HashMap<i32, i32>)> {
    let (Some(decider), Some(blocker)) = (&graph.decider, &graph.blocker) else {
        anyhow::bail!(
            "this KnowledgeGraph was built without a linker; use KnowledgeGraph::with_linking"
        );
    };

    // Only the unlinked ones: a linked observation is already where it belongs.
    let pending: HashSet<i32> = observations
        .iter()
        .filter(|row| row.knowledge_entity_id.is_none())
        .map(|row| row.id)
        .collect();

    let decisions = decider
        .decide_link(&**blocker, &graph.evidence, observations)
        .await?;

    let mut stats = LinkStats::default();
    let mut linked: HashMap<i32, i32> = HashMap::with_capacity(decisions.len());
    for decision in &decisions {
        // The guarantee the `Decision` doc makes: the subject is one the Linker was given, and it was
        // not linked before this batch.
        anyhow::ensure!(
            pending.contains(&decision.observation.id),
            "the linker decided about observation {}, which is not pending in this batch",
            decision.observation.id
        );
        let entity = apply_decision(graph, decision).await?;
        graph.update_projection(entity).await?;
        linked.insert(decision.observation.id, entity);
        stats.record(decision);
    }

    // A policy may decide less than it was given. The executor applies what it got and says how much
    // it was not told about, rather than inventing a decision or calling the silence success.
    let decided: HashSet<i32> = decisions
        .iter()
        .map(|decision| decision.observation.id)
        .collect();
    stats.left_pending = pending.difference(&decided).count();

    Ok((stats, linked))
}

/// Apply one decision: attach the observation to its entity.
async fn apply_decision(graph: &KnowledgeGraph, decision: &Decision) -> anyhow::Result<i32> {
    let observation = &decision.observation;
    let entity = match &decision.action {
        Action::Attach(entity) => *entity,
        Action::Create => graph.create(observation).await?.id,
    };

    // `attach` refuses a tombstone, so this also covers an entity merged away since the decision.
    graph.attach_observation(observation.id, entity).await?;

    Ok(entity)
}

#[cfg(test)]
mod unit {
    use std::sync::Arc;

    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    use super::*;
    use crate::database::DbHandle;
    use crate::knowledge::blocker::Blocker;
    use crate::knowledge::evidence::Evidence;
    use crate::knowledge::linker::Linker;
    use crate::model::knowledge_entity as entity;
    use crate::test_support::{db_or_skip, fixtures};

    /// The public path for one illumination: the helper owns the composition.
    async fn link_illumination(
        store: &KnowledgeGraph,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<LinkStats> {
        crate::knowledge::helper::link_illumination(store, illumination).await
    }

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
    async fn linking_store(db: &DbHandle, decider: Arc<dyn Linker>) -> KnowledgeGraph {
        KnowledgeGraph::with_linking(
            db,
            Box::new(crate::knowledge::blocker::SqlBlocker::new(db)),
            decider,
        )
    }

    /// A decider that decides nothing. Exists to prove the policy is the caller's: if this store
    /// still named `HeuristicLinker` somewhere inside, this test could not pass.
    struct NoDecisions;

    #[async_trait::async_trait]
    impl Linker for NoDecisions {
        async fn decide_link(
            &self,
            _blocker: &dyn Blocker,
            _evidence: &Evidence,
            _observations: &[model::user_observed_entity::Model],
        ) -> anyhow::Result<Vec<crate::knowledge::linker::Decision>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn the_linking_policy_is_the_one_it_was_built_with() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "policy").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        let loaded = load_illumination(&db, illumination).await;
        let inert = linking_store(&db, Arc::new(NoDecisions)).await;
        let stats = link_illumination(&inert, &loaded).await.unwrap();
        assert_eq!(stats.attached, 0, "nothing decided, nothing written");
        assert_eq!(stats.created, 0);
        assert_eq!(stats.left_pending, 1, "and it says it was not told");
        assert_eq!(
            entity::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "an inert policy must not create entities"
        );

        let loaded = load_illumination(&db, illumination).await;
        let linking = linking_store(
            &db,
            Arc::new(crate::knowledge::linker::heuristic::HeuristicLinker::new()),
        )
        .await;
        let stats = link_illumination(&linking, &loaded).await.unwrap();
        assert_eq!(stats.created, 1, "the linker it was built with decides");
        assert_eq!(stats.left_pending, 0, "every pending observation decided");
    }

    /// A `Decision` is about an observation the Linker was given. One naming anything else is a
    /// broken policy, and the executor refuses it rather than writing to a row it never read.
    struct Foreign;

    #[async_trait::async_trait]
    impl Linker for Foreign {
        async fn decide_link(
            &self,
            _blocker: &dyn Blocker,
            _evidence: &Evidence,
            _observations: &[model::user_observed_entity::Model],
        ) -> anyhow::Result<Vec<crate::knowledge::linker::Decision>> {
            Ok(vec![crate::knowledge::linker::Decision {
                observation: fixtures::stub_observation(999_999, "Train Dreams", "book"),
                action: crate::knowledge::linker::Action::Create,
            }])
        }
    }

    #[tokio::test]
    async fn a_decision_about_an_observation_it_was_not_given_is_refused() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "foreign").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;
        let loaded = load_illumination(&db, illumination).await;

        let store = linking_store(&db, Arc::new(Foreign)).await;
        let error = link_illumination(&store, &loaded).await.unwrap_err();
        assert!(error.to_string().contains("not pending"), "{error}");
        assert_eq!(
            entity::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "and nothing was written"
        );
    }

    /// A store built for writing has no policy, and linking on it says so rather than pretending.
    #[tokio::test]
    async fn a_store_without_a_linker_refuses_to_link() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "no_linker").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;
        let loaded = load_illumination(&db, illumination).await;

        let store = KnowledgeGraph::new(&db);
        let error = link_illumination(&store, &loaded).await.unwrap_err();
        assert!(error.to_string().contains("without a linker"), "{error}");
    }

    /// The executor owns this precondition, not the decider: the decider here decides nothing and
    /// checks nothing, so an error naming the missing observations can only come from the store.
    #[tokio::test]
    async fn an_illumination_without_observations_is_refused() {
        db_or_skip!(db);
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

        let store = linking_store(&db, Arc::new(NoDecisions)).await;
        let error = link_illumination(&store, &unloaded).await.unwrap_err();
        assert!(
            error.to_string().contains("no observations loaded"),
            "{error}"
        );
    }

    /// A veto names a pairing, and the capture writer asks it too: a handle the graph was told to
    /// reject is not re-asserted by the next link.
    #[tokio::test]
    async fn a_vetoed_handle_is_not_re_asserted() {
        db_or_skip!(db);
        let store = linking_store(
            &db,
            Arc::new(crate::knowledge::linker::heuristic::HeuristicLinker::new()),
        )
        .await;
        let live_claims = || async {
            crate::model::knowledge_claims::Entity::find()
                .filter(crate::model::knowledge_claims::Column::RetractedAt.is_null())
                .count(&db.conn)
                .await
                .unwrap()
        };

        let first = fixtures::Capture::new(&db, "link_veto_first").await;
        fixtures::observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x_twitter"),
            Some("tsoding"),
        )
        .await;
        let loaded = load_illumination(&db, first.illumination_id).await;
        link_illumination(&store, &loaded).await.unwrap();
        assert_eq!(live_claims().await, 1);

        let entity_id = entity::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .id;
        crate::knowledge::graph::verdicts::veto(&store, entity_id, "x_twitter", "tsoding", None)
            .await
            .unwrap();
        crate::knowledge::replay::replay(&db.conn, &store)
            .await
            .unwrap();
        assert_eq!(
            live_claims().await,
            0,
            "the veto retracted the capture's own claim"
        );

        let second = fixtures::Capture::new(&db, "link_veto_second").await;
        fixtures::observe(
            &db,
            &second,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x_twitter"),
            Some("tsoding"),
        )
        .await;
        let loaded = load_illumination(&db, second.illumination_id).await;
        let stats = link_illumination(&store, &loaded).await.unwrap();
        assert_eq!(
            stats.vetoed, 1,
            "the capture writer asked the veto: {stats:?}"
        );
        assert_eq!(live_claims().await, 0, "and did not write the edge");
        assert_eq!(
            entity::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "and it attached rather than duplicated"
        );
    }
}

#[cfg(test)]
mod corpus {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

    use crate::knowledge::Status;
    use crate::model::{knowledge_entity as entity, user_observed_entity as observation};
    use crate::test_support::db_or_skip;
    use crate::test_support::fixtures::{self, entity_count, observe, Capture};

    /// Nothing exists for either name yet, and every decision is made before any of them is written,
    /// so neither observation can find the other's entity: the batch is what keeps them apart, with
    /// no rule needed. An entity that *did* already exist would take both.
    #[tokio::test]
    async fn two_new_names_in_one_illumination_get_their_own_entities() {
        db_or_skip!(db);
        let same = Capture::new(&db, "link_same_capture").await;
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

    #[tokio::test]
    async fn a_second_run_does_nothing() {
        db_or_skip!(db);
        let first = Capture::new(&db, "link_idempotent").await;
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
        db_or_skip!(db);
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
        db_or_skip!(db);
        let first = Capture::new(&db, "link_persona_first").await;
        let second = Capture::new(&db, "link_persona_second").await;
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
            "the claim joined the existing entity instead of duplicating it"
        );
        assert_eq!(
            crate::model::knowledge_claims::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1
        );
    }

    /// A handle arriving for an entity known only by its name. The claim is asserted on that entity;
    /// no second one is minted for the handle alone.
    ///
    /// This is the case an `Attach` carrying an identity exists for. Without the fall-through to
    /// similarity — "a handle means look it up, else create" — this capture would mint a second
    /// `Train Dreams` that shares no identity with the first, and the scan only ever asks about
    /// entities that share one.
    #[tokio::test]
    async fn a_handle_joins_the_entity_its_name_names() {
        db_or_skip!(db);
        let first = Capture::new(&db, "link_handle_late_first").await;
        observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a novella by Denis Johnson"),
            "book",
            None,
            None,
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let second = Capture::new(&db, "link_handle_late_second").await;
        observe(
            &db,
            &second,
            0,
            "Train Dreams",
            Some("a novella by Denis Johnson"),
            "book",
            Some("x"),
            Some("traindreams"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("second build should succeed");

        assert_eq!(
            entity_count(&db).await,
            1,
            "the handle joins the entity its name names"
        );
        assert_eq!(
            crate::model::knowledge_claims::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "and the claim is asserted on that entity"
        );
    }

    #[tokio::test]
    async fn a_handle_identified_core_is_not_provisional() {
        db_or_skip!(db);
        let first = Capture::new(&db, "link_status").await;
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
        assert_eq!(entity.status, Status::Identified);
    }
}
