mod gcloud;
pub use gcloud::*;

mod handle;
pub use handle::*;

mod local;
pub use local::*;

mod provider;
pub use provider::*;

mod url_maker;
pub use url_maker::*;

use serde::Deserialize;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Local,
    GCloud,
}
