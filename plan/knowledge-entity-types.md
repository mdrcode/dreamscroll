# Knowledge entity types — working plan

**Status:** Record current decisions and open taxonomy questions. Defer the next taxonomy pass until an environment has been fully backfilled with v2.

## Current model

Illumination v2 uses one entity shape: `name`, optional `description`, `type`, and optional `platform_link`. The type is a practical capture-classification label, not a complete ontology or a claim that entity identity has been resolved across captures. The platform link is locator metadata, not a replacement for the entity.

V1 remains the production path: application producers enqueue v1 and only v1 writes the current relational projection. V2 is persisted as raw illumination for evaluation; it does not yet populate the KNode projection. The two versions have independent prompts and schemas.

## Decisions so far

- Keep one unified entity shape; do not restore a separate account list in v2.
- Do not add a generic person `role` field yet. Occupations such as “musician” differ from work-specific relationships such as “author of this book.” If structured querying creates a concrete need, model occupations separately from relationships rather than conflating them in one role string.
- Keep `financial` for securities and investing/trading concepts such as stocks, funds, indexes, indicators, and strategies. Classify the issuer as an `organization`; include a visible ticker in the financial entity’s name. Split this category only if real usage needs a further distinction.
- Use `religious_figure` for religiously significant figures without taking a position on historicity or theology. Do not classify such a figure as `fictional_character` merely because they appear in sacred narratives; reserve that type for figures presented as fictional. This safeguard is in the v2 prompt and schema.
- Preserve the existing distinctions: occurrences versus enduring organizations; specific products versus brands and makers; publications versus publishers/operators and standalone books; people/communities versus their platform accounts.

## What the current `unknown` inventory tells us

The authenticated `GET /api/entity?type=unknown` route and `dreamscroll_api unknown-entities` command list persisted, user-owned KNodes from non-archived captures. A local v1 scan returned 323 records. This is a v1 baseline, not a v2 evaluation set.

Several examples appear unknown because the active v1 enum lacks types that v2 already has, including `event`, `product`, and `organization` (for example, World War II, Monster Boomerang Petite, and Special Operations Executive). Other examples are movements, practices, or abstract concepts, such as Symbolism, Shinrin-yoku, and Network Science. Those may indicate a real taxonomy gap, but should be assessed from v2 results before adding a broad `concept` type. The scan also contained duplicate World War II KNodes in one capture; treat that as a possible extraction/deduplication issue, not a new entity type.

The current route does **not** return v2 raw-only entities. Do not interpret the legacy v1 unknown count as the number or character of v2 unknowns.

## Revisit after v2 backfill

After the v2 backfill is complete in the selected environment, use reviewed captures to decide:

1. Whether the less-defined types need clearer boundaries: `art_work`, `music`, `meme`, `software`, and `fictional_character`.
2. Whether recurring practices, rituals, movements, fields, or abstract concepts need a category, or should remain `unknown`.
3. Whether `unknown` means an identifiable entity with no supported category, and when an entity should instead be omitted.
4. Whether the prompt consistently prefers specific types over broader ones, and avoids duplicate entities within a capture.

Do not expand the type list solely to eliminate `unknown`. Add or split a type only when reviewed v2 outputs show recurring, user-relevant cases that cannot be represented clearly with the current labels and descriptions.

## Retrieval with a cached token

The CLI uses the macOS Keychain token cache, keyed by API host and username. Once a token has been cached for the target host, the assistant can retrieve the current persisted unknown KNodes with:

```sh
cargo run --bin dreamscroll_api -- --host <host> --user <username> unknown-entities
```

This command currently reads the KNode projection, not v2 raw illuminations. Before using it to review a v2 backfill, ensure the chosen environment exposes the v2 results through an appropriate authenticated read path; deciding how that path relates to the backfill is deferred with the methodology.

## Reference models

These are useful precedents, not schemas to adopt wholesale:

- [Schema.org Person](https://schema.org/Person), [Role](https://schema.org/Role), and [author](https://schema.org/author) separate occupation-like facts from roles that qualify relationships to works or events.
- [Wikidata occupation (P106)](https://www.wikidata.org/wiki/Property:P106) and [position held (P39)](https://www.wikidata.org/wiki/Property:P39) distinguish general professions from specific positions.
