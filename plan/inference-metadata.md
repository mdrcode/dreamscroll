# General inference metadata

**Status:** implemented. TaskRun-backed illumination, Spark, and search-index embedding requests append usage metadata. Schema version 6 requires explicit `dreamscroll_admin sync_schema`; no historical metadata migration.

## Goal

Replace illumination-specific provider bookkeeping with one `InferenceMeta` entity that records useful metadata for any model inference operation. Keep typed, queryable fields for provider/model identity, duration, and normalized token counts; put additional request configuration in JSONB `params` and provider-reported details in JSONB `provider_metadata`.

## Current state

- `src/model/inference_meta.rs` owns the shared `inference_metas` usage ledger; the illumination- and Spark-specific metadata entities are retired.
- `llms::InferenceOutcome<T>`, declared with the request/client contracts in `src/llms/inference.rs`, carries metadata separately from `output: anyhow::Result<T>`, so provider-output and typed-validation errors do not discard available usage. `InferenceClient::infer` returns `anyhow::Result<InferenceOutcome<serde_json::Value>>` directly, without a response alias. The outer result handles failures before a usable response is available.
- Typed v1/v2 illumination values expose inherent `raw_json()` and `to_markdown()` methods. Raw persistence takes `&serde_json::Value` directly; no shared result/presentation trait is needed under `llms`.
- Gemini Interactions, Gemini/xAI Spark, and Vertex embedding adapters normalize response usage and retain the original usage object in `provider_metadata.provider_usage`.
- TaskRun orchestration uses `ServiceApiClient::record_inference` for one shared best-effort append before propagating output errors or writing results. Direct CLI inference and inline query embeddings do not persist usage.
- `TaskRun.run_id` is the stable string UUID used by result rows. It is distinct from the database-native `task_runs.id` primary key.

## Entity contract

`src/model/inference_meta.rs`, exported from `src/model/mod.rs`, declares table `inference_metas`:

```text
inference_metas(
  id SERIAL PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
  task_run_id TEXT NULL,
  attempt INTEGER NULL,
  task_type TEXT NULL,
  provider TEXT NOT NULL,
  backend TEXT NOT NULL,
  model TEXT NOT NULL,
  duration_ms BIGINT NOT NULL,
  tokens_in BIGINT NULL,
  tokens_in_cached BIGINT NULL,
  tokens_out BIGINT NULL,
  tokens_thought BIGINT NULL,
  params JSONB NOT NULL DEFAULT '{}',
  provider_metadata JSONB NOT NULL DEFAULT '{}'
)
```

- Use the database-generated `id` as the primary key; `task_run_id` is an additional lookup key containing `TaskRun.run_id`, not the numeric `task_runs.id`.
- Make `task_run_id` nullable so request-time embeddings (which have no TaskRun) can use the same entity. Add a non-unique index on `(task_run_id, attempt)` for lookup: a TaskRun attempt may generate multiple inference records, while many unassociated rows remain valid.
- For TaskRun-backed calls, populate `task_type` from the existing `task::Task::task_type()` contract (the current values are `illuminate`, `spark`, and `search_index`). `task_runs.task_type` uses the same values. Keep it nullable for inference operations without a TaskRun.
- Keep illumination's logical upsert semantics: one `illumination_raws` row and, when requested, one v2 `illuminations` projection per TaskRun. Both store their own nullable `inference_meta_id` FK identifying the request that produced their retained content. Raw and projection writes can succeed independently, so provenance is not inferred through their shared run ID. Each upsert replaces or clears its own link alongside its content; the projection does so inside its existing transaction. `IlluminationTask.persist_projection` defaults to true; false retains raw output and metadata but skips the projection and automatic search-index follow-up, leaving older projections/indexes untouched. Spark stores the same nullable FK directly on `sparks`. Metadata itself is append-only, including calls whose output fails validation or persistence.
- Keep token values nullable. `NULL` means the provider did not report that measure; do not invent a zero or silently substitute a locally estimated count.
- Define `tokens_in` as the provider-reported total input count, including cached input when the provider defines it that way. `tokens_in_cached` is a subset, never an extra amount to add. `tokens_out` follows the provider's own output-token definition. No separate `tokens_out_cached` column: current providers expose no such count, and cached response replay is a different request-level concern.
- `tokens_thought` is the provider-reported thinking/reasoning count, nullable when unavailable. It is separate from `tokens_out` for Gemini, but a subset of `tokens_out` for xAI Responses; do not add the two for xAI or assume a provider-independent billing formula.
- `params` is `{}` by default and contains only additional request configuration: `prompt_version`, configured tools, optional `generation_config.thinking_level`, and embedding task type/dimensions. Illumination keeps its existing prompt versions; Spark identifies the shared prompt as `spark_v1`. Response formats/schemas and their MIME types remain defined in code and sent to providers, but are not duplicated in per-request metadata. Capture counts, input prompts/media, credentials, and response data are excluded.
- `provider_metadata` is `{}` by default and contains raw `provider_usage`, `provider_request_id`, and `provider_grounding` (including citations/annotations) when available. Preserve raw usage so normalized fields remain auditable and future parsing can be corrected.
- Provider identity is `gemini` or `xai`; backend is `developer_api`, `vertex`, or `responses`. Model is the actual request model. Duration measures HTTP send through complete response-body receipt, excluding application validation and database persistence.
- Every currently persisted record has a user ID, run ID, claimed attempt, and task type. Run/attempt/type remain nullable in the general entity, but non-TaskRun persistence is outside this implementation.
- Keep the Rust persistence entity named `model::inference_meta::Model` / `InferenceMeta`; avoid conflating it with the existing `llms::InferenceMetadata` request-result value type.

Store one row per provider inference request, not one row per TaskRun or attempt. Append rows across retries and additional requests under the same TaskRun; never upsert metadata by `task_run_id`. Use `id` / `inference_meta_id` for exact association to the currently retained result, and `task_run_id` plus `attempt` to find calls for a run or attempt. The `(task_run_id, attempt)` index must be non-unique: one attempt may make multiple calls, and concurrent deliveries can claim the same attempt ordinal. Non-TaskRun inference rows may use NULL `task_run_id` and `attempt`; query-embedding persistence is deferred.

## Provider usage research and normalization

Always normalize from response usage fields, not estimated prompt tokenization or `total_tokens` arithmetic. Keep the original provider usage object in `provider_metadata.provider_usage` where supplied.

| Current operation / API | Input (`tokens_in`) | Cached input (`tokens_in_cached`) | Output (`tokens_out`) | Reasoning (`tokens_thought`) |
| --- | --- | --- | --- | --- |
| Gemini Interactions, Developer API and Vertex backend | `usage.total_input_tokens` | `usage.total_cached_tokens` | `usage.total_output_tokens` | `usage.total_thought_tokens` |
| Gemini `generateContent` used by Firestarter | `usageMetadata.promptTokenCount` | `usageMetadata.cachedContentTokenCount` when present | `usageMetadata.candidatesTokenCount` | `usageMetadata.thoughtsTokenCount` |
| xAI Responses API used by Grok Firestarter | `usage.input_tokens` | `usage.input_tokens_details.cached_tokens` | `usage.output_tokens` | `usage.output_tokens_details.reasoning_tokens` (included in output) |
| Vertex `embedContent` used by GeminiEmbedder | `usageMetadata.promptTokenCount` | Not reported; `NULL` | Not a generated-token measure; `NULL` | Not reported; `NULL` |

Provider definitions and caveats:

- Gemini Interactions defines `total_input_tokens` as prompt/context tokens and `total_cached_tokens` as the cached portion of the prompt; `total_output_tokens` excludes `total_thought_tokens`. Gemini bills generated output plus thinking tokens, so `tokens_in + tokens_out` is not a complete billing total. Raw usage retains modality breakdowns.
- Vertex `generateContent` defines prompt count as including cached content, and defines `cached_content_token_count` as the cached portion. Its `total_token_count` also includes tool-use prompt and thoughts tokens, so it is not an input-plus-output substitute. `thoughtsTokenCount` is retained separately in `tokens_thought`.
- xAI Responses reports `input_tokens`, `output_tokens`, and `total_tokens`; `input_tokens_details.cached_tokens` is a subset of input. `output_tokens_details.reasoning_tokens` is exposed as `tokens_thought` but is already part of `tokens_out`; never add it a second time.
- Gemini `embedContent` returns `usageMetadata.promptTokenCount`; an embedding vector is not an output-token count. Leave the output columns NULL.
- Treat unsupported or missing provider counters as NULL, not zero. These normalized totals are provider-reported and may not be directly comparable across vendors; retain raw usage JSON for the original semantics.

Research sources (official provider references):

- [Gemini Interactions API response / usage fields](https://ai.google.dev/static/api/interactions.md.txt) — `total_input_tokens`, `total_cached_tokens`, `total_output_tokens`, `total_thought_tokens`.
- [Gemini thinking pricing](https://ai.google.dev/gemini-api/docs/interactions/thinking#pricing) — billed generation includes both output and thinking tokens.
- [Vertex AI UsageMetadata](https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rpc/google.cloud.aiplatform.v1#google.cloud.aiplatform.v1.GenerateContentResponse.UsageMetadata) — prompt count includes cached content; total also includes tool-use and thoughts; cached-content count is a subset.
- [Gemini Developer API UsageMetadata](https://ai.google.dev/api/generate-content#UsageMetadata) — directly applicable to Spark's Developer API `generateContent` integration.
- [xAI prompt-cache usage and pricing](https://docs.x.ai/developers/advanced-api-usage/prompt-caching/usage-and-pricing) — Responses API cached count is `input_tokens_details.cached_tokens`.
- [Vertex `embedContent` REST response](https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rest/v1/projects.locations.publishers.models/embedContent) — embedding input usage is `usageMetadata.promptTokenCount`.
- [Vertex AI Generative AI pricing](https://cloud.google.com/gemini-enterprise-agent-platform/generative-ai/pricing#embedding-costs) — Gemini Embedding 2 (Unified Multimodal, Preview) lists online text input at $0.20 per 1M tokens and no output-token charge; verify current pricing before using this as a budget estimate.

## Implemented flow and cutover

1. Worker webhooks pass a borrowed `&TaskRun<T>` envelope and the attempt returned by `begin_attempt` into illumination, Spark, and search-index logic. Logic reads the task and stable run ID from that envelope; attempt stays separate because it identifies the current execution, not the queued run. No second run ID is added to task payloads; mutable task status is not reread to infer request provenance.
2. Provider adapters return an outcome containing metadata even when valid response JSON contains no usable output, malformed generated JSON, or an HTTP error. Typed illumination validation also preserves the outcome metadata. Transport/body/invalid-response-JSON errors without usable usage remain ordinary errors.
3. Orchestration appends metadata independently of result writes. An insert failure logs a warning and returns no metadata ID; it does not change the inference result or retry policy. Later output persistence failures do not roll back the usage record.
4. Raw illumination and the relational v2 projection each upsert their content and metadata ID together. Spark stores the metadata ID on its result row; readers load `inference_meta`. `SparkMetaInfo` exposes provider/backend/model, duration, nullable `i64` counts, request `params`, and separate `provider_metadata`. The obsolete normalized `total_tokens` field is removed; original provider totals remain in raw usage.
5. For the original metadata cutover, the operator manually removes obsolete metadata tables (`illumination_metas`, `spark_metas`) and old bookkeeping columns from `illumination_raws`, or drops/recreates affected prototype tables. Explicit `dreamscroll_admin sync_schema` creates the current schema, ensures the non-unique `(task_run_id, attempt)` index, and records schema version 6. Sync contains no inference-specific table/column drops. Version 3 added the nullable FK to `illuminations`; version 5 names the separate provider-details column `inference_metas.provider_metadata`, with an empty-object default; version 6 adds nullable `tokens_thought`. Upgrading from version 3, 4, or 5 requires only schema sync, not table drops. Existing `params` values are left untouched; the request-only contract applies to newly recorded requests. If the earlier version-4 column was already created, its contents are not transferred and that obsolete column can be removed manually.

SeaORM sync is additive and cannot remove retired tables/columns. Manual cleanup must also address the old required columns on `illumination_raws`; dropping only the retired metadata tables is insufficient for new inserts. Do not start the new binary before cleanup and sync. No automatic service-startup DDL or migration dependency is introduced.

## Boundaries and accepted tradeoffs

- Persistence is strictly TaskRun-backed. Inline query embeddings and direct CLI provider calls remain database-independent; no TaskRuns are created for accounting.
- Attempt to record usage whenever a usable provider response supplies it, even if output interpretation fails. Metadata writes are best-effort, not a guaranteed billing ledger: database failures or process termination can lose usage. No bookkeeping retries or recovery machinery.
- Concurrent deliveries can share an attempt ordinal or finish out of order. The non-unique index and exact result FK accommodate existing accepted behavior; this project adds no task fencing or request deduplication.
- Provider usage schemas evolve. Parser regression cases cover missing counts, explicit zero, cached subsets, large counts, provider-specific totals, and output failures. Preserve raw usage for audit and future reinterpretation.
- Schema cutover has no historical metadata transfer or rolling compatibility promise; destructive cleanup is operator-owned, followed by the explicit admin sync command in a controlled deployment.

## Verification

- `cargo check --all-targets` passed; `cargo test --lib` passed all 258 tests.
- Before automatic cleanup was removed, an approved throwaway-schema smoke exercised the destructive cutover, multiple requests sharing a run/attempt, exact raw-result association, warning-and-continue bookkeeping failure with a cleared result link, and Spark result association. The temporary schema and smoke source were removed; no real provider calls or app-database cutover were performed. Destructive cleanup is now manual.
- Projection-provenance follow-up: `cargo check --all-targets` and the focused persistence regression passed. An approved throwaway-schema smoke verified the version-2 schema upgrade preserves projection content, retry replacements update exact provenance, and metadata-free replacements clear the old link. Its temporary schema and source were removed.
- Result-trait cleanup: all callers now use inherent illumination methods and raw JSON persistence; `cargo check --all-targets` passed. No unit tests were run for this plumbing-only change.
- Request/response separation: `cargo test --lib -- --nocapture` passed 259 tests, including an approved temporary-schema smoke that exercised the version-3 upgrade, empty response-metadata defaults on legacy rows, separate request/response persistence, repeated sync preservation, and explicit schema removal. The temporary smoke source was removed; `cargo check --all-targets` then passed. No provider calls or app-table changes were made.
- Provider-metadata naming: renamed the in-memory field, persisted column, API field, and callers to `provider_metadata`; schema version 5 requires explicit sync. `cargo check --all-targets` passed. No unit tests or smoke checks were rerun for this naming-only change.
- Compact request params: response formats/schemas are no longer persisted, including Gemini Spark's generation config and xAI Spark's text-format config; provider request bodies are unchanged. The 8 Gemini client tests and `cargo check --all-targets` passed. No schema change or historical JSON rewrite is needed.
- Thinking-level controls: `cargo test --lib thinking` passed all 3 focused tests; `cargo check --all-targets` and the API CLI build passed. An approved offline smoke launched the real CLI against a disposable HTTP server, verified omitted/default and all four explicit levels in enqueue JSON, rendered exact-run results, rejected an invalid level before HTTP/auth, and checked flag help. The server and disposable keychain token were removed. No live provider inference or application-database writes were performed.
- Provider ownership: moved the enum to `src/llms/gemini/thinking_level.rs`, exposed only as `llms::gemini::ThinkingLevel`, and migrated task, request, CLI, REST, and test references without changing wire values or behavior. `cargo check --all-targets` passed; no unit tests or smoke checks were rerun for this module-only relocation.
- Raw-only illumination: the worker-result, projected-V2 Beacon, and CLI bool-validation regressions passed; `cargo check --all-targets` passed. An approved standalone smoke used a disposable Postgres schema and synthetic inference responses to verify false on a fresh capture stores raw output/usage without projection/entities/index tasks, omission retains normal projection/indexing, and false on a later run preserves prior projection/entities without a new index task. The schema and temporary runner source were removed. An actual-CLI offline smoke verified omitted/true/false payloads, rendered exact-run results, invalid/missing bool rejection before HTTP/auth, and help text; its HTTP server and keychain token were removed. No live provider calls or application-table writes were performed.
- Schema version 6 adds nullable `tokens_thought` to the inference usage ledger and exposes it in Spark metadata. Run `dreamscroll_admin sync_schema` before starting the updated application against an existing database.
- Thought-token accounting: all 20 focused tests passed across Gemini Interactions, Gemini/xAI Spark, and embeddings. The approved isolated Postgres smoke rejected a schema-5 startup, added the nullable column via explicit sync without losing prior usage, round-tripped NULL/zero/>32-bit counts through persistence and Spark JSON, and preserved appended rows across repeated sync. The disposable schema and runner source were removed; no live provider calls or application-table writes were performed.
- Final `cargo check --all-targets` passed after removing the temporary thought-token smoke runner.
