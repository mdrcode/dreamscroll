# Claim management — one identity phase, one claim phase

**Status:** Implemented 2026-11-09. A claim is written in two phases, both callers use both, and a
veto names a pairing. Supersedes the veto's addressing in
[knowledge-verdicts.md](archive/knowledge-verdicts.md).

**See also:** [lexicon.md](lexicon.md) (vocabulary), [knowledge-canonical-entities.md](archive/knowledge-canonical-entities.md)
(evidence as an append-only edge), `src/knowledge/knowledge.md` (the model as built).

## The trigger

A veto records `(subject_origin, scheme, external_id)` — a **pairing** — and `withdraw` found the
claim to retract with an extra predicate, `basis = 'text'`. `basis` records the **route**, not the
writer, and a census of the live graph (2026-11-09, 1,967 live claims) showed three populations:

| basis | `provider` | live claims | written then by | means |
| --- | --- | --- | --- | --- |
| `text` | `wikidata` | 1,595 | `store_claim` | "this entity is Q42", reached by title/label/search |
| `handle` | `platform` | 345 | `Evidence::record` | "the screenshot showed `@handle` on this entity" |
| `handle` | `wikidata` | 27 | `store_claim` | "this entity is Q42", reached **through** the handle (P2002) |

The census predates schema 34: the column was `asserted_by` (now `provider`), and the capture value
was `platform` (now `illuminator`).

The third population is an authority cross-reference that arrived through a handle, so
`basis = 'text'` excluded it: `identity veto 245622 wikidata Q317521` (`Elon Musk`, P2002) wrote the
decision, retracted **0** rows, and printed `withdrawn 0` — which reads as success.

The same mismatch had a second face: capture claims were exempt from vetoes twice. `withdraw` could
not see them (they are always `handle`), and `Evidence::record` never consulted `vetoed()` the way
`store_claim` did, so the next `link` re-claimed them.

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

1. **Settle** — identity rows.
   - `Evidence::ensure_identity(draft)` creates if absent and leaves an existing row alone.
     **Create only.**
   - `Evidence::enrich_identity(draft)` fills gaps and may upgrade `provider`. **Enrich.**
   - The caller chooses the mode; the store knows only those two behaviours.
2. **Claim** — settle and write the edge in one operation:
   `Evidence::claim_identity(holder, IdentitySettlement, ClaimDraft, vetoes) -> ClaimWrite`.
   - `IdentitySettlement::CreateOnly(draft)` uses create-only;
     `IdentitySettlement::Enrich(draft)` enriches.
   - Asks the veto once, at the one place a claim becomes a row.
   - Provenance is a value, not a code path: `ClaimDraft::capture(observation)` carries the
     observation and the surface; `ClaimDraft::authority(basis, provider, method)` carries neither.
   - Idempotent: a unique violation is `AlreadyClaimed`, not an error, and the existing claim id is
     returned.

The observation has no identity. Its optional `PlatformLink` maps to an `IdentityKey` when the link
has a handle; the store never reads observation fields.

**Link, then claim.**

```
KnowledgeGraph::link          a collection of observations in: decide, attach, project;
                              returns observation → entity
helper::link_illumination     batches one illumination's observations, calls `link`, then
                              write_capture_claims: for each pair with a platform link, claim_identity
                              (settle on first sighting + write claim), then project again
```

`KnowledgeGraph` knows only collections of observations. The fact that a batch is one reading,
and that a capture's claims are written afterwards, lives in `helper.rs`.

The first projection has no claims to see: it settles the entity's surfaces, and the index row the
blocker reads. The second is where `status` and `confidence` follow the claims.

Nothing inside the linker knows about identities or claims: the platform fields are used to decide
and otherwise left alone. The post-pass is where a capture's identity and claim are written, and
projection follows it because status and confidence are functions of the claims.

A crash between the two stages loses that observation's claim; the window is small, and a re-link
will not repair it because the observation is already attached.

## Why not durable claim ids

A claim row is derived: `reset` deletes claims and `link`/`resolve` re-create them with new ids. A
decision addressed by claim id points at nothing (or a different edge) after one reset, while it
still has to **suppress** the new claim — and suppression runs at admission, where no row exists. The
durable name is the pairing, which the verdict already stores. A row id is a within-generation
locator, and within a generation it is redundant: the unique index makes the pairing select at most
one live row.

`ClaimWrite` therefore returns the written row id for immediate callers, but it is not a durable
decision key.

## The veto rule

- A veto names a **pairing**. `withdraw` retracts that pair's live claim whatever route produced it,
  then reprojects the entity — status and confidence are functions of the live claims.
- A retraction with nothing to retract is **`settled`**, not `withdrawn 0`: the decision holds and
  says so. `gone` means the subject origin no longer names a live entity.
- The same predicate (`verdicts::vetoed`) is asked by `write_claim`, so admission and retraction
  cannot disagree.

## What changed

| file | change |
| --- | --- |
| `evidence/store.rs` | `IdentityKey`; `IdentityDraft`; `ensure_identity` (create-only), `enrich_identity`, `IdentitySettlement`; `ClaimDraft` + `claim_identity` (veto + edge). Deleted `record` and `store_claim`. |
| `evidence/mod.rs` | the "two writers" paragraph became the two phases; exports updated. |
| `graph/link.rs` | `link`: decisions applied and projected, returning observation → entity; `LinkStats`. No illumination type in sight. |
| `graph/knowledge_graph.rs` | `link(&[Observation])` is the only linking method. |
| `helper.rs` | the illumination composition and the claim post-pass. |
| `linker/mod.rs`, `linker/heuristic/linker.rs` | `Decision` no longer carries an identity — it is observation data, not a decision. |
| `identity/resolution.rs` | `claim_identity`, the same operation the capture path uses. |
| `verdicts.rs` | `withdraw` drops the basis filter; `ReplayReport.settled`. |
| `bin/dreamscroll_knowledge.rs` | `link` prints `vetoed`; `verdicts replay` prints `settled`. |
| `knowledge.md` | the evidence row, the veto row, `LinkStats`. |

No schema change. Existing verdict rows already carry the key; nothing to backfill.

## Tests

- A capture claim is suppressed by a standing veto at the next `link` (`LinkStats.vetoed`).
- A veto on a P2002/`handle` authority claim retracts it, and the entity returns to provisional.
- A repeat replay reports `settled`, not a zero withdrawal.
- Capturing a link never downgrades an answered identity (method stays `P2002`, label kept).
- Negative controls: without the veto check the capture claim returns; with `enrich_identity` for a
  capture the method becomes `capture`; with the old basis filter the P2002 claim survives.

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

- **Durable claim ids.** `ClaimWrite` returns the row id for immediate callers; decisions still address
  the pairing, because reset recreates claims with new ids. See above.
- **Conditional vetoes** ("distrust the authority's answer, keep the capture's"). With one claim per
  pair it would leave the pair claimed and change only the corroboration bonus. If a future schema
  allows several claims per pair, the key grows by `(basis, source)` — still not an id.
- **An event log.** Claim writes and retractions are symmetric at the row level (`insert` /
  `retracted_at`); the verdict table is the durable reason.
- **Per-generation retractions.** The `generation` column exists, but a veto is a standing decision.

## Open questions

- Is `settled` the right word? Does `ReplayReport.gone` need to separate "subject entity gone" from
  "identity row gone"?
- ~~Should `Kind::Veto` be renamed `Reject`, so the log says what it means?~~ Resolved 2026-11-10:
  renamed `veto_identity`, not `Reject` — the identity is the object, and `Reject` could read as a
  merge decision.
- `link_illumination` is not atomic. A crash between attach and claim write is repaired by the next link
  (the claim phase reads every observation), but the illumination is a natural transaction boundary
  if it should never happen at all.
