use anyhow::anyhow;
use argh::FromArgs;
use base64::Engine;

use crate::illumination::*;

use super::*;

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

pub async fn run(state: CmdState, args: EvalArgs) -> anyhow::Result<()> {
    let user = auth_helper::authenticate_user_stdin(&state.db).await?;

    let capture_id = args.id;

    tracing::info!(
        "Comparing illuminators '{}' and '{}' for capture ID {}",
        args.illuminator_a,
        args.illuminator_b,
        capture_id
    );

    let mut fetch = state
        .user_api
        .get_captures(&user.into(), Some(vec![capture_id]))
        .await
        .map_err(|_| anyhow!("Capture with ID {} not found in database.", capture_id))?;
    let capture_info = fetch.remove(0);
    tracing::info!("Fetched capture {} from db.", capture_info.id);

    let illuminator_a = make_illuminator(&state.config, &args.illuminator_a, state.stg.clone());
    let illuminator_b = make_illuminator(&state.config, &args.illuminator_b, state.stg.clone());

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
                std::path::PathBuf::from(format!("localdev/media/{}", media.storage_uuid));

            match std::fs::read(&source_path) {
                Ok(bytes) => {
                    let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    Some(format!("data:image/jpeg;base64,{}", base64_data))
                }
                Err(e) => {
                    tracing::warn!("Failed to read media file {}: {}", media.storage_uuid, e);
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
