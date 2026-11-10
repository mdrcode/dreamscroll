//! Proximities as a person reads them: what each pair is, how close, and whether a person has
//! already decided it.
//!
//! A report over the tables rather than graph work, so it lives here with the other reports.

use std::collections::BTreeMap;

use sea_orm::{
    ConnectionTrait, DatabaseBackend, EntityTrait, QueryOrder, Statement,
};

use crate::database;
use crate::knowledge::verdicts::{Cause, pair_key};
use crate::model;
use crate::model::knowledge_verdicts::Kind;

/// What the graph says about the pair right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// One side is a tombstone: the pair is folded.
    Merged,
    /// Both live, but a standing `repair` verdict says they should be folded and replay has not
    /// applied it yet.
    MergePending,
    /// Both live, and a `distinct` verdict says they are deliberately separate.
    KeptApart,
    /// Both live, no standing merge decision.
    Open,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::MergePending => "merge_pending",
            Self::KeptApart => "kept_apart",
            Self::Open => "",
        }
    }

    /// The wire word for filters; `open` is explicit here even though the text column is blank.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "merged" => Some(Self::Merged),
            "merge_pending" => Some(Self::MergePending),
            "kept_apart" => Some(Self::KeptApart),
            "open" => Some(Self::Open),
            _ => None,
        }
    }

    pub fn filter_name(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::MergePending => "merge_pending",
            Self::KeptApart => "kept_apart",
            Self::Open => "open",
        }
    }
}

/// Why a pair is decided. `replayed` is an application event, not a why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Repair,
    ProximityAuto,
    Distinct,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repair => "repair",
            Self::ProximityAuto => "proximity_auto",
            Self::Distinct => "distinct",
        }
    }
}

/// One candidate, readable.
pub struct ProximityRow {
    pub id: i32,
    pub score: Option<f32>,
    /// Why the pair is worth a second look — `same entity_type`, or blank.
    pub type_note: String,
    pub basis: String,
    /// The entity ids `merge` and `eval inspect` take; the names are for reading.
    pub holder_id: i32,
    pub counterpart_id: i32,
    pub holder: String,
    pub counterpart: String,
    pub identity: String,
    /// The stable keys the verdict log uses: origin observation ids, not generation-scoped entity ids.
    pub holder_origin: Option<i32>,
    pub counterpart_origin: Option<i32>,
    pub status: Status,
    /// The standing decision, when there is one: `repair`, `proximity_auto` or `distinct`.
    /// `replayed` is an application event and is deliberately not a why here.
    pub why: Option<Why>,
}

impl ProximityRow {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "row": self.id,
            "status": match self.status {
                Status::Merged => "merged",
                Status::MergePending => "merge_pending",
                Status::KeptApart => "kept_apart",
                Status::Open => "open",
            },
            "why": self.why.map(|why| why.as_str()),
            "score": self.score,
            "type_note": self.type_note,
            "basis": self.basis,
            "holder_id": self.holder_id,
            "counterpart_id": self.counterpart_id,
            "holder_origin": self.holder_origin,
            "counterpart_origin": self.counterpart_origin,
            "holder": self.holder,
            "counterpart": self.counterpart,
            "identity": self.identity,
        })
    }
}

/// One row per candidate. Ordered for triage, not by score: the highest scores are the pairs that
/// must *never* merge (same name, different type: Nintendo the organisation and the brand sit at
/// 0.74), while the actionable ones are same-type pairs whose names differ, where the shared identity
/// is all the evidence there is. Those come first, then score descending.
pub async fn proximity_list(
    db: &database::DbHandle,
    limit: usize,
    status: Option<Status>,
) -> anyhow::Result<Vec<ProximityRow>> {
    let rows = db
        .conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT p.id, \
                    p.score, \
                    CASE WHEN h.entity_type = c.entity_type \
                         THEN 'same entity_type' ELSE '' END, \
                    p.basis, \
                    h.name || ' [' || h.entity_type || ']', \
                    c.name || ' [' || c.entity_type || ']', \
                    coalesce(i.scheme || ':' || i.external_id, ''), \
                    h.origin_observation_id, \
                    c.origin_observation_id, \
                    h.id, \
                    c.id, \
                    h.merged_into_id, \
                    c.merged_into_id \
               FROM knowledge_proximities p \
               JOIN knowledge_entities h ON h.id = p.holder_id \
               JOIN knowledge_entities c ON c.id = p.counterpart_id \
               LEFT JOIN knowledge_identities i ON i.id = p.knowledge_identity_id \
              ORDER BY (h.entity_type = c.entity_type) DESC, \
                       p.score DESC NULLS LAST, \
                       p.id",
        ))
        .await?;

    // A verdict names two origins; a candidate row names two entities. This is where the two meet.
    // The latest standing decision wins: a repair beats an auto fold, and a distinct verdict is a
    // decision of its own. `replayed` is an application event, not a decision.
    let mut decisions: BTreeMap<(i32, i32), Why> = BTreeMap::new();
    for verdict in model::knowledge_verdicts::Entity::find()
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(&db.conn)
        .await?
    {
        let (Some(a), Some(b)) = (verdict.winner_origin, verdict.loser_origin) else {
            continue;
        };
        let pair = pair_key(a, b);
        match verdict.kind {
            Kind::Merge => match verdict.cause {
                Cause::Repair => {
                    decisions.insert(pair, Why::Repair);
                }
                Cause::ProximityAuto => {
                    decisions.entry(pair).or_insert(Why::ProximityAuto);
                }
                Cause::Replayed => {}
            },
            Kind::Distinct => {
                decisions.insert(pair, Why::Distinct);
            }
            Kind::VetoIdentity => {}
        }
    }

    let mut rows: Vec<ProximityRow> = rows
        .iter()
        .map(|row| {
            let holder_origin: Option<i32> = row.try_get_by_index(7)?;
            let counterpart_origin: Option<i32> = row.try_get_by_index(8)?;
            let holder_merged: Option<i32> = row.try_get_by_index(11)?;
            let counterpart_merged: Option<i32> = row.try_get_by_index(12)?;

            let why = match (holder_origin, counterpart_origin) {
                (Some(h), Some(c)) => decisions.get(&pair_key(h, c)).copied(),
                _ => None,
            };
            let merged = holder_merged.is_some() || counterpart_merged.is_some();
            let (status, why) = if merged {
                (Status::Merged, why)
            } else if why == Some(Why::Distinct) {
                (Status::KeptApart, why)
            } else if why == Some(Why::Repair) {
                (Status::MergePending, why)
            } else {
                // An old `proximity_auto` verdict on two live entities is history: the auto pass
                // re-derives every generation, so it is not a standing decision.
                (Status::Open, None)
            };

            Ok(ProximityRow {
                id: row.try_get_by_index(0)?,
                score: row.try_get_by_index(1)?,
                type_note: row.try_get_by_index(2)?,
                basis: row.try_get_by_index(3)?,
                holder: row.try_get_by_index(4)?,
                counterpart: row.try_get_by_index(5)?,
                identity: row.try_get_by_index(6)?,
                holder_id: row.try_get_by_index(9)?,
                counterpart_id: row.try_get_by_index(10)?,
                holder_origin,
                counterpart_origin,
                status,
                why,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    rows.retain(|row| status.map_or(true, |filter| row.status == filter));
    rows.truncate(limit);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{db_or_skip, fixtures};
    use sea_orm::sea_query::Expr;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};

    async fn add_proximity(db: &database::DbHandle, holder: i32, counterpart: i32) -> i32 {
        model::knowledge_proximities::Entity::insert(model::knowledge_proximities::ActiveModel {
            holder_id: Set(holder),
            counterpart_id: Set(counterpart),
            knowledge_identity_id: Set(None),
            basis: Set("shared_id".to_string()),
            score: Set(Some(0.5)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32
    }

    async fn add_verdict(db: &database::DbHandle, winner: i32, loser: i32, cause: Cause) {
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            cause: Set(cause),
            kind: Set(model::knowledge_verdicts::Kind::Merge),
            winner_origin: Set(Some(winner)),
            loser_origin: Set(Some(loser)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
    }

    async fn add_distinct(db: &database::DbHandle, a: i32, b: i32) {
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            cause: Set(Cause::Repair),
            kind: Set(model::knowledge_verdicts::Kind::Distinct),
            winner_origin: Set(Some(a)),
            loser_origin: Set(Some(b)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
    }

    async fn tombstone(db: &database::DbHandle, loser: i32, winner: i32) {
        model::knowledge_entity::Entity::update_many()
            .col_expr(
                model::knowledge_entity::Column::MergedIntoId,
                Expr::value(Some(winner)),
            )
            .filter(model::knowledge_entity::Column::Id.eq(loser))
            .exec(&db.conn)
            .await
            .unwrap();
    }

    /// `status` is the graph fact, `why` the standing decision: a repair with both sides live is
    /// pending, a distinct verdict keeps them apart, an auto verdict on two live sides is history,
    /// and only an applied fold is merged.
    #[tokio::test]
    async fn status_separates_the_decision_from_its_application() {
        db_or_skip!(db);

        let pending_loser = fixtures::knowledge_entity_with_member(&db, "Pending Loser").await;
        let pending_winner = fixtures::knowledge_entity_with_member(&db, "Pending Winner").await;
        let pending_row = add_proximity(&db, pending_loser, pending_winner).await;
        add_verdict(
            &db,
            fixtures::origin(&db, pending_winner).await,
            fixtures::origin(&db, pending_loser).await,
            Cause::Repair,
        )
        .await;

        let auto_loser = fixtures::knowledge_entity_with_member(&db, "Auto Loser").await;
        let auto_winner = fixtures::knowledge_entity_with_member(&db, "Auto Winner").await;
        let auto_row = add_proximity(&db, auto_loser, auto_winner).await;
        add_verdict(
            &db,
            fixtures::origin(&db, auto_winner).await,
            fixtures::origin(&db, auto_loser).await,
            Cause::ProximityAuto,
        )
        .await;
        tombstone(&db, auto_loser, auto_winner).await;

        let old_loser = fixtures::knowledge_entity_with_member(&db, "Old Loser").await;
        let old_winner = fixtures::knowledge_entity_with_member(&db, "Old Winner").await;
        let old_row = add_proximity(&db, old_loser, old_winner).await;
        add_verdict(
            &db,
            fixtures::origin(&db, old_winner).await,
            fixtures::origin(&db, old_loser).await,
            Cause::ProximityAuto,
        )
        .await;

        let distinct_loser = fixtures::knowledge_entity_with_member(&db, "Distinct Loser").await;
        let distinct_winner = fixtures::knowledge_entity_with_member(&db, "Distinct Winner").await;
        let distinct_row = add_proximity(&db, distinct_loser, distinct_winner).await;
        add_distinct(
            &db,
            fixtures::origin(&db, distinct_winner).await,
            fixtures::origin(&db, distinct_loser).await,
        )
        .await;

        let rows = proximity_list(&db, 50, None).await.unwrap();
        let row = |id: i32| rows.iter().find(|row| row.id == id).expect("row");

        assert_eq!(row(pending_row).status, Status::MergePending);
        assert_eq!(row(pending_row).why, Some(Why::Repair));
        assert_eq!(row(pending_row).to_json()["status"], "merge_pending");
        assert_eq!(row(pending_row).to_json()["why"], "repair");
        assert!(row(pending_row).to_json()["holder_origin"].is_number());

        assert_eq!(row(auto_row).status, Status::Merged);
        assert_eq!(row(auto_row).why, Some(Why::ProximityAuto));
        assert_eq!(row(auto_row).to_json()["why"], "proximity_auto");

        assert_eq!(row(distinct_row).status, Status::KeptApart);
        assert_eq!(row(distinct_row).why, Some(Why::Distinct));
        assert_eq!(row(distinct_row).to_json()["status"], "kept_apart");
        assert_eq!(row(distinct_row).to_json()["why"], "distinct");

        assert_eq!(row(old_row).status, Status::Open);
        assert_eq!(row(old_row).why, None);
        assert_eq!(row(old_row).to_json()["status"], "open");
        assert!(row(old_row).to_json()["why"].is_null());
    }
}
