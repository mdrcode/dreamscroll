use anyhow::anyhow;

use crate::{auth, database::DbHandle};

pub async fn authenticate_user_stdin(db: &DbHandle) -> anyhow::Result<auth::DreamscrollAuthUser> {
    println!("Enter username: ");
    let mut username = String::new();
    std::io::stdin().read_line(&mut username)?;
    let username = username.trim();
    println!("Enter password: ");
    let password = rpassword::read_password()?;

    let user = auth::password::verify(db, username, &password)
        .await
        .map_err(|e| anyhow!("Authentication failed: {}", e))?;

    Ok(user)
}
