use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::*;

pub const PROMPT: &str = r#"
You are my opinionated, thoughtful guide to the best of the Internet. I want
you to examine what has recently captured my interest and then recommend a
list of links to interesting online content for me to explore. For each link,
you should provide a brief explanation of why you think it is interesting and
how it relates to my recent interests.

I will provide you with a list of summaries of recent things which have
captured my interest online. Originally I captured these things with photos or
screenshots, and then I used an AI tool to describe the contents of the image.
What I include below are these AI-generated summaries, which are intended to
give you a sense of what has piqued my curiosity. Hopefully, there is some
thematic clustering or relation amongst the images, feel free to group them in
whatever way you think provides the best organization and coherence for your
recommendations.

Be opinionated, bold, and thoughtful. I don't want sterile, clinical
definitions and descriptions. I want a "spark", I want to be pushed forward by
something that really helps me learn, grow, and take meaningful action that
improves my life.
"#;

pub struct GrokFirestarter {
    api_key: String,
}

impl GrokFirestarter {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl Firestarter for GrokFirestarter {
    fn name(&self) -> &str {
        "GrokFirestarter"
    }

    async fn spark(&self, captures: Vec<crate::api::CaptureInfo>) -> anyhow::Result<String> {
        let captures_section = captures
            .iter()
            .enumerate()
            .map(|(idx, capture)| {
                let illumination = capture.illuminations.first();
                let summary = illumination
                    .map(|it| it.summary.as_str())
                    .unwrap_or("(no summary available)");
                let details = illumination
                    .map(|it| it.details.as_str())
                    .unwrap_or("(no details available)");

                format!(
                    "{}. Capture ID {}\nSummary: {}\nDetails: {}",
                    idx + 1,
                    capture.id,
                    summary,
                    details,
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let user_prompt = format!(
            "{}\n\nRecent captures and summaries:\n\n{}",
            PROMPT, captures_section
        );

        let request_body = ChatCompletionRequest {
            model: "grok-4-1-fast-reasoning".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_prompt,
            }],
        };

        let client = Client::new();
        let response = client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("XAI API error: {}", error_text);
        }

        let parsed: ChatCompletionResponse = response.json().await?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.trim())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            anyhow::bail!("XAI API returned an empty response body");
        }

        Ok(content)
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}
