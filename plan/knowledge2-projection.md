# Projection hardening

Status: Ready — v1 is removed.

The v2 graph projects natively (`src/knowledge2/graph/project.rs`, commit `5409005497`); the last
corpus check matched the old shape exactly. This plan covers the projection changes that were held
behind the v1 comparison.

## 1. Serialize projection with a row lock

Projection reads the members, computes the columns, then updates the entity row. Two mutations can
target the same source entity and interleave their final projections; the later-committing stale
projection can win.

`proc` locks many entities through `resolve`, but not every entity it later projects — the source side
of `AttachObservation` is the clear example.

- Take `SELECT id FROM knowledge_entities WHERE id = $1 FOR UPDATE` before reading members.
- Every mutation writes before projecting, so the lock gives the last committed mutation the final
  word.
- Behavior-neutral; add a concurrency test (two mutations, one entity, both memberships in the final
  projection).

## 2. Write only when the derived values changed

Every projection rewrites the row and the index. `reproject --all` therefore bumps every entity's
`updated_at` and every index row's `built_at`, even when nothing changed.

- Compare the computed columns to the current row; skip the entity update when equal.
- Rewrite the index when the entity changed, or when its `built_with` fingerprint is stale.
- Keeps `updated_at` meaning "the projection changed," not "someone asked."
- Behavior-neutral for the shape comparison; makes `Report.touched` meaningful.

## 3. Make projection pure

The impure part is the fallback to the entity's own previous values:

| column group | primary source | fallback today |
|---|---|---|
| `name`, `entity_type`, `alt_types` | members | none |
| `description`, `alt_descriptions` | selected member | previous entity row |
| `search_query`, `alt_search_queries` | selected member | previous entity row |
| `platform`, `platform_handle`, `platform_display_name` | selected member | previous entity row |
| `status`, `confidence` | identity rows | none |

The comment says the fallback protects a resolver-sourced description. That writer does not exist:

- `CreateEntity` (`src/knowledge2/graph/sql.rs`) sets the entity description from the observation at
  create.
- Identity rows carry their own `description`; nothing copies it onto the entity.
- Projection is the only other writer of the entity description.

So the fallback preserves **the last projection**, which can be a detached or merged-away member's
text or platform. After a detach, an entity can keep a description and a handle no current member
carries, and `reproject` faithfully preserves them.

The pure rule is a function of members and identities only:

- `description` / `search_query`: the selected member's, else `None` — or, if provider text is
  wanted, the best identity annotation's description, chosen explicitly.
- `alt_descriptions` / `alt_search_queries`: members only.
- `platform_*`: members' presence only — or an explicit evidence source, never the previous row.
- `origin` remains the durable input; the origin/newest-illumination selection stays.
- `status` / `confidence`: identity rows, unchanged.

This makes `reproject` a function of the durable graph rather than of its own prior output.

## Decisions

- **Provider text.** Does an entity with no member description take the best identity's description,
  or stay empty? If the former, the identity row is the explicit source; if the latter, the entity
  has no description until a member supplies one.
- **Platform presence.** Members only, an explicit identity-derived presence, or keep the current
  hint? This is the same zero-platform-coupling question as the entity columns themselves.
- **Timing.** v1 is gone; apply purity directly to the only engine.

## Keep

- **Origin selection and the newest-illumination rule.** Subtle, measured, and verified by the corpus
  diff.
- **Projection owns the blocking index.** The index is derived retrieval material, not blocker
  policy; writing it in the mutation transaction is the right boundary.
- **`alt_*` selection.** The selected value stays while a member still offers it; everything else is
  an alternative.

## Watch / out of scope

- **`Scope::Stale` is narrower than it sounds.** It selects entities with zero identities; a
  one-identity entity whose confidence or surface changed is not selected. Rename it to
  `Unidentified`, or default repairs to `All`.
- **Per-entity N+1.** Fine for a link batch; potentially expensive for a resolver batch of hundreds
  inside one transaction. Measure at the resolve cutover; a batched projector is the optimization.
- **Dedupe inconsistency.** Names dedupe by folded key (first spelling wins); types and descriptions
  dedupe by exact string. Make that deliberate.
- **Status semantics.** `identified` currently means "any identity row," including an illuminator
  handle. The attestation model (`plan/knowledge-entity-attestations.md`) may change this; keep it
  out of this plan.

## Verification

- Projection unit tests for: superseded origin, detached origin, empty members, multi-member
  selection, no member text, platform fallback, and identity-derived text if chosen.
- Concurrency test for the row lock.
- Idempotence: `reproject --all` twice; the second run changes nothing.
- Corpus shape diff when purity lands, with every difference classified as stale text or platform.

## Files

- `src/knowledge2/graph/project.rs`
- `src/knowledge2/graph/graph.rs` (`proc`, `reproject`)
- `src/knowledge2/blocker/index.rs` (fingerprint and write)
- `src/knowledge2/canonical.rs` (verification)
