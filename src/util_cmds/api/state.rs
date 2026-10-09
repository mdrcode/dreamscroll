use crate::{config, rest};

pub struct ApiCmdState {
    pub cfg: config::Config,
    pub rest_host: String,
    pub rest_user: Option<String>,
    pub rest_client: rest::client::Client,
}

impl ApiCmdState {
    pub fn from_config(
        cfg: config::Config,
        rest_host: String,
        rest_user: Option<String>,
        rest_client: rest::client::Client,
    ) -> Self {
        Self {
            cfg,
            rest_host,
            rest_user,
            rest_client,
        }
    }
}
