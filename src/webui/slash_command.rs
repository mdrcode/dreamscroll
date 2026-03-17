use crate::{api, auth};

struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
}

const COMMAND_SPECS: &[CommandSpec] = &[CommandSpec {
    name: "spark",
    usage: "/spark <capture_id> [capture_id ...]",
    description: "Queue a spark job for one or more capture IDs.",
}];

pub fn command_help_text() -> String {
    let mut lines = Vec::with_capacity(COMMAND_SPECS.len() + 1);
    lines.push("Supported slash commands:".to_string());

    for spec in COMMAND_SPECS {
        lines.push(format!("  {} - {}", spec.usage, spec.description));
    }

    lines.join("\n")
}

pub fn command_names_csv() -> String {
    COMMAND_SPECS
        .iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn process(
    raw_command: &str,
    context: &auth::Context,
    user_api: &api::UserApiClient,
) -> anyhow::Result<()> {
    tracing::info!("Processing slash command: {}", raw_command);

    if !raw_command.starts_with('/') {
        anyhow::bail!(
            "Invalid slash command: {}\n{}",
            raw_command,
            command_help_text()
        );
    }

    let (command, raw_args) = raw_command
        .trim_start_matches('/')
        .split_once(' ')
        .unwrap_or((raw_command.trim_start_matches('/'), ""));

    match command {
        "spark" => command_spark(raw_args, context, user_api).await?,
        _ => anyhow::bail!(
            "Unknown slash command: {} (known commands: {}).\n{}",
            command,
            command_names_csv(),
            command_help_text()
        ),
    }

    tracing::debug!("Successfully processed slash command: {}", raw_command);
    Ok(())
}

async fn command_spark(
    raw_args: &str,
    context: &auth::Context,
    user_api: &api::UserApiClient,
) -> anyhow::Result<()> {
    let mut actual_ids: Vec<i32> = Vec::new();

    for arg in raw_args.split_whitespace() {
        if let Ok(id) = arg.parse::<i32>() {
            actual_ids.push(id);
        } else {
            anyhow::bail!(
                "Invalid capture ID: {}. Expected integer IDs.\n{}",
                arg,
                command_help_text()
            );
        }
    }

    user_api.enqueue_spark(context, actual_ids).await?;
    Ok(())
}
