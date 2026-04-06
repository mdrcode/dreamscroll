const SERVICE_NAME: &str = "dreamscroll-util-rest-token";

fn canonicalize_host_for_cache(host: &str) -> String {
    let mut normalized = host.trim().trim_end_matches('/').to_ascii_lowercase();

    if let Some(stripped) = normalized.strip_prefix("https://") {
        normalized = stripped.to_string();
    } else if let Some(stripped) = normalized.strip_prefix("http://") {
        normalized = stripped.to_string();
    }

    if let Some(stripped) = normalized.strip_suffix("/api") {
        normalized = stripped.to_string();
    }

    normalized
}

fn canonicalize_username_for_cache(username: &str) -> String {
    username.trim().to_ascii_lowercase()
}

fn canonical_identity(host: &str, username: &str) -> String {
    format!(
        "{}|{}",
        canonicalize_host_for_cache(host),
        canonicalize_username_for_cache(username)
    )
}

fn account_key(host: &str, username: &str) -> String {
    let identity = canonical_identity(host, username);
    let digest = blake3::hash(identity.as_bytes());
    format!("v2-{}", digest.to_hex())
}

pub fn get_token(host: &str, username: &str) -> anyhow::Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE_NAME, &account_key(host, username))?;

    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(anyhow::anyhow!("failed reading cached API token: {}", err)),
    }
}

pub fn set_token(host: &str, username: &str, token: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, &account_key(host, username))?;
    entry
        .set_password(token)
        .map_err(|err| anyhow::anyhow!("failed storing cached API token: {}", err))
}

pub fn delete_token(host: &str, username: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(SERVICE_NAME, &account_key(host, username))?;

    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("failed deleting cached API token: {}", err)),
    }
}
