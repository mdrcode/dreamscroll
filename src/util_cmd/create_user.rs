use argh::FromArgs;
use sea_orm::{ActiveModelTrait, Set};

use crate::{auth, database, facility, model::user};

#[derive(FromArgs)]
#[argh(subcommand, name = "create_user")]
#[argh(description = "Create a new user in the database")]

pub struct CreateUserArgs {
    #[argh(positional)]
    #[argh(description = "username for the new user")]
    username: String,
}

pub async fn run(config: facility::Config, args: CreateUserArgs) -> anyhow::Result<()> {
    let db = database::connect(config.db_config).await?;

    // first check if user already exists
    let existing_user = user::Entity::find_by_username(args.username.clone())
        .one(&db.conn)
        .await?;
    if existing_user.is_some() {
        anyhow::bail!("User with username '{}' already exists.", args.username);
    }

    println!("Enter password for new user '{}':", args.username);
    let password = rpassword::read_password()?;
    let password_hash = auth::hash_password(&password)?;

    let new_user = user::ActiveModel {
        username: Set(args.username),
        password_hash: Set(password_hash),
        ..Default::default()
    };
    let new_user = new_user.insert(&db.conn).await?;

    println!("Successfully created new user: {}", new_user.username);

    Ok(())
}
