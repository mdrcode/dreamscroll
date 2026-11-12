# Identity annotations — delete the claim edge

**Status:** Implemented 2026-11-11, schema 39; archived 2026-11-12. `knowledge_claims` is gone; an
identity is an annotation on one entity. The day-to-day model is `src/knowledge/knowledge.md`.

**See also:** [lexicon.md](../lexicon.md) (vocabulary), [knowledge-resolution.md](knowledge-resolution.md)
(the resolver contract), [knowledge-claim-management.md](knowledge-claim-management.md) (the claim
model being removed), `src/knowledge/knowledge.md` (the model as built).

## The insight

A claim is not a thing. It is a row that exists only to say **this entity has this identity**. We
already know which entity, because resolution is always scoped to an entity, and the identity key is
`(scheme, external_id)`. The middle table has never carried a semantic that the identity annotation
could not carry directly.

The graph should model:

```
knowledge_entity 1 ─── * knowledge_identity
```

An identity is an **annotation on an entity**: "this authority named this entity
`(scheme, external_id)`, and said this about it." That is all. Sharing a key between two entities is
observed by grouping on the key — it is not a reason to store the key once and broker it.

This removes `knowledge_claims`, `Claim`, `ClaimOutcome`, `upsert_identity`, claim retraction,
claim moving, per-scheme replacement, identity-takeover precedence and the unique-violation match.
It also removes the global "one identity row per key" anchor: rows are annotations, and the key is
the durable name.

## The target schema

```sql
knowledge_identities
  id                      serial primary key
  knowledge_entity_id     integer not null references knowledge_entities(id)
  scheme                  text not null          -- wikidata, x_twitter, ...
  external_id             text not null          -- Q104919, bandy1803, ...
  label                   text
  description             text
  classes                 text[] not null default '{}'
  provider                text not null          -- resolver id, illuminator, model
  method                  text
  confidence              real
  user_observed_entity_id integer                -- provenance when an observation carried it
  observed_at             timestamptz not null default now()
  unique (knowledge_entity_id, provider, scheme, external_id)
```

No `knowledge_claims` table. No global `unique (scheme, external_id)`.

**Why `provider` stays in the key.** Two authorities may name one key for one entity. Wikidata may
confirm a handle the capture already showed; that is corroboration, with its own confidence and
method. Collapsing them into one row makes "last provider wins" and lets a resolver's full replace
delete a capture's handle annotation. Per-provider rows preserve both signals for one extra row.

**What the key means.** `(scheme, external_id)` is still the durable identity. A row is one
provider's annotation of one entity with that key. Two entities may hold the same key; that is a
proximity candidate, found by grouping rows on the key.

### Claim-field disposition

| `knowledge_claims` field | Destination |
| --- | --- |
| `id` | identity row's own id |
| `knowledge_entity_id` | `knowledge_identities.knowledge_entity_id` |
| `knowledge_identity_id` | gone — the row *is* the identity annotation |
| `provider` | already on the identity; becomes part of the unique key |
| `method` | already on the identity |
| `user_observed_entity_id` | moved to the identity row |
| `observed_at` | moved to the identity row |
| `retracted_at` | **deliberately dropped** — a veto deletes the annotation; `knowledge_verdicts` is the durable decision (see open question) |

`first_seen` is also dropped. It is unused, and `observed_at` is the timestamp that matters.

## Resolver contract

```rust
trait Resolver {
    fn id(&self) -> &'static str;

    /// "For this entity, here is the complete set of identities this authority asserts."
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)>;
}
```

`Identity` is the row before it has an entity or a provider:

```rust
pub struct Identity {
    pub scheme: String,
    pub external_id: String,
    pub method: Option<String>,
    pub confidence: Option<f32>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub observation_id: Option<i32>,   // observation provenance; None for an authority answer
}
```

The engine stamps `provider = resolver.id()` and the entity id. A resolver cannot attribute an
identity to another authority.

`IdentityKey` and `Identity::from_platform_link` stay; `IdentityDraft` disappears.

## The Evidence facade

Three public operations, plus the capture append and the fold move:

```rust
impl Evidence {
    /// Replace this provider's complete annotation set for one entity.
    /// One transaction: delete `(entity, provider)`, insert the new rows.
    pub async fn replace_provider_identities(
        &self,
        entity_id: i32,
        provider: &str,
        identities: &[Identity],
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<IdentityWrite>;

    /// Append one annotation, insert-if-absent. Used by the capture path, where one illumination
    /// is not the complete history of the entity.
    pub async fn add_identity(
        &self,
        entity_id: i32,
        provider: &str,
        identity: &Identity,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<bool>;

    /// This entity's annotations, optionally for one provider.
    pub async fn query_identities(
        &self,
        entity_id: i32,
        provider: Option<&str>,
    ) -> anyhow::Result<Vec<model::knowledge_identity::Model>>;

    /// Live entities holding this key, most substantiated first.
    pub async fn query_entities_for_identity(
        &self,
        key: &IdentityKey,
    ) -> anyhow::Result<Vec<model::knowledge_entity::Model>>;
}
```

Internals that remain but are not the facade: `shared_identity_keys()` for the proximity scan,
`move_identities(conn, loser, winner)` for a merge, `delete_identity(entity, key)` for a veto.

`IdentityWrite` is `{ written: usize, vetoed: usize }`.

## Full replace is the policy

A resolver's output is a complete statement about the entities it names. The engine therefore calls
`replace_provider_identities` for every entity in that resolver's batch **that got at least one
identity back**. An entity the resolver abstained on — no answer, an empty cache, a parser that now
rejects every candidate — keeps its old rows. This is deliberately conservative: only a positive
answer can overwrite, so an abstention never destroys an earlier identification.

`Options.replace`, `--replace`, `retract_unproposed` and the scheme-based keep set all disappear.

```
for each resolver:
    batch = entities not already annotated by this resolver (unless include_already_annotated)
    proposals = resolver.resolve(batch)
    if the resolver was refused (budget / rate limit):
        skip replacement for this resolver   // partial output is not a complete statement
    else:
        for each entity in batch with proposals:
            replace_provider_identities(entity, resolver.id(), proposals[entity])
        reproject touched entities
```

`--entity` still forces `include_already_annotated`, so a re-resolve replaces that entity's set when
the authority answers again. A normal corpus run selects only entities the provider has not
annotated, so replacement usually writes into an empty set. The observable rule is simple: **one
resolver owns its rows for an entity; a positive run rewrites them, an abstention leaves them.**

The capture path is the exception: `link` appends the handle an illumination showed with
`add_identity(provider = "illuminator")`. A later capture must not delete an earlier capture's
handle, so it can never be a full replace.

## Veto and replay

A veto still names `(entity, scheme, external_id)`. It deletes that key's annotation rows for the
entity, whatever providers wrote them, then reprojects. Admission asks the same veto set before
inserting, so a later `link` or `resolve` does not recreate the annotation.

No `retracted_at`: the annotation is derived and the verdict row is the durable record. The existing
`ReplayReport` counts stay: `withdrawn` when rows were deleted, `settled` when there was nothing to
delete.

## Projection, proximity, merge, selection

- **Projection** reads this entity's identity rows directly: status is `row count > 0`; confidence is
  the best row's `confidence`. No join to a claim table.
- **Proximity** groups identity rows by `(scheme, external_id)` where more than one live entity holds
  the key. `knowledge_proximities.knowledge_identity_id` becomes nullable `scheme` + `external_id`;
  the list prints the key directly, so it no longer joins the identity table.
- **Merge** deletes the loser's rows that would duplicate a winner row on
  `(provider, scheme, external_id)`, then re-points the rest to the winner, as `move_claims` does
  today.
- **Selection** becomes `IdentitySelection`: `awaiting_authority` finds live entities with no identity
  row from the given providers; `annotated_by` finds entities with one.
- **Reset** snapshots and deletes identities only. `Report.claims` and `Report.snapshot_claims` go;
  `knowledge_generations.claims` is dropped.

## What gets deleted

| File / item | Fate |
| --- | --- |
| `src/model/knowledge_claims.rs` | deleted |
| `src/knowledge/evidence/claim.rs` | deleted; `Claim` → `Identity` |
| `Evidence::upsert_identity` | deleted; full replace instead |
| `Evidence::claim_identity`, `write_claim`, `find_claim_id`, `unique_violation`, `ClaimOutcome` | deleted |
| `Evidence::retract`, `retract_unproposed` | deleted; `delete_identity` + replacement |
| `Evidence::holders_of_identity`, `find_identity_id` | replaced by `query_entities_for_identity` |
| `Evidence::move_claims` | `move_identities` |
| `ClaimSelection` | `IdentitySelection`, same two questions |
| `identity::MAX_CLAIMS_PER_ENTITY` | deleted (no cap) |
| `Options.replace`, `--replace` | deleted |
| `IdentityDraft` | deleted; one `Identity` type |
| claim snapshot / generation fields | deleted |
| claim indexes (`idx-knowledge_claims_*`) | deleted |
| `knowledge_claims` model references in stats, `identity inspect`, fixtures, verdict tests | updated to identities |

The expected result is a net deletion of several hundred lines of implementation and test, and one
fewer table to join, index, snapshot and reason about.

## Migration (schema 39)

`knowledge_identities`, `knowledge_claims` and `knowledge_proximities` are all derived: observations
and `fetch_cache` are the inputs, and `link` + `resolve --offline` rebuild them. The proposed
migration therefore **drops all three** and lets the registry create the new identity table. It does
not attempt to reshape or backfill rows in place.

`before_schema_sync`:

```sql
DROP TABLE IF EXISTS knowledge_proximities;
DROP TABLE IF EXISTS knowledge_claims;
DROP TABLE IF EXISTS knowledge_identities;
DROP INDEX IF EXISTS "idx-knowledge_identities_identity";
ALTER TABLE knowledge_generations DROP COLUMN IF EXISTS claims;
```

Registry sync creates `knowledge_identities` with the FK and the new unique key. Then the operator
runs the normal rebuild:

```sh
reset -y
link
verdicts replay
identity resolve --offline
proximity scan
```

Snapshots from earlier generations keep their old shape; `Snapshot`'s prune family list keeps
`knowledge_claims` so they are still cleaned up. If preserving the current local identity rows
matters, the alternative is a backfill from `knowledge_claims` (one new row per claim) before the
drop — more migration SQL for data that is reproducible from cache.

## Tests

Delete tests that only exist because of the claim indirection:

- identity upsert precedence and "a capture does not downgrade an answer" — no upsert exists;
- duplicate-claim idempotency and `AlreadyClaimed`;
- `retract_unproposed` scheme scoping;
- per-provider claim counts that are not assertions about the new model.

Keep and rewrite the behavioural ones:

- a provider's replace removes its old rows and leaves other providers' rows alone;
- an empty output leaves that provider's old rows (and a refused run does not replace at all);
- a capture's append never deletes another capture's handle;
- veto deletes the key across providers, reprojects, and admission refuses to recreate it;
- merge dedupes one identity per `(provider, key)` and moves the rest;
- proximity pairs two entities on one key, including a key held by two provider rows on one side;
- projection status/confidence read identity rows directly.

Add only where the new policy has a boundary that the old tests cannot cover: full replace is
entity-scoped and provider-scoped, and abstention is not deletion.

## Acceptance criteria

- `knowledge_claims` is gone from models, schema, indexes, stats, eval and tests.
- `Resolver::resolve` returns `(entity, Identity)` with no provider on the identity.
- `Evidence` exposes the four operations above; no caller writes `knowledge_identities` directly.
- A resolver re-run fully replaces its rows for an entity when it answers; an empty answer leaves
  them.
- A capture append is never a replace.
- A veto deletes the annotation and survives replay; `withdrawn`/`settled` distinguish the cases.
- Merge, reset, proximity and projection work without a claim join.
- The corpus rebuilds offline from `fetch_cache` with the same entity/identity coverage as before
  (accounting for the per-provider row split).

## Decisions

Decided 2026-11-11 before implementation:

1. **Uniqueness.** One row per `(entity, provider, scheme, external_id)`. Corroboration is
   preserved, and a resolver's replace cannot delete a capture's handle annotation.
2. **Retraction.** A veto deletes the annotation rows. `retracted_at` is dropped; the verdict row is
   the durable decision.
3. **Empty output.** Replace only when the resolver returned at least one identity for the entity.
   An abstention leaves the old rows. A refused provider (budget or rate limit) replaces nothing.
4. **Migration.** Drop `knowledge_identities`, `knowledge_claims` and `knowledge_proximities` in
   schema 39 and rebuild from observations plus `fetch_cache`.
