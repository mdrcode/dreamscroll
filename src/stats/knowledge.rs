//! Counts that tell us whether the knowledge graph is working.
//!
//! Read-only, over `model`: it touches no `knowledge` type, which is why it lives with the other
//! subsystem reports rather than inside the module it reports on.

use std::collections::HashSet;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, PaginatorTrait, QueryFilter,
    Statement,
};

use crate::{database, model};

/// One `(provider, scheme)` group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimGroup {
    pub provider: String,
    pub scheme: String,
    /// Identity rows in this group, including ones no live claim cites.
    pub identities: u64,
    pub live_claims: u64,
    /// Distinct live entities holding at least one live claim in the group.
    pub entities: u64,
    pub retracted: u64,
}

/// One platform's observation-level link counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlatformGroup {
    pub platform: String,
    pub observations: u64,
    /// Distinct identities the observations resolve to — repeats of one handle collapse.
    pub identities: u64,
    /// Distinct live entities holding those observations.
    pub entities: u64,
    pub display_names: u64,
    pub captures: u64,
}

/// Live entities by how many live claims they hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimCounts {
    pub entities: u64,
    pub zero: u64,
    pub one: u64,
    pub two: u64,
    pub three_plus: u64,
}

/// The projected confidence over live entities.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfidenceSpread {
    pub unknown: u64,
    pub min: Option<f64>,
    pub average: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Default)]
pub struct KnowledgeStats {
    pub observations: u64,
    pub linked: u64,
    pub entities: u64,
    pub provisional: u64,
    pub identified: u64,
    /// Candidates in the batch the last scan produced.
    pub proximities: u64,
    /// Entities appearing on either side of a candidate.
    pub entities_in_proximity: u64,
    /// Platform links observed on captures, grouped by platform. This is the input side of the
    /// provider/scheme breakdown: what the illuminator actually saw, before linking collapses
    /// repeated handles into identities.
    pub platforms: Vec<PlatformGroup>,
    /// How many live claims a live entity holds. The shape a second authority changes.
    pub claim_counts: ClaimCounts,
    /// The projected confidence over live entities, and how many have none.
    pub confidence: ConfidenceSpread,
    /// Entities with a live authority claim — the number `resolve` moves.
    pub with_claim: u64,
    /// Identities and claims grouped by who supplied them and in which scheme. `provider` is the
    /// source (a resolver id, `illuminator`, `model`); `scheme` is the namespace the identifier
    /// lives in. Both halves are needed: one authority may answer in several schemes, and one
    /// scheme may be supplied by more than one authority.
    pub claims_by: Vec<ClaimGroup>,
    /// Entities whose projected platform presence is set. A member had one and `project` kept it.
    pub with_platform: u64,
    /// Distinct entities that *should* have one, counted from the observations. Always equal to
    /// `with_platform`; when it is not, projection has lost something.
    pub platform_from_members: u64,
    pub cache_found: u64,
    /// Live answers that arrived, but not in a way worth trusting — the category most likely to hide a
    /// silent provider failure, so it is counted rather than folded into `found`.
    pub cache_degraded: u64,
    pub cache_empty: u64,
    pub cache_failed: u64,
    /// Rows past their expiry: not answers any more, and refetchable. Kept for `PRUNE_GRACE`
    /// after expiry, then pruned by the next online resolve.
    pub cache_expired: u64,
}

impl KnowledgeStats {
    /// `1 - distinct_entities / observations`: the share of observations that share a referent.
    pub fn duplicate_rate(&self) -> f64 {
        if self.observations == 0 {
            return 0.0;
        }
        1.0 - self.entities as f64 / self.observations as f64
    }
}

pub async fn knowledge_stats(db: &database::DbHandle) -> anyhow::Result<KnowledgeStats> {
    let observations = model::user_observed_entity::Entity::find()
        .count(&db.conn)
        .await?;
    let linked = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
        .count(&db.conn)
        .await?;

    let platform_from_members: HashSet<i32> = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::Platform.is_not_null())
        .filter(model::user_observed_entity::Column::Platform.ne(""))
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
        .all(&db.conn)
        .await?
        .into_iter()
        .filter_map(|row| row.knowledge_entity_id)
        .collect();

    let mut stats = KnowledgeStats {
        observations,
        linked,
        platform_from_members: platform_from_members.len() as u64,
        ..Default::default()
    };

    for entity in model::knowledge_entity::Entity::find()
        .all(&db.conn)
        .await?
    {
        // A tombstone is history, not an entity: it keeps the columns it had and its members have
        // moved. Counting it inflated every number here by the number of merges.
        if entity.merged_into_id.is_some() {
            continue;
        }
        stats.entities += 1;
        match entity.status {
            model::knowledge_entity::Status::Identified => stats.identified += 1,

            model::knowledge_entity::Status::Provisional => stats.provisional += 1,
        }
        if entity.platform.is_some() {
            stats.with_platform += 1;
        }
    }

    // Platform links are observations; the claim on that observation names the identity and the
    // entity it landed on. `count(DISTINCT o.id)` keeps an observation with a moved claim from
    // counting twice, and the claim filters keep retracted and tombstoned rows out of the totals.
    let rows = sea_orm::ConnectionTrait::query_all_raw(
        &db.conn,
        Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT o.platform, \
                    count(DISTINCT o.id) AS observations, \
                    count(DISTINCT c.knowledge_identity_id) \
                        FILTER (WHERE c.retracted_at IS NULL) AS identities, \
                    count(DISTINCT c.knowledge_entity_id) FILTER ( \
                        WHERE c.retracted_at IS NULL AND e.merged_into_id IS NULL \
                    ) AS entities, \
                    count(DISTINCT o.id) FILTER ( \
                        WHERE o.platform_display_name IS NOT NULL \
                          AND o.platform_display_name <> '' \
                    ) AS display_names, \
                    count(DISTINCT o.capture_id) AS captures \
               FROM user_observed_entities o \
               LEFT JOIN knowledge_claims c ON c.user_observed_entity_id = o.id \
               LEFT JOIN knowledge_entities e ON e.id = c.knowledge_entity_id \
              WHERE o.platform IS NOT NULL AND o.platform <> '' \
              GROUP BY o.platform \
              ORDER BY observations DESC, o.platform",
        ),
    )
    .await?;
    stats.platforms = rows
        .into_iter()
        .map(|row| {
            Ok(PlatformGroup {
                platform: row.try_get_by_index(0)?,
                observations: row.try_get_by_index::<i64>(1)? as u64,
                identities: row.try_get_by_index::<i64>(2)? as u64,
                entities: row.try_get_by_index::<i64>(3)? as u64,
                display_names: row.try_get_by_index::<i64>(4)? as u64,
                captures: row.try_get_by_index::<i64>(5)? as u64,
            })
        })
        .collect::<Result<Vec<_>, sea_orm::DbErr>>()?;

    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS entities, \
                    count(*) FILTER (WHERE claims = 0) AS zero, \
                    count(*) FILTER (WHERE claims = 1) AS one, \
                    count(*) FILTER (WHERE claims = 2) AS two, \
                    count(*) FILTER (WHERE claims >= 3) AS three_plus \
               FROM ( \
                 SELECT e.id, count(c.id) FILTER (WHERE c.retracted_at IS NULL) AS claims \
                   FROM knowledge_entities e \
                   LEFT JOIN knowledge_claims c ON c.knowledge_entity_id = e.id \
                  WHERE e.merged_into_id IS NULL \
                  GROUP BY e.id \
               ) counts",
        ))
        .await?
        .expect("count always returns one row");
    stats.claim_counts = ClaimCounts {
        entities: row.try_get_by_index::<i64>(0)? as u64,
        zero: row.try_get_by_index::<i64>(1)? as u64,
        one: row.try_get_by_index::<i64>(2)? as u64,
        two: row.try_get_by_index::<i64>(3)? as u64,
        three_plus: row.try_get_by_index::<i64>(4)? as u64,
    };

    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FILTER (WHERE confidence IS NULL) AS unknown, \
                    min(confidence)::float8 AS minimum, \
                    avg(confidence)::float8 AS average, \
                    max(confidence)::float8 AS maximum \
               FROM knowledge_entities \
              WHERE merged_into_id IS NULL",
        ))
        .await?
        .expect("aggregate always returns one row");
    stats.confidence = ConfidenceSpread {
        unknown: row.try_get_by_index::<i64>(0)? as u64,
        min: row.try_get_by_index::<Option<f64>>(1)?,
        average: row.try_get_by_index::<Option<f64>>(2)?,
        max: row.try_get_by_index::<Option<f64>>(3)?,
    };

    // An authority claim is an edge to an identity an authority answered for — not the illuminator
    // that produced the observation, and not a model guess.
    let with_claim: Vec<(i32,)> = sea_orm::ConnectionTrait::query_all_raw(
        &db.conn,
        sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT DISTINCT c.knowledge_entity_id \
               FROM knowledge_claims c \
              WHERE c.retracted_at IS NULL \
                AND c.provider NOT IN ('illuminator', 'platform', 'model', 'operator', 'user')",
        ),
    )
    .await?
    .into_iter()
    .map(|row| row.try_get_by_index::<i32>(0).map(|id| (id,)))
    .collect::<Result<_, _>>()?;
    stats.with_claim = with_claim.len() as u64;

    // One row per `(provider, scheme)`. `count(DISTINCT i.id)` is the identity count: the left
    // join would otherwise count one row per claim. An identity with no claim still appears, with
    // `live_claims = 0` — that is a row a rebuild or a retraction left behind, worth seeing.
    let rows = sea_orm::ConnectionTrait::query_all_raw(
        &db.conn,
        Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT i.provider, i.scheme, \
                    count(DISTINCT i.id) AS identities, \
                    count(c.id) FILTER (WHERE c.retracted_at IS NULL) AS live_claims, \
                    count(DISTINCT c.knowledge_entity_id) FILTER ( \
                        WHERE c.retracted_at IS NULL AND e.merged_into_id IS NULL \
                    ) AS entities, \
                    count(c.id) FILTER (WHERE c.retracted_at IS NOT NULL) AS retracted \
               FROM knowledge_identities i \
               LEFT JOIN knowledge_claims c ON c.knowledge_identity_id = i.id \
               LEFT JOIN knowledge_entities e ON e.id = c.knowledge_entity_id \
              GROUP BY i.provider, i.scheme \
              ORDER BY live_claims DESC, identities DESC, i.provider, i.scheme",
        ),
    )
    .await?;
    stats.claims_by = rows
        .into_iter()
        .map(|row| {
            Ok(ClaimGroup {
                provider: row.try_get_by_index(0)?,
                scheme: row.try_get_by_index(1)?,
                identities: row.try_get_by_index::<i64>(2)? as u64,
                live_claims: row.try_get_by_index::<i64>(3)? as u64,
                entities: row.try_get_by_index::<i64>(4)? as u64,
                retracted: row.try_get_by_index::<i64>(5)? as u64,
            })
        })
        .collect::<Result<Vec<_>, sea_orm::DbErr>>()?;

    // One scan's candidates: the whole table.
    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM knowledge_proximities",
        ))
        .await?
        .expect("count always returns one row");
    stats.proximities = row.try_get_by_index::<i64>(0)? as u64;

    let row = db
        .conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM ( \
               SELECT holder_id AS entity FROM knowledge_proximities \
               UNION \
               SELECT counterpart_id FROM knowledge_proximities \
             ) sides",
        ))
        .await?
        .expect("count always returns one row");
    stats.entities_in_proximity = row.try_get_by_index::<i64>(0)? as u64;

    // The cache owns what an outcome means; this only reports what it counted.
    let cache = crate::fetch::stats::tally(&db.conn).await?;
    stats.cache_found = cache.found;
    stats.cache_empty = cache.empty;
    stats.cache_failed = cache.failed;
    stats.cache_degraded = cache.degraded;
    stats.cache_expired = cache.expired;

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn reports_linked_share_and_duplicate_rate() {
        db_or_skip!(db);

        let user_id = fixtures::user(&db, "stats").await;
        for (index, capture_id) in [
            (0, fixtures::capture(&db, user_id).await),
            (1, fixtures::capture(&db, user_id).await),
        ] {
            let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
            fixtures::entity(
                &db,
                user_id,
                capture_id,
                illumination_id,
                index,
                "Train Dreams",
            )
            .await;
        }
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        let stats = super::knowledge_stats(&db)
            .await
            .expect("stats should succeed");

        assert_eq!(stats.observations, 2);
        assert_eq!(stats.linked, 2);
        assert_eq!(stats.entities, 1);
        assert_eq!(stats.provisional, 1);
        assert!((stats.duplicate_rate() - 0.5).abs() < 1e-9, "{stats:?}");
    }

    #[test]
    fn duplicate_rate_of_an_empty_graph_is_zero() {
        assert_eq!(super::KnowledgeStats::default().duplicate_rate(), 0.0);
    }

    /// The two axes are independent: `provider` is who answered, `scheme` is the namespace. And
    /// an identity shared by two holders is still one identity — the left join must not count it
    /// once per claim.
    #[tokio::test]
    async fn reports_claims_grouped_by_provider_and_scheme() {
        db_or_skip!(db);
        let first = fixtures::knowledge_entity_with_member(&db, "Grouped Claim").await;
        let second = fixtures::knowledge_entity_with_member(&db, "Another Holder").await;
        for entity in [first, second] {
            fixtures::claim(
                &db,
                entity,
                "wikidata",
                "Q42",
                "wikidata",
                Some("exact_label"),
                Some(0.9),
                crate::model::knowledge_claims::ClaimBasis::Text,
            )
            .await;
        }

        let stats = super::knowledge_stats(&db).await.expect("stats");
        let group = stats
            .claims_by
            .iter()
            .find(|group| group.provider == "wikidata" && group.scheme == "wikidata")
            .expect("the fixture's group");
        assert_eq!(group.identities, 1, "one identity, not one per claim");
        assert_eq!(group.live_claims, 2);
        assert_eq!(group.entities, 2);
        assert_eq!(group.retracted, 0);
        assert_eq!(stats.claim_counts.entities, 2);
        assert_eq!(stats.claim_counts.one, 2);
        assert_eq!(stats.claim_counts.two, 0);
        assert_eq!(stats.confidence.unknown, 2);
    }

    /// Platform links are observations, not identities: two sightings of one handle are two
    /// observations and one identity, and the platform is the grouping key.
    #[tokio::test]
    async fn reports_platform_links_by_platform() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "platform_links").await;
        fixtures::observe(
            &db,
            &capture,
            0,
            "Ada",
            None,
            "real_person",
            Some("x_twitter"),
            Some("@ada"),
        )
        .await;
        fixtures::observe(
            &db,
            &capture,
            1,
            "Ada Again",
            None,
            "real_person",
            Some("x_twitter"),
            Some("@ada"),
        )
        .await;
        fixtures::observe(
            &db,
            &capture,
            2,
            "Grace",
            None,
            "real_person",
            Some("instagram"),
            Some("@grace"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let stats = super::knowledge_stats(&db).await.expect("stats");
        let x = stats
            .platforms
            .iter()
            .find(|group| group.platform == "x_twitter")
            .expect("x_twitter group");
        assert_eq!(x.observations, 2, "two sightings");
        assert_eq!(x.identities, 1, "one folded handle");
        assert_eq!(
            x.entities, 2,
            "one illumination is decided as a batch, so both sightings create before either can attach"
        );
        assert_eq!(x.captures, 1);

        let instagram = stats
            .platforms
            .iter()
            .find(|group| group.platform == "instagram")
            .expect("instagram group");
        assert_eq!(instagram.observations, 1);
        assert_eq!(instagram.identities, 1);
    }

    #[tokio::test]
    async fn projected_platform_presence_matches_the_members() {
        db_or_skip!(db);

        let user_id = fixtures::user(&db, "platform").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        let capture = fixtures::Capture {
            user_id,
            capture_id,
            illumination_id,
        };
        fixtures::observe(
            &db,
            &capture,
            0,
            "Some Person",
            None,
            "real_person",
            Some("instagram"),
            Some("@some_person"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let stats = super::knowledge_stats(&db)
            .await
            .expect("stats should succeed");
        assert_eq!(stats.with_platform, 1);
        assert_eq!(
            stats.with_platform, stats.platform_from_members,
            "projection lost presence a member had"
        );
    }
}
