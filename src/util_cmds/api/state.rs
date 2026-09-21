use anyhow::Context;

use crate::rest;

use super::token_cache;
use crate::util_cmds::{prompt_password_stdin, prompt_username_stdin};

pub struct ApiCmdState {
    pub cfg: crate::config::Config,
    pub rest_host: String,
    pub rest_user: Option<String>,
    rest_client: Option<rest::client::Client>,
}

impl ApiCmdState {
    pub fn from_config(
        cfg: crate::config::Config,
        rest_host: String,
        rest_user: Option<String>,
    ) -> Self {
        Self {
            cfg,
            rest_host,
            rest_user,
            rest_client: None,
        }
    }

    pub async fn rest_client(&mut self) -> anyhow::Result<rest::client::Client> {
        if let Some(client) = &self.rest_client {
            return Ok(client.clone());
        }
        println!("Using REST host: {}", self.rest_host);
        let username = self.rest_user.clone().unwrap_or(prompt_username_stdin()?);
        if username.trim().is_empty() {
            anyhow::bail!("Cannot construct REST client without a username.");
        }
        let client = Self::initialize_rest_client(&self.rest_host, username.trim()).await?;
        self.rest_client = Some(client.clone());
        Ok(client)
    }

    async fn initialize_rest_client(
        host: &str,
        username: &str,
    ) -> anyhow::Result<rest::client::Client> {
        if let Some(token) = token_cache::get_token(host, username)? {
            let client = rest::client::Client::connect_with_token(host, token)
                .context("failed to initialize REST client from cached token")?;
            match client.validate_auth().await {
                Ok(()) => return Ok(client),
                Err(err) if err.to_string().contains("unauthorized (401)") => {
                    let _ = token_cache::delete_token(host, username);
                }
                Err(err) => return Err(err).context("failed to validate cached API token"),
            }
        }
        let password = prompt_password_stdin()?;
        let client = rest::client::Client::connect(host, username, &password).await?;
        if let Err(err) = token_cache::set_token(host, username, client.access_token()) {
            eprintln!("Warning: unable to cache API token: {}", err);
        }
        Ok(client)
    }
}
