use crate::model::user;

use axum_login::AuthUser;

// This trait tells axum-login how to identify your user
impl AuthUser for user::Model {
    type Id = i32;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        self.password_hash.as_bytes()
    }
}
