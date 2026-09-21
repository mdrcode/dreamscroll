use anyhow::anyhow;

use crate::{auth, database::DbHandle};

pub fn prompt_username_stdin() -> anyhow::Result<String> {
    println!("Enter username: ");
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;
    Ok(username.trim().to_string())
}

pub fn prompt_password_stdin() -> anyhow::Result<String> {
    println!("Enter password: ");
    let password = rpassword::read_password()?;
    Ok(password)
}

pub async fn authenticate_db_user_from_stdin(
    db: &DbHandle,
) -> anyhow::Result<auth::DreamscrollAuthUser> {
    let username = prompt_username_stdin()?;
    let password = prompt_password_stdin()?;

    let user = auth::password::authenticate(db, &username, &password)
        .await
        .map_err(|e| anyhow!("Authentication failed: {}", e))?;

    Ok(user)
}
