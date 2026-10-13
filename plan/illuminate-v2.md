# Illumination v2 — unified entity capture

**Status:** The provider-neutral inference client and per-task model selection are implemented. V1 and v2 retain versioned prompts/schemas, both persist raw output and relational illumination results, and both now project entity records into `knowledge_entities`. V1's prompt/DTO remains legacy: it keeps its own `EntityType` and separate `social_media_accounts` field; v2 uses shared `KnowledgeEntityType`.

## Problem

V1's legacy prompt still returns separate `entities` and `social_media_accounts` arrays.
V2 unifies them in one `entities` list with an optional `platform_link`; both
projections now persist to `knowledge_entities`, while the v1 prompt/DTO remains
unchanged.

That split makes it easy for the model to treat an account as a substitute for the person behind it. A result such as “Joe Smith’s X account” is not an adequate description of Joe Smith.

## Product priorities

In order:

1. Capture the primary entity's intrinsic identity.
2. Give it a rich, useful description that stands on its own.
3. Attach a revisit-able platform link when one is visible and clearly associated.

Descriptions are about the entity itself. Do not use them to describe the screenshot, the app's schema, or limits of the model's knowledge. Do not pad a description with boilerplate when there is no useful information to add.

## Unified entity model

Use one entity framework with common fields such as `name`, `description`, and `type`, plus optional typed platform-link metadata. A platform link is secondary metadata, not a replacement for the entity or its description.

```json
{
  "type": "real_person",
  "name": "Joe Smith",
  "description": "Joe Smith is notable for …",
  "platform_link": {
    "platform": "x_twitter",
    "handle": "@joesmith"
  }
}
```

`platform_link.display_name` optionally preserves the name visibly shown by the platform. It is distinct from the entity's name and the account handle, is not a locator, and does not establish account ownership.

The v2 prompt and schema request platform, handle, and optional display_name only; the model does not generate URLs. Deterministic URL enrichment from platform and handle is a separate post-inference step and should only be used for unambiguous mappings.

Apply the same pattern to entities that live on a platform:

- **Person with a clearly associated social profile:** one person entity with the person's description and an optional platform link. Do not create a second, equally weighted account entity by default.
- **Online community:** an `online_community` entity, e.g. `NFCWestMemeWar`, described by what the community is about, with Reddit and `r/NFCWestMemeWar` as platform-link metadata.
- **Account with no identified person behind it:** a `social_media_account` entity named by its visible name or handle, with its platform link. Describe useful information about the account (for example, its topic) when supported. Do not invent a person or use disclaimers such as “this capture does not establish who runs it.”

A separate account entity is appropriate when the account itself is independently meaningful, or when it is the primary identifiable entity. The common person-plus-account case should prioritize the person and attach the account as a link.

## Illumination model behavior

The v2 prompt and structured response schema enforce the priorities above. V1 keeps
its legacy prompt/schema until that version is retired. In particular:

- Establish and describe the primary person, community, or other entity independently of its handle.
- Add a platform link only when the image clearly associates it with that entity.
- Preserve the entity's meaningful description; never replace it with a description of its account.
- Do not infer that an account is authentic or controlled by a person merely from a matching name or handle.
- Distinguish occurrences from enduring organizations: festivals, conferences, competitions, performances, and launches are events; their organizers are organizations and should be separate entities only when independently identifiable.
- Distinguish a specific product from its brand or manufacturer; use `product` for a named item or model, `brand` for a brand, `organization` for its maker, and `software` for apps or programs.
- Use `publication` for an editorial outlet or periodical (such as a magazine, newspaper, journal, or digital editorial site), not its publisher/operator; `book` remains for standalone books.
- Classify religiously significant figures as `religious_figure`, not `fictional_character` merely because they appear in sacred narratives; this label takes no position on historicity or theology.
- Preserve a visible platform display name separately from the entity name and handle; do not treat it as proof of who owns an account.
- Do not generate a person entity from an anonymous account without evidence.
- Do not put capture-specific or schema-specific caveats in the entity description.
- Provide `search_query` only when it helps locate reliable, canonical information
  about that entity. Prefer official or primary sources, otherwise reputable
  references; keep the query narrow and non-exploratory. Keep it distinct from
  capture-level `suggested_searches` and never use it to invent identity.

The v2 prompt/schema include people, communities, and accounts in the unified entity
list, with platform links kept as secondary metadata. The v1 prompt remains a legacy
contract with a separate social-account array; its `EntityType` is intentionally not
the shared `KnowledgeEntityType`.

## Implemented persistence and consumers

- `src/illumination/knowledge_entity_type.rs` defines the shared v2 taxonomy.
  V1 retains its narrower `EntityType` and output contract; its persistence adapter
  maps both output arrays into the common table.
- `src/model/knowledge_entity.rs` is the single entity projection table.
  `insert_illumination_v1` and `insert_illumination_v2` populate it, preserving each
  result's entity order and source `illumination_id`.
- `knodes` and `social_medias` are retired from the model/API/UI. Capture reads,
  entity-type listing, search text, deletion, and templates use `knowledge_entities`.
- A single `KnowledgeEntityInfo` DTO serves both illumination responses and
  `/entity?type`; it exposes the source `capture_id`, not a full nested capture.
- Rows are still capture/result-scoped. Identical real-world entities in different
  captures or reruns remain separate; cross-capture identity, clustering, and merges
  are explicitly deferred.

## Knowledge entity persistence model

The existing `illuminations` result parent, `illumination_raws`, `xqueries`, and
`search_index` remain. The single unified entity table is `knowledge_entities`, used
for all entity types, including social accounts and platform-hosted communities. Each
row represents one entity from one illumination result and is not deduplicated across
captures or runs.

```text
knowledge_entities(
  id PK, user_id FK, capture_id FK, illumination_id FK,
  position INT NOT NULL, created_at TIMESTAMPTZ NOT NULL,
  name TEXT NOT NULL, entity_type TEXT NOT NULL,
  description TEXT NULL, search_query TEXT NULL,
  platform TEXT NULL, platform_handle TEXT NULL, platform_display_name TEXT NULL,
  UNIQUE (illumination_id, position)
)
```

- One row represents one entity in one illumination result. `id` is the stable row ID;
  zero-based `position` preserves model order. Names and handles are not unique, so
  identical real-world entities in different captures or reruns remain separate rows.
- `entity_type` is stored as snake_case text. V2's prompt schema derives its allowed
  values from the shared `KnowledgeEntityType` enum. V1 deliberately retains its
  legacy `EntityType` and narrower schema; its social-account output is mapped to
  `social_media_account` only by the v1 persistence adapter.
- `description` and entity-specific canonical `search_query` are nullable. The
  zero-or-one `platform_link` is flattened to nullable `platform`, `platform_handle`,
  and `platform_display_name`; the v1/v2 writers and v2 response schema pair platform
  and handle, but the table has no additional check constraint. No model-generated URL.
- Capture-wide `suggested_searches` continue to use the existing `xqueries` table and
  remain distinct from an entity's `search_query`. Raw JSON/provenance stays in
  `illumination_raws`; `search_index` remains the derived result document.
- `knowledge_entities` replaces the separate `knodes` and `social_medias` tables in
  the model, API, search input, deletion path, and capture UI. V1 and v2 projection
  writers both persist into it; no legacy rows are migrated, and the full backfill
  repopulates from the v2 output.
- Reruns still append per-result rows and readers keep the existing per-route result
  ordering. No active-head, batch-promotion, obsolete-row, merge, or deduplication
  logic is implemented. Cross-result identity remains a future project.

## Acceptance criteria

- A screenshot of a notable person and their clearly attributed account yields a person entity whose description explains the person; the account handle is separate optional metadata.
- A subreddit or similar platform-hosted community is represented as a community entity with its platform link.
- An anonymous account is captured as an account entity without fabricating a person or adding app/model disclaimers to its description.
- A named recurring festival, such as the Venice International Film Festival, is represented as an `event`; its organizer is an `organization` only when independently identifiable.
- A specific item such as the Nikon Z8 is represented as a `product`, distinct from its brand or manufacturer.
- A named magazine or digital editorial outlet is represented as a `publication`, distinct from its publisher/operator, brand, and standalone books.
- A religious figure such as Jesus or Muhammad is represented as `religious_figure`, without making claims about historicity or theology.
- Missing or ambiguous links do not displace the primary entity or cause an unsupported association.
- When visible, a platform display name is preserved separately from the entity name and handle, without being used as identity evidence.
- An ambiguously named entity may include a concise `search_query` to locate canonical
  information about the intended referent, not exploratory topics; omit it when its
  name and type already identify it.
- Prompt, structured response, persistence, API, search, and UI all use the unified entity contract; no parallel social-media entity path remains by accident.

## Gemini client and API direction

The illumination path uses the provider-neutral `llms::InferenceClient` contract for model selection, text/image inputs, structured response schemas, and capabilities such as web search. `GeminiInferenceClient` maps that request to the Gemini Interactions API over `reqwest`; `GEMINI_BACKEND` selects Developer API for local/Docker and Vertex with ADC for production. The adapter sets `store: false`, keeping screenshot interactions provider-side ephemeral.

The large `google-cloud-aiplatform-v1` dependency is removed from the inference path. Vertex embeddings still call `embedContent` via REST, and vector storage/search still use `google-cloud-vectorsearch-v1`.

See [the Interactions API sidebar](google-ai-interactions-api.md) for the API-specific request mapping, verification, and remaining trade-offs.

## Illumination module organization

- `src/illumination/v1/` retains the legacy v1 prompt/schema/DTO; its `EntityType` and
  separate social-account field intentionally do not use `KnowledgeEntityType`.
- `src/illumination/v2/` uses the shared `KnowledgeEntityType` for its unified output.

`src/llms/gemini/` remains provider plumbing only: authentication, endpoint mapping, generic request serialization, structured output options, and Gemini interaction response parsing. Both version modules depend on `llms::InferenceClient`, not Gemini interaction DTOs, alongside storage and inference metadata/result contracts.

Each typed result's `to_markdown()` is a lossy, whole-document rendering for side-by-side evaluation and console display. V1 and v2 preserve their schema-specific prompt/output differences while persisting through the unified entity table.

- The task payload's `prompt_version` selects `v1` or `v2`; API/WebUI producers still enqueue v1. Both versions persist raw output and relational illumination/entity/search-index projections. V2 remains the evaluation path until producers are switched; task identity remains capture-scoped, so versions run sequentially per capture.


`dreamscroll_api --host <host> inference illuminate <capture_id>` defaults to `v1`; `--prompt-version v2` selects v2. Each accepted worker attempt creates a fresh UUID for its concrete inference. On success, `TaskRunInfo` and SSE expose the successful inference's ID as an `inference` result reference; retries use distinct IDs. The CLI fetches `/illuminations/raw/{inference_id}` and prints schema-specific Markdown to stdout.

The unused `GrokIlluminator` was removed: it read from a hard-coded `localdev/media` path instead of the configured storage provider, hard-coded JPEG MIME, returned empty entity/search fields, and no local/Docker/production config selected it. The separate `GrokFirestarter` used by `FIRESTARTER=grok` remains untouched.

## Decisions still to make

- V2 platform links require a handle and may include a display name; URLs are not model output. Deterministic post-inference URL construction remains to be evaluated for unambiguous platform/handle pairs.
- Whether and when to add canonical cross-capture identity and explicit relationships. These are distinct from unifying the current capture-level extraction schema.
