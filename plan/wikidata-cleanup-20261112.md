# Wikidata module — cleanup and review

**Status:** Active. The mechanical cleanup shipped 2026-11-12 (behaviour-neutral, proven by a
rebuild diff). The structural review is open: one known scoring defect and a set of design
questions below, none of them started.

**Purpose:** hold what `identity/wikidata/` is for, what was consolidated, what is actually wrong
with it, and how to verify a change here without guessing.

**See also:** `src/knowledge/knowledge.md` (the module map), `plan/lexicon.md` (vocabulary),
`plan/backlog.md` (the queue entry for the scoring defect), `plan/pragmatism.md` (accepted
shortcuts). Policy numbers and their provenance live in `ranking.rs` and `types.rs` themselves.

## What the module is for

Given a knowledge entity — a name, maybe a description, maybe an `entity_type` guess, maybe a
platform link — and a `Gateway`, produce **zero or one** `Identity`: `wikidata` plus a QID, with a
method, a confidence, and the label/description/classes behind it. Everything else is machinery for
getting there honestly.

Four properties that follow, and that every change should be judged against:

1. **Soundness over coverage.** A wrong QID is worse than none: it merges entities, distorts
   proximity, and costs a verdict to repair. "An incomplete table can cost a rejection but never a
   correct claim" is the rule `types.rs` and `aggregate.rs` are written to.
2. **Cost.** Wikidata is rate-limited. The field asks every question for every entity, so a cold
   run spends ~3.5k calls — 97% of them the two unbatched searches. Staging them by refusal or by
   corroboration was measured and declined (*Staged asking*, below); the cache is the lever, and an
   offline rebuild answers all but the first-contact questions from it.
3. **Auditability.** `method` and `confidence` are the recorded reason; `fetch_cache` is the
   evidence.
4. **Replayability.** `plan`, `aggregate` and `decision` touch no network, so an offline rebuild
   over cached payloads reproduces the decision.

## The mechanical pass (shipped 2026-11-12)

Behaviour-neutral, and proven so: see "Verifying a change here" below for the numbers.

| Change | Why |
| --- | --- |
| Deleted `answer::AuthorityIdentity` | It duplicated `evidence::Identity` field for field; `resolve_answer` existed only to copy one into the other, per claim |
| Deleted `plan::for_entity` / `plan::fallback` | `#[cfg(test)]` bundled passes kept only for tests; `ladder`/`step` already produce the same lists |
| Moved the decision tests `resolver.rs` → `policy.rs` | They assert gates, not the ladder. `resolver.rs` went from 1019 to 391 lines |
| Moved the shape test → `shape.rs` | It asserts a cache-key property, not a ladder one |
| Merged the two `impl Wikidata` blocks | Noise |
| `types::incompatible`: `every`/`allowed` → `every_class_is` | Every arm passes the *forbidden* families; the parameter name said the opposite |
| `transport`: `chunks(50)` → `chunks(BATCH)`, dropped two shadowing `use HashMap`, collapsed a tripled negative, trimmed a doc line duplicated from the module doc | The batch size had three spellings of one number |
| `policy::parse_search` takes `&[Candidate]` | Its siblings did; it only ever read the slice |
| Extracted `transport::entity_candidate` + tests | The `P31` read had no coverage, and an empty `classes` silently disables the kind gate |
| Consolidated four platform-property tests into one table | They duplicated `property_query`'s table; kept the one seam it cannot cover (DB string → property) |

## The defect: one candidate, two fits

The search rung measures a candidate twice, in one parse, against two different corpora.

```
search answer (N candidates, menus and wrong kinds already removed)
  │
  ├─ exactly one candidate whose label == our name?
  │     └─ fit_within → fit(ctx) → Weights::over(all N)        ranking.rs:187
  │           ├─ fit > 0 → claim · search_lookup · 0.85
  │           └─ fit == 0 → fall through
  │
  └─ admit (name-relatedness) → decide
        └─ Ranking::rank → fit(ctx) → Weights::over(admitted)  ranking.rs:124 via policy.rs:295
```

`fit` reads two things from its corpus: the document count `N`, and per-gram document frequency.

$$\text{of}(g) = \text{boost}(g)\cdot\left(1+\ln\max\!\left(\tfrac{N}{1+\mathrm{df}(g)},1\right)\right),
\qquad \text{fit} = \frac{\sum_g \text{of}(g)}{\sum_g \text{of}(g)+3}$$

So the corpus sets the magnitude of every matched term, and one hard rule in `fit` at `ranking.rs:254` —
`terse_and_rare = terse && matched.len() == 1 && df <= 1` — is a threshold **on the corpus**.

Two effects:

1. **Magnitude.** Two matched unigrams at `df = 1`: `N = 10` gives fit 0.635, `N = 3` gives 0.484.
   Through `blend` (0.6 × fit) and `answer_confidence` (0.45 × blend) that is ≈ 0.04 of confidence
   for the same candidate in the same answer, depending only on which path asked. Nothing is
   decided differently; the number is path-dependent.
2. **Threshold flip.** A lone exact-surface candidate with a terse description and one shared word
   that appears in two candidates' descriptions: the shortcut sees `df = 2`, refuses; the name rule
   then drops the other carrier, `df` becomes 1, the ranker sees evidence, and the same candidate
   can clear `CLAIM_BAR`. One candidate, "uncorroborated" and "corroborated", in one parse.

Why neither corpus is obviously right — they answer different questions, and `fit` is doing both
jobs:

- **The answer's candidates**: "how distinctive is this word among everything this name
  retrieves?" That is what IDF was written for, and what `Ranking`'s doc claims ("terms are weighed
  by rarity among the answers the question returned"). `fit_within` honours the doc; `rank`
  inherits whatever subset its caller passes, so the doc and its only caller disagree.
- **The admitted set**: "how distinctive is this word among the things we might actually pick?" A
  word carried by five already-excluded items is not common among our choices.

Scope: the *double measurement* is search-only. The *corpus choice* is broader — `parse_title`
filters to `titled` and then calls `decide`, so the title rung also ranks over a subset, just
without a second opinion to contradict it. The label rung hands `decide` the whole answer, so its
two corpora already coincide.

Options:

| | change | behaviour | cost |
| --- | --- | --- | --- |
| **A** | corpus = the answer everywhere; `decide` passes the full set for weighting, the admitted set for selection | confidence moves on `search_ranked` and on title; no accept/refuse flips unless `df` crosses 1 | `Rank`'s trait signature survives; `Ranking` needs a corpus, and `fit_within`/`blend_within` collapse into it |
| **B** | corpus = admitted everywhere; admit first, then let the shortcut measure over `admitted` | same direction as A; one corpus | **the corpus is usually 1–3 candidates, where IDF is inert**: with `N <= 2` every `N/(1+df)` is `<= 1`, the floor applies, and `fit` collapses to `matched/(matched+3)` — 0.25, 0.40, 0.50 for one, two, three matched grams, whatever the words are. Search would measure with a statistic that has stopped measuring; confidence drops correspondingly |
| **C** | the shortcut stops using `fit`: lone exact surface + plain token overlap | narrowest blast radius; no second corpus to synchronise | a third notion of "corroboration" |

Correction to an earlier claim: the shortcut does **not** bypass admission — `near_surface` accepts an
equal surface (`text/surface.rs:17`), so its candidate always survives `admit`. What it bypasses is
the *margin* and the blend pricing: it claims at `LOOKUP_CONFIDENCE` with method `search_lookup`
rather than competing. B does not make it consult rules it avoids; B changes the corpus it is weighed
against.

Preference: **A**, on corpus quality rather than documentation — IDF needs a retrieval set to have
frequency to read, and the answer is the only set that exists before the decision it informs. B's
corpus is conditioned on that decision and is too small for the statistic to work. A also lets one
`Weights` per answer serve the shortcut, `choose`, and the title rung, deleting both `*_within`
helpers. Its cost is real: `Ranking` becomes per-answer (`Ranking::over(&answer)`) instead of a unit
struct, so `Scoring` becomes "score these against that corpus". Whichever option lands, the ranking
module's one-line claim about the corpus gets rewritten to match.

## Other findings, by severity

1. **`Step::SurfaceTitle` is a decision wearing a request's clothes** *(accepted 2026-11-13: the
   ladder stays one shape; the cost is two special cases, documented at the site; revisit with a
   third consumer)*. It plans the identical
   `Request` as `Step::Title` and exists only to re-decide the same cached answer after everything
   else refused. The cost is two special cases in two consumers (`resolver::ladder`'s
   `if step == SurfaceTitle`, `inspect`'s two arms) for what is really "the last rule of the title
   question, deferred". The ladder conflates *what we ask* with *how we decide*.
2. **`inspect` re-implements the ladder loop** *(partly eased 2026-11-13: it now calls
   `decision::surface_only(entity, raw)`, so the answer's outcome is visible there; the loop itself
   is still a second implementation)*. It walks `plan::ladder`, does its own cache lookup,
   its own `admit`/`surface_only`/`explained` dispatch, and its own stop-at-first-claim. The steps
   are shared (which is what `plan` was fixed for); the *loop semantics* are not. `plan::Step`'s doc
   claims "used by the resolver and by the eval replay" — **`eval` does not touch this module at
   all**. The only replay of the decision is `inspect`, and it is a second implementation.
3. **Two different name-relatedness rules** *(accepted 2026-11-13: each is justified where it lives
   and both were measured; revisit with a second provider)*. `parse_title` admits a candidate that shares *any*
   content token with our name; `parse_search` demands near-surface, a qualifier suffix (`, …` or
   `(…)`), or subset-of-content-words. Each has a long justification and neither is tested against
   the other. They may genuinely need to differ (a title query was our exact name; a search result
   was generated from text) — but that is currently an accident of authorship, not a stated rule.
4. **Confidence is neither a rung label nor a calibrated probability** *(accepted 2026-11-13: it
   stays a score, and downstream keeps sorting on it; a calibration sample does not exist)*. It is a float mixing
   hard-coded rung values (0.95/0.85/0.75), a corpus-size-dependent `fit`, and a ceiling. Downstream
   treats it as comparable: `project` takes `best_confidence` as the entity's confidence, and
   `evidence` sorts entities by it. Either it becomes an enum (and consumers stop ranking on it) or
   it gets calibrated against a labelled sample, which does not exist. `types.rs` is the model for
   the second: it is the one place here where measured corpus evidence decided a rule.
5. ~~**A `Degraded` answer can claim.**~~ **Fixed 2026-11-13** — a degraded answer is decided and
   then refused (`decision::trusted`), and the claim it would have made is logged as a warning. See
   "Degraded answers never claim" below.
6. ~~**`Kind::Handle(&'static str)` defeats `PropertyId`.**~~ **Fixed 2026-11-13** — `Handle` carries
   the newtype, the constants are the only way to build one, and tests name `PropertyId::X_USERNAME`
   instead of a literal.
7. **The ladder is budget-blind** *(accepted 2026-11-13: ordering rungs by cost needs per-rung costs
   measured first, and no run has been budget-bound yet)*. `Resolver::resolve` receives the `Gateway` and can observe a
   refusal, but not how much allowance is left, so it cannot prefer cheap rungs when a run is
   short. Ordering rungs by value per call is the resolver's business and it has no input.
8. ~~**Test topology is archaeology, not specification.**~~ **Done 2026-11-13** — every gate is a
   `Case` table (gates, each rung, admission, degraded) and `ranking.rs` carries one test per rule.
   Three white-box tests remain by design: `choose`'s margin ordering, `surface_only`'s guards, and
   the degraded contract, none of which `decide` can express. ~25 single-case tests, most documenting a
   historical bug, several pinning exact confidences (0.75, 0.80). Valuable as history; fragile as
   regression protection, and duplicative (three `Dune` tests for one tie-break rule). What is
   missing is one table test per *gate* — menu, incompatible class, name-relatedness of each kind,
   corroboration, margin, deferral — because the gates are the rules that matter.

## Deliberately not changed

- The ladder's order, the gates, the thresholds, and every confidence number. The mechanical pass
  moved code and deleted duplicates; nothing above under "Other findings" is addressed by it.
- The `search` rung's substitution for `wbsearchentities`, and the `x-search-id` degraded signal.
  Both are documented at the site and were measured.
- `types.rs`'s family table. Every QID there was checked against Wikidata (2026-11-07) and the
  unmapped entity types are deliberate — mapping them rejected correct claims.
- **The stemmer's plural disagreement — accepted 2026-11-13.** `carbines` stems to `carbin` while
  `carbine` does not stem. Trimming the trailing `e` was implemented and measured: 23 decisions
  moved, one gain and one improvement, but merging `force`/`forces` let a *painting* ("Royal Air
  Force Museum") outrank the B-17 whose description is thin. A one-word stemmer fix is not worth a
  wrong referent; the pair stays split until the thin-description/margin problem is solved for its
  own reasons.

## Verifying a change here

A resolver change moves claims, so it needs a before/after rebuild, not a unit test. The working
procedure, learned the hard way on 2026-11-12:

```bash
git stash push -m <wip>              # or: check out the baseline commit
cargo build --bin dreamscroll_knowledge
dreamscroll_knowledge reset -y       # snapshots the projection, then empties it
dreamscroll_knowledge link
dreamscroll_knowledge identity resolve --offline
# capture the claims, keyed by OBSERVATION id (see below)
git stash pop && cargo build --bin dreamscroll_knowledge
dreamscroll_knowledge reset -y
dreamscroll_knowledge link
dreamscroll_knowledge identity resolve --offline
# capture again and diff
```

Three traps:

1. **Entity ids are reallocated by `reset`.** A dump keyed by `knowledge_entity_id` differs on
   every row between two builds that decided *identically*. Key by the observation
   (`user_observed_entities.id`, or the `knowledge_entity_membership__*` snapshot) — that is why
   `compare` exists.
2. **Always rebuild both sides.** On 2026-11-12 the live graph held 1665 identities against a fresh
   1677: 12 claims stale. Using it as the baseline would have reported a false diff.
3. **Never `cargo fmt` at the repo root.** The tree is not rustfmt-clean; `cargo fmt` rewrote 31
   unrelated files. Format the module root only, or not at all. (`plan.rs` and `transport.rs` still
   carry ~26 lines of incidental reflow from this.)

Result of the 2026-11-12 pass: 484 tests green; both builds 2734 observations / 458 linked / 2276
entities / 5392 requested / 5369 cache hits / 1677 identities / 5 vetoed; claim table keyed by
observation, 2474 rows each, `diff` empty.

## Review log (file by file)

Walking the module top-down with the reviewer, in the order recorded here; each row gains findings
and the fix that closed them. Status values: `pending`, `in progress`, `done`, `accepted`.

| # | File | Status | Findings / decisions |
| --- | --- | --- | --- |
| 1 | `resolver.rs` | done | R1–R11: free functions, `accumulate` / `last_resort`, the vocabulary sweep. Open: no precedence test, the `Canned` footgun |
| 2 | `plan.rs` | done | P1 `plan::all_steps`, P2 `trim()` measured; `Handle` now carries `PropertyId` (2026-11-13) |
| 3 | `decision.rs` | done | M1–M8: one entry, `choose` private, dead guards, method vocabulary, `Result` refusals, both score questions measured. Open: file size only |
| 4 | `ranking.rs` | done | S1 scoring contract, two-fits (option A shipped, measured), `terse_and_rare` kept, comment pass S5 |
| 5 | `types.rs` | accepted | best quality-per-line; arms enumerate forbidden families inline. Two findings measured and declined 2026-11-13: the gate reads one of the entity's types (`alt_types` unread), and nine of twenty types are unmapped — mapping them refused four wrong referents but enabled two new ones, reverted |
| 6 | `transport.rs` | pending | `search` mixes three concerns; `sparql` picks its query from `first.kind`; no degraded-branch test |
| 7 | `property_query.rs` | pending | clean; its `WikidataKind::Handle` coupling is closed (2026-11-13), the file is still unreviewed |
| 8 | `query.rs` | pending | `escape` coverage; a comment block inside `format!` |
| 9 | `shape.rs` | pending | maintained by hand, not derived from the builder it must agree with |
| 10 | `answer.rs` | pending | `is_menu`'s description signal is a convention, not a rule |
| 11 | `kind.rs` | pending | `property()` has one caller; the bare-string finding is closed (2026-11-13) |
| 12 | `candidate.rs` | pending | `classes` empty meaning "not asked" is load-bearing |
| 13 | `mod.rs` | pending | declarations plus three constants |

The order is spine-first (the ladder, its questions, its decisions), then the measures and the
wire, then the leaves. Every later file is only meaningful in light of the ladder, and the leaves
each hold at most one question.

### 1. `resolver.rs` — done 2026-11-12

| # | Finding | Fix | State |
| --- | --- | --- | --- |
| R1 | `USER_AGENT` and `Client::builder()` sat in `resolver.rs`, so the policy file named `reqwest`. `Wire`'s own doc claims "one client" but was handed one. Moved to `transport.rs`: `USER_AGENT`, the 30 s timeout and the builder; `Wire::new() -> anyhow::Result<Self>`; `Wikidata::new()` is now a wrapper. `resolver.rs` names no HTTP crate. | fixed |
| R2 | `Kind` was one of **three** `Kind` enums in `crate::knowledge` (`snapshot::Kind`, `knowledge_verdicts::Kind`, `wikidata::Kind`), and it was `pub use`d at the module root. Renamed `WikidataKind` — unambiguous, unlike `WikiKind`, which in this domain reads as *kind of wiki entity* (item, article, file), a different axis. (`Rung` was considered and rejected as prose-only vocabulary.) | fixed |
| R3 | `collect` named the wrong half of the loop: it reads one wave and arbitrates first-to-speak, into two `&mut` out-params. Renamed `accumulate` (via `harvest`, which read as a metaphor rather than a description): it appends one wave's identities and arbitrates first-to-speak. | fixed |
| R4 | `parse` was misleading — parsing happens in `transport` (JSON → `Candidate`); this module decides what a candidate *means*. `identities` also returned `Vec` where every arm returns exactly one or none, and was sugar over `explained`. Now module `interpret`, entry `identify(...) -> Option<Identity>`, reason variant `identify_explained(...)`; `policy`'s four entry points return `Option` too, so no layer keeps the lie. | fixed |
| R5 | `Gateway::resolve_requests` reused the domain verb — the lexicon defines **resolve** as "knowledge entities → evidence, network, cache-first". Renamed `Gateway::answers_for(...)`, which names its return type (`Answers<K, P>`). Deliberately not `fetch`: that is `Transport::fetch`, the network call, one layer down. | fixed |
| R6 | `Step::SurfaceTitle` encodes a *decision* as a `Step`: it plans the identical `Request` as `Title`, then takes a different branch (`admit` + `surface_only`) in the step loop. Kept as a step (the deferral is visible as data), but now **labelled a hack**: documented as "a decision wearing a step's clothes", marked `HACK` in `plan.rs` with what it costs and what it wants to become, and extracted in `resolver` into a named `last_resort` pass with its own doc. The loop is one `if` and two calls. | kept |
| R7 | `accumulate`, `last_resort` and `ladder` took no `self`, yet lived in `impl Wikidata`. Residue: they were methods when the loop was written against resolver state, and after R1/R3 that state is gone — `Wikidata` holds one field and `ladder` takes the transport as a parameter. Moved to module-level free functions; the impl now holds only `new()`. | fixed |
| R8 | `Candidate` was ambiguous across modules: `inspect` has its own `Candidate` (a *blocking* candidate) and imported the identity resolver's as `Candidate as Offered` purely to avoid the clash, so nobody could tell which one a signature meant. Both identity resolver types now carry the provider's name — `WikidataKind`, `WikidataCandidate` — and the `Offered` alias is gone. | fixed |
| R9 | Considered returning a map from `Resolver::resolve` instead of `Vec<(i32, Identity)>`. Rejected: the engine needs entity -> resolver -> identities (`proposed: BTreeMap<i32, Vec<(usize, Identity)>>`) and validates every id against the batch it sent, so the grouping moves rather than disappears; a map would cost one `Vec` per entity and lose production order. Shape kept. | kept |
| R10 | `interpret::claim` reintroduced the word the lexicon retired when the identity annotations replaced the old model. Renamed `identify` / `identify_explained`, locals and the module's comments swept with it; the only `claims` left are Wikidata's own JSON field, backticked where it appears in prose. | fixed |
| R11 | Three functions were named `ladder`: the resolver's wave driver, a pure per-entity projection in `plan`, and `inspect`'s diagnostic traversal. Only the driver climbs; the other two are a view and a walk. Renamed `plan::all_steps` and `inspect::walk`, leaving one `ladder` in the tree that means what the word means. | fixed |

Left open on purpose when `resolver.rs` was signed off:

- **Handle > title > label precedence has no test.** The file's job is first-to-speak-wins; the
  tests pin the *ordering of requests* and that a title resolution suppresses the label wave, but
  nothing asserts that a handle answer beats a title answer when both arrive.
- **The `Canned` fake answers `Empty` for any unseeded request**, so a typo in a seeded query looks
  identical to a provider with nothing to say. `asked()` guards only the assertions that call it.
- **`accumulate` and `last_resort` stay near-twins** (same guard, same lookup, different decision).
  That is the price of keeping R6 as a step; we decided the `identify_for` dispatch was a lateral
  move that would hide the anomaly from the file where it is visible.

What R6 became, since it is the one we did not restructure: `Step::SurfaceTitle` is documented as
a **decision wearing a step's clothes** and marked `HACK` in `plan.rs`, naming exactly what it
costs (a branch in `resolver::ladder`, a matching one in `inspect`) and what it wants to become (a
post-pass over the title answer). In `resolver` it is now a named pass — `last_resort` — with its
own doc, instead of 15 lines inline in the ladder loop; the loop is one `if` and two calls.

### 2. `plan.rs` — done 2026-11-12

- **P1 — three functions named `ladder`.** The resolver's wave driver, a pure per-entity projection
  in `plan`, and `inspect`'s diagnostic traversal. Only the driver climbs; the other two are a view
  and a walk. Now `plan::all_steps` (a flattening, not a one-to-one map: a platform link asks a
  query per value) and `inspect::walk`, with one `ladder` left in the tree. `plan_flat` was
  considered and dropped: it names the mechanism, stutters as `plan::plan_flat`, and "projection"
  already means the graph's derived columns.
- **P2 — `trim()` is the whole query-boundary policy, and that is right.** Measured over 2,281
  entities: no leading/trailing whitespace, no non-NFC text, no invisible characters, no doubled
  inner spaces. The label rung is an exact match, so folding would *cause* misses; the title API
  normalises itself; search is tokenised. The tripwire (silent duplicate entity if a future
  illuminator emits ZWSP/NFD) is recorded in `plan/backlog.md`.

Left open on purpose when `plan.rs` was signed off:

- **`WikidataKind::Handle(&'static str)` drops `PropertyId`.** The newtype exists because "a typo is
  invisible" in a query, and it is the only source of the value today, so nothing is broken — but the
  type no longer stops a future caller from minting a handle kind from an arbitrary string. Not worth
  churn now; revisit if a second producer of handle kinds appears.

### 4. `ranking.rs` — done 2026-11-13

- **S1 — the shared scoring contract, before the file.** `Rank` promised "ordering *and* the
  threshold that turns a candidate into an answer — an empty list means nothing here is this". True
  for the graph's conjunction; false for an identity resolver, whose gate is a margin and whose answer also
  carries a method and a confidence that `(usize, f32)` cannot hold. So the promise moved into a verb
  that can keep it: `Policy` authors `evaluate` / `qualifies` / `order`, while `Rank` became its
  object-safe projection written once as a blanket impl. `Similarity` is now corpus-free *by rule*,
  which is the distinction whose absence produced the identity resolver's two incompatible fits. `Order`
  names the two-part key; each side's evidence is named (`Features` owns its judgement, the identity resolver
  has `Fit { fit, blend }`). Behaviour identical: rebuild reproduces the identity table row for row.
- **S2 — the bundle became a view instead of three trait objects.** `Scoring` was a struct of three
  `Arc<dyn …>` fields, because a trait object fixes its type parameters — so one policy had to be
  stored three times and `Scoring::new` repeated the bounds. `Rank` already solved that shape with a
  blanket impl, so `Scoring` is now a trait with the graph's three questions, blanket-implemented for
  any policy that answers them. The heuristic is stateless, so `Heuristic::scoring()` hands out one
  `&'static dyn Scoring`; `HeuristicLinker` borrows it and proximity takes `&dyn Scoring`.
  `with_scoring` was deleted — defined, documented, never called.
- **S3 — the string work moved out, and the dependencies it should use are on the table.** `ranking.rs`
  had grown a tokenizer, a suffix stemmer, a gram builder, a fuzzy-surface test and a Levenshtein
  loop: all pure, all about text, none of them Wikidata's. They now live in `knowledge::text`, beside
  `normalize` — which asks a different question (the fold produces a key that is stored; `text`
  produces nothing that outlives the comparison). Moved verbatim; identity table reproduced row for
  row.
- **S4 — `text` is a module, `normalize` is absorbed, and the edit distance is no longer ours.**
  `src/knowledge/text/` now holds one file per question: `fold` (the comparable key), `key` (the
  word-order-insensitive form), `fingerprint` (the stamp both are versioned by), `tokens` (words,
  stems, grams, `jaccard`) and `surface` (`near_surface`). `normalize.rs` is gone; its call sites
  name `text::`. `Cargo.toml` gains **`strsim` 0.11** (no transitive dependencies) for
  `levenshtein`, replacing the loop we wrote and tested ourselves.
  *Open work, tracked in `plan/backlog.md`* (the one list): unifying the second tokenizer
  (`linker/heuristic/scoring.rs` splits a folded surface on whitespace; `text::tokens` stems and
  filters), the stem's plural disagreement, the sub-three-character blind spot, and `inspect`'s
  four-candidate truncation. *Foregone, with reasons:* `unicode-normalization` (would move `norm_fingerprint`, so a
  blocking-index rebuild — its own project) and `unicode-segmentation` (the corpus is ASCII-ish).
- **S5 — `ranking.rs`'s own comment pass.** Behaviour was right; the prose had drifted. Two orphaned
  blocks left behind when `text/` took the string code (`NOISE`'s doc sitting on the `Ranking` struct;
  `stem`'s and `grams`'s docs stacked above `Weights`), three stale claims (the calibration header
  pointed at "the blend above", which is below; `CLAIM_MARGIN` was "held to the same margin" as the fit
  margin, which it is not; `fit_within` credited `parse_search`, renamed `decision::search`), and a
  module doc that described the whole decision when the file only produces the evidence for it. Also
  removed the places where a doc and its code said the same thing twice: the mapping constants moved
  inside `answer_confidence`, and the tie-break rationale moved to `order()` where the tie-break is set.
  343 → 333 lines, 86 comment lines, no behaviour change.

  While moving it: the stem's comment claimed a guard that does not exist (`filmmaker` → `filmmak`),
  now corrected and pinned. Options, with crates.io facts as of 2026-11-13:

  | crate | version | normal deps | replaces | behaviour |
  | --- | --- | --- | --- | --- |
  | `strsim` | 0.11.1 (2024, 1.27B dl) | none | `edit_distance` | identical — same Levenshtein |
  | `unicode-normalization` | 0.1.26 (2026) | tinyvec | `normalize`'s hand-written fold, and the NFD tripwire in `plan/backlog.md` | moves `norm_fingerprint`, so a blocking-index rebuild |
  | `unicode-segmentation` | 1.13.3 (2026) | none | the `split(!is_alphanumeric)` tokenizer | mild; the corpus is ASCII-ish |

  Recommendation: take `strsim` (no tradeoff: it deletes an algorithm we own and returns the same
  numbers); keep the hand-rolled stem and fix its plural bug (tracked in the backlog) rather than
  adopting `rust-stemmers` — decided 2026-11-13, the swap is not worth measuring; treat
  `unicode-normalization` as its own project (`norm_fingerprint` and an index rebuild), not a
  drive-by; skip `unicode-segmentation` until the corpus asks for it.

### 3. `decision.rs` (was `policy.rs`) — done 2026-11-13

- **M1 — the method vocabulary had four producers.** `method_for` mapped rung+exactness to a name,
  while `parse_search` and `surface_only` wrote `"search_lookup"` and `"wikipedia_title"` as their
  own literals — two names that had to agree with `method_for` by construction and could not. Deleted
  the duplication instead of centralising it: both sites now call `method_for`, leaving exactly two
  producers (the rung mapping, and `by_hint`'s one literal, which is correct because a hint is not a
  rung). A new test pins the surface path's method, which nothing covered before.
- **M2 — the file was named after a word this crate already uses.** `linker/heuristic/scoring.rs`
  calls the pluggable scoring strategy "the policy" (`knowledge.md`: "Choosing one policy for both
  sides"), so `wikidata/policy.rs` was a second meaning of `policy` — the same collision class as
  `resolve`, `Kind` and `claim`. Renamed `decision.rs`; the module header now states its two layers
  (admission per rung, then the decision) and the rule that admission may not re-implement the
  decision.
- **M3 — `decide` took a `&Request` but only ever read `kind`.** Narrowed to
  `decide(entity, candidates, kind, reason)`: the signature now says the decision depends on *which
  rung* answered, and stops advertising a dependency on the question text it must never score
  against. `surface_only` lost its `kind != Title` guard and its request parameter for the same
  reason — its only caller is the deferred title step, so the guard was a comment that could not
  fail.
- **M4 — the dispatch was a hybrid: four kinds, three depths.** `interpret` decided `Handle` itself,
  sent `Title`/`Search` to their own procedures, and let `Label` reach the shared tail directly.
  Folded into one entry: `decision::decide(entity, kind, answer, reason)` gates, then matches
  exhaustively — `handle` / `label` / `title` / `search` — each ending in the private `choose`.
  `interpret.rs` is gone; `admit` and `surface_only` live beside the entry. Adding a rung is now one
  exhaustive match next to the four procedures, instead of an edit in one file plus a function in
  another. Cost: a 1008-line file (392 impl / 616 test).
- **M5 — dead guards, and the invariant nobody had written down.** `admit` guarantees a non-empty
  candidate set, so `handle`'s "the handle answer carried no item" branch and `by_hint`'s empty-set
  return could never fire — an affordance in the code claiming a failure is possible when it isn't.
  Both deleted (and `handle` lost its `reason` parameter). `by_hint`'s score is now documented as a
  *different axis* from `choose`'s blend — support proportion, not IDF-weighted fit — with the
  ceiling as the guard, plus its two call contracts (admitted set from `choose`, unadmitted from
  `search`). The file header finally states the per-rung invariant: each rung has exactly one early
  exit, matched to what its query guarantees, which is also why `title` admits generously and
  `search` strictly.
- **M6 — the two name tests: measured, and *not* unified.** The hypothesis was that `title`'s
  any-shared-word rule and `search`'s containment rule diverged for no reason. Both directions were
  rebuilt and diffed against the baseline:
  tightening `title` to the search rule lost 17 claims, 16 of them correct (`Colonel Sanders` →
  *Harland Sanders*, `Ray Kurzweil` → *Raymond Kurzweil*, `Ali Khamraev` → *Ali Hamroyev*,
  `20th Century Fox` → *20th Century Studios*, `Google Colaboratory` → *Google Colab*);
  loosening `search` to the title rule netted six claims but **reassigned twelve referents**.
  So the divergence is load-bearing in both directions: a Wikipedia *article title* can be our name
  while the *item label* is not, and a text-search result set carries no such guarantee. Code
  unchanged; the file header now states the reason with the measurement behind it.
- **M7 — the refusal was written through an out-param.** `reason: &mut &'static str` ran through six
  functions and made `resolver` carry a scratch variable it ignored. `decision` now returns
  `Result<Identity, &'static str>` — `Ok` the identity, `Err` the reason a stage closed — and `?`
  replaces the `*reason = …; return None` pairs. `handle` cannot fail (`admit` guarantees a candidate),
  so it returns `Identity`. The lone-exact-label shortcut moved out of `choose` into `label`, where its
  rung is; the shared tail no longer holds a kind-specific decision, and `kind` reaches it only to name
  the method that won. `admit`'s doc now says what it returns as well as what it rejects.
- **M8 — the hint's score: measured, and left alone.** The hint scored itself as
  `NAME_WEIGHT + SUPPORT_WEIGHT × support`, assuming full name credit, while `choose` uses the real
  `blend`. Switching it to `blend` cost five identities (six hint claims lost, one gained): the
  assumption is the point, because the hint runs only after the name test failed and weighting name
  agreement again would double-count that failure. Reverted, and the doc now says so with the
  measurement.

Left open on purpose when `decision.rs` was reviewed:

- **The file is 1,040 lines** (392 impl / 648 test). Accepted as the price of one entry plus four
  sibling rungs (M4); the thing to watch if it keeps growing.

Left open when `decision.rs` was reviewed, and deliberately declined:

- `admit`'s doc describes what it rejects but not what it returns ("the surviving candidates, or why
  the answer was refused"). Reviewer's call: leave it.

Verification of this pass: `cargo test --all-targets` 484 passed; full offline rebuild (2739
observations, 458 linked, 2281 entities, 1677 identities) and a claim table keyed by observation
that is **identical** to the pre-rename build — 2474 rows, 0 differ.

Cross-cutting consequences, all applied:

- `Kind` → `RequestKind`: `kind.rs`, `mod.rs` re-export, `plan.rs`, `shape.rs`, `parse.rs`, `policy.rs`, `transport.rs`, `resolver.rs`, and `inspect.rs`'s test import.
- `parse` → `interpret`: `mod.rs`, `resolver.rs`, `inspect.rs` (three call sites), and `knowledge.md`.
- `Gateway::resolve_requests` → `answers_for`: `fetch/gateway.rs` (definition + 2 test sites), `fetch/mod.rs`, `fetch/stats.rs`, `identity/mod.rs`, and `knowledge.md`.

## The two-fits defect: decided, and what it cost (2026-11-13)

**Option A shipped.** `Ranking` carries the answer's `Weights`; the lone-surface shortcut and `choose`
share one instance, so `fit_within`/`blend_within` are gone. Measured over the full 2281-entity
rebuild, keyed by `origin_observation_id`:

| | baseline | with A |
| --- | --- | --- |
| identities | 1677 | 1672 |
| `wikipedia_title` | 1249 | 1249 (no change at all) |
| `search_ranked` | 45 | 41 |
| `search_lookup` | 20 | 19 |

25 entity rows moved: 18 confidences up (max +0.049), 2 down, 5 refused. Judged case by case:

| obs | old claim was | verdict |
| --- | --- | --- |
| 155 | the hardcover *edition* of the book, on `2024` alone | **win** — edition ≠ work |
| 240 | Bushmaster M4 Type Carbine, a clone, on `carbine` | **win** — wrong referent |
| 729 | *Rebel Country*, a 2024 documentary, on `american` | **win** — wrong referent |
| 757 | Bell System — correct (label is 2 of our 3 name words) | **loss** |
| 1784 | candied orange peel — correct, exact label | **loss** |

All five were `choose` claims resting on one shared word, accepted only because `df <= 1` over an
admitted set of 1–3. Over the answer corpus that word is carried by 2–7 candidates. **The gate was
asking a competitor question while measuring a document-frequency fact.**

Two reformulations were measured and **rejected**:

- **Competitor-only** (`df` over the admitted set): restores all five, i.e. Bell and Candied return
  *and so do* the edition, the clone and the documentary. Net: +2 right, +3 wrong.
- **Ordered-label + non-year** (lone word counts only when the label is our name in order and the
  word is not a bare year): `Bell Telephone System` and `Candied orange peel` still refused, and it
  loses five more (`Westinghouse`, `Lexia Learning`, `Vodolazkin`, `Drafting Table`, …).

Conclusion: keep the answer-corpus `df` gate. The two losses are the price of refusing the three
wrong claims, and wrong identities are the more expensive error.

### Two defects the comparison exposed, both fixed

- **A short label was not a name.** `content_tokens` drops runs under three characters, so `M4`
  tokenised to nothing and Q214688 (the actual M4) was never admissible for `M4 carbine`. `search`
  now also admits when one folded surface is a whole-word prefix of the other (`M4` ⊂ `M4 carbine`),
  with a boundary test so `roy` is not `royal`. Measured: **zero corpus change**, so no risk.
- **A label-less candidate could not be the answer.** `search`'s admission refused any candidate
  without a label, which excluded the *correct* item for `Gladiator` (Q128518), `The Big Short`
  (Q19850715) and `The Talented Mr. Ripley` (Q390052) — after which `by_hint` claimed a sibling:
  a soundtrack, a film festival, a 1955 novel. Blanket admission was measured first and **rejected**:
  9 gains, 6 of them wrong (`2026 video game`, `book 2009`, `open source tank battle game`, the NPR
  CEO, an xAI encyclopaedia). The rule kept requires **three substantive description words in common**
  with us — a mention is not a naming.

Measured result of the two kept fixes: 3 wrong referents corrected (festival→film, soundtrack→film,
novel→film), 4 method upgrades (`search_hint` → `search_ranked` at the same QID, honest confidence),
and no wrong gains.

**Correction, found while working the backlog:** the three-word bar counted *tokens*, not distinct
words, so `gaming` + `game` (both `gam`) counted twice and a two-word overlap passed. It now counts
distinct words. Two claims did not survive that: `Nostalgic Gamer` (a 2026 video game) and
`Eurail Pass` (a genuine 2-word match: `rail`, `pass`). The rule is stricter than intended and
Eurail is the price; both are recorded rather than tuned away.

Net against the baseline: 3 wrong claims corrected, 3 wrong claims refused, 1 correct claim gained,
2 correct claims lost. `cargo test --lib`: 476 pass.

### Found while measuring, not yet fixed

- **`identity inspect` truncates each rung to four candidates** (`.take(4)` in `inspect.rs`), so a
  decision can name a QID the report never showed. `M4 carbine` printed four of nine and decided on a
  candidate outside the four. Diagnostic only — but it misled this review twice.
- **`content_tokens` drops runs under three characters.** Admission now compensates with the
  word-prefix rule (`M4` ⊂ `M4 carbine`), but the *measure* still cannot see `M4` inside a
  description, so a short token contributes no evidence anywhere.
- **The stemmer disagrees with itself on plurals here:** `carbines` → `carbin`, `carbine` →
  `carbine`, so a label saying `carbines` never meets a description saying `carbine`.

All three, and the tokenizer split with the linker, are tracked in `plan/backlog.md` — the single
list to iterate against.

## Degraded answers never claim (2026-11-13)

`Outcome::answered()` includes `Degraded`, so before this change a prefix-only result set could claim
— the exact circumstance that produces a plausible wrong QID. Now `decision` decides the answer and
then refuses it (`trusted`), which is what lets the discarded claim be *reported* instead of dropped:

```
WARN a degraded answer would have claimed; discarded entity=… qid=… method=…
```

Both claim paths go through it: the ladder (`decide`) and the deferred surface pass
(`surface_only`, which now takes the `Answer` rather than pre-admitted candidates, so it can see the
outcome). The warning fires only when the answer *would* have produced an identity — a degraded
answer that decides nothing was not discarded, it was empty.

Why keep the refusal: the payload is a prefix-only set from a backend that never answered, and a
wrong QID is more expensive than a missing one. Why log it: that log line is the evidence needed to
revisit the rule, and a silent refusal is how "degraded may claim again" would slip back in.

Measured: the local cache holds zero degraded answers, so the identity table is unchanged (1673).
`a_degraded_answer_never_claims` pins both sides — the same payload claims when `Found` and is
refused when `Degraded`.

## The startable list, worked (2026-11-13)

Four items from `plan/backlog.md`, each measured against the identity table keyed by
`origin_observation_id`. Net against the baseline: **1677 → 1671** identities, made of 4 wrong
referents corrected, 2 wrong claims refused, 2 correct claims lost, 2 duplicate entities folded
(`Nintendo` × 2, `The Criterion Collection` × 2), 4 method upgrades, 27 confidence-only moves.

- **`inspect` truncation — fixed.** The decided candidate is always printed (`.take(4)` used to hide
  it: `M4 carbine` decided on a QID the report never showed) and the rest are counted as `… N more`.
  Display only; a unit test pins both paths.
- **Short tokens — fixed.** Two-character runs carrying a digit are words (`M4`, `5G`); other
  two-letter runs stay dropped. `M4 carbine` had been unidentifiable because `M4` tokenised to
  nothing; it now resolves to Q214688, with 7 confidence nudges and no loss.
- **Plural stems — attempted, reverted.** Trimming a trailing `e` makes `carbine`/`carbines` meet,
  and moved 23 decisions: one gain (`National Museum of Ancient Art`), one improvement
  (`Hittitology` → `Hittites`) — and one wrong referent, because `force`/`forces` then match and a
  *painting* ("Royal Air Force Museum") outranked the B-17. Reverted; the backlog entry now carries
  this evidence instead of a plan.
- **Tokenizer unification — shipped in two steps.** Step 1: the linker drops the resolver's
  `NOISE` words, which folds `Nintendo` × 2 and nothing else. Step 2: `tokens_of` delegates to
  `text::content_tokens` outright, which folds `The Criterion Collection` × 2 and nothing else.
  Both merges are right (same company, same QID each time), and the identity rows they deduplicate
  are why the count drops again.

**Bug found in my own rule:** the label-less admission counted matching *tokens*, not distinct
words, so repeated stems inflated the count. Fixed; see the correction above.

## Pooled ranking experiment (2026-11-13)

Hypothesis, from the failure analysis: precision is decided by *what may compete*, not by who wins.
So ask every rung, pool the candidates, rank once. Working tree: `wikidata/pool.rs` + the resolver
asking all rungs; the ladder's per-rung admission and first-claim stop are gone from the resolver
(they remain for `inspect`, which still replays the old path).

Score: `tier_prior + 0.6·surface + 0.4·fit + kind + breadth`, where surface is the label against
the entity *name* or its `search_query` (so an acronym's expansion counts), tier is the question's
authority (title 0.30, label 0.20, search 0.00), kind is `types::compatible` (a class the entity
can be `+0.15`, only classes it cannot `−0.30`), and breadth rewards a candidate two questions
produced independently. Eligibility: search needs a prefix or better, label needs the same, and a
label match with no description corroboration needs a title answer too.

**First measurement** (offline, surface-first ranking before tuning): 1645 identities vs 1664;
87 losses, 69 gains, 49 reassignments; 8466 requests vs 5416 — and 2416 of those questions had no
cached answer, because the ladder never asked them. Random sample, n=30: **22/30** against 30/30 for
the ladder. The failures are homonyms and adjacency (Phil Eaton → basketball player, Wez → a French
commune, Lolita the film → the novel, an account → a 3D model): an exact surface carried them and
the description could not separate them. Wins: `BBC` → `Q9531`, `Industrial Revolution` → `Q2269` —
the two cases the design was aimed at.

Then the cache was filled online (~3.5k answers in twelve paced passes) and the ranking was tuned
in three rounds. Final shape of the score, all terms measured:

| term | what it says | measured effect |
| --- | --- | --- |
| `tier_prior` | title 0.15, label 0.10, search 0.0 | keeps a title answer ahead of a search hit without gating |
| `surface` | label vs name, hint, and every `alt_name`; a label-less candidate is judged by its description | the hint term is what fixed `BBC`; the label-less proxy the film/book cases |
| `fit` | IDF-weighted unigram + bigram overlap, saturating | the ladder's own measure, unchanged |
| `kind` | `compatible` over **all 19 types** (`+0.15` / `−0.30`) | the arms stay out of the gate; a person cannot be the organisation |
| `breadth` | distinct questions that produced the candidate | small, positive |
| `year` | descriptions agree / disagree on a year (`+0.10` / `−0.20`) | what separates `Gladiator` 2000 from 1992 |
| `mention` | a label-less description that names the entity in full | stops a CEO's title claiming NPR |
| `position` | the provider's own order | small, positive |
| account rule | an account-only entity may claim only via a platform property | 25 of 26 account identities were wrong; see below |
| kind refusal | never assert a candidate whose kind is known impossible, unless something possible exists | 14 of 15 refusals were wrong claims |

Final graph: **1676 identities** (ladder 1664). Precision on a 100-row random sample: **93/100,
95% CI [86%, 97%]**; the ladder's 30-row sample was 30/30, CI [88.6%, 100%] — indistinguishable.

The account bucket was a census, not a sample: 26 accounts carried a Wikidata identity, **25 were
wrong** (`Bear` → a gay-culture term, `Blue` → a colour, `Drake` → a botanist, `sphinx` → a myth),
and the one right answer came from `P2002`. The rule keys on *account-only* entities, so a
`social_media_account` that is also a person still resolves as one.


## Fixed evaluation set and ablation (2026-11-13)

`plan/eval/identity-labels.tsv` holds 200 knowledge entities drawn at random and judged once:
185 correct identities, 15 known-wrong. It never changes. Every ranking change is scored against
it — `kept` (of the 185 correct, how many the build still asserts), `avoided` (of the 15 wrong,
how many the wrong QID is gone), and identity coverage.

Why it exists: the tuning before it judged each build against a *fresh* 60-row sample, so every
observed error became a rule. That cannot converge. The fixed set makes a term justify itself or be
deleted.

The ablation, one term per build, baseline first:

| configuration | kept | avoided | identities | precision on asserted |
| --- | --- | --- | --- | --- |
| baseline: surface + fit + kind + tier + account rule | 177/185 | 3/15 | 1649 | 93.7% |
| + label corroboration (a label with no description overlap needs a title answer) | 178 | 5 | 1662 | |
| + description head noun (kind from `1902 novel by …`, ±0.10/0.15) | 180 | 6 | 1682 | |
| + year agreement (±0.10/0.20) | 181 | 6 | 1693 | **95.3%** |
| + mention penalty | 181 | 6 | 1692 | no gain, one identity lost — removed |
| + position + breadth | 181 | 6 | 1704 | no precision gain — removed |

Then a **refusal floor** — a minimum absolute score, because a pool of one has no runner-up to lose
to — swept on the same set:

| floor | kept | avoided | identities | precision on asserted |
| --- | --- | --- | --- | --- |
| 0.00 | 181/185 | 6/15 | 1689 | 95.3% |
| 0.50 | 181/185 | 10/15 | 1637 | 97.3% |
| 0.55 | 179/185 | 12/15 | 1610 | 98.4% |
| **0.60** | 178/185 | 14/15 | 1576 | **99.4%** |
| 0.65 | 177/185 | 14/15 | 1554 | 99.4% |
| 0.70 | 177/185 | 14/15 | 1539 | 99.4% |

**0.50 shipped** (decided 2026-11-13): 181/185 correct kept, 10 of 15 known-wrong withheld,
1637 identities. The four claims it admits over 0.60 are `Billy Costigan` → *The Departed*,
`Sten Mk III` → the Sten family, `All Things Are Full of Gods` → the hardcover edition, and
`Death Bunny` → a short story. The first two are adjacent-by-design (a character and the film
about them, a variant and its family); the edition is the work's edition. `Death Bunny` is the one
genuine homonym — capture 70 is a meme of a Lily Seika Jones watercolour that has **no Wikidata
item**, so no scoring change can resolve it and the floor is the only lever: the search answer for
the name returns only other referents (`The Death of Bunny Munro`, `Death to the Easter Bunny!`,
a Rupert Bunny painting). 0.60 refuses it at the cost of seven real identities and 61 rows.

**Rejected after measurement:** splitting `Family::Work` into film / book / painting / game, with
the QIDs already mapped. On top of the floor it scored 1580 identities, 178 kept, **13** avoided —
one known-wrong claim returned — so it was reverted. The kind confusion it targets is real, but the
head-noun term already covers enough of it that the split is neutral-to-negative on this set.

Final: **1576 identities, 99.4% precision on asserted identities** (178/179), 21 refusals (7
correct entities given up, 14 wrong QIDs withheld). Two heuristics that earlier versions carried —
the mention penalty and source breadth — were removed on this evidence, and the kind-refusal veto
and contradiction sort were removed with them.


## Open questions to settle before touching behaviour

1. ~~Two-fits: option A, B, or C?~~ **A shipped** — see the measurement above; the lone-word gate stays on the answer corpus.
2. Is `confidence` a step's label or a probability? This decides whether `project`/`evidence`
   should keep sorting on it.
3. ~~May a `Degraded` answer claim, or only answer empty?~~ **No — and the discarded claim is logged.** See below.
4. ~~Does the title step need its own name-relatedness rule?~~ **Answered by the field**: a title
   answer is tier 0 and its surface still has to bear the name, so it is the same rule with a
   stronger prior — measured, and no separate gate survived.
5. ~~Should `SurfaceTitle` become a rule on the title question?~~ **Moot**: `SurfaceTitle` is gone,
   reusing the title answer (`answer.rs`), and `inspect` now decides through the same
   `aggregate::decide` the resolver does, so it is no longer a second implementation.

## Staged asking: measured twice, declined (2026-11-13)

The field asks every question for every entity, so a rebuild spends ~3,500 transport calls: the
three exact questions batch 50 per call (~100 calls together), while the two CirrusSearch questions
cannot be batched at all — `search_label` 2,279 calls, `search_hint` 1,146. Provenance over the 1,601
field-decided identities (the other ~36 are handle matches, which bypass the field) prices them:

| question | winners it produced | sole source of the winner |
| --- | --- | --- |
| `label` (SPARQL) | 1291 | 63 |
| `title` (SPARQL) | 1240 | 28 |
| `search_label` | 1249 | 91 |
| `search_hint` | 517 | 11 |

Two stagings were built, each keeping the single `aggregate::decide` and one combined field — staging
the *asking*, never the admission:

| staging | requests | identities | fixed set | verdict |
| --- | --- | --- | --- | --- |
| every question (shipped) | 8466 | 1637 | 181/185, 10/15, 5 still wrong | — |
| search only where exact answers refused | 6108 | 1647 | 177/185, 10/15 | **declined** |
| search unless title *and* label agree | 6825 | 1642 | 179/185, 10/15 | **declined** |

Both fail the gate (kept must not fall). Refusal is not a signal of weakness: `Heat`, `Hunter
Hancock`, `John Roberts` and `Mont-Saint-Michel` were all claimed by a wave-one winner that search
would have outranked. Confidence does not separate them either — they sat at 0.77–0.91 with 745
other claims at ≥ 0.90 — and corroboration does not: Hunter Hancock and John Roberts are named by
both the title and the label, and search still finds the item the description supports.

So search stays asked for every non-handle entity. The cost is deferred rather than solved: an
offline rebuild answers 7,956 of 8,466 questions from the cache, and only a first-contact run pays
the unbatched calls.

**Renamed 2026-11-13:** `pool` → `aggregate`, `Pooled` → `Aggregate`, and the collection it ranks is
the *field* (`aggregate::decide(entity, &field)`). A pool implies undifferentiated members; this file
ranks them and decides one.

**The words of the design, after the ladder:** a resolver **fans out** — one batch, every
**strategy**, every entity; the answers merge into one **field** per entity; one **decision** names a
referent or refuses with a reason. A **strategy** is a way of asking (`Strategy::Handle`, `::Title`,
`::Label`, `::SearchHint`, `::SearchLabel`); it is *applied*, not selected and not sequenced — the
resolver applies every one regardless of what the others said. "Ladder" and "rung" are retired
vocabulary: they name sequence, authority ordering and early exit, and the resolver has none of the
three. The words survive in this file only as dated history.
