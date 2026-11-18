# Projection hardening

Status: Landed 2026-11-18 — projection is pure for member text and platform presence. Watch items remain.

The v2 graph projects natively (`src/knowledge/graph/project.rs`, commit `5409005497`); the last
corpus check matched the old shape exactly. This plan covers the projection changes that were held
behind the v1 comparison.

## 1. Serialize projection with a row lock — landed

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

## 2. Write only when the derived values changed — landed

Every projection rewrites the row and the index. `reproject --all` therefore bumps every entity's
`updated_at` and every index row's `built_at`, even when nothing changed.

- Compare the computed columns to the current row; skip the entity update when equal.
- Rewrite the index when the entity changed, or when its `built_with` fingerprint is stale.
- Keeps `updated_at` meaning "the projection changed," not "someone asked."
- Behavior-neutral for the shape comparison; makes `Report.touched` meaningful.

## 3. Make projection pure — landed

Decisions: provider text is not copied onto the entity (identity descriptions would feed back into
`measure`), and platform presence is member-only (one slot cannot represent several identities).
A live entity also cannot detach its last member, so name and type always have a member source.

The impure part was the fallback to the entity's own previous values:

| column group | primary source | fallback before |
|---|---|---|
| `name`, `entity_type`, `alt_types` | members | previous entity row |
| `description`, `alt_descriptions` | selected member | previous entity row |
| `search_query`, `alt_search_queries` | selected member | previous entity row |
| `platform`, `platform_handle`, `platform_display_name` | selected member | previous entity row |
| `status`, `confidence` | identity rows | none |

The old fallback preserved **the last projection**, which could be a detached or merged-away member's
text or platform. After a detach, an entity could keep a description and a handle no current member
carried, and `reproject` faithfully preserved them. The corpus check found zero rows relying on that
behavior.

The pure rule is now:

- `description` / `search_query`: the selected member's, else `None`.
- `alt_descriptions` / `alt_search_queries`: members only.
- `platform_*`: the selected member's presence only, else `None`.
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

- **Per-entity N+1 — accepted.** Fine for a link batch; a resolver batch can be hundreds through
  `identify_all_pending` because it builds one mutation list and one `proc` transaction. Accepted
  until measurement shows it matters; a batched projector is the optimization if it does.
- **Status semantics — accepted.** `identified` means “any identity row,” including an illuminator
  handle. The attestation model may refine this; not worth changing in projection now.

## Verification

- [x] Concurrency test for the row lock.
- [x] Idempotence: `reproject --all` twice; the second run changes nothing.
- [x] Detached origin drops the origin's text and platform.
- [x] Last-member detach is refused.
- [x] `Scope::Stale` renamed to `Scope::Unidentified`, matching what it selects.
- [x] Dedupe policy explicit: surfaces fold, assertion text stays exact; key-aware selection keeps
  the origin spelling when two surfaces fold together.
- [x] Corpus check: 0 entity descriptions and 0 platforms were being kept alive only by the fallback
  (45 / 2,806 observations carry no description; 40 / 2,326 live entities have no member
  description).
- [ ] Re-run the canonical shape diff if the projection rule changes again.

## Files

- `src/knowledge/graph/project.rs`
- `src/knowledge/graph/graph.rs` (`proc`, `reproject`)
- `src/knowledge/blocker/index.rs` (fingerprint and write)
- `src/knowledge/canonical.rs` (verification)
