use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "clear_token")]
#[argh(description = "Clear cached API token for the selected host and username")]
pub struct ClearTokenArgs {}

pub async fn run(host: &str, username: &str, _args: ClearTokenArgs) -> anyhow::Result<()> {
    token_cache::delete_token(host, username)?;

    println!("Cleared cached API token.");
    println!("- Host: {}", host);
    println!("- Username: {}", username);

    Ok(())
}
