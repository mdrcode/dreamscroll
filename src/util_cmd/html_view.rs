/// Shared HTML generation utilities for illumination views
use crate::illumination::{Entity, Illumination};

/// Represents a single illumination result panel for comparison views
pub struct IlluminationPanel<'a> {
    pub name: &'a str,
    pub illumination: &'a Illumination,
}

/// Generate an HTML view for a single capture with one or two illumination panels.
/// Used by both single illumination (one panel) and eval comparison (two panels).
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

    let media_previews = generate_media_previews(media_data_uris);

    let content_section = if is_comparison {
        // Side-by-side comparison layout
        let panels_html: String = panels
            .iter()
            .map(|panel| {
                format!(
                    r#"            <div class="panel">
                <h2>{}</h2>
                {}
            </div>"#,
                    html_escape(panel.name),
                    render_illumination(panel.illumination)
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
        panels
            .first()
            .map(|p| {
                format!(
                    r#"<div class="content">
            <h2>{}</h2>
            {}
        </div>"#,
                    html_escape(p.name),
                    render_illumination(p.illumination)
                )
            })
            .unwrap_or_default()
    };

    let container_max_width = if is_comparison { "1800px" } else { "1200px" };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    {CSS_STYLES}
</head>
<body>
    <div class="container" style="max-width: {container_max_width};">
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

/// Generate an HTML view for multiple captures stacked vertically.
/// Each capture shows its media and illumination result.
pub fn generate_multi_capture_html(
    model_name: &str,
    capture_results: &[(i32, Vec<String>, Illumination)],
) -> String {
    let capture_sections: String = capture_results
        .iter()
        .map(|(capture_id, media_data_uris, illumination)| {
            let media_previews = generate_media_previews(media_data_uris);

            format!(
                r#"        <div class="capture-section">
            <div class="capture-header">
                <h2>Capture ID: {}</h2>
            </div>
            
            <div class="media-section">
                <h3>Media</h3>
                {}
            </div>

            <div class="content">
                <h3>{}</h3>
                {}
            </div>
        </div>"#,
                capture_id,
                media_previews,
                html_escape(model_name),
                render_illumination(illumination)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let capture_ids: Vec<String> = capture_results
        .iter()
        .map(|(id, _, _)| id.to_string())
        .collect();
    let title = format!(
        "Multi-Capture Illumination - {} Captures",
        capture_results.len()
    );
    let subtitle = format!(
        "Capture IDs: {} | Model: {}",
        capture_ids.join(", "),
        model_name
    );

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{}</title>
    {}
</head>
<body>
    <div class="container">
        <div class="main-header">
            <h1>Multi-Capture Illumination</h1>
            <p>{}</p>
        </div>

{}
    </div>
</body>
</html>"#,
        title, CSS_STYLES, subtitle, capture_sections
    )
}

/// Render a structured Illumination into HTML content
fn render_illumination(illumination: &Illumination) -> String {
    let mut html = String::new();

    // Summary section
    html.push_str(&format!(
        r#"<div class="summary">
            <h4>Summary</h4>
            <p>{}</p>
        </div>"#,
        html_escape(&illumination.summary)
    ));

    // Details section
    html.push_str(&format!(
        r#"
        <div class="details">
            <h4>Details</h4>
            {}</div>"#,
        render_paragraphs(&illumination.details)
    ));

    // Suggested searches section
    if !illumination.suggested_searches.is_empty() {
        html.push_str(
            r#"
        <div class="suggested-searches">
            <h4>Suggested Searches</h4>
            <ul>"#,
        );
        for search in &illumination.suggested_searches {
            html.push_str(&format!(
                r#"
                <li><a href="https://www.google.com/search?q={}" target="_blank">{}</a></li>"#,
                urlencoded(search),
                html_escape(search)
            ));
        }
        html.push_str(
            r#"
            </ul>
        </div>"#,
        );
    }

    // Entities section
    if !illumination.entities.is_empty() {
        html.push_str(
            r#"
        <div class="entities">
            <h4>Entities</h4>
            <div class="entity-list">"#,
        );
        for entity in &illumination.entities {
            html.push_str(&render_entity(entity));
        }
        html.push_str(
            r#"
            </div>
        </div>"#,
        );
    }

    html
}

/// Render a single entity as an HTML card
fn render_entity(entity: &Entity) -> String {
    format!(
        r#"
                <div class="entity-card">
                    <div class="entity-header">
                        <span class="entity-name">{}</span>
                        <span class="entity-type">{}</span>
                    </div>
                    <p class="entity-description">{}</p>
                </div>"#,
        html_escape(&entity.name),
        entity.entity_type,
        html_escape(&entity.description)
    )
}

/// Render text with paragraph breaks into HTML paragraphs
fn render_paragraphs(text: &str) -> String {
    text.split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", html_escape(p.trim())))
        .collect::<Vec<_>>()
        .join("\n            ")
}

/// Generate media preview images HTML
fn generate_media_previews(media_data_uris: &[String]) -> String {
    media_data_uris
        .iter()
        .map(|data_uri| {
            format!(
                r#"<img src="{}" alt="capture media" style="max-width: 100%; max-height: 400px; height: auto; margin-bottom: 10px; display: block; margin-left: auto; margin-right: auto;" />"#,
                data_uri
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Escape HTML special characters to prevent injection
pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// URL-encode a string for use in query parameters
fn urlencoded(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' ' => "+".to_string(),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' => {
                c.to_string()
            }
            c => format!("%{:02X}", c as u8),
        })
        .collect()
}

const CSS_STYLES: &str = r#"<style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            line-height: 1.6;
            color: #333;
            background: #f5f5f5;
            padding: 20px;
        }
        .container {
            max-width: 1200px;
            margin: 0 auto;
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
            overflow: hidden;
        }
        .header, .main-header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            text-align: center;
        }
        .main-header {
            border-radius: 8px;
            margin-bottom: 20px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
        }
        .header h1, .main-header h1 {
            font-size: 2em;
            margin-bottom: 10px;
        }
        .header p, .main-header p {
            opacity: 0.9;
        }
        .media-section {
            padding: 30px;
            background: #fafafa;
            border-bottom: 2px solid #eee;
        }
        .media-section h2, .media-section h3 {
            margin-bottom: 20px;
            color: #555;
        }
        .content {
            padding: 30px;
        }
        .content h2, .content h3 {
            color: #667eea;
            margin-bottom: 20px;
            padding-bottom: 10px;
            border-bottom: 3px solid #667eea;
            font-size: 1.5em;
        }
        .comparison {
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 0;
        }
        .panel {
            padding: 30px;
            min-height: 400px;
        }
        .panel:first-child {
            border-right: 2px solid #eee;
        }
        .panel h2 {
            color: #667eea;
            margin-bottom: 20px;
            padding-bottom: 10px;
            border-bottom: 3px solid #667eea;
            font-size: 1.5em;
        }
        .panel:last-child h2 {
            color: #764ba2;
            border-bottom-color: #764ba2;
        }
        .capture-section {
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 8px rgba(0,0,0,0.1);
            margin-bottom: 30px;
            overflow: hidden;
        }
        .capture-header {
            background: #667eea;
            color: white;
            padding: 20px 30px;
        }
        .capture-header h2 {
            font-size: 1.5em;
        }
        /* Illumination structure styles */
        .summary, .details, .suggested-searches, .entities {
            margin-bottom: 24px;
        }
        .summary h4, .details h4, .suggested-searches h4, .entities h4 {
            color: #555;
            font-size: 0.9em;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            margin-bottom: 12px;
            border-bottom: 1px solid #eee;
            padding-bottom: 6px;
        }
        .summary p {
            font-size: 1.1em;
            font-weight: 500;
            color: #333;
            background: #f8f9fa;
            padding: 16px;
            border-radius: 6px;
            border-left: 4px solid #667eea;
        }
        .details p {
            margin-bottom: 1em;
            text-align: justify;
        }
        .suggested-searches ul {
            list-style: none;
            display: flex;
            flex-wrap: wrap;
            gap: 8px;
        }
        .suggested-searches li a {
            display: inline-block;
            background: #e8eaf6;
            color: #3949ab;
            padding: 6px 12px;
            border-radius: 16px;
            text-decoration: none;
            font-size: 0.9em;
            transition: background 0.2s;
        }
        .suggested-searches li a:hover {
            background: #c5cae9;
        }
        .entity-list {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
            gap: 16px;
        }
        .entity-card {
            background: #fafafa;
            border: 1px solid #e0e0e0;
            border-radius: 8px;
            padding: 16px;
        }
        .entity-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 8px;
        }
        .entity-name {
            font-weight: 600;
            color: #333;
        }
        .entity-type {
            font-size: 0.75em;
            background: #667eea;
            color: white;
            padding: 2px 8px;
            border-radius: 10px;
            text-transform: lowercase;
        }
        .entity-description {
            font-size: 0.9em;
            color: #666;
        }
        @media (max-width: 1024px) {
            .comparison {
                grid-template-columns: 1fr;
            }
            .panel:first-child {
                border-right: none;
                border-bottom: 2px solid #eee;
            }
            .entity-list {
                grid-template-columns: 1fr;
            }
        }
    </style>"#;
