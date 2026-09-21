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

