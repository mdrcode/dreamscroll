# Resolution — cache first, providers second

**Status:** Implemented (schema version 14). `dreamscroll_knowledge resolve` runs manually; there is no
scheduled trigger and no enrichment yet. Wikidata is the first provider.

**See also:** [knowledge-canonical-entities.md](knowledge-canonical-entities.md) (the graph this feeds),
[knowledge-evidence-prototype.md](knowledge-evidence-prototype.md) (the measured prototype, and the
failure catalogue this design answers).

## The rule

**Only the executor calls `fetch`.** A provider may say what it *would* ask (`plan`) and interpret an
answer (`parse`), but every request goes through `resolution_cache` first. Adding IMDb cannot forget
the cache, because there is no other door to the network.

```rust
trait Engine {
    fn id(&self) -> &'static str;
    fn plan(&self, core: &CoreView) -> Vec<Request>;          // stage 1: identifiers, exact labels
    fn plan_fallback(&self, core: &CoreView) -> Vec<Request>; // stage 2: full text
    fn parse(&self, request: &Request, raw: &Raw) -> Vec<Claim>;  // pure
    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>>; // the only I/O
}
```

`plan` and `parse` are pure, so the entire decision can be tested from a fixture and re-run over cached
payloads for free. That is what makes iterating on matching rules cost nothing.

## The cache

```sql
resolution_cache
  provider, kind, query, shape      -- unique together: one row per logical request
  outcome                            -- found | empty | failed
  payload      jsonb                 -- the raw answer for this query, so parsing can be re-run
  fetched_at, expires_at
```

Four decisions worth naming:

- **`empty` is a cached answer.** A third of the corpus will never resolve; re-asking a question we
  have already answered is exactly the spam this table prevents. The tests assert a second run of an
  unresolved core makes zero calls.
- **`shape` is in the key.** It hashes *how* the request is built (endpoint, language, limit,
  properties). Change the construction and every old answer — including negatives — is bypassed rather
  than silently wrong. The prototype's stale-negative bug, made structurally impossible.
- **`provider` is `text`, not an enum.** Adding an authority must not be a migration, for the same
  reason the core has no `imdb_id` column.
- **Expiry is stored, not computed.** One comparison at read time, and each row records the policy it
  was fetched under.

`found` means the provider returned **candidates**, not that we identified anything — whether one of
them is the referent is the ranking's decision. `empty` is "nothing has that label", cached like any
other answer.

| outcome | TTL | why |
| --- | --- | --- |
| `found` | 7 d | short enough to notice a new item, long enough that a rerun is free |
| `empty` | 7 d | a niche name may gain an item next week |
| `failed` | **10 min** | a provider that is down is not a fact about the world |

Batching is a transport concern, caching is per query: the executor looks up each planned query, groups
the misses into one call (50 at a time), and writes back one row per query. Partial hits work, and one
warm query costs nothing.

## Providers

An authority is a namespace, so a provider is a trait implementation, not a schema change. `applies` is
not even needed yet — but `plan` returns nothing when a provider has nothing to ask, which is the same
seam: TMDB will simply refuse cores that are not films.

Wikidata implements three request kinds:

| kind | how | batchable | confidence |
| --- | --- | --- | --- |
| `handle:P2002` (also P2003, P2037, P2397, P4033) | SPARQL `VALUES` over handles | yes | 0.95 |
| `label` | SPARQL `VALUES` over labels, `P31` and descriptions alongside | yes | ranked |

One request per batch. There is **no** second request: Wikidata's `wbsearchentities` is a label
matcher, so a disambiguating string like `"Sunset Boulevard 1950 film"` returns nothing. Genuine full
text would need `list=search` plus a page→QID hop, which is deferred.

**A claim is a ranking decision over the candidates the label returned**, not a yes/no on the label.
Each candidate is scored on two signals from text we already hold:

- **name**: 1.0 when the candidate's label matches ours exactly, else token overlap.
- **fit**: half *containment* — how much of what Wikidata says about the candidate is accounted for by
  the capture's description — and half symmetric *overlap*. Containment alone survives our description
  being long; overlap is what separates `Dune` (1965 novel) from `Dune` (the series), whose description
  is a superset of ours.

```
score = 0.40 · name + 0.60 · fit
```

A name blocks; the evidence decides. A claim needs `score ≥ 0.60` **and** a margin of `0.10` over the
runner-up — ambiguity is not a reason to give up (`Sunset Boulevard` has eleven candidates and the
description picks the film), but a near-tie is. A single exact candidate with nothing competing is
identity, claimed at 0.75 without corroboration.

### The hint is a fallback, never a scorer

`search_query` is not part of the score above. It is consulted only when **no** candidate clears the
bar — never to break a tie between two candidates that both fit our description, because that tie is
genuine ambiguity and the hint is our own text echoing back.

Two rules make it safe, both learned by measuring:

1. **The label's own tokens are subtracted first.** `"James Davies anthropologist author"` scores a
   *photograph of James Davies* on "james" and "davies" — words every candidate shares, because they
   are the name. Only the remainder ("anthropologist", "author") carries information.
2. **It cannot override a description that decided.** The `Sunset Boulevard` failure — the hint lifting
   the musical adaptation above the 1950 film — cannot recur, because there the description resolves
   the case and the hint is never read.

Claims from this path are capped at 0.85 confidence, and the method recorded is `search_hint`.

## Confidence and status

`status` and `confidence` are written by `project()`, and by nothing else:

- `identified` when any live identity claim exists, `contested` when a live collision does.
- `confidence` = the best claim's confidence **+ 0.1 for each additional independent scheme**, capped
  at 1.0, where a claim's own confidence is `0.50 + 0.45 · score` (0.75 for a lone exact label).
  Two authorities asserting the same referent is a stronger claim than either alone — this is why
  identities are plural and why a core identified only by a handle is still worth resolving.

## Throttling and provider etiquette

WDQS publishes its limits in the
[User Manual](https://www.mediawiki.org/wiki/Wikidata_Query_Service/User_Manual#Query_limits):

| Rule | What we do |
| --- | --- |
| A good `User-Agent`; non-compliance can mean a full block | `dreamscroll-knowledge/0.1 (https://dreamscroll.ai)` on the client |
| 5 parallel queries per IP | one request in flight; the executor is sequential |
| 60 s of processing time per 60 s per client | 50 labels per query, so a batch is milliseconds of processing |
| 30 error queries per minute | a run stops at the first transport failure that is not a rate limit, rather than retrying into it |
| 429 with `Retry-After`, and a temporary ban for ignoring it | **the run stops**, caches nothing, and reports the interval. A 429 is never recorded as an answer — it is an instruction |
| 60 s query deadline | queries are bounded by the `VALUES` list |

On top of the published rules:

- **`--min-interval-ms`** (default 1000) puts a floor between transport calls, so a walk stays under
  60 requests a minute regardless of how fast the queries return.
- **`--max-requests`** (default 200) caps calls per run. A bug cannot turn into a flood.
- **Cache first**: a warm query costs no request at all, which is the cheapest possible compliance.
- **Failures are cached for 10 minutes** so a sick provider is not hammered — but a 429 is not a
  failure and is not cached.

What is *not* done: resuming automatically after a `Retry-After`. The run stops and the operator
reruns it; with the cache warm, the rerun costs only the requests that had not been answered.

## Running it

```sh
dreamscroll_knowledge resolve --dry-run --limit 20   # what would be asked
dreamscroll_knowledge resolve --limit 20 --max-requests 5
dreamscroll_knowledge resolve --core 20190           # one case, for debugging
```

`--max-requests` caps transport calls (not queries) per run; `--core <id>` resolves a single case. The run
is idempotent: resolved cores leave the candidate set, and unresolved ones are answered from the cache
on the next pass.

`stats` reports the number that matters — cores with a live ontology claim — plus cache rows by outcome.

## What is not here

- **Enrichment.** No `entity` kind, so labels/descriptions/images are not fetched by QID yet.
- **Type validation.** `P31` classes are stored on the claim but never checked against the core's type.
  With ranking in place this is now the remaining wrong-referent risk: descriptions separate a novel
  from a film, but not two films, and `entity_type` is the signal that would.
- **Genuine full text.** Deferred, per the note above on `wbsearchentities`.

### The fallback, measured (2026-10-31)

| step | claims | precision | network |
| --- | --- | --- | --- |
| hint scored in the description (rejected) | 23 | **~61%** — a photograph, an album, a Quebec post office | 0 calls |
| hint as a fallback, name tokens included (rejected) | 23 | ~61%, same failures | 0 calls |
| hint as a fallback, name tokens subtracted | **64** | **~94%** | 0 calls |

The two remaining errors were both *type* errors, not name errors: `Monster` (a brand) matched a
Shawn Mendes audio track, and `Q` (a character) matched a Star Wars comic. Both were retracted through
`retracted_at`. That is exactly what `P31` validation would have rejected outright, and it is the
strongest argument for building the type map next.

Every number in that table came from re-ranking cached payloads: the whole experiment cost nothing.

## The tuning loop

Scoring cannot be tuned case by case — one failing entity is an anecdote, and the fix that makes it
pass may break ten others. So there is a small gold set of entities that should be impossible to miss:

```sh
cargo run --example eval_resolve -- localdev/analysis/entity-promotion/evidence/resolve-gold.tsv
```

Each row is `label <TAB> qid <TAB> keyword`. **The gold is verified against Wikidata first** — the
item's label must equal ours and its description must contain the keyword — so a mistaken gold QID is
dropped rather than counted against the scorer. Of the 25 rows written from memory, 12 were dropped
that way.

### What each change bought

| change | gold |
| --- | --- |
| starting point (`score = 0.4·name + 0.6·fit` against a 0.60 bar) | **0 / 13** |
| decide on `fit`, not the blended score (the name is why the candidate exists) | **8 / 13** |
| weight terms by rarity within the candidate set (IDF) | 8 / 13 |
| score matched evidence saturating instead of dividing by the candidate's length | 8 / 13 |
| light suffix stripping (`painter` / `painting` / `paint`) | **9 / 13** |
| adjacent bigrams, weighted double (`"first president"`) | 9 / 13, +3 corpus claims (HBO, Bach, Armani — all correct) |

The corpus effect of the last three steps together: **+97 claims at ~96% precision**, 0 requests
(cached payloads), measured by reading all 97.

### What is left, and why it is not a scoring problem

| failure | reason |
| --- | --- |
| `Fritz Lang`, `Ridley Scott` | our "film director" vs Wikidata's "filmmaker" — a *synonym*, not a word form |
| `Lake Sevan` | our "body of water" vs "lake" — same |
| `George Washington` | with bigrams: the president scores **0.858** sharing `[first, president, "first president"]`; two statues score **0.847** sharing `[unit, stat, "unit stat"]` — both share a phrase, because "United States" is the subject's country in ours and the sculpture's *location* in theirs. Bag-of-words cannot see the difference in role, so the margin is 0.011. |

And the recurring false positives in the corpus are the same shape: `Q` (a character) claimed a Star
Wars *comic*, `Monster` (a brand) claimed a Shawn Mendes *audio track*, `Kerala` (a state) claimed a
*village in Gujarat*.

Every one of those is a **type** question, not a wording question. Lexical scoring has a ceiling
around 9/13 here, and `P31` → `entity_type` compatibility is what lifts it.

## First measurements (2026-10-31)

| case | result |
| --- | --- |
| `Dune` + "1965 science fiction novel by Frank Herbert" | `Q190192` at 0.95 — runner-up was the *series*, 0.898 |
| `Sunset Boulevard` + the capture's description | `Q193570` (1950 film) at 0.86 — 11 candidates |
| `Nintendo` + "Japanese video game company founded in 1889" | `Q8093` at 0.82 |
| 30 real cores, one SPARQL batch | 12 claims (40%) — previously 3 of 10, and only where the label was unique |

The last row is the point of ranking: the ambiguous majority used to be a dead end, and now it is where
most of the yield comes from.
- **Merges.** A claim for a QID another core already holds is counted as a `conflict` and skipped —
  that is a merge the graph has not made (§Repair).
- **Scheduling.** Manual runs only.
