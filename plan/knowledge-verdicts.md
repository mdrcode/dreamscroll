# Rebuilding the graph without losing what a person decided

**Status:** Partly implemented — the proximity batch shipped 2026-11-05 (`9407182`); verdicts,
generations and replay are designed here and not yet built.

> **See also:** `plan/knowledge-canonical-entities.md` (observation → entity → evidence),
> `plan/knowledge-resolution.md` (claims from authorities), `src/knowledge/knowledge.md` (the
> modules as they stand), `plan/lexicon.md` (the vocabulary).

## The problem

`reset` is destructive by design: it snapshots the projected graph, then empties it. Rebuilding is
`link` → `resolve --offline`, seconds, no transport. What survives: **observations, illuminations,
captures, the resolution cache.** What does not: entities, identities, claims, merges, proximities.

Two of those are not the model's output — they are a person's judgement:

| decision | today | lost because |
| --- | --- | --- |
| "fold these two entities" (`merge -y`, `cause = repair`) | `knowledge_merges` row | the row names two entity ids, and both rows are deleted |
| "these two are different" (`distinct`) | `knowledge_proximities.verdict` | same, plus the table itself is now replaced every scan |

After a rebuild the graph is the model's opinion again, so a person re-does the same work every time,
and worse, the model can quietly re-merge something they had rejected.

## What a durable decision has to be keyed by

Entity ids are generation-scoped — the sequence advances, so `190995` means nothing after a reset.
A decision that outlives a rebuild therefore needs a name for each side that survives it.

**Decision: `knowledge_entities.origin_observation_id`.** It is already there, set once when the
first observation creates an entity, never rewritten, and observation ids are never reused. It needs
no new column:

- `link` processes illuminations ordered by id (`link_all_pending` orders explicitly), and
  observations within an illumination by `(position, id)`, so "the first observation that created
  this entity" is a deterministic function of the corpus.
- Measured 2026-11-05, across two rebuild cycles on the same corpus: **2134 of 2134 entity groups
  kept the same origin** — zero drift.

A uuid on observations was considered and deferred. The integer already has every property that
matters (unique, immutable, never reused); a string key earns its place only when keys must travel
between databases (dev → prod export, merging two graphs), and then it is a one-off backfill over
rows that never change. Human-friendliness is a display problem, not a storage one.

Erosion is accepted: if a rebuild builds "the same thing" from a different observation, the decision
no longer applies and replay says so. See *Failure modes*.

## Shipped: proximities are one scan's output (2026-11-05)

The table used to be a ledger — unique pair index, score never recomputed, `verdict`/`settled_at`
mutated in place. Now:

- `scan` deletes and re-inserts the derived set in one transaction; the table always means "the
  candidates as of the last scan".
- `merge` reads the batch, folds what qualifies, leaves the rest. Rows are not deleted; the next
  scan replaces them.
- Removed with it: `verdict`, `settled_at`, `raised_at` (now `scanned_at`), the unique and partial
  indexes, `proximity::settle`, the CLI `distinct` command, the settlement write inside `merge`,
  `stats.proximities_settled`. Schema 26.

`distinct` is not coming back as a status on a derived row. If it returns, it returns as a durable
verdict kind (below).

## Designed: tables

```sql
knowledge_generations (
  id            serial primary key,
  started_at    timestamptz not null default now(),
  superseded_at timestamptz,        -- reset: snapshotted and emptied at this moment
  snapshot      text,               -- archive-table suffix; null when the graph was empty
  entities int, identities int, claims int, merges int, proximities int, members int
);

knowledge_verdicts (
  id            serial primary key,
  generation    int not null references knowledge_generations(id),
  kind          text not null check (kind in ('same', 'distinct')),
  left_origin   int not null,       -- origin_observation_id of that side
  right_origin  int not null,
  left_label    text, right_label text,
  survivor      text check (survivor in ('left', 'right')),
  decided_at    timestamptz not null default now(),
  note          text,
  check ((kind = 'same') = (survivor is not null))
);
```

- **`reset` becomes the generation manager**: it closes the current generation (writing
  `superseded_at`, the snapshot suffix and the counts onto that row), then opens the next. At most
  one snapshot per generation, because reset is the only snapshotter and it empties as it copies.
  Snapshot metadata therefore needs no table of its own.
- **Verdicts are authored, not derived**: reset never deletes them. `merge -y` with `cause = repair`
  writes a `same` verdict; the survivor side is recorded because the survivor is what keeps the
  name, the origin and the projected columns.
- The schema is `kind`-shaped from the start so `distinct` can arrive as a second kind without a
  migration. It is not implemented, and its semantics are not settled — see *Open questions*.

## Designed: replay

A command (`verdicts replay`, or whatever the CLI naming pass lands on) that walks the log oldest
first and applies what still maps:

```
for each verdict:
  left  = the live entity whose origin_observation_id is left_origin   (else skip: "gone")
  right = the same for right_origin
  same:     left == right        → no-op ("already one")
            merge the loser into the survivor recorded in the verdict
            → applied | skipped ("refused: shared illumination")
  distinct: left == right        → contradiction: the graph folded a pair a person called different
            otherwise            → no-op; nothing to write
```

- Idempotent by construction: applying it twice is a no-op the second time.
- Applies verdicts of any age. There is no "previous generation only" rule; the generation on a
  verdict is provenance, not a gate.
- Tags the merges it applies `cause = repair`, `source = user`, so `knowledge_merges` stays a
  truthful per-generation audit of what happened and why.
- Replay touches the graph only — entities, observations, verdicts. It never reads
  `knowledge_proximities`; the batch is a candidate source for the automatic pass, not a durable
  artefact.

`proximity list` showing "a person has decided this pair" is a separate concern: for each candidate
row it resolves the two sides' origins and asks the verdict log. That matching belongs to
presentation and is not shared with replay.

## Failure modes, all reported rather than guessed

| situation | outcome |
| --- | --- |
| a side's origin no longer names a live entity | skip — "gone"; the thing was rebuilt from a different first observation |
| both sides resolve to one entity | no-op — already folded, by the model or by an earlier replay |
| the automatic pass folded the pair with the *other* survivor | no-op — same partition, different survivor; the recorded direction is not enforced |
| `merge` refuses (shared illumination) | skip with the reason; the pair stays open |
| `kind = distinct` and the graph already folded them | contradiction, reported loudly; there is no split to undo it |

## Open questions

1. **Ordering**: does `resolve` call replay between its scan and merge steps (so a single run leaves
   the graph consistent and honours verdicts), or is replay always a separate command? Replay after
   `merge` means the automatic pass may fold a pair a person called distinct before replay ever runs.
2. **`distinct` semantics, if it returns.** As a durable verdict it has to be consulted by the
   automatic merge pass — which contradicts "replay never touches proximities" only on paper: the
   pass would ask the verdict log, not the other way round. Cheap (array/`origin` containment over a
   handful of pairs), but it is a second place that knows what a verdict is. Deferred until someone
   actually wants the convenience.
3. **Repeat verdicts on one pair**: last-wins, or refuse a second verdict that contradicts an
   existing one and make the person retract first?
4. **Contradiction handling** for a `same` verdict recorded while a `distinct` exists for overlapping
   origins.
5. **Validation builds.** After a replay the graph is intentionally not a clean build, so the
   `reset → link → resolve → compare` loop must run before replay (or with it disabled). Worth a flag
   or a line in the smoke procedure.

## Non-goals

- Incremental rebuilds (keeping entity ids stable instead of renumbering) — a different
  architecture, and the thing that would make observation-derived keys unnecessary.
- Undo/split. A merge stays irreversible; verdicts prevent folds, they do not reverse them.
- Replaying model decisions (`cause = identity_conflict`). Those are re-derived every build and
  deliberately not recorded as intent.
