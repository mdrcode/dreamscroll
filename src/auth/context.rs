use axum_login::AuthUser;

use crate::auth::WebAuthUser;

#[derive(Debug, Clone)]
pub struct Context {
    user_id: i32,
    // TODO more to come here
}

impl Context {
    pub fn user_id(&self) -> i32 {
        self.user_id
    }
}

impl From<&WebAuthUser> for Context {
    fn from(user: &WebAuthUser) -> Self {
        Context { user_id: user.id() }
    }
}
