/// Shared HTML generation utilities for illumination views

/// Represents a single illumination result panel
pub struct IlluminationPanel<'a> {
    pub name: &'a str,
    pub content: &'a str,
}

/// Generate an HTML view for illumination results.
/// Supports both single-panel (illuminate) and comparison (eval) modes.
pub fn generate_html(
    media_data_uris: &[String],
    capture_id: i32,
    panels: &[IlluminationPanel],
) -> String {
    let is_comparison = panels.len() > 1;

    let title = if is_comparison {
        format!("Illuminator Comparison - Capture {}", capture_id)
    } else {
        format!("Illumination - Capture {}", capture_id)
    };

    let header_title = if is_comparison {
        "Illuminator Comparison"
    } else {
        "Illumination Result"
    };

    let header_subtitle = if is_comparison {
        let names: Vec<&str> = panels.iter().map(|p| p.name).collect();
        format!(
            "Capture ID: {} | {} vs {}",
            capture_id,
            names.get(0).unwrap_or(&""),
            names.get(1).unwrap_or(&"")
        )
    } else {
        format!(
            "Capture ID: {} | Model: {}",
            capture_id,
            panels.first().map(|p| p.name).unwrap_or("")
        )
    };

    let media_previews = media_data_uris
        .iter()
        .map(|data_uri| {
            format!(
                r#"<img src="{}" alt="capture media" style="max-width: 100%; max-height: 400px; height: auto; margin-bottom: 10px; display: block; margin-left: auto; margin-right: auto;" />"#,
                data_uri
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content_section = if is_comparison {
        // Side-by-side comparison layout
        let panels_html: String = panels
            .iter()
            .map(|panel| {
                let html_content = markdown_to_html(panel.content);
                format!(
                    r#"            <div class="panel">
                <h2>{}</h2>
                <div class="result">
                    {}
                </div>
            </div>"#,
                    html_escape(panel.name),
                    html_content
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"<div class="comparison">
{}
        </div>"#,
            panels_html
        )
    } else {
        // Single panel layout
        let panel = panels
            .first()
            .map(|p| {
                let html_content = markdown_to_html(p.content);
                format!(
                    r#"<div class="content">
            <h2>{}</h2>
            <div class="result">
                {}
            </div>
        </div>"#,
                    html_escape(p.name),
                    html_content
                )
            })
            .unwrap_or_default();
        panel
    };

    let container_max_width = if is_comparison { "1800px" } else { "1200px" };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
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
            max-width: {container_max_width};
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
        .content {{
            padding: 30px;
        }}
        .content h2 {{
            color: #667eea;
            margin-bottom: 20px;
            padding-bottom: 10px;
            border-bottom: 3px solid #667eea;
            font-size: 1.5em;
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
            <h1>{header_title}</h1>
            <p>{header_subtitle}</p>
        </div>
        
        <div class="media-section">
            <h2>Capture Media</h2>
            {media_previews}
        </div>

        {content_section}
    </div>
</body>
</html>"#
    )
}

/// Convert markdown text to HTML with basic formatting support
pub fn markdown_to_html(markdown: &str) -> String {
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

    if in_list {
        html.push_str("</ul>\n");
    }

    html
}

/// Escape HTML special characters to prevent injection
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
