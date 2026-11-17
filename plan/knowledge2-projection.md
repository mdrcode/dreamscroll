# Projection after the v1 → v2 cutover

Status: Deferred — trigger: the v1 → v2 cutover is complete.

The v2 graph projects natively (`src/knowledge2/graph/project.rs`, commit `5409005497`), and its
corpus shape is byte-identical to v1's: 2,336 entities, 354 identities, 2,802 memberships, 2,336
index rows, empty diff. This plan covers the projection changes that should wait until v1's write
path is gone.

The purity change below alters derived output. Applying it now breaks the byte-identical comparison
that gates the cutover, so it belongs after v1's projection is deleted (or must be applied to both
engines at once to keep the control honest).

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

- `insert_new` (`src/knowledge/graph/knowledge_graph.rs:246`) sets the entity description from the
  observation at create.
- Identity rows carry their own `description` (`src/knowledge/evidence/store.rs:286`); nothing copies
  it onto the entity.
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
- **Timing.** Apply purity to v1 and v2 together to preserve the comparison, or land it immediately
  after v1 is deleted. The latter is preferred; the migration gate stays clean.

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
- `src/knowledge/blocker/index.rs` (fingerprint and write)
- `src/knowledge2/shape.rs` (verification)
- `src/knowledge/graph/project.rs` (v1 control, until deleted)
