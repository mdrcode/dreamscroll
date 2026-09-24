use crate::{api, auth};
use argh::FromArgs;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "create_user")]
#[argh(description = "Create a new user in the database")]
pub struct CreateUserArgs {}

pub async fn run(mut state: AdminCmdState, _args: CreateUserArgs) -> anyhow::Result<()> {
    println!("Enter ADMIN username:");
    let mut admin_username = String::new();
    std::io::stdin().read_line(&mut admin_username)?;
    let admin_username = admin_username.trim().to_string();

    println!("Enter ADMIN password:");
    let db = state.db_handle().await?;
    let admin_password = rpassword::read_password()?;
    let admin_user = auth::password::authenticate(&db, &admin_username, &admin_password).await?;
    let admin_context: auth::Context = admin_user.into();
    println!("Enter username for new user:");
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();

    println!("Enter email for new user:");
    let mut email = String::new();
    std::io::stdin().read_line(&mut email)?;
    let email = email.trim().to_string();

    println!("Enter password for new user:");
    let password = rpassword::read_password()?;

    if !admin_context.is_admin() {
        anyhow::bail!("Authenticated user is not an admin");
    }
    let new_user_info = api::create_user(&db, username, password, email).await?;

    println!("Created new user: {:?}", new_user_info);

    Ok(())
}
