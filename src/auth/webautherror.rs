#[derive(Debug)]
pub enum WebAuthError {
    Database(sea_orm::DbErr),
    Other(anyhow::Error),
}

impl std::fmt::Display for WebAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebAuthError::Database(e) => write!(f, "Database error: {}", e),
            WebAuthError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for WebAuthError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WebAuthError::Database(e) => Some(e),
            WebAuthError::Other(e) => e.source(),
        }
    }
}

impl From<anyhow::Error> for WebAuthError {
    fn from(e: anyhow::Error) -> Self {
        WebAuthError::Other(e)
    }
}

impl From<argon2::password_hash::Error> for WebAuthError {
    fn from(e: argon2::password_hash::Error) -> Self {
        WebAuthError::Other(anyhow::anyhow!("Password hash error: {}", e))
    }
}

impl From<sea_orm::DbErr> for WebAuthError {
    fn from(e: sea_orm::DbErr) -> Self {
        WebAuthError::Database(e)
    }
}
