mod apierror;
pub use apierror::*;

mod schema;
pub use schema::*;

mod admin;
pub use admin::{AdminApiClient, create_user};

mod service;
pub use service::client::ServiceApiClient;

mod user;
pub use user::client::UserApiClient; // TODO maybe this should live elsewhere
