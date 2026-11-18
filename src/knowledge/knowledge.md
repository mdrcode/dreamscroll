# knowledge: mutations and their parameters

Status: v12, implemented in `graph/`, `helper.rs`, and `heuristic/`. Projection and the blocking index
run here. Same four tables. Proximity is a transient scan.

Callers emit mutations; one call is one transaction. No mutation contains a generation-scoped entity id,
so a list is independent of the generation it is applied to.

The graph does not care which code assembled the list, or who is calling. It stores the provenance
each mutation carries — `provider` on an identity, `cause` on decisions — and enforces the invariants
that provenance implies. Authentication is the edge's job.

## Shape

```
Link ─────┐
Resolve ──┤
Replay ───┼──▶ Vec<GraphMutation> ──▶ validate ──▶ apply ──▶ project
Agent ────┤                                        (one transaction)
User ─────┤
Operator ─┘
```

## Types

```rust
/// An observation id. Stable across every rebuild.
pub struct ObservationId(pub i32);

/// The observation that created an entity: `knowledge_entities.origin_observation_id`.
/// The executor looks it up, follows tombstones to the live row, and never uses the
/// observation's current membership.
pub struct EntityOriginId(pub ObservationId);

pub struct Provider(pub String);     // illuminator | wikidata | model | user | operator

pub struct IdentityKey { pub scheme: String, pub external_id: String }

pub struct Identity {
    pub provider: Provider,                     // who asserts it
    pub scheme: String,
    pub external_id: String,
    pub method: Option<String>,
    pub confidence: Option<f32>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub observation_id: Option<ObservationId>,  // provenance: the observation that carried it
}

/// Who decided. Stored on the row; the two classes lifting distinguishes.
pub enum Cause { Repair, Model }
```

An entity is named by its origin: the observation that created it. There is no `EntityId` in the
vocabulary; `EntityOriginId` is looked up against `origin_observation_id`, tombstones are followed,
and the live id never leaves the transaction. Field names carry the role — `observation` is the thing
under edit; `entity`, `subject`, `loser`, `winner`, `a`, `b` are `EntityOriginId`.

## The enum

```rust
pub enum GraphMutation {
    CreateEntity {
        from: ObservationId,
        with: Option<Identity>,                   // None = provisional
    },
    AttachObservation {
        observation: ObservationId,
        entity: EntityOriginId,
        with: Option<Identity>,                   // the identity this observation asserts, if any
        why: Option<String>,
    },
    DetachObservation {
        observation: ObservationId,
        why: Option<String>,
    },
    MergeEntity {
        loser: EntityOriginId,
        winner: EntityOriginId,
        cause: Cause,
        why: String,
    },
    MarkDistinct {
        a: EntityOriginId,
        b: EntityOriginId,
        cause: Cause,
        why: String,
    },
    Identify {
        subject: EntityOriginId,
        identity: Identity,
    },
    RetractIdentity {
        subject: EntityOriginId,
        provider: Provider,
        key: IdentityKey,
    },
    VetoIdentity {
        subject: EntityOriginId,
        key: IdentityKey,
        cause: Cause,
        why: String,
    },
    UnvetoIdentity {
        subject: EntityOriginId,
        key: IdentityKey,
        cause: Cause,
    },
}
```

`Identity` and `Cause` are explicit because they are the provenance the row stores. The mutation means
what it says without the call; the caller is not consulted to interpret it, and is not stored.

The graph never sees a platform link. The caller decides whether an observation's `platform_*`
columns form a `PlatformLink`, converts that link to an `Identity`, and passes it on the membership
mutation or as an `Identify`. The graph upserts exactly what it is handed; it has no `platform_link` call and
no `illuminator` special case.

## Per-mutation contract

- **`CreateEntity { from, with }`**
  - Accepts: the observation exists; it is currently unlinked; `with`, when present, has a non-empty
    provider, scheme and external id. `with: None` is the provisional entity: no annotations are
    written.
  - NoChange: already linked to an entity whose origin is `from` — the retry of a committed create.
  - Refuses: linked to any other entity, or another entity already claims `from` as its origin.
  - Does: insert the entity with `origin_observation_id = from`, point the observation at it; upsert
    `with` if present, filtering a vetoed `(entity, key)`; project. The graph derives no identity.
  - Logs: nothing yet. A `create` kind is open question 4.
  - Why: creation is one observation; later members arrive as `AttachObservation`.

- **`AttachObservation { observation, entity, with, why }`**
  - Accepts: the observation exists; the `entity` origin resolves to a live entity (one created
    earlier in this call is fine); `with`, when present, has a non-empty provider, scheme and
    external id.
  - NoChange: already linked to that entity.
  - Refuses: the observation does not exist; the origin does not resolve.
  - Does: set `knowledge_entity_id`; upsert `with` if present, filtering a vetoed pairing; project
    the source and target. It does not move or delete the source's existing annotations; the caller
    retracts those explicitly.
  - Logs: nothing yet. An `attach` kind is open question 7.
  - Why: membership and identity are independent; the caller names both.

- **`DetachObservation { observation, why }`**
  - Accepts: the observation exists.
  - NoChange: already unlinked.
  - Does: clear the link; project the old entity. It does not delete any annotation; the caller
    retracts it explicitly.
  - Logs: nothing yet. A `detach` kind is open question 7.

- **`MergeEntity { loser, winner, cause, why }`**
  - Accepts: both origins resolve to live entities; no standing `distinct` on their pair; no shared
    illumination.
  - NoChange: the two origins resolve to the same live entity.
  - Refuses: a standing `distinct`; a shared illumination.
  - Does: move observations, move identities (dropping duplicates), tombstone the loser, forget its
    block index row, log, project the winner. A folded origin resolves to its survivor, so a merge
    that names it folds the survivor.
  - Logs: `kind = merge`, the pair's canonical origins, `why` in `note`, the mutation's `cause`.

- **`MarkDistinct { a, b, cause, why }`**
  - Accepts: both origins resolve to live entities and are distinct; no pending repair merge on the
    pair.
  - NoChange: already distinct.
  - Does: insert the verdict. No graph row changes; projection is not needed.
  - Logs: `kind = distinct`, the origin pair, the mutation's `cause`.
  - One-way: knowledge 1 has no inverse; a wrong `distinct` cannot be lifted yet.

- **`Identify { subject, identity }`**
  - Accepts: the subject resolves to a live entity; non-empty provider, scheme and external id;
    `observation_id`, when present, exists.
  - NoChange: the pairing is vetoed — the identity is dropped, not refused, so a resolver call is not
    blocked by a person's decision.
  - Does: upsert `(entity, provider, scheme, external_id)` with the metadata and provenance; project
    the subject.
  - Logs: nothing. Identities are recomputable, and a resolver run replaces them.
  - Why: `Identity` carries the provider, so the uniqueness rule and the veto rule are readable at
    the mutation.

- **`RetractIdentity { subject, provider, key }`**
  - Accepts: the subject resolves.
  - NoChange: no row.
  - Does: delete that provider's row for the key; project the subject.
  - Logs: nothing. Distinct from `VetoIdentity`: no standing decision is recorded.

- **`VetoIdentity { subject, key, cause, why }`**
  - Accepts: the subject resolves to a live entity.
  - NoChange: already vetoed on that origin and key.
  - Does: insert the verdict, delete every provider's row for `(entity, key)`, project the subject.
  - Logs: `kind = veto_identity`, subject origin, `identity_scheme`, `identity_external_id`, the
    mutation's `cause` and `why`.

- **`UnvetoIdentity { subject, key, cause }`**
  - Accepts: the subject resolves; no veto row for the origin and key has a `cause` above this one.
  - NoChange: no veto row.
  - Does: delete every veto row for the origin and key. Nothing is restored; the next resolve may
    assert the annotation again.
  - Lifting rule: `Model` cannot lift a `Cause::Repair` row. The mutation states its own cause; the
    edge is responsible for not letting a model claim `Repair`.

## The planner

The v2 planner is a policy: the pending batch in, one `GraphMutation` per observation out.

- `Blocker` is recall: live entities only, ordered by id. An error is not an empty set; empty means
  "nothing matched, create," and a failed probe stops the link.
- `GraphSelector::holder` is the one evidence read: which live entity already carries
  `(scheme, external_id)`, named by origin.
- `heuristic::HeuristicLinker` is today's policy: a held identity wins; else the best qualifying
  blocker candidate; else create. The target is an `EntityOriginId`, never an entity id, so a
  decision made against an older corpus follows tombstones when it is applied.

The caller selects the batch. Already-linked observations are not the planner's problem; the helper
filters them before planning.

The heuristic lives in `knowledge::heuristic` with the measures it ranks with. It is the default
policy, not the interface: another planner — including an agentic one — can replace it without
touching the graph.

## Text

`text` is split by who needs it.

- **`text` is the graph's contract.** The blocking index stores the folded surface and the
  word-order-insensitive key; the SQL blocker probes with the same two; `norm_fingerprint` stamps
  which rule produced a row. Projection also uses the fold to dedupe a member's surfaces.
  - `comparable` — the folded surface.
  - `word_key` — the same words, order-insensitive.
  - `norm_fingerprint` — the index's currency stamp.
- **`heuristic::text` is for pluggable components.** Tokens, grams, Jaccard, near-surface and
  word-prefix are how the heuristic linker and Wikidata decide whether two texts mean the same thing. The graph's integrity does not depend on them.

If Link and Identify are replaced by LLM-driven flows that do their own text, the core keeps `text`;
`heuristic::text` goes with those components. Identity-key folding (`Identity::from_platform_link`, v1's
`IdentityKey::try_from`) is producer-side: the caller supplies canonical `(scheme, external_id)`
strings, and the graph stores them as given.

## Identification

`knowledge::wikidata` is the common authority layer: provider id, language, request kinds, SPARQL,
transport, parsed candidate and answer. It does not rank or write.

The shared identity primitive is `graph.selector().awaiting_identity(...)`: live entities a provider
has not answered, with the per-provider filter. An agentic provider uses that, the Wikidata core, and
the graph's `Identify`/`RetractIdentity` mutations directly.

`HeuristicIdentifier` is the automated provider; `IdentifyHelper` selects and batches, runs the
policy in `heuristic::identification` through the gateway, replaces the provider's set, and reports.
The policy is `plan`, `gather`, `measure`, `classes`, `ranking` and `decide`; it produces
`knowledge::Identity`.

The live link task and CLI `link` use `LinkHelper`; the live identification path uses
`IdentifyHelper`. Both the heuristic and a future agentic flow go through `knowledge::wikidata`;
only the policy half is replaceable. Removing `knowledge::heuristic` leaves the core, the producer
and the graph untouched.

## Replay

`knowledge::replay::replay` applies only durable repairs: the latest repair merge per unordered
origin pair, logged as `replayed`, then repair vetoes through `VetoIdentity`. It reads them through
`graph.selector().verdicts()` — `merges()` and `vetoes()` are repair-only, oldest first;
`vetoed_keys()` is every live veto keyed by the live entity, for admission and dry runs. Sides that
no longer map are reported as gone; a standing `distinct` or shared illumination is a refusal, never
a guess. `distinct` itself is consultative and needs no write.

## Corpus check

Pre-cutover, local corpus, `link` only on both engines (2,802 observations, 732 illuminations):

- v1: 2,336 created · 466 attached · 0 left pending.
- v2: 2,336 created · 466 attached · 0 left pending.
- The full shapes — entities, identities, memberships, blocking-index rows — diffed **empty**
  (2,336 / 354 / 2,802 / 2,336).
- The full v1 rebuild afterwards reproduced the pre-run graph exactly (2,336 entities, 2,060
  identities, 2,802 memberships, 2,322 index rows).
- Repeated after projection and the blocking index moved into v2: still an empty diff, and the v1
  restore was byte-identical again.

## Identities are caller-supplied

If an observation's platform columns should become an identity, the caller says so. The caller — today
the Link producer — extracts a `PlatformLink` from the observation; if its policy admits the handle,
it calls `Identity::from_platform_link` and passes the result on `CreateEntity` or
`AttachObservation`. An agent or operator may pass none.

- The admission rule lives with the caller. Today it is "any handle is an identity"; a caller that
  wants a stricter rule changes its own construction, not the executor.
- The graph upserts the identity it is handed, filtering a vetoed `(entity, key)`, in the same
  transaction as the membership write. `with: None` writes nothing — that is the provisional entity.
  It never calls `platform_link` and has no `illuminator` case.
- Projection derives no identities.
- Membership and identity are independent. Moving or detaching an observation does not move or delete
  its old annotations; the caller retracts them explicitly. That is the cost of caller control.
- Caller-supplied identities are not replayed. Link rebuilds them from the observations on every
  build with the caller's current policy; only decisions — merge, distinct, veto, membership — are in
  the log.
- `Identify` is the same write without a membership change.

Why not derive internally: the admission rule is a policy about the data, and it may change. Keeping
it at the call site makes a change visible where it belongs and lets different callers hold different
rules without the graph growing a switch. Why not projection: the graph would need `platform_link`,
`illuminator`, and a policy function — exactly the coupling this removes.

One coupling remains: projection still reads member `platform_*` columns to fill the entity's
`platform_*` presence. That is a separate decision (open question 5); the identity path is clean.

## Provenance, not authority

`Link`, `Resolve` and `Replay` are code, not vocabulary. The graph never learns their names, and it
does not authenticate the caller.

- **Provenance is on the mutation.** An `Identity` carries `provider`; a decision carries `cause`.
  That is what the row stores, and it is enough for replay to re-emit the same mutation later.
- **The graph enforces the invariants of that provenance.** A lift may not delete a row whose
  `cause` is above its own (`Model` < `Repair`). A provider-scoped write is scoped by the identity's
  `provider`, and veto admission filters it.
- **The edge authenticates.** The agent adapter maps every model mutation to `provider = model` and
  `Cause::Model`, and exposes only the subset a model may call. Internal callers are trusted. There
  is no `Principal` in the graph and no capability table.
- **If the executor becomes a public boundary**, add a principal argument. That is authorization,
  not vocabulary, and it does not belong in the mutation.

What the code paths build:

- **Link**: `HeuristicLinker` plans the pending batch; `CreateEntity` and `AttachObservation` carry the
  `Identity` its policy admits. The graph never sees the platform link.
- **Identify**: `Identify`, `RetractIdentity`, chunked per entity. That is how a provider
  replaces its set without a set-shaped mutation.
- **Replay**: `CreateEntity`, `AttachObservation`, `DetachObservation`, `MergeEntity`,
  `VetoIdentity`, with the recorded `cause`. It resolves origins to entities through the rebuilt
  graph and never re-emits identities.
- **Agent / User / Operator**: whatever the edge authorizes.

## Call rules

- **One call, one transaction.** Every mutation implementation takes `&C: ConnectionTrait`; the
  executor opens the transaction, applies in canonical order, and projects once at the end.
- **Rows are locked, not just written.** Resolving an origin takes `FOR UPDATE` on the live entity;
  the observation under a membership edit is locked too. Two writers serialize, and a fold cannot
  tombstone an entity another mutation is mutating.
- **All-or-nothing.** Any `Refuse` blocks the call. `NoChange` is a success, not a failure.
- **Canonical order**, not caller order:
  1. `UnvetoIdentity` — lift a standing veto before anything it would block
  2. `CreateEntity`
  3. `AttachObservation`, `DetachObservation`
  4. `MergeEntity` — membership is final before the shared-illumination check
  5. `Identify`, `RetractIdentity` — lands on the survivor after a fold
  6. `VetoIdentity`, `MarkDistinct` — assert a standing denial last
- **Contradictory plans are not yet detected.** Canonical order decides the outcome; a call that
  asserts and denies one pairing applies both in order. A validation pass is a follow-up (open
  question 9), not a rule the executor pretends to have.
- **Entity origins resolve during execution**, in order. Validation resolves against current state
  and accepts an origin that an earlier `CreateEntity` in the call will establish; the transaction's
  guards are authoritative.
- **Projection set** is the union of: the new entity (create); source and target (attach); source
  (detach); winner (merge); subject (attest, retract, veto, unveto). Distinct mutations touch none.
  Dedup, project once.
- **Projection derives no identities.** The call recomputes name, `alt_*`, descriptions, search,
  platform presence, status and confidence from the members and the evidence, and writes the
  entity's blocking-index row. A merge forgets the loser's row. The index is retrieval material, not
  graph truth.

## Refusals

Typed, so an agent can correct without parsing prose:

```rust
pub enum Refusal {
    UnknownObservation(ObservationId),
    UnknownOrigin(EntityOriginId),
    AmbiguousOrigin(EntityOriginId),
    LinkedElsewhere(ObservationId),
    OriginClaimed(ObservationId),
    StandingDistinct,
    PendingMerge,
    SharedIllumination,
    Contradiction(&'static str),
    Unauthorized,
}
```

## Retries

Every mutation is safe to re-apply after a committed call, because the second pass lands on a
`NoChange` rather than a duplicate:

- create: the observation is on an entity whose origin is that observation → `NoChange`.
- attach: already on the target → `NoChange`.
- detach: already unlinked → `NoChange`.
- merge: loser already tombstoned into winner → `NoChange`.
- distinct / veto: already recorded → `NoChange`.
- attest: upsert.
- retract / unveto: already absent → `NoChange`.

## Open questions

1. `Identify` for `model`: an ephemeral identity annotation, or a logged decision? The cleaner
   split is that an agent's identities are an identification run (recompute them), and only its decisions
   are durable.
2. The log columns for `attach` / `detach` hold observation ids in `subject_origin`. Keep the
   overload, or add `subject_observation`?
3. Lifting after a rebuild: replay currently writes `Cause::Replayed`, which erases whether a person
   or an agent decided. Either preserve the original cause (preferred), or have the rung check scan
   the original row, which the reset keeps. Confirm before relying on provenance.
4. `CreateEntity` durability needs a `create` kind. With a single observation there is no
   root-plus-attaches encoding; later members log their own `attach` rows.
5. The entity's `platform_*` columns are still projected from the members' platform fields, so the
   graph still reads them even though identities are caller-supplied. Zero coupling means deciding
   whether those columns stay as a presence hint, or the identifier reads identity rows instead.
6. Is `Provider` a newtype over `String`, or the existing reserved-id constant set?
7. `DetachObservation`, and any move that empties its source: refuse `LastMember`, define a ghost
   state projection can write, or delete the entity. Related: an origin is a durable key, so
   `CreateEntity` must refuse an origin already claimed, or moving/detaching an origin can give two
   entities the same key. If attach/detach become durable they also gain a `cause`: the verdict
   column is not null.
8. `MarkDistinct` is one-way: knowledge 1 has no `UnmarkDistinct`. Add the inverse only if a wrong
   `distinct` needs lifting; until then it is a standing decision.
9. Contradictory plans: reject a call that merges and marks distinct one pair, attests and vetoes one
   pairing, or attaches and detaches one observation, instead of applying in canonical order.
