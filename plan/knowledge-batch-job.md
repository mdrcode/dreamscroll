# Knowledge batch job — the nightly drain

**Status:** Proposed — **tabled 2026-11-14**. Design captured from the session that split it from
`plan/knowledge-task-queues.md` (the online piece, which is being built). Nothing here is started.

**Purpose:** a scheduled, bounded drain that resolves every knowledge entity no identity resolver has
annotated yet — the safety net under the online resolve task.

**Relationship to the online plan:** two levels, two failure classes.

| | `plan/knowledge-task-queues.md` (level 1) | this plan (level 2) |
| --- | --- | --- |
| trigger | a linked illumination (user action) | Cloud Scheduler, nightly |
| unit | one capture | a page of the pending set |
| transport | Cloud Tasks delivery | Cloud Run Job execution |
| failure | retries its own attempt, then drops | *is* the retry |

**See also:** `src/knowledge/helper.rs` (`resolve_all_pending`), `src/knowledge/evidence/selection.rs`
(`awaiting_resolver`), `gcloud/prod-dreamscroll-db.yaml` (the job this copies),
`src/knowledge/knowledge.md` §Rebuild.

## Why a Cloud Run Job, not a task or an endpoint

| | Cloud Tasks delivery | Scheduler → endpoint inline | **Cloud Run Job** |
| --- | --- | --- | --- |
| wall-time ceiling | 30 min (dispatch deadline) | 60 min (request timeout) | hours (`timeoutSeconds`) |
| holds a request slot | yes | yes | no |
| needs task plumbing | type, route, queue, payload | no | no |
| in-app `TaskRun` row | yes | no | no (Cloud Logging) |
| retry | queue policy, per delivery | Scheduler only | job `maxRetries` |
| precedent in repo | yes | no | **yes** — `dreamscroll-db-sync` |

The work is "run the binary that already has the command", at ≥1 s pacing for ~20 minutes. That is a
Job's shape, not a request's.

## The shape

| piece | change |
| --- | --- |
| `Dockerfile` | build and copy `dreamscroll_knowledge` beside `dreamscroll_web` / `dreamscroll_db` |
| `gcloud/prod-dreamscroll-knowledge.yaml` | Job manifest, modeled on the db one: `command: ["/app/dreamscroll_knowledge"]`, `args: ["identity", "resolve", "--max-requests", "1200"]`, Cloud SQL annotation, secret-mounted `POSTGRES_PASSWORD`, `timeoutSeconds: 3600`, `maxRetries: 1`, `parallelism: 1`, `taskCount: 1` |
| `gcloud/prod-apply.sh` | a second job beside `--db`: apply, drift-check, then `jobs execute` for the first run |
| Cloud Scheduler | one HTTP job with OIDC: `https://us-central1-run.googleapis.com/apis/run.googleapis.com/v1/namespaces/mdrcode/jobs/dreamscroll-knowledge-resolve:run` |

No task type, no payload, no route, no queue. The web service is not involved, so a 20-minute drain
holds no request slot and is not interrupted by a service deploy.

## Selection, ordering, scope

**The checkpoint is the annotation set.** "New since last checkpoint" is `awaiting_resolver`: an
entity leaves the pending set when a resolver annotates it, and a reset re-creates every entity as
pending, which is correct. Any stored cursor would be an entity id (reallocated by `reset`) or an
observation id (an entity can gain members after its origin), so the query is the only cursor that
survives a rebuild.

- **Oldest first** — the CLI's default. The online path serves fresh captures; a newest-first drain
  would let a burst of uploads starve the oldest pending entities.
- **Scope is *never resolved*, not *resolved long ago*.** Annotations are durable rows, not cache
  entries: a 180-day cache TTL does not make a QID stale. Re-auditing old answers is a different
  question and is not smuggled in here.

## The set that never empties

An entity with no Wikidata item (a private photo, an obscure product) has no annotation and stays
"pending" forever. On the measured graph (~639 unannotated entities) that is affordable because the
cache absorbs it: an empty answer stands 30 days, so a nightly pass re-asks from cache and refetches
each roughly monthly — a few dozen cold calls a night.

If the pending set grows faster than that arithmetic tolerates, the fix is an `identity_checked_at`
marker (one nullable column on `knowledge_entities`, written by a run that asked and got nothing) so
the sweep can skip an entity for the cache's TTL at the entity level. Not worth a column until then.

## Serialisation — the advisory lock

A Job does not share the online queue's concurrency 1, so the invariant must be explicit:
**one resolution run per database at a time**, because the gateway paces per run and the authority is
rate-limited globally.

```sql
SELECT pg_try_advisory_lock(<knowledge-resolution key>)
```

Taken for the duration of every resolution run — the `identity_resolve` task exec, this job, and
`identity resolve` on a developer box. A run that cannot take it exits cleanly: the online task
completes and leaves its entity pending (this job reads it), the job exits 0 and tries tomorrow.
This replaces both a second queue and any notion of a shared pacer.

This is the one piece shared with the online plan, and it is deferred until this job exists: while
only the queue runs resolution, its concurrency of 1 covers the online path, and a developer CLI run
is already the same overlap that exists today.

## Budget and retries

- `--max-requests 1200` at ≥1 s pacing is ≈20 minutes, comfortably inside `timeoutSeconds: 3600` and
  the 30-minute delivery ceiling that a queued version would have had.
- `maxRetries: 1` re-runs the whole command. Safe: the selection is derived and the cache is read
  first, so the retry re-asks only what the first attempt did not annotate. A refusal (`Budget`,
  `RateLimited`) is a completed run with a report, not an error, and is not retried.
- Cadence is a spend knob: nightly drains ~500 entities; a larger backlog is a one-time manual
  `jobs execute`, not a reason to run the scheduler hourly forever.

## Visibility

No `TaskRun` row — the db-sync job lives with the same. Cloud Logging plus
`gcloud run jobs executions list`; the CLI prints its report (`identities`, `cache hits`, `refused`).
If in-app history is wanted later, the job can persist a row itself — not in this pass.

## Work plan (when un-tabled)

1. Advisory lock in `identity resolve` and the resolve task exec (shared with the online plan).
2. `Dockerfile` target + copy.
3. `gcloud/prod-dreamscroll-knowledge.yaml`; `prod-apply.sh` second job (apply, drift-check, execute).
4. Cloud Scheduler job + IAM (`run.jobs.run` for the Scheduler service account).
5. "Awaiting a resolver" count in `knowledge stats`, so the backlog is visible.
6. Verify: one manual execution on prod, compare `knowledge stats` before/after; the fixed evaluation
   set must stay 181/185 · 10/15 · 5.

## Non-goals

- Re-resolving old annotations (drift), or expiring them.
- Proximity scan/auto-merge tasks; they stay operator commands.
- Multi-provider priority or per-user fairness.
- Unmerge; resolving a tombstone.

## Open questions

1. **Cadence:** nightly, or every few hours with a smaller `--max-requests`? The backlog-versus-
   lock-contention trade; one arg either way.
2. **The never-empty set:** accept the cache arithmetic above, or take `identity_checked_at` in the
   first version so a no-item entity is asked at most monthly per entity rather than per answer.
3. **Backlog growth:** today ~639 pending, ~500 drained per night. Watch `knowledge stats` in prod;
   if captures outpace the drain, raise cadence before adding a marker.
