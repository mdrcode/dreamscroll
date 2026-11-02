# The knowledge graph

The knowledge lib turns the user's illuminated captures into a structured, deduped, resolved and
enriched graph of entities.

```
Capture ──illuminate──▶ Illumination ──▶ Observation ──link──▶ Entity ──resolve──▶ Evidence
```

Every clause of that line has a home in this directory, and everything else is detail.

The leftmost step is not ours: **illumination** — the model reading an image into a summary plus the
entities it believes are present — lives in the sibling `src/illumination/` lib, and its vocabulary
is described in `plan/lexicon.md`. (It has no `illumination.md` yet. When it does, this file should
link to it rather than paraphrase it.)

Captures, their illuminations, and their entity observations come from the user
(with a little help from inference). From these raw inputs, the knowledge lib
construct, maintains, evolves, and enriches a common knowledge graph shared by
all users.

## Observations: what the user saw

An observation is one thing one capture named — a name, a type, a description, sometimes a platform
account. It is the Illuminator's best effort, recorded **verbatim and never rewritten**, and it lives
in `user_observed_entities`.

Two properties matter more than the columns:

- **It belongs to one user.** Observations are personal.
- **It stands alone.** `knowledge_entity_id` is null until something links it, and `reset` can drop
  every entity and every edge without touching a single observation. Nothing is lost, because nothing
  was ever edited.

That is also why history is safe to keep: an illumination can be wrong — a re-read of one image once
described an entirely different film — so the graph holds **every** reading of every capture. The
question of which reading a user *sees* is answered later, in "The view boundary".

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
is **projected**: every derived column is a function of the entity's observations and its evidence, so
a merge or a fresh link simply recomputes them. `project()` is the only writer of `name`, `alt_names`,
`entity_type`, `alt_types`, `description`, `alt_descriptions`, `status` and `confidence`.

```
name / alt_names            the chosen surface, and every other one seen
entity_type / alt_types     the chosen type, and every other one asserted
description / alt_*         same shape: one selected, the rest kept for matching
status                      provisional → identified → contested
merged_into_id              set when this entity was folded into another
```

A **surface** is just a string the thing has been called: its name, a spelling variant, a display
name, a handle. The `alt_*` columns are not clutter — they are **the complement of the selected
value**, and they are what lets a later observation find this entity by a name it was once seen
under.

## Linking: are these the same thing?

For each observation that has not been linked yet, linking asks one question: **have we already got
this thing?** If the answer is yes, the observation attaches to that entity and enriches it; if not,
a new entity is created. Getting that wrong in either direction is the whole risk — one entity split
in two, or two things fused into one — so the decision is separated from everything around it.

Linking is offline, pure, and the only place that decision is made. It has three moving parts, and
the first two are traits precisely so a second implementation can exist.

```rust
trait Blocker {                  // recall: find candidates, over-generate
    fn index(&mut self, entity: &EntityView);
    fn candidates(&self, observation: &ObservationView) -> Vec<i32>;
}

trait Linker {                   // precision: decide, and nothing else
    fn decide(&self, observation: &ObservationView, candidates: &[&EntityView]) -> Decision;
}

enum Decision {
    Attach { entity: i32, score: f32 },
    Create { best: Option<(i32, f32)> },
}
```

Both traits see **views** — `ObservationView` and `EntityView` — which are plain values cloned out of
models. No database, no network, no async. That is what makes a decision reproducible from a fixture.

`link.rs` is the executor: load the views, drive them through the blocker and linker, apply the
effects. Everything that is *our guess* — normalised name keys, weighted features, the bar a
candidate has to clear — lives in `heuristic.rs` and nowhere else. Delete that file and the graph
still compiles; it simply has no policy.

## Resolving: what can the authorities tell us?

Resolving is the opposite kind of work: network, cached, and only ever through one door.

```rust
trait Resolver {
    fn id(&self) -> &'static str;
    fn plan(&self, entity: &EntityView) -> Vec<Request>;      // what I would ask
    fn parse(&self, entity: &EntityView, request: &Request, raw: &Raw) -> Vec<Claim>;  // what it means
    async fn fetch(&self, requests: &[Request]) -> Result<Vec<(String, Raw)>, FetchError>;
}
```

`plan` and `parse` are pure, so the whole decision can be re-run over cached payloads for free.
`fetch` is the only network call in the system, and only the executor calls it — a provider cannot
forget the cache, because there is no other way to reach the network.

Resolvers **accumulate**. Wikidata supplies a QID, and a second resolver might supply an IMDb id;
they answer different questions, so both are kept. The promise is not "the answer" but *independent
external evidence that this identity is true*.

Run in two stages: identity first (exact label, handle), then a search fallback only for what the
first stage missed.

## Evidence: why we believe it

Claims live in `knowledge_evidence`, append-only, with provenance:

```
claim       this entity has this identity      (source, method, confidence)
collision   a claim we declined; another entity holds it
merge       this entity was folded into that one
enrichment  the authority's description, names, types      (reserved)
rejection   a candidate considered and refused             (reserved)
```

A partial unique index does the hard part: no two entities may hold a *live* claim to the same
`(scheme, id)`. Retracting a claim (`retracted_at`) frees the slot — retraction is a fact with a
timestamp, not a status.

## The view boundary

The graph keeps every reading; a user sees one.

A capture's **current** illumination is its highest id, and every user-facing read starts there and
left-joins into entities (`knowledge::current`). Two consequences fall out for free:

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
  not the capture — re-reading an image is a separate reading.

## Where things live

| file | what it is |
|---|---|
| `view.rs` | the plain values strategies see |
| `blocking.rs`, `linker.rs` | the two traits, and nothing else |
| `heuristic.rs` | **all** of our guesswork: features, weights, bars, `NameBlocker`, `ScorerLinker` |
| `link.rs` | the linking executor |
| `resolve/driver.rs` | the resolver contract, the cache-first driver, claim storage |
| `resolve/wikidata/` | one authority: policy in `wikidata.rs`, HTTP in `transport.rs` |
| `project.rs` | the only writer of an entity's derived columns |
| `merge.rs`, `reset.rs`, `compare.rs` | repair and the tooling around it |
| `current.rs` | which reading a user sees |
| `cache.rs` | answers, keyed by `(provider, kind, shape, query)` |
| `normalize.rs` | the one definition of "the same name" |

The vocabulary — and the names we rejected — is in `plan/lexicon.md`. The design history, including
the failures that produced these rules, is in `plan/knowledge-illumination-history.md` and
`plan/knowledge-canonical-entities.md`.
