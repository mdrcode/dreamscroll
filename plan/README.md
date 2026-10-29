# Plan index

Where things stand, and where to look. Every doc states its status on line 3.

## How this directory is organized

| Location | Meaning |
| --- | --- |
| `plan/backlog.md` | The queue. Self-contained tasks, ordered, each one startable without this index. |
| `plan/pragmatism.md` | The ledger of accepted shortcuts. Check here before deciding to fix something. |
| top level | Live process docs, deferred decisions, and research. Nothing here is silently abandoned; each doc's status says what remains. |
| `archive/` | Design records for shipped work. Read for rationale; **do not treat as pending work**. |

**When work ships:** move its design doc to `archive/`, keep appending dated sections
inside it (never rewrite history), and update the row below.

**Referencing docs:** use repo-root paths (`plan/pragmatism.md`,
`plan/archive/sse.md`) so a reference is unambiguous from any directory.

## Active queue and reference

| Doc | Status | Scope |
| --- | --- | --- |
| [backlog.md](backlog.md) | Living — queue | Ordered next tasks, including pointers to the deferred plans below. |
| [pragmatism.md](pragmatism.md) | Living — ledger | Tolerated trade-offs, each with a revisit trigger. The default answer to "should we fix this now?" |
| [backfill.md](backfill.md) | Living — implemented | Typed backfills: bounded pumping, TaskRun correlation, dry-pump latch, status reporting. |
| [testing.md](testing.md) | Living | Two-tier test model, and what deliberately goes untested. |
| [smoke.md](smoke.md) | Living | Manual smoke procedure against the persistent local instance (`smoke-agent` account). |
| [devlog.md](devlog.md) | Living — historical | Reverse-chronological daily log. Last entry 2026-04-05; open items were triaged into `plan/backlog.md`. |
| [htmx4.md](htmx4.md) | Deferred decision | 2.0.11 pinned with SRI. 4.0 migration points and risks mapped; no timeline. |
| [feed-view-incremental-updates.md](feed-view-incremental-updates.md) | Planned follow-up | Replace the full `/feed` refetch after upload with server-defined membership updates. |
| [testing-auth-routes.md](testing-auth-routes.md) | Planned follow-up | Exercise protected routes through real Axum auth middleware on an isolated Postgres schema. |
| [testing-js-and-browser.md](testing-js-and-browser.md) | Planned follow-up | Browser-level coverage for SSE connection lifetime, partial refresh, and upload feedback. |
| [testing-coverage.md](testing-coverage.md) | Planned — initial | Strategy for coverage beyond the current two tiers. |
| [knowledge-entity-types.md](knowledge-entity-types.md) | Open questions | Current v2 entity shape; next taxonomy pass deferred until the environment is fully v2-backfilled. |
| [knowledge-wikidata-recommender.md](knowledge-wikidata-recommender.md) | Research | Wikipedia/Wikidata embeddings, SPARQL traversal, and disambiguation for entity linking. |
| [screenshot_3rd_party.md](screenshot_3rd_party.md) | Research | Screenshot provider comparison and selection criteria (Urlbox, ScreenshotOne). |
| [api_token_hardening.md](api_token_hardening.md) | Proposed | `iss`/`aud` claims, fallible config validation, and token-lifetime policy. |
| [static-serving-cdn.md](static-serving-cdn.md) | Recommendation | Why not to add an ALB or CDN for the small static bundle. |
| [media-serving.md](media-serving.md) | Partly implemented | Immutable-object cache policy shipped; private URL delivery waits on load-balancer/CDN work. |
| [schema-migration-strategy.md](schema-migration-strategy.md) | Partly implemented | Explicit `dreamscroll_db sync_schema` shipped; sync semantics and version marker. |
| [prod-deploy.md](prod-deploy.md) | Implemented; drift-clean — approved | Whole production release path: build/push, checked-in Cloud Run manifests, `prod-apply.sh` drift/apply/lint, `dreamscroll-db-sync` schema job, rollback. Service drift is clean; nothing applied, no job created. |
| [topology_and_throughput.md](topology_and_throughput.md) | Reference | Cloud Run / Cloud SQL connection and concurrency budgets. |
| [vertex-throughput.md](vertex-throughput.md) | Research — active | Vertex Gemini tail latency: evidence, hypotheses, experiments, mitigations. |
| [upgrade_3rd_party_crates.md](upgrade_3rd_party_crates.md) | Proposed process | Repeatable dependency-upgrade procedure. |
| [event-streaming-optimization.md](event-streaming-optimization.md) | Exploratory | Reducing per-connection task-status catch-up queries on `/events`. |
| [upload-attempts.md](upload-attempts.md) | Proposed | Structured upload-outcome logging and an `upload_attempts` table for failure triage and measurement. Nothing implemented; deferred to CLIENT_PULSE. |
| [upload-formats.md](upload-formats.md) | Partly implemented | Upload format policy: blocklist at the single ingest point, 415s, client `accept` steering shipped 2026-10-29. Client re-encode and server HEIC normalize deferred. |

## archive/ — shipped design records

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
| [cloud_build_release_deployment.md](archive/cloud_build_release_deployment.md) | Cloud Build tagged-release research: trigger flow, build identity, pricing, caching. Not pursued; local release tooling instead. |
