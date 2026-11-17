//! Every write the graph performs, in SQL.
//!
//! The statements carry the integrity rules: an observation is created from only while it is
//! unlinked, an identity is one row per `(entity, provider, key)`, a tombstone is a forwarding
//! pointer, and a veto is admission for every later write. The Rust above this file only decides
//! which statement to run and how to report its result.

use sea_orm::sea_query::ArrayType;
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, Value};

use crate::knowledge2::action::{Cause, KnowledgeAction};
use crate::knowledge2::identity::{Identity, IdentityKey, Provider};
use crate::knowledge2::ids::{EntityOriginId, ObservationId};

use super::report::{Outcome, Refusal};

pub(super) async fn apply<C: ConnectionTrait>(
    conn: &C,
    action: &KnowledgeAction,
) -> anyhow::Result<Outcome> {
    match action {
        KnowledgeAction::CreateEntity { from, with } => create(conn, *from, with.as_ref()).await,
        KnowledgeAction::AttachObservation {
            observation,
            entity,
            with,
            ..
        } => attach(conn, *observation, *entity, with.as_ref()).await,
        KnowledgeAction::DetachObservation { observation, .. } => detach(conn, *observation).await,
        KnowledgeAction::MergeEntity {
            loser,
            winner,
            cause,
            why,
        } => merge(conn, *loser, *winner, *cause, why).await,
        KnowledgeAction::MarkDistinct { a, b, cause, why } => {
            distinct(conn, *a, *b, *cause, why).await
        }
        KnowledgeAction::AttestIdentity { subject, identity } => {
            attest_to(conn, *subject, identity).await
        }
        KnowledgeAction::RetractIdentity {
            subject,
            provider,
            key,
        } => retract(conn, *subject, provider, key).await,
        KnowledgeAction::VetoIdentity {
            subject,
            key,
            cause,
            why,
        } => veto(conn, *subject, key, *cause, why).await,
        KnowledgeAction::UnvetoIdentity {
            subject,
            key,
            cause,
        } => unveto(conn, *subject, key, *cause).await,
    }
}

// ---------------------------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------------------------

fn statement(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values)
}

async fn one<C: ConnectionTrait>(
    conn: &C,
    sql: &str,
    values: Vec<Value>,
) -> anyhow::Result<Option<QueryResult>> {
    Ok(conn.query_one_raw(statement(sql, values)).await?)
}

async fn all<C: ConnectionTrait>(
    conn: &C,
    sql: &str,
    values: Vec<Value>,
) -> anyhow::Result<Vec<QueryResult>> {
    Ok(conn.query_all_raw(statement(sql, values)).await?)
}

async fn execute<C: ConnectionTrait>(
    conn: &C,
    sql: &str,
    values: Vec<Value>,
) -> anyhow::Result<u64> {
    Ok(conn.execute_raw(statement(sql, values)).await?.rows_affected())
}

fn int(value: i32) -> Value {
    Value::Int(Some(value))
}

fn text(value: &str) -> Value {
    Value::String(Some(value.to_string()))
}

fn strings(values: &[String]) -> Value {
    Value::Array(
        ArrayType::String,
        Some(Box::new(
            values
                .iter()
                .cloned()
                .map(|value| Value::String(Some(value)))
                .collect(),
        )),
    )
}

// ---------------------------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------------------------

/// An origin resolves through `merged_into_id` to the live row. Two live rows claim one origin only
/// if a caller wrote past the graph; the executor refuses rather than choosing.
const RESOLVE: &str = r#"
WITH RECURSIVE chain AS (
    SELECT id, merged_into_id
      FROM knowledge_entities
     WHERE origin_observation_id = $1
    UNION
    SELECT e.id, e.merged_into_id
      FROM knowledge_entities e
      JOIN chain c ON e.id = c.merged_into_id
)
SELECT id FROM chain WHERE merged_into_id IS NULL
"#;

async fn resolve<C: ConnectionTrait>(
    conn: &C,
    origin: EntityOriginId,
) -> anyhow::Result<Result<i32, Refusal>> {
    let rows = all(conn, RESOLVE, vec![int(origin.0.0)]).await?;
    let id: i32 = match rows.len() {
        0 => return Ok(Err(Refusal::UnknownOrigin(origin))),
        1 => rows[0].try_get_by_index(0)?,
        _ => return Ok(Err(Refusal::AmbiguousOrigin(origin))),
    };

    // Lock the live row: a concurrent fold must not tombstone it under this action.
    let Some(row) = one(
        conn,
        "SELECT id FROM knowledge_entities \
         WHERE id = $1 AND merged_into_id IS NULL FOR UPDATE",
        vec![int(id)],
    )
    .await?
    else {
        return Ok(Err(Refusal::UnknownOrigin(origin)));
    };

    Ok(Ok(row.try_get_by_index(0)?))
}

async fn origin_of<C: ConnectionTrait>(conn: &C, entity: i32) -> anyhow::Result<Option<i32>> {
    let row = one(
        conn,
        "SELECT origin_observation_id FROM knowledge_entities WHERE id = $1",
        vec![int(entity)],
    )
    .await?;
    Ok(match row {
        Some(row) => row.try_get_by_index(0)?,
        None => None,
    })
}

async fn generation<C: ConnectionTrait>(conn: &C) -> anyhow::Result<i32> {
    let row = one(
        conn,
        "SELECT COALESCE(MAX(id), 1) FROM knowledge_generations",
        Vec::new(),
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("knowledge_generations is unreadable"))?;
    Ok(row.try_get_by_index(0)?)
}

// ---------------------------------------------------------------------------------------------
// Membership
// ---------------------------------------------------------------------------------------------

const CREATE: &str = r#"
WITH obs AS (
    SELECT id, name, entity_type, description, search_query,
           platform, platform_handle, platform_display_name
      FROM user_observed_entities
     WHERE id = $1 AND knowledge_entity_id IS NULL
), ent AS (
    INSERT INTO knowledge_entities
        (name, entity_type, status, description, search_query,
         platform, platform_handle, platform_display_name, origin_observation_id)
    SELECT name, entity_type, 'provisional', description, search_query,
           platform, platform_handle, platform_display_name, id
      FROM obs
    RETURNING id
)
UPDATE user_observed_entities o
   SET knowledge_entity_id = ent.id
  FROM ent
 WHERE o.id = $1
RETURNING o.knowledge_entity_id
"#;

async fn create<C: ConnectionTrait>(
    conn: &C,
    from: ObservationId,
    with: Option<&Identity>,
) -> anyhow::Result<Outcome> {
    let row = one(
        conn,
        "SELECT knowledge_entity_id FROM user_observed_entities WHERE id = $1 FOR UPDATE",
        vec![int(from.0)],
    )
    .await?;
    let Some(row) = row else {
        return Ok(Outcome::Refused(Refusal::UnknownObservation(from)));
    };
    let current: Option<i32> = row.try_get_by_index(0)?;

    if let Some(entity) = current {
        let is_origin = one(
            conn,
            "SELECT 1 FROM knowledge_entities \
             WHERE id = $1 AND origin_observation_id = $2 AND merged_into_id IS NULL",
            vec![int(entity), int(from.0)],
        )
        .await?
        .is_some();
        return Ok(if is_origin {
            Outcome::NoChange
        } else {
            Outcome::Refused(Refusal::LinkedElsewhere(from))
        });
    }

    if one(
        conn,
        "SELECT 1 FROM knowledge_entities WHERE origin_observation_id = $1",
        vec![int(from.0)],
    )
    .await?
    .is_some()
    {
        return Ok(Outcome::Refused(Refusal::OriginClaimed(from)));
    }

    let Some(row) = one(conn, CREATE, vec![int(from.0)]).await? else {
        return Ok(Outcome::Refused(Refusal::UnknownObservation(from)));
    };
    let entity: i32 = row.try_get_by_index(0)?;

    if let Some(identity) = with {
        // Vetoed evidence is dropped, not an error: the membership still applies.
        let _ = attest(conn, entity, from.0, identity).await?;
    }

    Ok(Outcome::Applied(vec![entity]))
}

async fn attach<C: ConnectionTrait>(
    conn: &C,
    observation: ObservationId,
    target: EntityOriginId,
    with: Option<&Identity>,
) -> anyhow::Result<Outcome> {
    let entity = match resolve(conn, target).await? {
        Ok(entity) => entity,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let origin = origin_of(conn, entity)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {entity} has no origin"))?;

    let row = one(
        conn,
        "SELECT knowledge_entity_id FROM user_observed_entities WHERE id = $1 FOR UPDATE",
        vec![int(observation.0)],
    )
    .await?;
    let Some(row) = row else {
        return Ok(Outcome::Refused(Refusal::UnknownObservation(observation)));
    };
    let old: Option<i32> = row.try_get_by_index(0)?;

    if old == Some(entity) {
        if let Some(identity) = with {
            let _ = attest(conn, entity, origin, identity).await?;
            return Ok(Outcome::Applied(vec![entity]));
        }
        return Ok(Outcome::NoChange);
    }

    let moved = execute(
        conn,
        "UPDATE user_observed_entities o \
            SET knowledge_entity_id = e.id \
           FROM knowledge_entities e \
          WHERE o.id = $1 AND e.id = $2 AND e.merged_into_id IS NULL",
        vec![int(observation.0), int(entity)],
    )
    .await?;
    anyhow::ensure!(moved == 1, "entity {entity} was tombstoned concurrently");

    if let Some(identity) = with {
        let _ = attest(conn, entity, origin, identity).await?;
    }

    let mut touched = vec![entity];
    if let Some(old) = old {
        touched.push(old);
    }
    Ok(Outcome::Applied(touched))
}

async fn detach<C: ConnectionTrait>(
    conn: &C,
    observation: ObservationId,
) -> anyhow::Result<Outcome> {
    let row = one(
        conn,
        "SELECT knowledge_entity_id FROM user_observed_entities WHERE id = $1 FOR UPDATE",
        vec![int(observation.0)],
    )
    .await?;
    let Some(row) = row else {
        return Ok(Outcome::Refused(Refusal::UnknownObservation(observation)));
    };
    let old: Option<i32> = row.try_get_by_index(0)?;
    let Some(old) = old else {
        return Ok(Outcome::NoChange);
    };

    execute(
        conn,
        "UPDATE user_observed_entities SET knowledge_entity_id = NULL WHERE id = $1",
        vec![int(observation.0)],
    )
    .await?;

    Ok(Outcome::Applied(vec![old]))
}

// ---------------------------------------------------------------------------------------------
// Fold
// ---------------------------------------------------------------------------------------------

const STANDING_DISTINCT: &str = r#"
SELECT 1 FROM knowledge_verdicts
 WHERE kind = 'distinct'
   AND ((winner_origin = $1 AND loser_origin = $2)
     OR (winner_origin = $2 AND loser_origin = $1))
 LIMIT 1
"#;

const SHARED_ILLUMINATION: &str = r#"
SELECT 1
  FROM user_observed_entities a
  JOIN user_observed_entities b ON a.illumination_id = b.illumination_id
 WHERE a.knowledge_entity_id = $1
   AND b.knowledge_entity_id = $2
 LIMIT 1
"#;

const DEDUPE_IDENTITIES: &str = r#"
DELETE FROM knowledge_identities l
 USING knowledge_identities w
 WHERE l.knowledge_entity_id = $1
   AND w.knowledge_entity_id = $2
   AND l.provider = w.provider
   AND l.scheme = w.scheme
   AND l.external_id = w.external_id
"#;

const INSERT_MERGE: &str = r#"
INSERT INTO knowledge_verdicts
    (generation, cause, kind, winner_origin, loser_origin, note)
VALUES ($1, $2, 'merge', $3, $4, $5)
"#;

async fn merge<C: ConnectionTrait>(
    conn: &C,
    loser: EntityOriginId,
    winner: EntityOriginId,
    cause: Cause,
    why: &str,
) -> anyhow::Result<Outcome> {
    let loser_id = match resolve(conn, loser).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let winner_id = match resolve(conn, winner).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    if loser_id == winner_id {
        return Ok(Outcome::NoChange);
    }

    let loser_origin = origin_of(conn, loser_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {loser_id} has no origin"))?;
    let winner_origin = origin_of(conn, winner_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {winner_id} has no origin"))?;

    if one(
        conn,
        STANDING_DISTINCT,
        vec![int(loser_origin), int(winner_origin)],
    )
    .await?
    .is_some()
    {
        return Ok(Outcome::Refused(Refusal::StandingDistinct));
    }
    if one(
        conn,
        SHARED_ILLUMINATION,
        vec![int(loser_id), int(winner_id)],
    )
    .await?
    .is_some()
    {
        return Ok(Outcome::Refused(Refusal::SharedIllumination));
    }

    execute(
        conn,
        "UPDATE user_observed_entities SET knowledge_entity_id = $1 WHERE knowledge_entity_id = $2",
        vec![int(winner_id), int(loser_id)],
    )
    .await?;
    execute(conn, DEDUPE_IDENTITIES, vec![int(loser_id), int(winner_id)]).await?;
    execute(
        conn,
        "UPDATE knowledge_identities SET knowledge_entity_id = $1 WHERE knowledge_entity_id = $2",
        vec![int(winner_id), int(loser_id)],
    )
    .await?;

    let tombstoned = execute(
        conn,
        "UPDATE knowledge_entities SET merged_into_id = $1, updated_at = now() \
         WHERE id = $2 AND merged_into_id IS NULL",
        vec![int(winner_id), int(loser_id)],
    )
    .await?;
    anyhow::ensure!(
        tombstoned == 1,
        "entity {loser_id} was tombstoned under another writer"
    );
    crate::knowledge::blocker::index::forget(conn, loser_id).await?;

    execute(
        conn,
        INSERT_MERGE,
        vec![
            int(generation(conn).await?),
            text(cause.as_str()),
            int(winner_origin),
            int(loser_origin),
            text(why),
        ],
    )
    .await?;

    Ok(Outcome::Applied(vec![winner_id]))
}

// ---------------------------------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------------------------------

const PENDING_MERGE: &str = r#"
SELECT 1 FROM knowledge_verdicts
 WHERE kind = 'merge' AND cause = 'repair'
   AND ((winner_origin = $1 AND loser_origin = $2)
     OR (winner_origin = $2 AND loser_origin = $1))
 LIMIT 1
"#;

const INSERT_DISTINCT: &str = r#"
INSERT INTO knowledge_verdicts
    (generation, cause, kind, winner_origin, loser_origin, note)
VALUES ($1, $2, 'distinct', $3, $4, $5)
"#;

async fn distinct<C: ConnectionTrait>(
    conn: &C,
    a: EntityOriginId,
    b: EntityOriginId,
    cause: Cause,
    why: &str,
) -> anyhow::Result<Outcome> {
    let a_id = match resolve(conn, a).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let b_id = match resolve(conn, b).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    if a_id == b_id {
        return Ok(Outcome::Refused(Refusal::Contradiction(
            "an entity cannot be distinct from itself",
        )));
    }

    let a_origin = origin_of(conn, a_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {a_id} has no origin"))?;
    let b_origin = origin_of(conn, b_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {b_id} has no origin"))?;

    if one(conn, PENDING_MERGE, vec![int(a_origin), int(b_origin)])
        .await?
        .is_some()
    {
        return Ok(Outcome::Refused(Refusal::PendingMerge));
    }
    if one(conn, STANDING_DISTINCT, vec![int(a_origin), int(b_origin)])
        .await?
        .is_some()
    {
        return Ok(Outcome::NoChange);
    }

    execute(
        conn,
        INSERT_DISTINCT,
        vec![
            int(generation(conn).await?),
            text(cause.as_str()),
            int(a_origin),
            int(b_origin),
            text(why),
        ],
    )
    .await?;

    Ok(Outcome::Applied(Vec::new()))
}

// ---------------------------------------------------------------------------------------------
// Identity evidence
// ---------------------------------------------------------------------------------------------

const VETOED: &str = r#"
SELECT 1 FROM knowledge_verdicts
 WHERE kind = 'veto_identity'
   AND subject_origin = $1
   AND identity_scheme = $2
   AND identity_external_id = $3
 LIMIT 1
"#;

const UPSERT_IDENTITY: &str = r#"
INSERT INTO knowledge_identities
    (knowledge_entity_id, provider, scheme, external_id, method, confidence,
     label, description, classes, user_observed_entity_id)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
ON CONFLICT (knowledge_entity_id, provider, scheme, external_id)
DO UPDATE SET
    method = EXCLUDED.method,
    confidence = EXCLUDED.confidence,
    label = EXCLUDED.label,
    description = EXCLUDED.description,
    classes = EXCLUDED.classes,
    -- Provenance is the first observation that carried this key; a repeat refines the metadata,
    -- it does not rewrite where the evidence came from.
    user_observed_entity_id = COALESCE(
        knowledge_identities.user_observed_entity_id,
        EXCLUDED.user_observed_entity_id
    )
"#;

async fn attest_to<C: ConnectionTrait>(
    conn: &C,
    subject: EntityOriginId,
    identity: &Identity,
) -> anyhow::Result<Outcome> {
    let entity = match resolve(conn, subject).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let origin = origin_of(conn, entity)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {entity} has no origin"))?;
    attest(conn, entity, origin, identity).await
}

async fn attest<C: ConnectionTrait>(
    conn: &C,
    entity: i32,
    origin: i32,
    identity: &Identity,
) -> anyhow::Result<Outcome> {
    if one(
        conn,
        VETOED,
        vec![
            int(origin),
            text(&identity.scheme),
            text(&identity.external_id),
        ],
    )
    .await?
    .is_some()
    {
        return Ok(Outcome::NoChange);
    }

    execute(
        conn,
        UPSERT_IDENTITY,
        vec![
            int(entity),
            text(&identity.provider.0),
            text(&identity.scheme),
            text(&identity.external_id),
            Value::String(identity.method.clone()),
            Value::Float(identity.confidence),
            Value::String(identity.label.clone()),
            Value::String(identity.description.clone()),
            strings(&identity.classes),
            Value::Int(identity.observation_id.map(|id| id.0)),
        ],
    )
    .await?;

    Ok(Outcome::Applied(vec![entity]))
}

async fn retract<C: ConnectionTrait>(
    conn: &C,
    subject: EntityOriginId,
    provider: &Provider,
    key: &IdentityKey,
) -> anyhow::Result<Outcome> {
    let entity = match resolve(conn, subject).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };

    let removed = execute(
        conn,
        "DELETE FROM knowledge_identities \
         WHERE knowledge_entity_id = $1 AND provider = $2 AND scheme = $3 AND external_id = $4",
        vec![
            int(entity),
            text(&provider.0),
            text(&key.scheme),
            text(&key.external_id),
        ],
    )
    .await?;

    Ok(if removed == 0 {
        Outcome::NoChange
    } else {
        Outcome::Applied(vec![entity])
    })
}

const INSERT_VETO: &str = r#"
INSERT INTO knowledge_verdicts
    (generation, cause, kind, subject_origin, identity_scheme, identity_external_id, note)
SELECT $1, $2, 'veto_identity', $3, $4, $5, $6
 WHERE NOT EXISTS (
    SELECT 1 FROM knowledge_verdicts
     WHERE kind = 'veto_identity'
       AND subject_origin = $3
       AND identity_scheme = $4
       AND identity_external_id = $5
 )
"#;

async fn veto<C: ConnectionTrait>(
    conn: &C,
    subject: EntityOriginId,
    key: &IdentityKey,
    cause: Cause,
    why: &str,
) -> anyhow::Result<Outcome> {
    let entity = match resolve(conn, subject).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let origin = origin_of(conn, entity)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {entity} has no origin"))?;

    let inserted = execute(
        conn,
        INSERT_VETO,
        vec![
            int(generation(conn).await?),
            text(cause.as_str()),
            int(origin),
            text(&key.scheme),
            text(&key.external_id),
            text(why),
        ],
    )
    .await?;

    let removed = execute(
        conn,
        "DELETE FROM knowledge_identities \
         WHERE knowledge_entity_id = $1 AND scheme = $2 AND external_id = $3",
        vec![int(entity), text(&key.scheme), text(&key.external_id)],
    )
    .await?;

    Ok(if inserted == 0 && removed == 0 {
        Outcome::NoChange
    } else {
        Outcome::Applied(vec![entity])
    })
}

async fn unveto<C: ConnectionTrait>(
    conn: &C,
    subject: EntityOriginId,
    key: &IdentityKey,
    cause: Cause,
) -> anyhow::Result<Outcome> {
    let entity = match resolve(conn, subject).await? {
        Ok(id) => id,
        Err(refusal) => return Ok(Outcome::Refused(refusal)),
    };
    let origin = origin_of(conn, entity)
        .await?
        .ok_or_else(|| anyhow::anyhow!("entity {entity} has no origin"))?;

    const ROWS: &str = r#"
    SELECT cause FROM knowledge_verdicts
     WHERE kind = 'veto_identity'
       AND subject_origin = $1
       AND identity_scheme = $2
       AND identity_external_id = $3
    "#;
    let values = || {
        vec![
            int(origin),
            text(&key.scheme),
            text(&key.external_id),
        ]
    };
    let rows = all(conn, ROWS, values()).await?;
    if rows.is_empty() {
        return Ok(Outcome::NoChange);
    }

    if cause == Cause::Model {
        for row in &rows {
            let stored: String = row.try_get_by_index(0)?;
            if stored == Cause::Repair.as_str() {
                return Ok(Outcome::Refused(Refusal::Unauthorized));
            }
        }
    }

    execute(
        conn,
        "DELETE FROM knowledge_verdicts \
         WHERE kind = 'veto_identity' AND subject_origin = $1 \
           AND identity_scheme = $2 AND identity_external_id = $3",
        values(),
    )
    .await?;

    Ok(Outcome::Applied(Vec::new()))
}
