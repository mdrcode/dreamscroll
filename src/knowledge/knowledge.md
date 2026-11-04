# The knowledge graph

The knowledge lib turns the user's illuminated captures into a structured, deduped, resolved and
enriched graph of entities.

```
Capture ──illuminate──▶ Illumination ──▶ Observation ──link──▶ Entity ──resolve──▶ Evidence
```

Every clause of that line has a home in this directory, and everything else is detail.

The leftmost step is not ours: **illumination** — the model turning an image into a summary plus the
entities it believes are present — lives in the sibling `src/illumination/` lib, and its vocabulary
is described in `plan/lexicon.md`. (It has no `illumination.md` yet. When it does, this file should
link to it rather than paraphrase it.)

Captures, their illuminations, and their entity observations come from the user
(with a little help from inference). From these raw inputs, the knowledge lib
construct, maintains, evolves, and enriches a common knowledge graph shared by
all users.

## Observations: what the user saw

An observation is one thing one capture named — a name, a type, a description, sometimes a platform
link. It is the Illuminator's best effort, recorded **verbatim and never rewritten**, and it lives
in `user_observed_entities`.

Two properties matter more than the columns:

- **It belongs to one user.** Observations are personal.
- **It stands alone.** `knowledge_entity_id` is null until something links it, and `reset` can drop
  every entity and every edge without touching a single observation. Nothing is lost, because nothing
  was ever edited.

That is also why history is safe to keep: an illumination can be wrong — a re-read of one image once
described an entirely different film — so the graph holds **every** illumination of every capture. The
question of which illumination a user *sees* is answered later, in "The view boundary".

## Entities: what the graph knows

Every row of the graph starts the same way. The first observation that names something — *Train
Dreams*, in one screenshot — has nothing to attach to, so `link` creates an entity for it. Later
observations that look like the same thing are **linked to that entity rather than creating another**,
and each one hands over what it knows: another name, another description, another type. The entity
keeps them all.

That accumulation is the point. Every extra surface and every extra sentence of description makes the
entity easier to recognise the next time it appears, so the graph gets better the more the user
captures — and the first observation of a thing is always the expensive one.

An entity is the thing itself, shared by every user, in `knowledge_entities`. It is not authored; it
is **recomputed**: every derived column is a function of the entity's observations and its evidence, so
a merge or a fresh link simply recomputes them. `update_projection()` is the only writer of `name`, `alt_names`,
`entity_type`, `alt_types`, `description`, `alt_descriptions`, `search_query`, `alt_search_queries`,
`status` and `confidence`.

```
name / alt_names            the chosen surface, and every other one seen
entity_type / alt_types     the chosen type, and every other one asserted
description / alt_*         same shape: one selected, the rest kept for matching
search_query / alt_*        the disambiguation hint, projected the same way
platform / platform_*       the presence one member carried, so the resolver can ask by handle
status                      provisional → identified
merged_into_id              set when this entity was folded into another
```

A **surface** is just a string the thing has been called: its name, a spelling variant, a display
name, a handle. The `alt_*` columns are not clutter — they are **the complement of the selected
value**, and they are what lets a later observation find this entity by a name it was once seen
under.

## Linking: are these the same thing?

Linking asks one question — **have we already got this thing?** — and asks it about a whole
**illumination** at once. That is the unit, because it is what the system produces: one illumination of one
image, a coherent set of the things that image showed. An observation is a part of that claim set,
not an independent event.

```rust
trait Blocker {                  // recall: find candidates, over-generate
    async fn candidates(&self, observation: &Observation) -> Vec<Entity>;
    fn changed(&self, entity: &Entity);          // an in-memory blocker only; a no-op otherwise
    fn forget(&self, id: i32);                   // discarded or tombstoned; ditto
}

// All three take `&self`, so the store holds one `Arc<dyn Blocker>` and no lock: an implementation
// that keeps an index owns its own mutex, and nobody holds a guard across a `candidates` call.

trait Linker {                   // precision: decide, and nothing else
    async fn link_illuminated_observations(
        &self,
        blocker: &dyn Blocker,
        illumination: &Illumination,
        observations: &[Observation],
    ) -> LinkResult;
}

enum Decision {
    Attach { entity: i32, score: f32 },
    Create { best: Option<(i32, f32)> },
}

struct LinkResult {
    decisions: Vec<(i32, Decision)>,   // observation id → what to do
    vetoed: u32,                       // how often the rule below actually fired
}
```

**The veto is the linker's, and it is why the batch exists.** One extraction that emitted two names
meant two things, so an entity a sibling from the same illumination has already taken is not a
candidate — however well it scores. That is a statement about the *set*, which is why a per
observation loop could only ever apply it after the fact. Siblings that arrived already linked (by
an account identifier, before the linker ran) are context: they seed the set, and no decision is owed
for them.

Both traits take **model rows** — `user_observed_entity::Model` and `knowledge_entity::Model` —
because a separate view layer was a second place for a column to go missing, and it went missing
twice. What a strategy must not do is *write* one: `update_projection` is the only writer of a derived column.
The derived forms the scorer works with (`surfaces()`, `descriptions()`, `types()`) are inherent
methods on the model, beside its columns.

`link.rs` is the executor: load the rows, drive them through the blocker and linker, apply the
effects.

**The heuristic is one implementation of that chain — not the chain.** `heuristic.rs` holds our
first prototype: normalized name keys to block on, weighted features, a bar to clear. It is
deliberately self-contained, which is the property worth protecting: everything it needs lives in
that one file, so it can be tuned, replaced, or deleted without touching the framework. A future
linker might block on embeddings, decide with a model, or reason over evidence in a way that shares
nothing with this one — it would implement the same two traits and import not a line of `heuristic`.
Delete that file today and the lib still compiles; it simply has no policy.

## Resolving: what can the authorities tell us?

Resolving is the opposite kind of work: network, cached, and only ever through one door.

```rust
trait Resolver {
    fn id(&self) -> &'static str;
    async fn resolve(&self, entities: &[Model], ctx: &ResolutionContext)
        -> Vec<(i32, Claim)>;                 // claims, each tagged with its entity
}

trait Transport {
    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError>;
}
```

`resolve` takes a *batch*, not one entity: batching across entities is the difference between a
hundred transport calls for a corpus walk and five thousand, and only the resolver knows how to
group its own requests.

How a resolver gets to its claims is its own business. `Wikidata` uses a two-pass ladder — handles,
titles and exact labels first, then full-text search for whatever is left — but that shape is
internal to it, not vocabulary the engine knows about. There is no `plan`/`plan_fallback` in the
contract, so **adding an authority means writing one `Resolver`**, and a resolver is free to make one
request or a dozen.

The one thing a resolver cannot do is fetch. Its only route to the world is
`ResolutionContext::ask`, which consults the cache first, charges the budget and paces the call —
which is why a resolver can own its own strategy without being able to bypass any of them. (An
agentic resolver, whose tool calls happen inside a provider's runtime, is the one exception: it uses
the same context by convention, and it is expected to fail at that rather than to pretend.)

Resolvers **accumulate**. Wikidata supplies a QID, and a second resolver might supply an IMDb id;
they answer different questions, so both are kept. The promise is not "the answer" but *independent
external evidence that this identity is true*.

`plan` and `parse` inside `Wikidata` are pure, so its whole decision can be re-run over cached
payloads for free — that is what `examples/eval_resolve` does.

## Evidence: why we believe it

Four tables, because they answer four different questions:

```
knowledge_identities   the thing the world names: scheme + external id, and the answer
                       whoever gave it (label, description, classes, method, confidence)
knowledge_claims       this entity is that thing — the edge; several entities may claim
                       one identity, because an album's article covers its songs
knowledge_proximities  two entities the graph considers close, with the score that a pass
                       computed and the verdict somebody gave — a record, never deleted
knowledge_merges       this entity was folded into that one, and why — the log an unmerge reads
```

Only the first two are about the world. A proximity is a judgement about our own rows and a merge is a
mutation of them. `enrichment` and `rejection` stay reserved vocabulary, not tables.

Sharing an identity is a **candidate**, not a verdict — that is why the uniqueness lives on the edge
(one entity asserts an identity once) rather than on the identity. `reconcile.rs` asks the question:
for each identity with two or more holders, score the holders pairwise with the linker's weights, fold
the pairs that look like one thing, and queue the ones the veto refuses.

The only uniqueness is per entity: one live claim per `(entity, identity)`, so retracting frees that
pair's slot and re-asserting it is a no-op. Nothing forbids two entities holding one identity — that is
the many-to-one the model exists for.

`reconcile.rs` runs in two phases. **Scan** reads the graph and only writes proximities (idempotent, so
it runs after a link as well as at the end of a resolve). **Merge** consumes the open rows, re-checks
that both sides still exist, scores the pair with the linker's own weights and folds the convincing
ones; the rest stay open for a person, who answers `distinct` or merges by hand.

## The view boundary

The graph keeps every illumination; a user sees one.

A capture's **current** illumination is its highest id, and every user-facing read starts there and
left-joins into entities. That rule lives where the view does
(`api::user::get_entity::current_illumination`), not here: `link` links every illumination, and the
API decides which one a user sees. Two consequences fall out for free:

- An entity whose observations are all superseded — a *ghost* — can never surface, because nothing
  queries entities to decide what a user sees.
- An unlinked observation still renders. The card shows the observation's own name and type; entity
  enrichment must stay optional.

## Repair

Entities are cheap to rebuild and expensive to get wrong, so the model prefers undoable moves:

- `reset` snapshots the projection, then empties it. Observations are untouched; `link` rebuilds.
- `merge` folds one entity into another, tombstones the loser, and runs in a **transaction** — five
  writes that must all land.
- The **same-illumination veto**: two entities that both appear in one illumination cannot be one
  referent, because a single extraction emitting both means two things. The unit is the illumination,
  not the capture — re-illuminating an image is a separate illumination.

## Where things live

| file | what it is |
|---|---|
| `blocker.rs`, `link_decider.rs` | the two traits, and nothing else — the contract a linker implements |
| `heuristic.rs` | our first linker, self-contained: token keys, features, weights, bars, `NameBlocker`, `HeuristicLinker` |
| `resolve/driver.rs` | the resolver contract, the engine that runs it, claim storage |
| `resolve/wikidata/` | one authority: policy in `resolver.rs`, HTTP in `transport.rs` |
| `graph.rs` | the facade: `Knowledge`, its constructors and the methods that are one write each |
| `link.rs` | the linking executor, and `LinkStats` |
| `project.rs` | `project`: the only writer of derived columns, with `gather`/`select`/`surfaces` |
| `merge.rs` | fold one entity into another |
| `evidence.rs` | claims and identities: who holds one, and the platform key a presence answers to |
| `reconcile.rs` | the pass that asks whether one identity's holders are one thing |
| `snapshot/` | take a copy of the projection, empty it, diff two copies |
| `resolve/cache.rs` | answers, keyed by `(provider, kind, shape, query)` |
| `normalize.rs` | `comparable`: the form two surfaces are compared in. The token pipeline is policy, in `heuristic.rs` |

The three write paths that carry their own logic and tests (`link.rs`, `project.rs`, `merge.rs`) are
private modules: nothing in them is `pub`, and callers reach them through `Knowledge`'s methods. The
graph's health counters are a report over the tables rather than graph work, so they live in
`crate::stats::knowledge_stats`.

The vocabulary — and the names we rejected — is in `plan/lexicon.md`. The design history, including
the failures that produced these rules, is in `plan/knowledge-illumination-history.md` and
`plan/knowledge-canonical-entities.md`.
