pub mod backfill_search;
pub mod create_user;
pub mod enums;
pub mod eval;
pub mod export_digest;
pub mod export_uniq;
pub mod html_view;
pub mod illuminate;
pub mod import;
pub mod import_digest;

mod auth_helper;

pub struct CmdState {
    pub api_client: crate::api::ApiClient,
    pub db: crate::database::DbHandle,
    pub stg: Box<dyn crate::storage::StorageProvider>,
}
