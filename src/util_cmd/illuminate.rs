use anyhow::anyhow;
use argh::FromArgs;
use base64::Engine;

use super::html_view;
use crate::{api, database, facility, illumination::*, storage};

#[derive(FromArgs)]
#[argh(subcommand, name = "illuminate")]
#[argh(description = "Illuminate one or more captures without saving to the database.")]
pub struct IlluminateArgs {
    #[argh(positional)]
    #[argh(description = "ID(s) of the capture(s) to illuminate")]
    ids: Vec<i32>,

    #[argh(
        option,
        long = "model",
        short = 'm',
        default = "String::from(\"geministructured\")",
        description = "illuminator model (grok, gemini, geministructured, loremipsum) [default: geministructured]"
    )]
    model: String,
}

pub async fn run(config: facility::Config, args: IlluminateArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let db = database::connect(config.db_config).await?;
    let _storage = storage::make(config.storage_config);

    // Process each capture
    let mut capture_results = Vec::new();

    for capture_id in &args.ids {
        tracing::info!("Starting illumination for capture ID {}", capture_id);

        let capture_info = api::fetch_capture_by_id(&db, *capture_id)
            .await
            .map_err(|_| anyhow!("Capture with ID {} not found in database.", capture_id))?;
        tracing::info!("Fetched capture {} from db.", capture_info.id);

        let illuminator: Box<dyn Illuminator> = match args.model.as_str() {
            "grok" => Box::new(grok::GrokIlluminator::default()),
            "gemini" => Box::new(gemini::GeminiIlluminator::default()),
            "geministructured" => {
                Box::new(geministructured::GeminiStructuredIlluminator::default())
            }
            "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator::default()),
            other => {
                return Err(anyhow!(
                    "Unknown model '{}'. Supported: grok, gemini, geministructured, loremipsum.",
                    other
                ));
            }
        };
        let result = illuminator.illuminate(capture_info.clone()).await?;

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

        capture_results.push((capture_info.id, media_data_uris, result));
    }

    // Generate HTML based on whether we have single or multiple captures
    let html_content = if args.ids.len() == 1 {
        let (capture_id, media_data_uris, illumination) = &capture_results[0];
        html_view::generate_html(
            media_data_uris,
            *capture_id,
            &[html_view::IlluminationPanel {
                name: &args.model,
                illumination,
            }],
        )
    } else {
        html_view::generate_multi_capture_html(&args.model, &capture_results)
    };

    // Write to temporary file
    let temp_dir = std::env::temp_dir();
    let filename = if args.ids.len() == 1 {
        format!("illumination_{}.html", args.ids[0])
    } else {
        format!(
            "illumination_multi_{}.html",
            args.ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join("_")
        )
    };
    let html_path = temp_dir.join(filename);
    std::fs::write(&html_path, html_content)?;

    println!(
        "\n✓ Illumination complete! View results at:\n  file://{}",
        html_path.display()
    );
    println!(
        "\nOn macOS, you can open it with:\n  open {}",
        html_path.display()
    );

    let _ = webbrowser::open(html_path.to_str().unwrap());

    Ok(())
}
