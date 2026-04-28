use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "hash_password")]
#[argh(description = "Hash a plaintext password exactly as stored in the database")]
pub struct HashPasswordArgs {}

pub async fn run(_state: CmdState, _args: HashPasswordArgs) -> anyhow::Result<()> {
    println!("Enter password:");
    let password = rpassword::read_password()?;

    let password_hash = crate::auth::password::hash(&password)?;

    // Print only the hash to make copy/paste into SQL UPDATE safe.
    println!("{}", password_hash);

    Ok(())
}
