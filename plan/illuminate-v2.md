# Illumination v2 — unified entity capture

**Status:** V2 has a relational `knowledge_entities` projection and successful v2 illumination now enqueues vector search indexing. API/WebUI capture producers request v2. V1 remains a legacy prompt/DTO and is deliberately raw-only: both versions persist inference JSON/provenance, but only v2 writes relational illumination, entity, query, and search-index rows.

## Problem

V1's legacy prompt returns separate `entities` and `social_media_accounts` arrays;
v2 unifies them in one `entities` list with an optional `platform_link`. V1 output is
kept only in `illumination_raws`; v2 output also becomes the application-facing
relational projection.

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
  V1 keeps its narrower `EntityType` and legacy DTO; the code comment records that it
  does not use `KnowledgeEntityType`.
- `src/model/knowledge_entity.rs` is the single v2 entity projection table.
  `insert_illumination_v2` populates it in source-result order.
- `knodes` and `social_medias` are retired from the model/API/UI. V2 capture reads,
  entity-type listing, search text, deletion, and templates use `knowledge_entities`.
- Rows are capture/result-scoped. Identical real-world entities in different captures
  or reruns remain separate; cross-capture identity, clustering, and merges are deferred.
- V1 requests store only `illumination_raws` and emit a warning. API/WebUI
  producers now request v2, so new captures update relational summaries, entities,
  and search-index content; explicitly requested v1 output remains raw-only.

The existing `illuminations` result parent, `illumination_raws`, `xqueries`, and
`search_index` remain. `knowledge_entities` is the single v2 entity table for all
entity types, including social accounts and platform-hosted communities. Each row
represents one entity from one v2 illumination result and is not deduplicated across
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

- One row represents one v2 entity in one illumination result. `id` is the stable row
  ID; zero-based `position` preserves model order. Names and handles are not unique.
- `entity_type` is stored as snake_case text from shared `KnowledgeEntityType`; V1
  retains its separate enum and writes no relational entity rows.
- `description` and canonical entity `search_query` are nullable. Flatten the optional
  platform link into nullable `platform`, `platform_handle`, and
  `platform_display_name`; v2 schema requires platform and handle when a link exists.
  No model-generated URL column or separate link table.
- V2 capture-wide `suggested_searches` remain in `xqueries`, distinct from each
  entity's `search_query`. Raw JSON/provenance stays in `illumination_raws`;
  `search_index` remains the derived result document.
- `knowledge_entities` replaces `knodes` and `social_medias` for relational v2
  persistence and its API/UI readers. V1 raw output does not create `illuminations`,
  `knowledge_entities`, `xqueries`, or `search_index` rows.
- V2 reruns append per-result rows and readers keep the existing per-route ordering.
  No migration of v1 rows or merge/deduplication logic; the full backfill populates
  the v2 projection.

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

Each typed result's `to_markdown()` is a lossy, whole-document rendering for side-by-side evaluation and console display. V1 and v2 preserve their schema-specific prompt/output differences; only v2 writes the relational KnowledgeEntity projection.

- The task payload's `prompt_version` selects `v1` or `v2`; API/WebUI producers enqueue v2. V1 persists raw only and does not create a relational illumination, KnowledgeEntity, xquery, or search-index row. V2 writes the relational projection and enqueues vector search indexing after persistence; task identity remains capture-scoped, so versions run sequentially per capture.


`dreamscroll_api --host <host> inference illuminate <capture_id>` defaults to `v1`; `--prompt-version v2` selects v2. Each accepted worker attempt creates a fresh UUID for its concrete inference. On success, `TaskRunInfo` and SSE expose the successful inference's ID as an `inference` result reference; retries use distinct IDs. The CLI fetches `/illuminations/raw/{inference_id}` and prints schema-specific Markdown to stdout.

The unused `GrokIlluminator` was removed: it read from a hard-coded `localdev/media` path instead of the configured storage provider, hard-coded JPEG MIME, returned empty entity/search fields, and no local/Docker/production config selected it. The separate `GrokFirestarter` used by `FIRESTARTER=grok` remains untouched.

## Decisions still to make

- V2 platform links require a handle and may include a display name; URLs are not model output. Deterministic post-inference URL construction remains to be evaluated for unambiguous platform/handle pairs.
- Whether and when to add canonical cross-capture identity and explicit relationships. These are distinct from unifying the current capture-level extraction schema.
