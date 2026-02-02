use argh::FromArgs;

use crate::model;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "backfill_search")]
#[argh(description = "Backfill search indexes for all captures of a user")]
pub struct BackfillSearchArgs {}

/// Represents a single capture in the export digest.
pub async fn run(state: CmdState, _args: BackfillSearchArgs) -> anyhow::Result<()> {
    let user = auth_helper::authenticate_user_stdin(&state.db).await?;
    let user_context = user.into();

    // Fetch captures without a corresponding search_index
    let ids_without_index = state
        .api_client
        .get_capture_ids_missing_search(&user_context)
        .await?;

    let capture_infos = state
        .api_client
        .fetch_captures(&user_context, Some(ids_without_index))
        .await?;
    let nci = capture_infos.len();
    println!("Found {} captures missing search indexes", nci);

    let mut count = 0;
    for capture_info in capture_infos {
        // An insult to search indexes everywhere, but it'll do for now
        let raw_content_for_search = format!(
            "{} {} {} {} {}",
            capture_info
                .illuminations
                .first()
                .map(|i| &i.summary)
                .unwrap_or(&"".to_string()),
            capture_info
                .illuminations
                .first()
                .map(|i| &i.details)
                .unwrap_or(&"".to_string()),
            capture_info
                .k_nodes
                .into_iter()
                .map(|e| e.name)
                .collect::<Vec<String>>()
                .join(" "),
            capture_info
                .x_queries
                .into_iter()
                .collect::<Vec<String>>()
                .join(" "),
            capture_info
                .social_medias
                .into_iter()
                .map(|s| s.display_name)
                .collect::<Vec<String>>()
                .join(" ")
        );

        model::search_index::ActiveModel::builder()
            .set_user_id(user_context.user_id())
            .set_capture_id(capture_info.id)
            .set_content(raw_content_for_search)
            .save(&state.db.conn)
            .await?;
        count += 1;
    }

    println!("Backfilled search indexes for {}/{} captures", count, nci);

    Ok(())
}
