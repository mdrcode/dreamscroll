# Public entities and the private overlay

**Status:** Design. A parallel workstream to
[knowledge-canonical-entities.md](knowledge-canonical-entities.md) (phases 1–4); this layer is
phases P-a…P-d. Nothing implemented.

**See also:** [knowledge-canonical-entities.md](knowledge-canonical-entities.md) (private identity,
merges, similarity), [knowledge-wikidata-recommender.md](knowledge-wikidata-recommender.md) (the
QID-centric recommender research), [knowledge-entity-types.md](knowledge-entity-types.md) (taxonomy),
[pragmatism.md](pragmatism.md), [prod-deploy.md](prod-deploy.md) (topology).

## The correction

An earlier draft of this doc treated "sharing an entity across users" as a privacy problem. That was
wrong, and it is worth stating plainly:

> **The entity is public. The association is private.**

*Elon Musk*, *Game of Thrones*, *C++* are public, factual referents. A record that says "this is
Elon Musk, here is a definition, a Wikipedia link, an image" carries no user information and should be
shared, reused, enriched once, and shown to anyone. What is private is that **Bob captured him**, in
**which screenshot**, with **what description**, next to **which other entities**. That is the edge,
not the node.

The practical consequence is a single admission test for anything that wants to live in the shared
layer.

### The publishability test

> Every column of a shared entity record must be safe to show to **any** user, knowing **nothing**
> about who captured it. If a value fails that test, it is not a shared attribute.

This is a better rule than "public sources only" because it says what the record *is* rather than
where it came from, and it immediately answers the cases:

| Value | Publishable? | Where it lives |
| --- | --- | --- |
| Canonical label, aliases, universal description, image, Wikidata link, type | yes | shared |
| `"the documentary about him that I saved"` | no — meaningless without the capture | private |
| The user's own spelling `"the Elon guy"` | no — it is the user's phrasing | private |
| Co-capture edges, related-entity ranking | no — derived from one person's activity | private |
| "captured by 12 users" | publishable in isolation, but aggregate user behaviour | separate decision, see A5 |

## Two descriptions, not one

The design adopts two separate descriptions, one per layer:

```
public entity                          private node
  definition (universal)                 context (per user)
  "American businessman and investor,    "Saved from the SpaceX profile;
   co-founder of Tesla…"                 the bit about the Mars timeline"
```

They answer different questions and should not compete:

- The **universal description** is generated once per public entity, from public sources, and is
  identical for everyone. It is the "what is this" answer, and it is the thing we refuse to compute
  N times.
- The **private description** is the model's reading of *this capture*, or the user's own words. It
  is the "why is this in my graph" answer. It is short, contextual, and often more interesting than
  a definition.

Two rules keep the layers honest:

1. **User text is never an input to a universal description.** Not summarized, not averaged, not
   "used only for coverage". Generation reads public sources (Wikidata claims, Wikipedia lead, or
   grounded web text); the private description is a dead end. Otherwise the shared record stops being
   publishable the moment someone's private context bleeds into it — and the failure is invisible.
2. **The private description never overrides the universal one; it accompanies it.** Private text is
   not a better definition, it is a different kind of statement. Rendering both is the honest UI:
   *what it is* / *why it's here*.

## What is shared, what is private

Nothing here is about hiding entities. It is about keeping the shared record publishable and the
per-user record contextual.

| Concern | Scope | Why |
| --- | --- | --- |
| Entity identity (which public thing), QID | **shared** | public fact; the only cross-user anchor |
| Platform handle + profile URL, when promoted | **shared** | a public profile is the identifier; the account is the referent |
| Canonical label, aliases, universal description, image, sitelinks, P31 type | **shared** | definitions, computed once |
| Universal-description embedding (from public text) | **shared** | derived from public text only |
| Mention text, capture, media, annotations | **private** | user content |
| The user's chosen label, contextual description, notes | **private** | user-authored / capture-derived |
| Node↔mention assignment; merges between private nodes | **private** | interpretation, per-user, reversible |
| Private node embedding (includes user text) | **private** | derived from the user's screenshot |
| Co-capture edges, related-entity ranking | **private** | one person's activity |
| Public↔public merges (QID redirects) | **shared** | reference-data correction |
| Cross-user counts, "N users captured this", user↔user similarity | **separate decision** | aggregate behaviour, not an entity fact |

So the previous draft's "privacy cost: none if nothing queries across users" was the right
conclusion attached to the wrong reasoning. The reasoning is: **the shared record is a definition;
the private record is a relationship.** Definitions are shareable by nature; relationships are not.

## Evidence: what Wikidata actually covers

Probed 2026-10-29 with `wbsearchentities` (limit 3, `type=item`, 1.1 s spacing) against the local
corpus. The sample set is the 30 highest-frequency `(name, type)` pairs, plus 40 randomly chosen
mention rows (37 distinct names). One pair was probed with a disambiguating suffix
(`Train Dreams (book)`) and correctly returned nothing — parenthetical disambiguation breaks label
matching, which is itself worth remembering. Results are indicative, not a benchmark; this is one
user's taste.

| Set | n | transient errors | no hit | exact-label hit |
| --- | --- | --- | --- | --- |
| Top 30 `(name, type)` pairs by mentions | 30 | 7 | 2 | 18 (60%) |
| Random 40 mentions (37 distinct) | 37 | 6 | 7 | 20 (54%) |
| Mentions weighted (top 30 pairs) | 184 | — | — | 113 (61%, lower bound) |

Readings, and they matter more than the headline rate:

- **~60% of distinct entities resolve, and they are the ones that carry the duplication.** Enough for
  the shared layer to be worth building; not enough to replace the private fuzzy pipeline.
- **An exact label match is not a correct match.** In the random sample, at least five of the twenty
  "exact" hits point at the wrong referent: `Sunset Boulevard` → the street (Q860442), not the film;
  `The Great Game` → a *Sherlock* episode (Q1308120); `Crime and Punishment` → a 1984 video game
  (Q5185252); `Bobbie` → a given name (Q888407); `Bacardi` → a family name (Q99980734). Top-30 adds
  `Ditto` → a family name (Q37255369), not the Pokémon. **Candidate validation is mandatory**; label
  equality alone is roughly 75% precise at best in this sample.
- **Label equality is also too strict.** `China` → Q148 *People's Republic of China*,
  `Napoleon Bonaparte` → Q517 *Napoleon*, `Federal Reserve` → Q53536 *Federal Reserve System*. These
  are right answers with different labels, so scoring must consider aliases and the description, not
  just exact label equality.
- **The long tail is genuinely long.** `Wojak`, `Claude Code`, `Journal of Biophotonics`,
  `Invesco QQQ` and `Epitalon` are all in Wikidata; `Voices of WW2`, `The Graph Maker`,
  `Blake Burge`, `Max Dembo`, `cloudflared` and `MUME` are not. Private-only nodes are a permanent
  population, not a transitional one — see phase P-d for the opt-in path to sharing the ones that are
  nonetheless real public things.
- **The API pushes back.** Seven of thirty requests returned `cirrussearch-too-busy-error` at ~1
  request/second. The design must queue, retry with backoff, and degrade to private-only resolution;
  it must not sit inline in an upload request.

**Full-corpus audit.** The 70-name probe above was extended into a stratified assessment of all 2,518
local mentions — method, numbers, and per-bucket gates in
[knowledge-promotion-audit.md](knowledge-promotion-audit.md). Headlines: 84.6% of rows are candidates
once a platform link counts as an identifier, 12.1% need disambiguation, 2.5% collide across types,
and 92% of sampled handles resolve to live accounts. The gate below is sized from that data rather
than guessed.

## Schema

```sql
public_entities                       -- shared definitions; publishable by construction
  id                  i32 pk
  wikidata_qid        text null unique   -- null for promoted-from-source rows
  label               text not null
  description         text null          -- universal, generated; see "Descriptions"
  description_source  text null          -- wikidata | wikipedia | llm_grounded
  description_model   text null
  described_at        timestamptz null
  entity_type         text not null      -- mapped from P31; 'unknown' default
  image_url           text null          -- shared asset, fetched lazily
  platform            text null          -- the identifier for account-backed entities
  platform_handle     text null
  platform_url        text null          -- canonical profile URL, the citable source
  promotion_method    text not null      -- wikidata | platform_link | public_source | manual
  promotion_sources   jsonb null         -- URLs/citations the row is reproducible from
  promoted_at         timestamptz null
  merged_into_id      i32 null -> public_entities.id   -- QID redirects and base merges
  enriched_at         timestamptz null
  created_at, updated_at

  -- a handle is an identity key, like a QID
  unique (platform, lower(platform_handle)) where platform_handle is not null

public_entity_aliases                 -- public surfaces only
  id, public_entity_id -> public_entities.id
  surface, normalized, lang
  source      text                      -- label | alias | sitelink
  unique (public_entity_id, normalized, surface, lang)
  index  (normalized, lang)             -- the "second user costs nothing" path

public_entity_merges                  -- audit for QID redirects and base merges
  id, child_id, parent_id, method ('wikidata_redirect' | 'resolved' | 'manual'),
  created_at, reverted_at
  unique (child_id) where reverted_at is null
```

`promotion_method` + `promotion_sources` are the audit surface: a base row must be
**reproducible from its sources alone**, with no knowledge of which user triggered it. If it cannot be
regenerated that way, it should not exist.

Private side, one column plus one invariant:

```sql
knowledge_nodes
  + public_entity_qid text null

  -- at most one live root node per (user, public entity)
  unique (user_id, public_entity_qid)
    where merged_into_id is null and public_entity_qid is not null
  index (user_id, public_entity_qid)
```

That partial index is the load-bearing constraint: "a user has one node per public entity" becomes a
database invariant rather than a convention. Linking a second node to an already-claimed QID *must*
merge it into the existing root, so the merge machinery cannot be skipped by a code path that forgets.

**The private row stores the QID string, not `public_entities.id`.** Three reasons:

1. The layers decouple. The public table can be rebuilt, truncated, or re-ingested without touching a
   private row, and without a cascade that could delete a user's identity.
2. A QID that disappears from Wikidata leaves a dangling reference, not a broken graph: the read path
   finds no public row and renders the private node unenriched.
3. It makes the layer boundary legible in the schema: private rows reference a public identifier,
   never a row of ours.

The cost is a text join instead of an integer one, which at ~2k nodes per user is irrelevant.

## Descriptions

Generation is a first-class task, not a prompt smuggled into the enrichment fetch.

- **Inputs:** Wikidata claims (P31, P106, P569/P570, P27, …), the Wikipedia lead section, and for
  P-d entities a small number of grounded web results. Never a mention, a capture, or a private
  description.
- **Prompt contract:** entity-centric, timeless, no assumed context: describe what this is for
  someone who has never heard of it, in one or two sentences, and name the specific claim you are
  relying on.
- **Provenance:** `description_source` and `description_model` are stored. A sourced (Wikidata /
  Wikipedia) description outranks a generated one everywhere it exists; generation fills gaps and
  rewrites awkward sourced text, it never invents facts.
- **Refresh:** regenerate when the model changes or the entity's claims change materially; record
  `described_at`. Cheap, because it is once per public entity for all users.
- **Verification:** sample-audit accuracy the same way the resolve thresholds are audited; a
  wrong universal description is visible to every user, which is a different failure class from a
  wrong private one.

The private description needs no such machinery: it is produced per illumination, stored on the
mention and the node, and displayed as context. It is never promoted.

## Ingestion: private graph first, then promotion

The flow is: **illumination writes the private graph; canonicalization is a second, queued stage that
decides whether the entity also exists in the base graph.**

```
upload
  │
  ▼
illuminate ────────────► mentions + private nodes          (fast, no network, UI is live)
  │
  ▼
entity_resolve (one task per illumination)
  │
  ├─ 1. normalize; look up public aliases                 (shared index, no network)
  │      hit  → validate → LINK (promotion-merge)
  │
  ├─ 2. Wikidata lookup                                   (queued, cached, backoff)
  │      hit  → validate → LINK, minting the base row on first use
  │
  ├─ 3. no public identity, but a citable source exists   → PROMOTE (mint a base row)
  │
  └─ 4. otherwise                                         → PRIVATE ONLY
```

Four properties this ordering buys, and they are the reason to do it this way:

1. **The upload never waits on Wikidata.** The private graph is complete and correct before
   canonicalization starts; a failure there costs enrichment, not the user's data.
2. **One task per illumination, not per entity.** The sibling entities are the best disambiguation
   context we have: `Ditto` next to `Pokémon` and `Game Freak` is the Pokémon, alone it is a mood.
   Context may *propose and rank* candidates; it never auto-accepts one.
3. **Cheap paths run first.** The alias lookup is free and covers the common case; the tail pays for
   verification. New entities are rare once the base graph is warm, so cost decays with usage.
4. **The private layer keeps its own merge forest.** Linking sets one column on the private node. The
   two graphs are never fused: the user's label, description, aliases, and mentions stay private.

### Promotion versus linking

`promotion-merge` (link) attaches the private node to a base row that already exists. `promotion`
mints a new base row. The asymmetry that matters: **a base row becomes a link target for every
other user.** A wrong private merge is one user's nuisance; a wrong promotion is an identity the
system will impose on everyone. That justifies a stricter bar than private merging:

| Outcome | Requires | Result |
| --- | --- | --- |
| **link** | an identifier already claimed by a base row (QID, or platform handle) plus validation | attach to the existing row; deterministic |
| **promote** | a citable identifier: a QID, a **platform link**, or a source-grounded public reference; type-compatible; content reproducible from that source | one new base row, `promotion_sources` recorded |
| **private only** | nothing | the default for the tail |

A platform link is a first-class identifier, not a lesser one. It is public, citable, and stable
enough to key on: 92% of sampled handles resolved to live accounts, and 375 rows in the local corpus
collapse to 340 distinct accounts — including pairs the name alone would never merge (`cremieuxrecueil`
appears as both a `real_person` and a `social_media_account`).

**Promotion is an identity claim, not a content copy.** The trigger may be a private mention; the
row's label, description, type, and aliases must be reproducible from its cited public sources alone.
That is the operational form of the publishability test, and it is checkable: regenerate the row from
`promotion_sources` and compare.

**No tentative base rows.** There is no "draft public entity visible only to the user who promoted
it". That state would require recording the triggering user on a shared row, which is exactly what the
base layer must not do. If an entity cannot be corroborated publicly, it stays private (optionally
flagged `promotion_candidate` on the private node, which is user-scoped data).

**Person-shaped entities need an identifier, not notoriety.** A person backed by a platform link is
promotable: the handle is a public identifier and the profile URL is the source. What is *not*
promotable is a bare name with no handle and no other source — and in the local corpus that is one
row out of 2,518, because the extractor attaches a handle to essentially every person it pulls from a
post.

Two rules keep this honest:

- **The record is about the account, not the person.** "This account posts book reviews" is a
  statement the account makes about itself. A biography ("a 34-year-old teacher in Ohio") is not, and
  must never be written from a screenshot. The audit found zero biographical claims across 166
  accounts; that is a property to preserve, not to assume.
- **Attribution is decided by the record's type, not by confidence.** The handle is reliable; what
  varies is what it identifies. The name kind is inferable with no schema change
  (`mash(name) == mash(handle)` → the name is a handle), and the decision follows in order: (a) the
  display name corroborates the entity name → promote under the person's name; (b) the description
  names the persona ("known online as Tsoding") → promote under the persona; (c) the entity name *is*
  the handle (`dennisbhooper`, `SlopHq`) → promote as the account, never as a `real_person`;
  (d) otherwise → promote as the account. Local data: 0 handles shared by two people, 0 malformed
  handles, 168 of 184 person links corroborated by display name, and 1 handle-named row typed as a
  person — a stylisation, not a misattribution.

Volume of mentions is still not verification, and a frequently matched base row is not thereby
confirmed. A handle that no longer resolves is **not** an error: the corpus contains throwback posts,
and 5 of 60 checked handles had already been renamed or deleted. The URL is evidence, not immutable
identity — keep the description and provenance, re-check slowly, and never delete a base row because
its URL died. A *malformed* handle is the real signal, and there are none (0 of 375 break their
platform's charset or length rules).

### Candidate validation

A `wbsearchentities` hit becomes a link only when:

- the P31 class maps to a type compatible with the mention's type;
- the description is consistent with the mention's description (token overlap, or an LLM yes/no in
  the ambiguous band);
- the match is not one of the known-wrong shapes (`family name`, `given name`, `human name`,
  `Wikimedia disambiguation page`).

Otherwise the hit becomes a candidate on the private node — visible, reversible, not applied. The five
wrong referents measured above are exactly what this filter exists for.

**Cache rule.** `public_entity_aliases` may contain only surfaces that came from public data —
labels, aliases, sitelinks. A user's phrasing is cached on the user's own `knowledge_aliases` row.
This is the publishability test applied to an index, and it costs almost nothing: the common case is
that a user wrote the public label, which is exactly the row that already exists.

### Two entity-resolution problems, not one

Minting a base row raises the same question the private layer answers: is this the same thing as an
existing base row? So there are two ER problems — private identity and base identity — sharing the
normalization, scoring, and forest-merge code, but with different evidence and much higher precision
requirements at the base layer. Do not assume one solves the other, and do not let private merges
vote on base identity.

### Promotion is a broadcast

Linking is a point lookup; promotion fans out. When a base row is minted, every user's private nodes
with a matching normalized name and type should be *offered* the link. That runs as a background
re-match, not an inline write, and it is the real cost of promotion. It also means the base graph
converges gradually rather than atomically: `MUME` promoted today links yesterday's captures
tomorrow.

## Enrichment

Lazy, not a Wikidata mirror:

1. A mention links to a QID with no `public_entities` row → insert a stub from the search result.
2. `public_entity_enrich` fetches `wbgetentities` (labels, descriptions, aliases, P31, P18 image,
   sitelinks), maps the class to our type, stores the image in shared storage.
3. `public_entity_describe` generates the universal description from the sourced material.
4. `enriched_at` / `described_at` drive refresh (months, not minutes; reference data changes slowly).

Cost is per public entity, once, for every user forever. The row carries no `user_id`, so the
enrichment pipeline cannot accidentally depend on one.

Images are the one place where "shared" means bytes and money: cache per QID, serve from one path for
everyone, never re-fetch per user.

## Consistency across layers

Three separate corrections, with different semantics — do not collapse them:

| Event | What changes | Reversible by |
| --- | --- | --- |
| **Private merge** (Bob's "Elon" node into Bob's "Elon Musk" node) | `knowledge_nodes.merged_into_id` in Bob's graph | clearing one pointer |
| **Unlink** (the QID was wrong, or should not apply) | `knowledge_nodes.public_entity_qid = null` | re-linking |
| **Public merge** (Wikidata redirects Q_old → Q_new) | `public_entities.merged_into_id`; every user node follows at read time | clearing one pointer; no user rows touched |

Unmerge and unlink interact, and the partial unique index makes the interaction explicit: if a child
and its parent both carry the *same* `public_entity_qid`, unmerging would violate "one root per QID",
and it is also meaningless — a QID link is deterministic identity. So:

- `unmerge(child)` requires the two sides to have different (or null) public identities; otherwise
  the operation is refused with "these are the same public entity".
- `unlink(node)` is the operation for "that QID is wrong"; it clears the link and leaves the private
  cluster exactly as it was.

A deleted or split QID is handled as `unlink`, not as a data repair.

## Invariants

Two groups. The first is about the shared record; the second is about the association.

**Publishability of the shared record**

- **S1.** Every column of `public_entities` (and `public_entity_aliases`) is safe to show to any
  user, knowing nothing about who captured anything.
- **S2. A private action may *trigger* a base row; it may never *populate* one.** Labels, aliases,
  descriptions, types, and sources come from public material only. The test: regenerate the row from
  `promotion_sources` alone and it should match, without reference to any user.
- **S2a.** Promotion of `real_person` and `social_media_account` requires an identifier: a platform
  handle (with display-name or role corroboration when promoting under the person's name) or an
  external public source. Volume of mentions is never evidence, and a frequently matched base row is
  not thereby confirmed.
- **S3.** Deleting a user changes no `public_*` row. A shared entity that only one user ever linked
  survives their deletion intact.

**Association**

- **A1.** No table has a foreign key joining rows of two different users. `public_*` tables have no
  `user_id`.
- **A2.** No response to any user reveals another user's data: no counts, no ids, no "also captured
  by", no cross-user co-occurrence.
- **A3.** A public entity page renders the shared definition only. It does not reveal that anyone
  captured it, or how many.
- **A4.** The link is private: `knowledge_nodes.public_entity_qid` is user-scoped data.
- **A5.** Aggregate public-entity statistics (if ever built) are a separate, explicit product
  decision with k-anonymity — not a side effect of this design.

S2 is the invariant most at risk: the fastest way to write a good universal description is to read
someone's private description, and the resulting record would still *look* publishable. Worth a test
that asserts the describe task reads no private tables.

## What this does not solve

- **Entities without public evidence stay private.** Promotion needs a citable source; a name that
  only exists inside a screenshot does not get a base row. `Wojak` and `Claude Code` are in Wikidata;
  `The Graph Maker` is not, and will stay private unless something public can be cited.
- **No cross-user recommendations** — "people who captured X also captured Y" is aggregate behaviour,
  not an entity fact.
- **Not a replacement for the private pipeline.** ~40% of entities have no QID (more once wrong
  referents are excluded), and linked ones still need private aliases, descriptions, and merges.
- **Promotion is not free and not instant.** Minting a base row costs a lookup, a verification, and a
  background re-match across users; the base graph converges gradually.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| **Per-user entities and per-user enrichment** | Correct and private by construction, but every user pays to describe Elon Musk, and the definition drifts between users. This is phase 1; the shared layer is the upgrade. |
| **Build the shared record from the union of user mentions** | Produces a record that is no longer self-contained: its content depends on who captured what, so it fails the publishability test and gets worse as the user base changes. Rejected. |
| **Shared identity but no shared description** | Half the win: identity converges, but everyone still generates their own definition, which is the expensive and inconsistent part. |
| **Mirror Wikidata in bulk** | Hundreds of millions of items for a corpus that touches a few thousand. Lazy per-QID enrichment keeps storage and cost proportional to usage. |
| **Separate global service / graph database** | A second source of truth, a second deployment, and a network hop for data that fits in a few Postgres tables. Revisit only if enrichment volume exceeds the main database. |
| **One description, shared, overridden per user** | An override model forces a loser: either the universal definition is hidden or the user's context is. Two descriptions answer two different questions; keep both. |
| **Promote on mention, without a citable source** | Maximum coverage and the shortest flow, but it turns the base graph into an unverifiable projection of user traffic: private individuals, model misreadings, and one-off strings all become link targets imposed on everyone. It also makes the base graph impossible to audit or rebuild. Rejected; the private tail is the price of a defensible dictionary. |
| **Let the promoting user's description seed the base description** | The fastest way to good coverage, and the one that quietly breaks S2: the row would look publishable while depending on private context. Rejected. |

## Phases

| Phase | Work | Acceptance |
| --- | --- | --- |
| **P-a — Link** | `public_entities`, aliases, merges; nullable `public_entity_qid`; shared alias lookup; queued lookup with backoff; candidate validation (P31 + description + known-wrong shapes); unlink. | Link precision ≥ 0.9 on 50 labelled mentions; the five known-wrong referents above are rejected; a second user's `Elon Musk` costs zero network calls. |
| **P-b — Enrich** | `wbgetentities` fetch, P31→type mapping, labels/aliases, shared image cache, `enriched_at` refresh. | One fetch per QID for all users; node page renders the shared definition; coverage reported per user. |
| **P-c — Describe** | Universal description generation from sourced material, provenance, refresh, sample audit. | 20 sampled descriptions are accurate, context-free, and identical for all users; a test asserts the describe path reads no private tables. |
| **P-d — Promote** | Promotion for source-backed entities: platform-link identifier (handle + URL + display name), the attribution type decision, grounded source retrieval for the rest, `promotion_sources` provenance, base-layer candidate/merge, background re-match of existing private nodes. | Every promoted row is reproducible from its sources; **zero** rows whose name is a handle promoted as `real_person` (the audit's 32 account rows are the regression set); promotion precision ≥ 0.95 on a 40-entity audit; the re-match links prior captures without touching user-visible state. |

## Open questions

1. **Where is the promotion bar, exactly?** QID-backed promotion is easy to defend. Source-grounded
   promotion without an external identifier is the interesting case: which sources count (Wikipedia,
   an official site, two independent mentions?) and how do we reject a private individual whose name
   appears in a screenshot? Start strict, measure the promotion rate, loosen with evidence.
2. **Type mapping ownership.** P31 → `KnowledgeEntityType` is a curated table; who maintains it, and
   how does it interact with the taxonomy pass in
   [knowledge-entity-types.md](knowledge-entity-types.md)? It is also the best available evidence
   for the `unknown` problem.
3. **Display precedence.** Proposed: the user's label wins for the title; the universal description
   and the private context are shown as two labelled blocks, never merged into one paragraph.
4. **Rate-limit budget.** Wikidata asks for polite usage and there is no API key. A single global
   concurrency cap plus backoff is the minimum; do we also want a `maxlag` budget alarm?
