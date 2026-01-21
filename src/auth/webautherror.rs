#[derive(Debug)]
pub enum AuthError {
    Database(sea_orm::DbErr),
    Other(anyhow::Error),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Database(e) => write!(f, "Database error: {}", e),
            AuthError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for AuthError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AuthError::Database(e) => Some(e),
            AuthError::Other(e) => e.source(),
        }
    }
}

impl From<anyhow::Error> for AuthError {
    fn from(e: anyhow::Error) -> Self {
        AuthError::Other(e)
    }
}

impl From<argon2::password_hash::Error> for AuthError {
    fn from(e: argon2::password_hash::Error) -> Self {
        AuthError::Other(anyhow::anyhow!("Password hash error: {}", e))
    }
}

impl From<sea_orm::DbErr> for AuthError {
    fn from(e: sea_orm::DbErr) -> Self {
        AuthError::Database(e)
    }
}
