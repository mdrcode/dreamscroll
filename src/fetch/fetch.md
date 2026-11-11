# The fetch layer

One door to every third party, and one rule: **a provider cannot fetch, only ask for answers.**

```
caller ──resolve_requests(requests)──▶ Gateway ──▶ cache ──hit──▶ answers
                                            │
                                            └──miss──▶ Transport::fetch ──▶ cache ──▶ answers
```

A provider owns its strategy — one request or a ladder, one pass or several — and knows nothing about
the cache, the pace or the budget, because none of them are reachable any other way. That is the
whole design: the rule is not a convention someone must remember, it is the absence of a second door.

## Nothing here knows what the answer means

```rust
trait Transport: Send + Sync {
    type Kind: Clone + Eq + Ord + Hash + Display + Send + Sync;   // the provider's request kinds
    type Payload: Serialize + DeserializeOwned + Send + Sync;     // the provider's answer body

    async fn fetch(&self, requests: &[Request<Self::Kind>])
        -> Result<Vec<(String, Answer<Self::Payload>)>, FetchError>;

    fn batch_size(&self, kind: &Self::Kind) -> usize;
}
```

Both are associated types on purpose. This module stores a kind as a string in the cache, groups a
batch by it, and hands the payload back untouched — so it can never grow a vocabulary that a second
provider would have to satisfy. `Request` is the cache key (`provider`, `kind`, `shape`, `query`), and
`Answers` is keyed by the whole `Request`, so two providers asking about the same string cannot
overwrite each other's answer before it is read.

## The cache

```sql
fetch_cache
  provider, kind, query, shape      -- unique together: one row per logical request
  outcome                            -- found | empty | failed | degraded
  payload      jsonb                 -- the provider's payload, serialised; the cache never reads it
  fetched_at, expires_at
```

Four decisions worth naming:

- **`empty` is a cached answer.** A third of any corpus will never resolve; re-asking a question we
  have already answered is exactly the spam this table prevents.
- **`shape` is in the key.** It fingerprints *how* the request is built (endpoint, language, limit,
  properties). Change the construction and every old answer — including negatives — is bypassed
  rather than silently wrong.
- **`provider` is `text`, not an enum.** Adding an authority is not a migration.
- **A payload that no longer decodes is a miss.** Not an empty answer. A changed payload shape has to
  be re-asked, or a caller would be handed a body nothing can use for the rest of the TTL.

| outcome | TTL | why |
| --- | --- | --- |
| `found` | 7 d | short enough to notice a new item, long enough that a rerun is free |
| `empty` | 7 d | a niche name may gain an item next week |
| `degraded` | 1 h | usually still true, just not what a healthy backend would have said |
| `failed` | 10 min | a provider that is down is not a fact about the world |

An answer's TTL is fixed by its outcome when it is written, and nothing revises it afterwards.
Demoting an "unproductive" answer to 6 h once the caller's stages found nothing usable was tried and
removed: it could only ever fire on `empty` (a healthy negative, which is a fact), it was a no-op on
`degraded` and `failed` (already shorter than the demotion), it contradicted the `empty | 7 d` row
above, and it emptied the cache overnight so the next morning's replay had nothing to read.

## Batching, then what a run may spend

The gateway groups the planned requests by `(provider, kind, shape)`, dedupes the queries, reads the
group from the cache, and fetches only the misses in chunks of `Transport::batch_size`.

**Batch membership is not in the key.** A batch is a chunk of cache misses, not a cache entity: the
gateway reads one row per query before the call and writes one row per query after it. A batch of 50,
a batch of 37, and a single-entity wave hit the same rows; `shape` covers how a query is built, not
which other queries travelled with it. The contract assumes each query's answer is independent of the
batch — a provider that ranked or limited *across* the batch would need that fact in `kind`/`shape`,
or a different transport contract.

Everything the door may *do* is three independent parameters:

| run | `cache` | `allowance` | `fail_fast` |
| --- | --- | --- | --- |
| production | some | some | `true` |
| local, warm cache — `resolve --offline` | some | `None` | — |
| probe: ignore what we know | `None` | some | `true` |

```rust
let allowance = Allowance::new(200, secs(1));
let gateway = Gateway::new(Some(&cache), Some(&allowance), true);

// cache only: nothing can be spent, so every group is read and nothing is fetched
let gateway = Gateway::new(Some(&cache), None, false);
```

- **`allowance: None`** never fetches, so nothing can be refused — the cache-only run.
- **`cache: None`** asks the provider for every query and stores nothing. Safe only because the
  allowance bounds the calls.
- **`fail_fast`** decides what a spent budget does: `true` ends the walk, `false` keeps reading the
  remaining groups' caches and fetches nothing more. The cache is durable, so the groups a stopped run
  skips are read by the next run at no network cost — which is why ending is the default. Harvesting
  costs one wide `IN (…)` per remaining group. A provider's `429` stops that provider regardless.

A `429` latches against that provider, so it stops every later batch and stage for that provider only;
independent providers continue. A `429` is never cached as a fact; a transport failure is cached
briefly and counted.

## Where the numbers are

Two kinds, both in `stats.rs`, because they answer different questions. `Stats` is one run, written
as it goes and read once at the end: `requested`, `cache_hits`, `fetched` (HTTP requests, not batched
calls — a title lookup reads the page and then the items it names), `failed`. A caller turns that into
its own report; this module has no opinion about what a run should print.

`Tally` is the table rather than the run: one grouped query over `fetch_cache`, counting each live
outcome and the rows past their expiry separately.
