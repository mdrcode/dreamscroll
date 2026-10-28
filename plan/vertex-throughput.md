# Vertex Gemini throughput and tail latency

**Date:** 2026-10-27
**Status:** Research — active
**Purpose:** Explain and mitigate pathological inference latency observed in production
(3.6 s–266 s for the same model, 45–170 KB inputs), and decide whether it is transient,
quota-related, or structural.

Related: `plan/topology_and_throughput.md` (Cloud Run/Cloud SQL budgets),
`plan/archive/google-ai-interactions-api.md` (the client that issues these calls),
`plan/archive/inference-metadata.md` (where duration/tokens are recorded).

---

## 1. Observed evidence

From production logs, 2026-10-27 17:45–18:46 UTC (29 completed inferences):

| Metric | Value |
| --- | --- |
| `inference_ms` range | 3,631 – 266,035 |
| Median | ≈ 74,000 |
| p90 | ≈ 226,000 |
| Slow calls | 266 s, 258 s, 226 s, 222 s, 192 s, 190 s, 178 s |
| Fast calls interleaved | 3.6 s @ 18:28:56, 9.5 s @ 18:01:46, 13.8 s @ 18:15:04 |
| Input size vs latency | uncorrelated (45 KB → 226 s; 169 KB → 48 s) |
| Provider 429 / `RESOURCE_EXHAUSTED` | none |
| Cloud Run 504s | at the 300 s request deadline; each retry re-runs inference |

Client path (`src/llms/gemini/client.rs`): one `POST` to the Vertex Interactions endpoint,
`Instant::now()` around the call, no client timeout, no retries. `inference_ms` is therefore
pure provider round-trip time. `duration_ms` and token counts are persisted in
`inference_metas` (`src/model/inference_meta.rs`).

The bimodal distribution — seconds and minutes in the same minutes — looks like
**queueing behind shared capacity**, not compute proportional to input, and not a
code-level regression: the Interactions client has been live since 2026-10-07.

Caveat for token accounting: `inference_metas` is written only after the provider
call returns, so failed/timeout calls contribute no duration or token cost to the
inference stats. Their rate and wall-clock cost appear in the queue stats instead
(see `plan/pragmatism.md`, "Failed inference calls record no usage row").

## 2. Answering the standing questions

**Is it transient?** Not yet known. The pattern (fast calls interleaved with multi-minute
calls) is more consistent with intermittent shared-capacity contention than a single
incident, but one 1-hour sample cannot separate the two. A 24–72 h time series of
`inference_metas.duration_ms` per hour settles it.

**Are we slow because we don't buy capacity?** Vertex Gemini pay-as-you-go runs on
Dynamic Shared Quota: per-project RPM/TPM ceilings, capacity shared with other tenants,
no latency guarantee. Provisioned Throughput (PT) is a paid reservation that converts
that into guaranteed throughput (tokens/min) for PT-eligible models; 3.8 Flash is
eligible, and some preview models are too (verified in §8), while quota *increases*
only raise ceilings, they do not reserve capacity. So PT is a possible mitigation, not
the explanation for why one request waits 4 minutes while the next takes 4 seconds. Docs:
[Throughput quota](https://cloud.google.com/vertex-ai/generative-ai/docs/resources/throughput-quota),
[Provisioned Throughput](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/provisioned-throughput/use-provisioned-throughput).

**What can we do?** Section 5, ordered by cost of adoption.

### Inspecting it

```bash
# Queue-side: queued wait and in-process duration, success vs failure, backlog,
# failure codes. One section per window.
dreamscroll_api queue throughput --hours 1,24,168,720

# Provider-side: duration and token percentiles per task type/model/backend.
dreamscroll_api inference throughput --hours 1,24,168,720
```

Both read `task_runs`/`inference_metas` through admin-only REST endpoints
(`/api/admin/queue/stats`, `/api/admin/inference/stats`).

## 3. Hypotheses, ranked

| # | Hypothesis | Discriminating evidence |
| --- | --- | --- |
| H1 | Dynamic Shared Quota queueing / noisy-neighbour capacity | Fast and slow interleaved; no 429s; latency uncorrelated with input |
| H2 | Unset `thinking_level` → provider default reasoning level inflates tokens and latency | Pinning `minimal` collapses duration and `tokens_thought` |
| H3 | Model version/capacity (`gemini-3.8-flash` likely preview; not PT-eligible) | Same prompt on a GA flash model is consistently fast |
| H4 | Region/endpoint capacity (`us-central1`, Interactions endpoint) | Another region or backend (Developer API) behaves differently |
| H5 | Output-length variance from the v2 entity schema | `tokens_out`/`tokens_thought` correlate with slow calls |
| H6 | Our runtime (cold start, CPU throttling, token refresh) | `inference_ms` is measured around the HTTP call only; cold starts show as ~seconds, not minutes |

## 4. Experiments (cheap → expensive)

1. **Thinking-level sweep.** Same capture, `illuminate --thinking-level minimal|low|medium|high`,
   3 samples each; record `inference_ms`, `tokens_out`, `tokens_thought`.
2. **Model comparison.** Same capture against a stable GA flash model, 3 samples each.
3. **Time series.** Per-hour p50/p90 from `inference_metas` over 24–72 h (this is what
   `dreamscroll_api inference throughput --hours …` answers).
4. **Concurrency sweep.** Backfill pump with queue concurrency 1 → 3 → 5; watch p50/p90 and
   timeouts. Tests whether our own in-flight count is part of the queue.
5. **Vertex observability.** Cloud Monitoring model invocation latency histograms and quota
   usage in `us-central1` for the model; confirm absence of 429s at the API, not just in our logs.
6. **Backend comparison.** Vertex vs Developer API on identical input (Developer API gives a
   vendor-independent control).

## 5. Mitigations under consideration

| Mitigation | Cost | Tradeoff |
| --- | --- | --- |
| Raise Cloud Run request timeout 300 → 600 s | trivial (manifest) | 504 retries stop; slow calls still slow |
| Pin `thinking_level = minimal` for backfill tasks | small code change | possible quality loss on entity extraction; A/B one capture first |
| Cap queue concurrency (e.g. 3) until variance is understood | queue config | backlog drains slower, but bounded in-flight retries |
| Batch API for the backfill | medium (new submission path) | ~50% cheaper, async, no latency SLA; good fit for 400 historical captures |
| Provisioned Throughput | money/month commitment | only if volume and PT eligibility justify it |
| Model/region failover | config + code | splits traffic; needs a second measurement baseline |

## 6. Decision log

| Date | Decision | Rationale |
| --- | --- | --- |
| 2026-10-27 | Diagnostics before configuration: build windowed queue/inference stats first | Tuning without p90/p99 over windows is guesswork |

## 7. Open items

- [ ] Populate per-hour latency series; decide transient vs structural.
- [ ] Run thinking-level and model sweeps.
- [ ] Decide on timeout bump and concurrency cap.
- [ ] Decide whether the backfill moves to Batch API.

## 8. External research — 2026-10-27

Independent web review, with claims tagged `[DOC]` (vendor documentation) or `[ANEC]` (community).

- **PayGo queueing is documented, not an incident.** Standard PayGo "runs on shared resources and doesn't reserve dedicated capacity or provide assured throughput during periods of high contention"; shared-pool traffic can see "higher latency or 429 errors". The consumption-options doc says PT "eliminates the variability caused by 'cold starts' or queuing that can occasionally occur in PayGo models during high traffic". `[DOC]`
  - https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/standard-paygo
  - https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/deploy/consumption-options
- **No latency guarantee without PT.** The SLA covers uptime via 5XX only; a latency SLO exists only under Provisioned Throughput, only for `streamGenerateContent` with >75 output tokens, and only for the covered 2.5 models. A 266 s response on 3.8 Flash PayGo is within contract. `[DOC]` https://cloud.google.com/vertex-ai/generative-ai/sla
- **Model/region correction.** `gemini-3.8-flash` is GA (2026-09-02), supported in `global`, `us`, `eu` only — there is no `us-central1` deployment. Our client already targets `locations/global` (`src/llms/gemini/client.rs`), so single-region routing is not a lever. The Interactions API we call is `v1beta1` (preview). Thinking defaults to **MEDIUM** for this model. `[DOC]` https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/gemini/3-8-flash
- **PT is available for this model**, 675 tok/s per GSU, minimum 1 GSU. List ≈ $1,200/GSU-week, ≈ $2,700/GSU-month; a 50% promotional credit runs through 2026-12-31 (≈ $1,350/mo). PT excludes batch. `[DOC]` https://cloud.google.com/gemini-enterprise-agent-platform/generative-ai/pricing
- **No matching status incidents.** No 2026 latency incidents for Gemini; the only 2026 entry was an error-rate incident (429/503), not slow success. `[DOC]` https://status.cloud.google.com/products/Z0FZJAMvEB4j3NbCJs6B/history
- **Community reports match our curve**: ~200 s spikes on the global endpoint (2026-04); a 3.8 Flash 503 thread where Google staff describe services as "temporarily overloaded" (2026-10-23). `[ANEC]`
- **Untested lever: Priority PayGo.** Header `X-Vertex-AI-LLM-Shared-Request-Type: priority`, supported for 3.8 Flash, no commitment (per-token premium), documented for latency-sensitive traffic without PT. `[DOC]`

### Revised experiments (delta)

7. **Priority PayGo A/B**: identical capture with and without the priority header, 10 calls each; compare p50/p90 and the per-call cost delta.
8. **Stream and record TTFT vs TTLT** to separate admission wait from generation time.

### Open items (delta)

- [ ] Test the Priority PayGo header.
- [ ] Decide whether the backfill moves to Batch (50% cheaper, 24 h turnaround, no PT).
- [ ] Revisit PT only if priority traffic still degrades and volume justifies ~$1,350–2,700/mo.

Interpretation: the fast/slow split with no 429s is consistent with shared-pool admission, not token generation; that remains `[SPEC]` but is now supported by vendor language about queuing in PayGo.

### Failure-code semantics (2026-10-27)

`task_runs.last_error_code` now records the **upstream provider status** when the
failure came from a provider response: `InferenceOutcome` carries the provider's
HTTP status, callers attach it via `ApiError::with_inner_status_code`, and
`TaskRunTracker` prefers it over the route's own status. Webhooks still return 500, so Cloud Tasks retries
instead of tripping queue-wide congestion. Rows written before this change
recorded only 500; the provider detail was always in `last_error_message`.
