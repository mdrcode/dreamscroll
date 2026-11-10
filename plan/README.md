# Plan index

Where things stand, and where to look. Every doc states its status on line 3.

## How this directory is organized

| Location | Meaning |
| --- | --- |
| `plan/backlog.md` | The queue. Self-contained tasks, ordered, each one startable without this index. |
| `plan/pragmatism.md` | The ledger of accepted shortcuts. Check here before deciding to fix something. |
| top level | Docs receiving active attention: live process, deferred decisions, open research. Nothing here is silently abandoned; each doc's status says what remains. |
| `archive/` | Docs **no longer receiving active attention** — shipped, abandoned, or superseded. Inert by default: do not read one unless a task names it or you need the rationale behind existing behavior. Unarchive it (and restore its row below) if its questions become live again. |

**When a doc stops receiving active attention** — shipped, abandoned, or superseded —
move it to `archive/` and update the rows below. Shipping alone is not the trigger:
a doc with live open questions stays at top level even if most of it is implemented
(`media-serving.md` is the example). Keep appending dated sections to an archived doc
rather than rewriting it; history is the point.

**Referencing docs:** use repo-root paths (`plan/pragmatism.md`,
`plan/archive/sse.md`) so a reference is unambiguous from any directory.

## Active queue and reference

| Doc | Status | Scope |
| --- | --- | --- |
| [backlog.md](backlog.md) | Living — queue | Ordered next tasks, including pointers to the deferred plans below. |
| [pragmatism.md](pragmatism.md) | Living — ledger | Tolerated trade-offs, each with a revisit trigger. The default answer to "should we fix this now?" |
| [testing.md](testing.md) | Living | Two-tier test model, and what deliberately goes untested. |
| [smoke.md](smoke.md) | Living | Manual smoke procedure against the persistent local instance (`smoke-agent` account). |
| [htmx4.md](htmx4.md) | Deferred decision | 2.0.11 pinned with SRI. 4.0 migration points and risks mapped; no timeline. |
| [feed-view-incremental-updates.md](feed-view-incremental-updates.md) | Planned follow-up | Replace the full `/feed` refetch after upload with server-defined membership updates. |
| [testing-auth-routes.md](testing-auth-routes.md) | Planned follow-up | Exercise protected routes through real Axum auth middleware on an isolated Postgres schema. |
| [testing-js-and-browser.md](testing-js-and-browser.md) | Planned follow-up | Browser-level coverage for SSE connection lifetime, partial refresh, and upload feedback. |
| [testing-coverage.md](testing-coverage.md) | Planned — initial | Strategy for coverage beyond the current two tiers. |
| [rest-testing.md](rest-testing.md) | Planned — design only | The REST surface: `test_support::rest` harness, auth matrix, route inventory, per-route contract assertions against the `.http` docs. |
| [lexicon.md](lexicon.md) | Authoritative — names | The vocabulary: observations, knowledge entities, evidence; the verbs; the rename map and the names considered and rejected. Read before renaming anything in the graph. |
| [knowledge-illumination-history.md](knowledge-illumination-history.md) | Implemented — 2026-11-01 | The graph keeps every reading of a capture, the view shows one. Trigger: a re-read described the wrong film and the correct reading was the superseded one. |
| [knowledge-entity-types.md](knowledge-entity-types.md) | Open questions | Current v2 entity shape; next taxonomy pass deferred until the environment is fully v2-backfilled. |
| [knowledge-canonical-entities.md](knowledge-canonical-entities.md) | Implemented — core graph; disclosure deferred | Observation → knowledge entity (no owner, projected from evidence) → evidence (append-only). Merge is a scored decision; identity is a claim. Names per [lexicon.md](lexicon.md). |
| [knowledge-claim-management.md](knowledge-claim-management.md) | Implemented — 2026-11-09 | Two phases (settle an identity, assert an edge), both callers use both; a veto names a pairing, so the 27 handle-routed authority claims are retractable. No claim ids. |
| [knowledge-resolution.md](knowledge-resolution.md) | Implemented (v14) | Cache-first resolution against evidence providers: the DB cache, the provider seam, Wikidata's three request kinds, confidence and corroboration. |
| [screenshot_3rd_party.md](screenshot_3rd_party.md) | Research | Screenshot provider comparison and selection criteria (Urlbox, ScreenshotOne). |
| [api_token_hardening.md](api_token_hardening.md) | Proposed | `iss`/`aud` claims, fallible config validation, and token-lifetime policy. |
| [media-serving.md](media-serving.md) | Partly implemented | Immutable-object cache policy shipped; private URL delivery waits on load-balancer/CDN work. |
| [prod-deploy.md](prod-deploy.md) | Implemented; drift-clean — approved | Whole production release path: build/push, checked-in Cloud Run manifests, `prod-apply.sh` drift/apply/lint, `dreamscroll-db-sync` schema job, rollback. Service drift is clean; manifests and job verified in production. |
| [topology_and_throughput.md](topology_and_throughput.md) | Reference | Cloud Run / Cloud SQL connection and concurrency budgets. |
| [vertex-throughput.md](vertex-throughput.md) | Research — active | Vertex Gemini tail latency: evidence, hypotheses, experiments, mitigations. |
| [upgrade_3rd_party_crates.md](upgrade_3rd_party_crates.md) | Proposed process | Repeatable dependency-upgrade procedure. |
| [event-streaming-optimization.md](event-streaming-optimization.md) | Exploratory | Reducing per-connection task-status catch-up queries on `/events`. |
| [upload-attempts.md](upload-attempts.md) | Proposed | Structured upload-outcome logging and an `upload_attempts` table for failure triage and measurement. Nothing implemented; deferred to CLIENT_PULSE. |
| [upload-formats.md](upload-formats.md) | Partly implemented | Upload format policy: blocklist at the single ingest point, 415s, client `accept` steering shipped 2026-10-29. Client re-encode and server HEIC normalize deferred. |

## archive/ — inactive records (default: ignore)

| Doc | Scope |
| --- | --- |
| [sse.md](archive/sse.md) | Authenticated SSE delivery: one stream per page, heartbeat, reconnect, catch-up. |
| [task-status.md](archive/task-status.md) | Task framework: identity, queues, retry/exhaustion policy, run dimension, query API. |
| [task-status-timing-progress-bar.md](archive/task-status-timing-progress-bar.md) | Persisted timing facts, update-in-place aggregates, estimated progress. |
| [illuminate-v2.md](archive/illuminate-v2.md) | Unified entity capture, relational projection, and the raw-only v1 legacy path. |
| [inference-metadata.md](archive/inference-metadata.md) | One `InferenceMeta` for illumination, Spark, and embedding requests. |
| [rest-task-submission.md](archive/rest-task-submission.md) | Authenticated REST task submission plus CLI wait/eval. |
| [db.md](archive/db.md) | DB clock and snapshot watermarks for card freshness. |
| [split_utility_binaries.md](archive/split_utility_binaries.md) | Split of `dreamscroll_util` into explicit admin and api utilities. |
| [webhook_oidc_same_service.md](archive/webhook_oidc_same_service.md) | Google OIDC verification for `/_wh/*` routes in the production service. |
| [google-ai-interactions-api.md](archive/google-ai-interactions-api.md) | One Interactions API client for both Google backends. |
| [webui-v2-audit.md](archive/webui-v2-audit.md) | Full audit of `web/v2`; all findings fixed or accepted. |
| [feed-route-rename.md](archive/feed-route-rename.md) | Feed fragment route renamed `/cards` → `/feed`; what moved and what deliberately did not. |
| [webui-capture-route.md](archive/webui-capture-route.md) | Capture page routes moved from `/detail` to `/capture`; upload field renamed with them. |
| [backfill.md](archive/backfill.md) | Typed backfill mechanism: bounded pumping, TaskRun correlation, dry-pump latch, status reporting. Implemented; no open questions. |
| [schema-migration-strategy.md](archive/schema-migration-strategy.md) | Explicit `sync_schema` design: model registry, marker, manual indexes; implemented and exercised in production. |
| [static-serving-cdn.md](archive/static-serving-cdn.md) | Decision: browser caching first, no ALB/CDN while the static bundle is small; implemented. |
| [cloud_build_release_deployment.md](archive/cloud_build_release_deployment.md) | Cloud Build tagged-release research: trigger flow, build identity, pricing, caching. Not pursued; local release tooling instead. |
| [knowledge-blocking.md](archive/knowledge-blocking.md) | Blocking research: channels/fusion/budgets not started; one SQL surface channel shipped. Rebuild guide moved to `src/knowledge/knowledge.md`. |
| [knowledge-verdicts.md](archive/knowledge-verdicts.md) | The modification log design (implemented); `distinct` later built, `veto` renamed `veto_identity`. Unmerge remains a non-goal. |
| [knowledge-promotion-audit.md](archive/knowledge-promotion-audit.md) | Complete local-corpus promotion audit (classifier era). |
| [knowledge-promotion-audit-prod.md](archive/knowledge-promotion-audit-prod.md) | Complete prod-corpus audit; superseded for prototyping by the evidence model. |
| [knowledge-promotion-frontier.md](archive/knowledge-promotion-frontier.md) | Analysis of the non-promoted 16% and the evidence each kind needs. |
| [knowledge-evidence-prototype.md](archive/knowledge-evidence-prototype.md) | Multi-dimensional evidence prototype: assessed, not shipped; failure catalogue. |
| [knowledge-wikidata-recommender.md](archive/knowledge-wikidata-recommender.md) | Long-term QID/embedding recommendation research; QID remains optional. |
| [devlog.md](archive/devlog.md) | Reverse-chronological daily log, retired 2026-04-05 when open items were triaged into `plan/backlog.md`. Read only for what the project was thinking on a given day. |
