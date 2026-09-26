use crate::{api, config, database, storage};

pub struct AdminCmdState {
    pub cfg: config::Config,
    db: database::DbHandle,
    service_api: Option<api::ServiceApiClient>,
}

impl AdminCmdState {
    pub async fn from_config(cfg: config::Config) -> anyhow::Result<Self> {
        let (connection, _) = database::connect(&cfg).await?;
        Ok(Self {
            cfg,
            db: database::DbHandle::new(connection),
            service_api: None,
        })
    }

    pub async fn db_handle(&mut self) -> anyhow::Result<database::DbHandle> {
        Ok(self.db.clone())
    }

    pub async fn service_api_client(&mut self) -> anyhow::Result<api::ServiceApiClient> {
        if self.service_api.is_none() {
            let url_maker = storage::UrlMaker::from_config(&self.cfg)?;
            self.service_api = Some(api::ServiceApiClient::new(self.db.clone(), url_maker));
        }
        Ok(self
            .service_api
            .as_ref()
            .expect("service api initialized")
            .clone())
    }
}
