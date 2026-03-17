use axum::{Form, http::StatusCode, response::Redirect};
use axum_login::AuthSession;
use serde::Deserialize;
use tower_sessions::Session;

use crate::auth;

#[derive(Deserialize)]
pub struct LoginFormData {
    pub username: String,
    pub password: String,
    pub csrf_token: String,
}

pub async fn login_post(
    mut auth: AuthSession<auth::WebAuthBackend>,
    session: Session,
    Form(form): Form<LoginFormData>,
) -> Result<Redirect, StatusCode> {
    let stored: Option<String> = session
        .remove("login_csrf_token")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match stored {
        Some(t) if t == form.csrf_token => {}
        _ => {
            tracing::warn!("CSRF token mismatch on POST /v2/login");
            return Ok(Redirect::to("/v2/login?error=Invalid+or+expired+form"));
        }
    }

    let creds = auth::Credentials {
        username: form.username,
        password: form.password,
    };

    let authentication = auth.authenticate(creds).await;

    let user = match authentication {
        Ok(Some(user)) => user,
        Ok(None) => {
            return Ok(Redirect::to("/v2/login?error=Invalid+username+or+password"));
        }
        Err(_) => {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if auth.login(&user).await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/v2"))
}

pub async fn logout_post(
    mut auth: AuthSession<auth::WebAuthBackend>,
) -> Result<Redirect, StatusCode> {
    if auth.logout().await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/v2/login"))
}
