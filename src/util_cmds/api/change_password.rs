use anyhow::Context;
use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "change_password")]
#[argh(description = "Change account password via REST API")]
pub struct ChangePasswordArgs {}

pub async fn run(state: ApiCmdState, _args: ChangePasswordArgs) -> anyhow::Result<()> {
    let rest_host = state.rest_host.clone();
    let rest_user = state
        .rest_user
        .as_deref()
        .context("REST username missing")?
        .to_string();

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
        .client
        .change_password(&current_password, &new_password)
        .await
        .context("failed to change password")?;

    println!("Password changed successfully.");

    if let Err(err) = token_cache::delete_token(&rest_host, &rest_user) {
        eprintln!("Warning: unable to clear cached API token: {}", err);
    }

    Ok(())
}
