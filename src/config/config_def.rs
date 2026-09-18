use std::str::FromStr;

use anyhow::{Context, bail};
use serde::{Deserialize, Deserializer};
use strum::{Display, EnumString};

#[derive(Debug, Display, EnumString, PartialEq)]
#[strum(serialize_all = "lowercase")]
pub enum Service {
    WebUI,
    API,
    Webhook,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Local,
    GCloud,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskQueueBackend {
    Local,
    GCloudTasks,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GeminiPayloadMethod {
    FileUri,
    Inline,
}

fn default_cookie_secure() -> bool {
    true
}

fn default_session_always_save() -> bool {
    true
}

fn default_gemini_payload_method() -> GeminiPayloadMethod {
    GeminiPayloadMethod::Inline
}

fn default_task_max_attempts() -> i32 {
    3
}

fn default_task_webhook_base_url() -> String {
    "http://localhost:8080".to_string()
}

fn default_jwt_user_expiration_secs() -> u64 {
    24 * 60 * 60
}

fn default_jwt_validation_leeway_secs() -> u64 {
    0
}

fn default_max_upload_bytes() -> usize {
    5 * 1024 * 1024
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub gcloud_project_id: String,
    pub gcloud_project_region: String,

    pub port: u16,

    #[serde(deserialize_with = "deserialize_comma_list")]
    pub services: Vec<Service>,

    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool, // true == only send cookies over HTTPS

    #[serde(default = "default_session_always_save")]
    pub session_always_save: bool, // true == refresh inactivity timeout on any valid request

    pub jwt_secret: Option<String>, // must be 32+ bytes for HS256 signing

    pub illuminator: String,
    pub gemini_api_key: Option<String>,
    pub gemini_model_id: Option<String>,
    #[serde(default = "default_gemini_payload_method")]
    pub gemini_payload_method: GeminiPayloadMethod,

    pub firestarter: String,
    pub xai_api_key: Option<String>,

    pub postgres_host_port: String, // e.g. "localhost:5432" or "db:5432"
    pub postgres_user: String,
    pub postgres_password: String,
    pub postgres_connection_params: Option<String>, // e.g. "sslmode=require"
    pub postgres_db: String,

    pub storage_backend: StorageBackend,
    pub storage_local_file_path: Option<String>,
    pub storage_local_url_prefix: Option<String>,
    pub storage_gcloud_emulator: Option<String>, // e.g. "http://localhost:4443"
    pub storage_gcloud_prod_endpoint: Option<String>,
    pub storage_gcloud_bucket_name: Option<String>,

    pub search_embed_collection_id: Option<String>,
    pub search_embed_vector_field: Option<String>,
    pub search_embed_vector_dims: Option<u32>,

    pub task_backend: TaskQueueBackend,
    #[serde(default = "default_task_max_attempts")]
    pub task_max_attempts: i32,
    #[serde(default = "default_task_webhook_base_url")]
    pub task_webhook_base_url: String,
    pub task_oidc_service_account_email: Option<String>,
    pub task_oidc_audience: Option<String>,

    // Currently, we assume that the queue_name below is used for *BOTH*
    //  - the Cloud Task resource: projects/{project_id}/locations/{region}/queues/$QUEUE_NAME
    //  - the app internal webhook URL: /_wh/cloudtask/$QUEUE_NAME
    pub task_queue_name_illumination: String,
    pub task_queue_name_search_index: String,
    pub task_queue_name_spark: String,

    #[serde(default = "default_jwt_user_expiration_secs")]
    pub jwt_user_expiration_secs: u64,
    #[serde(default = "default_jwt_validation_leeway_secs")]
    pub jwt_validation_leeway_secs: u64,

    #[serde(default = "default_max_upload_bytes")]
    pub max_upload_bytes: usize,
}

pub fn make() -> anyhow::Result<Config> {
    make_from_envy_iter(std::env::vars())
}

fn make_from_envy_iter<I>(vars: I) -> anyhow::Result<Config>
where
    I: IntoIterator<Item = (String, String)>,
{
    let cfg = envy::from_iter::<_, Config>(vars)
        .context("Failed to load config (missing required env vars or invalid values)")?;

    match cfg.storage_backend {
        StorageBackend::Local => {
            require_some(
                &cfg.storage_local_file_path,
                "STORAGE_BACKEND is local but no STORAGE_LOCAL_FILE_PATH",
            )?;
            require_some(
                &cfg.storage_local_url_prefix,
                "STORAGE_BACKEND is local but no STORAGE_LOCAL_URL_PREFIX",
            )?;
        }
        StorageBackend::GCloud => {
            require_some(
                &cfg.storage_gcloud_bucket_name,
                "STORAGE_BACKEND is gcloud but no STORAGE_GCLOUD_BUCKET_NAME",
            )?;
        }
    }

    match cfg.task_backend {
        TaskQueueBackend::Local => {
            if cfg.task_oidc_service_account_email.is_some() || cfg.task_oidc_audience.is_some() {
                tracing::info!(
                    "Ignoring webhook OIDC settings which are not used with the local backend"
                );
            }
        }
        TaskQueueBackend::GCloudTasks => {
            require_some(
                &cfg.task_oidc_service_account_email,
                "TASK_BACKEND is gcloudtasks but no TASK_OIDC_SERVICE_ACCOUNT_EMAIL",
            )?;
            require_some(
                &cfg.task_oidc_audience,
                "TASK_BACKEND is gcloudtasks but no TASK_OIDC_AUDIENCE",
            )?;
        }
    }

    Ok(cfg)
}

fn require_some<T>(value: &Option<T>, message: &str) -> anyhow::Result<()> {
    if value.is_none() {
        bail!("{}", message);
    }
    Ok(())
}

fn deserialize_comma_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    String::deserialize(deserializer)?
        .split(',')
        .map(|item| item.trim().parse::<T>().map_err(serde::de::Error::custom))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_services_with_whitespace() {
        #[derive(Deserialize)]
        struct Services {
            #[serde(deserialize_with = "deserialize_comma_list")]
            services: Vec<Service>,
        }

        let services: Services = serde_json::from_str(r#"{"services":"webui, api"}"#)
            .expect("service list should deserialize");

        assert_eq!(services.services, vec![Service::WebUI, Service::API]);
    }

    #[test]
    fn config_defaults_are_stable() {
        assert!(default_cookie_secure());
        assert!(default_session_always_save());
        assert_eq!(default_gemini_payload_method(), GeminiPayloadMethod::Inline);
        assert_eq!(default_task_max_attempts(), 3);
        assert_eq!(default_task_webhook_base_url(), "http://localhost:8080");
        assert_eq!(default_jwt_user_expiration_secs(), 86400);
        assert_eq!(default_jwt_validation_leeway_secs(), 0);
        assert_eq!(default_max_upload_bytes(), 5 * 1024 * 1024);
    }

    fn required_vars(storage_backend: &str) -> Vec<(String, String)> {
        vec![
            ("GCLOUD_PROJECT_ID".into(), "project".into()),
            ("GCLOUD_PROJECT_REGION".into(), "region".into()),
            ("PORT".into(), "8080".into()),
            ("SERVICES".into(), "webui,api".into()),
            ("ILLUMINATOR".into(), "loremipsum".into()),
            ("FIRESTARTER".into(), "grok".into()),
            ("POSTGRES_HOST_PORT".into(), "localhost:5432".into()),
            ("POSTGRES_USER".into(), "user".into()),
            ("POSTGRES_PASSWORD".into(), "password".into()),
            ("POSTGRES_DB".into(), "database".into()),
            ("STORAGE_BACKEND".into(), storage_backend.into()),
            ("TASK_BACKEND".into(), "local".into()),
            ("TASK_QUEUE_NAME_ILLUMINATION".into(), "illumination".into()),
            ("TASK_QUEUE_NAME_SEARCH_INDEX".into(), "search-index".into()),
            ("TASK_QUEUE_NAME_SPARK".into(), "spark".into()),
        ]
    }

    #[test]
    fn make_rejects_local_storage_without_local_paths() {
        let error =
            make_from_envy_iter(required_vars("local")).expect_err("config should be invalid");

        assert_eq!(
            error.to_string(),
            "STORAGE_BACKEND is local but no STORAGE_LOCAL_FILE_PATH"
        );
    }

    #[test]
    fn make_accepts_gcloud_storage_and_applies_defaults() {
        let mut vars = required_vars("gcloud");
        vars.push(("STORAGE_GCLOUD_BUCKET_NAME".into(), "bucket".into()));
        let config = make_from_envy_iter(vars).expect("config should be valid");

        assert_eq!(config.storage_backend, StorageBackend::GCloud);
        assert_eq!(config.task_backend, TaskQueueBackend::Local);
        assert_eq!(config.task_max_attempts, 3);
        assert_eq!(config.task_webhook_base_url, "http://localhost:8080");
        assert_eq!(config.jwt_user_expiration_secs, 86400);
        assert_eq!(config.jwt_validation_leeway_secs, 0);
        assert_eq!(config.max_upload_bytes, 5 * 1024 * 1024);
        assert_eq!(config.gemini_payload_method, GeminiPayloadMethod::Inline);
        assert!(config.cookie_secure);
        assert!(config.session_always_save);
    }

    #[test]
    fn local_tasks_ignore_oidc_settings() {
        let mut vars = required_vars("gcloud");
        vars.push(("STORAGE_GCLOUD_BUCKET_NAME".into(), "bucket".into()));
        vars.push((
            "TASK_WEBHOOK_BASE_URL".into(),
            "https://unused.example".into(),
        ));
        vars.push((
            "TASK_OIDC_SERVICE_ACCOUNT_EMAIL".into(),
            "unused@example.iam.gserviceaccount.com".into(),
        ));
        vars.push(("TASK_OIDC_AUDIENCE".into(), "https://unused.example".into()));
        make_from_envy_iter(vars).expect("local config should ignore OIDC settings");
    }

    #[test]
    fn cloud_tasks_require_complete_oidc_settings() {
        let mut vars = required_vars("gcloud");
        vars.push(("STORAGE_GCLOUD_BUCKET_NAME".into(), "bucket".into()));
        vars.iter_mut()
            .find(|(key, _)| key == "TASK_BACKEND")
            .expect("required vars should include TASK_BACKEND")
            .1 = "gcloudtasks".into();

        let error = make_from_envy_iter(vars).expect_err("Cloud Tasks config should require OIDC");
        assert_eq!(
            error.to_string(),
            "TASK_BACKEND is gcloudtasks but no TASK_OIDC_SERVICE_ACCOUNT_EMAIL"
        );
    }
}
