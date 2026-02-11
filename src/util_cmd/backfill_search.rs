use argh::FromArgs;

use crate::{api, model};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "backfill_search")]
#[argh(description = "Backfill search indexes for all captures of a user")]
pub struct BackfillSearchArgs {}

/// Represents a single capture in the export digest.
pub async fn run(state: CmdState, _args: BackfillSearchArgs) -> anyhow::Result<()> {
    let user = auth_helper::authenticate_user_stdin(&state.db).await?;
    let user_context = user.into();

    // Fetch illuminations without a corresponding search index
    let iids_without_search = state
        .api_client
        .get_illumination_ids_need_search(&user_context)
        .await?;

    let iinfos = state
        .api_client
        .get_illuminations(&user_context, iids_without_search)
        .await?;
    let nci = iinfos.len();
    println!("Found {} illuminations missing search indexes", nci);

    let mut count = 0;
    for i in iinfos {
        // An insult to search indexes everywhere, but it'll do for now

        let raw_content_for_search = format_for_search(&i);

        model::search_index::ActiveModel::builder()
            .set_user_id(user_context.user_id())
            .set_capture_id(i.capture_id)
            .set_illumination_id(i.id)
            .set_content(raw_content_for_search)
            .save(&state.db.conn)
            .await?;
        count += 1;
    }

    println!("Backfilled search indexes for {}/{} captures", count, nci);

    Ok(())
}

fn format_for_search(i: &api::IlluminationInfo) -> String {
    // lol, a naive approach for now
    format!(
        "{} {} {} {} {}",
        i.summary,
        i.details,
        i.k_nodes
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<String>>()
            .join(" "),
        i.x_queries.join(" "),
        i.social_medias
            .iter()
            .map(|sm| format!("{} {}", sm.display_name, sm.handle))
            .collect::<Vec<String>>()
            .join(" ")
    )
}
