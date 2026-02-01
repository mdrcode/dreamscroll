use anyhow::anyhow;
use argh::FromArgs;
use base64::Engine;

use crate::{api, database, facility, illumination::*};

use super::{auth_helper, html_view};

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
    let db = database::connect(config.db_config).await?;

    let user = auth_helper::authenticate_user_stdin(&db).await?;

    let capture_id = args.id;

    tracing::info!(
        "Comparing illuminators '{}' and '{}' for capture ID {}",
        args.illuminator_a,
        args.illuminator_b,
        capture_id
    );

    let mut fetch = api::fetch_captures(&db, &user.into(), Some(vec![capture_id]))
        .await
        .map_err(|_| anyhow!("Capture with ID {} not found in database.", capture_id))?;
    let capture_info = fetch.remove(0);
    tracing::info!("Fetched capture {} from db.", capture_info.id);

    // Helper to create illuminator from string
    fn make_illuminator(model: &str) -> anyhow::Result<Box<dyn Illuminator>> {
        match model {
            "grok" => Ok(Box::new(grok::GrokIlluminator::default())),
            "gemini" => Ok(Box::new(gemini::GeminiIlluminator::default())),
            "geministructured" => Ok(Box::new(
                geministructured::GeminiStructuredIlluminator::default(),
            )),
            "loremipsum" => Ok(Box::new(loremipsum::LoremIpsumIlluminator::default())),
            other => Err(anyhow!(
                "Unknown model '{}'. Supported: grok, gemini, geministructured, loremipsum.",
                other
            )),
        }
    }

    let illuminator_a = make_illuminator(&args.illuminator_a)?;
    let illuminator_b = make_illuminator(&args.illuminator_b)?;

    tracing::info!("Running illumination with '{}'...", args.illuminator_a);
    let result_a = illuminator_a.illuminate(&capture_info).await?;

    tracing::info!("Running illumination with '{}'...", args.illuminator_b);
    let result_b = illuminator_b.illuminate(&capture_info).await?;

    // Convert media files to base64 data URIs
    let media_data_uris: Vec<String> = capture_info
        .medias
        .iter()
        .filter_map(|media| {
            let source_path =
                std::path::PathBuf::from(format!("localdev/media/{}", media.storage_id));

            match std::fs::read(&source_path) {
                Ok(bytes) => {
                    let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    Some(format!("data:image/jpeg;base64,{}", base64_data))
                }
                Err(e) => {
                    tracing::warn!("Failed to read media file {}: {}", media.storage_id, e);
                    None
                }
            }
        })
        .collect();

    // Generate HTML comparison with embedded images
    let html_content = html_view::generate_html(
        &media_data_uris,
        capture_info.id,
        &[
            html_view::IlluminationPanel {
                name: &args.illuminator_a,
                illumination: &result_a,
            },
            html_view::IlluminationPanel {
                name: &args.illuminator_b,
                illumination: &result_b,
            },
        ],
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
