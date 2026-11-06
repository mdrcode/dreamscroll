# Resolution — cache first, providers second

**Status:** Implemented (schema version 16). `dreamscroll_knowledge resolve` runs manually; there is no
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
    fn plan(&self, knowledge entity: &CoreView) -> Vec<Request>;          // stage 1: identifiers, exact labels
    fn plan_fallback(&self, knowledge entity: &CoreView) -> Vec<Request>; // stage 2: full text
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
  unresolved knowledge entity makes zero calls.
- **`shape` is in the key.** It hashes *how* the request is built (endpoint, language, limit,
  properties). Change the construction and every old answer — including negatives — is bypassed rather
  than silently wrong. The prototype's stale-negative bug, made structurally impossible.
- **`provider` is `text`, not an enum.** Adding an authority must not be a migration, for the same
  reason the knowledge entity has no `imdb_id` column.
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
seam: TMDB will simply refuse knowledge entities that are not films.

Wikidata implements three request kinds:

| kind | how | batchable | confidence |
| --- | --- | --- | --- |
| `handle:P2002` (also P2003, P2037, P2397, P4033) | SPARQL `VALUES` over handles | yes | 0.95 |
| `label` | SPARQL `VALUES` over labels, `P31` and descriptions alongside | yes | ranked |

One request per batch. A disambiguating string like `"Sunset Boulevard 1950 film"` returns nothing to
a label query; that case belongs to the full-text stage below, which runs MediaWiki's own search
(`list=search`) plus a page→QID hop.

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
At least **half** the disambiguating words must be accounted for: one of three cleared the old bar,
which is how `Suspicion` (a Matsumoto novel) claimed a Dürrenmatt one on the word "novel" alone. The
stricter floor costs one correct claim (`A Quiet Place`) and removes one wrong one.

### Stage zero is the article title

A capture's name is often not a Wikidata label at all — it is a **Wikipedia article title**, complete
with the parenthetical disambiguator. `Symphony No. 7 (Sibelius)` is the article; the item is labelled
`Symphony No. 7`. Label search returns **zero** hits for it, and no amount of ranking fixes a query
that found nothing.

So the first request is a title lookup:

```
stage 0  enwiki title → pageprops.wikibase_item   → item, corroborated by description
stage 1  exact label (SPARQL)                     → claim
stage 2  list=search (CirrusSearch)               → ranked, corroborated
```

A title is unique where a label is not: 213 items are labelled `Paris`, but exactly one page is titled
`Paris`. The lookup follows `normalized` and `redirects`, which is how `Saint-Quentin Canal` reaches
`Canal de Saint-Quentin` — a word-order variant no label search finds. Titles that resolve to a
`Q4167410` disambiguation page are refused: a menu is not a referent.

Measured on the 502 provisional knowledge entities (2026-11-01): **171 (34%) resolve by title**, of which 30 are
disambiguation pages, leaving **141 (28%) usable**. It batches 50 titles per call — the cheapest
request in the system — and its claim wins over the label stage because a title that matches is
stronger evidence than a label that matches.

### The search endpoint lies, sometimes — measured 2026-11-01

Wikidata's search can answer **HTTP 200, `success: 1`** with nothing useful in it. This is a known,
documented failure ([Phabricator T284478](https://phabricator.wikimedia.org/T284478), "wbsearchentities
randomly not returning search results"): when the search backend fails, the API used to swallow the
error and return an empty list. The upstream fix was titled *"throw exception on backend failure"*,
and the diagnostic tell, identified by a Wikimedia search engineer, is the response header:

> *"X-Search-Id is attached to responses whenever CirrusSearch has made a request to elasticsearch.
> That suggests that somehow these requests are getting a response without ever hitting the backend."*

Measured on the live API — 20 queries × 5 repeats, 100 requests:

| observation | count |
| --- | --- |
| responses with **no `x-search-id`** and zero results | **4 / 100** |
| queries that produced *different result sets* across repeats | 12 / 20 |

Four responses in a hundred were silently empty. Those are indistinguishable from a genuine "no such
thing", and the cache would have held each for a week.

The second row is a different problem: the result *sets* move between identical requests even when ten
come back, so search is not deterministic. The cache is what stabilises it — but only if the cached
answer is a real one.

**How the pipeline treats an answer now.**

| signal | outcome | TTL |
| --- | --- | --- |
| transport error, or `{"error": …}` in a 200 body | `Failed` | 10 minutes |
| no `x-search-id` (search never reached the backend) | `Degraded` — parsed, but distrusted | 60 minutes |
| answered, but produced no claim | demoted after the stage | 6 hours |
| answered and produced a claim | `Found` | 7 days |

The third row is the general lesson: **a cached answer that produced no claim is a guess, not a fact.**
It may be honest and merely unhelpful, or it may be degraded, and either way re-asking costs nothing
when the query is already on disk.

The failure that actually bit us is the second one: **`cirrussearch-too-busy-error`**
([T297319](https://phabricator.wikimedia.org/T297319)), returned as an `error` body. Twenty-three of our
queries hit it in one pass, and before this change every one of them would have been cached as an empty
result for a week. Retrying those 23 produced **no claims** — the queries are genuinely absent — so what
the fix buys is not recall but honesty: a transient overload can no longer be recorded as a fact.

690 cached search negatives were then deleted and 250 of them re-asked, which produced **one** claim
(0.4%). So stale caching is a real but small effect in claim terms. The least trustworthy thing in the
cache is an `empty` search answer, and the cheapest thing to do about it is re-ask it.

### Stage two is full-text search, and it is the last resort

When the title and the exact label both fail, the resolver asks MediaWiki's own search:

```
stage 1  exact label (SPARQL)             → claim
stage 2  list=search, namespace 0         → candidates, ranked by the same `decide`
```

`list=search` on wikidata.org runs **CirrusSearch**, whose analyser tokenises on punctuation and ranks
by relevance. Two things follow: `A Christmas Carol` ranks the 1843 novella first where a prefix
matcher puts a musical on top, and `J.R.R. Tolkien` matches the item labelled `J. R. R. Tolkien`
because both tokenise to `j r r tolkien`. Page titles in namespace 0 *are* QIDs, so a hit list is
already a candidate set — one `wbgetentities` call then supplies labels, descriptions and `P31`.

**Not `wbsearchentities`.** That endpoint is a prefix matcher over labels and aliases, which is the
wrong instrument for a string that is a spelling of a name rather than a prefix of a label. The
Wikidata developers say so directly: *"wbsearchentities is prefix-only for performance reasons and
there is no way around it. Try to use MediaWikis regular search API."*

An answer only becomes a claim when the winner's label is **our name** — the same surface, or a
trivial respelling (`dostoyevsky`/`dostoevsky`). Measured earlier: without that guard the best-fitting
candidate for `Royal Navy` was *Royal Navy fleet aircraft carrier fleet*, whose description contains
our words verbatim.

Search takes one query per call, so this stage is paced per query — 50-at-a-time bursts are what
earned a 429.

### The margin gets stricter as the evidence improves — measured 2026-11-01

`fit` saturates: `matched_weight / (matched_weight + EVIDENCE_SCALE)`. The ambiguity test then demands an
**absolute** gap of `FIT_MARGIN` in that saturated space, so the bar rises as the evidence gets better:

| case | best fit | runner-up | margin | verdict |
| --- | --- | --- | --- | --- |
| `A Christmas Carol` | 0.856 | 0.831 | 0.024 | refused — the evidence differs by ~20%, the gap is unattainable at this height |
| `The Lowdown` | 0.530 | 0.530 | 0.000 | refused — a genuine tie |
| `Generation Kill` | 0.687 | 0.647 | 0.040 | refused |

At `best = 0.856` the runner-up must fall below `0.756` — 45% less evidence — for the claim to stand. As
fits approach 1.0 the achievable margin collapses toward `1 − runner_up`, so the rule tightens exactly
where the evidence is strongest. The margin belongs on the unsaturated weight, or as a ratio
(`best ≥ 1.2 × runner`), not on the saturated score.

Two things sampling the provisional set showed: short descriptions let homographs through corroboration
(`xAI` → `Q109778862`, *"Xai, a person identified in the Museu da Pessoa collection"*), and a near-tie is
often a **type** question the fit cannot see at all (`Train Dreams`, the 2011 novella versus the 2025
film).

## Lessons this cost us

Three misses, all of the same shape: we searched our own code for a fix instead of asking what the
problem is called.

- **Characterise the input before tuning the matcher.** A five-minute query — how many capture labels
  contain `(`, `:`, `.`? — would have revealed that a large share are Wikipedia article titles, not
  Wikidata labels. 18 of 502 provisional labels carry a parenthetical disambiguator. We tuned a
  matcher for weeks without ever describing the strings they carry.
- **A probe that works where the pipeline fails is the finding.** `wbgetentities&sites=enwiki&titles=`
  resolved `J. R. R. Tolkien` to `Q892` in a session where the label lookup returned nothing. That
  discrepancy was written down as "another option" instead of as evidence that our strings were not
  labels.
- **Name the field for what it holds.** The column was called `label` from the first schema, and the
  name settled the question it should have kept open. It is now `alt_names` alongside `label`, and the
  honest framing is *surface* — the vocabulary of the source, which may be a Wikipedia title, a shop
  page or a journal, never a promise about Wikidata.

## The shape this should take

Entity reconciliation has a published contract: the [W3C Entity Reconciliation Community Group
specification](https://www.w3.org/community/reports/reconciliation/CG-FINAL-specs-0.2-20230410/),
documented by [OpenRefine](https://openrefine.org/docs/technical-reference/reconciliation-api). The
request is *a string, optionally a type, optionally a list of properties*; the response is a **ranked
list of candidates**, explicitly not a decision.

We implemented the string and dropped the other two inputs. That is why every rung of this ladder has
needed its own acceptance rule: with a single signal — lexical overlap — each new data shape demands a
new heuristic, and the resolver now has four stages carrying three different notions of "close enough".

```
input: { surface, type?, descriptors[], identifiers[] }

  identifier   handle → item            exact, no scoring
  title        enwiki title → item      exact, unique (redirects followed)
  label        exact label → items      all share the label; only the text can separate them
  search       full text                ranked
        ↓
  one decide(): name, type agreement, description fit, hint
```

Every rung feeds the same `decide`; they differ only in *which candidates are admitted*. `type` becomes
an input rather than a tiebreak — map `entity_type` to a `P31` set, constrain the search stage with
`haswbstatement:P31=Q…`, and let type agreement corroborate on its own instead of being added to
lexical overlap.

The measured case for doing it: of 50 randomly sampled provisional knowledge entities, 23 had search candidates and
no claim, and the dominant rejection was `fit == 0` — the description must share **two** content tokens,
and real descriptions paraphrase. `Odessa` is the cleanest example: the title stage found `Q1874`, the
correct city, and the claim was refused because "major historic port city on the Black Sea" and "city in
Odesa Oblast, Ukraine" share one token. Type would settle it with no overlap at all.

## Confidence and status

`status` and `confidence` are written by `project()`, and by nothing else:

- `identified` when any live identity claim exists, `contested` when a live collision does.
- `confidence` = the best claim's confidence **+ 0.1 for a second independent *input***, capped at
  1.0, where a claim's own confidence is `0.50 + 0.45 · score` (0.75 for a lone exact label).

  Inputs, not scheme labels. A `p2002` claim is derived *from* the handle — the resolver asked
  Wikidata which item owns that account — so pairing it with the handle's own `claim` row is
  one chain, not two, and must not earn a bonus. Counting labels gave all 26 handle-resolved knowledge entities
  the maximum confidence for no extra evidence.
  Two authorities asserting the same referent is a stronger claim than either alone — this is why
  identities are plural and why a knowledge entity identified only by a handle is still worth resolving.

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
- **`--offline`** is the only way to make zero calls. `--max-requests 0` is rejected: a budget of zero
  used to read as a successful run that resolved nothing, because every answer came from the cache.
- **Cache first**: a warm query costs no request at all, which is the cheapest possible compliance.
- **Failures are cached for 10 minutes** so a sick provider is not hammered — but a 429 is not a
  failure and is not cached.

What is *not* done: resuming automatically after a `Retry-After`. The run stops and the operator
reruns it; with the cache warm, the rerun costs only the requests that had not been answered.

## Running it

```sh
dreamscroll_knowledge resolve                          # default budget of 200 calls
dreamscroll_knowledge resolve --limit 20 --max-requests 5
dreamscroll_knowledge resolve --offline --limit 20     # cache only, no transport calls
dreamscroll_knowledge resolve --knowledge entity 20190             # one case, for debugging
```

`--max-requests` caps transport calls (not queries) per run; `--knowledge entity <id>` resolves a single case. The run
is idempotent: resolved knowledge entities leave the candidate set, and unresolved ones are answered from the cache
on the next pass.

`stats` reports the number that matters — knowledge entities with a live authority claim — plus cache rows by outcome.

## What is not here

- **Enrichment.** No `entity` kind, so labels/descriptions/images are not fetched by QID yet.
- **Type validation.** `P31` classes are stored on the claim but never checked against the knowledge entity's type.
  With ranking in place this is now the remaining wrong-referent risk: descriptions separate a novel
  from a film, but not two films, and `entity_type` is the signal that would.
- **Genuine full text.** Implemented 2026-11-01: `list=search` (CirrusSearch), see *Stage two*.

### The fallback, measured (2026-10-31)

| step | claims | precision | network |
| --- | --- | --- | --- |
| hint scored in the description (rejected) | 23 | **~61%** — a photograph, an album, a Quebec post office | 0 calls |
| hint as a fallback, name tokens included (rejected) | 23 | ~61%, same failures | 0 calls |
| hint as a fallback, name tokens subtracted | **64** | **~94%** | 0 calls |
| search lookup for empty labels (stage two) | **11** | **100%** (of 220 lookups) | 220 calls |

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

### Replaying the whole corpus

The gold set answers "did this change break a case I care about". The corpus replay answers "what
would change across everything I have". It walks the same ladder the driver walks — `plan`, then
`plan_fallback` — over `resolution_cache`, with no transport:

```sh
cargo run --bin dreamscroll_knowledge -- eval resolve            # counts, by method
cargo run --bin dreamscroll_knowledge -- eval resolve --verbose  # every claim, by name
```

Seconds, against the ~640 fetches a live pass cost. This is how the 2026-11-06 admission changes were
measured: one gate changed at a time, every new claim read, and the two that looked wrong traced to a
loosened rarity bound rather than shipped (see the commits for `resolve:`).

`src/knowledge/eval/resolve.rs` names the two ways a replay can lie: walking a ladder the driver does
not walk (the first version skipped `plan_fallback` and reported zero), and counting an uncached
entity as undecided (it is a gap in the evidence, and is reported separately).

`examples/eval_resolve` still reads `core_knowledge_entities`, a table that no longer exists; the
gold-set path needs porting before it runs again.

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
| 30 real knowledge entities, one SPARQL batch | 12 claims (40%) — previously 3 of 10, and only where the label was unique |

The last row is the point of ranking: the ambiguous majority used to be a dead end, and now it is where
most of the yield comes from.
- **Merges.** A claim for a QID another knowledge entity already holds is counted as a `conflict` and skipped —
  that is a merge the graph has not made (§Repair).
- **Scheduling.** Manual runs only.
