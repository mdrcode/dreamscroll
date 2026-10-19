# REST Task Submission and CLI Evaluation

**Status:** Implemented.
**Scope:** Submit supported background tasks through the authenticated REST API,
then optionally wait for a specific run from `dreamscroll_api`.

> **See also:**
> - `task-status.md` — task identity, queues, run lifecycle, and persisted status.
> - `testing.md` — validation strategy.

## 1. Goals

- Make it easy to submit concrete task implementations to a running Dreamscroll
  service for development, testing, and evaluation.
- Keep execution on the service instance(s), using the existing queue and
  `TaskMaster` lifecycle rather than running handlers inside the CLI process.
- Support multiple task implementations without making a queue name part of a
  task-run identity.
- Let callers retrieve and optionally wait on the exact run they submitted.

Initial task support:

| Queue          | Task               | Request body          | Ownership validation                      |
| -------------- | ------------------ | --------------------- | ----------------------------------------- |
| `illuminate`   | `IlluminationTask` | `{ "capture_id": N }` | Capture must belong to authenticated user |
| `search_index` | `SearchIndexTask`  | `{ "capture_id": N }` | Capture must belong to authenticated user |

These are independent task types. `IlluminationTask` performs illumination;
`SearchIndexTask` performs search indexing. New captures are submitted through
Beacon, which coordinates the follow-up behavior after illumination succeeds.

This replaces the older `POST /api/captures/{capture_id}/illuminate` route,
which returned only `204` and could not provide a run identity for polling.

## 2. REST API

All routes require the existing JWT bearer authentication and use a normal user
context. The API derives ownership from that context; clients do not send a
`user_id`.

### 2.1 Submit

```http
POST /api/queues/illuminate
Authorization: Bearer <token>
Content-Type: application/json

{ "capture_id": 123, "prompt_version": "v2", "thinking_level": "low" }
```

Illumination accepts optional `prompt_version` (default `v2`) and `thinking_level`
(`minimal`, `low`, `medium`, or `high`). Omit the level to keep the provider's
default. The task stores the selected override alongside its model and prompt
version. Model-specific supported levels are validated by Gemini.

Search-index submissions use `POST /api/queues/search_index` with only
`{ "capture_id": N }`. The queue segment identifies the submission destination
and selects the concrete task implementation. It is not the task identity.

On success, the server verifies that the capture is accessible to the caller,
submits through `TaskMaster`, and returns HTTP `202 Accepted`:

```json
"b91a7c4f-7e8a-4bf8-9a76-c81e258ec113"
```

The response body is the `run_id` string itself. Use it to query this exact run's
status. A submission refused because the same logical task is already in flight
returns `409 Conflict`. A missing or inaccessible capture returns `404` without
revealing cross-user existence.

### 2.2 Query exact-run status

```http
GET /api/tasks/{run_id}
Authorization: Bearer <token>
```

The lookup is scoped to the authenticated user and exact run. A missing run or
a run owned by another user returns `404`. The successful response contains:

```json
{
  "logical_id": "u1-illuminate-capture123",
  "run_id": "b91a7c4f-7e8a-4bf8-9a76-c81e258ec113",
  "run_number": 2,
  "task_type": "illuminate",
  "entity_type": "capture",
  "entity_id": 123,
  "status": { "name": "in_progress", "discriminant": 2 },
  "attempts": 1,
  "created_at": "2026-10-01T12:00:00Z",
  "updated_at": "2026-10-01T12:00:03Z"
}
```
After the run reaches `CompleteSuccess`, fetch its raw illumination result with
`GET /api/illuminations/raw/{task_run_id}`, passing the `run_id` from the task
status response. `illumination_raws` stores one raw output per TaskRun; retries
upsert that row, replacing the previous attempt's result. TaskRun status responses
do not include a separate result type or result ID.

`SubmissionFailed`, `CompleteSuccess`, and `CompleteFailure` are settled states.
`Queued`, `InProgress`, and `ErrorWillRetry` may still progress.

## 3. CLI

`dreamscroll_api` already accepts `--host` and optional `--user`, authenticates
with a JWT, and caches credentials in the macOS keychain. The new commands reuse
that URL/authentication flow and the shared REST client:

- `dreamscroll_api task illuminate <capture_id>`
- `dreamscroll_api task search_index <capture_id>`
- Either command may add `--wait`.
- `--wait-seconds N` sets the maximum wait; default is 60 seconds.

Without `--wait`, the CLI submits and immediately queries the exact run status.
With `--wait`, it queries every two seconds until the run settles or the timeout
expires. It prints the `run_id`, logical ID, run number, and current status.
On timeout, it reports the latest observed status and exits successfully: task
submission succeeded even though completion was not observed within the wait
window. The `run_id` can be used for a later status lookup.

### 3.1 Compare illumination thinking levels

`inference illuminate` submits a run, waits for success, and prints that exact
run's result as Markdown. Run the same capture sequentially with different
levels; save stdout separately to compare outputs:

```sh
dreamscroll_api --host localhost:8080 --user alice inference illuminate 123 --thinking-level low > /tmp/illumination-low.md
dreamscroll_api --host localhost:8080 --user alice inference illuminate 123 --thinking-level high > /tmp/illumination-high.md
```

V2 is the default; `--prompt-version v2` makes it explicit. `--wait-seconds N`
changes the 60-second wait limit. The service's `GEMINI_MODEL_ID` selects the
model for both runs. Omit `--thinking-level` for the provider-default baseline.
The shared inference adapter also supports the override for V1.

As of Oct 2026, Gemini's default effort is model-specific: for example, `high`
for `gemini-3-flash-preview` and `gemini-3.1-pro-preview`, `medium` for
`gemini-3.8-flash`, and `minimal` for `gemini-3.5-flash-lite`. Omission delegates
to Gemini; Dreamscroll does not pin these defaults. See the
[provider's current table](https://ai.google.dev/gemini-api/docs/thinking#controlling-thinking).

This is normal persisted inference, not a dry run: each submission creates a
TaskRun and writes its raw result and, for V2, relational projection. A second
run is refused while the same capture's illumination is still in flight. These
commands do not freeze captures or model aliases; keep those unchanged when
comparing results. Output can vary between runs even at the same thinking level.

Gemini Interactions calls the setting
[`generation_config.thinking_level`](https://ai.google.dev/api/interactions-api).
The selected override is also retained in
`InferenceMeta.params.generation_config.thinking_level`, so results and usage
can be associated with the chosen level. Supported levels vary by model; see
the [thinking guide](https://ai.google.dev/gemini-api/docs/thinking#controlling-thinking).
No local model-support matrix or automatic downgrade is maintained.
The Rust enum lives at `llms::gemini::ThinkingLevel`: these named levels are a
Gemini-specific contract, not a provider-neutral effort scale.

## 4. Design boundaries

- Queue names (`illuminate`, `search_index`) are REST submission destinations;
  they are not interchangeable with task identities.
- `logical_id` identifies the logical task, while `run_id` identifies one globally
  unique invocation. `run_number` is the per-logical-task ordinal used for ordering.
  Use `run_id` to fetch the exact TaskRun status.
- The REST routes call user-facing API client methods, which validate capture
  ownership before enqueue. Task execution continues to use the existing
  webhook/worker flow.
- Status queries return one exact run, not the latest run implicitly. This
  avoids a later rerun changing what the CLI is observing.
- This API is intentionally small and task-specific at its edge. A tagged
  generic task payload is not introduced until more task types or payload
  variation justify it.

## 5. Implementation map

| Area                                  | Files                                                     |
| ------------------------------------- | --------------------------------------------------------- |
| Authenticated REST routes             | `src/rest/r_task.rs`, `src/rest/maker.rs`                 |
| Ownership, enqueue, status conversion | `src/api/user/client.rs`, `src/api/schema/taskruninfo.rs` |
| User-scoped exact-run lookup          | `src/task/taskmaster.rs`, `src/task/taskruntracker.rs`    |
| REST HTTP client                      | `src/rest/client/mod.rs`                                  |
| CLI command                           | `src/bin/dreamscroll_api.rs`, `src/util_cmds/api/task.rs` |