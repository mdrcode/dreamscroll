use axum::{Form, response::Redirect};
use axum_login::AuthSession;

use crate::auth;

pub async fn login_handler(
    mut auth: AuthSession<auth::WebAuthBackend>,
    Form(creds): Form<auth::Credentials>,
) -> Result<Redirect, axum::http::StatusCode> {
    let authentication = auth.authenticate(creds).await;

    let user = match authentication {
        Ok(Some(user)) => user,
        Ok(None) => {
            return Ok(Redirect::to("/login?error=Invalid+username+or+password"));
        }
        Err(_) => {
            return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if auth.login(&user).await.is_err() {
        return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/"))
}

pub async fn logout_handler(
    mut auth: AuthSession<auth::WebAuthBackend>,
) -> Result<Redirect, axum::http::StatusCode> {
    if auth.logout().await.is_err() {
        return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/login"))
}
