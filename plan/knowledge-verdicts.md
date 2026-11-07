# Rebuilding the graph without losing what a person decided

**Status:** Built 2026-11-05 — the proximity batch (`9407182`), the generation register, one
modification log (`knowledge_verdicts`), and `verdicts replay` / `verdicts list`. Second kind built
2026-11-07: **`veto`**, an entity is not an identity. `distinct` (a rejected proximity pair) is
deliberately not built; see *Decisions*.

## The two commands

```
dreamscroll_knowledge verdicts replay
dreamscroll_knowledge verdicts list [--generation N] [--cause repair|proximity_auto|replayed] [--limit N]
```

`replay` prints `applied · already · gone · refused · withdrawn`; `gone` drifting upward across rebuilds is the
signal that linking stopped being deterministic, before anything is lost.

`list` reads newest first. It is the viewer the old `knowledge_merges` table never had, which is part
of why that table read as dead weight.

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
| "fold these two entities" (`merge -y`, `cause = repair`) | `knowledge_merges` row (since folded into `knowledge_verdicts`) | the row named two entity ids, and both rows were deleted |
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

## Built: one log

`knowledge_verdicts` is the **one modification log**: every fold, whoever made it, and nothing else.
`knowledge_merges` — the old per-generation audit — is gone; keeping two tables with different
lifetimes made "which one is the log?" a question nobody could answer from the names.

```sql
knowledge_verdicts (
  id            serial primary key,
  generation    int not null,       -- when it happened
  cause         text not null,      -- repair | proximity_auto | replayed
  kind          text not null default 'merge',  -- merge | veto
  winner_origin int,                -- merge only; nullable since schema 29
  loser_origin  int,                -- merge only
  subject_origin       int,         -- veto: the entity, by origin_observation_id
  identity_scheme      text,        -- veto: the identity's natural key —
  identity_external_id text,        -- a rebuild recreates identity rows, so their ids are unstable
  winner_label  text, loser_label text,
  decided_at    timestamptz not null default now(),
  note          text
);

knowledge_generations (
  id            serial primary key,
  started_at    timestamptz not null default now(),
  superseded_at timestamptz,        -- reset: snapshotted and emptied at this moment
  snapshot      text,               -- archive suffix; null when the graph was empty
  entities int, identities int, claims int, proximities int, members int
);
```

`cause` carries the authority, which is the only thing that differs between the three:

| cause | who | lifetime |
| --- | --- | --- |
| `repair` | a person | forever; **replay re-applies it in every later generation** |
| `proximity_auto` | the automatic pass | logged for the generation it happened in, never replayed — the model's opinion is re-derived |
| `replayed` | the replay, acting for a person | logged per generation, so "what changed in generation N" is complete |

Direction is in the column names, not a `survivor` field.

- **`reset` becomes the generation manager**: it closes the current generation (writing
  `superseded_at`, the snapshot suffix and the counts onto that row), then opens the next. At most
  one snapshot per generation, because reset is the only snapshotter and it empties as it copies.
  Snapshot metadata therefore needs no table of its own.
- **Verdicts are authored, not derived**: reset never deletes them. `merge -y` with `cause = repair`
  writes a `same` verdict; the survivor side is recorded because the survivor is what keeps the
  name, the origin and the projected columns.
- The schema was `kind`-shaped from the start; `veto` is the second kind that used it (below).
  `distinct` remains unbuilt, and its semantics are not settled — see *Open questions*.

## Built: replay

`dreamscroll_knowledge verdicts replay` — a separate command that `resolve` knows nothing about; it
walks the log and applies what still maps. Newest verdict per pair wins; chains still apply oldest
first.

```
for each verdict:
  left  = the live entity whose origin_observation_id is left_origin   (else skip: "gone")
  right = the same for right_origin
  same:     left == right        → no-op ("already one")
            merge the loser into the survivor recorded in the verdict
            → applied | skipped ("refused: shared illumination")
  distinct: left == right        → contradiction: the graph folded a pair a person called different
            otherwise            → no-op; nothing to write
  veto:     subject = live entity with this subject_origin    (else skip: "gone")
            retract the live claims (basis = text) whose identity is scheme:external_id
            → "withdrawn" (a count: a pair can be re-claimed and withdrawn again)
```

- Idempotent by construction: applying it twice is a no-op the second time.
- Applies verdicts of any age. There is no "previous generation only" rule; the generation on a
  verdict is provenance, not a gate.
- Logs the folds it applies as `cause = replayed`, so the log stays a truthful record of what
  changed in each generation.
- Follows tombstones on **both** sides before deciding: if the model folded the person's winner into
  something else, the decision still means "these two are one" about wherever they are now.
- Replay touches the graph only — entities, observations, verdicts. It never reads
  `knowledge_proximities`; the batch is a candidate source for the automatic pass, not a durable
  artefact.

`proximity list` showing "a person has decided this pair" is a separate concern: for each candidate
row it resolves the two sides' origins and asks the verdict log. That matching belongs to
presentation and is not shared with replay.

## Built: a second kind — `veto` (2026-11-07)

A person can say **an entity is not an identity**. The claim is withdrawn, kept out, and the item
stays valid everywhere else. This is not `distinct`: a veto is about one entity's cross-reference to
an authority item, not about a proximity pair.

```
dreamscroll_knowledge identity veto --entity 221194 --identity Q131472411 \
                   [--scheme wikidata] [--note "a short fiction, not the being"]
dreamscroll_knowledge identity unveto <verdict id>
```

Why it cannot be a flag on the claim row, which is the obvious cheaper shape: claims are derived and
append-only. A flag on row 900 does not gate the row that the next resolve writes for the same pair,
and there will always be one, because the parser's decision has not changed. Only a decision keyed by
what survives derivation — entity origin, identity natural key — can gate every future row.

Three consequences that differ from the merge story:

- **Admission, not only replay.** `store_claim` asks a preloaded veto set before writing, so a vetoed
  claim is never made; replay withdraws any that predate the veto. This is the one place `resolve`
  knows about verdicts, and it has to: unlike a fold, which a scan can refuse to consider, a claim
  has exactly one writer.
- **Basis `handle` is out of scope.** A veto withdraws a cross-reference. A capture's own account is
  what the source showed, not a cross-reference.
- **Lifting is deletion.** `unveto` removes the row and restores nothing; claims are append-only and
  the partial unique index (`WHERE retracted_at IS NULL`) lets the next resolve insert a fresh row.

First use, 2026-11-07: `Q` (the Star Trek being) out of `Q131472411` ("2004 short fiction by John
Grant"), `Los sorias` out of `Q122707294` ("1998 edition, published by Simurg"). Both withdrew one
claim, both entities fell back to `provisional`, and the next `resolve --offline` reported
`vetoed 2` with **no** claims written — the admission path, not a replay aftermath.

## Failure modes, all reported rather than guessed

| situation | outcome |
| --- | --- |
| a side's origin no longer names a live entity | skip — "gone"; the thing was rebuilt from a different first observation |
| both sides resolve to one entity | no-op — already folded, by the model or by an earlier replay |
| the automatic pass folded the pair with the *other* survivor | no-op — same partition, different survivor; the recorded direction is not enforced |
| `merge` refuses (shared illumination) | skip with the reason; the pair stays open |
| a veto's subject origin no longer names a live entity | skip — "gone", counted like any erosion |
| a person's merge was recorded, then the pair was folded the other way round automatically | no-op — same partition; the recorded direction is not enforced on an already-folded pair |

## Tolerated

- **Replay is manual** — see the `pragmatism.md` row. A plain `resolve` neither honours nor repeats a
  person's *folds*: the log keeps them waiting and the pairs reappear as candidates. Vetoes are the
  exception — honoured at admission, with replay only as the backstop for older claims.
- **`replayed` rows grow** by one per verdict per generation. Fine while rebuilds are rare; a dev
  database is disposable, and a frequent prod rebuild is the real problem.

## Decisions (2026-11-05)

1. **Replay is a separate command — for merges.** A plain `resolve` may still fold a pair that a
   future `distinct` would have prevented. **Vetoes are the exception and must be**: admission is
   asked inside `store_claim`, because a claim has exactly one writer while a fold has a scan that
   can filter. Without it a vetoed claim returns on the next resolve and the decision is decorative.
2. **`distinct` is not built.** The convenience is deferred until the queue is actually annoying.
   When it comes back, the cheapest shape is a filter in `scan` — a decided-distinct pair never
   enters the batch — which keeps `merge` and `replay` both verdict-free.
3. **Latest by id wins** for a repeated pair. Append-only log; `replay` keeps the newest verdict per
   unordered origin pair before applying, so direction is whatever was decided last.
4. **Contradictions are not policed at write time** — with one verdict kind there is nothing to
   contradict; rule 3 covers it once `distinct` exists.
5. **Validation builds are unaffected**: replay is always explicit, so `reset → link → resolve →
   compare` measures the model alone.

Verified end to end: a manual merge records a verdict (`cause = repair`), a `reset` keeps it, and
replay folds the rebuilt pair by origin — `a_manual_merge_records_a_verdict`,
`replay_folds_the_pair_again`, `replay_is_idempotent`, `replay_reports_sides_that_are_gone`,
`an_identity_merge_records_nothing`.

First real run, 2026-11-05: eight manual merges recorded (five same-referent pairs, and three
aspect pairs resolved toward place/organization), then `reset` → `link` → `resolve --offline` →
`verdicts replay` reported **applied 8 · already 0 · gone 0 · refused 0**. Every survivor came back
with the other side's name in `alt_names`, and the candidate batch fell from 21 to 13 — the eight
pairs never reappear.

## Non-goals

- Incremental rebuilds (keeping entity ids stable instead of renumbering) — a different
  architecture, and the thing that would make observation-derived keys unnecessary.
- Undo/split. A merge stays irreversible; verdicts prevent folds, they do not reverse them.
- Replaying model decisions (`cause = identity_conflict`). Those are re-derived every build and
  deliberately not recorded as intent.
