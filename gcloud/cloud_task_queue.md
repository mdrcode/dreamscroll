# Creating Cloud Tasks queues

The production application creates tasks with the target URL and OIDC token on
each task. The queue commands below are for creating the queues and their retry
policies; they do not need HTTP target or OIDC overrides.

The queue names intentionally match the webhook route suffixes configured by the
application:

| Queue | Webhook route |
| --- | --- |
| `illuminate` | `/_wh/cloudtask/illuminate` |
| `search-index` | `/_wh/cloudtask/search-index` |
| `spark` | `/_wh/cloudtask/spark` |
| `knowledge-link` | `/_wh/cloudtask/knowledge-link` |
| `identity-resolve` | `/_wh/cloudtask/identity-resolve` |

All five are required by the service's config (`TASK_QUEUE_NAME_*`); the app
refuses to boot if a configured name has no route.

Use the same dedicated service account configured as
`TASK_OIDC_SERVICE_ACCOUNT_EMAIL` when granting Cloud Tasks permission to mint
tokens for task delivery.

```bash
gcloud tasks queues create illuminate \
  --location=us-central1 \
  --max-dispatches-per-second=2 \
  --max-concurrent-dispatches=5 \
  --max-attempts=3 \
  --min-backoff=10s \
  --max-backoff=600s \
  --max-doublings=16
```

Concurrency differs by queue (2026-10-27): `illuminate` runs **one** dispatch at
a time so Gemini inference is serialized (bounded spend, no quota spikes);
`spark` and `search-index` remain at 5. Set it with:

```bash
gcloud tasks queues update illuminate \
  --location=us-central1 \
  --max-concurrent-dispatches=1
```

`max-dispatches-per-second` (2) governs how fast attempts start and is unchanged;
with concurrency 1 the throughput ceiling becomes one illumination per task
duration, so the all-captures backfill takes roughly that long per capture.

`identity-resolve` runs at **1 concurrent dispatch** (reduced from 2 on
2026-11-18). The ceiling is 2, derived from the Wikimedia per-minute limit rather
than chosen: each run paces at 1 s ⇒ 60 requests/min, so two runs = 120/min
against the 200/min bucket a compliant User-Agent gets. The queue is kept at 1
while an operator drain runs slices at ~40 calls/min: 60 + 40 = 100/min, half the
bucket spare. Raise it back to 2 when no drain is running. Never exceed 2 — three
runs (180/min) plus a CLI run is 240/min and earns 429s, which latch the provider
for the remainder of the run. Same retry policy:

```bash
gcloud tasks queues create knowledge-link \
  --location=us-central1 \
  --max-dispatches-per-second=2 \
  --max-concurrent-dispatches=1 \
  --max-attempts=3 \
  --min-backoff=10s \
  --max-backoff=600s \
  --max-doublings=16

gcloud tasks queues create identity-resolve \
  --location=us-central1 \
  --max-dispatches-per-second=2 \
  --max-concurrent-dispatches=1 \
  --max-attempts=3 \
  --min-backoff=10s \
  --max-backoff=600s \
  --max-doublings=16
```

Verify:

```bash
gcloud tasks queues describe illuminate \
  --location=us-central1 \
  --project=mdrcode
```
