# Upload attempts

**Status:** proposed — nothing implemented. §1 (log events) and §2 (the table) are
both design only, deferred to a CLIENT_PULSE milestone.
**Last updated:** 2026-10-29.

> **See also:**
> - `src/webui/upload.rs` — the single place an upload outcome is decided.
> - `plan/archive/task-status.md` — the `task_runs` model this parallels.
> - `plan/README.md` — the index.

## Why

On 2026-10-29 three uploads failed in production because the request body
exceeded the 5 MiB cap. Nothing in the product recorded those attempts: the only
evidence was an `ApiError` line whose text had to be read to tell an oversized
image from a malformed body, and the browser told the user "Upload failed.
Please try again." — advice that could never work for the same file.

Two gaps, in order of cost:

1. **Triage.** "What are recent upload failures?" must be one query, not an
   incident-specific grep of prose.
2. **Measurement.** How often do uploads fail, for whom, at what size, and how
   far above the cap? Answering that from logs only works inside the log
   retention window.

## 1. Logs (proposed)

Each upload attempt would emit exactly one structured line, flat in `jsonPayload`:

```
jsonPayload.event="upload_attempt"
  outcome      accepted | rejected_too_large | rejected_no_image
               | client_disconnected | rejected_bad_multipart | failed_insert
               | rejected_format
  user_id      i32
  body_bytes   u64   (declared Content-Length, when the client sent one)
  image_bytes  usize (decoded image part, when the body parsed)
  max_bytes    usize (MAX_UPLOAD_BYTES in force)
  filename, capture_id, status_code  (when known)
```

`rejected_format` is the format gate (HEIC/HEIF and anything unidentifiable) —
see `plan/upload-formats.md`.

`client_failed` is the one outcome the server cannot observe by itself: a failure
that never leaves the device — an unreadable file, a dead socket, a session that
expired before the POST. That needs the client to report it, which is exactly
what CLIENT_PULSE exists for.

**Nothing here ships yet.** FIX_TONIGHT deliberately omitted it: today's
rejections are already one query away via the `ApiError` line, which logs
`status_code` and the reason (`jsonPayload.status_code=415` for a refused
format). Structured outcomes add filtering without prose, and the per-attempt
detail (`image_bytes`, `max_bytes`) — worth building as one framework, not
piecemeal.

`client_disconnected` and `rejected_bad_multipart` would be separated on purpose:
a phone losing signal mid-upload and a body the server could not parse look
identical in the HTTP log (both a 400), and only the first explains why a request
body arrived short. Classification walks the error source chain for a socket error
kind, falling back to `multer`'s two stream-level messages — `axum_extra` does not
expose the inner error, so the fallback costs at most a mislabelled line. It is
also the fiddliest part of the slice: the prototype was written and pulled back
out with the rest, so re-derive it with tests attached.

What the events would buy:

- **One query for triage** instead of grepping prose:
  `jsonPayload.event="upload_attempt" AND jsonPayload.outcome!="accepted"`.
- **Counts by outcome**, from `value(jsonPayload.outcome) | sort | uniq -c` — how
  `plan/upload-formats.md` intends to measure the HEIC refusal rate.
- **Sizes on rejection** (`image_bytes / max_bytes`), the only honest way to size
  the cap: 12 MiB is a tuned guess today.

What answers the question today: `ApiError` logs `status_code` as a structured
field, so a refusal class is countable with no new code.

```bash
# Format refusals, which became an explicit 415 on 2026-10-29.
gcloud logging read 'resource.type="cloud_run_revision"
  AND resource.labels.service_name="dreamscroll-webui-api"
  AND jsonPayload.status_code=415' --project mdrcode --freshness=7d \
  --format="value(timestamp,jsonPayload.message)"
```

The rejection's *reason* lives in the `ApiError` log line, not the response: the
error body stays opaque (`Request failed. status=413 report_id=… trace_id=…`) and
the size detail appears in the logged message, e.g.
`Upload is 6.5 MB; the limit is 12.0 MB (12582912 bytes)`. Surfacing a safe
message to the user is USER_FACING_LOGGING (see `plan/backlog.md`).

Logs answer the question today. They do not survive retention, cannot be joined
to captures, and cannot produce per-user rates. That is what the table is for.

## 2. `upload_attempts` (proposed)

Shape follows `task_runs`: identity, owner, lifecycle facts, sizes, and error
triage fields, with no dependent rows. Unlike `task_runs` there is **one row per
attempt and no state machine** — an attempt is born terminal.

```sql
CREATE TABLE upload_attempts (
    id            BIGSERIAL PRIMARY KEY,

    -- Correlates the row with the `upload_attempt` log line and with any
    -- client-side report. Stable across retries of the same file.
    attempt_id    TEXT NOT NULL UNIQUE,

    user_id       INTEGER NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,

    outcome       TEXT NOT NULL,   -- the same vocabulary as the log event
    http_status   INTEGER NOT NULL,
    source        TEXT NOT NULL,   -- 'webui' | 'rest_api' (later: 'cli')

    -- Sizes. `declared_bytes` is Content-Length as sent; `image_bytes` is the
    -- decoded part. Both are null when the body was rejected before parsing.
    declared_bytes BIGINT,
    image_bytes   BIGINT,
    max_bytes     BIGINT NOT NULL,

    mime_type     TEXT,
    filename      TEXT,
    user_agent    TEXT,            -- mobile browsers are the observed failure set

    capture_id    INTEGER,         -- set when outcome = 'accepted'
    error_message TEXT,            -- internal chain, cf. task_runs.last_error_message
    duration_ms   BIGINT
);
```

Indexes:

| Index | Serves |
| --- | --- |
| `(created_at DESC)` | "recent failures" |
| `(user_id, created_at DESC)` | per-user history, retry storms |
| `(outcome, created_at DESC)` partial `WHERE outcome <> 'accepted'` | failure-rate reporting without scanning successes |
| `(capture_id)` | attempts that produced a given capture |

Reporting queries the shape is designed for:

- failure rate per day, split by `outcome`
- size distribution of `rejected_too_large` versus `accepted` — the input for
  choosing the cap and for justifying client-side downscaling
- attempts per accepted capture (retry cost)
- per-user failure counts, to separate "one phone on a bad network" from a
  systematic regression

### Write path

Insert in the upload handler once the outcome is known, before responding.
Rejections (the interesting rows) never read the body, so the insert must be
awaited; successes may be written in the same call. Cost is one INSERT per
attempt on a table this size — irrelevant.

If the insert ever fails, log it and **do not fail the upload**: the capture is
the user's work, the attempt row is telemetry. That asymmetry is the reason this
is not folded into `captures`.

### Deliberately not modelled

- **Per-file identity / dedupe.** The same file retried is a new attempt. That is
  the point.
- **Client-side numbers.** Bytes the browser *meant* to send, if it aborts
  before the request lands, are invisible server-side. Needs a client beacon;
  out of scope.
- **Pre-handler rejections.** Requests Cloud Run answers without invoking the
  container (over 32 MiB) never reach this code.
- **Auth failures.** `/upload` requires login; an unauthenticated POST has no
  `user_id` and is not an upload attempt in the product sense.

### Rollout

No migrations, per `plan/README.md`: add the `sea_orm` model under
`src/model/`, bump `REQUIRED_SCHEMA_VERSION` in `src/database/sync_schema.rs`,
and ship it with the schema-sync job. Local dev writes are per-project policy:
ask before running `sync_schema` locally.

## Open questions

1. Does the table replace the log event once it exists? Keep both: logs are the
   real-time triage path (no query, no auth) and the table is the analysis path.
2. Retention. Rows are tiny; the pragmatic default is keep-forever until query
   latency or privacy argues otherwise.
3. `filename` and `user_agent` are user data. The existing logs already carry
   filenames; revisit if the table is ever exported.
4. Should `MAX_UPLOAD_BYTES` be per-user (a power-user allowance) or per source
   (`rest_api` import can legitimately send larger files than a phone)? The
   config is a single scalar today; the column `max_bytes` is per-row precisely
   so that decision can be made later without a schema change.
