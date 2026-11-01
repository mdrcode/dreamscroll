# Illumination history — the graph keeps every reading, the view keeps one

**Status:** Implemented (2026-11-01). `link` links every observation, including those from
superseded illuminations; the user's view is anchored on each capture's current illumination. The
change was triggered by a measured failure, not a hunch.

**See also:** [lexicon.md](lexicon.md) (vocabulary), [knowledge-canonical-entities.md](knowledge-canonical-entities.md)
(the model).

## The trigger

Capture 683 is a still from *Apocalypse Now* — Martin Sheen as Captain Willard, glass in hand. It was
illuminated twice:

| illumination | reading |
| --- | --- |
| 4 | Apocalypse Now · Martin Sheen · Captain Willard · Francis Ford Coppola |
| 7 | Wake in Fright · Gary Bond · John Grant · Ted Kotcheff · Kenneth Cook |

The second reading is wrong. *Wake in Fright* (Kotcheff, 1971) also opens with a man drinking alone
in a drab room, so the model substituted one film for another and then produced a consistent, real
cast list around it. `resolve` then attached genuine Wikidata QIDs to all five entities, and the
graph held them as `identified` with two members each.

Before this change `link` took only the capture's latest illumination. So the wrong reading became
*the* reading, the correct one was never linked, and the graph gained five well-substantiated
entities that are not in the picture — while `Francis Ford Coppola` did not exist at all.

Two properties made this invisible to every check we had:

- the failure was **self-consistent** — real film, real cast, real QIDs;
- it happened **twice** (the same image was uploaded on the 16th and the 19th and misread both
  times), which manufactured the appearance of corroboration.

## The rule

1. **A capture has exactly one current illumination** — the highest id, because illuminations are
   only ever appended. Nothing stores the flag; it is derived, so it cannot drift.
2. **The graph holds every reading.** `link` links all observations. An illumination can be wrong,
   and the correct reading is often the older one.
3. **The user's view is a left join from the current illumination into knowledge entities.** The
   view starts at the observation, never at the entity.

Rule 3 is what makes rule 2 safe. Ghost entities — those whose members are all superseded — cannot
surface, because nothing queries knowledge entities to decide what a user sees. The left join also
means unlinked observations still render: the capture card shows the observation's own name and
type, and entity enrichment (canonical name, QID) must remain `Option` so an unlinked observation
does not vanish between capture and `link`.

## What changed

| Where | Change |
| --- | --- |
| `knowledge::link` | no longer skips superseded illuminations; counts them as `stale` in the report |
| `knowledge::link`, `knowledge::merge` | the one-extraction guard is scoped to `illumination_id`, not `capture_id` |
| `knowledge::current` | new: the current illumination per capture, for one user (`DISTINCT ON`) |
| `api::user::get_entity` | both reads scoped through `knowledge::current` |

`get_knowledge_entities_by_type` filters `IlluminationId.is_in(current)`; `get_knowledge_entity`
rejects an id whose illumination is not current.

### Why the guard moved to the illumination

The old rule was: two cores appearing in one *capture* cannot be one referent, because a single
extraction emitting the same string twice means two things. That reasoning is about one extraction,
not one capture. With history linked, capture-scoped would have refused exactly the merges worth
making — two readings of one image naming the same referent — and refused them permanently.

## What it cost — measured after a clean rebuild

Reset, then `link` + `resolve --offline` over the 2,533 observations, with the new rules:

| | before (snapshot 2026-10-31) | after |
| --- | --- | --- |
| entities | 2,075 | 2,117 (2,104 live, 13 merged) |
| duplicated names | 10 | **7** |
| authority claims | 1,471 | 1,472 |
| ghost entities (every member superseded) | — | 25 |

Duplication went **down**. Two reasons: history gives an observation more chances to attach to the
entity it belongs to, and scoping the one-extraction guard to the illumination unblocks merges the
capture-scoped rule had refused permanently. The 7 survivors are pairs resolution cannot identify —
a novella and a film both titled *Train Dreams* is the canonical one — and they predate this change.

### The measurement that nearly misled us

Applying the change **incrementally** to the existing graph produced +125 entities and 106 duplicated
names, and a following `resolve --offline` folded 88 of them, leaving 19. That looked like the cost
of keeping history; it was an artefact of mixing rule generations, because the old graph had been
built under the old rule and newly linked stale observations matched their counterparts poorly.

**Measure rule changes from a rebuild, never from an increment.** `reset --yes` keeps the
observations and drops everything derived, so the rebuild is cheap: 12 seconds for `link`, 9 for
`resolve --offline`, no network.

## Still open

- **There is no arbiter for which reading is right.** `current` is `max(illumination_id)` — recency,
  not quality, and nothing in the system pretends otherwise. In the three re-reads examined closely
  the *newest* reading was the worse one every time (captures 683, 697, 684). Only a user can say a
  reading is right, and a correction needs somewhere to live: today the sole way to change what the
  graph believes is to produce another reading. A capture-level pointer to the chosen illumination is
  the natural home for that decision, and is deliberately deferred. It would also be the first source
  of labelled data — every pinned reading says which run was correct, which is the only way to ever
  answer whether re-illumination helps or hurts.
- **Retries still destroy history.** `insert_illumination` upserts the illumination row by
  `task_run_id` and then deletes and re-inserts its observations, so a retry of the same run
  discards the previous attempt before `link` sees it. Re-illumination from a *new* task run is
  unaffected. Making retries append is the supersede change, and the only part of this that touches
  ingest.
- **Ghost set is not exposed.** "Every member of this entity is superseded" is a one-line query and
  a decent drift signal; nothing surfaces it yet.
- **Corroboration** already counts independent *inputs* (text vs handle) rather than observations,
  so re-reads do not inflate confidence. If a member count is ever used as evidence, it must count
  distinct captures.

## Verification

- `cargo test --lib` — 372 passing, including `a_superseded_illumination_is_linked_too` and
  `only_the_current_reading_is_visible`.
- Live: after the change, `link` on the local corpus links both readings of captures 683 and 697.
