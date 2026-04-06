use anyhow::Context;
use argh::FromArgs;

use crate::rest;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "change_password")]
#[argh(description = "Change account password via REST API")]
pub struct ChangePasswordArgs {}

pub async fn run(state: CmdState, _args: ChangePasswordArgs) -> anyhow::Result<()> {
    println!("Enter current password:");
    let current_password = rpassword::read_password()?;

    println!("Enter new password:");
    let new_password = rpassword::read_password()?;

    println!("Confirm new password:");
    let confirm_new_password = rpassword::read_password()?;

    if new_password != confirm_new_password {
        anyhow::bail!("New password and confirmation do not match");
    }

    state
        .rest_client
        .change_password(&current_password, &new_password)
        .await
        .context("failed to change password")?;

    println!("Password changed successfully.");

    if let Err(err) = local_token_cache::delete_token(&state.rest_host, &state.username) {
        eprintln!("Warning: unable to clear cached API token: {}", err);
    }

    let fresh_client =
        rest::client::Client::connect(&state.rest_host, &state.username, &new_password)
            .await
            .context("password changed, but failed to fetch a fresh token with the new password")?;

    if let Err(cache_err) = local_token_cache::set_token(
        &state.rest_host,
        &state.username,
        fresh_client.access_token(),
    ) {
        eprintln!("Warning: unable to cache fresh API token: {}", cache_err);
    } else {
        println!("Successfully refreshed cached API token.");
    }

    Ok(())
}
