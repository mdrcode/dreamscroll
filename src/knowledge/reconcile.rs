//! Reconciliation: identities held by more than one entity.
//!
//! Sharing an identity is a candidate, not a verdict. An album's article is the identity of the
//! album *and* covers each of its songs, so two holders may be one thing or may be a work and the
//! things inside it. Claims are many-to-one precisely so that both readings can be stored; this pass
//! is where the question gets asked.
//!
//! It asks what linking asks — are these one thing? — with the same weights and the same
//! same-illumination veto, and it asks it pairwise within each identity group. A pair that scores
//! merges; a pair the veto refuses becomes a collision; a pair that does not score is left alone,
//! which is the shape of the album case.

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::Knowledge;
use super::evidence::Evidence;
use super::heuristic::{self, NormalizedCandidate, NormalizedObservation};
use super::merge::{Cause, Outcome};

/// What one pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Identities with more than one live holder.
    pub groups: usize,
    /// Pairs that scored high enough to be one thing, and were folded.
    pub merged: usize,
    /// Pairs that scored, but the same-illumination veto refused.
    pub refused: usize,
    /// Pairs that share an identity and do not look like one thing.
    pub distinct: usize,
}

/// Fold the holders of each identity that look like one thing, and queue the refusals.
pub(super) async fn reconcile(
    knowledge: &Knowledge,
    conn: &DatabaseConnection,
) -> anyhow::Result<Report> {
    let evidence = Evidence::new(conn.clone());
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT c.knowledge_identity_id \
               FROM knowledge_claims c \
               JOIN knowledge_entities e ON e.id = c.knowledge_entity_id \
              WHERE c.retracted_at IS NULL AND e.merged_into_id IS NULL \
              GROUP BY 1 HAVING count(*) > 1 \
              ORDER BY 1",
        ))
        .await?;

    let mut report = Report::default();
    for row in rows {
        let identity_id: i32 = row.try_get_by_index(0)?;
        report.groups += 1;

        // Most substantiated first: it survives, the rest are asked about.
        let holders = evidence.held_by_identity(identity_id).await?;
        let Some(winner) = holders.first().cloned() else {
            continue;
        };
        for loser in holders.iter().skip(1) {
            let features = heuristic::score(
                &NormalizedObservation::new(
                    &loser.name,
                    loser.description.as_deref(),
                    &loser.entity_type,
                ),
                &NormalizedCandidate::from(&winner),
            );
            if !heuristic::worth_merging(&features) {
                report.distinct += 1;
                continue;
            }
            match knowledge
                .merge(loser.id, winner.id, Cause::IdentityConflict)
                .await?
            {
                Outcome::Merged => report.merged += 1,
                Outcome::AlreadyMerged => {}
                // The veto says these two cannot be one referent, so the graph holds a real
                // contradiction: record it for adjudication rather than folding.
                Outcome::RefusedSharedIllumination => {
                    record_collision(conn, winner.id, loser.id, identity_id).await?;
                    report.refused += 1;
                }
            }
        }
    }
    Ok(report)
}

/// Queue a refused pair. The partial unique index makes a repeat a no-op.
async fn record_collision(
    conn: &DatabaseConnection,
    holder: i32,
    claimant: i32,
    identity_id: i32,
) -> anyhow::Result<()> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO knowledge_collisions \
           (holder_id, claimant_id, knowledge_identity_id, reason) \
         VALUES ($1, $2, $3, 'shared_illumination') \
         ON CONFLICT (holder_id, claimant_id, knowledge_identity_id) \
           WHERE adjudicated_at IS NULL \
         DO NOTHING",
        [holder.into(), claimant.into(), identity_id.into()],
    ))
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fixtures, test_db};

    /// Two entities claiming one identity that look like one thing are folded.
    #[tokio::test]
    async fn holders_that_look_alike_are_merged() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        let first = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let second = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::claim(&db, first, "wikidata", "Q1", "wikidata", None, None).await;
        fixtures::claim(&db, second, "wikidata", "Q1", "wikidata", None, None).await;

        let report = store.reconcile().await.expect("reconcile");

        assert_eq!(report.groups, 1, "{report:?}");
        assert_eq!(report.merged, 1, "{report:?}");
        assert_eq!(report.distinct, 0, "{report:?}");
    }

    /// Holders that do not look alike keep the identity: the album article, and the songs it covers.
    #[tokio::test]
    async fn holders_that_do_not_look_alike_are_left_alone() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        let album = fixtures::knowledge_entity_with_member(&db, "Belfast, Maine").await;
        let song = fixtures::knowledge_entity_with_member(&db, "The Fishing Fleet").await;
        fixtures::claim(&db, album, "wikidata", "Q1", "wikidata", None, None).await;
        fixtures::claim(&db, song, "wikidata", "Q1", "wikidata", None, None).await;

        let report = store.reconcile().await.expect("reconcile");

        assert_eq!(report.groups, 1, "{report:?}");
        assert_eq!(report.merged, 0, "sharing an identity is not a merge");
        assert_eq!(report.distinct, 1, "{report:?}");
    }
}
