use anyhow::anyhow;

use crate::{api::*, auth, database};

pub struct AdminApiClient {
    db: database::DbHandle,
    _admin_context: auth::Context,
}

impl AdminApiClient {
    pub fn new(db: database::DbHandle, admin_context: auth::Context) -> Result<Self, ApiError> {
        if !admin_context.is_admin() {
            return Err(ApiError::forbidden(anyhow!(
                "Only admin users can create new users"
            )));
        }

        Ok(Self {
            db,
            _admin_context: admin_context,
        })
    }

    pub async fn create_user(
        &self,
        username: String,
        password: String,
        email: String,
    ) -> Result<UserInfo, ApiError> {
        super::create_user(&self.db, username, password, email).await
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::auth::{Context, DreamscrollAuthUser, JwtUserClaims};

    fn make_context(is_admin: bool) -> Context {
        let claims = JwtUserClaims {
            sub: "123".to_string(),
            username: if is_admin {
                "admin_user".to_string()
            } else {
                "normal_user".to_string()
            },
            is_admin,
            exp: 9_999_999_999,
            iat: 1_000_000_000,
            storage_shard: "testshard".to_string(),
        };

        let user = DreamscrollAuthUser::try_from(claims).expect("valid claims should parse");
        user.into()
    }

    async fn make_test_db() -> database::DbHandle {
        let conn = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        database::DbHandle::new(conn)
    }

    #[tokio::test]
    async fn new_rejects_non_admin_context() {
        let db = make_test_db().await;
        let non_admin_context = make_context(false);

        let result = AdminApiClient::new(db, non_admin_context);

        assert!(result.is_err());
        let err = result.err().unwrap();
        assert_eq!(err.status_code, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn new_accepts_admin_context() {
        let db = make_test_db().await;
        let admin_context = make_context(true);

        let result = AdminApiClient::new(db, admin_context);

        assert!(result.is_ok());
    }
}
