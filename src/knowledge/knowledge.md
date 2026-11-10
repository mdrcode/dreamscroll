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
a merge or a fresh link simply recomputes them. After `create`, `update_projection()` is the only writer of `name`, `alt_names`,
`entity_type`, `alt_types`, `description`, `alt_descriptions`, `search_query`, `alt_search_queries`,
`status` and `confidence`.

```
name / alt_names            the chosen surface, and every other one seen
entity_type / alt_types     the chosen type, and every other one asserted
description / alt_*         same shape: one selected, the rest kept for matching
search_query / alt_*        the disambiguation hint, projected the same way
platform / platform_*       the presence one member carried, so the resolver can ask by handle
confidence                  best claim's score + 0.1 per independent input (text, handle)
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

trait Linker {              // the procedure: decide, and nothing else
    // The policy is a member of the implementation, not an argument.
    async fn decide_link(
        &self,
        blocker: &dyn Blocker,
        evidence: &Evidence,
        observations: &[Observation],
    ) -> Result<Vec<Decision>>;
}

struct Decision {
    observation: Observation,      // the row itself, not its id
    action: Action,
}

enum Action {
    Attach(i32),                   // this entity
    Create,
}

struct LinkReport {                 // what one illumination's decisions did
    attached: usize,
    created: usize,
    left_pending: usize,           // pending observations the linker did not decide
    vetoed: usize,                 // claims a standing decision refused
}
```

**A batch, so a reading is decided as one.** Every observation of an illumination is decided before
any of them is applied, so the decisions share one view of the corpus and none of them can see the
others' writes. Two consequences follow, and neither is a rule the linker has to enforce: a new name
cannot fold into a sibling's new entity, because nothing has been created yet; and two observations
that name one existing entity both attach to it, because nothing is set aside between them.
Siblings that arrived already linked are context, not work — no decision is owed for them.

Both traits take **model rows** — `user_observed_entity::Model` and `knowledge_entity::Model` —
because a separate view layer was a second place for a column to go missing, and it went missing
twice. What a strategy must not do is *write* one: `update_projection` is the only writer of a derived column.
The derived forms the scorer works with (`surfaces()`, `descriptions()`, `types()`) are inherent
methods on the model, beside its columns.

`link.rs` is the executor. It loads the rows, drives them through the blocker and linker, and applies
the effects: decisions first, writes second. It knows nothing about identities or claims — the
capture's handle is asserted afterwards, by `helper::assert_claims` (`identity_for_link`, then
`assert`, then a second projection), once the observations have homes. Every decision is made before
any is applied, so the illumination's own claims are invisible to its own decisions: two observations
that assert one previously unknown handle are both told to create, and become two entities holding
one identity. `merge` refuses that pair (shared illumination), so the scan records it once and it
stays open — the terminal state the merge-side rule produces everywhere.

**The heuristic is one implementation of that chain — not the chain.** The chain is `scoring.rs`, and
it is two verbs:

- [`Similarity`] measures one pair: how strongly the evidence says these two are one thing.
- [`Rank`] says which of a candidate set is *this*, best first — and only the ones worth choosing.

The filter is the second verb's, and it is not a threshold on the score: the rule is a conjunction —
a name match has to be corroborated — and a number cannot carry its own components. That is also why
a rejected pair still reports its score: `Nintendo [organization] ⇄ Nintendo [brand]` at 0.74 is a
pair that must never merge, and the number is how a person sees why.

Everything else is not a verb. The scan stores the number and `merge` asks `Rank` for the verdict; the
linker asks `Rank` and applies the answer; `project` calls `combine`, which is arithmetic over claim
scores; `merge` and the holder order ask `TotalCmp`, a convention about `NULL`. The linker **holds** its
`Scoring`; the proximity passes are **handed** one; the store holds none. Choosing one policy for both
sides is the composition root's job — it constructs the linker and passes the same `Scoring` to `scan`
and `merge` — because a store that owned one would let a caller swap the merge side without swapping
the link side, silently.

`linker/heuristic/scoring.rs` is today's only answer: normalized keys, weights, the merge rule, the
corroboration bonus. `linker/heuristic/linker.rs` is the **procedure** — the best qualifying candidate,
else create — and holds the policy it applies, but no numbers of its own. `blocker/sql.rs` is recall:
every folded surface and its word-order-insensitive form, probed out of `knowledge_blocking_index` —
nothing scored. A platform handle is a surface too, so a capture that named an account only by its
handle still finds the entity another capture named by hand. The
resolver keeps its own measure and its own calibration, because its candidate type is private and
its fit is corpus-aware (terms are weighed by rarity among the answers it got back) — a pair
comparison cannot see the corpus.

That is what makes a second policy possible without a second graph: implement the two verbs for the
pairs the graph asks about, build a `Scoring`, and hand it to the `HeuristicLinker` and to the passes
that measure.

## Resolving: what can the authorities tell us?

Resolving is the opposite kind of work: network, cached, and only ever through one door.

```rust
trait Resolver {                              // src/knowledge/identity
    fn id(&self) -> &'static str;
    async fn resolve(&self, entities: &[Model], gateway: &fetch::Gateway<'_>)
        -> Vec<(i32, Claim)>;                 // claims, each tagged with its entity
}

// src/fetch: generic over the provider's own vocabulary, so it can hold none of ours.
trait Transport {
    type Kind;                                // the provider's request kinds
    type Payload;                             // the provider's answer body
    async fn fetch(&self, requests: &[Request<Self::Kind>])
        -> Result<Vec<(String, Answer<Self::Payload>)>, FetchError>;
}
```

`resolve` takes a *batch*, not one entity: batching across entities is the difference between a
hundred transport calls for a corpus walk and five thousand, and only the resolver knows how to
group its own requests.

How a resolver gets to its claims is its own business. `Wikidata` uses a ladder — handles, titles
and exact labels first, then full-text search, then the bare title as a last resort — but that shape
is internal to it, not vocabulary the engine knows about. `Kind` is the *provider's* enum, so
**adding an authority means writing one `Resolver` plus one `Transport`**, and a resolver is free to
make one request or a dozen.

The one thing a resolver cannot do is fetch. Its only route to the world is
`Gateway::resolve_requests`, which consults the cache first, charges the budget and paces the call —
which is why a resolver can own its own strategy without being able to bypass any of them. The door
is `crate::fetch` and knows nothing about knowledge entities at all; see `src/fetch/fetch.md`. (An
agentic resolver, whose tool calls happen inside a provider's runtime, is the one exception: it uses
the same gateway by convention, and it is expected to fail at that rather than to pretend.)

Resolvers **accumulate**. Wikidata supplies a QID, and a second resolver might supply an IMDb id;
they answer different questions, so both are kept. The promise is not "the answer" but *independent
external evidence that this identity is true*.

`plan` and `parse` inside `Wikidata` are pure, so its whole decision can be re-run over cached
payloads for free — that is what `dreamscroll_knowledge eval resolve` does.

## Evidence: why we believe it

Four tables, because they answer four different questions:

```
knowledge_identities   the thing the world names: scheme + external id, and the answer
                       whoever gave it (label, description, classes, method, confidence)
knowledge_claims       this entity is that thing — the edge, with `basis` (text or handle)
                       naming what the question was; several entities may claim one identity,
                       because an album's article covers its songs
knowledge_proximities  two entities the graph considers close, with the score that a pass
                       computed — one scan's batch, replaced wholesale by the next
knowledge_verdicts     a person's decision, and what decided it — the modification log: a
                       `merge` (two origins) or a `veto` (an entity is not an identity)
```

Only the first two are about the world. A proximity is a judgement about our own rows and a merge is a
mutation of them. `enrichment` and `rejection` stay reserved vocabulary, not tables.

Sharing an identity is a **candidate**, not a verdict — that is why the uniqueness lives on the edge
(one entity asserts an identity once) rather than on the identity. `proximity` asks the question:
for each identity with two or more holders, score the holders pairwise with the `Scoring` the pass is
handed, fold the pairs that look like one thing, and queue the ones the shared-illumination rule
refuses.

The only uniqueness is per entity: one live claim per `(entity, identity)`, so retracting frees that
pair's slot and re-asserting it is a no-op. Nothing forbids two entities holding one identity — that is
the many-to-one the model exists for.

`proximity` runs in two passes. **Scan** derives the candidate set and replaces the table with
it in one transaction, so what it holds is always what the graph looked like when it was last asked
— a scan after a link, and again at the end of a resolve. **Auto-merge** answers the batch: it
re-checks that both sides still exist, scores the pair with the `Scoring` it was handed and folds the
convincing ones. The rest stay in the batch, which is where a person reads them.

Nothing in that table is a status or a verdict. A pair that keeps looking like one thing keeps
reappearing; one that stops, stops. Human decisions that must outlive a rebuild go somewhere
durable instead — `plan/knowledge-verdicts.md` — and are replayed onto the new graph.

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

- `reset` snapshots the projection, then empties it — entities, claims, identities, proximities and
  the index rows, all of it derived. Observations, captures, illuminations, the merge log
  (`knowledge_verdicts`) and the **fetch cache** are untouched. The teardown is therefore free: `reset`, then `link`, then
  `verdicts replay` (a person's folds, applied to the new ids), then `identity resolve --offline`
  (identities and claims from cached answers), then `proximity scan` and `proximity auto-merge`. Six
  commands, each one job, none invoking another — the order and its reasons are in
  `plan/knowledge-blocking.md` §Rebuild order. No transport, seconds.
- `merge` folds one entity into another, tombstones the loser, and runs in a **transaction** — five
  writes that must all land.
- The **shared-illumination rule**: `merge` refuses two entities that both appear in one
  illumination, because a single extraction emitting both means two things. The unit is the
  illumination, not the capture — re-illuminating an image makes a separate one.

## Where things live

| file | what it is |
|---|---|
| `scoring.rs` | the two verbs — `Similarity`, `Rank` — `TotalCmp`, and the `Scoring` bundle. The linker holds one; the proximity passes are handed one |
| `normalize.rs` | the two key generations: `comparable` (the fold) and `word_key` (words sorted, one leading article dropped), plus `norm_fingerprint` — the behavioural stamp they are versioned by. Shared by the blocker, identity, evidence and projection |
| `blocker/mod.rs` | the `Blocker` trait: which entities are worth considering, and nothing else |
| `blocker/sql.rs` | `SqlBlocker`: recall by folded surface and word key, one indexed probe per candidate query |
| `blocker/index.rs` | `knowledge_blocking_index`: the folded forms a blocker matches on — `rebuild`, `check`, `write`, `forget` |
| `linker/mod.rs` | the `Linker` trait: one batch of observations in, decisions out. A `&Scoring` argument is deliberately absent — the implementation holds its policy. Everything upstream is recall, everything downstream is effect; the caller decides how the batch was assembled |
| `linker/heuristic/linker.rs` | `HeuristicLinker`: the procedure over the heuristic's `Scoring` — best candidate that clears the bar, else create |
| `linker/heuristic/scoring.rs` | the only `Scoring` today: weights, the merge rule (a name match plus one corroborating signal), the corroboration bonus |
| `graph/link.rs` | `link`: a batch of observations in, decisions applied and projected out — plus the `observation → entity` mapping and `LinkReport`. It knows nothing about identities or claims |
| `snapshot/` | take a copy of the projection, empty it, diff two copies |
| `evidence/` | the vocabulary — `Claim`, `ClaimBasis`, `ClaimDraft`, `Stored` in `claim.rs`; `IdentityKey` (with its capture constructor) and `DraftIdentity` in `identity.rs` — and the two phases of a claim in `store.rs`: `identity_for_link` settles a capture's identity (create only) and `upsert_identity` an answer's (fill gaps); `assert` writes the edge, asking the veto at the one place a claim becomes a row. `store.rs` also owns the other two changes to the table, `move_claims` on a fold and `retract` on a veto, and the reads (`held_by_identity`, `awaiting_authority`) |
| `graph/knowledge_graph.rs` | the facade: `KnowledgeGraph`, its constructors and the methods that are one write each. It holds the connection, the blocker, the decider and the evidence — no policy of its own |
| `helper.rs` | the illumination composition: batch one reading's observations, call `KnowledgeGraph::link`, then assert each observation's handle and project again. The graph links a collection; only this knows it is one reading |
| `graph/project.rs` | `project`: the only writer of derived columns — including the entity's `knowledge_blocking_index` row — with `gather`/`select`/`surfaces`; confidence is `combine`, arithmetic over claim scores |
| `graph/merge.rs` | fold one entity into another |
| `proximity/` | the derived batch of look-alike pairs, and the two passes: `mod.rs` is declarations and the `Basis` vocabulary; `scan.rs` derives the batch (one transaction replaces the table); `auto_merge.rs` folds what it can on its own and leaves the rest. Both take the connection and the `Scoring` as arguments |
| `verdicts/` | the register: the modification log and the standing vetoes, held by `KnowledgeGraph`. `store.rs` is the type, its connection, `record` (the one write that takes the caller's transaction, so a merge's row commits with the fold) and the reads the replay and `stats` share; `veto.rs` holds `veto`/`unveto`/`vetoed`; `origin.rs` turns an origin into the entity it names now. Applying the log is the graph's command (`graph/replay.rs`), exposed as `KnowledgeGraph::replay_verdicts`; `pair_key` is shared with `stats` |
|`identity/`|`mod.rs` (the `Resolver` trait a provider implements) · `engine` (the run loop, candidate selection, vetoes) · `wikidata/` (today's only authority). The claim it returns, and where it becomes a row, are `evidence` — the cache, the pace and the budget are `crate::fetch`|
| `identity/wikidata/` | one authority, entirely in its own vocabulary: `kind` (its request kinds) and `candidate` (the item it offers) · the gates in `policy.rs` · answer parsing in `answer.rs` · the ladder and its request shapes in `resolver.rs` · candidate scoring in `ranking.rs` · HTTP in `transport.rs` |
|`../fetch/`|the only door to a third party — see `src/fetch/fetch.md`: `transport` (the trait, and the request and answer it speaks, generic over a provider's kind and payload) · `cache` (answers keyed by `(provider, kind, shape, query)`, and their TTLs) · `stats` (a run's counters, and the table's tally) · `gateway` (cache first, then a paced, budgeted fetch)|
| `eval/` | offline evaluation: `eval resolve` replays the resolver's ladder over the answer cache, with no transport |

The three write paths that carry their own logic and tests (`link.rs`, `project.rs`, `merge.rs`) are
reached through `KnowledgeGraph`'s methods; `link` and `project` are private modules, and what
escapes them is data and vocabulary — `LinkReport`, `merge::Outcome`, `Preview`, and the verdict
`Cause`. Passes over *derived* data are public functions taking what they act on —
`proximity::scan(conn, scoring)` and `proximity::auto_merge(conn, graph, scoring)` — while operations
over the graph's own state are methods: `link`, `merge`, `replay_verdicts`, which applies the log to a
rebuilt graph. The graph's health counters are a report over the tables rather than graph work, so
they live in `crate::stats::knowledge_stats`.

The vocabulary — and the names we rejected — is in `plan/lexicon.md`. The design history, including
the failures that produced these rules, is in `plan/knowledge-illumination-history.md` and
`plan/knowledge-canonical-entities.md`.
