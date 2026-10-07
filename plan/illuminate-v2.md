# Illumination v2 — unified entity capture

**Status:** Design plan. No implementation changes have been made yet.

## Problem

Illumination currently models ordinary knowledge entities and social-media accounts as separate concepts throughout the pipeline. The Gemini prompt directs the model not to include accounts in `entities`; the response schema has separate `entities` and `social_media_accounts` arrays; persistence uses separate `knodes` and `social_medias` models; and API/UI code exposes and renders them separately.

That split makes it easy for the model to treat an account as a substitute for the person behind it. A result such as “Joe Smith’s X account” is not an adequate description of Joe Smith.

## Product priorities

In order:

1. Capture the primary entity's intrinsic identity.
2. Give it a rich, useful description that stands on its own.
3. Attach a revisit-able platform link when one is visible and clearly associated.

Descriptions are about the entity itself. Do not use them to describe the screenshot, the app's schema, or limits of the model's knowledge. Do not pad a description with boilerplate when there is no useful information to add.

## Proposed entity model

Use one entity framework with common fields such as `name`, `description`, and `type`, plus optional typed platform-link metadata. A platform link is secondary metadata, not a replacement for the entity or its description.

```json
{
  "type": "real_person",
  "name": "Joe Smith",
  "description": "Joe Smith is notable for …",
  "platform_link": {
    "platform": "x_twitter",
    "handle": "@joesmith",
    "url": "https://x.com/joesmith"
  }
}
```

Apply the same pattern to entities that live on a platform:

- **Person with a clearly associated social profile:** one person entity with the person's description and an optional platform link. Do not create a second, equally weighted account entity by default.
- **Online community:** an `online_community` entity, e.g. `NFCWestMemeWar`, described by what the community is about, with Reddit and `r/NFCWestMemeWar` as platform-link metadata.
- **Account with no identified person behind it:** a `social_media_account` entity named by its visible name or handle, with its platform link. Describe useful information about the account (for example, its topic) when supported. Do not invent a person or use disclaimers such as “this capture does not establish who runs it.”

A separate account entity is appropriate when the account itself is independently meaningful, or when it is the primary identifiable entity. The common person-plus-account case should prioritize the person and attach the account as a link.

## Illumination model behavior

The prompt and structured response schema must express the priority above, not the current exclusion rule. The output should keep the entity description and platform link in distinct fields. In particular:

- Establish and describe the primary person, community, or other entity independently of its handle.
- Add a platform link only when the image clearly associates it with that entity.
- Preserve the entity's meaningful description; never replace it with a description of its account.
- Do not infer that an account is authentic or controlled by a person merely from a matching name or handle.
- Do not generate a person entity from an anonymous account without evidence.
- Do not put capture-specific or schema-specific caveats in the entity description.

The current prompt explicitly excludes social accounts from `entities`, and Gemini's response schema mirrors that split. Both must change together. Add concrete examples for a notable person with an account, a platform-hosted community, and an anonymous account. Flash-class models are expected to handle clear examples, but this is a hypothesis to validate against reviewed screenshots, not a guarantee from prompt wording alone.

## Code areas affected

A full cutover will need to align the end-to-end contract:

- `src/illumination/illuminator.rs`: replace parallel entity/account collections with the unified entity and platform-link types.
- `src/illumination/gemini/prompts.rs` and `src/illumination/gemini/response.rs`: update instructions and structured output schema together.
- `src/api/service/insert_illumination.rs` and `src/model/`: persist entities through a unified path rather than separate KNode/social-media paths.
- `src/api/schema/illuminationinfo.rs`, `entityinfo.rs`, `infomaker.rs`, and capture loaders: expose the unified records consistently.
- `web/v2/templates/partials/cards/capture.html.tera` and `capture_detail.html.tera`: render unified entities and their links without treating platform links as a competing entity category.
- Search-index formatting and entity-detail lookup/routes: migrate callers to the unified representation and include useful entity/link text where appropriate.

Today, extracted KNodes and social-media rows are capture-scoped; the code does not resolve the same real-world entity across captures or store graph edges. Unifying these records alone must not be presented as solving cross-capture identity resolution.

## Acceptance criteria

- A screenshot of a notable person and their clearly attributed account yields a person entity whose description explains the person; the account handle is separate optional metadata.
- A subreddit or similar platform-hosted community is represented as a community entity with its platform link.
- An anonymous account is captured as an account entity without fabricating a person or adding app/model disclaimers to its description.
- Missing or ambiguous links do not displace the primary entity or cause an unsupported association.
- Prompt, structured response, persistence, API, search, and UI all use the unified entity contract; no parallel social-media entity path remains by accident.

## Gemini client and API direction

The illumination provider now uses one reusable Rust `GeminiInferenceClient` over `reqwest` and the Interactions API. `GEMINI_BACKEND` selects the Developer API for local/Docker and Vertex with ADC for production; prompt, input, schema, tools, and `store` are per-call options.

The large `google-cloud-aiplatform-v1` dependency is removed from the inference path. Vertex embeddings still call `embedContent` via REST, and vector storage/search still use `google-cloud-vectorsearch-v1`.

See [the Interactions API sidebar](google-ai-interactions-api.md) for the API-specific request mapping, verification, and remaining trade-offs.

## Decisions still to make

- Whether an entity can have one `platform_link` or multiple platform links.
- Whether descriptions may be absent when no useful, supportable description exists, rather than requiring filler text.
- Whether social-account entities use the same platform-link shape as other entities or whether the link is their defining locator.
- Whether and when to add canonical cross-capture identity and explicit relationships. These are distinct from unifying the current capture-level extraction schema.
