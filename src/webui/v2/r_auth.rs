use axum::{http::StatusCode, response::Redirect};
use axum_login::AuthSession;

use crate::auth;

pub async fn logout_post(
    mut auth: AuthSession<auth::WebAuthBackend>,
) -> Result<Redirect, StatusCode> {
    if auth.logout().await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Redirect::to("/login"))
}
