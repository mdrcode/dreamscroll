use axum::{Form, http::StatusCode, response::Redirect};
use axum_login::AuthSession;
use serde::Deserialize;
use tower_sessions::Session;

use crate::auth;

#[derive(Deserialize)]
pub struct LoginFormData {
    pub username: String,
    pub password: String,
    /// Single-use token matching the value stored in the session by GET /login.
    pub csrf_token: String,
}

pub async fn login_handler(
    mut auth: AuthSession<auth::WebAuthBackend>,
    session: Session,
    Form(form): Form<LoginFormData>,
) -> Result<Redirect, StatusCode> {
    // --- CSRF validation ---
    // Read the expected token from the session and immediately remove it so
    // it cannot be replayed (even on a failed login attempt).
    let stored: Option<String> = session
        .remove("login_csrf_token")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match stored {
        Some(t) if t == form.csrf_token => {}
        _ => {
            // Token missing or mismatched — the form was not issued by us.
            tracing::warn!("CSRF token mismatch on POST /login");
            return Ok(Redirect::to("/login?error=Invalid+or+expired+form"));
        }
    }

    // --- Authentication ---
    let creds = auth::Credentials {
        username: form.username,
        password: form.password,
    };

    let authentication = auth.authenticate(creds).await;

    let user = match authentication {
        Ok(Some(user)) => user,
        Ok(None) => {
            return Ok(Redirect::to("/login?error=Invalid+username+or+password"));
        }
        Err(_) => {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if auth.login(&user).await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/"))
}

/// Logout is POST-only. This, combined with `SameSite=Lax` on the session
/// cookie, prevents forced-logout CSRF attacks since cross-site POSTs cannot carry
/// the session cookie.
pub async fn logout_handler(
    mut auth: AuthSession<auth::WebAuthBackend>,
) -> Result<Redirect, StatusCode> {
    if auth.logout().await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/login"))
}
