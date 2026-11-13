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
`platform`, `platform_handle`, `platform_display_name`, `status` and `confidence`.

```
name / alt_names            the chosen surface, and every other one seen
entity_type / alt_types     the chosen type, and every other one asserted
description / alt_*         same shape: one selected, the rest kept for matching
search_query / alt_*        the disambiguation hint, projected the same way
platform / platform_*       the presence one member carried, so the resolver can ask by handle
confidence                  the best identity's score; null when no identity has one
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
image, a coherent set of the things that image showed. An observation is a part of that reading,
not an independent event.

```rust
trait Blocker {                  // recall: find candidates, over-generate
    async fn candidates(&self, observation: &Observation) -> Vec<Entity>;
}

// `&self`, so the graph holds one `Arc<dyn Blocker>` and no lock. The SQL blocker keeps no state to
// invalidate: it reads the index every projection already updates.

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
    vetoed: usize,                 // identity annotations a standing decision refused
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
the effects: decisions first, writes second. It knows nothing about identities — the observation's
handle is annotated afterwards, by `helper::write_observation_identities` (`add_identity`, then a
second projection), once the observations have homes. Every decision is made before any is applied,
so the illumination's own annotations are invisible to its own decisions: two observations that name
one previously unknown handle are both told to create, and become two entities annotated with one
identity. `merge` refuses that pair (shared illumination), so the scan records it once and it stays
open — the terminal state the merge-side rule produces everywhere.

**The heuristic is one implementation of that chain — not the chain.** The chain is `scoring.rs`, and
it is three verbs' worth of contract:

- `Similarity` measures one pair: a corpus-free number, never a verdict.
- `Policy` measures a *set*: the evidence for each candidate, which of them qualifies, and where each
  sorts. The set is the corpus, which is why a corpus-aware measure cannot be a `Similarity`.
- `Rank` is `Policy` in the form the store holds — qualifying candidates, best first — and it is
  written once, as a blanket implementation, instead of once per policy.

The filter is the policy's, and it is not a threshold on the score: the rule is a conjunction — a
name match has to be corroborated — and a number cannot carry its own components. That is also why
a rejected pair still reports its score: `Nintendo [organization] ⇄ Nintendo [brand]` at 0.74 is a
pair that must never merge, and the number is how a person sees why.

Everything else is not a verb. The scan stores the number and `merge` asks `Rank` for the verdict; the
linker asks `Rank` and applies the answer; `project` takes the best identity's confidence; `merge` and
the holder order ask `TotalCmp`, a convention about `NULL`. The linker **holds** its
`Scoring`; the proximity passes are **handed** one; the store holds none. Choosing one policy for both
sides is the composition root's job — it builds the linker and a `Scoring` from the same policy, and
passes that to `proximity scan` and `auto-merge` — because a store that owned one would let a caller
swap the merge side without swapping the link side, silently.

`linker/heuristic/scoring.rs` is today's only answer: normalized keys, weights, and the merge rule. `linker/heuristic/linker.rs` is the **procedure** — a handle the graph already holds settles the
observation before any scoring, else the best qualifying candidate, else create — and holds the policy it applies, but no numbers of its own. `blocker/sql.rs` is recall:
every folded surface and its word-order-insensitive form, probed out of `knowledge_blocking_index` —
nothing scored. A platform handle is a surface too, so a capture that named an account only by its
handle still finds the entity another capture named by hand. The
resolver keeps its own measure and its own calibration, because its candidate type is private and
its fit is corpus-aware (terms are weighed by rarity among the answers it got back) — a pair
comparison cannot see the corpus.

That is what makes a second policy possible without a second graph: implement `Policy` for the two
pairs the graph asks about plus `Similarity` for the stored number, build a `Scoring`, and hand it to
the `HeuristicLinker` and the passes that measure.

## Resolving: what can the identity resolvers tell us?

Resolving is the opposite kind of work: network, cached, and only ever through one door.

```rust
trait Resolver {                              // src/knowledge/identity
    fn id(&self) -> &'static str;
    async fn resolve(&self, entities: &[Model], gateway: &fetch::Gateway<'_>)
        -> Vec<(i32, Identity)>;              // this entity's identities
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

How a resolver gets to its identities is its own business. `Wikidata` climbs one step at a time — handle,
title, label, search hint, search label, surface title — and each step asks only the entities still
unresolved, batched across them. A title that resolves an entity drops its label and search questions.
That shape is internal to the provider, not vocabulary the engine knows about. `Kind` is the
*provider's* enum, so **adding an identity resolver means writing one `Resolver` plus one `Transport`**, and a
resolver is free to make one request or a dozen.

The one thing a resolver cannot do is fetch. Its only route to the world is
`Gateway::answers_for`, which consults the cache first, charges the budget and paces the call —
which is why a resolver can own its own strategy without being able to bypass any of them. The door
is `crate::fetch` and knows nothing about knowledge entities at all; see `src/fetch/fetch.md`. (An
agentic resolver, whose tool calls happen inside a provider's runtime, is the one exception: it uses
the same gateway by convention, and it is expected to fail at that rather than to pretend.)

Identities **accumulate**. Wikidata supplies a QID, a capture's handle sits beside it as a separate
annotation, and a second identity resolver may assert the same or another key. The engine selects candidates
per resolver, replaces that provider's set for every entity it answered about, and applies every
identity resolver before projecting the entity. Confidence is the best annotation's score; one without a
score contributes nothing. An abstention is not an answer: an entity the resolver returned nothing for
keeps its old annotations.

`plan` and `decision` inside `Wikidata` are pure, so its whole decision can be re-run over cached
payloads for free — that is what the resolver's tests and `identity inspect` do.

## Evidence: why we believe it

Three tables, because they answer three different questions:

```
knowledge_identities   an annotation on one entity: scheme + external id, and the answer its
                       provider gave (label, description, classes, method, confidence);
                       provider is a resolver id, `illuminator`, or `model`
knowledge_proximities  two entities the graph considers close, with the score that a pass
                       computed — one scan's batch, replaced wholesale by the next
knowledge_verdicts     a person's decision, and what decided it — the modification log: a
                       `merge` (two origins), a `veto_identity` (an entity is not an
                       identity), or a `distinct` (two origins are deliberately separate)
```

An identity annotation belongs to one entity. Its durable name is the pair `(scheme, external_id)`;
the row is one provider's statement of it. Two providers may assert one key for one entity — that is
corroboration with its own confidence and method — and two entities may hold one key, which is what
`proximity` asks about. The uniqueness is `(entity, provider, scheme, external_id)`: one provider
annotates an entity with a key once, and replacing that provider's set is one delete and one insert.

A proximity is a judgement about our own rows and a merge is a mutation of them. `enrichment` and
`rejection` stay reserved vocabulary, not tables.

Sharing an identity is a **candidate**, not a verdict. `proximity` asks the question: for each key
with two or more holder entities, score the holders pairwise with the `Scoring` the pass is handed,
fold the pairs that look like one thing, and queue the ones the shared-illumination rule refuses.

`proximity` runs in two passes. **Scan** derives the candidate set and replaces the table with
it in one transaction, so what it holds is always what the graph looked like when it was last asked
— a scan after a link, and again at the end of a resolve. **Auto-merge** answers the batch: it
re-checks that both sides still exist, scores the pair with the `Scoring` it was handed and folds the
convincing ones. The rest stay in the batch, which is where a person reads them.

Nothing in that table is a status or a verdict. A pair that keeps looking like one thing keeps
reappearing; one that stops, stops. Human decisions that must outlive a rebuild go somewhere
durable instead — `plan/archive/knowledge-verdicts.md` — and are replayed onto the new graph.

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

## Rebuild and repair

Entities are cheap to rebuild and expensive to get wrong, so the model prefers undoable moves. A
rebuild is six commands, each one job, none invoking another:

```sh
reset -y                    snapshot the projection, then empty it
link                        observations → entities
verdicts replay             a person's recorded folds, applied to the new ids
identity resolve --offline  evidence: identity annotations from cached answers
proximity scan              derive the candidate batch from shared identities
proximity auto-merge --yes  answer the batch (dry run without --yes)
```

The order is not a preference:

- **`link` first**, because everything else needs entity rows.
- **`replay` before `identity resolve`**, two reasons. `proximity auto-merge` picks a survivor by the
  projected `confidence`, and `replay` reports a pair as `already`/`gone` rather than reversing a
  direction — so if the automatic pass folded a decided pair the other way first, the recorded winner
  is lost and there is no unmerge. And a folded entity is resolved *once*, from the union of both
  sides' surfaces, instead of twice and then shuffled by a merge.
- **`scan` after `identity resolve`**, because its basis is a shared identity: measured on the local
  corpus, 0 of 345 platform identities have more than one holder, while 13 Wikidata keys account for
  all 26 multi-holder annotations. A scan before resolution raises nothing.
- **`auto-merge` after `scan`**, because it answers the batch the scan wrote.

`identity resolve` reprojects every entity it gives an identity to, before returning: a later
`auto-merge` reads that confidence, so leaving it stale would fold the wrong side.

**A rebuild is a single-writer operation. Stop the link worker first.** Running these commands
against a live queue has produced both failures it can produce: a link task attaching to an entity a
`reset` had just deleted (the illumination is left permanently unlinked), and a `proximity scan`
taken from a half-built generation (53 rows where the finished graph supports 13). Neither is a bug
in the command that reported it; both are two writers.

Nothing else writes the batch. The live pipeline is `illuminate` → `knowledge_link`, and the link
task links and stops; there is no resolve task, so resolution and both proximity passes are operator
commands. A live capture updates entities immediately and leaves the proximity batch untouched until
the next `proximity scan` — the table is a snapshot of when it was last asked.

`reset` snapshots the projection, then empties it — entities, identities, proximities and the index
rows, all of it derived. It keeps the newest **ten snapshot suffixes** and drops older
archive tables; the generation register keeps its history even after a suffix is pruned.
Observations, captures, illuminations, the merge log (`knowledge_verdicts`) and the **fetch cache**
are untouched. `merge` folds one entity into another, tombstones the loser, and runs in a
**transaction** — five writes that must all land. The **shared-illumination rule**: `merge` refuses
two entities that both appear in one illumination, because a single extraction emitting both means
two things. The unit is the illumination, not the capture — re-illuminating an image makes a separate
one.

## Where things live

| file | what it is |
|---|---|
| `scoring.rs` | the verbs — `Similarity` for a pair, `Policy` for a set with `Rank` derived from it — plus `Order`, `TotalCmp` and the `Scoring` bundle. The linker holds one; the proximity passes are handed one |
| `normalize.rs` | the two key generations: `comparable` (the fold) and `word_key` (words sorted, one leading article dropped), plus `norm_fingerprint` — the behavioural stamp they are versioned by. Shared by the blocker, identity, evidence and projection |
| `blocker/mod.rs` | the `Blocker` trait: which entities are worth considering, and nothing else |
| `blocker/sql.rs` | `SqlBlocker`: recall by folded surface and word key, one indexed probe per candidate query |
| `blocker/index.rs` | `knowledge_blocking_index`: the folded forms a blocker matches on — `rebuild`, `check`, `write`, `forget` |
| `linker/mod.rs` | the `Linker` trait: one batch of observations in, decisions out. A `&Scoring` argument is deliberately absent — the implementation holds its policy. Everything upstream is recall, everything downstream is effect; the caller decides how the batch was assembled |
| `linker/heuristic/linker.rs` | `HeuristicLinker`: the procedure over the heuristic's `Scoring` — best candidate that clears the bar, else create |
| `linker/heuristic/scoring.rs` | the only `Scoring` today: weights, the merge rule (a name match plus one corroborating signal), and the best-identity confidence |
| `graph/link.rs` | `link`: a batch of observations in, decisions applied and projected out — plus the `observation → entity` mapping and `LinkReport`. It knows nothing about identities |
| `snapshot/` | take a copy of the projection, empty it, diff two copies |
| `evidence/` | the vocabulary — `PlatformLink`, `IdentityKey`, `Identity` — and the write path in `store.rs`: `replace_provider_identities` rewrites a provider's set for one entity, `add_identity` appends a capture's handle, `query_identities` and `query_entities_for_identity` answer the reads. `store.rs` also owns `move_identities` on a fold and `delete_identity` on a veto; `selection.rs` owns the candidate reads (`awaiting_authority`, `annotated_by`) |
| `graph/knowledge_graph.rs` | the facade: `KnowledgeGraph`, its constructors (`new`, `with_linking`, `with_heuristic_linking`) and the operations that own a write path — link, merge, replay, reproject, the veto commands. It holds the connection, the blocker, the decider, the evidence and the verdict register — no policy of its own |
| `graph/replay.rs` | the verdict effects: `replay` (merges, then withdrawals) and the withdrawal the veto command shares |
| `graph/project.rs` | `project`: the only writer of derived columns — including the entity's `knowledge_blocking_index` row — with `gather`/`select`/`surfaces`; confidence is the best identity's score. `reproject` repairs a scope of entities, with `Stale` the "live, no identity annotation" rule |
| `graph/merge.rs` | fold one entity into another |
| `proximity/` | the derived batch of look-alike pairs, and the two passes: `mod.rs` is declarations and the `Basis` vocabulary; `scan.rs` derives the batch (one transaction replaces the table); `auto_merge.rs` folds what it can on its own and leaves the rest. Both take the connection and the `Scoring` as arguments |
| `verdicts/` | the register: the modification log and the standing vetoes, held by `KnowledgeGraph`. `store.rs` is the type, its connection, `record` (the one write that takes the caller's transaction, so a merge's row commits with the fold) and the reads the replay and `stats` share; `veto.rs` holds `veto`/`unveto`/`vetoed`; `origin.rs` turns an origin into the entity it names now. Applying the log is the graph's command (`graph/replay.rs`), exposed as `KnowledgeGraph::replay_verdicts`; `pair_key` is shared with `stats` |
|`identity/`|`mod.rs` (module declarations) · `resolution` (the `Resolver` trait a provider implements, the run loop, candidate selection, vetoes) · `wikidata/` (today's only identity resolver). The identities it returns, and where they become rows, are `evidence` — the cache, the pace and the budget are `crate::fetch`|
| `identity/wikidata/` | one identity resolver, entirely in its own vocabulary: `kind` (its request kinds) and `candidate` (the item it offers) · the ladder in `resolver.rs` · request planning in `plan.rs` · query text in `query.rs` · a platform link turned into the property and value form to ask (`property_query.rs`) · shape keys in `shape.rs` · a decided candidate turned into the stored annotation, and the menu rule (`answer.rs`) · the single entry, the gates and the per-rung admission in `decision.rs` · candidate scoring in `ranking.rs`, as a `Policy` over the answer's candidates · class filtering in `types.rs` · HTTP in `transport.rs` |
|`../fetch/`|the only door to a third party — see `src/fetch/fetch.md`: `transport` (the trait, and the request and answer it speaks, generic over a provider's kind and payload) · `cache` (answers keyed by `(provider, kind, shape, query)`, and their TTLs) · `stats` (a run's counters, and the table's tally) · `gateway` (cache first, then a paced, budgeted fetch)|
| `identity/inspect.rs` | `identity inspect`: walks one entity through blocking, ranking and the ladder, read-only |
| `eval/` | offline diagnostics: `eval blocker` measures recall |

The three write paths that carry their own logic and tests (`link.rs`, `project.rs`, `merge.rs`) are
reached through `KnowledgeGraph`'s methods; `link` and `project` are private modules, and what
escapes them is data and vocabulary — `LinkReport`, `merge::Outcome`, `Preview`, and the verdict
`Cause`. Passes over *derived* data are public functions taking what they act on —
`proximity::scan(conn, scoring)` and `proximity::auto_merge(conn, graph, scoring)` — while operations
over the graph's own state are methods: `link`, `merge`, `replay_verdicts`, which applies the log to a
rebuilt graph. The graph's health counters are a report over the tables rather than graph work, so
they live in `crate::stats::knowledge_stats`.

The vocabulary — and the names we rejected — is in `plan/lexicon.md`. The design history, including
the failures that produced these rules, is in `plan/archive/knowledge-illumination-history.md` and
`plan/archive/knowledge-canonical-entities.md`.
