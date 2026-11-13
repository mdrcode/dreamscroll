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
   correct claim" is the rule `types.rs` and `decision.rs` are written to.
2. **Cost.** Wikidata is rate-limited. The ladder exists to spend the fewest calls.
3. **Auditability.** `method` and `confidence` are the recorded reason; `fetch_cache` is the
   evidence.
4. **Replayability.** `plan`/`interpret`/`decision` are pure, so an offline rebuild over cached payloads
   reproduces the decision.

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
| **B** | corpus = admitted everywhere; admit first, then let the shortcut measure over `admitted` | same direction as A; removes the second `Weights` construction | the exact-surface candidate always survives admission, so ordering is nearly free — but it makes a rung that exists to bypass the name rules consult them |
| **C** | the shortcut stops using `fit`: lone exact surface + plain token overlap | narrowest blast radius; no second corpus to synchronise | a third notion of "corroboration" |

Preference: **A** — the only option that makes the doc true, and it deletes two helpers rather than
adding a third notion of fit. B removes a concept but re-consults the name rule the shortcut exists
to bypass.

## Other findings, by severity

1. **`Step::SurfaceTitle` is a decision wearing a request's clothes.** It plans the identical
   `Request` as `Step::Title` and exists only to re-decide the same cached answer after everything
   else refused. The cost is two special cases in two consumers (`resolver::ladder`'s
   `if step == SurfaceTitle`, `inspect`'s two arms) for what is really "the last rule of the title
   question, deferred". The ladder conflates *what we ask* with *how we decide*.
2. **`inspect` re-implements the ladder loop.** It walks `plan::ladder`, does its own cache lookup,
   its own `admit`/`surface_only`/`explained` dispatch, and its own stop-at-first-claim. The steps
   are shared (which is what `plan` was fixed for); the *loop semantics* are not. `plan::Step`'s doc
   claims "used by the resolver and by the eval replay" — **`eval` does not touch this module at
   all**. The only replay of the decision is `inspect`, and it is a second implementation.
3. **Two different name-relatedness rules.** `parse_title` admits a candidate that shares *any*
   content token with our name; `parse_search` demands near-surface, a qualifier suffix (`, …` or
   `(…)`), or subset-of-content-words. Each has a long justification and neither is tested against
   the other. They may genuinely need to differ (a title query was our exact name; a search result
   was generated from text) — but that is currently an accident of authorship, not a stated rule.
4. **Confidence is neither a rung label nor a calibrated probability.** It is a float mixing
   hard-coded rung values (0.95/0.85/0.75), a corpus-size-dependent `fit`, and a ceiling. Downstream
   treats it as comparable: `project` takes `best_confidence` as the entity's confidence, and
   `evidence` sorts entities by it. Either it becomes an enum (and consumers stop ranking on it) or
   it gets calibrated against a labelled sample, which does not exist. `types.rs` is the model for
   the second: it is the one place here where measured corpus evidence decided a rule.
5. **A `Degraded` answer can claim.** `admit` accepts `outcome.answered()`, which includes
   `Degraded` — the state produced when CirrusSearch never reached its backend and returned a
   prefix-only result set. That payload is "possibly useful"; for *claims* it is exactly the
   circumstance that produces a wrong QID. Worth a deliberate decision: a degraded search should
   probably be able to answer *empty* and not able to answer *claim*.
6. **`Kind::Handle(&'static str)` defeats `PropertyId`.** `property_query` introduces a newtype
   "because the number is otherwise a bare string in a query, where a typo is invisible", then
   `Kind` carries the property as a raw `&'static str` across the transport boundary.
7. **The ladder is budget-blind.** `Resolver::resolve` receives the `Gateway` and can observe a
   refusal, but not how much allowance is left, so it cannot prefer cheap rungs when a run is
   short. Ordering rungs by value per call is the resolver's business and it has no input.
8. **Test topology is archaeology, not specification.** ~25 single-case tests, most documenting a
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
and the fix that closed them. Status values: `pending`, `done`.

| # | File | Status | Findings / decisions |
| --- | --- | --- | --- |
| 1 | `resolver.rs` | done | R1–R11: free functions, `accumulate` / `last_resort`, the vocabulary sweep. Open: no precedence test, the `Canned` footgun |
| 2 | `plan.rs` | done | P1 `plan::all_steps`, P2 `trim()` measured. Open: `Handle(&'static str)` drops `PropertyId` |
| 3 | `decision.rs` | done | M1–M8: one entry, `choose` private, dead guards, method vocabulary, `Result` refusals, both score questions measured. Open: file size only |
| 4 | `ranking.rs` | in progress | S1 scoring contract rewritten; the two-fits defect (options A/B/C), `terse_and_rare`, hand-rolled `stem`, 4 tests for 347 lines |
| 5 | `types.rs` | pending | best quality-per-line; arms enumerate forbidden families inline |
| 6 | `transport.rs` | pending | `search` mixes three concerns; `sparql` picks its query from `first.kind`; no degraded-branch test |
| 7 | `property_query.rs` | pending | clean; only its coupling to `WikidataKind::Handle` |
| 8 | `query.rs` | pending | `escape` coverage; a comment block inside `format!` |
| 9 | `shape.rs` | pending | maintained by hand, not derived from the builder it must agree with |
| 10 | `answer.rs` | pending | `is_menu`'s description signal is a convention, not a rule |
| 11 | `kind.rs` | pending | `property()` has one caller; carries a bare `&'static str` |
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

### 4. `ranking.rs` — in progress 2026-11-13

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
  *Tracked:* the **rust-stemmers experiment** (replace `tokens::stem` with Porter2 and measure the
  corpus the way M6/M8 were measured) and **unifying the second tokenizer**
  (`linker/heuristic/scoring.rs` splits a folded surface on whitespace; `text::tokens` stems and
  filters). *Foregone, with reasons:* `unicode-normalization` (would move `norm_fingerprint`, so a
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
  now corrected and pinned. That is one reason to stop owning a stemmer. Options, with crates.io
  facts as of 2026-11-13:

  | crate | version | normal deps | replaces | behaviour |
  | --- | --- | --- | --- | --- |
  | `strsim` | 0.11.1 (2024, 1.27B dl) | none | `edit_distance` | identical — same Levenshtein |
  | `rust-stemmers` | 1.2.0 (2019, 42M dl) | serde (already in tree) | `stem` | changes tokens, so the corpus |
  | `unicode-normalization` | 0.1.26 (2026) | tinyvec | `normalize`'s hand-written fold, and the NFD tripwire in `plan/backlog.md` | moves `norm_fingerprint`, so a blocking-index rebuild |
  | `unicode-segmentation` | 1.13.3 (2026) | none | the `split(!is_alphanumeric)` tokenizer | mild; the corpus is ASCII-ish |

  Recommendation: take `strsim` (no tradeoff: it deletes an algorithm we own and returns the same
  numbers); measure `rust-stemmers` the way M6/M8 were measured, since a stemmer changes which
  identities are reached; treat `unicode-normalization` as its own project (`norm_fingerprint` and an
  index rebuild), not a drive-by; skip `unicode-segmentation` until the corpus asks for it.

### 3. `decision.rs` (was `policy.rs`) — in progress 2026-11-12

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

## Open questions to settle before touching behaviour

1. Two-fits: option A, B, or C?
2. Is `confidence` a rung label or a probability? This decides whether `project`/`evidence` should
   keep sorting on it.
3. May a `Degraded` answer claim, or only answer empty?
4. Does the title rung need its own name-relatedness rule, or is it the search rule with a
   different surface expectation?
5. Should `SurfaceTitle` become a deferred *rule on the title question* rather than a pseudo-step —
   and should `inspect` stop being the second implementation of the loop?
