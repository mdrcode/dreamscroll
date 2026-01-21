use axum_login::AuthUser;

use crate::auth::WebAuthUser;

/// Security context for business logic which is agnostic to the authentication
/// mechanism (session-based, JWT, etc.). The ultimate (aspirational) goal is
/// that this type encapsulates both the identity and authorization information
/// about the user. So if an administrator wants to perform some action on
/// behalf of another user, the context would reflect both the admin's
/// authority and the target user's identity.
///
/// Different auth extractors convert their user representations into this
/// common context, allowing the same business logic to work with multiple
/// authentication strategies.
///
/// Currently contains user identity information, with plans to extend with
/// permissions and roles as the authorization model evolves.
#[derive(Debug, Clone)]
pub struct Context {
    user_id: i32,
    // TODO more to come
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
