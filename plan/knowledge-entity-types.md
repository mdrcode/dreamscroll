# Knowledge entity types — working plan

**Status:** Record current v2 decisions and open taxonomy questions. Defer the next taxonomy pass until the selected environment has been fully backfilled with v2.

## Current model

Illumination v2 uses one entity shape: `name`, optional `description`, `type`, and optional `platform_link`. The type is a practical capture-classification label, not a complete ontology or a claim that entity identity has been resolved across captures. The platform link is locator metadata, not a replacement for the entity.

V2 is the application projection: API, REST, WebUI, and `dreamscroll_api unknown-entities` read the unified `knowledge_entities` table. V1 remains available only for explicit raw-result evaluation and stores inference output in `illumination_raws`; it does not write a relational projection. The versions retain separate prompts and schemas.

The WebUI exposes each row as an occurrence at `/entity/{id}`. That page shows the source capture and sibling entities, but it does not establish canonical identity across captures. Cross-capture grouping by exact `(name, type)` is deferred; the identity, merge, and similarity design now lives in [knowledge-canonical-entities.md](knowledge-canonical-entities.md).

## 2026-10-29: decisions from the full-corpus promotion audit

The audit in [knowledge-promotion-audit.md](knowledge-promotion-audit.md) reviewed all 2,518 local
mentions and settles several of the open questions below.

**No `persona` type.** The v2 prompt already models the three strings that were being confused:

- the entity's `name` (the person, the work, the community),
- `platform_link.handle` (the locator),
- `platform_link.display_name` (the name the platform shows).

It also already decides the persona case explicitly: "attach an account as platform_link only when
the image clearly associates that profile with the person; do not create a second, equally weighted
account entity by default" and "if an account is the only identifiable entity, represent it as
`social_media_account`; do not invent the person behind it". A `persona` type would ask the model to
re-make that decision as a *type*, at extraction time, from one screenshot — the judgment it is least
able to make — and there is no row it would apply to: the corpus contains **zero** captures where the
person and the persona appear as separate entities.

What is actually missing is that the persona name is a **label, not a node**. `Tsoding`, `Meta-Nomad`
and `clara` live in `display_name` or in prose ("known online as Tsoding"), so a later capture that
shows only the persona cannot match the person. The fix is an `aliases` array on the entity (persona
names, pseudonyms, work subtitles), which flows into `knowledge_aliases` and lets the two match
without splitting the node.

If a persona ever needs to be a node of its own — the user keeps the persona's work separate from the
person's biography — model it as a **relation** (`persona --pseudonym_of--> person`,
`account --operated_by--> person`), not a type. That is the first non-derived edge in the graph and
the trigger for the deferred edge table in
[knowledge-canonical-entities.md](knowledge-canonical-entities.md). Today's data does not ask for it.

**`unknown` is not one missing type — it is six, and half of them are classes.** Fetching P31 for
the 16 items that resolved (local analysis: `localdev/analysis/entity-promotion/taxonomy.md`, which is
gitignored with the rest of the corpus data):

| P31 | Entities | What Wikidata calls it |
| --- | --- | --- |
| `taxon` (Q16521) | Dahlia, *Clostridium butyricum*, Northern cardinal | "group or class of living things" → material |
| `painting technique` (Q1231896) | Chiaroscuro, Tenebrism | "visual arts technique" → artistic technique |
| `movement in cinema` (Q3001152) | New Objectivity | → art movement → cultural movement |
| `art genre` (Q1792379) | Figurative Art | → genre → category |
| `type of sport` (Q31629) | Eight-ball | a class of activities |
| `class of anatomical entity` (Q112826905) | Parasympathetic nervous system | a class of body parts |
| `human activity` (Q24902509) | Shinrin-yoku | → activity → process |
| `cultural generation` / `cohort` | Generation Z | a demographic category |
| `schema` (Q1051200) | Learning Pyramid | → heuristic → method = an information artifact |
| *(none)* | Dry stone wall (`P279 wall`, `P279 dry stone walling`), Smiling curve | modelled as a subclass, not an instance |

Three findings matter more than the label:

1. **`concept` would be wrong for the taxa.** A taxon is a class of living things, not an abstract
   idea; Wikidata's parent chain is "group or class of physical objects".
2. **Most of these are types, not instances.** `taxon`, `type of sport`, `class of anatomical
   entity`, `art genre` are metaclasses, and `dry stone wall` is modelled as a `subclass of` a wall.
   Our taxonomy assumes every row is an individual — a person, a book, a place — which is why these
   rows had nowhere to go.
3. **Two of the sixteen matches are wrong referents.** `Sword in the Stone` → a 2019 film project;
   `Social defeat stress` → a 1996 journal article. The validation gate matters more than the label.

**Precedent.** Upper ontologies do not have "concept" as a category. BFO and DOLCE split continuants
from occurrents, and within continuants distinguish material entities from dependent ones (dispositions,
roles, plans) and from social entities; SUMO has `Abstract` and `Physical`; Schema.org's nearest bucket
is `Intangible`, which does not cover techniques, movements or taxa. Wikidata itself has Q151885
"concept" but its modelling guidance is to use the most specific class available — exactly the
opposite of adding a catch-all.

**Recommendation.** Two additions, one axis, one field:

- **`concept`** — abstract or social kind: techniques, practices, movements, genres, models, cohorts,
  and non-material classes. This is the enum's honest upper bound, and it covers 13 of the 17.
- **`organism`** — a taxon: a class of living things. Three rows, and a recurring capture genre
  (nature, gardening, biology); typing *Dahlia* as `concept` would be a modelling error, not a
  simplification.
- **`referent_form` (`individual` | `kind`)** — the distinction the precedent actually uses, and the
  reason these rows resisted classification. Inferable from P31 (`taxon`, `type of …`, `class of …`,
  `genre`, `schema`, and `P279`-only items are kinds) and cheap to add. It also gives merges a veto:
  an individual never merges with a kind.
- **`wikidata_class` on the base row** (QID + label) — keep the precise class in the reference layer
  so the UI can say "painting technique" rather than "concept", without growing the enum.

Do not add five types for seventeen rows, and do not let `concept` absorb taxa.

**Cultivars are organisms, not products.** This is the case a single-topic corpus exposes. Local
dahlia captures show the model typing named cultivars as `product` — `Dahlia 'Rhubarb and Custard'`,
`Dahlia 'Kelsey Annie Joy'` — while the genus alone came back `unknown`. Wikidata agrees on the
referent class: `Dahlia 'Café au Lait'` is Q21160478, described as "Dahlia cultivar", and `cultivar`
(Q4886) is "a grouping of plants selected for desirable characteristics"; the two local cultivars have
no item at all. So:

- a named cultivar is `organism` with `referent_form = kind` (Wikidata's `wikidata_class` records
  `cultivar` and, where present, the QID);
- `product` stays for manufactured goods, which is what the prompt currently conflates;
- the prompt should name the cultivar when it is visible or labelled, and fall back to the genus
  otherwise — a dahlia-only corpus is one graph of varieties, not one node called "Dahlia".

Why this is not taxonomy perfectionism: a user whose captures are almost exclusively dahlias decides
the question. If cultivars are `product`, her entire graph is typed as merchandise and merges against
real products; if they are `organism`, the graph reads as a plant collection, and the few Wikidata
cultivars can still enrich it.

**Re-illumination is the duplicate source, not the model.** 34 local captures carry two or three
illuminations, and every one of the 84 "duplicate inside one capture" groups disappears when you
restrict to the latest illumination per capture: **0 groups repeat a name inside a single
illumination**. The model never duplicates within a run; appending a second run does. So the fix is
supersede-on-write, not a dedupe guard, and resolution must read the latest illumination only (see
[knowledge-canonical-entities.md](knowledge-canonical-entities.md)).

**`role_in_capture` is deferred, not rejected.** The model already attaches each entity's *own*
account rather than the post author's: in every local capture with three or more linked entities,
each entity carries its own handle and no handle is shared across distinct names. A per-entity
`author | subject | mentioned` field would make that checkable and would later give author→subject
edges, but no current defect requires it.

## Decisions so far

- Keep one unified entity shape; do not restore a separate account list in v2.
- Do not add a generic person `role` field yet. Occupations such as “musician” differ from work-specific relationships such as “author of this book.” If structured querying creates a concrete need, model occupations separately from relationships rather than conflating them in one role string.
- Keep `financial` for securities and investing/trading concepts such as stocks, funds, indexes, indicators, and strategies. Classify the issuer as an `organization`; include a visible ticker in the financial entity’s name. Split this category only if real usage needs a further distinction.
- Use `religious_figure` for religiously significant figures without taking a position on historicity or theology. Do not classify such a figure as `fictional_character` merely because they appear in sacred narratives; reserve that type for figures presented as fictional. This safeguard is in the v2 prompt and schema.
- Preserve the existing distinctions: occurrences versus enduring organizations; specific products versus brands and makers; publications versus publishers/operators and standalone books; people/communities versus their platform accounts.

## What the current `unknown` inventory tells us

The authenticated `GET /api/entity?type=unknown` route and `dreamscroll_api unknown-entities` command list user-owned v2 `knowledge_entities` of the requested type from non-archived captures. A local v1 scan returned 323 records; that is a historical v1 baseline, not a v2 evaluation set.

Several examples in that historical scan appeared unknown because the v1 enum lacked types that v2 supports, including `event`, `product`, and `organization` (for example, World War II, Monster Boomerang Petite, and Special Operations Executive). Other examples were movements, practices, or abstract concepts, such as Symbolism, Shinrin-yoku, and Network Science. These remain taxonomy questions to assess against v2 results. The scan also contained duplicate World War II KNodes in one capture; treat that as a historical extraction/deduplication observation, not a new entity type.

The current route reads the persisted v2 projection; it does not return explicit v1 results that exist only in `illumination_raws`. Do not compare the v2 unknown count directly with the historical v1 count.

## Revisit after v2 backfill

After the v2 backfill is complete in the selected environment, use reviewed captures to decide:

1. Whether the less-defined types need clearer boundaries: `art_work`, `music`, `meme`, `software`, and `fictional_character`.
2. Whether recurring practices, rituals, movements, fields, or abstract concepts need a category, or should remain `unknown`. **Answered by the 2026-10-29 audit: they do — add `concept` and `organism`.**
3. Whether `unknown` means an identifiable entity with no supported category, and when an entity should instead be omitted.
4. Whether the prompt consistently prefers specific types over broader ones, and avoids duplicate entities within a capture. **The duplicate half is answered: it does not duplicate inside a run — 0 groups — and the 84 apparent duplicates are re-illumination rows. Fix that at write time, not in the prompt.**

Do not expand the type list solely to eliminate `unknown`. Add or split a type only when reviewed v2 outputs show recurring, user-relevant cases that cannot be represented clearly with the current labels and descriptions.

## Retrieval with a cached token

The CLI uses the macOS Keychain token cache, keyed by API host and username. Once a token has been cached for the target host, the assistant can retrieve current persisted v2 unknown entities with:

```sh
cargo run --bin dreamscroll_api -- --host <host> --user <username> unknown-entities
```

This command reads the `knowledge_entities` projection from non-archived captures. Explicit v1 results that exist only in `illumination_raws` are intentionally excluded.

## Reference models

These are useful precedents, not schemas to adopt wholesale:

- [Schema.org Person](https://schema.org/Person), [Role](https://schema.org/Role), and [author](https://schema.org/author) separate occupation-like facts from roles that qualify relationships to works or events.
- [Wikidata occupation (P106)](https://www.wikidata.org/wiki/Property:P106) and [position held (P39)](https://www.wikidata.org/wiki/Property:P39) distinguish general professions from specific positions.
