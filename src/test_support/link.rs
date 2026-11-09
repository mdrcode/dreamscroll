//! Rebuild the whole graph, using only the public API.
//!
//! Two callers need this shape — the CLI's `link` command and the tests that want a populated
//! graph — and neither belongs in the library, which only knows how to link *one* illumination.
//! Writing it here is also the check that the library's boundary is sufficient: everything below
//! is public.

use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

use crate::database::DbHandle;
use crate::knowledge::blocker::SqlBlocker;
use crate::model;

/// Link every illumination that still has unlinked observations.
pub async fn link_all(db: &DbHandle) -> anyhow::Result<()> {
    // One blocker for the whole rebuild: the store holds it and keeps it current as it links.
    let knowledge = crate::knowledge::Knowledge::with_linking(
        db,
        Box::new(SqlBlocker::new(db)),
        std::sync::Arc::new(crate::knowledge::linker::heuristic::HeuristicLinker),
    );

    let pending = Query::select()
        .column(model::user_observed_entity::Column::IlluminationId)
        .from(model::user_observed_entity::Entity)
        .and_where(model::user_observed_entity::Column::KnowledgeEntityId.is_null())
        .to_owned();

    // Ordered: linking an illumination changes what a later one finds, so the order decides who owns a
    // contested entity.
    let illuminations = model::illumination::Entity::load()
        .filter(Expr::col(model::illumination::Column::Id).in_subquery(pending))
        .with(model::user_observed_entity::Entity)
        .order_by_asc(model::illumination::Column::Id)
        .all(&db.conn)
        .await?;

    for illumination in &illuminations {
        knowledge.link_illumination(illumination).await?;
    }
    Ok(())
}
