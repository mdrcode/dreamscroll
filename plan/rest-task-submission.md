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

{ "capture_id": 123 }
```

Search-index submissions use `POST /api/queues/search_index` with the same
request schema. The queue segment identifies the submission destination and
selects the concrete task implementation. It is not the task identity.

On success, the server verifies that the capture is accessible to the caller,
submits through `TaskMaster`, and returns HTTP `202 Accepted`:

```json
{ "envelope_id": "u1-illuminate-capture123", "run": 2 }
```

The returned `(envelope_id, run)` identifies one run and is used to query its
status. A submission refused because the same logical task is already in flight
returns `409 Conflict`. A missing or inaccessible capture returns `404` without
revealing cross-user existence.

### 2.2 Query exact-run status

```http
GET /api/tasks/{envelope_id}/{run}
Authorization: Bearer <token>
```

The lookup is scoped to the authenticated user and exact run. A missing run or
a run owned by another user returns `404`. The successful response contains:

```json
{
  "envelope_id": "u1-illuminate-capture123",
  "run": 2,
  "task_type": "illuminate",
  "entity_type": "capture",
  "entity_id": 123,
  "status": { "name": "in_progress", "discriminant": 2 },
  "attempts": 1,
  "created_at": "2026-10-01T12:00:00Z",
  "updated_at": "2026-10-01T12:00:03Z"
}
```
On successful completion, the response also includes the optional
`result_entity_type` and `result_entity_id` fields. For an illumination task,
these identify its inference; the caller knows the task kind and fetches the raw
result from `GET /api/illuminations/raw/{inference_id}`. This endpoint is
illumination-specific, not a generic inference-result resolver. The fields are
omitted while no result reference is available.

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
expires. In all cases, it prints the `(envelope_id, run)` pair and current status.
On timeout, it reports the latest observed status and exits successfully: task
submission succeeded even though completion was not observed within the wait
window. The printed identity can be used for a later status lookup.

## 4. Design boundaries

- Queue names (`illuminate`, `search_index`) are REST submission destinations;
  they are not interchangeable with task identities.
- `envelope_id` identifies the logical task, while `run` identifies one
  execution of that logical task. Keep `run` separate from `envelope_id`; the
  pair is the exact-run handle.
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