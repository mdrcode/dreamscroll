mod builder;
mod error;

pub mod prelude {
    pub use super::error::*;
}

pub use builder::WebState;
pub use builder::make_axum_router;

pub mod r_detail;
pub mod r_index;
pub mod r_upload;
