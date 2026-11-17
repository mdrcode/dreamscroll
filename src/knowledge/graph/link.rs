//! The linking executor: a linker's decisions applied to a batch of observations.
//!
//! One call is one transaction: decide, attach, annotate the handles the observations assert,
//! project — or nothing.

use std::collections::{HashMap, HashSet};

use sea_orm::{ConnectionTrait, TransactionTrait};

use crate::knowledge::evidence::{Identity, PLATFORM_LINK_PROVIDER_ID, platform_link};
use crate::knowledge::linker::{Action, Decision};
use crate::model;

use super::KnowledgeGraph;
use super::knowledge_graph::{attach, insert_new};

// Linking: the one write path that decides before it applies.
// ---------------------------------------------------------------------------------------------

/// What one batch of decisions actually did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkReport {
    pub attached: usize,
    pub created: usize,
    /// Pending observations the Linker returned no decision for.
    pub left_pending: usize,
    /// Identity annotations a veto "verdict" refused — `vetoed` in the verdict store.
    pub vetoed: usize,
}

impl LinkReport {
    fn record(&mut self, decision: &Decision) {
        match decision.action {
            Action::Attach(_) => self.attached += 1,
            Action::Create => self.created += 1,
        }
    }
}

/// Link a batch of observations: decide, attach, annotate the handles they assert, project.
///
/// One transaction, so a failure leaves every observation of the batch pending for the retry.
/// Attached-but-unannotated does not heal on the next projection: the identity is never written.
///
/// The decider runs before the transaction opens — its reads are committed state, and it is the
/// slow part. The batch is the caller's unit only: the policy decides each observation against the
/// corpus as it stood before the batch, so a wider batch only widens the transaction.
pub(crate) async fn link(
    graph: &KnowledgeGraph,
    observations: &[model::user_observed_entity::Model],
) -> anyhow::Result<LinkReport> {
    let vetoes = graph.vetoed().await?;
    let decisions = decide(graph, observations).await?;

    // The callback must be `'static`: the graph is cloned (a pool handle) and the batch moves in.
    let conn = graph.db.conn.clone();
    let graph = graph.clone();
    let observations = observations.to_vec();
    conn.transaction(move |txn| {
        Box::pin(async move {
            let (mut report, linked) = apply(txn, &observations, &decisions).await?;
            report.vetoed =
                write_identities(&graph, txn, &observations, &linked, &vetoes).await?;
            Ok::<LinkReport, anyhow::Error>(report)
        })
    })
    .await
    .map_err(|error| anyhow::anyhow!("link transaction failed: {error}"))
}

/// Ask the linker what each observation is.
async fn decide(
    graph: &KnowledgeGraph,
    observations: &[model::user_observed_entity::Model],
) -> anyhow::Result<Vec<Decision>> {
    let (Some(decider), Some(blocker)) = (&graph.decider, &graph.blocker) else {
        anyhow::bail!(
            "this KnowledgeGraph was built without a linker; use KnowledgeGraph::with_linking"
        );
    };
    decider
        .decide_link(&**blocker, &graph.evidence, observations)
        .await
}

/// Apply the decisions and project each entity they touched.
async fn apply<C: ConnectionTrait>(
    conn: &C,
    observations: &[model::user_observed_entity::Model],
    decisions: &[Decision],
) -> anyhow::Result<(LinkReport, HashMap<i32, i32>)> {
    // Only the unlinked ones: a linked observation is already where it belongs.
    let pending: HashSet<i32> = observations
        .iter()
        .filter(|row| row.knowledge_entity_id.is_none())
        .map(|row| row.id)
        .collect();

    let mut report = LinkReport::default();
    let mut linked: HashMap<i32, i32> = HashMap::with_capacity(decisions.len());
    for decision in decisions {
        // The guarantee the `Decision` doc makes: the subject is one the Linker was given, and it was
        // not linked before this batch.
        anyhow::ensure!(
            pending.contains(&decision.observation.id),
            "the linker decided about observation {}, which is not pending in this batch",
            decision.observation.id
        );
        let entity = apply_decision(conn, decision).await?;
        super::project::project(conn, entity).await?;
        linked.insert(decision.observation.id, entity);
        report.record(decision);
    }

    // A policy may decide less than it was given. The executor applies what it got and says how much
    // it was not told about, rather than inventing a decision or calling the silence success.
    let decided: HashSet<i32> = decisions
        .iter()
        .map(|decision| decision.observation.id)
        .collect();
    report.left_pending = pending.difference(&decided).count();

    Ok((report, linked))
}

/// Apply one decision: attach the observation to its entity.
async fn apply_decision<C: ConnectionTrait>(
    conn: &C,
    decision: &Decision,
) -> anyhow::Result<i32> {
    let observation = &decision.observation;
    let entity = match &decision.action {
        Action::Attach(entity) => *entity,
        Action::Create => insert_new(conn, observation).await?.id,
    };

    // `attach` refuses a tombstone, so this also covers an entity merged away since the decision.
    attach(conn, observation.id, entity).await?;

    Ok(entity)
}

/// Annotate each linked observation's handle, and project where it landed.
///
/// The only place a capture's identity is written. Returns how many a standing veto refused.
async fn write_identities<C: ConnectionTrait>(
    graph: &KnowledgeGraph,
    conn: &C,
    observations: &[model::user_observed_entity::Model],
    linked: &HashMap<i32, i32>,
    vetoes: &HashSet<(i32, String, String)>,
) -> anyhow::Result<usize> {
    let mut vetoed = 0;
    for observation in observations {
        let Some(entity) = linked.get(&observation.id).copied() else {
            continue;
        };
        if let Some(link) = platform_link(observation)
            && let Ok(mut identity) = Identity::try_from(&link)
        {
            identity.observation_id = Some(observation.id);
            vetoed += graph
                .evidence
                .add_identity(conn, entity, PLATFORM_LINK_PROVIDER_ID, &identity, vetoes)
                .await?
                .vetoed;
        }
        // After the annotations: status and confidence are functions of them.
        super::project::project(conn, entity).await?;
    }
    Ok(vetoed)
}

#[cfg(test)]
mod unit {
    use std::sync::Arc;

    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

    use super::*;
    use crate::database::DbHandle;
    use crate::knowledge::blocker::Blocker;
    use crate::knowledge::linker::{IdentityHolder, Linker};
    use crate::model::knowledge_entity as entity;
    use crate::test_support::{db_or_skip, fixtures};

    /// The public path for one illumination: the helper owns the composition, the graph the writes.
    async fn link_illumination(
        store: &KnowledgeGraph,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<LinkReport> {
        crate::knowledge::link_illumination(store, illumination).await
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
            _holder: &dyn IdentityHolder,
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
            _holder: &dyn IdentityHolder,
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

    /// A decider whose second decision names a tombstone, after the first has already written.
    struct SecondDecisionIsATombstone {
        tombstone: i32,
    }

    #[async_trait::async_trait]
    impl Linker for SecondDecisionIsATombstone {
        async fn decide_link(
            &self,
            _blocker: &dyn Blocker,
            _holder: &dyn IdentityHolder,
            observations: &[model::user_observed_entity::Model],
        ) -> anyhow::Result<Vec<Decision>> {
            assert_eq!(observations.len(), 2, "the fixture captures two");
            Ok(vec![
                Decision {
                    observation: observations[0].clone(),
                    action: Action::Create,
                },
                Decision {
                    observation: observations[1].clone(),
                    action: Action::Attach(self.tombstone),
                },
            ])
        }
    }

    /// Both writes vanish with the refusal — the created entity and both attaches — so the retry
    /// finds the whole reading pending.
    #[tokio::test]
    async fn a_refused_decision_rolls_the_whole_reading_back() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Old Train").await;
        store
            .merge(loser, winner, crate::knowledge::verdicts::Cause::Repair)
            .await
            .expect("the two entities are mergeable");
        let before = entity::Entity::find().count(&db.conn).await.unwrap();

        let capture = fixtures::Capture::new(&db, "link_rollback").await;
        fixtures::observe(
            &db,
            &capture,
            0,
            "Train Dreams",
            Some("a novella by Denis Johnson"),
            "book",
            None,
            None,
        )
        .await;
        fixtures::observe(
            &db,
            &capture,
            1,
            "The Largesse of the Sea Maiden",
            Some("a short story collection"),
            "book",
            None,
            None,
        )
        .await;

        let loaded = load_illumination(&db, capture.illumination_id).await;
        let store = linking_store(
            &db,
            Arc::new(SecondDecisionIsATombstone { tombstone: loser }),
        )
        .await;
        let error = link_illumination(&store, &loaded).await.unwrap_err();
        assert!(error.to_string().contains("tombstone"), "{error}");

        assert_eq!(
            entity::Entity::find().count(&db.conn).await.unwrap(),
            before,
            "the entity the first decision created was rolled back"
        );
        let unlinked = crate::model::user_observed_entity::Entity::find()
            .filter(
                crate::model::user_observed_entity::Column::KnowledgeEntityId.is_null(),
            )
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 2, "both observations are pending again");
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
        let live_identities = || async {
            crate::model::knowledge_identity::Entity::find()
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
        assert_eq!(live_identities().await, 1);

        let entity_id = entity::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .id;
        store
            .veto(entity_id, "x_twitter", "tsoding", None)
            .await
            .unwrap();
        store.replay_verdicts().await.unwrap();
        assert_eq!(
            live_identities().await,
            0,
            "the veto deleted the capture's own identity"
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
        assert_eq!(
            live_identities().await,
            0,
            "and did not write the annotation"
        );
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

    use crate::model::knowledge_entity::Status;
    use crate::model::{knowledge_entity as entity, user_observed_entity as observation};
    use crate::test_support::db_or_skip;
    use crate::test_support::fixtures::{self, Capture, entity_count, observe};

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
            "the identity joined the existing entity instead of duplicating it"
        );
        assert_eq!(
            crate::model::knowledge_identity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1
        );
    }

    /// A handle arriving for an entity known only by its name. The identity is annotated on that
    /// entity; no second one is minted for the handle alone.
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
            crate::model::knowledge_identity::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "and the identity is annotated on that entity"
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
