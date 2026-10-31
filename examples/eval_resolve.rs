//! Score the resolver against a gold set of entities that should be impossible to miss.
//!
//! Run: cargo run --example eval_resolve -- <gold.tsv>
//!
//! Each gold row is `label <TAB> qid <TAB> keyword`. The qid is verified against Wikidata first
//! (label must match, description must contain the keyword), so a wrong "gold" QID is dropped rather
//! than counting against the scorer.

use std::collections::HashMap;

use dreamscroll::knowledge::cache::Outcome;
use dreamscroll::knowledge::resolve::wikidata::Wikidata;
use dreamscroll::knowledge::resolve::{CoreView, Engine, Raw};
use dreamscroll::{config, database};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::Value;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::populate_env_if_test_or_dev();
    let path = std::env::args().nth(1).expect("usage: eval_resolve <gold.tsv>");
    let gold: Vec<(String, String, String)> = std::fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            (
                parts.next().unwrap_or_default().to_string(),
                parts.next().unwrap_or_default().to_string(),
                parts.next().unwrap_or_default().to_lowercase(),
            )
        })
        .filter(|(label, qid, _)| !label.is_empty() && !qid.is_empty())
        .collect();

    let conn = database::connect_postgres_db(&config::make_postgres()?).await?;
    let engine = Wikidata::new()?;

    // 1. verify the gold itself
    let values = gold
        .iter()
        .map(|(_, qid, _)| format!("wd:{qid}"))
        .collect::<Vec<_>>()
        .join(" ");
    let query = format!(
        "SELECT ?item ?itemLabel ?itemDescription WHERE {{ VALUES ?item {{ {values} }} \
         SERVICE wikibase:label {{ bd:serviceParam wikibase:language \"en\". }} }}"
    );
    let response = reqwest::Client::builder()
        .user_agent("dreamscroll-knowledge/0.1 (https://dreamscroll.ai)")
        .build()?
        .post("https://query.wikidata.org/sparql")
        .header("Accept", "application/sparql-results+json")
        .form(&[("query", query.as_str()), ("format", "json")])
        .send()
        .await?
        .json::<Value>()
        .await?;
    let mut verified: HashMap<String, (String, String)> = HashMap::new();
    for row in response
        .pointer("/results/bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let get = |key: &str| row.get(key).and_then(|v| v.get("value")).and_then(Value::as_str).unwrap_or_default();
        let qid = get("item").rsplit('/').next().unwrap_or_default().to_string();
        verified.insert(qid, (get("itemLabel").to_string(), get("itemDescription").to_lowercase()));
    }

    let mut kept = Vec::new();
    for (label, qid, keyword) in gold {
        match verified.get(&qid) {
            Some((gold_label, description))
                if gold_label.eq_ignore_ascii_case(&label) && description.contains(&keyword) =>
            {
                kept.push((label, qid));
            }
            other => println!("DROP {label} -> {qid}: gold does not check out ({other:?})"),
        }
    }
    println!("{} of the gold set verified\n", kept.len());

    // 2. score each verified case with the real engine, against cached candidates
    println!("{:<30} {:<10} {:<10} result", "label", "gold", "got");
    let (mut pass, mut fail, mut missing) = (0, 0, 0);
    for (label, qid) in &kept {
        let row = conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT c.id, c.entity_type, coalesce(c.description,'') FROM core_knowledge_entities c \
                 WHERE c.label = $1 LIMIT 1",
                [label.clone().into()],
            ))
            .await?;
        let Some(row) = row else {
            println!("{label:<30} {qid:<10} {:<10} no core", "-");
            missing += 1;
            continue;
        };
        let payload = conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT payload::text FROM resolution_cache WHERE kind='label' AND query=$1 \
                 ORDER BY id DESC LIMIT 1",
                [label.clone().into()],
            ))
            .await?;
        let Some(payload) = payload else {
            println!("{label:<30} {qid:<10} {:<10} not cached", "-");
            missing += 1;
            continue;
        };
        let view = CoreView {
            id: row.try_get_by_index(0)?,
            label: label.clone(),
            entity_type: row.try_get_by_index(1)?,
            description: Some(row.try_get_by_index::<String>(2)?).filter(|d| !d.is_empty()),
            aliases: Vec::new(),
            search_query: None,
            handle: None,
        };
        let request = engine.plan(&view).into_iter().next().expect("a plan");
        let raw = Raw {
            outcome: Outcome::Found,
            payload: Some(serde_json::from_str(&payload.try_get_by_index::<String>(0)?)?),
        };
        let claims = engine.parse(&view, &request, &raw);
        let got = claims.first().map(|c| c.id.clone()).unwrap_or_default();
        if &got == qid {
            pass += 1;
            println!("{label:<30} {qid:<10} {got:<10} PASS");
        } else {
            fail += 1;
            let why = raw
                .payload
                .as_ref()
                .and_then(|p| p.get("candidates"))
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(got.as_str())))
                .and_then(|c| c.get("description"))
                .and_then(Value::as_str)
                .unwrap_or("(nothing claimed)");
            println!("{label:<30} {qid:<10} {:<10} FAIL — got {why}", if got.is_empty() { "-" } else { &got });
        }
    }
    println!("\npass {pass} · fail {fail} · not scored {missing}");
    Ok(())
}
