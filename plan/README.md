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
| [lexicon.md](lexicon.md) | Authoritative — names | The vocabulary: observations, knowledge entities, evidence; the verbs; the rename map and the names considered and rejected. Read before renaming anything in the graph. |
| [knowledge-entity-attestations.md](knowledge-entity-attestations.md) | Proposed — not committed | Entity-level references and a three-state status (`identified` / `attested` / `provisional`): web-search results document an entity without pretending an article is its identity. Design only. |
| [wikidata-cleanup-20261112.md](wikidata-cleanup-20261112.md) | Active — mechanical pass shipped, review open | The `identity/wikidata/` module: what was consolidated and proven neutral, the one known scoring defect, and the design questions still open. |
| [media-serving.md](media-serving.md) | Partly implemented | Immutable-object cache policy shipped; private URL delivery waits on load-balancer/CDN work. |
| [prod-deploy.md](prod-deploy.md) | Implemented; drift-clean — approved | Whole production release path: build/push, checked-in Cloud Run manifests, `prod-apply.sh` drift/apply/lint, `dreamscroll-db-sync` schema job, rollback. Service drift is clean; manifests and job verified in production. |
| [topology_and_throughput.md](topology_and_throughput.md) | Reference | Cloud Run / Cloud SQL connection and concurrency budgets. |
| [vertex-throughput.md](vertex-throughput.md) | Research — active | Vertex Gemini tail latency: evidence, hypotheses, experiments, mitigations. |
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
| [htmx4.md](archive/htmx4.md) | HTMX 4 evaluation: migration points and risks; 2.0.11 pinned, no timeline. |
| [feed-view-incremental-updates.md](archive/feed-view-incremental-updates.md) | Incremental `/feed` updates after upload; planned, gated on refetch cost. |
| [testing-auth-routes.md](archive/testing-auth-routes.md) | Authenticated WebUI route tests through real middleware; planned. |
| [testing-js-and-browser.md](archive/testing-js-and-browser.md) | Browser-level SSE/refresh/upload coverage; planned. |
| [testing-coverage.md](archive/testing-coverage.md) | Coverage-tool strategy and gap inventory; initial plan. |
| [rest-testing.md](archive/rest-testing.md) | REST router harness, auth matrix and contract design; not implemented. |
| [knowledge-entity-types.md](archive/knowledge-entity-types.md) | v2 entity taxonomy decisions and deferred boundaries; model lives in code. |
| [screenshot_3rd_party.md](archive/screenshot_3rd_party.md) | Screenshot provider comparison (Urlbox/ScreenshotOne); no provider selected. |
| [api_token_hardening.md](archive/api_token_hardening.md) | API token `iss`/`aud`, validation and lifetime hardening; proposed. |
| [upgrade_3rd_party_crates.md](archive/upgrade_3rd_party_crates.md) | Dependency-upgrade procedure; proposed. |
| [event-streaming-optimization.md](archive/event-streaming-optimization.md) | `/events` catch-up query reduction; exploratory. |
| [upload-attempts.md](archive/upload-attempts.md) | Upload outcome log and `upload_attempts` table; proposed, deferred to CLIENT_PULSE. |
| [knowledge-canonical-entities.md](archive/knowledge-canonical-entities.md) | Core entity/evidence design and rationale; implemented, with `src/knowledge/knowledge.md` as the day-to-day model. Disclosure policy deferred. |
| [backfill.md](archive/backfill.md) | Typed backfill mechanism: bounded pumping, TaskRun correlation, dry-pump latch, status reporting. Implemented; no open questions. |
| [schema-migration-strategy.md](archive/schema-migration-strategy.md) | Explicit `sync_schema` design: model registry, marker, manual indexes; implemented and exercised in production. |
| [static-serving-cdn.md](archive/static-serving-cdn.md) | Decision: browser caching first, no ALB/CDN while the static bundle is small; implemented. |
| [cloud_build_release_deployment.md](archive/cloud_build_release_deployment.md) | Cloud Build tagged-release research: trigger flow, build identity, pricing, caching. Not pursued; local release tooling instead. |
| [knowledge-blocking.md](archive/knowledge-blocking.md) | Blocking research: channels/fusion/budgets not started; one SQL surface channel shipped. Rebuild guide moved to `src/knowledge/knowledge.md`. |
| [knowledge-claim-management.md](archive/knowledge-claim-management.md) | The claim edge and its two-phase write (schema 35–38), superseded by identity annotations in schema 39. Kept for the veto pairing rationale. |
| [knowledge-identity-annotations.md](archive/knowledge-identity-annotations.md) | The schema-39 migration that deleted the claim edge: an identity became an annotation on one entity. Implemented 2026-11-11; the model as built is `src/knowledge/knowledge.md`. |
| [knowledge-resolution.md](archive/knowledge-resolution.md) | The measured record of resolution: the cache-first door, the provider seam, the Wikidata resolver and what each scoring change bought. Implemented; the model is `src/knowledge/knowledge.md` and the door is `src/fetch/fetch.md`. |
| [knowledge-illumination-history.md](archive/knowledge-illumination-history.md) | Why the graph keeps every reading of a capture and the view shows one, with the cost measured on a rebuild. Implemented 2026-11-01; the open items moved to `plan/backlog.md`. |
| [knowledge-verdicts.md](archive/knowledge-verdicts.md) | The modification log design (implemented); `distinct` later built, `veto` renamed `veto_identity`. Unmerge remains a non-goal. |
| [knowledge-promotion-audit.md](archive/knowledge-promotion-audit.md) | Complete local-corpus promotion audit (classifier era). |
| [knowledge-promotion-audit-prod.md](archive/knowledge-promotion-audit-prod.md) | Complete prod-corpus audit; superseded for prototyping by the evidence model. |
| [knowledge-promotion-frontier.md](archive/knowledge-promotion-frontier.md) | Analysis of the non-promoted 16% and the evidence each kind needs. |
| [knowledge-evidence-prototype.md](archive/knowledge-evidence-prototype.md) | Multi-dimensional evidence prototype: assessed, not shipped; failure catalogue. |
| [knowledge-wikidata-recommender.md](archive/knowledge-wikidata-recommender.md) | Long-term QID/embedding recommendation research; QID remains optional. |
| [devlog.md](archive/devlog.md) | Reverse-chronological daily log, retired 2026-04-05 when open items were triaged into `plan/backlog.md`. Read only for what the project was thinking on a given day. |
