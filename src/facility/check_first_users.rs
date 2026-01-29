use sea_orm::prelude::*;

use crate::{auth, database::DbHandle, model};

pub async fn check_first_users(db: &DbHandle) -> anyhow::Result<()> {
    let user_count = model::user::Entity::find().count(&db.conn).await?;

    if user_count > 0 {
        return Ok(());
    }

    println!("No users found in current database, creating default admin user.");
    println!("Enter username for new user with admin privileges:");
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();

    println!("Enter password for new user:");
    let password1 = rpassword::read_password()?;
    println!("Re-enter password for new user:");
    let password2 = rpassword::read_password()?;

    if password1 != password2 {
        anyhow::bail!("Passwords do not match");
    }

    let hash = auth::password::hash(&password1)?;

    let new_user = model::user::ActiveModel::builder()
        .set_username(username.clone())
        .set_password_hash(hash)
        .set_is_admin(true)
        .save(&db.conn)
        .await?;

    println!(
        "Successfully created admin user '{}' with id {}",
        username,
        new_user.id.unwrap()
    );

    Ok(())
}
