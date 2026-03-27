use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use reqwest;
use serde::{Deserialize, Serialize};

use crate::api;

pub struct Client {
    reqwest_client: reqwest::Client,
    base_url: String,
    access_token: String,
}

#[derive(Debug, Serialize)]
struct TokenRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

impl Client {
    pub async fn connect(host: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let base_url = normalize_base_url(host);
        let reqwest_client = reqwest::Client::new();

        let token_res = reqwest_client
            .post(format!("{base_url}/token"))
            .json(&TokenRequest { username, password })
            .send()
            .await
            .context("failed to call token endpoint")?;

        let status = token_res.status();
        if !status.is_success() {
            let body = token_res
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(anyhow!(
                "token endpoint failed with status {}: {}",
                status,
                body
            ));
        }

        let token = token_res
            .json::<TokenResponse>()
            .await
            .context("failed to parse token response")?;

        Ok(Self {
            reqwest_client,
            base_url,
            access_token: token.access_token,
        })
    }

    pub fn connect_with_token(host: &str, access_token: String) -> anyhow::Result<Self> {
        let base_url = normalize_base_url(host);
        let reqwest_client = reqwest::Client::new();

        Ok(Self {
            reqwest_client,
            base_url,
            access_token,
        })
    }

    pub async fn validate_auth(&self) -> anyhow::Result<()> {
        let response = self
            .reqwest_client
            .get(format!("{}/dummy", self.base_url))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .context("failed to call dummy endpoint")?;

        let status = response.status();
        if status.is_success() {
            Ok(())
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = read_error_body(response).await;
            Err(anyhow!("unauthorized (401): {}", body))
        } else {
            let body = read_error_body(response).await;
            Err(anyhow!("request failed with status {}: {}", status, body))
        }
    }

    pub async fn get_timeline(&self, limit: Option<u64>) -> anyhow::Result<Vec<api::CaptureInfo>> {
        let url = format!("{}/timeline", self.base_url);
        let mut request = self.reqwest_client.get(url).bearer_auth(&self.access_token);

        if let Some(limit) = limit {
            request = request.query(&[("limit", limit)]);
        }

        let response = request
            .send()
            .await
            .context("failed to call timeline endpoint")?;

        Self::parse_json_response(response).await
    }

    pub async fn get_captures(&self, ids: Option<&[i32]>) -> anyhow::Result<Vec<api::CaptureInfo>> {
        let url = format!("{}/captures", self.base_url);
        let mut request = self.reqwest_client.get(url).bearer_auth(&self.access_token);

        if let Some(values) = ids {
            request = request.query(&values.iter().map(|id| ("id", *id)).collect::<Vec<_>>());
        }

        let response = request
            .send()
            .await
            .context("failed to call captures endpoint")?;

        Self::parse_json_response(response).await
    }

    pub async fn import_capture(
        &self,
        media_bytes: bytes::Bytes,
        created_at: DateTime<Utc>,
    ) -> anyhow::Result<api::CaptureInfo> {
        let media_part = reqwest::multipart::Part::bytes(media_bytes.to_vec())
            .file_name("capture")
            .mime_str("application/octet-stream")
            .context("failed to create multipart media part")?;

        let form = reqwest::multipart::Form::new()
            .text("created_at", created_at.to_rfc3339())
            .part("image", media_part);

        let response = self
            .reqwest_client
            .post(format!("{}/captures/import", self.base_url))
            .bearer_auth(&self.access_token)
            .multipart(form)
            .send()
            .await
            .context("failed to call import capture endpoint")?;

        Self::parse_json_response(response).await
    }

    pub async fn delete_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        let response = self
            .reqwest_client
            .delete(format!("{}/captures/{}", self.base_url, capture_id))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .context("failed to call delete capture endpoint")?;

        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = read_error_body(response).await;
            Err(anyhow!("unauthorized (401): {}", body))
        } else {
            let body = read_error_body(response).await;
            Err(anyhow!("request failed with status {}: {}", status, body))
        }
    }

    pub async fn archive_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        let response = self
            .reqwest_client
            .post(format!("{}/captures/{}/archive", self.base_url, capture_id))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .context("failed to call archive capture endpoint")?;

        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = read_error_body(response).await;
            Err(anyhow!("unauthorized (401): {}", body))
        } else {
            let body = read_error_body(response).await;
            Err(anyhow!("request failed with status {}: {}", status, body))
        }
    }

    pub async fn unarchive_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        let response = self
            .reqwest_client
            .post(format!(
                "{}/captures/{}/unarchive",
                self.base_url, capture_id
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .context("failed to call unarchive capture endpoint")?;

        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = read_error_body(response).await;
            Err(anyhow!("unauthorized (401): {}", body))
        } else {
            let body = read_error_body(response).await;
            Err(anyhow!("request failed with status {}: {}", status, body))
        }
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }
}

fn normalize_base_url(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        format!("{trimmed}/api")
    } else if trimmed.eq_ignore_ascii_case("localhost")
        || trimmed.starts_with("localhost:")
        || trimmed.starts_with("127.0.0.1")
        || trimmed.starts_with("[::1]")
    {
        format!("http://{trimmed}/api")
    } else {
        format!("https://{trimmed}/api")
    }
}

async fn read_error_body(response: reqwest::Response) -> String {
    response
        .text()
        .await
        .unwrap_or_else(|_| "<no body>".to_string())
}

impl Client {
    async fn parse_json_response<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
    ) -> anyhow::Result<T> {
        let status = response.status();

        if status.is_success() {
            response
                .json::<T>()
                .await
                .context("failed to parse JSON response")
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            let body = read_error_body(response).await;
            Err(anyhow!("unauthorized (401): {}", body))
        } else {
            let body = read_error_body(response).await;
            Err(anyhow!("request failed with status {}: {}", status, body))
        }
    }
}
