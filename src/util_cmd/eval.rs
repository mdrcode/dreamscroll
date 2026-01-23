use anyhow::anyhow;
use argh::FromArgs;
use base64::Engine;

use crate::{api, database, facility, illumination::*};

#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
#[argh(description = "Compare two illuminators side by side on a capture.")]
pub struct EvalArgs {
    #[argh(positional)]
    #[argh(description = "first illuminator model name")]
    illuminator_a: String,

    #[argh(positional)]
    #[argh(description = "second illuminator model name")]
    illuminator_b: String,

    #[argh(positional)]
    #[argh(description = "ID of the capture to illuminate")]
    id: i32,
}

pub async fn run(config: facility::Config, args: EvalArgs) -> anyhow::Result<()> {
    let capture_id = args.id;

    tracing::info!(
        "Comparing illuminators '{}' and '{}' for capture ID {}",
        args.illuminator_a,
        args.illuminator_b,
        capture_id
    );

    let db = database::connect(config.db_config).await?;

    let capture_info = api::fetch_capture_by_id(&db, capture_id)
        .await
        .map_err(|_| anyhow!("Capture with ID {} not found in database.", capture_id))?;
    tracing::info!("Fetched capture {} from db.", capture_info.id);

    // Helper to create illuminator from string
    fn make_illuminator(model: &str) -> anyhow::Result<Box<dyn Illuminator>> {
        match model {
            "grok" => Ok(Box::new(GrokIlluminator::default())),
            "gemini" => Ok(Box::new(GeminiIlluminator::default())),
            "geministructured" => Ok(Box::new(GeminiStructuredIlluminator::default())),
            "loremipsum" => Ok(Box::new(LoremIpsumIlluminator::default())),
            other => Err(anyhow!(
                "Unknown model '{}'. Supported: grok, gemini, geministructured, loremipsum.",
                other
            )),
        }
    }

    let illuminator_a = make_illuminator(&args.illuminator_a)?;
    let illuminator_b = make_illuminator(&args.illuminator_b)?;

    tracing::info!("Running illumination with '{}'...", args.illuminator_a);
    let result_a = illuminator_a.illuminate(capture_info.clone()).await?;

    tracing::info!("Running illumination with '{}'...", args.illuminator_b);
    let result_b = illuminator_b.illuminate(capture_info.clone()).await?;

    // Convert media files to base64 data URIs
    let media_data_uris: Vec<String> = capture_info
        .medias
        .iter()
        .filter_map(|media| {
            let source_path =
                std::path::PathBuf::from(format!("localdev/media/{}", media.filename));

            match std::fs::read(&source_path) {
                Ok(bytes) => {
                    let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    Some(format!("data:image/jpeg;base64,{}", base64_data))
                }
                Err(e) => {
                    tracing::warn!("Failed to read media file {}: {}", media.filename, e);
                    None
                }
            }
        })
        .collect();

    // Generate HTML comparison with embedded images
    let html_content = generate_comparison_html(
        &media_data_uris,
        capture_info.id,
        &args.illuminator_a,
        &result_a,
        &args.illuminator_b,
        &result_b,
    );

    // Write to temporary file
    let temp_dir = std::env::temp_dir();
    let html_path = temp_dir.join(format!("illuminator_comparison_{}.html", capture_id));
    std::fs::write(&html_path, html_content)?;

    println!(
        "\n✓ Comparison complete! View results at:\n  file://{}",
        html_path.display()
    );
    println!(
        "\nOn macOS, you can open it with:\n  open {}",
        html_path.display()
    );

    let _ = webbrowser::open(html_path.to_str().unwrap());

    Ok(())
}

fn generate_comparison_html(
    media_data_uris: &[String],
    capture_id: i32,
    name_a: &str,
    result_a: &str,
    name_b: &str,
    result_b: &str,
) -> String {
    let html_result_a = markdown_to_html(result_a);
    let html_result_b = markdown_to_html(result_b);

    let media_previews = media_data_uris
        .iter()
        .map(|data_uri| {
            // Use base64-encoded data URI for embedded images
            format!(
                r#"<img src="{}" alt="capture media" style="max-width: 100%; max-height: 400px; height: auto; margin-bottom: 10px; display: block; margin-left: auto; margin-right: auto;" />"#,
                data_uri
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Illuminator Comparison - Capture {}</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            line-height: 1.6;
            color: #333;
            background: #f5f5f5;
            padding: 20px;
        }}
        .container {{
            max-width: 1800px;
            margin: 0 auto;
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
            overflow: hidden;
        }}
        .header {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            text-align: center;
        }}
        .header h1 {{
            font-size: 2em;
            margin-bottom: 10px;
        }}
        .header p {{
            opacity: 0.9;
        }}
        .media-section {{
            padding: 30px;
            background: #fafafa;
            border-bottom: 2px solid #eee;
        }}
        .media-section h2 {{
            margin-bottom: 20px;
            color: #555;
        }}
        .comparison {{
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 0;
        }}
        .panel {{
            padding: 30px;
            min-height: 400px;
        }}
        .panel:first-child {{
            border-right: 2px solid #eee;
        }}
        .panel h2 {{
            color: #667eea;
            margin-bottom: 20px;
            padding-bottom: 10px;
            border-bottom: 3px solid #667eea;
            font-size: 1.5em;
        }}
        .panel:last-child h2 {{
            color: #764ba2;
            border-bottom-color: #764ba2;
        }}
        .result {{
            background: #fafafa;
            padding: 20px;
            border-radius: 4px;
            white-space: pre-wrap;
            word-wrap: break-word;
        }}
        .result p {{
            margin-bottom: 1em;
        }}
        .result h1, .result h2, .result h3 {{
            margin-top: 1.5em;
            margin-bottom: 0.5em;
        }}
        .result ul, .result ol {{
            margin-left: 2em;
            margin-bottom: 1em;
        }}
        .result code {{
            background: #e0e0e0;
            padding: 2px 6px;
            border-radius: 3px;
            font-family: 'Courier New', monospace;
        }}
        .result pre {{
            background: #2d2d2d;
            color: #f8f8f2;
            padding: 15px;
            border-radius: 4px;
            overflow-x: auto;
            margin-bottom: 1em;
        }}
        .result pre code {{
            background: none;
            padding: 0;
        }}
        @media (max-width: 1024px) {{
            .comparison {{
                grid-template-columns: 1fr;
            }}
            .panel:first-child {{
                border-right: none;
                border-bottom: 2px solid #eee;
            }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>Illuminator Comparison</h1>
            <p>Capture ID: {} | {} vs {}</p>
        </div>
        
        <div class="media-section">
            <h2>Capture Media</h2>
            {}
        </div>

        <div class="comparison">
            <div class="panel">
                <h2>{}</h2>
                <div class="result">
                    {}
                </div>
            </div>
            <div class="panel">
                <h2>{}</h2>
                <div class="result">
                    {}
                </div>
            </div>
        </div>
    </div>
</body>
</html>"#,
        capture_id,
        capture_id,
        name_a,
        name_b,
        media_previews,
        name_a,
        html_result_a,
        name_b,
        html_result_b
    )
}

/// Simple markdown to HTML converter (basic implementation)
fn markdown_to_html(markdown: &str) -> String {
    // This is a very basic converter - in production you'd want to use a proper markdown library
    let mut html = String::new();
    let mut in_code_block = false;
    let mut in_list = false;
    let mut code_block_content = String::new();

    for line in markdown.lines() {
        if line.starts_with("```") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            if in_code_block {
                html.push_str(&format!(
                    "<pre><code>{}</code></pre>\n",
                    html_escape(&code_block_content)
                ));
                code_block_content.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_block_content.push_str(line);
            code_block_content.push('\n');
            continue;
        }

        if line.trim().is_empty() {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str("<br/>\n");
        } else if line.starts_with("### ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h3>{}</h3>\n", html_escape(&line[4..])));
        } else if line.starts_with("## ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h2>{}</h2>\n", html_escape(&line[3..])));
        } else if line.starts_with("# ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h1>{}</h1>\n", html_escape(&line[2..])));
        } else if line.starts_with("- ") || line.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            html.push_str(&format!("<li>{}</li>\n", html_escape(&line[2..])));
        } else if line.ends_with(":") && !line.contains(' ')
            || line == "Suggested searches:"
            || line == "Entities:"
        {
            // Treat section headers as h3
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h3>{}</h3>\n", html_escape(line)));
        } else {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<p>{}</p>\n", html_escape(line)));
        }
    }

    // Close any open list at the end
    if in_list {
        html.push_str("</ul>\n");
    }

    html
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
