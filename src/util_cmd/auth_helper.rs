use anyhow::anyhow;

use crate::{auth, database::DbHandle};

pub fn prompt_credentials_stdin() -> anyhow::Result<(String, String)> {
    println!("👤 Enter username: ");
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;

    println!("🔒 Enter password: ");
    let password = rpassword::read_password()?;

    Ok((username.trim().to_string(), password))
}

pub async fn authenticate_user_stdin(db: &DbHandle) -> anyhow::Result<auth::DreamscrollAuthUser> {
    let (username, password) = prompt_credentials_stdin()?;

    let user = auth::password::verify(db, &username, &password)
        .await
        .map_err(|e| anyhow!("Authentication failed: {}", e))?;

    Ok(user)
}
