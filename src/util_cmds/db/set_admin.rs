use argh::FromArgs;
use sea_orm::{ActiveModelTrait, Set};

use crate::{database, model};

#[derive(FromArgs)]
#[argh(subcommand, name = "set_admin")]
#[argh(description = "Grant or revoke admin on an existing user")]
pub struct SetAdminArgs {
    #[argh(positional, description = "username")]
    username: String,

    #[argh(switch, description = "revoke admin instead of granting it")]
    revoke: bool,
}

/// Flips `users.is_admin` for one account.
///
/// Direct-database tool like `sync_schema`; holding the DB credentials is the
/// gate, so it does not prompt for an admin login. Used to make the local
/// `smoke-agent` account an admin so UI and REST smoke runs do not need a
/// personal account or a hand-minted token.
pub async fn run(db: &database::DbHandle, args: SetAdminArgs) -> anyhow::Result<()> {
    let user = model::user::Entity::find_by_username(args.username.clone())
        .one(&db.conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No user named '{}'", args.username))?;

    let is_admin = !args.revoke;
    if user.is_admin == is_admin {
        println!("{} is already is_admin={}", user.username, is_admin);
        return Ok(());
    }

    let mut active: model::user::ActiveModel = user.into();
    active.is_admin = Set(is_admin);
    active.update(&db.conn).await?;

    println!("{} is now is_admin={}", args.username, is_admin);
    Ok(())
}
