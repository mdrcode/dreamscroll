# Knowledge entities — private canonicalization and the shared base

**Status:** Design. Private phases 1–4 are startable; shared phases P-a…P-d are deferred behind them.
Nothing implemented.

**See also:** [knowledge-entity-types.md](knowledge-entity-types.md) (taxonomy; settled by the
2026-10-29 audit), [knowledge-promotion-audit.md](knowledge-promotion-audit.md) (the measured
evidence this design is sized from), [knowledge-wikidata-recommender.md](knowledge-wikidata-recommender.md)
(QID-keyed recommender research), [pragmatism.md](pragmatism.md) (ledger),
[backfill.md](backfill.md) (typed backfills), `plan/archive/illuminate-v2.md` (the projection this
builds on).

## The gap

Illumination v2 gives us **mentions**, not **entities**. Every `knowledge_entities` row is one span of
text a model reported for one capture: "Train Dreams", `movie`, in this capture. Nothing says the
fourteen `Train Dreams` mentions are one work, nothing says which of them is the same work another
user captured, and nothing lets us say how close two of them are.

Four layers, only two of which exist:

```
   capture ───────────────────────────────────────────────────────┐
     │ media, url, annotation                                     │  the artifact (private)
     ▼                                                            │
   mention ("this capture said X")        knowledge_entities      │  exists today (private)
     │ name, type, description, platform_link                     │
     ▼                                                            │
   node    ("X, as this user knows it")   knowledge_nodes         │  private layer, phase 1
     │ label, type, aliases, embedding, optional public link      │
     ▼                                                            │
   public entity ("X, for everyone")      public_entities         │  shared layer, P-a…
        label, type, universal description, image, QID or handle  │
```

Three properties drive the private layer:

1. **Identity is an assertion, not a rewrite.** Merging must not move or destroy mentions, so
   reverting a merge is one pointer write, not a data reconstruction.
2. **Precision beats recall.** A missed merge is a duplicate row. A wrong merge silently fuses two
   people, and through transitivity can swallow a neighbourhood.
   ([ER in Practice, 2026](https://arxiv.org/abs/2607.26298) calls this out as its third lesson.)
3. **Similarity is computed, not declared.** Store evidence (co-capture counts, embeddings, shared
   handles); derive the score in one pure function so it can be re-tuned without touching data.

One property drives the shared layer:

4. **The entity is public; the association is private.** *Elon Musk*, *Game of Thrones*, *C++* are
   public factual referents; a record saying "this is Elon Musk, here is a definition, a link, an
   image" carries no user information and should be shared, enriched once, and shown to anyone. What
   is private is that **Bob captured him**, in **which screenshot**, with **what description**, next
   to **which other entities**. That is the edge, not the node.

### The publishability test

> Every column of a shared entity record must be safe to show to **any** user, knowing **nothing**
> about who captured it. If a value fails that test, it is not a shared attribute.

This is better than "public sources only" because it says what the record *is* rather than where it
came from, and it answers the cases immediately:

| Value | Publishable? | Where it lives |
| --- | --- | --- |
| Canonical label, aliases, universal description, image, QID, type | yes | shared |
| `"the documentary about him that I saved"` | no — meaningless without the capture | private |
| The user's spelling `"the Elon guy"` | no — it is the user's phrasing | private |
| Co-capture edges, related-entity ranking | no — derived from one person's activity | private |
| "captured by 12 users" | publishable in isolation, but aggregate behaviour | separate decision, A5 |

## What the current data says

Measured read-only against the local dev database (`dreamscroll_dev1`, 2026-10-29). Method,
per-bucket tables, and scripts: [knowledge-promotion-audit.md](knowledge-promotion-audit.md). The
audit is the home for detail; this document keeps the numbers that justify a decision.

| Fact | Value |
| --- | --- |
| `knowledge_entities` rows | 2,518 (**2,369 current**; 149 superseded by re-illumination) |
| captures / users with entities | 710 / 3 (one holds 2,486 rows) |
| distinct `(user, type, normalized name)` keys | 2,134 |
| excess mentions under exact normalized name + type | 384 — 293 across captures, 91 from re-illumination |
| captures with ≥ 2 mentions | 579 (avg 4.02, max 18) |
| co-occurrence pairs within a capture | 4,515 |
| mentions with a description | 2,474 (92.7% are full definitions) |
| Wikidata resolvable | ~60% of distinct names; 84.6% of rows are promotion candidates once a platform link counts |
| platform links | 375 rows, 340 distinct accounts, 0 malformed, 92% of sampled handles live |
| person links corroborated by display name | 168 of 184 |

Readings that shape the design:

- **~15% of mentions are already exact duplicates under conservative normalization**, and most of the
  excess is cross-capture — real canonicalization wins, not noise.
- **The graph is small.** ~2k nodes per user and 4.5k co-capture pairs. Any design needing a service
  to hold vectors at this scale is premature.
- **Normalization must be conservative.** Stripping punctuation collapses `C`, `C#`, and `C++` into
  one key (observed, three variants); `@handle` vs `handle` is a genuine variant worth folding. The
  fix is type-aware rules plus a symbol veto, not a greedier key.
- **Wikidata covers the head, not the tail.** `Wojak`, `Claude Code` and `Invesco QQQ` resolve;
  `The Graph Maker`, `Max Dembo` and `MUME` do not. Private-only nodes are a permanent population.
- **An exact label match is not a correct match.** Measured wrong referents: `Sunset Boulevard` → a
  street, `The Great Game` → a *Sherlock* episode, `Crime and Punishment` → a 1984 video game,
  `Bobbie` → a given name, `Bacardi` → a family name, `Ditto` → a family name. Label equality alone
  is roughly 75% precise; candidate validation is mandatory.
- **Label equality is also too strict.** `China` → *People's Republic of China*, `Napoleon Bonaparte`
  → *Napoleon*, `Federal Reserve` → *Federal Reserve System* — right answers, different labels.

## Decisions

Private layer:

1. **Canonicalize per user, not globally.** Not because sharing a referent is unsafe — public entities
   are public — but because *interpretation* is context-dependent: the same string is a Pokémon in
   one graph and a mood in another. Nodes carry `user_id`; the public referent is a separate layer
   nodes may link to.
2. **Every mention gets exactly one node at write time.** Merging is then purely node→node, and no
   merge ever edits a mention row. This is what makes reversibility cheap.
3. **Merges are forward pointers in a forest, not destructive folds.**
   `knowledge_nodes.merged_into_id` is the authoritative edge; `knowledge_merges` is the audit log.
   Unmerge = clear the pointer.
4. **Automatic merges attach a leaf to a root.** Cluster-into-cluster merges require stronger evidence
   (LLM adjudication with both member lists, or a user action). The damage from one bad automatic
   merge is bounded to one node.
5. **Reads resolve to the root lazily; we do not path-compress.** A few pointer hops beat a
   denormalized root column that can drift and destroy per-edge reversibility.
6. **Aliases are never copied during a merge.** Alias lookup considers the whole subtree, so a merge
   writes nothing but the pointer. The display label is derived, not mutated.
7. **The node is what gets embedded, and vectors live in Postgres.** Cosine over one user's ~2k nodes
   is sub-millisecond; a second vector index with its own staleness story is not.
8. **Similarity is a ranking, not a threshold.** Raw Gemini cosine has a high, model-dependent
   baseline. Calibrate within a user before combining with structural signals.
9. **Names decide automatic merges; embeddings break ties.** Embeddings conflate related entities
   (Jobs ≈ Wozniak), so they are the weakest of the strong identity signals.

Shared base:

10. **The shared record is reference data, not a union of user graphs.** It is fed only from public
    sources. A user action may *trigger* a row; it may never *populate* one. Otherwise the record
    stops being self-contained and gets worse as the user base changes.
11. **Two descriptions, one per layer.** The universal description is generated once from public
    sources and is identical for everyone; the private one is the model's reading of that capture.
    They answer different questions and neither overrides the other.
12. **User text is never an input to a universal description.** Not summarized, not averaged, not
    "used for coverage" — the private description is a dead end.
13. **An identifier is required to promote.** A QID, a **platform link**, or a source-grounded public
    reference. A bare name inside a screenshot is not enough.
14. **No tentative base rows.** There is no draft public entity visible only to its promoter: that
    state would require recording the triggering user on a shared row. Uncorroborated entities stay
    private.
15. **Wikidata QIDs are an optional link, not the identity key.** The
    [recommender doc](knowledge-wikidata-recommender.md) keys every node by QID; that is right long
    term and premature as the primary key (ambiguous matches, network dependency, taxonomy still
    moving). A QID is a strong merge signal and the key for enrichment fetched once rather than per
    user.

## Schema

Seven new tables (four private, three shared), one nullable column on the mention table, and one
partial index that carries an invariant. Bump `REQUIRED_SCHEMA_VERSION` (currently 8).

```sql
-- Private layer: every row carries user_id.
knowledge_nodes
  id, user_id
  name             text    -- the label this node was created with; merges never mutate it
  entity_type      text
  description      text    -- best description seen; a merge may only fill a null
  merged_into_id   i32 null -> knowledge_nodes.id     -- union-find parent
  embedding        real[] null
  embedding_hash   text null                          -- sha256 of the embedded text
  public_entity_qid text null                         -- link to the shared layer (QID string)
  created_at, updated_at

  -- at most one live root node per (user, public entity)
  unique (user_id, public_entity_qid)
    where merged_into_id is null and public_entity_qid is not null
  index (user_id, public_entity_qid)

knowledge_aliases
  id, user_id, node_id -> knowledge_nodes.id
  surface          text    -- exactly what the model wrote
  normalized       text    -- conservative canonical key
  from_mention     i32 null -> knowledge_entities.id
  created_at
  unique (node_id, normalized, surface)
  index  (user_id, normalized)          -- blocking lookup

knowledge_merges
  id, user_id
  child_id   i32 -> knowledge_nodes.id  -- the node that now forwards
  parent_id  i32 -> knowledge_nodes.id  -- the node it forwards to
  method     text   -- exact_key | token_set | embedding | llm | user | external | backfill
  score      real null
  evidence   jsonb null
  created_at, reverted_at timestamptz null
  check (child_id <> parent_id)
  unique (child_id) where reverted_at is null
  index  (parent_id, reverted_at)

knowledge_candidates                   -- the ambiguous band, deferred decisions
  id, user_id
  node_a_id i32, node_b_id i32         -- always a < b, one row per unordered pair
  score     real
  features  jsonb                      -- name_sim, type_match, cosine, method
  status    text                       -- pending | accepted | rejected
  created_at, decided_at
  unique (node_a_id, node_b_id)

-- Shared layer: no user_id anywhere.
public_entities
  id
  wikidata_qid        text null unique        -- null for promoted-from-source rows
  label               text not null
  description         text null               -- universal, generated
  description_source  text null               -- wikidata | wikipedia | llm_grounded
  description_model   text null
  described_at        timestamptz null
  entity_type         text not null           -- mapped from P31; 'unknown' default
  image_url           text null               -- shared asset, fetched lazily
  platform            text null               -- the identifier for account-backed entities
  platform_handle     text null
  platform_url        text null               -- canonical profile URL, the citable source
  promotion_method    text not null           -- wikidata | platform_link | public_source | manual
  promotion_sources   jsonb null              -- URLs/citations the row is reproducible from
  promoted_at         timestamptz null
  merged_into_id      i32 null -> public_entities.id   -- QID redirects and base merges
  enriched_at         timestamptz null
  created_at, updated_at

  unique (platform, lower(platform_handle)) where platform_handle is not null

public_entity_aliases                  -- public surfaces only
  id, public_entity_id -> public_entities.id
  surface, normalized, lang
  source      text                     -- label | alias | sitelink
  unique (public_entity_id, normalized, surface, lang)
  index  (normalized, lang)            -- the "second user costs nothing" path

public_entity_merges                   -- audit for QID redirects and base merges
  id, child_id, parent_id
  method     text                      -- wikidata_redirect | resolved | manual
  created_at, reverted_at
  unique (child_id) where reverted_at is null
```

Plus `knowledge_entities.node_id i32 not null -> knowledge_nodes.id` (added nullable, backfilled,
then tightened) with indexes on `(user_id, node_id)` and `(capture_id, node_id)`.

**Why the private row stores the QID string rather than `public_entities.id`:**

1. The layers decouple. The public table can be rebuilt, truncated, or re-ingested without touching a
   private row, and without a cascade that could delete a user's identity.
2. A QID that disappears from Wikidata leaves a dangling reference, not a broken graph: the read path
   finds no public row and renders the private node unenriched.
3. The boundary is legible in the schema: private rows reference a public identifier, never a row of
   ours.

The cost is a text join instead of an integer one, which at ~2k nodes per user is irrelevant.

**The partial index is load-bearing.** "A user has one node per public entity" becomes a database
invariant rather than a convention: linking a second node to an already-claimed QID *must* merge it
into the existing root, so the merge machinery cannot be skipped by a path that forgets.

**No edge table yet.** Co-capture connectedness is a join over `knowledge_entities`. Add
`knowledge_edges` when we have non-derived edges — a persona relation, an LLM-asserted relation, a
user-drawn link. That is the trigger, not "we want a graph": the derived graph is already available.

Model files: `src/model/knowledge_node.rs`, `knowledge_alias.rs`, `knowledge_merge.rs`,
`knowledge_candidate.rs`, `public_entity.rs`, `public_entity_alias.rs`, `public_entity_merge.rs`;
logic in `src/knowledge/` (`normalize.rs`, `resolve.rs`, `merge.rs`, `similarity.rs`,
`promotion.rs`) with a lean root.

## Merge mechanics

A forest, one outgoing edge per node, roots have none.

```
resolve(n):  while n.merged_into_id { n = merged_into_id }; return n

merge(child, parent):
    assert resolve(child) == child          -- only roots are attached
    assert resolve(parent) != child         -- no cycles
    child.merged_into_id = resolve(parent)
    log(child, parent, method, score, evidence)     -- same tx

unmerge(child):
    assert child.merged_into_id is not null
    child.merged_into_id = null
    log.reverted_at = now()                          -- same tx
```

- **Merging never touches mentions or aliases.** After `child → parent`, the cluster is
  `{n : resolve(n) == parent}`; every mention and alias still points at its original node and is
  reachable because reads resolve the node first.
- **New mentions attach to the most specific node, never the flattened root.** If the best candidate
  is itself forwarding (`M → R`), the mention points at `M`. That keeps the finer distinction
  available for a later unmerge; flattening on write would make every unmerge restore a label rather
  than a partition.
- **Unmerge is exact.** Cutting `child → parent` re-roots the child's whole subtree and nothing else.
  This is why only roots are attached: the edge you cut corresponds to the decision you made.
- **The only field a merge may write outside the pointer is `description`**, and only to fill a null
  on the parent. Labels, aliases and types stay on their own nodes; display is derived at read time.
- **Depth stays ~1** for automatic merges (leaf into root). Cluster merges add a level. Depth above
  ~8 is a signal to stop and re-verify, not to compress.
- **No deletion.** A node with zero mentions is invisible to reads but retained, because the merge log
  may still reference it.
- **Archived captures** follow today's read filters: their mentions stay linked and still feed
  alias/co-capture evidence, but are excluded from user-facing lists.

Reads use one recursive CTE, so callers never loop:

```sql
-- cluster of any node: walk up to the root, then collect everything forwarding to it
WITH RECURSIVE up AS (
  SELECT id, merged_into_id FROM knowledge_nodes WHERE id = $1 AND user_id = $2
  UNION ALL
  SELECT n.id, n.merged_into_id
  FROM knowledge_nodes n JOIN up u ON n.id = u.merged_into_id
), root AS (
  SELECT id FROM up WHERE merged_into_id IS NULL
), down AS (
  SELECT id FROM root
  UNION ALL
  SELECT n.id FROM knowledge_nodes n JOIN down d ON n.merged_into_id = d.id
)
SELECT id FROM down;
```

Upward answers "which canonical entity is this"; downward answers "which nodes belong to it". The
upward walk is bounded by the forest depth, which is why depth matters.

### Unmerge, unlink, and public merges are three different operations

| Event | What changes | Reversible by |
| --- | --- | --- |
| **Private merge** (Bob's "Elon" node into Bob's "Elon Musk" node) | `knowledge_nodes.merged_into_id` in Bob's graph | clearing one pointer |
| **Unlink** (the QID was wrong, or should not apply) | `knowledge_nodes.public_entity_qid = null` | re-linking |
| **Public merge** (Wikidata redirects Q_old → Q_new) | `public_entities.merged_into_id`; every user node follows at read time | clearing one pointer; no user rows touched |

The partial unique index makes the interaction explicit: if a child and its parent carry the *same*
`public_entity_qid`, unmerging would violate "one root per QID" — and it is meaningless, because a QID
link is deterministic identity. So `unmerge(child)` requires different (or null) public identities and
is otherwise refused with "these are the same public entity"; `unlink(node)` is the operation for
"that QID is wrong". A deleted or split QID is handled as `unlink`, never as a data repair.

### The write path has to stop churning rows

`insert_illumination_v2` currently deletes all `knowledge_entities` for an illumination and re-inserts
them. On re-illumination that would silently discard every node assignment. Change it to upsert by
the existing unique key `(illumination_id, position)`:

- position present in both runs → same mention row, keep the node unless `(normalized name, type)`
  changed;
- new positions → insert, resolve;
- vanished positions → delete the mention, keep the node.

A mention whose normalized key or type changed is re-assigned; everything else is left alone.

**A capture can have several illuminations, and only the latest is live.** Illuminations are
append-only: 34 of 628 illuminated local captures carry two or three, and the read path already
exposes only `max(id)`. The earlier runs' mention rows are still in the table (149 of 2,518), so
anything that resolves or counts "all mentions" double-counts a capture. Therefore:

- `entity_resolve` must scope to the latest illumination per capture, not to every mention row;
- superseded runs' mentions should be marked (or deleted) when a new illumination lands, so a later
  backfill cannot resurrect them;
- every "mentions" metric needs the same filter, or the audit's numbers drift by ~6%.

This is also the whole explanation for the apparent "duplicate inside one capture" groups: restricted
to the latest illumination per capture, **zero** groups repeat a name inside one run. The model does
not duplicate entities; re-running illumination does.

## Ingestion: private graph first, then promotion

Illumination writes the private graph. Canonicalization is a second stage that decides whether the
entity also exists in the base graph.

```
upload
  │
  ▼
illuminate ──────────► mentions + private nodes            (inline, no network, UI is live)
  │
  ▼
entity_resolve (one queued task per illumination)
  │
  ├─ private stage (only if a mention changed since the last run)
  │     normalize → block → score → direct attach | create-then-merge → new node
  │
  ├─ 1. shared alias lookup                          (no network)
  │      hit → validate → LINK
  ├─ 2. Wikidata lookup                              (queued, cached, backoff)
  │      hit → validate → LINK, minting the base row on first use
  ├─ 3. no public identity, but a citable source     → PROMOTE, minting a base row
  └─ 4. otherwise                                    → PRIVATE ONLY
```

Properties this ordering buys:

1. **The upload never waits on Wikidata.** The private graph is complete and correct before
   canonicalization starts; a failure there costs enrichment, not the user's data.
2. **One task per illumination, not per entity.** The sibling entities are the best disambiguation
   context we have: `Ditto` next to `Pokémon` and `Game Freak` is the Pokémon; alone it is a mood.
   Context may *propose and rank* candidates; it never auto-accepts one.
3. **Cheap paths run first.** The alias lookup is free and covers the common case; the tail pays for
   verification. New entities are rare once the base graph is warm, so cost decays with usage.
4. **The private layer keeps its own merge forest.** Linking sets one column. The graphs are never
   fused: the user's label, description, aliases and mentions stay private.

### Normalization

One pure function, `knowledge::normalize::key(surface) -> String`:

1. Unicode NFKC; casefold.
2. Collapse whitespace, trim.
3. Fold diacritics (`Pokémon` → `pokemon`).
4. Strip *leading* `@` / `#` and *surrounding* quotes; strip a trailing period. Nothing else.
5. Keep inner punctuation (`C#`, `C++`, `U.S.`, `Spider-Man`).
6. Token-set key for blocking: sorted tokens minus leading articles. Candidate generation only,
   never an identity key.

Rule 5 is deliberate: the observed `C / C# / C++` collision is what aggressive stripping buys. Rule 4
is deliberate: `@bandy1803` and `bandy1803` are one account.

### Candidates (blocking)

Per incoming mention, over the *same user's* nodes only:

- exact `normalized` match in `knowledge_aliases` (indexed);
- token-set key equality (in-app, ~2k nodes — trivial; revisit above ~100k nodes/user);
- same `platform` + `platform_handle` (deterministic for accounts);
- phase 2: top-k cosine against node embeddings.

### Scoring

```
score = 0.55 * name_sim
      + 0.20 * type_match
      + 0.15 * cosine            (0 when unavailable)
      + 0.10 * description_sim   (token Jaccard, 0 when absent)
```

Hard vetos apply after scoring, not as negative weights — a veto must not be tradeable:

| Veto | Rationale |
| --- | --- |
| types differ, both non-`unknown`, in the incompatibility set (`real_person`/`fictional_character`, `book`/`movie`, `organization`/`real_person`, …) | reusing a name is not the same entity |
| names differ only by a symbol, both `software`/`product`/`financial` | `C` ≠ `C#` |
| `platform` set on both and differs | different accounts |
| `platform` equal but handles differ | different accounts |

Thresholds, as constants in one module with a table test:

- **direct attach**: veto-free and `name_sim = 1.0` (identical normalized key), or same
  platform+handle. No node, no merge row.
- **auto-merge**: veto-free and `0.97 <= name_sim < 1.0` (diacritic, article, minor punctuation). New
  node, merged leaf → root.
- **candidate link**: veto-free and `0.75 <= name_sim < 0.97`, or phase 2 `cosine >= p99` of the
  user's distribution with `type_match = 1`. Stored, not merged.
- **new node**: everything else.

`name_sim` = 1 − normalized Levenshtein over `normalized`, computed in-app (no `pg_trgm` dependency).

### Direct attach vs. create-then-merge

- **Direct attach** when the mention's normalized key is already an alias of a type-compatible node.
  No node, no merge row: the same entity spelled the same way. All 14 `Train Dreams` mentions become
  one node and one alias, not a 14-node chain.
- **Create-then-merge** when the match is fuzzy. A node is created for the variant surface and
  immediately merged into the matching root, producing exactly one revertible merge-log row.

Exact-key matches still have to clear the type vetos: `Dune` the `book` and `Dune` the `movie` are
different nodes with the same alias, and `knowledge_aliases` deliberately has no global uniqueness on
`normalized`.

### Aliases are where personas live

Illumination keeps one entity per person and hangs the account off it, so a persona name (`Tsoding`,
`Meta-Nomad`, `clara`) exists only as `platform_display_name` or as prose ("known online as Tsoding").
That is enough for display and useless for matching: a later capture showing only the persona produces
a different node.

The mention should therefore carry an **`aliases` array** — persona names, pseudonyms, alternate
titles — and every alias becomes a `knowledge_aliases` row. Resolution then matches on the alias, and
"Tsoding" and "Alexey Kutepov" converge without a `persona` entity type (see
[knowledge-entity-types.md](knowledge-entity-types.md)). Because an alias is a *label*, a merge never
has to invent a relation to express it.

If a persona ever needs to be a node of its own, that is a relation (`pseudonym_of`, `operated_by`),
not a second label on the same node — and it is the trigger for the deferred edge table. The local
corpus has no capture where both appear, so it can wait.

### Cluster-level verification

Before merging two *clusters*, score every cross pair (bounded: if either side exceeds ~20 members,
fall back to root-vs-root plus a sample) and require the minimum pair score to clear the bar. This is
the concrete defence against the transitivity failure in
[ER in Practice](https://arxiv.org/abs/2607.26298): "A matches B" and "B matches C" must not imply
"A matches C" without a direct check.

### Promotion versus linking

`link` attaches the private node to a base row that already exists; `promote` mints a new one. The
asymmetry that matters: **a base row becomes a link target for every other user.** A wrong private
merge is one user's nuisance; a wrong promotion is an identity the system imposes on everyone.

| Outcome | Requires | Result |
| --- | --- | --- |
| **link** | an identifier already claimed by a base row (QID or platform handle) plus validation | attach to the existing row; deterministic |
| **promote** | a citable identifier — a QID, a **platform link**, or a source-grounded public reference — type-compatible, content reproducible from that source | one new base row, `promotion_sources` recorded |
| **private only** | nothing | the default for the tail |

**Promotion is an identity claim, not a content copy.** The trigger may be a private mention; the
row's label, description, type and aliases must be reproducible from its cited public sources alone.
That is the operational form of the publishability test, and it is checkable: regenerate the row from
`promotion_sources` and compare.

**A platform link is a first-class identifier.** It is public, citable, and stable enough to key on:
92% of sampled handles resolved to live accounts, 375 rows collapse to 340 distinct accounts, and the
handle merges pairs the name never would (`cremieuxrecueil` appears as both a `real_person` and a
`social_media_account`). An unverifiable handle is *not* an error — the corpus contains throwback
posts, and 5 of 60 checked handles had already been renamed or deleted. A malformed handle would be
the real signal, and there are none.

**Person-shaped entities need an identifier, not notoriety.** A person backed by a platform link is
promotable; a bare name with no handle and no other source is not — and in the local corpus that is
one row out of 2,518, because the extractor attaches a handle to essentially every person it pulls
from a post. Two rules keep this honest:

- **The record is about the account, not the person.** "This account posts book reviews" is a
  statement the account makes about itself; a biography ("a 34-year-old teacher in Ohio") is not, and
  must never be written from a screenshot. The audit found zero biographical claims across 166
  accounts — a property to preserve, not assume.
- **Attribution is decided by the record's type, not by confidence.** The name kind is inferable with
  no schema change (`mash(name) == mash(handle)` → the name is a handle). In order: (a) the display
  name corroborates the entity name → promote under the person's name; (b) the description names the
  persona → promote under the persona; (c) the entity name *is* the handle → promote as the account,
  never as a `real_person`; (d) otherwise → promote as the account. Local data: 0 handles shared by
  two people, 0 malformed handles, 168 of 184 person links corroborated, and 1 handle-named row typed
  as a person — a stylisation, not a misattribution.

### Candidate validation

A `wbsearchentities` hit becomes a link only when:

- the P31 class maps to a type compatible with the mention's type;
- the description is consistent with the mention's description (token containment, or an LLM yes/no
  in the ambiguous band);
- the match is not a known-wrong shape (`family name`, `given name`, `human name`,
  `Wikimedia disambiguation page`).

Otherwise the hit becomes a candidate on the private node — visible, reversible, not applied. The
wrong referents measured above are exactly what this filter exists for.

**Cache rule.** `public_entity_aliases` may contain only surfaces that came from public data —
labels, aliases, sitelinks. A user's phrasing is cached on the user's own `knowledge_aliases` row.
This is the publishability test applied to an index, and it costs almost nothing: the common case is
that a user wrote the public label, which is exactly the row that already exists.

### Two entity-resolution problems, not one

Minting a base row raises the same question the private layer answers: is this the same thing as an
existing base row? So there are two ER problems — private identity and base identity — sharing the
normalization, scoring and forest code, but with different evidence and much higher precision
requirements at the base. Do not assume one solves the other, and do not let private merges vote on
base identity.

### Promotion is a broadcast

Linking is a point lookup; promotion fans out. When a base row is minted, every user's private nodes
with a matching normalized name and type should be *offered* the link. That runs as a background
re-match, not an inline write, and it is the real cost of promotion. The base graph therefore
converges gradually: `MUME` promoted today links yesterday's captures tomorrow.

### LLM adjudication (phase 3)

For candidate links, one pairwise call with both sides' evidence: label, type, description, up to
three distinct mention descriptions, and the source capture titles. Ask for
`{"same": true|false|"unsure", "reason": "..."}`. Cap at 5 candidates per mention so cost per
illumination stays ≈ one call, and record through `record_inference` like every other billable call.
Only `same: true` merges, and only leaf-into-root unless the call included both full member lists.

## Similarity and connectedness

Two orthogonal signals, reported together — never one opaque number in the UI:

| Signal | Source | Meaning |
| --- | --- | --- |
| **semantic** | cosine of node embeddings | "these are about the same kind of thing" |
| **structural** | co-capture counts over the cluster's mentions | "this user keeps encountering them together" |

```
semantic(a, b)   = cosine(emb(a), emb(b))                      -- calibrated per user
structural(a, b) = |caps(a) ∩ caps(b)|                         -- raw familiarity
strength(a, b)   = |caps(a) ∩ caps(b)| / |caps(a) ∪ caps(b)|   -- 0..1 association
```

Node embedding text: `"{display label} | {type} | {aliases} | {description}"`, dims from
`SEARCH_EMBED_VECTOR_DIMS`, hashed into `embedding_hash` so a typed task recomputes only on change.
Embedding is never used alone to merge; a high cosine across types is usually a *relation*, not an
identity.

Ranking for "related entities": z-score the cosine within the user, then
`rank = 0.6 * z + 0.4 * strength`. Store nothing derived; recompute per request over ≤ a few thousand
nodes.

## Descriptions

Two descriptions, one per layer:

- **Universal (shared).** Generated once per public entity from public sources, identical for
  everyone. This is the "what is this" answer and the thing we refuse to compute N times.
- **Private (per user).** The model's reading of that capture, or the user's own words. The "why is
  this in my graph" answer — short, contextual, often more interesting than a definition.

Generation is a first-class task, not a prompt smuggled into the enrichment fetch:

- **Inputs:** Wikidata claims (P31, P106, P569/P570, P27, …), the Wikipedia lead, and for
  source-grounded entities a small number of cited web results. Never a mention, a capture, or a
  private description.
- **Prompt contract:** entity-centric, timeless, no assumed context — describe what this is for
  someone who has never heard of it, in one or two sentences, and name the claim being relied on.
- **Provenance:** `description_source` and `description_model` are stored. A sourced description
  outranks a generated one; generation fills gaps and rewrites awkward sourced text, it never invents
  facts.
- **Refresh:** regenerate when the model or the entity's claims change materially; cheap, because it
  is once per public entity for all users.
- **Verification:** sample-audit accuracy the way resolve thresholds are audited. A wrong universal
  description is visible to everyone — a different failure class from a wrong private one.

The private description needs no such machinery: it is produced per illumination, stored on the
mention and the node, displayed as context, and never promoted. Rendering both is the honest UI:
*what it is* / *why it's here*.

## Enrichment

Lazy, not a Wikidata mirror:

1. A mention links to a QID with no `public_entities` row → insert a stub from the search result.
2. `public_entity_enrich` fetches `wbgetentities` (labels, descriptions, aliases, P31, P18 image,
   sitelinks), maps the class to our type, stores the image in shared storage.
3. `public_entity_describe` generates the universal description from the sourced material.
4. `enriched_at` / `described_at` drive refresh — months, not minutes.

Cost is per public entity, once, for every user forever. The row carries no `user_id`, so the
pipeline cannot accidentally depend on one. Images are the one place where "shared" means bytes and
money: cache per QID, serve from one path for everyone, never re-fetch per user.

## Read path and UI

- `KnowledgeEntityInfo` stays the network mention shape. Add `node_id` and a `canonical` summary
  (`id`, `label`, `type`, `mention_count`) so clients can group without a second call.
- New `GET /api/node/{id}` → node + aliases + mentions grouped by capture + related entities (with
  the signal breakdown) + merge children + the shared record when linked. Node-level access reuses
  the `user_id` + non-archived-capture guard.
- WebUI: `/entity/{id}` (mention) gains a "part of" header linking to `/node/{id}`; `/node/{id}` is
  the canonical page. Unmerge is `POST /node/{id}/unmerge/{child_id}`; unlink is
  `POST /node/{id}/unlink`.
- A public entity page renders the shared definition only, never counts or "also captured by" (A3).
- `/graph` (phase 4): force-directed over nodes and co-capture edges. Cytoscape.js is the boring
  choice at this scale; sigma.js if it ever outgrows canvas. This is the "Mind Map view" carried in
  [backlog.md](backlog.md).

## Invariants

Publishability of the shared record:

- **S1.** Every column of `public_entities` and `public_entity_aliases` is safe to show to any user,
  knowing nothing about who captured anything.
- **S2.** A private action may *trigger* a base row; it may never *populate* one. Labels, aliases,
  descriptions, types and sources come from public material only. The test: regenerate the row from
  `promotion_sources` alone and it should match, without reference to any user.
- **S2a.** Promotion of `real_person` and `social_media_account` requires an identifier — a platform
  handle (with display-name or role corroboration when promoting under the person's name) or an
  external public source. Volume of mentions is never evidence.
- **S3.** Deleting a user changes no `public_*` row. A shared entity that only one user ever linked
  survives their deletion intact.

Association:

- **A1.** No table has a foreign key joining rows of two different users. `public_*` tables have no
  `user_id`.
- **A2.** No response to any user reveals another user's data: no counts, no ids, no "also captured
  by", no cross-user co-occurrence.
- **A3.** A public entity page renders the shared definition only; it does not reveal that anyone
  captured the entity, or how many.
- **A4.** The link is private: `knowledge_nodes.public_entity_qid` is user-scoped data.
- **A5.** Aggregate public-entity statistics (if ever built) are a separate, explicit product
  decision with k-anonymity — not a side effect of this design.

S2 is the invariant most at risk: the fastest way to write a good universal description is to read
someone's private description, and the resulting record would still *look* publishable. Worth a test
that asserts the describe task reads no private tables.

## Backfill and measurement

- **Phase 1 backfill** is pure SQL and idempotent: resolve every *current* mention with
  `node_id is null`, in `id` order, per user. Implement as a `dreamscroll_db entities resolve`
  command (direct DB, like `sync_schema`) rather than a cloud Backfill — there is no inference, no
  queue, and 2,369 rows take milliseconds. Re-running must be a no-op; that is the acceptance test.
- **Phase 2/3 backfill** (embeddings, adjudication) is cloud work → typed `Backfill` definitions per
  [backfill.md](backfill.md), with the node id as the target.
- **P-a/P-c/P-d backfill** (linking, describing, promoting) is queued work per entity, resumable, and
  must be idempotent by `(identifier, method)`.
- `dreamscroll_db entities stats --user <id>`: current mentions, nodes, clusters, largest cluster,
  aliases/node, candidates by status, merges by method, linked share, and the duplicate rate
  `1 - distinct_keys / current_mentions`. This is the number that tells us whether the framework
  works.

## Phases

| Phase | Work | Acceptance |
| --- | --- | --- |
| **0 — Measure** | The queries above, turned into `entities stats`. | The summary table here and the audit's detail reproduce from the command. |
| **1 — Identity** | 4 private tables + `node_id`; normalization; rule-based resolve; mention upsert-by-position + latest-illumination scoping; `/api/node/{id}`; `/node/{id}`; unmerge; DB backfill. | Re-running the backfill changes nothing; the cross-capture duplicate groups collapse to one cluster each (spot-audited); unmerge restores the prior cluster exactly; `/entity/{id}` links to its node. |
| **2 — Similarity** | Node embeddings + hash; candidate links; related-entities UI; `entity_embed` task. | 20 hand-checked nodes show plausible neighbours; candidate-link precision ≥ 0.8 on a 30-pair sample. |
| **3 — Adjudication** | `entity_resolve` task; pairwise LLM; cluster merges with min-pair verification. | ≤1 extra inference call per illumination; measured precision ≥ 0.95 on 30 adjudicated pairs; no cluster grows by transitivity alone. |
| **4 — Graph** | `/graph` over private nodes and co-capture edges. | Renders 2k nodes with pan/zoom and click-through. |
| **P-a — Link** | `public_entities`, aliases, merges; nullable `public_entity_qid`; shared alias lookup; queued lookup with backoff; candidate validation; unlink. | Link precision ≥ 0.9 on 50 labelled mentions; the measured wrong referents are rejected; a second user's `Elon Musk` costs zero network calls. |
| **P-b — Enrich** | `wbgetentities` fetch, P31 → type mapping, labels/aliases, shared image cache, refresh. | One fetch per QID for all users; node page renders the shared definition; coverage reported per user. |
| **P-c — Describe** | Universal description generation from sourced material, provenance, refresh, sample audit. | 20 sampled descriptions are accurate, context-free, and identical for all users; a test asserts the describe path reads no private tables. |
| **P-d — Promote** | Source-backed promotion: platform-link identifier, attribution type decision, grounded source retrieval, `promotion_sources`, base-layer candidate/merge, background re-match. | Every promoted row is reproducible from its sources; zero rows whose name is a handle promoted as `real_person`; promotion precision ≥ 0.95 on a 40-entity audit; the re-match links prior captures without touching user-visible state. |

P-a needs phase 1 (there is nothing to link), and P-b…P-d need P-a. Within the private track, phases 2
and 3 are independent of each other; phase 4 needs 2.

## Non-goals and limits

- **Cross-user identity inside this design.** Per-user identity only. Shared *reference* identity is
  the public layer, and user graphs never merge into it.
- **Entities without public evidence stay private.** Promotion needs a citable source; a name that
  only exists inside a screenshot does not get a base row. `Wojak` and `Claude Code` are in Wikidata;
  `The Graph Maker` is not, and stays private.
- **No cross-user recommendations.** "People who captured X also captured Y" is aggregate behaviour,
  not an entity fact.
- **No user-contributed commons.** If it ever becomes desirable it needs consent, moderation and a
  curation path; it is the only way user-derived content becomes shared, and it must be explicit.
- **Fine-tuned entity-linking models.** Rules + embeddings + targeted LLM calls.
- **A graph database.** Postgres adjacency is sufficient at personal-graph scale.
- **Sentence-level coreference inside a capture** — the illumination prompt's job.
- **Rewriting the taxonomy.** See [knowledge-entity-types.md](knowledge-entity-types.md).
- **Promotion is not free and not instant.** Minting a base row costs a lookup, a verification, and a
  background re-match across users.

## Alternatives considered

| Alternative | Why not (now) |
| --- | --- |
| **Global entity table keyed by Wikidata QID** ([recommender doc](knowledge-wikidata-recommender.md)) | Coverage and ambiguity: most captures are niche enough to miss a clean QID; the write path gains a network dependency; per-user evidence ("what *I* mean by Ditto") is lost. Kept as an optional link. |
| **Unique `(user, type, normalized_name)` and merge by re-pointing mentions** | Cheapest to write, but destructive: a wrong merge rewrites history and cannot be undone without a backup. Also cannot express aliases or "same name, different type". |
| **Mention↔node many-to-many edge table** | One mention belongs to one entity in every case we can name. Ambiguous mentions are rare; the node tree already attaches at the most specific level. |
| **In-context LLM clustering per illumination** ([design space](https://arxiv.org/abs/2506.02509)) | Non-deterministic across runs, so re-illumination reshuffles identity; cost per capture rather than per ambiguity; no per-decision audit trail. |
| **pgvector or a second Vertex index for node similarity** | At ~2k nodes per user, in-app cosine is the boring answer; a vector index adds infrastructure and a second source of truth. |
| **Per-user entities and per-user enrichment** | Correct and private by construction, but every user pays to describe Elon Musk and the definition drifts between users. This is phase 1; the shared layer is the upgrade. |
| **Build the shared record from the union of user mentions** | The record stops being self-contained: its content depends on who captured what, so it fails the publishability test and degrades as the user base changes. |
| **Shared identity but no shared description** | Half the win: identity converges, but everyone still generates their own definition, which is the expensive and inconsistent part. |
| **Mirror Wikidata in bulk** | Hundreds of millions of items for a corpus that touches a few thousand. Lazy per-QID enrichment keeps storage and cost proportional to usage. |
| **One description, shared, overridden per user** | An override model forces a loser: either the definition or the user's context is hidden. Two descriptions answer two different questions. |
| **Promote on mention, without a citable source** | Maximum coverage and the shortest flow, but it turns the base graph into an unverifiable projection of user traffic: private individuals and model misreadings become link targets imposed on everyone, and the graph can never be audited or rebuilt. |
| **Let the promoting user's description seed the base description** | The fastest route to coverage and the one that quietly breaks S2: the row would look publishable while depending on private context. |
| **Separate global service or graph database** | A second source of truth and a network hop for data that fits in a few Postgres tables. Revisit only if enrichment volume exceeds the main database. |

## Risks and open questions

| Risk | Mitigation |
| --- | --- |
| Same-name distinct people auto-merge (e.g. `julius_caesar`). | Veto set + one-edge unmerge + precision-first thresholds; sample audit after phase 1. Record in [pragmatism.md](pragmatism.md). |
| Transitive mega-cluster from one bad link. | Leaf-into-root only for automatic merges; min-pair verification for cluster merges. |
| Re-illumination orphans assignments. | Upsert-by-position + latest-illumination scoping; a prerequisite, not a follow-up. |
| Embedding model/dims churn invalidates similarity. | `embedding_hash` + dims from config; rank within a user instead of thresholding absolute cosine. |
| Type churn invalidates existing merges. | Never auto-unmerge on taxonomy change; the type is re-evaluated as a candidate, not rewritten. |
| LLM pair costs on every capture. | Cap candidates per mention; only the ambiguous band. The model does not duplicate inside a run, so repeats reaching adjudication are genuine cross-capture cases. |
| Two concurrent captures create a node for the same new entity. | Accepted: the next resolution pass emits a candidate link, and the cluster is recoverable with one merge. Record in [pragmatism.md](pragmatism.md) when phase 1 lands. |
| A wrong promotion is imposed on every user. | Citable sources only, `promotion_sources` on every row, reversible base merges, sampled audit instead of a review queue. |
| Promotion fan-out overwhelms the queue. | Re-match runs in the background per user, bounded per run; it is a backlog, not a promise of immediacy. |

Open, worth deciding before the phase that needs them:

1. **Node display label** — most frequent alias vs. the highest-quality surface vs. the search query.
   Derived labels are cheap to change; pick one and move on.
2. **Do archived captures feed node evidence?** Current answer: yes for aliases/co-capture, no for
   read surfaces.
3. **Should a mention's type ever be corrected by its node's type?** Suggested: no. The mention
   records what the model said; the node records what we concluded.
4. **Where is the promotion bar, exactly?** Which sources count for a QID-less entity (Wikipedia, an
   official site, two independent reports?), and how do we reject a private individual whose name
   appears in a screenshot? Start strict, measure the promotion rate, loosen with evidence.
5. **Type mapping ownership.** P31 → `KnowledgeEntityType` is a curated table; who maintains it, and
   how does it interact with the taxonomy pass?
6. **Display precedence.** Proposed: the user's label wins for the title; the universal description
   and the private context are two labelled blocks, never merged into one paragraph.
7. **Rate-limit budget.** Wikidata asks for polite usage and has no API key. A global concurrency cap
   plus backoff is the minimum; do we also want a `maxlag` budget alarm?

## References

- [Entity Resolution in Practice: Lessons from a Self-Serve Pipeline](https://arxiv.org/abs/2607.26298)
  — bake-off thinking, precision vetos, and the transitive-closure failure mode.
- [In-context Clustering-based Entity Resolution with LLMs](https://arxiv.org/abs/2506.02509) —
  pair-vs-cluster prompting tradeoffs; why pairwise judgments need explicit transitivity guards.
- [Match, Compare, or Select? LLMs for Entity Matching](https://aclanthology.org/2025.coling-main.8.pdf)
  — listwise prompting is stronger than pairwise at equal cost for small candidate lists.
- [Entity Resolution and Record Linkage, Explained Plainly](https://dedupe.dev/guides/entity-resolution)
  — normalize before matching; wrong merges are worse than missed ones.
- [Almost all of entity resolution](https://www.science.org/doi/10.1126/sciadv.abi8021) — the
  canonical framing (blocking → comparison → clustering).
- [Wikidata `wbsearchentities`](https://www.wikidata.org/w/api.php?action=help&modules=wbsearchentities)
  — the lookup used by P-a; QIDs are stable keys, Wikipedia URLs are not.
- [Schema.org Person / Role / author](https://schema.org/Role) — the reference for the
  occurrence-vs-role distinction in the taxonomy doc.
- [Cytoscape.js](https://js.cytoscape.org/) / [sigma.js](https://www.sigmajs.org/) — phase-4
  visualization candidates.
