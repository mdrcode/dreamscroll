use super::DreamscrollAuthUser;

/// Security context for business logic which is agnostic to the authentication
/// mechanism (session-based, JWT, etc.). The ultimate (aspirational) goal is
/// that this type encapsulates both the identity and authorization information
/// about the user. So if an administrator wants to perform some action on
/// behalf of another user, the context would reflect both the admin's
/// authority and the target user's identity.
///
/// Currently contains user identity information, with plans to extend with
/// permissions and roles as the authorization model evolves.
#[derive(Debug, Clone)]
pub struct Context {
    user: DreamscrollAuthUser,
    // TODO more to come
}

impl Context {
    pub fn user_id(&self) -> i32 {
        self.user.user_id()
    }
}

impl From<DreamscrollAuthUser> for Context {
    fn from(user: DreamscrollAuthUser) -> Self {
        Context { user }
    }
}
