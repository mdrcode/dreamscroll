use anyhow::Context;

use crate::{api, database, facility, rest, search, storage, task};

use super::*;

pub struct CmdState {
    pub config: facility::Config,
    pub rest_host: Option<String>,
    pub rest_user: Option<String>,

    db: Option<database::DbHandle>,
    stg: Option<Box<dyn storage::StorageProvider>>,
    user_api: Option<api::UserApiClient>,
    service_api: Option<api::ServiceApiClient>,
    rest_client: Option<rest::client::Client>,
}

impl CmdState {
    pub async fn from_config(
        config: facility::Config,
        rest_host: Option<String>,
        rest_user: Option<String>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            config,
            rest_host,
            rest_user,
            db: None,
            stg: None,
            user_api: None,
            service_api: None,
            rest_client: None,
        })
    }

    pub async fn db_handle(&mut self) -> anyhow::Result<database::DbHandle> {
        if self.db.is_none() {
            let (db_connection, _) = database::connect(&self.config).await?;
            self.db = Some(database::DbHandle::new(db_connection));
        }

        Ok(self
            .db
            .as_ref()
            .expect("db should be initialized before access")
            .clone())
    }

    pub async fn storage_provider(&mut self) -> anyhow::Result<Box<dyn storage::StorageProvider>> {
        if self.stg.is_none() {
            let stg = storage::make_provider(&self.config).await;
            self.stg = Some(stg);
        }

        Ok(self
            .stg
            .as_ref()
            .expect("storage provider should be initialized before access")
            .clone())
    }

    pub async fn user_api_client(&mut self) -> anyhow::Result<api::UserApiClient> {
        if self.user_api.is_none() {
            let db = self.db_handle().await?;
            let stg = self.storage_provider().await?;
            let url_maker = storage::UrlMaker::from_config(&self.config);

            // We use an empty beacon for the util commands, so no background tasks
            // will be enqueued.
            // TODO this should be a NOOP queue that logs tasks so we can verify behavior
            let empty_beacon = task::Beacon::default();
            let searcher = search::CaptureSearcher::from_config(&self.config)
                .await
                .context("Failed to initialize required CaptureSearcher")?;

            self.user_api = Some(api::UserApiClient::new(
                db,
                stg,
                url_maker,
                empty_beacon,
                searcher,
            ));
        }

        Ok(self
            .user_api
            .as_ref()
            .expect("user api should be initialized before access")
            .clone())
    }

    pub async fn service_api_client(&mut self) -> anyhow::Result<api::ServiceApiClient> {
        if self.service_api.is_none() {
            let db = self.db_handle().await?;
            let url_maker = storage::UrlMaker::from_config(&self.config);
            self.service_api = Some(api::ServiceApiClient::new(db, url_maker));
        }

        Ok(self
            .service_api
            .as_ref()
            .expect("service api should be initialized before access")
            .clone())
    }

    pub async fn rest_client(&mut self) -> anyhow::Result<rest::client::Client> {
        if let Some(rest_client) = self.rest_client.as_ref() {
            return Ok(rest_client.clone());
        }

        let rest_host = self
            .rest_host
            .as_deref()
            .context("Cannot construct REST API client without a host.")?
            .to_string();

        println!("Using REST host: {}", rest_host);

        let username = if let Some(user) = self.rest_user.as_deref() {
            user.trim().to_string()
        } else {
            prompt_username_stdin()?
        };

        if username.is_empty() {
            anyhow::bail!("Cannot construct REST API client without a username.");
        }

        let rest_client = Self::initialize_rest_client(&rest_host, &username).await?;
        self.rest_client = Some(rest_client);

        Ok(self
            .rest_client
            .as_ref()
            .expect("rest client should be initialized before access")
            .clone())
    }

    async fn initialize_rest_client(
        rest_host: &str,
        username: &str,
    ) -> anyhow::Result<rest::client::Client> {
        if let Some(cached_token) = token_cache::get_token(rest_host, username)? {
            println!(
                "Found cached API token for host='{}' username='{}'.",
                rest_host, username
            );
            let cached_client = rest::client::Client::connect_with_token(rest_host, cached_token)
                .context("failed to initialize REST client from cached token")?;

            match cached_client.validate_auth().await {
                Ok(()) => {
                    println!("Cached API token is valid.");
                    Ok(cached_client)
                }
                Err(err) => {
                    if err.to_string().contains("unauthorized (401)") {
                        println!("Cached token expired or invalid; requesting a new token.");
                        let _ = token_cache::delete_token(rest_host, username);
                        let password = prompt_password_stdin()?;
                        let fresh_client =
                            rest::client::Client::connect(rest_host, username, &password).await?;
                        println!("Successfully authenticated and retrieved API token.");
                        if let Err(cache_err) =
                            token_cache::set_token(rest_host, username, fresh_client.access_token())
                        {
                            eprintln!("Warning: unable to cache API token: {}", cache_err);
                        } else {
                            println!("Successfully cached API token.");
                        }
                        Ok(fresh_client)
                    } else {
                        println!("Cached token validation failed for a non-auth reason.");
                        Err(err).context("failed to validate cached API token")
                    }
                }
            }
        } else {
            let password = prompt_password_stdin()?;
            let fresh_client =
                rest::client::Client::connect(rest_host, username, &password).await?;
            println!("Successfully authenticated and retrieved API token.");
            if let Err(cache_err) =
                token_cache::set_token(rest_host, username, fresh_client.access_token())
            {
                eprintln!("Warning: unable to cache API token: {}", cache_err);
            } else {
                println!("Successfully cached API token.");
            }
            Ok(fresh_client)
        }
    }
}
