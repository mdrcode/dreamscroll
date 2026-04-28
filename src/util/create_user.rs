use argh::FromArgs;

use crate::{api, auth, task};

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "create_user")]
#[argh(description = "Create a new user in the database")]

pub struct CreateUserArgs {}

pub async fn run(state: CmdState, _args: CreateUserArgs) -> anyhow::Result<()> {
    println!("Enter ADMIN username:");
    let mut admin_username = String::new();
    std::io::stdin().read_line(&mut admin_username)?;
    let admin_username = admin_username.trim().to_string();

    println!("Enter ADMIN password:");
    let db = state.db_handle();
    let admin_password = rpassword::read_password()?;
    let admin_user = auth::password::authenticate(&db, &admin_username, &admin_password).await?;
    let admin_context: auth::Context = admin_user.into();
    let admin_client = api::AdminApiClient::new(
        db.clone(),
        state.service_api.clone(),
        task::Beacon::default(),
    );

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

    let new_user_info = admin_client
        .create_user(&admin_context, username, password, email)
        .await?;

    println!("Created new user: {:?}", new_user_info);

    Ok(())
}
