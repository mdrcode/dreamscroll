// Core search interfaces.
mod data_object;
pub use data_object::*;
mod embedder;
pub use embedder::*;
mod embedding;
pub use embedding::*;
mod searcher;
pub use searcher::*;
mod vectorstore;
pub use vectorstore::*;

// Implementations stay private; the module root is the public facade.
mod gcloud;
mod local;
mod service;
pub use service::SearchService;

mod capture_data_object;
pub use capture_data_object::make_capture_info_embed_input;

pub mod prelude;

#[cfg(test)]
pub(crate) mod test_support;
