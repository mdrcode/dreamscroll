use dotenvy;

pub fn load_local_config_files() {
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
