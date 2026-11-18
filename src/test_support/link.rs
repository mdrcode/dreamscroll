//! Rebuild the whole graph, for tests that want a populated one.
//!
//! The batch is the library's; this wrapper only discards the run report, so a test can say
//! `link_all(&db).await.unwrap()`.

use crate::database::DbHandle;

/// Link every illumination that still has unlinked observations.
pub async fn link_all(db: &DbHandle) -> anyhow::Result<()> {
    crate::knowledge2::LinkHelper::new(db)
        .link_all_pending(db)
        .await?;
    Ok(())
}
