use std::collections::HashMap;

use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::{QueryOrder, QuerySelect};

use crate::{api::*, auth, database::DbHandle, model};

/// Page size used when the caller does not ask for one.
pub const EXPORT_DEFAULT_LIMIT: u64 = 1_000;
/// Largest page the export will serve; callers page with `after_id`.
pub const EXPORT_MAX_LIMIT: u64 = 5_000;

#[derive(Clone, Debug)]
pub struct ExportEntitiesRequest {
    /// Restrict the export to one user; `None` streams every user in one pass.
    pub user_id: Option<i32>,
    /// Exclusive cursor: only entities with a greater id are returned.
    pub after_id: i32,
    pub limit: u64,
}

/// Export knowledge entities, one `id`-ordered page at a time.
///
/// Admin only: rows carry another user's entity text, so this deliberately
/// bypasses every per-user scoping rule in `get_knowledge_entity`. Archived
/// captures are included; the row says so via `capture_archived_at`.
///
/// Returns the page plus the cursor for the next one (`None` when exhausted).
pub async fn export_entities(
    db: &DbHandle,
    context: &auth::Context,
    req: ExportEntitiesRequest,
) -> Result<
    (
        Vec<schema::ExportRow<schema::KnowledgeEntityInfo>>,
        Option<i32>,
    ),
    ApiError,
> {
    super::client::ensure_admin(context)?;

    if req.limit == 0 || req.limit > EXPORT_MAX_LIMIT {
        return Err(ApiError::bad_request(anyhow!(
            "limit must be between 1 and {EXPORT_MAX_LIMIT}"
        )));
    }

    let mut query = model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::Id.gt(req.after_id))
        .order_by_asc(model::knowledge_entity::Column::Id)
        .limit(req.limit);
    if let Some(user_id) = req.user_id {
        query = query.filter(model::knowledge_entity::Column::UserId.eq(user_id));
    }
    let entities = query.all(&db.conn).await?;

    let next_after_id = if entities.len() as u64 == req.limit {
        entities.last().map(|entity| entity.id)
    } else {
        None
    };

    let mut capture_ids: Vec<i32> = entities.iter().map(|entity| entity.capture_id).collect();
    capture_ids.sort_unstable();
    capture_ids.dedup();

    // Archived captures are part of the export, so the archived flag cannot be
    // filtered out the way the product read paths do it.
    let archived: HashMap<i32, Option<chrono::DateTime<chrono::Utc>>> =
        model::capture::Entity::find()
            .filter(model::capture::Column::Id.is_in(capture_ids))
            .all(&db.conn)
            .await?
            .into_iter()
            .map(|capture| (capture.id, capture.archived_at))
            .collect();

    let mut rows = Vec::with_capacity(entities.len());
    for entity in entities {
        let capture_id = entity.capture_id;
        let archived_at = *archived.get(&capture_id).ok_or_else(|| {
            ApiError::internal(anyhow!(
                "knowledge entity {} references missing capture {capture_id}",
                entity.id
            ))
        })?;

        rows.push(schema::ExportRow {
            user_id: entity.user_id,
            row: schema::KnowledgeEntityInfo::new(
                model::knowledge_entity::ModelEx::from(entity),
                archived_at,
            ),
        });
    }

    Ok((rows, next_after_id))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::{
        auth::{self, JwtUserClaims},
        test_support::{fixtures, test_db},
    };

    use super::{ExportEntitiesRequest, export_entities};

    fn admin_context(user_id: i32) -> auth::Context {
        auth::Context::from(auth::DreamscrollAuthUser::new_test_jwt(
            user_id,
            JwtUserClaims {
                sub: user_id.to_string(),
                username: "export-admin".to_string(),
                is_admin: true,
                exp: u64::MAX,
                iat: 0,
                storage_shard: "testshard".to_string(),
            },
        ))
    }

    fn req(user_id: Option<i32>, after_id: i32, limit: u64) -> ExportEntitiesRequest {
        ExportEntitiesRequest {
            user_id,
            after_id,
            limit,
        }
    }

    #[tokio::test]
    async fn export_pages_exclusively_and_can_span_users() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();

        let alice = fixtures::user(&db, "export_alice").await;
        let bob = fixtures::user(&db, "export_bob").await;

        let capture_1 = fixtures::capture(&db, alice).await;
        let illumination = fixtures::illumination(&db, alice, capture_1).await;
        let first = fixtures::entity(&db, alice, capture_1, illumination, 0, "first").await;
        let second = fixtures::entity(&db, alice, capture_1, illumination, 1, "second").await;

        let archived_capture = fixtures::archived_capture(&db, alice).await;
        let archived_illumination = fixtures::illumination(&db, alice, archived_capture).await;
        let third = fixtures::entity(
            &db,
            alice,
            archived_capture,
            archived_illumination,
            0,
            "third",
        )
        .await;

        let bob_capture = fixtures::capture(&db, bob).await;
        let bob_illumination = fixtures::illumination(&db, bob, bob_capture).await;
        let bob_entity =
            fixtures::entity(&db, bob, bob_capture, bob_illumination, 0, "bob only").await;

        let context = admin_context(alice);

        // One user: ordering, labels and cursor.
        let (rows, next) = export_entities(&db, &context, req(Some(alice), 0, 2))
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.row.id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(rows.iter().all(|r| r.user_id == alice));
        assert_eq!(rows[0].row.illumination_id, illumination);
        assert_eq!(rows[0].row.capture_archived_at, None);
        assert_eq!(next, Some(second));

        let (rows, next) = export_entities(&db, &context, req(Some(alice), next.unwrap(), 2))
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.row.id).collect::<Vec<_>>(),
            vec![third]
        );
        // Archived captures are exported, and the row says so.
        assert!(rows[0].row.capture_archived_at.is_some());
        assert_eq!(next, None);

        // The cursor is exclusive and does not re-emit the last row.
        let (rows, next) = export_entities(&db, &context, req(Some(alice), third, 2))
            .await
            .unwrap();
        assert!(rows.is_empty());
        assert_eq!(next, None);

        // No filter: one stream carries both users, each line labelled.
        let (rows, _) = export_entities(&db, &context, req(None, 0, 10))
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|r| r.row.id == bob_entity && r.user_id == bob)
        );
        assert!(rows.iter().any(|r| r.user_id == alice));
    }

    #[tokio::test]
    async fn export_requires_admin() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "export_peon").await;
        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));

        let err = export_entities(&db, &context, req(None, 0, 10))
            .await
            .unwrap_err();
        assert_eq!(err.http_status_code, StatusCode::FORBIDDEN);
    }
}
