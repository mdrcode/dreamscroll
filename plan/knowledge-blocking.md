# Blocking: retrieval for linking

Status: Research — design in progress, 2026-11-06. Nothing implemented beyond the
current `NameBlocker`; no decision frozen.

Scope: how the linker gets from one observation to a candidate set. The metric
retrieval path (channels, fusion, budgets) and the ownership boundary between
`Blocker`, `LinkDecider`, `Scoring` and the executor. Not the decision model's
internals, and not the evidence/proximity subsystems.

## The layer model

Retrieval is a set of independent **channels** — separate retrieval schemes, each
with its own query shape, index, score, budget and completeness flag. They are not
branches of one predicate. Channels run independently (and in parallel), and each
emits comparable-shaped *hits* even though its scores are not comparable.

| layer | responsibility | owner | executes in | today | target |
|---|---|---|---|---|---|
| L0 definitions | query shape, index, budget, complete? | `Blocker` | Rust (config) | hardcoded in `NameBlocker` | per-channel definitions |
| L0 execution | run one channel's query | `Blocker` impl | **SQL** | full table scan in Rust | indexed queries |
| L0.1 hits | `(entity_id, channel, rank, raw_score)` | `Blocker` | SQL emits, Rust carries | bare `Vec<i32>` | explicit per-channel hits |
| L0.2 union | dedupe by entity id, keep provenance | `Blocker` | **SQL** (`UNION ALL` + `GROUP BY`) or Rust merge | n/a — one channel | distinct step, observable |
| L1 fusion + cap | RRF, global budget, complete-channel exemption | `Blocker` | **Rust** | none; id order, unbounded | RRF + budgets |
| L2 features | comparison vector for one pair | `LinkDecider` + `Scoring` | **Rust** (full rows) | `Features { name, description, ty }` | full vector |
| L3 decision | attach-or-create, and which entity | `LinkDecider` + `Rank` | **Rust** | `worth_merging` conjunction | calibrated model |
| L4 record | why it attached | `link.rs` executor | Rust → **SQL write** | `Decision { observation, action, identity }` | + features, channel, ranks |

**Ownership boundary: the blocker owns L0–L1; the decider and scoring own L2–L3;
the executor owns L4.** The decider never branches on which channel returned a
candidate — recall policy must not leak into the verdict. Only L4 records the
channel.

Consequences of that line:

- The blocker returns whole `knowledge_entity` rows, because L2 needs `name`,
  `alt_names`, `description`, `alt_descriptions`, `types`, `confidence`. An
  id-then-refetch design would put a query inside the decider.
- Per-channel `LIMIT` is the SQL-side bound (rows fetched = Σ budgets); RRF and the
  global cap are the Rust-side bound. The fusion formula never exists in SQL.
- Normalization is neither layer: it lives in SQL objects (collation, text search
  configuration, generated column), authored by schema sync.

## Channels

| channel | query source | index | score | completeness |
|---|---|---|---|---|
| identity | `(scheme, external_id)` | b-tree | exact | complete |
| surface | folded name / handle / display name | GIN `array_ops` over folded surfaces | exact, token-equal | complete |
| lexical | folded text | GIN `tsvector`, GIN `trgm` | tiered / similarity | approximate, budget k |
| phonetic | `daitch_mokotoff(name)` | GIN over code array | code overlap | approximate |
| dense | embedding of name + description | HNSW | cosine | approximate, budget k |
| co-capture | entities in the same illumination / capture | join | count | complete w.r.t. the join |
| alias | every surface the entity has worn (`alt_names`) | the surface index | exact | grows monotonically |
| personal prior | entities this user already linked | user→entity join | prior | complete w.r.t. the join |

Normalization options for the surface channel, all DB-side so the app writes raw
text: core PostgreSQL has an `IMMUTABLE`-expressible fold (`btrim`,
`regexp_replace`, `lower`, `translate` — 21/21 vectors of
`knowledge::normalize::comparable` reproduced by a SQL expression, the 58-code-point
diacritic table aligned), stored as an expression index, a `STORED` generated column,
or a trigger; nondeterministic ICU collations cover case/accent/punctuation at
comparison time but ignore inner punctuation; `unaccent` covers diacritics only.

## The query, and what happened to the "splitter"

There is no split stage. What crosses the layer boundary is the observation, raw:

```rust
struct Query<'a> {
    observation: &'a user_observed_entity::Model,  // raw columns, unfolded
    illumination_id: i32,                          // co-capture context
    user_id: i32,                                  // personal prior
}

trait Blocker {
    async fn retrieve(&self, query: &Query) -> Result<Candidates>;
}
```

| former "splitter" job | new home |
|---|---|
| which observation fields are searchable, deduped | **channel definitions** (`surface` reads all surfaces, `lexical` the name, `dense` name+description, `identity` the handle pair, `co-capture` the illumination) |
| folding (`comparable`) | **SQL objects**, applied to the bound parameter by the same definition that normalizes the indexed side |

Channels bind raw fields and fold inside their own predicate, so a channel need not
be SQL at all. `normalize::comparable` remains in Rust for `project`'s `alt_names`
dedupe and `evidence::Claim::key`; it simply stops being on the blocking path.

Per-channel invariant: a surface that folds to nothing yields no rows. In SQL this
holds by accident (`blocking && '{}'` is false), so it is stated as a contract.

## Adding and retiring channels

A channel is one row in the definition table and one query; it changes no other
layer. That makes channel value an empirical question, which is the point:

- The existing exhaustive `NameBlocker` over a corpus is the **oracle**. For each
  observation it enumerates every folded-surface and token-key match, so it defines
  ground truth for recall without any labelled data.
- **recall@k per channel** = for each observation, does the oracle's match (the
  entity the exhaustive scan found) appear in that channel's top k? This is the
  measurement that decides whether a channel earns its place, and it needs no graph
  writes.
- **Fusion lift** = does the union beat the best single channel, and does adding a
  channel move any decision (compare `Decision` vectors, or the projection snapshot,
  before and after).
- Because channels are independent, a corpus-level A/B is: enable channel set A,
  link, snapshot; enable set B, link, snapshot; diff. That is exactly what
  `snapshot::compare` already does.

The order this implies: **surface + identity first, then the oracle harness, then
measure each candidate channel before building it.** The measured evidence already
says the lexical tier adds no decision today at the current name floor.



## The fold, audited — 2026-11-06

`normalize::comparable` was written as a prototype. Measured against 2,126 live entity
names and 2,560 observation names, it is close to right, with two defects that cost
real recall and several latent ones:

| candidate widening | new (observation, entity) pairs | verdict |
|---|---|---|
| collapse whitespace adjacent to `. , ; : ! ?` | 3 (`A. W. Moore` ⇄ `A.W. Moore`) | **fix in the fold** |
| remove inner `" “ ” « »` (keep `'` / `’`) | 3 (`"Fast Eddie" Felson` ⇄ `Fast Eddie Felson`) | **fix in the fold** |
| fold ligatures `æ → ae`, `œ → oe`, `ß → ss` | 0 today | **fix in the fold** — the corpus holds `Æon Flux`; an `Aeon Flux` capture would miss |
| Unicode NFC first | 0 rows | **fix in the fold** (free, defensive) |
| remove apostrophes anywhere | 0 today | leave alone — apostrophes are part of names (`O'Brien`, `She's a Whore`) |
| unify separators `- _ / – —` with space | 0 today | **channel generation only** (`spider-man` ⇄ `spider man`) |
| drop a leading article | 2 — `The Bear` ⇄ `Bear` | **channel generation only**; the definite article is meaningful |
| drop all punctuation (skeleton) | 5, of which 3 are `C` / `C#` / `C++` | **channel generation only** |

Base fold yields 39 foreign candidates over 2,560 observations, max 1 per observation:
it is very tight, so these widenings are additive rather than a rescaling.

### Why one policy does not fit the three call sites

| call site | needs | cost of over-merging |
|---|---|---|
| `project::gather` (`alt_names` dedupe) | conservative | one surface variant silently dropped from the display aliases |
| `evidence::Claim::key` (`scheme:handle`) | near-exact | two different accounts resolve to one identity |
| blocking channels | generous | none — `Rank` filters |

So: **one fold plus named generations.** `comparable` = trim, strip wrapping quotes,
remove inner double quotes, strip leading `@`/`#`, strip one trailing `.`, case fold,
Latin-1 + ligature folding, collapse whitespace, collapse whitespace adjacent to
punctuation. `article`, `separators` and `skeleton` become separate functions used
**only** by blocking channel definitions, never by `project` or `evidence` — article and
separator equivalence is a matching hypothesis, not a fact about a surface. (None of the
three exists yet; they arrive with the channels that need them.)

### What was built, and measured (2026-11-06)

One fold for the whole graph: `normalize::comparable` now folds inner double quotes,
whitespace beside punctuation, and ligatures. The separate blocking-only implementation
(`knowledge::recall`) was deleted after the A/B below; `ds_fold(text)` mirrors
`comparable` call for call and `normalize.rs`'s test keeps them identical.

The widenings stay out of `comparable` only in one sense: article dropping, separator
unification and punctuation removal (skeleton) remain **channel generations**, because
each is a matching hypothesis rather than a fact about a surface — `The Bear` is not
`Bear`.

Measured, same 2,560 observations, `reset` + `link` each time:

| | v1 fold | v2 fold |
|---|---|---|
| live entities after link | 2,135 | 2,133 |
| observations attached to an existing entity | 425 | 427 |
| duplicate folded-name groups | 15 | 13 |
| claims / identities | 345 / 345 | 345 / 345 |
| entities with ≥2 members | 257 | 257 |

Then `identity resolve --offline` (the reconcile step: cache-only claims, then the proximity scan
and merge passes):

| | v1 reconciled | v2 reconciled |
|---|---|---|
| live entities | 2,134 | 2,132 |
| claims / identities | 1,863 / 1,842 | 1,862 / 1,841 |
| identified | 1,779 | 1,778 |
| proximities | 22 | 22 |
| entities with ≥2 members | 257 | 257 |

Both pairs collapsed to one entity with three members: `A. W. Moore` (identified) and
`"Fast Eddie" Felson` (provisional). The one fewer claim and identity is that pair's
authority claim now belonging to one entity instead of two. No other metric moved, so the
widenings neither collided handles (`evidence::Claim::key`) nor changed member
distribution.

Migration: none. `alt_names` in existing rows keep the dedupe they were written with; the rebuild
order below recomputes them under the new fold.

## Rebuild order

Six commands, each one job, none invoking another. A rebuild is these, in this order:

```
reset             snapshot the projection, then empty it
link              observations → entities
verdicts replay   a person's recorded folds, applied to the new ids
identity resolve  evidence: identities and claims
proximity scan    derive the candidate batch from shared identities
proximity merge   answer the batch
```

The order is not a preference in three of the six places:

- **`link` first**, because everything else needs entity rows.
- **`replay` before `identity resolve`**, two reasons. `proximity merge` picks a survivor by the projected
  `confidence`, and `replay` reports a pair as `already`/`gone` rather than reversing a direction —
  so if the automatic pass folds a decided pair the other way first, the recorded winner is lost and
  there is no unmerge. And a folded entity is resolved *once*, from the union of both sides'
  surfaces, instead of twice and then shuffled by a merge.
- **`scan` after `identity resolve`**, because its basis is a shared identity: measured on the local corpus,
  of 345 platform identities **0** have more than one holder, while 13 Wikidata identities account
  for all 26 multi-holder claims. A scan before resolution raises nothing.
- **`merge` after `scan`**, because merge answers the batch scan wrote.

`identity resolve` still re-projects every entity it gave a claim to before returning: a later `proximity
merge` reads that confidence, so leaving it stale would fold the wrong side.

**A rebuild is a single-writer operation.** Stop the link worker first. Running these commands
against a live queue has produced both failures it can produce: a link task attaching to an entity a
`reset` had just deleted (`knowledge entity 127951 does not exist`, illumination 680 left permanently
unlinked), and a `proximity scan` taken from a half-built generation (53 rows where the finished
graph supports 13). Neither is a bug in the command that reported it; both are two writers.

Nothing else writes the batch. The live pipeline is `illuminate` → `knowledge_link`, and the link
task links and stops; there is no resolve task, so resolution and both proximity passes are operator
commands. The consequence, stated so it is a decision and not a surprise: a live capture updates the
entities immediately and leaves the proximity batch untouched until the next `proximity scan`. The
table is a snapshot of when it was last asked — which is exactly what its own model doc says — and
asking is now always an explicit act.

## Fusion: reciprocal rank fusion

Channel scores are not comparable. RRF keeps only positions:

```
fused(d) = Σ_c  w_c / (k + rank_c(d)),   k = 60
```

Worked example (`name = "Sten Mk II"`, unweighted, k = 60):

| candidate | surface | lexical | trigram | co-capture | fused |
|---|---|---|---|---|---|
| `Sten Mk II` | 1 | 1 | 1 | — | 0.0492 |
| `Sten Mk V` | — | 2 | 2 | — | 0.0323 |
| `Sten gun` | — | 4 | 3 | — | 0.0315 |
| `Madsen M-50` | — | — | — | 1 | 0.0164 |
| `Sten` | — | 3 | — | — | 0.0159 |

RRF has no notion of channel authority — a co-capture rank 1 outranks a lexical
rank 3. `w_c` is the lever if a channel must dominate. It orders retrieval only; it
never decides.

## Measured, 2026-11-06 (local corpus, 2,126 live entities)

- Value of the fuzzy tiers, measured against the current token key: **0** entity
  pairs at token-set Jaccard ≥ 0.9 that the token key misses; 103 pairs at
  0.5–0.89, dominated by must-not-merge (sequels, brands, person/institution); 5
  pairs differing only in punctuation skeleton, of which 2 are genuine duplicates
  (`A. W. Moore`/`A.W. Moore`, `"Fast Eddie" Felson`/`Fast Eddie Felson`) and 3 are
  the `C`/`C#`/`C++` collision the fold exists to prevent.
- 14 pairs are token-key-equal; 9 folded names have two entities, all identical
  names.
- 112 of 2,126 names are changed by the fold; 2,117 distinct folded keys.
- FTS on real names (`name` only) returns 2–5 rows; `plainto_tsquery('simple', …)`
  returns 0 for `The Last of Us` because it requires every term including articles.
- All-pairs at this size is ≈2.26M comparisons: the exhaustive blocker is the
  correct design at this scale and doubles as the **oracle** for measuring any
  indexed blocker's recall@k.

## Open questions

Blocks the first line of code:

1. **Fold placement.** Nondeterministic collation / text search configuration /
   `IMMUTABLE` function with expression index / function with `STORED` generated
   column / trigger. `alt_names` is an array, so anything set-based needs a
   function (index expressions admit no subqueries; `array_to_string` is `STABLE`).
   Related judgement call: must the blocking fold be *identical* to
   `normalize::comparable`, or merely a superset of it? Blocking may over-generate;
   `project` and `evidence` may not.
2. **`alt_names` in v1**: fold the array through a function, or index the scalar
   `name` only and let the lexical channel cover the rest.
3. **Fate of `changed` / `forget` on the trait.** No-ops for SQL channels, still
   needed by the in-memory oracle. Either keep them on `Blocker` or split a
   `Mutable` extension trait. Touches five construction sites.

Decided by measurement, not argument:

4. Which channels ship beyond surface + identity, and their per-channel budgets and
   weights `w_c` (defaults `k = 100`, `w = 1` are sufficient to start).
5. Whether the illumination is used as context (the batch is decided together, but
   each observation is scored alone today).

Later:

6. Decision model: keep the conjunction, or move to a Fellegi–Sunter comparison
   vector with calibrated `m`/`u`. No threshold is defensible before a labelled
   pair set exists.
7. Provenance in L4 so decisions can be replayed and calibrated later.
8. Transaction visibility: if `link_illumination` ever runs inside a transaction, SQL
   channels see committed state only — the trait takes no connection today, so this
   must be settled before that change, not after.


## References

- [Cloud SQL supported extensions](https://docs.cloud.google.com/sql/docs/postgres/extensions)
- [PostgreSQL generated columns](https://www.postgresql.org/docs/18/ddl-generated-columns.html)
- [PostgreSQL full text: tables and indexes](https://www.postgresql.org/docs/18/textsearch-tables.html)
- [PostgreSQL nondeterministic collations](https://www.postgresql.org/docs/18/collation.html#COLLATION-NONDETERMINISTIC)
- [pg_trgm](https://www.postgresql.org/docs/18/pgtrgm.html), [unaccent](https://www.postgresql.org/docs/18/unaccent.html), [fuzzystrmatch](https://www.postgresql.org/docs/18/fuzzystrmatch.html)
- [pgvector](https://github.com/pgvector/pgvector) — ANN indexing, vacuum, reindex
- [GIN indexes](https://www.postgresql.org/docs/18/gin.html) — pending list, `fastupdate`
