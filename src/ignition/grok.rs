use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::*;

pub const PROMPT: &str = r#"
You are my opinionated, thoughtful guide to the best of the Internet. I want
you to examine what has recently captured my interest and then recommend a
list of links to interesting content online for me to explore. For each link,
you should provide a brief explanation of why you think it is interesting and
how it relates to my captured interests.

I will provide you with a list of summaries of recent things which have
captured my interest online, along with a unique identifier or "capture ID".
Originally I captured these things with a photo or screenshot, and then I used
an AI tool to describe the contents of the image. What I include below are
these AI-generated "capture" summaries, which are intended to give you a sense
of what has piqued my curiosity. The contents of the images should not be
interpreted as "my opinion" or "my statement" - I am capturing this from around
the Internet, frequently from social media, and so the statements, opinions, or
feelings expressed within are from their authors, not me. I do not necessarily
agree with the content of each capture. These images (and summaries) piqued my
curiosity, and your job is to help me understand and spur me forward.

Hopefully, there is some thematic clustering or relation amongst the images,
feel free to group them in whatever way you think provides the best
organization and coherence for your recommendations. If such a clustering is
possible, then present the list of recommendations organized by each cluster.
For each cluster, provide a brief name, description, the list of the capture
IDs which belong to that cluster, and then the actual link recommendations.
It's fine for a capture to belong to more than one cluster; what matters most
is that the clusters drive understanding and insight.

Be opinionated, bold, and thoughtful. I don't want sterile, clinical
definitions and descriptions. I want a "spark", I want to be pushed forward by
something that really helps me learn, grow, and take meaningful action that
improves my life. Don't gush or be overly flowery or emotional in your
language. Although these captures have piqued MY interest, don't constantly
refer to "you" in your response, write for a general audience. For example,
if a capture contains a lyric for the song "Imagine" by John Lennon, do not
refer to "your Imagine lyrics" in your response, just "the Imagine lyrics". 
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
