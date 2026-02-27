pub mod backfill_search;
pub mod check_first_user;
pub mod create_user;
pub mod enums;
// pub mod eval; ignore for now until refactoring allows creating multiple different illuminators
pub mod export_digest;
pub mod first_user;
pub mod html_view;
pub mod illuminate_all;
pub mod illuminate_id;
pub mod import_digest;

mod auth_helper;

pub struct CmdState {
    pub config: crate::facility::Config,
    pub user_api: crate::api::UserApiClient,
    pub import_api: crate::api::ImportApiClient,
    pub service_api: crate::api::ServiceApiClient,
    pub db: crate::database::DbHandle,
    pub stg: Box<dyn crate::storage::StorageProvider>,
}
