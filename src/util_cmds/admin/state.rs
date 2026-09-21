use std::sync::Arc;

use crate::{api, database, search, storage, task};

pub struct AdminCmdState {
    pub cfg: crate::config::Config,
    db: database::DbHandle,
    stg: Option<Box<dyn storage::StorageProvider>>,
    user_api: Option<api::UserApiClient>,
    service_api: Option<api::ServiceApiClient>,
}

impl AdminCmdState {
    pub async fn from_config(cfg: crate::config::Config) -> anyhow::Result<Self> {
        let (connection, _) = database::connect(&cfg).await?;
        Ok(Self {
            cfg,
            db: database::DbHandle::new(connection),
            stg: None,
            user_api: None,
            service_api: None,
        })
    }

    pub async fn db_handle(&mut self) -> anyhow::Result<database::DbHandle> {
        Ok(self.db.clone())
    }

    pub async fn storage_provider(&mut self) -> anyhow::Result<Box<dyn storage::StorageProvider>> {
        if self.stg.is_none() {
            self.stg = Some(storage::make_provider(&self.cfg).await);
        }
        Ok(self.stg.as_ref().expect("storage initialized").clone())
    }

    pub async fn user_api_client(&mut self) -> anyhow::Result<api::UserApiClient> {
        if self.user_api.is_none() {
            let stg = self.storage_provider().await?;
            let url_maker = storage::UrlMaker::from_config(&self.cfg);
            let task_master = Arc::new(task::TaskMaster::builder().db(self.db.clone()).build()?);
            let searcher = search::CaptureSearcher::from_config(&self.cfg).await?;
            self.user_api = Some(api::UserApiClient::new(
                self.db.clone(),
                stg,
                url_maker,
                task_master,
                searcher,
            ));
        }
        Ok(self
            .user_api
            .as_ref()
            .expect("user api initialized")
            .clone())
    }

    pub async fn service_api_client(&mut self) -> anyhow::Result<api::ServiceApiClient> {
        if self.service_api.is_none() {
            let url_maker = storage::UrlMaker::from_config(&self.cfg);
            self.service_api = Some(api::ServiceApiClient::new(self.db.clone(), url_maker));
        }
        Ok(self
            .service_api
            .as_ref()
            .expect("service api initialized")
            .clone())
    }
}
