use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// Durable record of one code-defined backfill's manual pumping.
///
/// The backfill ID is the definition identity: change the literal to start a
/// new epoch. Progress itself lives in `task_runs`; this row only records what
/// the pump observed, never a claim about the work graph.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "backfills")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub backfill_id: String,

    /// When the most recent pump actually ran. A refused pump writes nothing.
    pub last_pumped_at: DateTime<Utc>,

    /// Start of the current dry streak: set by a pump that selected no targets,
    /// preserved by later dry pumps, cleared by a pump that finds work. While
    /// set, pumping is refused unless forced.
    pub dry_since: Option<DateTime<Utc>>,
}

impl ActiveModelBehavior for ActiveModel {}
