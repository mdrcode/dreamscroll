# Claim management — one identity phase, one claim phase

**Status:** Implemented 2026-11-09. A claim is asserted in two phases, both callers use both, and a
veto names a pairing. Supersedes the veto's addressing in
[knowledge-verdicts.md](knowledge-verdicts.md).

**See also:** [lexicon.md](lexicon.md) (vocabulary), [knowledge-canonical-entities.md](knowledge-canonical-entities.md)
(evidence as an append-only edge), `src/knowledge/knowledge.md` (the model as built).

## The trigger

A veto records `(subject_origin, scheme, external_id)` — a **pairing** — and `withdraw` found the
claim to retract with an extra predicate, `basis = 'text'`. `basis` records the **route**, not the
writer, and a census of the live graph (2026-11-09, 1,967 live claims) showed three populations:

| basis | `asserted_by` | live claims | written by | means |
| --- | --- | --- | --- | --- |
| `text` | `wikidata` | 1,595 | `store_claim` | "this entity is Q42", reached by title/label/search |
| `handle` | `platform` | 345 | `Evidence::record` | "the screenshot showed `@handle` on this entity" |
| `handle` | `wikidata` | 27 | `store_claim` | "this entity is Q42", reached **through** the handle (P2002) |

The third population is an authority cross-reference that arrived through a handle, so
`basis = 'text'` excluded it: `identity veto 245622 wikidata Q317521` (`Elon Musk`, P2002) wrote the
decision, retracted **0** rows, and printed `withdrawn 0` — which reads as success.

The same mismatch had a second face: capture claims were exempt from vetoes twice. `withdraw` could
not see them (they are always `handle`), and `Evidence::record` never consulted `vetoed()` the way
`store_claim` did, so the next `link` re-asserted them.

## Why there were two mechanisms

Not a design: a protocol artefact. `Decision` carried an identity **id**, so the row had to exist
before the decider ran — `import_identities` was the first phase of the capture path. The authority
path needed no pre-pass because the engine held the answer, so it did identity upsert and edge insert
in one function. The claim row also has capture-only columns (`surface`,
`user_observed_entity_id`, `source_user_id`), so the capture path grew its own insert.

Those functions then drifted: the veto was in one, the uniqueness match duplicated, the basis
hardcoded in one, and the identity-upsert precedence was decided by execution order rather than by a
rule (`method` is "newest non-null wins" while its doc says "first answer only" — a capture that
arrives after an answer downgrades `method` to `capture`).

## The model

**Two phases, one implementation of each.**

1. **Ingest** — settle identity rows.
   - `Evidence::ingest(observations)` for a capture: every platform link becomes an identity, and
     the ids come back by durable name. **Create only.**
   - `Evidence::upsert_identity(draft)` for an answer: fills gaps and may upgrade `asserted_by`.
     **Enrich.**
   - The rule: *a capture may create an identity; only an answer may enrich one.* A screenshot knows
     a handle exists, not what the thing is.
2. **Assert** — write the edge (`Evidence::assert(ClaimDraft, vetoes)`).
   - Asks the veto once, at the one place a claim becomes a row.
   - Provenance is a value, not a code path: `ClaimDraft::capture(holder, observation, key, id)`
     carries the observation and the surface; `ClaimDraft::authority(holder, key, id, basis)` carries
     neither. `basis` is a field either way.
   - Idempotent: a unique violation is `AlreadyClaimed`, not an error.

**`link_illumination` is four steps.**

```
ingest     identities for every observation of the illumination, deduped by name
decide     all decisions, against the corpus as it stood before any of these writes
attach     create or attach each pending observation
assert     every observation with a handle asserts it against wherever it now lives,
           then project each touched entity once
```

The claim phase reads every observation of the illumination, not only the pending ones. The insert is
idempotent, so this repairs a link that died between attach and assert; today `pending` would be
empty and the claim lost forever.

## Why not claim ids

A claim row is derived: `reset` deletes claims and `link`/`resolve` re-create them with new ids. A
decision addressed by claim id points at nothing (or a different edge) after one reset, while it
still has to **suppress** the new claim — and suppression runs at admission, where no row exists. The
durable name is the pairing, which the verdict already stores. A row id is a within-generation
locator, and within a generation it is redundant: the unique index makes the pairing select at most
one live row.

## The veto rule

- A veto names a **pairing**. `withdraw` retracts that pair's live claim whatever route produced it,
  then reprojects the entity — status and confidence are functions of the live claims.
- A retraction with nothing to retract is **`settled`**, not `withdrawn 0`: the decision holds and
  says so. `gone` means the subject origin no longer names a live entity.
- The same predicate (`verdicts::vetoed`) is asked by `assert`, so admission and retraction cannot
  disagree.

## What changed

| file | change |
| --- | --- |
| `evidence/store.rs` | `IdentityKey`; `DraftIdentity::{key, from_claim}`; `ingest` (create-only) and `create_identity`; `ClaimDraft` + `assert` (veto + edge). Deleted `record` and `store_claim`. |
| `evidence/mod.rs` | the "two writers" paragraph became the two phases; exports updated. |
| `graph/link.rs` | four phases; `LinkStats.vetoed`; projection once per touched entity, after the claims. |
| `linker/mod.rs`, `linker/heuristic/linker.rs` | `Decision` no longer carries an identity — it is observation data, not a decision. |
| `identity/engine.rs` | `upsert_identity` + `assert`, the same two calls the capture path makes. |
| `graph/verdicts.rs` | `withdraw` drops the basis filter; `ReplayReport.settled`. |
| `bin/dreamscroll_knowledge.rs` | `link` prints `vetoed`; `verdicts replay` prints `settled`. |
| `knowledge.md` | the evidence row, the veto row, `LinkStats`. |

No schema change. Existing verdict rows already carry the key; nothing to backfill.

## Tests

- A capture claim is suppressed by a standing veto at the next `link` (`LinkStats.vetoed`).
- A veto on a P2002/`handle` authority claim retracts it, and the entity returns to provisional.
- A repeat replay reports `settled`, not a zero withdrawal.
- A later link restores a claim that went missing while the observation stayed linked.
- Capture ingestion never downgrades an answered identity (method stays `P2002`, label kept).
- Negative controls: without the veto check the capture claim returns; with `upsert_identity` in
  `ingest` the method becomes `capture`; with the old basis filter the P2002 claim survives.

## Measured result

A full rebuild on the local corpus after the change:

```
link      considered 2677 · already linked 0 · linked 453 · created 2224 · left pending 0 · vetoed 0
replay    applied 7 · already 0 · gone 1 · refused 0 · withdrawn 0 · settled 8
resolve   2217 candidates · 6424 requested · 3259 cache hits · 1582 claims · 5 vetoed
scan      10      merge  1 auto, 9 undecided      index  2216 rows, current
stats     2216 entities · 1858 identified · 358 provisional · duplicate rate 0.172
```

`link` produced exactly the same 2,224 entities and 453 attaches as the previous build: the refactor
changed no linking decision. The claim count fell from 1,622 to 1,581 because the answer cache aged
between builds — expired rows rose by exactly the 2,523 that left "with candidates" and "no match"
(13,363 − 10,840 = 2,523 = 783 + 1,740). That is evidence loss, not a code path.

## Deliberately not doing

- **Claim ids.** Above.
- **Conditional vetoes** ("distrust the authority's answer, keep the capture's"). With one claim per
  pair it would leave the pair asserted and change only the corroboration bonus. If a future schema
  allows several claims per pair, the key grows by `(basis, source)` — still not an id.
- **An event log.** Assert and retract are symmetric at the row level (`insert` / `retracted_at`);
  the verdict table is the durable reason.
- **Per-generation retractions.** The `generation` column exists, but a veto is a standing decision.

## Open questions

- Is `settled` the right word? Does `ReplayReport.gone` need to separate "subject entity gone" from
  "identity row gone"?
- Should `Kind::Veto` be renamed `Reject`, so the log says what it means and nothing invites a
  `basis` field back onto it?
- `link_illumination` is not atomic. A crash between attach and assert is repaired by the next link
  (the claim phase reads every observation), but the illumination is a natural transaction boundary
  if it should never happen at all.
