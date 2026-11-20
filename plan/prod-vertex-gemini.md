# Vertex / Gemini operations

**Status:** Living — provider operations reference. Last updated 2026-11-20.

> **See also:** [`plan/prod-deploy.md`](prod-deploy.md) for the release path,
> [`plan/vertex-throughput.md`](vertex-throughput.md) for latency research,
> [`src/illumination/illumination.md`](../src/illumination/illumination.md) for the module contract.

Everything here is about the provider boundary: which backend and endpoint we call, what is stored,
how many tokens the model may generate, how to retrieve a stored interaction, and which schema
constraints Gemini actually enforces. Application behavior lives in the illumination docs; release
procedure lives in `prod-deploy.md`.

Code of record: `src/llms/gemini/client.rs` (`GeminiInferenceClient`), the task shape in
`src/logic/illuminate.rs`, and the debug flags in `src/util_cmds/api/illuminate.rs`.

## Backend and endpoints

`GEMINI_BACKEND` selects the backend:

| backend | create endpoint | auth |
| --- | --- | --- |
| `vertex` (production) | `POST https://aiplatform.googleapis.com/v1beta1/projects/{project}/locations/global/interactions` | Application Default Credentials / service account, `cloud-platform` scope |
| `developer_api` | `POST https://generativelanguage.googleapis.com/v1beta/interactions` | `GEMINI_API_KEY` |

The Vertex project comes from `GCLOUD_PROJECT_ID`; location is always `global` in code. Retrieval
works on both `v1` and `v1beta1`; the docs use `v1`.

The code sends the same interaction body to both backends. Differences that matter are storage,
retention, console visibility, and the shape of the provider id (below).

## Storage: `store`

`store` is a per-request policy, not a capability or tool. The app defaults to **false**; the task
field is `#[serde(default)]` so old queued payloads stay stateless.

| mode | what happens | constraints |
| --- | --- | --- |
| `store: false` (default) | Provider keeps no Interaction object. The response still carries an id, but it is not retrievable. | No `previous_interaction_id`, no `background: true`. Implicit caching still works. |
| `store: true` | The full interaction is stored: prompt, input media, steps, thought signatures, output. | Debug only. Retains user media at the provider. |

Retention:

- **Vertex:** stored interactions persist for **7 days** (Interactions developer guide).
- **Developer API:** 55 days Paid Tier, 1 day Free Tier, configurable shorter.

`store: false` is not a blanket zero-data guarantee. Paid Gemini API services still log prompts and
responses for a limited period for Prohibited Use Policy enforcement, and grounding with Google
Search stores prompt/context/output for 30 days with no opt-out. That grounding retention is
independent of `store` and applies whenever `web_search` is enabled.

Debug invocation:

```bash
dreamscroll_api illuminate <capture-id> --store true --thinking-summaries true --raw-json
```

Both flags travel on `IlluminationTask`, the REST request, and the CLI. The WebUI never sets them.

## Token budget and thinking

`MAX_OUTPUT_TOKENS = 8_192` in `src/llms/gemini/client.rs` is **ours**, not the provider default.
Gemini otherwise allows 65,536 output tokens in a foreground call.

- The ceiling covers **visible output plus thought tokens**. It is a hard infrastructure cutoff.
- Hitting it stops generation with status `incomplete` and returns truncated or empty output; serde
  then reports “not valid structured JSON” at the truncation point.
- It was added after capture 766 generated ~65k tokens over 335 s and truncated mid-JSON
  (`dac9102`, 2026-11-15). The 8,192 number avoids killing good runs: median illumination bills
  ~1,440 total output tokens.
- Two 2026-11-20 failures landed at **8,177 / 8,192**: one spent 7,860 on thoughts and only 317 on
  visible JSON, the other wrote 7,650 visible tokens and was cut inside a long `details` string.
- `thinking_level` (low/medium/high, default provider-chosen) is the supported lever for thought
  volume. Lowering it trades identification quality; it is not a default cost fix.
- The 4,000-character soft cap on `details` (prompt + schema description, `v2.4`) targets the
  long-output tail. It does not help the thought-starved case.

## Thought summaries and retrieval

Thought steps are present in the response, but by default they contain only an encrypted
`signature`; there is no readable reasoning. Request summaries at generation time:

```json
"generation_config": { "thinking_summaries": "auto" }
```

With that flag (and `store: true` to retrieve later), the stored interaction contains `thought`
steps with `summary` content blocks. The summaries are model-written retrospective summaries, not
the raw hidden chain; signatures remain opaque.

Retrieval recipe — no custom binary needed:

```bash
# 1. Find the interaction id recorded with the attempt.
psql "$DATABASE_URL" -c "
  select m.provider_metadata->>'provider_request_id'
  from inference_metas m
  where m.task_run_id = '<task-run-uuid>';"

# 2. Fetch with ADC against Vertex.
TOKEN=$(gcloud auth application-default print-access-token)
curl -sS \
  -H "Authorization: Bearer $TOKEN" \
  "https://aiplatform.googleapis.com/v1/projects/${GCLOUD_PROJECT_ID}/locations/global/interactions/${INTERACTION_ID}" \
  -o /tmp/interaction.json

# 3. Read the summaries.
jq '.steps[] | select(.type == "thought") | .summary[].text' /tmp/interaction.json
```

Gotchas:

- `provider_request_id` lives in `inference_metas.provider_metadata`, not its own column.
- Vertex ids look like `Ch...`; Developer API ids look like `req_...`. A Developer API id against
  the Vertex endpoint returns 404.
- With `store: false`, the id is a request trace id, not a retrievable interaction.
- AI Studio Logs show Developer API interactions. A stored Vertex interaction is retrieved through
  the API above, not AI Studio.
- Stored interactions expire; after the retention window the id returns 404.
- Without `thinking_summaries`, retrieval shows signature lengths only. That is expected.
- A summaries-enabled run costs more than a plain run; the 794 sample was 21.2 s / 772 output /
  1,822 thought versus 18.4 s / 755 / 1,671 for store-only. One sample, not a benchmark.

## Structured output constraints

Gemini's constrained decoding supports a JSON Schema subset:

- `maxItems` **is enforced** on arrays.
- `minLength` / `maxLength` are **not enforced**; the API strips/ignores them.
- `minProperties` / `maxProperties`, `uniqueItems`, `contains`, etc. are not enforced.
- `description` guides the model but enforces nothing.

Practical consequence: length limits are prompt-level soft limits. The app uses:

- `summary`: 1–2 sentences, ≤ 280 characters (Prompt + schema description).
- `details`: multi-paragraph, ≤ 4,000 characters (`v2.4`).
- entity `description`: currently unbounded in prose; observed p99 is 201 characters, max 256.

Corpus context (2026-11-20, 771 stored illuminations): `details` p50 1,089, p99 1,717, max 2,022
characters; entity descriptions p50 128, p99 201, max 256. The pathological failures were tens of
thousands of characters or thousands of thought tokens, not a gradual drift.

## Failure observability

A JSON parse failure returns before `insert_illumination_raw`, so the provider output does not reach
`illumination_raws`, and `task_runs.last_error_message` is capped at 2,000 characters. With
`store: false`, the provider does not retain it either.

Every failure that still has a provider payload now writes it to the nullable
`inference_metas.raw_output` column (`TEXT`, NULL on success):

- non-2xx response body;
- an invalid response envelope;
- no `model_output` step or empty output text;
- model text that is not valid JSON;
- valid JSON that fails the illumination shape.

The admin inference-meta detail endpoint exposes the field. The column is bounded by the
`max_output_tokens` ceiling; no separate size cap. A `store: true` debug rerun remains useful for
thought summaries and input media, but it cannot recover a failed stateless response that was
already dropped.

## Checklist

- Production backend is Vertex, location `global`, project from `GCLOUD_PROJECT_ID`.
- Keep `store: false` for normal illuminations. `store: true` is a debug opt-in that retains user
  media for 7 days.
- `max_output_tokens` includes thoughts; 8,192 is ours.
- No readable thoughts without `thinking_summaries: "auto"`.
- Retrieve stored Vertex interactions with `GET .../interactions/{id}` + ADC.
- Failed outputs are in `inference_metas.raw_output`; success rows are NULL.
- AI Studio is not the Vertex debugging surface.
- Schema `maxLength` does nothing; use prompt guidance and `maxItems` where enforcement matters.

## Sources

- [Interactions API overview](https://ai.google.dev/gemini-api/docs/interactions-overview) — storage,
  `store`, `previous_interaction_id`, retention, implicit caching.
- [Gemini thinking](https://ai.google.dev/gemini-api/docs/thinking#token-limits) —
  `max_output_tokens` includes thought tokens; hard cutoff; `incomplete` status.
- [Structured outputs](https://ai.google.dev/gemini-api/docs/structured-output#json-schema-support) —
  enforced and ignored schema keywords.
- [Zero data retention](https://ai.google.dev/gemini-api/docs/zdr) — abuse logging, grounding
  retention, `store=false`.
- [Vertex Interactions developer guide](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/capabilities/interactions/developer-guide) —
  Vertex storage and retention.
- [Vertex Interactions API reference](https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/models/interactions-api) —
  create/list/get/delete endpoints.
