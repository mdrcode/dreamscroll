use anyhow::anyhow;
use argh::FromArgs;
use base64::Engine;

use crate::illumination::*;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "illuminate_id")]
#[argh(description = "Illuminate one or more captures without saving to the database.")]
pub struct IlluminateIdArgs {
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

pub async fn run(mut state: CmdState, args: IlluminateIdArgs) -> anyhow::Result<()> {
    if args.ids.is_empty() {
        return Err(anyhow!("At least one capture ID must be provided."));
    }

    let db = state.db_handle().await?;
    let user_api = state.user_api_client().await?;
    let stg = state.storage_provider().await?;
    let user = auth_helper::authenticate_user_stdin(&db).await?;

    let illuminator = make_illuminator(&state.config, stg);

    // Process each capture
    let capture_infos = user_api
        .get_captures(&user.clone().into(), args.ids.clone())
        .await?;

    if capture_infos.is_empty() {
        return Err(anyhow!(
            "No matching captures found for user_id {}.",
            user.user_id()
        ));
    }

    tracing::info!("Fetched {} capture(s) from db.", capture_infos.len());
    let mut capture_results = Vec::new();

    for c in capture_infos {
        tracing::info!("Starting illumination for capture ID {}", c.id);

        let result = illuminator.illuminate(&c).await?;

        // Convert media files to base64 data URIs
        let media_data_uris: Vec<String> = c
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

        capture_results.push((c.id, media_data_uris, result));
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
