mod builder;
mod error;
mod helper_structs;

pub mod prelude {
    pub use super::error::*;
    pub use super::helper_structs::*;
}

pub use builder::WebState;
pub use builder::build_axum_router;

pub mod r_detail;
pub mod r_index;
pub mod r_upload;
