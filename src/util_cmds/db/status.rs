use argh::FromArgs;

use crate::database;

#[derive(FromArgs)]
#[argh(subcommand, name = "status")]
#[argh(
    description = "Read-only report: schema marker, pending migrations, legacy tables, graph counts"
)]
pub struct StatusArgs {
    #[argh(switch)]
    #[argh(description = "print JSON")]
    pub json: bool,
}

pub async fn run(db: &database::DbHandle, args: StatusArgs) -> anyhow::Result<()> {
    let status = database::status(&db.conn).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    let applied = match status.applied_schema_version {
        Some(version) => version.to_string(),
        None => "missing".to_owned(),
    };
    println!(
        "schema applied={applied} required={} ({})",
        status.required_schema_version,
        if status.in_sync() {
            "in sync"
        } else {
            "needs sync"
        }
    );

    if status.pending.is_empty() {
        println!("pending: none");
    } else {
        for step in &status.pending {
            println!("pending: {step}");
        }
    }

    if status.legacy_tables.is_empty() {
        println!("legacy tables: none");
    } else {
        println!("legacy tables: {}", status.legacy_tables.join(", "));
    }

    println!("rows:");
    for (table, rows) in &status.row_counts {
        println!("  {table} = {rows}");
    }
    if let Some(linked) = status.observations_linked {
        println!("observations: linked {linked}");
    }
    if !status.entities_by_status.is_empty() {
        let rendered = status
            .entities_by_status
            .iter()
            .map(|(state, count)| format!("{state}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("entities: {rendered}");
    }
    if !status.identities_by_provider.is_empty() {
        let rendered = status
            .identities_by_provider
            .iter()
            .map(|(provider, count)| format!("{provider}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!("identities: {rendered}");
    }

    Ok(())
}
