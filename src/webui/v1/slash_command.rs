use std::sync::Arc;

use crate::auth;

use super::*;

pub async fn process(
    raw_command: &str,
    context: &auth::Context,
    state: Arc<WebState>,
) -> anyhow::Result<()> {
    tracing::info!("Processing slash command: {}", raw_command);

    if !raw_command.starts_with("/") {
        anyhow::bail!("Invalid slash command: {}", raw_command);
    }

    let (command, raw_args) = raw_command
        .trim_start_matches("/")
        .split_once(" ")
        .unwrap_or((raw_command, ""));

    match command {
        "spark" => command_spark(raw_args, context, state).await?,
        _ => anyhow::bail!("Unknown slash command: {}", command),
    }

    tracing::debug!("Successfully processed slash command: {}", raw_command);
    Ok(())
}

async fn command_spark(
    raw_args: &str,
    context: &auth::Context,
    state: Arc<WebState>,
) -> anyhow::Result<()> {
    let mut actual_ids: Vec<i32> = Vec::new();

    for arg in raw_args.split_whitespace() {
        if let Ok(id) = arg.parse::<i32>() {
            actual_ids.push(id);
        } else {
            anyhow::bail!("Invalid ID: {}", arg);
        }
    }

    state
        .user_api
        .enqueue_spark(context, actual_ids.clone())
        .await?;

    tracing::info!("Enqueued spark for IDs: {:?}", actual_ids);

    Ok(())
}
