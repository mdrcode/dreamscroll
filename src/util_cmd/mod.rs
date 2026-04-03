pub mod backfill;
pub mod check_first_user;
pub mod clear_token;
pub mod create_user;
pub mod enums;
// pub mod eval; ignore for now until refactoring allows creating multiple different illuminators
pub mod export_digest;
pub mod first_user;
pub mod html_view;
pub mod illuminate_all;
pub mod illuminate_id;
pub mod illumination_text;
pub mod import_digest;
pub mod search;
pub mod search_index;
pub mod search_similar;
pub mod spark;
pub mod token_cache;

mod auth_helper;
pub use auth_helper::prompt_credentials_stdin;
pub use auth_helper::prompt_password_stdin;
pub use auth_helper::prompt_username_stdin;

pub struct CmdState {
    pub config: crate::facility::Config,
    pub user_api: crate::api::UserApiClient,
    pub service_api: crate::api::ServiceApiClient,
    pub rest_client: crate::rest::client::Client,
    pub rest_host: String,
    pub db: crate::database::DbHandle,
    pub stg: Box<dyn crate::storage::StorageProvider>,
}
