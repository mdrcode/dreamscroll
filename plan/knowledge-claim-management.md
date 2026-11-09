# Claim management — assert and retract are one key

**Status:** Proposed — design agreed 2026-11-09, not implemented. Replaces the veto's addressing,
not the verdict log.

**See also:** [lexicon.md](lexicon.md) (vocabulary), [knowledge-verdicts.md](knowledge-verdicts.md)
(the log this revises), [knowledge-canonical-entities.md](knowledge-canonical-entities.md) (evidence
as an append-only edge), `src/knowledge/knowledge.md` (the model as built).

## The trigger

A veto records `(subject_origin, scheme, external_id)` — a **pairing**. `withdraw` then finds the
claim to retract with an extra predicate, `basis = 'text'`. That predicate is not part of the
pairing, and a census of the live graph (2026-11-09, 1,967 live claims) shows three populations:

| basis | `asserted_by` | live claims | written by | means |
| --- | --- | --- | --- | --- |
| `text` | `wikidata` | 1,595 | `store_claim` | "this entity is Q42", reached by title/label/search |
| `handle` | `platform` | 345 | `Evidence::record` | "the screenshot showed `@handle` on this entity" |
| `handle` | `wikidata` | 27 | `store_claim` | "this entity is Q42", reached **through** the handle (P2002) |

`basis` records the **route**, not the writer. The third population is an authority cross-reference
that happened to arrive through a handle, so `basis = 'text'` excludes it:

- `identity veto 245622 wikidata Q317521` (`Elon Musk`, method P2002) writes the standing decision,
  then `withdraw` retracts 0 rows. The claim stays live, the entity stays `identified`, and the CLI
  prints `withdrawn 0` — which reads as success.

The same mismatch has a second face: capture claims are exempt from vetoes **twice**. `withdraw`
cannot see them (they are always `handle`), and `Evidence::record` never consults `vetoed()` the way
`store_claim` does. The next `link` re-asserts them unconditionally.

So the code says "a veto is about authority cross-references" through two independent mechanisms —
one admission check on one writer, one basis filter on the retraction — and expresses it cleanly in
neither.

## What a claim is

`(holder, identity, basis, source)`:

- **holder** — the entity. Its durable name is the origin observation; entity ids are renumbered by
  every rebuild.
- **identity** — `(scheme, external_id)` is the durable natural key.
- **basis** — how the answer was reached (`text`, `handle`). Load-bearing for confidence: `combine`
  prices `text` + `handle` as two independent inputs, and a QID reached through a handle is
  deliberately *not* corroboration.
- **source** — who asserted it: `platform` for a capture, the provider id for an authority. In the
  row this is `asserted_by` on the identity plus `user_observed_entity_id` on the claim.

`knowledge_claims` enforces **one live claim per `(holder, identity)`** (unique where
`retracted_at IS NULL`). That constraint is why the pairing and the claim coincide today: there is
nothing for a basis filter to choose between.

## The model: decision vs derived state

Two layers, and only one of them needs to survive a reset:

| | object | survives reset | role |
| --- | --- | --- | --- |
| claim row + `retracted_at` | a row | no | materialised state, re-derived every build |
| verdict row | the pairing | yes | the decision, re-applied at admission |

That distinction gives the two operations that look alike and are not:

- **retract** — now, this generation, on a claim that exists: set `retracted_at` on the row.
- **suppress** — every build, on a claim that does not exist yet: refuse the insert.

Suppression is what makes a veto standing, and it runs where there is no row to name. A decision
addressed by claim id therefore cannot do it. That settles the row-id question:

> **No claim identity is needed across resets, because the claim itself does not persist.**
> Claims are deleted and re-derived; a row id points at nothing (or at a different edge) after one
> reset, while the decision must still suppress the new claim. The durable name is the pairing.

A row id is a *within-generation locator*, and within a generation it is redundant: the pairing
already selects at most one live row. Storing it would add a second address that goes stale, and
would not help the path that makes a veto survive.

The one case that would make ids coherent is a graph that mutates in place — claims kept, only new
ones inserted, retractions accumulating on rows. That is not this graph: `reset` empties the
projection precisely so it can be re-derived from observations and the cache.

## The rules

1. **One key.** A veto names `(holder_origin, scheme, external_id)` — already what the verdict row
   stores. No new columns.
2. **One predicate.** `vetoed()` is the rule: "no claim for this pairing is live." It is asked by
   *both* writers (`store_claim` and `Evidence::record`) and by `withdraw`, so admission and
   retraction cannot disagree.
3. **Retraction is materialisation.** `withdraw` resolves the key to at most one live row — the
   unique index guarantees it — and sets `retracted_at`. No basis filter, no writer filter.
4. **A miss is reported, not counted as a withdrawal.** `gone` when the subject no longer names a
   live entity; `settled` when the decision holds but there is no live claim to retract (already
   retracted, or never written because admission suppressed it). `withdrawn` counts rows.
5. **`basis` stays a scoring input.** It is never a veto filter. If a conditional veto ("distrust
   the authority's answer, keep the capture's assertion") is ever wanted, the key grows by
   `(basis, source)` and the verdict says so — still not an id.
6. **No claim ids in the log.** They cannot survive a reset and add nothing within one.

## Changes

| file | change |
| --- | --- |
| `src/knowledge/graph/verdicts.rs` | `withdraw` drops the `Basis::Text` filter; a zero-row retraction becomes `settled`, not `withdrawn 0`. `ReplayReport` gains `settled`. |
| `src/knowledge/evidence/store.rs` | `Evidence::record` takes the veto set and returns `Stored`, like `store_claim`. It derives the key from the observation's `PlatformLink`. |
| `src/knowledge/graph/link.rs` | load `verdicts::vetoed()` once per illumination; pass it to `record`; `LinkStats` gains `vetoed`. |
| `src/bin/dreamscroll_knowledge.rs` | `link` prints `vetoed`; `verdicts replay` prints `settled`. |
| `src/knowledge/knowledge.md` | the veto paragraph and the `LinkStats` snippet. |

No schema change. Existing verdict rows already carry the key; nothing to backfill.

## Tests

- A veto on an entity whose only claim is P2002/`handle` retracts it: `withdrawn == 1`, `status`
  back to provisional, `confidence` recomputed.
- A veto on a capture assertion retracts it too (the exemption removed).
- The next `link` does not re-assert a vetoed platform pairing: `record` returns `Vetoed`,
  `LinkStats.vetoed` counts it, and the claim is absent.
- A veto whose claim is already gone reports `settled`, not `withdrawn 0`.
- A veto whose subject origin no longer names a live entity reports `gone`.
- Negative controls: without the admission check, the capture claim returns on the next link;
  without the filter removal, the P2002 claim survives.

## Consequences

- The 27 authority-handle claims become vetoable; so do the 345 capture claims, which is the point
  of naming a pairing.
- A vetoed pairing stays `provisional` after a rebuild, because the claim is never written and
  `withdraw` has nothing left to do. `resolve` already reports this in its `vetoed` bucket (5 in the
  current corpus).
- `verdicts replay`'s output becomes explicable: `applied/already/gone/refused` for merges,
  `withdrawn/settled/gone` for vetoes.

## Deliberately not doing

- **Claim ids.** Above: they cannot survive a reset, and the pairing is already unique.
- **Claim-level (conditional) vetoes.** With one claim per pair, "distrust the authority but keep the
  capture" would leave the pair asserted and change only the corroboration bonus — a 0.1 effect for
  a second address. If a future schema allows several claims per pair, revisit.
- **An event log.** Assert and retract are already symmetric at the row level (`insert` /
  `retracted_at`). The verdict table is the durable reason; a full event-sourced log is a larger
  change with no current requirement.
- **Per-generation retractions.** The `generation` column exists, but a veto is deliberately not
  generation-scoped: it is a standing decision, not a historical one.

## Open questions

- Naming: is `settled` the right word (alternative: `already`)? Does `ReplayReport.gone` need to
  distinguish "subject entity gone" from "identity row gone"?
- Should `Kind::Veto` be renamed `Reject` so the log says what it means — rejecting a pairing — and
  so nothing invites a `basis` field back onto it?
