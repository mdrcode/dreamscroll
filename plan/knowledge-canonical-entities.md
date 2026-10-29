# Knowledge canonical entities — identity, merges, and similarity

**Status:** Design. Phases 0–4 scoped; nothing implemented. Phase 1 is startable.
**See also:** [knowledge-public-entities.md](knowledge-public-entities.md) (the shared public entity
record, universal descriptions, and the private overlay), [knowledge-entity-types.md](knowledge-entity-types.md)
(taxonomy, explicitly defers cross-capture grouping),
[knowledge-wikidata-recommender.md](knowledge-wikidata-recommender.md) (QID-keyed recommender
research), [pragmatism.md](pragmatism.md) (ledger), [backfill.md](backfill.md) (typed backfills),
`plan/archive/illuminate-v2.md` (the projection this builds on).

## The gap

Illumination v2 gives us **mentions**, not **entities**. Every `knowledge_entities` row is one
span of text some model reported for one capture: "Train Dreams", `movie`, in this capture. Nothing
says that the fourteen `Train Dreams` mentions are one work, and nothing lets us say how close two
of them are. The graph we want to visualize does not exist yet; we have an unordered bag of labels.

The product goal is an *AI-enriched personal knowledge graph*, which needs three layers that are
currently collapsed into one:

```
   capture ────────────────────────────────────────────────────┐
     │ media, url, annotation                                   │   the artifact
     ▼                                                          │
   mention ("this capture said X")       knowledge_entities    │   exists today
     │  name, type, description, platform_link                  │
     ▼                                                          │
   node    ("X, as this user knows it")  knowledge_nodes        │   phase 1
     │  label, type, aliases, embedding, external ref           │
     ▼                                                          │
   cluster ("all nodes that are X")      knowledge_merges       │   phase 1
        forward pointers to a merge root                        │
```

Three properties drive every decision below:

1. **Identity is an assertion, not a rewrite.** Merging must not move or destroy mentions, so
   reverting a merge is one pointer write, not a data reconstruction.
2. **Precision beats recall.** A missed merge is a duplicate row. A wrong merge silently fuses two
   people, and — through transitivity — can swallow a whole neighbourhood.
   ([ER in Practice, 2026](https://arxiv.org/abs/2607.26298) calls this out as its third lesson.)
3. **Similarity is computed, not declared.** Store evidence (co-capture counts, embeddings, shared
   handles); derive the score in one pure function so it can be re-tuned without touching data.

## What the current data says

Measured read-only against the local dev database (`dreamscroll_dev1`, 2026-10-29):

| Fact | Value |
| --- | --- |
| `knowledge_entities` rows (mentions) | 2,518 |
| captures | 710 (13 archived) |
| users with entities | 3 (one holds 2,486) |
| distinct `(user, type, normalized name)` keys | 2,134 |
| **excess mentions under exact normalized name + type** | **384** |
| duplicate groups | 236 (max group 14) |
| — duplicated inside a *single* capture | 84 groups — all re-illumination; **0** within one illumination |
| — duplicated across captures | 152 groups, 293 excess mentions |
| captures with ≥ 2 mentions | 579 of 710 (avg 4.02, max 18) |
| co-occurrence pairs within a capture | 4,515 |
| mentions with a description / search query | 2,474 / 1,425 |

Readings:

- **~15% of mentions are already exact-duplicates after conservative normalization.** 293 of the
  excess are cross-capture, i.e. real canonicalization wins. 91 are duplicate extraction inside one
  illumination — a prompt/dedup issue that node assignment will also absorb harmlessly.
- The graph is small. 579 captures hold two or more mentions; after collapsing identical names, 577
  of them contribute 4,515 co-capture pairs — a legible graph, and per-user node counts are ~2k. Any
  design that needs a service to hold vectors at this scale is premature.
- Descriptions are nearly always present, so embedding node text has real signal.
- **Normalization must be conservative.** Stripping punctuation collapses `C`, `C#`, and `C++` into
  one key (observed, 3 surface variants). `@handle` vs `handle` for `social_media_account` is a
  genuine variant worth folding. There is no single normalization that is safe for both; the fix is
  a type-aware rule plus a symbol veto, not a greedier key.

## Decisions

Ordered. Each is a call I would make unless argued otherwise.

1. **Canonicalize per user, not globally.** Not because sharing a referent is unsafe — public
   entities are public — but because *interpretation* is context-dependent: the same string is a
   Pokémon in one graph and a mood in the other, and the user's label and context belong to their
   graph. Nodes carry `user_id`; the public referent is a separate shared layer that nodes may link
   to. See [knowledge-public-entities.md](knowledge-public-entities.md).
2. **Every mention gets exactly one node at write time.** Merging is then purely node→node, and no
   merge ever edits a mention row. This is what makes reversibility cheap.
3. **Merges are forward pointers in a forest, not destructive folds.** `knowledge_nodes.merged_into_id`
   is the authoritative edge; `knowledge_merges` is the audit log. Unmerge = clear the pointer.
4. **Automatic merges attach a leaf to a root.** Cluster-into-cluster merges require stronger
   evidence (LLM adjudication with both member lists, or a user action). Damage from one bad
   automatic merge is then bounded to one node.
5. **Reads resolve to the root lazily; we do not path-compress.** A handful of pointer hops beats a
   denormalized root column that can drift and destroy per-edge reversibility. Depth is small by
   construction (decision 4).
6. **Aliases are never copied during a merge.** Alias lookup considers the whole subtree, so a merge
   writes nothing but the pointer. Display label is derived (most frequent alias in the cluster),
   not mutated.
7. **The node is what gets embedded, and vectors live in Postgres.** Cosine over one user's ~2k
   nodes in Rust is sub-millisecond; a second vector index with its own staleness story is not.
8. **Similarity is a ranking, not a threshold.** Raw Gemini cosine has a high, model-dependent
   baseline. Calibrate per user (z-score against that user's own distribution) before combining with
   structural signals.
9. **Names decide automatic merges; embeddings break ties and rank recommendations.** Embeddings
   conflate related entities (Jobs ≈ Wozniak), so they are the weakest of the strong signals for
   identity.
10. **Wikidata QIDs are an optional link to a shared reference layer, not the identity key.** The
    [recommender doc](knowledge-wikidata-recommender.md) keys every node by QID; that is right long
    term and premature as the *primary* key (ambiguous matches, network dependency, taxonomy still
    moving). QID is a strong merge signal, a bridge to a shared public foundation, and the key for
    enrichment that is fetched once rather than per user. See
    [knowledge-public-entities.md](knowledge-public-entities.md).

## Schema

Four new tables plus one nullable column. No renames; `knowledge_entities` stays the mention table.
Bump `REQUIRED_SCHEMA_VERSION` (currently 8).

```sql
knowledge_nodes
  id              i32 pk
  user_id         i32   -- scoped identity; not nullable
  name            text  -- the label this node was created with (never mutated by merges)
  entity_type     text
  description     text  -- best description seen, promote-on-improve only
  merged_into_id  i32 null -> knowledge_nodes.id   -- union-find parent
  embedding       real[] null                      -- node text vector, nullable until phase 2
  embedding_hash  text null                        -- sha256 of the embedded text
  public_entity_qid text null                      -- link to the shared layer; see the public doc
  created_at, updated_at
```

> "At most one outgoing parent edge per node" is structural: the parent edge is a single column on
> the child row. The partial unique index lives on the merge log
> (`knowledge_merges (child_id) where reverted_at is null`) so log and pointer agree.

```sql
knowledge_aliases
  id, user_id, node_id -> knowledge_nodes.id
  surface      text  -- exactly what the model wrote
  normalized   text  -- conservative canonical key, see Normalization
  from_mention i32 null -> knowledge_entities.id
  created_at
  unique (node_id, normalized, surface)
  index  (user_id, normalized)      -- blocking lookup
```

```sql
knowledge_merges
  id, user_id
  child_id  i32 -> knowledge_nodes.id   -- the node that now forwards
  parent_id i32 -> knowledge_nodes.id   -- the node it forwards to
  method    text  -- exact_key | token_set | embedding | llm | user | external | backfill
  score     real null
  evidence  jsonb null
  created_at, reverted_at timestamptz null
  check (child_id <> parent_id)
  unique (child_id) where reverted_at is null
  index  (parent_id, reverted_at)
```

```sql
knowledge_candidates                 -- the ambiguous band, deferred decisions
  id, user_id
  node_a_id i32, node_b_id i32       -- always a < b, one row per unordered pair
  score     real
  features  jsonb                    -- name_sim, type_match, cosine, method
  status    text  -- pending | accepted | rejected
  created_at, decided_at
  unique (node_a_id, node_b_id)
```

Plus `knowledge_entities.node_id i32 not null -> knowledge_nodes.id` (added nullable, backfilled,
then tightened) with indexes on `(user_id, node_id)` and `(capture_id, node_id)`.

No edge table yet. Co-capture connectedness is a join over `knowledge_entities`; add
`knowledge_edges` only when we have non-derived edges (LLM-asserted relations, user-drawn links).
That is the trigger, not "we want a graph" — the derived graph is already available.

Model files: `src/model/knowledge_node.rs`, `knowledge_alias.rs`, `knowledge_merge.rs`,
`knowledge_candidate.rs`; logic in a new `src/knowledge/` module (`normalize.rs`, `resolve.rs`,
`merge.rs`, `similarity.rs`) with a lean root.

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
  `{n : resolve(n) == parent}`; every mention/alias still points at its original node and is
  reachable because reads resolve the node first.
- **New mentions attach to the most specific node, never the flattened root.** If the best candidate
  is itself forwarding (`M → R`), the mention points at `M`. That keeps the finer distinction
  available for a later unmerge: cut `M → R` and the mentions that resemble `M` are already on `M`.
  Flattening on write would make every unmerge restore only a label, not a partition.
- **Unmerge is exact.** Cutting `child → parent` re-roots the child's whole subtree and nothing
  else. This is why we attach roots only: the edge you cut corresponds to the decision you made.
- **The only field a merge may write outside the pointer is `description`**, and only to fill a null
  on the parent. Labels, aliases, and types stay on their own nodes; display is derived at read time,
  so unmerge has nothing to undo but the pointer.
- **Depth stays ~1** for automatic merges (leaf into root). Cluster merges add one level. If depth
  ever exceeds ~8, that is a signal to stop and re-verify, not to compress.
- **No deletion.** A node with zero mentions is invisible to reads but retained, because a merge log
  entry may still reference it. Clean-up is a separate, explicit operation (and is not needed).
- **Archived captures** follow today's read filters: mentions from archived captures stay linked and
  still feed alias/co-capture evidence, but are excluded from user-facing lists. A node whose only
  mentions are archived therefore disappears from the UI without corrupting the graph.

Reads use one recursive CTE to turn any node into its cluster, so callers never loop in application
code:

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
  SELECT n.id
  FROM knowledge_nodes n JOIN down d ON n.merged_into_id = d.id
)
SELECT id FROM down;
```

Both directions matter: upward for "which canonical entity is this", downward for "which nodes
belong to it". The upward walk is bounded by the forest depth, which is why depth matters.

### The write path has to stop churning rows

`insert_illumination_v2` currently deletes all `knowledge_entities` for an illumination and
re-inserts them. On re-illumination that would silently discard every node assignment. Change it to
upsert by the existing unique key `(illumination_id, position)`:

- position present in both runs → same mention row, keep the node unless `(normalized name, type)`
  changed;
- new positions → insert, resolve;
- vanished positions → delete the mention, keep the node (it may still have other mentions).

The re-resolution rule is: a mention whose normalized key or type changed is re-assigned; everything
else is left alone. Record the reassignment method as `reillumination` in a mention-level history
only if we need it later — start without it.

**A capture can have several illuminations, and only the latest is live.** Illuminations are
append-only: 34 of 628 illuminated local captures carry two or three, and the read path already
exposes only `max(id)`. The entity rows of the earlier runs are still in `knowledge_entities`
(**149 of 2,518 rows, 5.9%**), so anything that resolves or counts "all mentions" double-counts a
capture. Specifically:

- `entity_resolve` must scope to the latest illumination per capture, not to every mention row;
- the superseded runs' mentions should be marked (or deleted) when a new illumination lands, so a
  later backfill cannot resurrect them;
- every "mentions" metric needs the same filter, or the audit's numbers drift by ~6%.

Correction to the earlier audit summary: the 84 "duplicate inside one capture" groups are entirely
this artifact. Restricted to the latest illumination per capture, **zero** groups repeat a name inside
one run. The model does not duplicate entities; re-running the illumination does.

## Resolution pipeline

Runs after mentions are written, and deliberately after **nothing else**: the private graph is
complete before any network call, so an upload never waits on entity linking. Phase 1 resolves
inline in the projection transaction (DB reads plus in-app string work, no network). The public
stage — link or promotion against the shared base graph — always runs as a queued
`entity_resolve` task, one per illumination, because it needs embeddings, Wikidata, and adjudication
calls. See [knowledge-public-entities.md](knowledge-public-entities.md) for that stage. Logical id
`u{owner}-entity_resolve-illumination{id}` gets the framework's existing in-flight guard.

### Normalization

One pure function, `knowledge::normalize::key(surface) -> String`:

1. Unicode NFKC; casefold.
2. Collapse whitespace, trim.
3. Fold diacritics (`Pokémon` → `pokemon`).
4. Strip *leading* `@` / `#` and *surrounding* quotes; strip a trailing period. Nothing else.
5. Keep inner punctuation (`C#`, `C++`, `U.S.`, `Spider-Man`).
6. Token set key for blocking: sorted tokens minus leading articles (`the`, `a`, `an`). Used only
   for candidate generation, never as an identity key.

Rule 5 is deliberate: the observed `C / C# / C++` collision is exactly what aggressive stripping
buys us. Rule 4 is deliberate: `@bandy1803` and `bandy1803` are one account.

### Candidates (blocking)

Per incoming mention, over the *same user's* nodes only:

- exact `normalized` match in `knowledge_aliases` (indexed);
- token-set key equality (in-app, ~2k nodes — trivial; revisit above ~100k nodes/user);
- same `platform` + `platform_handle` (strong for `social_media_account`);
- phase 2: top-k cosine against node embeddings.

### Scoring

```
score = 0.55 * name_sim
      + 0.20 * type_match
      + 0.15 * cosine            (0 when unavailable)
      + 0.10 * description_sim   (token Jaccard, 0 when absent)
```

with hard vetos applied after scoring, not as negative weights (a veto must not be tradeable):

| Veto | Rationale |
| --- | --- |
| types differ, both non-`unknown` and pair is in the incompatibility set (`real_person`/`fictional_character`, `book`/`movie`, `organization`/`real_person`, …) | someone re-using a name is not the same entity |
| names differ only by a symbol, both `software`/`product`/`financial` | `C` ≠ `C#` |
| `platform` set on both and differs | different accounts |
| `platform` equal but handles differ | different accounts |

Thresholds, as starting constants in one module with a table test:

- **direct attach**: veto-free and `name_sim = 1.0` (identical normalized key) or same
  platform+handle. No node, no merge row.
- **auto-merge**: veto-free and `0.97 <= name_sim < 1.0` (diacritic, article, or minor punctuation
  variant). New node, merged leaf → root.
- **candidate link**: veto-free and `0.75 <= name_sim < 0.97`, or phase 2 `cosine >= p99` of the
  user's distribution with `type_match = 1`. Stored, not merged.
- **new node**: everything else.

`name_sim` = 1 − normalized Levenshtein over `normalized`, computed in-app (no `pg_trgm` dependency).

### Direct attach vs. create-then-merge

Two mechanically different outcomes, and the distinction is what keeps the tables small:

- **Direct attach** when the mention's `normalized` key is already an alias of a type-compatible
  node. No node is created, no merge row is written: this is the same entity, spelled the same way.
  All 14 `Train Dreams` mentions become one node and one alias, not a 14-node chain.
- **Create-then-merge** when the match is fuzzy (variant spelling, diacritics, article prefix). A new
  node is created for the variant surface and immediately merged into the matching root, producing
  exactly one merge-log row that can be reverted on its own.

Exact-key matches still have to clear the type vetos. `Dune` the `book` and `Dune` the `movie` are
different nodes with the same alias, and `knowledge_aliases` deliberately has no global uniqueness
on `normalized`.

### Aliases are where personas live

Illumination keeps one entity per person and hangs the account off it, so a persona name (`Tsoding`,
`Meta-Nomad`, `clara`) exists only as `platform_display_name` or as prose in the description ("known
online as Tsoding"). That is enough for display and useless for matching: a later capture showing only
the persona produces a different node.

The mention should therefore carry an **`aliases` array** — persona names, pseudonyms, alternate
titles — and every alias becomes a `knowledge_aliases` row on the node. Resolution then matches on the
alias, and "Tsoding" and "Alexey Kutepov" converge without inventing a `persona` type (see
[knowledge-entity-types.md](knowledge-entity-types.md)). Because an alias is a *label*, a merge never
has to invent a relation to express it.

If the persona ever needs to be a node of its own, that is a relation (`pseudonym_of`,
`operated_by`), not a second label on the same node — and it is the trigger for the deferred edge
table. The local corpus has no capture where both appear, so it can wait.

### Cluster-level verification

Before merging two *clusters*, score every cross pair (bounded: if either side exceeds ~20 members,
fall back to root-vs-root plus a sample) and require the minimum pair score to clear the bar. This is
the concrete defence against the transitivity failure in
[ER in Practice](https://arxiv.org/abs/2607.26298): "A matches B" and "B matches C" must not be
allowed to imply "A matches C" without a direct check.

### LLM adjudication (phase 3)

For candidate links, one pairwise call with both sides' evidence: label, type, description, up to 3
distinct mention descriptions, and the source capture titles. Ask for
`{"same": true|false|"unsure", "reason": "..."}`. Cheapest correct shape; cap at 5 candidates per
mention so cost per illumination stays ≈ one call. Record through `record_inference` like every
other billable call. Only `same: true` merges, and only leaf-into-root unless the call included both
full member lists.

## Similarity and connectedness

Two orthogonal signals, reported together — never one opaque number in the UI:

| Signal | Source | Meaning |
| --- | --- | --- |
| **semantic** | cosine of node embeddings | "these are about the same kind of thing" |
| **structural** | co-capture counts over the cluster's mentions | "this user keeps encountering them together" |

```
semantic(a, b)  = cosine(emb(a), emb(b))                      -- calibrated per user
structural(a, b) = |caps(a) ∩ caps(b)|                        -- raw familiarity
strength(a, b)   = |caps(a) ∩ caps(b)| / |caps(a) ∪ caps(b)|  -- 0..1 association
```

Node embedding text: `"{display label} | {type} | {aliases} | {description}"`, dims from
`SEARCH_EMBED_VECTOR_DIMS`, hashed into `embedding_hash` so a typed task recomputes only on change.
Embedding is never used alone to merge; a high cosine across types is usually a *relation*, not an
identity.

Ranking for "related entities": z-score the cosine within the user, then
`rank = 0.6 * z + 0.4 * strength`. Store nothing derived; recompute per request over ≤ a few
thousand nodes. Revisit only if a detail page's compute becomes measurable against its DB time.

## Read path and UI

- `KnowledgeEntityInfo` stays the network mention shape. Add `node_id` and a `canonical` summary
  (`id`, `label`, `type`, `mention_count`) so clients can group without a second call.
- New `GET /api/node/{id}` → node + aliases + mentions grouped by capture + related entities
  (with the signal breakdown) + merge children. Node-level access reuses the same
  `user_id` + non-archived-capture guard as `get_knowledge_entity`.
- WebUI: `/entity/{id}` (mention) gains a "part of" header linking to `/node/{id}`; `/node/{id}` is
  the canonical page. Unmerge is a `POST /node/{id}/unmerge/{child_id}` that clears one edge.
- `/graph` (phase 4): force-directed over nodes and co-capture edges, Cytoscape.js is the boring
  choice at this scale ([comparison](https://www.pistack.xyz/posts/2026-06-09-self-hosted-graph-network-visualization-cytoscapejs-g6-visnetwork-forcegraph));
  sigma.js if the graph ever outgrows SVG/canvas. This is the "Mind Map view" already carried in
  [backlog.md](backlog.md).

## Backfill and measurement

- **Phase 1 backfill** is pure SQL and idempotent: resolve every mention with `node_id is null`, in
  `id` order, per user. Implement as a `dreamscroll_db entities resolve` command (direct DB, like
  `sync_schema`) rather than a cloud Backfill — there is no inference, no queue, and 2,518 rows take
  milliseconds. Re-run must be a no-op; that is the acceptance test.
- **Phase 2/3 backfill** (embeddings, LLM adjudication) *is* cloud work → typed `Backfill`
  definitions per [backfill.md](backfill.md), with the node id as the target.
- Add `dreamscroll_db entities stats --user <id>`: mentions, nodes, clusters, largest cluster,
  aliases/node, candidates by status, merges by method, and the duplicate rate
  `1 - distinct_keys / mentions`. This is the number that tells us whether the framework works.

## Phases

| Phase | Work | Acceptance |
| --- | --- | --- |
| **0 — Measure** | The queries above, turned into `entities stats`. | Numbers in this doc reproduce from the command. |
| **1 — Identity** | 4 tables + `node_id`; normalization; rule-based resolve; mention upsert-by-position; `/api/node/{id}`; `/node/{id}`; unmerge; DB backfill. | Re-running the backfill changes nothing; all 236 exact-normalized duplicate groups collapse to one cluster (spot-audit for false merges); unmerge restores the prior cluster exactly; `/entity/{id}` links to its node. |
| **2 — Similarity** | Node embeddings + hash; candidate links; related-entities UI; `entity_embed` task. | 20 hand-checked nodes show plausible neighbours; candidate-link precision ≥ 0.8 on a 30-pair sample. |
| **3 — Adjudication** | `entity_resolve` task; pairwise LLM; cluster merges with min-pair verification. | ≤1 extra inference call per illumination; measured precision ≥ 0.95 on 30 adjudicated pairs; no cluster grows by transitivity alone. |
| **4 — Graph** | `/graph` over private nodes and co-capture edges. | Renders 2k nodes with pan/zoom and click-through. |

Public linking and enrichment are a separate, parallel workstream — see
[knowledge-public-entities.md](knowledge-public-entities.md) (phases P-a…P-d). It can land before,
after, or between phases 2 and 3; nothing here depends on it.

## Non-goals (now)

- Cross-user or global identity inside this plan. Per-user only; the shared *reference* layer is
  [knowledge-public-entities.md](knowledge-public-entities.md), and user graphs never merge into it.
- Fine-tuned entity-linking models. Rules + embeddings + targeted LLM calls.
- A graph database. Postgres adjacency is sufficient at personal-graph scale.
- Sentence-level coreference inside a capture (the illumination prompt's job).
- Merging `unknown`-typed entities into typed ones automatically beyond the one-way rule
  (unknown mention may attach to a known node; a known mention never attaches to an unknown node
  unless nothing else matches).
- Rewriting the taxonomy. See [knowledge-entity-types.md](knowledge-entity-types.md).

## Alternatives considered

Recorded so the next pass does not re-litigate them.

| Alternative | Why not (now) |
| --- | --- |
| **Global entity table keyed by Wikidata QID** ([recommender doc](knowledge-wikidata-recommender.md)) | Coverage and ambiguity: most captures are niche enough to miss a clean QID; the write path gains a network dependency; per-user evidence ("what *I* mean by Ditto") is lost. Kept as phase 4 `external_ref`. |
| **Unique `(user, type, normalized_name)` column and merge by re-pointing mentions** | Cheapest to write, but destructive: a wrong merge rewrites history and cannot be undone without a backup. Also cannot express fine-grained aliases or "same name, different type". |
| **Mention↔node many-to-many edge table** | One mention belongs to one entity in every case we can name today. Ambiguous mentions are rare; the node tree already lets us attach at the most specific level. Add the edge table if a real need appears. |
| **In-context LLM clustering per illumination** ([design space](https://arxiv.org/abs/2506.02509)) | Non-deterministic across runs, so re-illumination would reshuffle identity; cost per capture rather than per ambiguity; no audit trail per decision. Pairwise adjudication in the ambiguous band is cheaper and explainable. |
| **pgvector or a second Vertex index for node similarity** | At ~2k nodes per user, in-app cosine is the boring answer. A vector index adds infrastructure, staleness, and a second source of truth for a quantity we recompute per request. |

## Risks and open questions

| Risk | Mitigation |
| --- | --- |
| Same-name distinct people auto-merge (e.g. `julius_caesar`, 4 mentions). | Veto set + one-edge unmerge + precision-first thresholds; sample-audit after phase 1. Accepted trade-off to be recorded in [pragmatism.md](pragmatism.md). |
| Transitive mega-cluster from one bad link. | Leaf-into-root only for automatic merges; min-pair verification for cluster merges. |
| Re-illumination orphans assignments. | Upsert-by-position (above); it is a prerequisite, not a follow-up. |
| Embedding model/dims churn invalidates similarity. | `embedding_hash` + dims from config; rank within a user instead of thresholding absolute cosine. |
| Type churn invalidates existing merges. | Never auto-unmerge on taxonomy change; type lives on the node and is re-evaluated as a candidate, not a rewrite. |
| LLM pair costs on every capture. | Cap candidates per mention; only the ambiguous band. The model does not duplicate inside a run, so the same-entity repeats that reach adjudication are genuine cross-capture cases. |
| Two concurrent captures can both create a node for the same new entity (no unique key exists mid-flight). | Accepted: the next resolution pass emits a candidate link, and the cluster is recoverable with one merge. Record in [pragmatism.md](pragmatism.md) when phase 1 lands. |

Open, and worth a decision before phase 1 lands:

1. **Node display label** — most frequent alias vs. the highest-quality description-bearing surface
   vs. the LLM's `search_query`. Derived labels are cheap to change; pick one and move on.
2. **Do archived captures feed node evidence?** Current answer: yes for aliases/co-capture, no for
   read surfaces. If unarchiving must be lossless, this is already fine; if archived content must
   never influence identity, it is not.
3. **Should the mention type ever be corrected by the node's type?** Suggested: no. The mention
   records what the model said; the node records what we concluded.

## References

- [Entity Resolution in Practice: Lessons from a Self-Serve Pipeline](https://arxiv.org/abs/2607.26298)
  — bake-off thinking, precision vetos, and the transitive-closure failure mode.
- [In-context Clustering-based Entity Resolution with LLMs](https://arxiv.org/abs/2506.02509) —
  pair-vs-cluster prompting tradeoffs; why pairwise judgments need explicit transitivity guards.
- [Match, Compare, or Select? LLMs for Entity Matching](https://aclanthology.org/2025.coling-main.8.pdf)
  — listwise prompting is stronger than pairwise at equal cost, when the candidate list is small.
- [Entity Resolution and Record Linkage, Explained Plainly](https://dedupe.dev/guides/entity-resolution)
  — normalize before matching; ~0.88 is a starting fuzzy threshold; wrong merges are worse than
  missed ones.
- [Almost all of entity resolution](https://www.science.org/doi/10.1126/sciadv.abi8021) — the
  canonical survey framing (blocking → comparison → clustering).
- [Wikidata `wbsearchentities`](https://www.wikidata.org/w/api.php?action=help&modules=wbsearchentities)
  — the phase-4 lookup; QIDs are stable keys, Wikipedia URLs are not.
- [Schema.org Person / Role / author](https://schema.org/Role) — already the reference for the
  occurrence-vs-role distinction in the taxonomy doc.
- [Cytoscape.js](https://js.cytoscape.org/) / [sigma.js](https://www.sigmajs.org/) — phase-4
  visualization candidates.
