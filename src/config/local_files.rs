use dotenvy;

/// Containerized environments should set `NO_LOCAL_CONFIG_FILES` (any value)
/// to skip this, since config comes from Containerized real env vars.
pub fn import_local_if_test_or_dev() {
    if std::env::var("NO_LOCAL_CONFIG_FILES").is_ok() {
        return;
    }

    // Use (e)println! since tracing might not be initialized

    match dotenvy::from_filename("config_local.env") {
        Ok(_) => println!("Loaded config_local.env successfully"),
        Err(err) => eprintln!(
            "Failed to load config_local.env, will rely on env vars. Error: {:?}",
            err
        ),
    }

    match dotenvy::from_filename(".env") {
        Ok(_) => println!("Loaded .env successfully"),
        Err(err) => eprintln!(
            "Failed to load .env, will rely on env vars for secrets. Error: {:?}",
            err
        ),
    }
}
