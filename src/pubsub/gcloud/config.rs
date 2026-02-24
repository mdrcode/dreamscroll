use serde::Deserialize;

#[derive(Default, Deserialize)]
pub struct PubSubConfig {
    pub emulator_url_base: Option<String>, // e.g. "http://localhost:8085"
    // project_id comes from parent/main facility::config
    pub topic_id_new_capture: String,

    pub push_oidc_audience: Option<String>,
    pub push_oidc_service_account_email: Option<String>,
    pub push_oidc_jwks_url: Option<String>,
}
