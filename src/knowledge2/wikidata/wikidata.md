# Wikidata: what we ask, what they allow, and what we chose

**Status:** Reference. Read before changing pacing, budgets or endpoints. Facts about Wikidata's terms
verified 2026-11-14; the numbers on our side are traceable to a commit, not to a doc.

**Scope:** the Wikidata identity resolver only. The cache, the gateway and the budget mechanics are
`src/fetch/fetch.md`; this file is about the *authority's* rules and our compliance with them.

## What we ask, and how

| endpoint | used for | batchable |
| --- | --- | --- |
| `https://query.wikidata.org/sparql` | `label` (exact `rdfs:label` via `VALUES`) and `handle:P…` (external-id properties) | yes, `VALUES` blocks |
| `https://www.wikidata.org/w/api.php` | `search` — CirrusSearch (`action=query&list=search&srsearch=`), deliberately not `wbsearchentities` | **no**, one call per query |
| `https://en.wikipedia.org/w/api.php` | `title` — `pageprops` → `wikibase_item` | yes |

- **User-Agent:** `dreamscroll-knowledge/0.1 (https://dreamscroll.ai)` (`transport.rs:24`).
- **HTTP timeout:** 30 s (`transport.rs:27`).
- **Batch size:** 50 ids per request (`fetch::transport::BATCH`).
- **Pace:** one transport call per `min_interval`, default **1 s** ⇒ ≤60 requests/minute.
- **Spend:** `max_requests` calls per **attempt**, default **80** ⇒ a maxed attempt lasts ~80 s at
  1 s pacing. Sized from the corpus: one illumination's worst cold cost is 58 calls (17 distinct
  entities × up to 5 questions), p95 24, mean 14.6 — so 80 covers the maximum with headroom. The task framework may retry a failed attempt (`TASK_MAX_ATTEMPTS`, 3 in production),
  and each retry builds a fresh allowance, so a task's worst-case ceiling is **attempts × 80** (240 in production).
  In practice the product stays small: a spent budget completes the run rather than failing it, and a
  retry is served mostly from the cache the previous attempt warmed.
- **Concurrency:** at most **2** runs at a time — the `identity-resolve` queue is capped at 2; the
  batch job (tabled) will need an advisory lock for the same reason, since it would be a third.
- **Cache first:** every question is answered from `fetch_cache` when warm; a rebuild re-asks only
  what no run has ever fetched. TTLs: found 180 d, empty 30 d, degraded 23 h, failed 10 min.

## Where our numbers came from

Neither default was derived from Wikidata's documentation. Be precise about that before "fixing" one:

| value | provenance |
| --- | --- |
| `DEFAULT_MAX_REQUESTS = 200` — the **CLI** fallback; the task config defaults to 80, sized to the corpus | `src/knowledge/identity/mod.rs:25`, introduced in `d00a97d` (2026-10-31, "Resolve cores against Wikidata, behind a database cache"). Its doc: *"so that omitting the budget does real work instead of quietly resolving nothing"* — a fallback so a caller that forgets the flag still resolves |
| `min_interval_ms = 1000` | the CLI's `ResolveArgs` default (`src/bin/dreamscroll_knowledge.rs:333`), same original work: a serial pace chosen for politeness |
| `TASK_WIKIDATA_MAX_REQUESTS` / `TASK_WIKIDATA_MIN_INTERVAL` | added with the `identity_resolve` task; they copy the two CLI defaults so a delivery behaves exactly like a plain `identity resolve` |

The result is conservative, but it is conservative by inheritance rather than by measurement.

## Their terms

### API:Etiquette — reads

<https://www.mediawiki.org/wiki/API:Etiquette>

- **No hard speed limit on read requests**, but the server operators may block a client that
  endangers stability.
- Make requests **in series** rather than in parallel (our 1 s pace and one-run-at-a-time satisfy this).
- **Batch**: pipe-separated titles (`titles=A|B|C`) or a generator, instead of one request per item.
- Use **gzip** (`Accept-Encoding: gzip`) where the client supports it.
- **`maxlag` should be sent by non-interactive tasks** so work pauses when the servers are busy.
- Editing is rate-limited; reads are not — but don't take the site down.
- Cache responses that can be cached.

### Wikimedia API rate limits — new in 2026, per minute

<https://www.mediawiki.org/wiki/Wikimedia_APIs/Rate_limits>

| client group | limit (req/min) |
| --- | ---: |
| unidentified — no identifying characteristic other than the IP | **10** |
| browser, unauthenticated | 200 |
| **User-Agent only**, compliant UA, unauthenticated bot | **200** |
| authenticated, new or low-edit | 200 |
| authenticated, established editor | 2,000 |
| bot flag / Wikimedia Cloud Services / granted exemption | exempt |

- Best practice: **≤3 concurrent requests**; honour `Retry-After` on 429; if absent, wait **≥5 s** or
  back off exponentially.
- 503 is also possible when a backend is overloaded, likewise with `Retry-After`.
- The page states the 2026 scheme is **experimental and may change**.

### Wikidata Query Service (SPARQL) limits

<https://www.mediawiki.org/wiki/Wikidata_Query_Service/User_Manual>

- Hard query deadline: **60 s**.
- One client (UA + IP): **60 s of processing time per 60 s**.
- **30 error queries per minute** per client.
- **5 parallel queries per IP**.
- Exceeding any of these ⇒ **429 + `Retry-After`**; ignoring them can mean a temporary ban, and a
  non-compliant User-Agent can mean a block.
- **2026–2027: WDQS is migrating backends** and introduces **v2 endpoint URLs**. Our target
  (`https://query.wikidata.org/sparql`) may change; watch
  <https://www.wikidata.org/wiki/Wikidata:SPARQL_query_service/WDQS_backend_update>.

### User-Agent policy

<https://foundation.wikimedia.org/wiki/Policy:User-Agent_policy>

A meaningful UA with contact information (email or URL) is required. Ours carries name, version and
site URL; no email.

### SLA / SLO

There is **no availability SLA on the free endpoints** — WDQS documents deadlines and throttling, not
uptime guarantees. Commercial SLAs exist only through **Wikimedia Enterprise** (free tier with speed
and volume limits; paid for higher). Treat both endpoints as best-effort and keep the cache warm
enough that an outage delays enrichment instead of stopping it.

## Our settings against their limits

| our mechanism | value | status |
| --- | --- | --- |
| pacer (`min_interval`) | 1 s ⇒ 60 req/min per run | 3.3× under the 200/min UA-only bucket, per run |
| run concurrency | **2** — the prod queue cap and the local dispatcher | 120 req/min total, leaving one operator run's 60/min of headroom; under the ≤3 recommendation and WDQS's 5 parallel |
| `max_requests` | 80 calls per attempt | spend cap, not a rate; a maxed attempt stays at 60/min |
| 429 | `Retry-After` parsed, provider latched for the run (`Allowance::rate_limited`) | matches the documented behaviour |
| SPARQL queries | small `VALUES` queries, serial | nowhere near 60 s/60 s or 30 errors/min |
| UA | `dreamscroll-knowledge/0.1 (https://dreamscroll.ai)` | compliant bucket (200/min); *if stripped we fall to 10/min* |

## Gaps and watch list

1. **No `maxlag`** on the MediaWiki API reads. Etiquette says non-interactive tasks should send it;
   one query parameter, currently absent.
2. **No gzip request.** `reqwest` is built with `features = ["json"]` only, so we never send
   `Accept-Encoding`. Recommended, not required; our bodies are small (≤50 ids).
3. **UA has no contact email** — acceptable per policy, better with one.
4. **The 10/min unidentified bucket is the risk to know about.** A proxy that rewrites or drops the UA
   turns our 200/min headroom into 6× *over* the limit at 1 s pacing.
5. **WDQS v2 migration (2026–2027).** Endpoint URLs and possibly query behaviour change. Not a
   backlog item yet; make it one when the new endpoints are announced as default.

## Changing these settings

- Runtime knobs: `TASK_WIKIDATA_MAX_REQUESTS` (80 per attempt), `TASK_WIKIDATA_MIN_INTERVAL`
  (1000 ms between calls).
- CLI: `identity resolve --max-requests`, `--min-interval-ms`, `--offline`.
- **`min_interval` is the rate control; `max_requests` is the spend control.** Lowering the budget
  does not slow a run; raising the pace does not bound its cost.
- The cache is the first line of politeness: a warm rebuild costs zero calls
  (`src/fetch/fetch.md`, §TTLs).
