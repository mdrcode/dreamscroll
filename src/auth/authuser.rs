use axum_login::AuthUser;

use crate::entity::user;

#[derive(Clone)]
pub struct WebAuthUser {
    id: i32,
    session_hash: String,
}

impl std::fmt::Debug for WebAuthUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebAuthUser").field("id", &self.id).finish()
    }
}

// This trait tells axum-login how to identify your user
impl AuthUser for WebAuthUser {
    type Id = i32;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        self.session_hash.as_bytes()
    }
}

impl From<user::Model> for WebAuthUser {
    fn from(user_model: user::Model) -> Self {
        WebAuthUser {
            id: user_model.id,
            session_hash: user_model.password_hash,
        }
    }
}
