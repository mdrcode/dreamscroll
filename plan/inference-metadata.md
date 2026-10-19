# General inference metadata

**Status:** proposed; search-index TaskRun ID is now passed from the webhook into `logic::search_index::exec`. Inference-metadata persistence and schema changes remain unimplemented.

## Goal

Replace illumination-specific provider bookkeeping with one `InferenceMeta` entity that records useful metadata for any model inference operation. Keep typed, queryable fields for provider/model identity, duration, and normalized token counts; put request/task-specific details in JSONB `params`.

## Current state

- `src/model/illumination_meta.rs` declares `illumination_metas` with `id`, `user_id`, `illumination_id`, and `provider_name`. No application call sites reference this entity; removing its model/table is a straightforward cutover.
- `src/llms/inference_metadata.rs` already defines an in-memory `InferenceMetadata` value with prompt version, provider/backend/model, duration, request ID, and opaque provider usage JSON. It is currently persisted as columns on `illumination_raws` by `insert_illumination_raw`.
- `src/ignition/gemini.rs` and `src/ignition/grok.rs` are separate LLM integrations for Spark. Their `SparkMeta` / `spark_metas` persistence stores provider, duration, input/output/total counts, raw usage, grounding data, and capture count. Their token parsers currently do not record cached input counts.
- `src/search/gcloud/gemini_embedder.rs` calls Vertex `embedContent` for indexing and query embeddings, but discards response usage. Query embeddings are not tied to a `TaskRun`; indexing embeddings are.
- `TaskRun.run_id` is the stable string UUID used by result rows. It is distinct from the database-native `task_runs.id` primary key.

## Proposed entity contract

Add `src/model/inference_meta.rs`, exported from `src/model/mod.rs`, with table `inference_metas`:

```text
inference_metas(
  id SERIAL PRIMARY KEY,
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
  params JSONB NOT NULL DEFAULT '{}'
)
```

- Use the database-generated `id` as the primary key; `task_run_id` is an additional lookup key containing `TaskRun.run_id`, not the numeric `task_runs.id`.
- Make `task_run_id` nullable so request-time embeddings (which have no TaskRun) can use the same entity. Add a non-unique index on `(task_run_id, attempt)` for lookup: a TaskRun attempt may generate multiple inference records, while many unassociated rows remain valid.
- For TaskRun-backed calls, populate `task_type` from the existing `task::Task::task_type()` contract (the current values are `illuminate`, `spark`, and `search_index`). `task_runs.task_type` uses the same values. Keep it nullable for inference operations without a TaskRun.
- Keep the illumination result's existing logical upsert semantics: one `illumination_raws` row per `TaskRun`, containing the latest successful output. Store its current `inference_meta_id` FK so that output maps to the exact inference that produced it. In contrast, append a distinct `InferenceMeta` row per provider request and never overwrite prior token usage on retry; earlier metadata remains as the usage ledger even when its raw result is no longer retained.
- Keep token values nullable. `NULL` means the provider did not report that measure; do not invent a zero or silently substitute a locally estimated count.
- Define `tokens_in` as the provider-reported total input count, including cached input when the provider defines it that way. `tokens_in_cached` is a subset, never an extra amount to add. `tokens_out` follows the provider's own output-token definition. No separate `tokens_out_cached` column: current providers expose no such count, and cached response replay is a different request-level concern.
- `params` is `{}` by default. Store task-specific values such as `prompt_version`, `provider_request_id`, raw provider usage, tool/grounding details, and Spark's `input_capture_count` there. Preserve raw usage when available so normalized fields remain auditable and future parsing can be corrected.
- Keep the Rust persistence entity named `model::inference_meta::Model` / `InferenceMeta`; avoid conflating it with the existing `llms::InferenceMetadata` request-result value type.

Store one row per provider inference request, not one row per TaskRun or attempt. Append rows across retries and additional requests under the same TaskRun; never upsert metadata by `task_run_id`. Use `id` / `inference_meta_id` for exact association to the currently retained result, and `task_run_id` plus `attempt` to find calls for a run or attempt. The `(task_run_id, attempt)` index must be non-unique: one attempt may make multiple calls, and concurrent deliveries can claim the same attempt ordinal. Non-TaskRun inference rows may use NULL `task_run_id` and `attempt`; query-embedding persistence is deferred.

## Provider usage research and normalization

Always normalize from response usage fields, not estimated prompt tokenization or `total_tokens` arithmetic. Keep the original provider usage object in `params.provider_usage` where supplied.

| Current operation / API | Input (`tokens_in`) | Cached input (`tokens_in_cached`) | Output (`tokens_out`) |
| --- | --- | --- | --- |
| Gemini Interactions, Developer API and Vertex backend | `usage.total_input_tokens` | `usage.total_cached_tokens` | `usage.total_output_tokens` |
| Gemini `generateContent` used by Firestarter | `usageMetadata.promptTokenCount` | `usageMetadata.cachedContentTokenCount` when present | `usageMetadata.candidatesTokenCount` |
| xAI Responses API used by Grok Firestarter | `usage.input_tokens` | `usage.input_tokens_details.cached_tokens` | `usage.output_tokens` |
| Vertex `embedContent` used by GeminiEmbedder | `usageMetadata.promptTokenCount` | Not reported; `NULL` | Not a generated-token measure; `NULL` |

Provider definitions and caveats:

- Gemini Interactions defines `total_input_tokens` as prompt/context tokens and `total_cached_tokens` as the cached portion of the prompt; `total_output_tokens` counts generated response tokens. `total_thought_tokens` and modality breakdowns are additional usage fields and should remain in raw usage JSON, not be folded into these columns without an explicit provider definition.
- Vertex `generateContent` defines prompt count as including cached content, and defines `cached_content_token_count` as the cached portion. Its `total_token_count` also includes tool-use prompt and thoughts tokens, so it is not an input-plus-output substitute.
- xAI Responses reports `input_tokens`, `output_tokens`, and `total_tokens`; `input_tokens_details.cached_tokens` is a subset of input. `output_tokens_details.reasoning_tokens` is part of output usage and should remain in raw usage rather than added a second time.
- Gemini `embedContent` returns `usageMetadata.promptTokenCount`; an embedding vector is not an output-token count. Leave the output columns NULL.
- Treat unsupported or missing provider counters as NULL, not zero. These normalized totals are provider-reported and may not be directly comparable across vendors; retain raw usage JSON for the original semantics.

Research sources (official provider references):

- [Gemini Interactions API response / usage fields](https://ai.google.dev/static/api/interactions.md.txt) — `total_input_tokens`, `total_cached_tokens`, `total_output_tokens`, `total_thought_tokens`.
- [Vertex AI UsageMetadata](https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rpc/google.cloud.aiplatform.v1#google.cloud.aiplatform.v1.GenerateContentResponse.UsageMetadata) — prompt count includes cached content; total also includes tool-use and thoughts; cached-content count is a subset.
- [xAI prompt-cache usage and pricing](https://docs.x.ai/developers/advanced-api-usage/prompt-caching/usage-and-pricing) — Responses API cached count is `input_tokens_details.cached_tokens`.
- [Vertex `embedContent` REST response](https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/rest/v1/projects.locations.publishers.models/embedContent) — embedding input usage is `usageMetadata.promptTokenCount`.
- [Vertex AI Generative AI pricing](https://cloud.google.com/gemini-enterprise-agent-platform/generative-ai/pricing#embedding-costs) — Gemini Embedding 2 (Unified Multimodal, Preview) lists online text input at $0.20 per 1M tokens and no output-token charge; verify current pricing before using this as a budget estimate.

## Implementation sequence

1. Add the entity and schema export; remove `IlluminationMeta` and its model export. Rely on the existing explicit `dreamscroll_admin sync_schema` workflow and bump `REQUIRED_SCHEMA_VERSION` for the schema change.
2. Extend the in-memory inference result metadata contract with the three optional normalized `i64` counts and task parameters. Normalize Gemini Interactions usage in the Gemini adapter; normalize GenerateContent and xAI Responses usage in their existing provider adapters. Normalize Vertex embedding usage for TaskRun-backed search indexing; defer persistence for inline query embeddings (see the code TODO).
3. Persist one `InferenceMeta` row per provider request; metadata is append-only across retries. When a provider response includes usage, make that metadata available even if output parsing or validation fails, then use one shared best-effort persistence path. If the metadata insert fails, log a warning and continue the inference flow rather than re-invoking a billable provider call solely to retry bookkeeping. Keep illumination's existing TaskRun-keyed raw-result upsert, add `inference_meta_id`, and update it to point to the metadata row for the latest successful output. This preserves app result semantics while retaining token usage for earlier calls. When retiring `spark_metas`, preserve the direct link from each Spark result to its `InferenceMeta`. Pass run ID and attempt into metadata persistence. For search-index, pass `TaskRun.run_id` from the webhook to logic, then through embedding metadata persistence; do not add it to the task payload. Query-embedding persistence is deferred.
4. Move Spark-specific values (`input_capture_count`, provider grounding, and provider usage) into `params`; update Spark metadata readers/API projections before retiring the redundant provider/token columns and `spark_metas` table. Do not leave a second source of truth for provider, model, duration, or token counts.
5. Remove obsolete fields, model exports, and tests/call sites after all readers have moved. Add provider parser regression cases covering missing fields, cached-token subsets, large counts, and provider-specific totals. Verify that multiple requests for one TaskRun create distinct metadata rows and that each persisted result points to the metadata row for the request that produced it, including after a retry.
6. Apply schema with the explicit admin sync command. Existing metadata fields/tables may be discarded; no historical data transfer is required. Reset local/test databases if needed for the prototype cutover.

## Boundaries and risks

- Scope includes Gemini Interactions illumination, Gemini/xAI Spark, and TaskRun-backed Gemini embedding for search indexing. Search-index receives a `TaskRun<SearchIndexTask>` envelope with `run_id`; the webhook now passes it into `logic::search_index::exec`. The ID is not yet attached to embedding metadata or stored with embedding usage because inference-metadata persistence is not implemented. Do not add a second run ID to the task payload.
- Query embeddings are generated inline during synchronous authenticated `GET /api/search` requests and have no TaskRun. Their usage persistence is deferred; do not create TaskRuns solely for query accounting. A TODO at the call site tracks this work.
- Attempt to persist usage whenever a provider response supplies it, including responses whose generated output later fails parsing or validation. Keep this to a shared best-effort insert; metadata-write failure logs a warning and does not trigger another inference attempt. Transport errors without usage cannot yield token counts.
- Provider usage schemas evolve. Keep fixture tests against provider payload shape and preserve raw usage in `params`.
- Schema sync is reconciliation, not a data migration; old metadata can be discarded for this prototype cutover.
