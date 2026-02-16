pub mod backfill_search;
pub mod create_user;
pub mod enums;
pub mod eval;
pub mod export_digest;
pub mod export_uniq;
pub mod html_view;
pub mod illuminate;
pub mod import_digest;

mod auth_helper;

pub struct CmdState {
    pub user_api: crate::api::UserApiClient,
    pub import_api: crate::api::ImportApiClient,
    pub db: crate::database::DbHandle,
    pub stg: Box<dyn crate::storage::StorageProvider>,
}
