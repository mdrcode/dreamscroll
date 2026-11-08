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

**A cache-only replay is free only while the answers are live.** A search answer that produced no
claim is stored with `UNPRODUCTIVE_TTL` (6 h), so a replay more than six hours after the last online
run finds almost nothing. Measured 2026-11-07: `resolve --offline` planned 2,046 requests and matched
**2** live answers — the two belonging to the two vetoed entities — because the other 2,257 `empty`
rows had expired overnight. Offline replay is for iterating inside a working session, not for the
next morning.

Batching is a transport concern, caching is per query: the executor looks up each planned query, groups
the misses into one call (50 at a time), and writes back one row per query. Partial hits work, and one
warm query costs nothing.

## Many authorities, one claim edge

Today Wikidata is the only provider, and the claim model must not know that. What is authority-neutral
already: `knowledge_identities.scheme` names the namespace, `source` names who made the claim, and a
veto is keyed by `(scheme, external_id)` rather than by a QID. The target is **at least one claim per
entity**, from whichever authorities recognise it.

Two things were made authority-aware on 2026-11-07, both behaviour-identical with one provider:

- **Planning asks "has *this* provider claimed it?"** — `candidates()` filtered `source != 'platform'`,
  which means "any authority". With a second authority, an IMDb claim would silently stop Wikidata
  resolving that entity. It now filters on the configured providers' ids.
- **A veto is enforced at the claim write** (`store_claim`), not inside the Wikidata parser, so a new
  provider inherits the rule without knowing it exists.

Baseline to hold the line against: **0.89 claims per live entity, 0.86 authority claims per identified
entity** (2026-11-07).

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

## Coverage, measured 2026-11-07 — 500 random **entities**, not 500 claims

A precision sample conditioned on "has a claim" cannot see the population the resolver declines, so
the second draw took **500 live entities** in `md5(id)` order, 377 of which hold a Wikidata claim and
123 of which do not. Two questions per row: is the claim the described thing (claimed rows), and
could a resolver identify this at all (unclaimed rows).

| | count | share of 500 |
| --- | --- | --- |
| claim correct | 329 | 65.8% |
| claim wrong | 36 | 7.2% |
| claim unclear | 12 | 2.4% |
| no claim, resolvable | 62 | **12.4%** |
| no claim, correctly so | 61 | 12.2% |

- **Precision (claimed): 90.1%** (329/365, `[86.6, 92.8]`), consistent with the 91.6% claim-level draw.
- **Coverage: 75.4%** (377/500).
- **End-to-end correct: 65.8%**; the ceiling if every resolvable miss were claimed is **78.2%**.
- Half of the unclaimed population (62/123) is identifiable and left unresolved — a larger defect than
  precision, and invisible to the claim-level sample.

Two mechanisms, one of them wrong:

1. **A disambiguation page kills the ladder.** `The Silence of the Lambs` (1991 film) has the correct
   item — `Q133654`, "1991 film directed by Jonathan Demme" — sitting in its *own cached label answer*,
   and still resolves to nothing: the title answer is a menu, so the last-resort pass cannot fire, and
   the label rung refuses with "no disambiguation hint to fall back on" even though the capture
   description ("1991 psychological horror thriller film directed by Jonathan Demme") overlaps the
   candidate's heavily. `fit` should be near 1 there; that refusal is a bug to chase.
2. **Genuinely thin captures.** `Commission of Tears` (a 1999 novel) has no enwiki title and no label
   item; search returns unrelated papers. Correctly unresolved.

Recheck the two-row disagreement first: the sample query saw no claim for `John Singer Sargent`
(entity 231793) while the table holds `wikidata:Q155626` — so either the LATERAL ordering or the
sample's join is dropping live claims, which would understate coverage.

## Precision, measured 2026-11-07 — 500 claims, model-judged, before and after

**500 live Wikidata claims**, ordered by `md5(entity_id | qid)` so the draw is reproducible without a
seed, one row per claim. Each row was judged by a model (`judge_batch`, `typesafe/jev-1.14`) against
what the capture actually says — the entity's name, type and description versus the item's label and
description — with `correct` / `wrong` / `unclear`.

| | before the ordering change | after |
| --- | --- | --- |
| judged | 488 | 488 |
| correct | 438 | **447** |
| precision | 89.8% `[86.7, 92.1]` | **91.6% `[88.8, 93.7]`** |
| corrected for judge error | 91.5% | **93.0%** |

The judge under-detects correct claims: of the 41 it called wrong, 7 are plainly right on reading
(`John Roberts` → the Stanford GSB economist the capture describes, `Dan Wang`, `20th Century Fox`,
`Manila in the Claws of Light`, `Pomegranate Seed`, `Grok`, `Alan Durband`). That 17% false-negative
rate is what the corrected row applies; false positives were ~0 in a 15-row spot check.

**The paired comparison is the evidence for the delta, not the two point estimates** — their CIs
overlap and a 500-claim sample cannot resolve 1.8 points. Twelve of the 500 rows changed hands and
the same judge re-judged them: **9 wrong → correct, 0 correct → wrong, 3 unchanged.**

By method after the change:

| method | correct | judged | precision |
| --- | --- | --- | --- |
| `wikipedia_title` | 352 | 374 | 94.1% |
| `exact_label` | 80 | 93 | **86.0%** |
| `search_hint` | 5 | 6 | 83% |
| `search_lookup` | 4 | 5 | 80% |
| `search_ranked` | 6 | 10 | **60%** |

`exact_label` is the weakest high-volume method and the next target. The 41 errors fall into four
kinds: a different person with a common name (6), a work against its adaptation, franchise or series
(6), a broader or narrower class than the capture means — a gun *family* for a variant, a *commune*
for an island, river *barriers* for one chain (4), and related-but-distinct things (a publisher for a
journal, a book for the miniseries made from it, a lens for a magazine).

The earlier 96.3% came from a hand-judged 82-row sample and is **not comparable** to this: the model
judge is stricter than reading by hand was, and the same rows judged both ways would not agree.

## Precision, measured 2026-11-06

Random sample of **100 of the 1,877 identified entities** (seed `0.20261106`, one claim each), judged by
reading the stored label and description against the entity's name and description. Nine turned out to
be handle identities (`scheme = capture`), not Wikidata claims, leaving **91 judged**.

**83.5% correct (76/91), 95% CI [74.6, 89.7].** Counting the nine handles, 85 of 100.

| method | correct | total | |
| --- | --- | --- | --- |
| `exact_label` | 62 | 74 | 83.8% |
| `wikipedia_title` | 10 | 11 | 90.9% |
| `search_ranked` | 2 | 4 | |
| `P2002` | 2 | 2 | |

The surprise is the first row: **an exact label is the least reliable method.** A label match says
nothing about *which sense* of the label the entity is — twelve of the fifteen errors came from it.

| failure | n | example |
| --- | --- | --- |
| disambiguation page claimed as an item | 4 | `Rogue Heroes` → a menu page |
| concept instead of the work | 8 | `Dogma` → the concept, not the 1999 film |
| container instead of part | 2 | `Frontiers in Human Neuroscience` → `Frontiers Media` |
| a different work | 1 | `Suspicion` → Dürrenmatt's novel, not Matsumoto's |

Two of these are arguable as related-sense rather than wrong (`BBC` → `BBC News`; `The Summit of the
Gods` → the manga the film adapts). One reader's judgement; the sample supports ±7.5 points.

### The disambiguation check compares the wrong class

`parse_title` refuses a candidate whose `class == Q4167410`. That misses the subclass: three of the four
sampled pages carry **`Q22808320`** ("Wikimedia human name disambiguation page"), and `Q115697563`
carried no classes at all. Population-wide, **34 of the 1,877 identified entities hold a disambiguation
page** — definitionally wrong claims, and a `description ILIKE '%disambiguation%'` test catches every one
of them, because it is in the description we already store. The `classes` test is the one that misses.

### The other eight are the case for `type` as an input

Nothing lexical separates `Dogma` the film from `dogma` the concept; `P31` does, immediately. **1,365 of
1,948 identities already carry a class list** — fetched alongside every description, and used for
nothing. This is the argument two sections below, now with a population measurement behind it instead
of an example.

### Fixed 2026-11-06

The menu check moved from `parse_title` to the gate every rung passes through
(`parse_explained`), and matches either the class list or the description: `Q22808320` is a subclass
of `Q4167410`, and three of the pages carried no classes at all, so equality on the parent fired for
none of the 34.

Repair: the 34 claims were retracted and re-decided offline (`identity resolve --offline`, every
request already cached), then `identity reproject` — new verb, because `status` is derived from live
claims and nothing recomputed it for an entity that stopped claiming.

| | before | after |
| --- | --- | --- |
| live menu claims | 34 | **0** |
| claims | 1,960 | 1,954 |
| identified | 1,877 | 1,873 |

Of the 34: **23 re-claimed by exact label, 3 by capture handle, 8 unresolved.** Judged against the
entity's own description, 19 of the 23 are right and 4 are not — all four are same-name people picked
by the evidence: `Arthur Gibson` (aviation photographer) → a 19th-century cricketer, `David Vernon`
(British academic) → an Australian writer, `Jack O'Connell` (actor) → a footballer, `Colin Adams`
(Character.ai CEO) → an unnamed "executive". The last three are the `type` case, not the menu case.

On the same 100-entity sample: 76/91 → **79/90 (87.8%)**. `Rogue Heroes` left the identified set
(its series is a separate entity, `SAS: Rogue Heroes`, already correctly claimed), and no new error
was introduced among the sampled entities.

### Rebuilt 2026-11-06

The 1,609 resolver-owned claims were retracted and replayed under the current parser (fold v2, title
guard, IDF floor, terse rule, menu gate); the 345 `capture` handle claims were left alone, because a
capture's own account is not a decision the resolver can re-derive.

| | before | after |
| --- | --- | --- |
| claims | 1,609 | 1,609 |
| identified | 1,873 | 1,873 |
| `wikipedia_title` | 117 | 128 |
| `exact_label` | 1,383 | 1,379 |
| `search_hint` | 22 | 17 |

**Eleven entities changed referent, and nine of them are strictly better.** Every gain came from the
title rung now winning where the label rung used to: `Jane Eyre` → the 1847 novel (was a 2006 TV
adaptation), `Dustin Hoffman` → the actor (was an SNL sketch), `Andrey Markov` → the elder (was his
son), `Instagram` → the network (was "Instagram post"), `John Singer Sargent` → the painter (was a
catalogue raisonné), `Back to the Future Part II` → the film (was a 1990 video game), `MS-DOS` → the
operating system (was "DOS"), `Age of Empires` → the series, `Starlink` → the constellation.

One is worse: `Los sorias` moved from the novel to its *1998 edition* — a granularity step down that
the title rung cannot see. One is still wrong in both directions: `Q` (the Star Trek being) → a
2004 short fiction by John Grant.

The lesson is the precision sample's, confirmed at scale: **the exact label is not evidence of
sense.** Nine of eleven corrections came from preferring the *article* over the *string*.

### Sampled again after the rebuild — 2026-11-06

Same seed, same method, fresh draw over the rebuilt 1,873. Seventeen were capture handles, leaving
**83 judged**.

**92.8% correct (77/83), 95% CI [85.1, 96.6].** Counting the seventeen handles, 94 of 100.

| method | correct | total |
| --- | --- | --- |
| `wikipedia_title` | 10 | 10 |
| `exact_label` | 63 | 68 |
| `search_ranked` | 2 | 2 |
| `search_lookup` | 0 | 1 |
| `P2002` | 2 | 2 |

Six errors, and every one is sense or granularity — **no menu claims**:

| entity | claimed | should be |
| --- | --- | --- |
| Linkup | a French band | a web-search API provider |
| Los sorias | its 1998 edition | the novel |
| Micah | a male given name | the prophet |
| STUNNER | a 2025 EP by Ten | an X account |
| Phil Weir | a Scottish footballer (1901–1963) | a British naval historian |
| Freedom of Speech | the *concept* | the Rockwell painting |

The intervals overlap ([74.6, 89.7] before, [85.1, 96.6] now), so the +9.3 points is not established by
sampling alone. What establishes it is the exhaustive delta above: 34 claims that were definitionally
wrong are gone, nine were corrected, one got worse. The sample agrees with the audit rather than
proving it independently.

`exact_label` is still the rung to distrust — 5 of the 6 errors, and the one `search_lookup` error is the
edition.

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

## Built: `P31` as a one-sided filter (2026-11-07)

The resolver now asks for `P31` with the label query (`OPTIONAL`, so an item without a class is still
a label match — measured cost at 25 labels: 1.9 s against 2.5 s for the plain query, inside the noise)
and refuses a candidate whose **recognised** classes all belong to another family than its
`entity_type` expects. `types.rs` holds the table; every QID in it was checked against Wikidata.

Three properties make it safe to keep:

- **Unknown never rejects.** An unrecognised class passes, so the table being incomplete costs a
  rejection, never a correct claim.
- **Every recognised class must disagree.** A candidate that asserts `film` and `organization` is not
  refused for a movie.
- **Unmapped types never reject.** `publication`, `place`, `organization`, `event`, `brand`, `product`
  and `meme` are deliberately absent. Measured on the first rebuild, mapping them removed *correct*
  claims: `Rotten Tomatoes` is a business (`Q4830453`), the `Locarno Film Festival` an organization
  (`Q43229`), the `Twenty-six Martyrs of Japan` a group of humans (`Q16334295`). A capture's
  `entity_type` is a guess; Wikidata's classes are precise about a different axis.

**Measured on the full re-decide.** Removed, all correct: `Irene` (a hurricane, not a character),
`Flamingo` (a bird taxon, not the software), `Kabugo` (a family name, not a person), `Billy Costigan`
(*The Departed* itself, not a character), `SimClusters` (a scientific article, not the algorithm).
No correct claim was removed by the filter.

Two classes were missing from the first table and one type had no rule, found by sampling the
rebuilt graph (**92.9%, 78 of 84 judged**):

- `male given name` is **`Q12308941`**, not `Q202444` — Wikidata has one class per gender, and the
  table knew only the neutral one. `Micah` was claiming a given-name page through it; once refused,
  the next answer offered the prophet (`Q2804969`).
- `brand` had no rule. `Westinghouse` held a family-name page (`Q110089588`, P31 `Q101352`); a brand
  is not a word, so a brand now rejects a candidate that only asserts a name.

With those two fixed, the same 84 sampled rows read **80 correct (95.2%)**. The four that remain are
the shape of the ceiling: three are *same-family* collisions, which `P31` cannot see at all
(`Brett Scott` the campaigner against an Australian footballer; `From the Earth to the Moon` the
miniseries against Verne's novel; `Lolita` the 1962 film against the 1955 novel), and one is an item
with no `P31` stored (`The Ancient City` → `Q15661340`).

**What it cannot do.** Unrecognised classes pass, and that is where the remaining wrong claims live:
`Choo` (illustrator) → a river in Gabon (`Q4022`), `Heavy Metal` (1981 film) → the comics magazine
(`Q867242`), `Aardwolf` (a MUD) → a watercolour (`Q18761202`). Those six were recorded as vetoes
rather than chased with a longer table. Same-family collisions — two humans, two films — cannot be
`P31` at all: `Phil Weir` (naval historian) against a Scottish footballer are both `human`.

### Two things the rebuild exposed that are not about `P31`

- **`method` is stored on the identity, not the claim.** After a full re-decide the histogram reads
  `wikipedia_title` for 1,307 claims because the title rung claims first and the identity row is
  rewritten by whichever claim lands — not because titles produced them. Per-claim provenance needs
  `method` on `knowledge_claims`.
- **A lone exact label is still claimed with no corroboration.** `Choo` → a river is that rule, not
  the type filter: one candidate matched the name exactly and nothing was left to compete with it.

### Sampled again after the rebuild and `P31` — 2026-11-07

Fresh draw (seed `0.20261108`, 100 of 1,871 identified; 18 were capture handles, leaving **82
judged**): **91.5% correct (75/82), 95% CI [83.4, 95.8]**.

Seven wrong, and every one now has a diagnosis rather than a label:

| entity | claimed | why | what would fix it |
| --- | --- | --- | --- |
| `Orthodoxy` (Chesterton) | the *concept* `Q383258` | **no `P31` stored at all** | the lone-exact-label shortcut, below |
| `Adam Singer` (exec) | `Q110731458`, P31 `Q5` human | a different human | nothing in `P31` — same family |
| `Revolver` (Beatles album) | a handgun `Q170382` | class `Q124078422` *weapon type* is unrecognised | add a weapon/artifact family |
| `Liquidation` (2007 series) | corporate concept `Q885621` | class `Q3535173` *organizational life cycle* unrecognised | recognise it as a concept |
| `MUME` (the MUD) | a Papuan village `Q25906893` | class `Q12488913` *kampung of Papua* unrecognised | recognise it as a place |
| `The Great Betrayal` (Buchanan) | a book by Naumenko | both written works | nothing in `P31` — same family |
| `Poetics` (Aristotle) | the discipline `Q835023` | class `Q11862829` *academic discipline* unrecognised | recognise it as a concept |

Five of the seven are table gaps with the class named; two are the structural ceiling. The sample's
CIs overlap the previous three (83.5 → 92.8 → 92.9 → 91.5), so nothing here establishes a trend in
either direction — but the *shape* has changed: earlier samples' errors were disambiguation pages and
concepts, and now every error has a specific rule or a structural reason.

**Closed the same day.** The four named classes are in the table — `artifact` for *weapon type*,
`concept` for *organizational life cycle*, *academic discipline*, *academic major* and *literature
term*, `place` for *kampung of Papua* — and `television_show`, the spelling the graph actually uses
for its 23 series, joined the work arm (`television_series` was in the table and in no entity).
Re-decided from cache: **`Poetics` → the book by Aristotle, `Liquidation` → the Russian television
series, `Revolver` and `MUME` provisional** (no correct item was offered). The same 82 rows read
**79 correct (96.3%)**; the three that remain are two same-family collisions and `Orthodoxy`.

### The shortcut now speaks last

The lone-exact-label shortcut claimed the first answer whose label matched our name, on any rung, at
0.75, with no corroboration. On a *label* query that is evidence — the query enumerates every item
with the string, so one result means the name is unique. On a *title* query it is vacuous: a page
maps to one item by construction, so "there was only one answer" says nothing. Wikipedia had already
resolved the ambiguity by directory (`Orthodoxy (book)`), and we re-resolved it by accident.

Two changes, both in the ordering rather than the weights:

- `decide`'s shortcut is scoped to `Kind::Label`.
- A new last-resort pass (`policy::surface_only`) claims the exact-title article **after** every
  corroborating rung has been asked and refused. The title was the first speaker; it is now the last.

Measured by retracting the 1,603 Wikidata claims, re-deciding from cache and reprojecting:

| | before | after |
|---|---|---|
| resolved | 1,603 | **1,605** |
| changed | — | **44** |
| lost | — | **1** (`VIKARE`) |
| gained | — | **3** (`Kevin Lamb`, `Revolver`, `Westinghouse`) |

The changed set is the concept-theft class in bulk: `Dogma` was the doctrine, now the 1999 film;
`Monopoly` the market structure, now the board game; `Science` the concept, now the journal;
`Sunset Boulevard` a thoroughfare, now the 1950 film; `Stagecoach` a carriage, now the John Ford
film; `Tokyo Express` a military unit, now the Matsumoto novel; `Freedom of Speech` the right, now
the Rockwell painting; `Orthodoxy` the concept, now the Chesterton book. `Charlie Rose` was the
person, `Chinatown` an ethnic enclave, `Corsica`-like place articles were the same shape.

### The kind rule that worked, with no collateral

`product` was a mapped entity type with no class arm, so *a product could be an event*: `Boeing
B-17 Flying Fortress` claimed *2019 Boeing B-17 Flying Fortress crash* because "heavy bomber"
appears in a crash report. Mapping `Q3002150` (aviation accident) as an event and rejecting
`Event`/`Person` for `product` changes exactly one claim in the graph — the right one — with zero
lost and zero gained. The narrow arm is deliberate: a product's own classes are broad and unmapped,
and the rule only has to catch a kind of thing a product cannot be.

### The hint is a query (re-landed after a wrong revert)

1,322 of 2,211 live entities carry a `search_query`; 257 unclaimed ones did. It was a *scoring* signal
only — consulted against candidates some other question had produced — because `plan_fallback` searched
the bare label.

**Policy, set 2026-11-07: a correct fix in a component upstream of the decider ships even when it
regresses measured precision for decider reasons.** The fetch is right whether or not `decide` can yet
refuse its output; withholding retrieval to hide a decision defect inverts the layering. The decider
has one job and it is not the resolver's.

Measured, 242 requests: 9 claims, 3 wrong (`Hittite Empire` → *Hittitology*; `Extropians` → an acronym
item; `Earendil` → an AI coding agent). Those three are **decider false positives** and are tracked as
such, not as a reason to re-revert.

`wbsearchentities` matches labels and aliases, not text, so 285 of 343 hint queries return no candidate
at all (`Commission of Tears Antonio Lobo Antunes` → nothing). The endpoint is the limit; a text search
(`list=search`) is still the open item.

### Where the unresolved entities fail, by gate (331 unclaimed, replayed)

`eval inspect` over every unclaimed entity and a tally of its refusals — no judge needed, and it says
more than the presence split did:

| refusal | count |
| --- | --- |
| the answer carried no candidate | **726** |
| the cached answer is expired | **320** |
| the hint did not clear its bar | **56** |
| the title is about another subject | 34 |
| the answer carried only a disambiguation page | 23 |
| no disambiguation hint to fall back on | 20 |
| every candidate is a different kind of thing | 14 |
| two candidates fit and neither wins | 3 |

Re-judged: **54 of the 331** had their right item among the fetched candidates and were refused
(154 had only wrong candidates, 123 had no candidates at all). Replaying just those 54 gives their
per-entity gates:

| gate | entities |
| --- | --- |
| the hint did not clear its bar | **25** |
| no disambiguation hint to fall back on (no hint at all) | 20 |
| the title is about another subject | 13 |
| the answer carried only a disambiguation page | 6 |
| every candidate is a different kind of thing | 1 |

So the decider false-negative bucket is **not one knob**: half of it is `by_hint`'s bar
(`HINT_MIN_SUPPORT = 0.50` of the hint's tokens), a fifth has no hint to offer at all — which is a
retrieval question, not a decision one — and the rest are the title name-test and menus.

**Stale cache is not the constraint.** Refreshing the expired answers fetched 327 responses and
produced **0 claims** (the pass then hit the provider's rate limit). The rungs come back empty, not
stale: "no candidate" is the largest bucket by far, and that is a retrieval fact.

### Where the unresolved entities actually fail (340 unclaimed, judged)

| bucket | count | owner |
| --- | --- | --- |
| no cached candidate at all | 124 | plan / transport |
| candidates fetched, right thing absent | 159 | plan / transport |
| **candidates fetched, right thing present, refused** | **57** | **decider false negative** |

So ~17% of the unresolved population is a decision defect and ~83% is retrieval. The earlier "12.4%
resolvable but unresolved" number charged both to the resolver; it does not.

### Tried and reverted: a generic-word list

The obvious tightening — refuse the lone-label shortcut when both descriptions are present and
share no content word, and treat nationality words (`american`, `british`, …) as never-sufficient
evidence — fixes `Bethlehem Steel` and `Natural Philosophy`. It also costs **27 claims**: 29 lost
against 2 fixed, because for a person whose name is a Wikipedia disambiguation page the only
overlap there is *is* a shared nationality adjective (`American author` / `American novelist`),
and the title fallback cannot answer a menu.

The real defect is upstream: `Weights::frequency` is measured *over the answer's candidates*, so a
lone candidate makes every word in its description maximally rare and a single shared word clears
the terse-and-rare gate. A corpus-level frequency (the stored identity descriptions, fingerprinted
like any other derived data) is the fix; a word list is not.

Five went the wrong way, and they share a cause that predates the change — the label rung's fit
accepts a description that shares a stray word: `Boeing B-17` → a crash *event*, `Industrial
Revolution` → "Buildings of Scotland in the Industrial Revolution", `Lucifer` → the *Supernatural*
character, `Monster` → an audio track, `Natural Philosophy` → a Vanity Fair caricature, `Scorched
Earth` → a video game. The fit gate is the next thing to tighten; the shortcut is no longer hiding it.

The recurring offender is the **lone-exact-label shortcut**: one candidate whose surface matches
exactly is claimed at 0.75 with no corroboration. `Choo` → a river and `Orthodoxy` → a concept are
both it. It is the highest-yield policy change left.

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
