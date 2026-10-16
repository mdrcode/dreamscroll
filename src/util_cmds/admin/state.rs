use crate::{config, database};

pub struct AdminCmdState {
    pub cfg: config::Config,
    db: database::DbHandle,
}

impl AdminCmdState {
    pub async fn from_config(cfg: config::Config) -> anyhow::Result<Self> {
        let (connection, _) = database::connect(&cfg).await?;
        Ok(Self {
            cfg,
            db: database::DbHandle::new(connection),
        })
    }

    pub async fn db_handle(&mut self) -> anyhow::Result<database::DbHandle> {
        Ok(self.db.clone())
    }
}
