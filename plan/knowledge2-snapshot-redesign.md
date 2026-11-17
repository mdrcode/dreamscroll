# knowledge2 snapshot: migrate first, tighten the catalog later

Status: Active — Phase 1 implemented; Phase 2 deferred.

## Purpose

The snapshot code is the time-travel half of the graph tooling: it preserves a projection
before `reset` empties it, and `compare` answers what a linker change did to the partition.
It lived under `src/knowledge/snapshot/` (v1) but operates on tables both graphs share; Phase 1
moved it to `src/knowledge2/snapshot/`.

The goal is to move the working implementation into `knowledge2` without changing behavior,
then decide how much of its catalog machinery to replace. The storage strategy has earned its
place; the table-name discovery has not.

## Current implementation

Four files under `src/knowledge/snapshot/`:

- `record.rs` — `Snapshot { suffix }`; suffix is `%Y%m%dT%H%M%S%3f`; four archive table names:
  `knowledge_entities__<suffix>`, `knowledge_identities__<suffix>`,
  `knowledge_proximities__<suffix>`, `knowledge_entity_membership__<suffix>`. Copies with
  `CREATE TABLE AS SELECT`, not rename, because the live tables carry FKs and fixed index names.
- `reset.rs` — one transaction: copy four tables, unlink observations, delete blocking index,
  proximities, identities, entities; close the generation row with the snapshot suffix and counts;
  open the next generation; prune old archives. Empty graph creates no snapshot. Retention is
  `RETAIN_SNAPSHOTS = 10`, discovered by scanning `information_schema` and matching a historical
  `SNAPSHOT_FAMILIES` list plus a suffix regex.
- `compare.rs` — reads two membership tables and entity labels; reports unchanged/split/merged/
  moved/only-in by stable observation id. Quotes table identifiers; tests include a suffix with a
  quote.
- `mod.rs` — `reset`, `preview`, `compare`, `Snapshot`, `Kind`.

The membership table is the important derived artifact: `user_observed_entities.knowledge_entity_id`
is unlinked by reset, so the archive is the only record of which observations were grouped.

Consumers:

- CLI: `reset` (preview + `--yes`), `compare`.
- v1 tests: `proximity/tests`, `verdicts/tests` call `knowledge2::snapshot::reset`.
- `knowledge2/canonical` doc points at `knowledge2::snapshot`.
- Snapshot tests use the v1 `test_support::link::link_all` and v1 `KnowledgeGraph::merge`.

## Assessment

Keep:

- `CREATE TABLE AS SELECT`; do not rename live tables.
- Observations stay; reset only unlinks them.
- Explicit membership table; observation ids are the stable comparison key.
- One transaction around copy + clear; empty graph means no empty archive.
- Generation register recording snapshot suffix and counts.
- Compare by grouping, not entity ids.
- Excluding the blocking index (derived), verdicts (durable), and fetch cache.

Change:

1. **Use the generation register as the snapshot catalog.** `knowledge_generations` already stores
   `snapshot`, `entities`, `identities`, `proximities`, `members`, and `merges`. Pruning and
   validation should read it instead of scanning `information_schema` for every reset.
2. **Validate `Snapshot`.** `compare` currently accepts any suffix from argv and reads the matching
   tables. It quotes identifiers, but it cannot distinguish a real snapshot from a typo. The
   generation register can say whether a suffix exists.
3. **Make retention a policy, not a constant.** Keep the default 10, but make the count a CLI/config
   argument; prune by generation order.
4. **Decide what the archive is for.** `compare` uses only membership and entity labels. Identities
   and proximities are copied and counted but never read by code. If the archive is only for linker
   partition diffs, copy less; if it is time-travel, keep identities and document the manual use.
5. **Handle suffix collisions.** Two resets in one millisecond collide. Fail with a clear error or
   use a database-side unique suffix.
6. **Separate concerns.** `reset.rs` mixes snapshot creation, graph clearing, generation
   bookkeeping, retention, and preview. A `reset` module should compose smaller snapshot/store
   operations; this is optional if the file stays understandable.
7. **Remove the v1 dependency.** `reset` calls `knowledge::graph::current_generation`; the migrated
   module must read the generation from `knowledge2`.

If starting over, the strongest alternative is a registry plus snapshot-scoped rows:

```sql
knowledge_snapshots (id, suffix, generation, taken_at, entities, identities, members)
knowledge_snapshot_entities  (snapshot_id, entity_id, name, ...)
knowledge_snapshot_members   (snapshot_id, observation_id, knowledge_entity_id)
knowledge_snapshot_identities(snapshot_id, entity_id, provider, scheme, external_id, ...)
```

That removes dynamic DDL, table-name discovery, and per-snapshot pruning: compare joins snapshot
ids, retention deletes rows. It trades cheap `DROP TABLE` for a stable catalog. With ten snapshots,
the row model is preferable; with the current code already proven, use the generation register as
the first upgrade rather than a rewrite.

## Phase 1 — behavior-preserving move

1. `git mv src/knowledge/snapshot src/knowledge2/snapshot`; add `pub mod snapshot` to
   `knowledge2/mod.rs`.
2. Re-export v1 snapshot from `knowledge2` so existing callers keep working. The re-export was
   removed once the v1 tests were repointed to `knowledge2::snapshot::reset`.
3. Expose the existing v2 generation query as `knowledge2::graph::current_generation` and use it
   from `snapshot/reset.rs`; remove the v1 `knowledge::graph` dependency.
4. Update CLI `reset`/`compare` to `knowledge2::snapshot`.
5. Update `knowledge2/canonical` doc reference.
6. Adapt snapshot tests:
   - v1 `test_support::link::link_all` can remain test-only during the move, but prefer the v2
     `LinkHelper`.
   - The generation test's v1 `KnowledgeGraph::merge` becomes `GraphMutation::MergeEntity`.
   - v1 `proximity/tests` and `verdicts/tests` keep using the shim.
7. No schema changes, no table-name changes, no behavior changes.
8. Verify: full lib suite, clippy, both bins, and the snapshot tests. No live reset is required for
   the move.

## Phase 2 — catalog tightening (after the v1 cutover)

1. `prune_snapshots` lists non-null `knowledge_generations.snapshot` rows, keeps the newest N
   generations, and drops the archive tables for the rest. Keep the family list only for tables
   that predate the register.
2. Add `Snapshot::from_suffix(db, suffix) -> Result<Snapshot>` (or equivalent) that validates
   against the register; CLI `compare` uses it. Keep a test-only constructor for synthetic suffixes.
3. Make retention a parameter (default 10) from CLI/config.
4. Decide and document whether identities/proximities are archived. If they stay, add a read path
   or state that they are manual-inspection material; if not, stop copying them.
5. Optional: move to snapshot-scoped rows and a dedicated catalog if comparisons grow beyond
   membership. This is a separate migration with old-archive handling.

## Files

- `src/knowledge/snapshot/**` → `src/knowledge2/snapshot/**`
- `knowledge2/graph` (`current_generation`)
- `src/bin/dreamscroll_knowledge.rs` (`reset`, `compare`)
- `src/knowledge/mod.rs` (v1 shim)
- `src/knowledge2/canonical.rs` (doc path)
- Snapshot tests; v1 tests using the shim

## Verification

- `cargo test --lib` / snapshot filter: reset preserves the partition; empty reset; retention;
  generation register; compare split/merge; missing table; quoted suffix.
- Full suite, clippy, knowledge/web bins.
- If an end-to-end local run is desired: preview only, or reset with the approved backup/restore
  procedure; reset is destructive and assumes no concurrent graph writer.

## Open decisions

- Retention count and whether it is per-schema, per-user, or global.
- Whether the archive is only linker partitions or full graph time-travel.
- Whether proximities belong in the archive at all; identities are evidence and likely do.
- Whether old table-per-snapshot archives are migrated or left as legacy after Phase 2.
