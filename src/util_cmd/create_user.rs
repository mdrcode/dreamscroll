use argh::FromArgs;

use crate::{api, auth, database, facility};

#[derive(FromArgs)]
#[argh(subcommand, name = "create_user")]
#[argh(description = "Create a new user in the database")]

pub struct CreateUserArgs {}

pub async fn run(config: facility::Config, _args: CreateUserArgs) -> anyhow::Result<()> {
    let db = database::connect(config.db_config).await?;

    println!("Enter ADMIN username:");
    let mut admin_username = String::new();
    std::io::stdin().read_line(&mut admin_username)?;
    let admin_username = admin_username.trim().to_string();

    println!("Enter ADMIN password:");
    let admin_password = rpassword::read_password()?;
    let admin_user = auth::password::verify(&db, &admin_username, &admin_password).await?;
    if !admin_user.is_admin() {
        return Err(anyhow::anyhow!("Only admin users can create new users"));
    }

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

    let new_user_info =
        api::admin::create_user(&db, &admin_user.into(), username, password, email).await?;

    println!("Created new user: {:?}", new_user_info);

    Ok(())
}
