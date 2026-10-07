# Google AI Interactions API — sidebar investigation

**Status:** Implemented and verified: 199 library tests, web binary check, and full-prompt/schema live smoke pass on both APIs. Cloud Run service-account validation remains.

## Objective and guardrail

Unify the former public `generateContent` and Vertex `generateContent` illumination adapters behind one Rust `reqwest` client using the Interactions API. Preserve the two Google backends because local/Docker use the Gemini Developer API and production uses Vertex; unify request construction, response handling, and the client implementation, not credentials or endpoint URLs.

The user approved removing the generated `google-cloud-aiplatform-v1` dependency if the unified path preserves required behavior. No third-party dependencies are being added.

## Implementation

- The reusable `GeminiInferenceClient` and typed input/request/response types live under the top-level `src/llms/gemini/` module, available to future inference flows.
- Backend-specific mapping targets the Developer API `v1beta/interactions` endpoint with an API key, or the Vertex `v1beta1/projects/{project}/locations/global/interactions` endpoint with ADC. Input and schema wire shapes differ, but both use the same client, inline image representation, output-step parser, and caller contract.
- `src/illumination/gemini/illuminator.rs` uses the client for image analysis, Google Search grounding, JSON-schema output, and `store: false`.
- Local and Docker config now use `ILLUMINATOR=gemini` plus `GEMINI_BACKEND=developer_api`; production uses `ILLUMINATOR=gemini` plus `GEMINI_BACKEND=vertex`.
- The old `publicapi.rs`, `vertexapi.rs`, and legacy `legacy.rs` path are removed. `GeminiPayloadMethod` / `GEMINI_PAYLOAD_METHOD` are removed; inputs are inline base64 on both backends.
- `google-cloud-aiplatform-v1` is removed from `Cargo.toml` and `Cargo.lock`. Embedding and vector search remain unchanged: they use `reqwest` + `google-cloud-auth` and `google-cloud-vectorsearch-v1`, respectively.

## Current API research

Google recommends the [Interactions API](https://ai.google.dev/gemini-api/docs/interactions-overview) for new Gemini integrations and documents it on both products:

- **Gemini Developer API:** `POST https://generativelanguage.googleapis.com/v1beta/interactions` (API key; current examples use this version for Gemini 3.8).
- **Vertex / Gemini Enterprise Agent Platform:** `POST https://aiplatform.googleapis.com/v1beta1/projects/{project}/locations/global/interactions` (Google Cloud OAuth/ADC).

Both endpoints document multimodal input, structured output, and Google Search tools. Live smoke checks confirmed a synthetic PNG, search tool, JSON schema, and `store: false` work on both endpoints with the configured Gemini 3.8 Flash model.

Interactions return an `Interaction` with ordered `steps`, rather than the `generateContent` candidate envelope. Extract the final model-output text step and deserialize that text into the existing structured illumination response. Search/tool steps can occur before the final model output, so do not assume the first step contains the JSON.

Both Interactions endpoints support `store: false`. Make illumination stateless by default; the Cloud API documents seven-day retention for stored interactions. Stateless mode avoids server-side capture retention, but also disables interaction chaining and background execution. Agentic features may later require an explicit decision about state and retention.

The official Google Gen AI SDK supports Python, JavaScript/TypeScript, Go, Java, and C#, but not Rust. The Rust `google-cloud-aiplatform-v1` crate is a Vertex API client, and the Vertex Interactions guide says the legacy Vertex SDKs do not support Interactions. A single Rust implementation would therefore use `reqwest` for both REST endpoints, with backend-specific endpoint/auth configuration. Vertex authentication can reuse the existing `google-cloud-auth` ADC token pattern.

References:

- [Developer API Interactions REST reference](https://ai.google.dev/api/interactions-api-v1)
- [Vertex Interactions REST reference](https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/models/interactions-api)
- [Developer API image input](https://ai.google.dev/gemini-api/docs/image-understanding)
- [Vertex Interactions developer guide](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/capabilities/interactions/developer-guide)
- [Google Gen AI SDK languages](https://ai.google.dev/gemini-api/docs/libraries)

## API and implementation notes

Google recommends the [Interactions API](https://ai.google.dev/gemini-api/docs/interactions-overview) for new Gemini integrations; `generateContent` remains supported. The official [Google Gen AI SDK](https://ai.google.dev/gemini-api/docs/libraries) does not list Rust. The generated Rust aiplatform crate is Vertex-specific and does not expose the Interactions endpoint, so the shared Rust implementation uses `reqwest` and existing `google-cloud-auth` ADC support.

Interactions return ordered `steps`, not the old `candidates[].content.parts[]` envelope. The response parser selects the final `model_output` text after any tool steps and keeps raw steps available for future function/agent flows. Illumination opts out of server-side retention with `store: false`.

The client builds one logical request but maps to backend-specific REST fields: Developer API input is a flat content array with an enveloped JSON `response_format`; Vertex uses a step-list input (`{"type":"user_input","content":[...]}`), not the older role/content turn-list shape, and sends the schema with `response_mime_type`. Keep these differences at the request boundary.

Both backends use inline base64 images. The smoke used a 1x1 PNG; production-sized payload latency was not benchmarked. Inline input matches the current 5 MiB upload cap and existing deployment config, but adds roughly one-third payload size over Vertex protobuf and drops the GCS URI optimization.

## Verification and remaining scope

- `cargo test --lib` passes (199 tests); `cargo check --bin dreamscroll_web` passes; `google-cloud-aiplatform-v1` is absent from `Cargo.toml` and `Cargo.lock`.
- Live smoke passed on both endpoints using the actual illumination prompt, response schema, and deserializer with a synthetic inline PNG, Google Search, and `store: false`. Developer API used the configured key; Vertex used local ADC.
- The Cloud Run service-account credential path has not been exercised in a deployed instance. Production-sized inline payload latency is also unmeasured; the live smoke used a 1x1 image.
- `config_prod.env` selects `ILLUMINATOR=gemini` and `GEMINI_BACKEND=vertex`, but deployed Cloud Run environment values are maintained outside the checked-in repository; apply the same settings there before rollout.
- The client returns raw interaction steps for future tool/agent flows; stateful storage, multi-turn continuation, and background execution remain future work.

