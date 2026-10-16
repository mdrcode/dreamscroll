# Backfill management

**Status:** Flat TaskRun target ownership and worker verification are implemented; the backfill dispatcher remains unimplemented.

## Goal and scope

Build a small, code-defined backfill mechanism using existing Tasks, TaskRuns, and TaskMaster queues. Creating or changing a Backfill requires a deployment. A stable string ID (a fixed UUID is fine) identifies one Backfill execution. Developers own the mapping from IDs to definitions; the application does not detect reuse of an ID after changing its definition.

A Backfill definition provides:

- Its stable string ID.
- The target `TaskEntityKind`.
- A queryable target relation that selects target entities and their owners.
- A factory that creates the existing concrete Task's execution payload from a selected target entity.

Do not add a generic job framework, a second task execution system, or a per-target BackfillItem table.

## Ownership invariant

TaskRun target identity is the flat tuple `(entity_owner_id, entity_type, entity_id)`. The owner ID is the entity's actual owning user, not the user or administrator who requested work.

The task factory creates execution inputs from the selected target. Submission passes the target owner, kind, and ID separately from the Task payload; TaskRun does not derive ownership from the payload. Use the same owner for TaskRun persistence, `logical_id`, status queries, SSE, and downstream task submission. Keep `backfill_id` separate from entity ownership and from `logical_id`.

User-facing submission paths may use the authenticated context to authorize a request, but must verify it is permitted to submit work for the target owner. The admin pump obtains owner IDs from the server-side target query and must never use the administrator's context ID as the TaskRun owner.

Workers must verify that the TaskRun entity owner matches the persisted target entity's actual owner before processing. Persist outcome objects under that actual owner. Follow-up tasks inherit the verified target owner, never the administrator or an unchecked caller ID.

## TaskRun correlation and in-flight behavior

Add optional `backfill_id` to TaskRun persistence and its worker envelope. Ordinary TaskRuns leave it empty; backfill TaskRuns carry the stable Backfill ID. This allows task status to be aggregated by backfill.

Do not include `backfill_id` in `logical_id`. The logical identity uses the entity owner, task type, and target entity, so live and backfill submissions share the existing in-flight guard.

- If a live task is already in flight, a backfill submission is refused. No backfill TaskRun is created, so the target remains eligible in the next pump query.
- If a backfill task is in flight, a user's live rerun is refused. Keep this symmetric guard in v1 rather than allowing concurrent work on one entity.

## Target selection and pumping

The target relation is queried, not materialized as an unbounded collection. Conceptually:

```text
SELECT target.*
FROM (<Backfill's code-defined target query>) AS target
LEFT JOIN task_runs AS runs
  ON runs.backfill_id = <backfill_id>
 AND runs.entity_owner_id = target.owner_id
 AND runs.task_type = <backfill_task_type>
 AND runs.entity_type = target.entity_type
 AND runs.entity_id = target.entity_id
WHERE runs.id IS NULL
ORDER BY target.entity_id DESC
LIMIT <BATCH>
```

Each manual pump repeats this bounded query, creates the Task from each target, and submits it through the existing typed TaskMaster path. TaskRun rows are the per-target progress markers; no separate cursor is planned. The operator keeps pumping until a query returns zero rows.

The query is evaluated against current data on every pump; target membership is not frozen at the first pump. Newly eligible targets may appear while pumping. A zero-row result means there are no currently eligible targets without a backfill TaskRun.

An administrator invokes one pump through the admin API:

```text
dreamscroll_api backfill pump <string_id>
```

## Status and failures

Use TaskRuns tagged with `backfill_id` to report per-target and aggregate status. Any associated TaskRun, including a failed one, removes that target from later pump queries.

V1 reports `SubmissionFailed` and `CompleteFailure`; it does not retry or re-enqueue them. A refused submission caused by a live in-flight task has no associated backfill TaskRun and remains eligible for a later pump.

Concurrent pump requests could both select the same target before either creates a TaskRun. Serialize pumps per Backfill or otherwise protect TaskRun creation against that race; the mechanism remains open.

## Implementation sequence

1. **Implemented:** TaskRun stores flat target owner/type/ID fields separately from task payload. Submission context authorizes the actor but does not set target ownership, and workers verify entity ownership before processing.
2. Add optional `backfill_id` to TaskRun persistence/envelopes and the status queries needed for the left join and aggregate reporting.
3. Add the code-defined Backfill registry, bounded target query, task factory dispatch, pump admin API, and `dreamscroll_api` command.
