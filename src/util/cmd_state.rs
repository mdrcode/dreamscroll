use anyhow::Context;

use crate::{api, database, facility, rest, search, storage, task};

use super::*;

pub struct CmdState {
    pub config: facility::Config,
    pub rest_host: Option<String>,
    pub rest_user: Option<String>,

    db: database::DbHandle,
    stg: Box<dyn storage::StorageProvider>,
    user_api: api::UserApiClient,
    service_api: api::ServiceApiClient,
}

impl CmdState {
    pub async fn from_config(
        config: facility::Config,
        rest_host: Option<String>,
        rest_user: Option<String>,
    ) -> anyhow::Result<Self> {
        let (db_connection, _) = database::connect(&config).await?;
        let db = database::DbHandle::new(db_connection);

        let stg = storage::make_provider(&config).await;
        let url_maker = storage::UrlMaker::from_config(&config);

        // We use an empty beacon for the util commands, so no background tasks
        // will be enqueued.
        // TODO this should be a NOOP queue that logs tasks so we can verify behavior
        let empty_beacon = task::Beacon::default();
        let searcher = search::CaptureSearcher::from_config(&config)
            .await
            .context("Failed to initialize required CaptureSearcher")?;
        let user_api = api::UserApiClient::new(
            db.clone(),
            stg.clone(),
            url_maker.clone(),
            empty_beacon,
            searcher,
        );
        let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());

        Ok(Self {
            config,
            rest_host,
            rest_user,
            user_api,
            service_api,
            db,
            stg,
        })
    }

    pub fn db_handle(&self) -> database::DbHandle {
        self.db.clone()
    }

    pub fn storage_provider(&self) -> Box<dyn storage::StorageProvider> {
        self.stg.clone()
    }

    pub fn user_api_client(&self) -> api::UserApiClient {
        self.user_api.clone()
    }

    pub fn service_api_client(&self) -> api::ServiceApiClient {
        self.service_api.clone()
    }

    pub async fn rest_client(&self) -> anyhow::Result<rest::client::Client> {
        let rest_host = self
            .rest_host
            .as_ref()
            .context("Cannot construct REST API client without a host.")?;

        println!("Using REST host: {}", rest_host);

        let username = if let Some(user) = self.rest_user.as_ref() {
            user.trim().to_string()
        } else {
            prompt_username_stdin()?
        };

        if username.is_empty() {
            anyhow::bail!("Cannot construct REST API client without a username.");
        }

        let rest_client = if let Some(cached_token) = token_cache::get_token(&rest_host, &username)?
        {
            println!(
                "Found cached API token for host='{}' username='{}'.",
                rest_host, username
            );
            let cached_client = rest::client::Client::connect_with_token(&rest_host, cached_token)
                .context("failed to initialize REST client from cached token")?;

            match cached_client.validate_auth().await {
                Ok(()) => {
                    println!("Cached API token is valid.");
                    cached_client
                }
                Err(err) => {
                    if err.to_string().contains("unauthorized (401)") {
                        println!("Cached token expired or invalid; requesting a new token.");
                        let _ = token_cache::delete_token(&rest_host, &username);
                        let password = prompt_password_stdin()?;
                        let fresh_client =
                            rest::client::Client::connect(&rest_host, &username, &password).await?;
                        println!("Successfully authenticated and retrieved API token.");
                        if let Err(cache_err) = token_cache::set_token(
                            &rest_host,
                            &username,
                            fresh_client.access_token(),
                        ) {
                            eprintln!("Warning: unable to cache API token: {}", cache_err);
                        } else {
                            println!("Successfully cached API token.");
                        }
                        fresh_client
                    } else {
                        println!("Cached token validation failed for a non-auth reason.");
                        return Err(err).context("failed to validate cached API token");
                    }
                }
            }
        } else {
            let password = prompt_password_stdin()?;
            let fresh_client =
                rest::client::Client::connect(&rest_host, &username, &password).await?;
            println!("Successfully authenticated and retrieved API token.");
            if let Err(cache_err) =
                token_cache::set_token(&rest_host, &username, fresh_client.access_token())
            {
                eprintln!("Warning: unable to cache API token: {}", cache_err);
            } else {
                println!("Successfully cached API token.");
            }
            fresh_client
        };

        Ok(rest_client)
    }
}
