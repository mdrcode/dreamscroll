use dotenvy;

pub fn load_local_config_files() {
    let _ = dotenvy::from_filename("ds_config_local.env")
        .inspect_err(|_| tracing::warn!("Didn't find ds_local_config.env, will rely on env vars."));

    // secrets from .env (gitignored for api keys, etc)
    let _ = dotenvy::from_filename(".env").inspect_err(|_| {
        tracing::warn!("Didn't find .env, will rely on env vars for secrets instead")
    });
}
