# knowledge2: actions and their parameters

Status: v10, implemented in `graph/`, `helper.rs`, and `linker.rs`. Same four tables. Proximity out
of scope.

Callers emit actions; one call is one transaction. No action contains a generation-scoped entity id,
so a list is independent of the generation it is applied to.

The graph does not care which code assembled the list, or who is calling. It stores the provenance
each action carries — `provider` on an identity, `cause` on decisions — and enforces the invariants
that provenance implies. Authentication is the edge's job.

## Shape

```
Link ─────┐
Resolve ──┤
Replay ───┼──▶ Vec<KnowledgeAction> ──▶ validate ──▶ apply ──▶ project
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
pub enum KnowledgeAction {
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
    AttestIdentity {
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

`Identity` and `Cause` are explicit because they are the provenance the row stores. The action means
what it says without the call; the caller is not consulted to interpret it, and is not stored.

The graph never sees a platform link. The caller decides whether an observation's `platform_*`
columns are an identity, converts that to an `Identity`, and passes it on the membership action or as
an `AttestIdentity`. The graph upserts exactly what it is handed; it has no `platform_link` call and
no `illuminator` special case.

## Per-action contract

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
  - Logs: `kind = merge`, the pair's canonical origins, `why` in `note`, the action's `cause`.

- **`MarkDistinct { a, b, cause, why }`**
  - Accepts: both origins resolve to live entities and are distinct; no pending repair merge on the
    pair.
  - NoChange: already distinct.
  - Does: insert the verdict. No graph row changes; projection is not needed.
  - Logs: `kind = distinct`, the origin pair, the action's `cause`.
  - One-way: knowledge 1 has no inverse; a wrong `distinct` cannot be lifted yet.

- **`AttestIdentity { subject, identity }`**
  - Accepts: the subject resolves to a live entity; non-empty provider, scheme and external id;
    `observation_id`, when present, exists.
  - NoChange: the pairing is vetoed — the identity is dropped, not refused, so a resolver call is not
    blocked by a person's decision.
  - Does: upsert `(entity, provider, scheme, external_id)` with the metadata and provenance; project
    the subject.
  - Logs: nothing. Identities are recomputable, and a resolver run replaces them.
  - Why: `Identity` carries the provider, so the uniqueness rule and the veto rule are readable at
    the action.

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
    action's `cause` and `why`.

- **`UnvetoIdentity { subject, key, cause }`**
  - Accepts: the subject resolves; no veto row for the origin and key has a `cause` above this one.
  - NoChange: no veto row.
  - Does: delete every veto row for the origin and key. Nothing is restored; the next resolve may
    assert the annotation again.
  - Lifting rule: `Model` cannot lift a `Cause::Repair` row. The action states its own cause; the
    edge is responsible for not letting a model claim `Repair`.

## The planner

A v2 `Linker` is a policy: the pending batch in, one `KnowledgeAction` per observation out.

- `Blocker` is recall: live entities only, ordered by id. An error is not an empty set; empty means
  "nothing matched, create," and a failed probe stops the link.
- `IdentityHolder` is the one evidence read: which live entity already carries
  `(scheme, external_id)`, named by origin.
- `HeuristicPlanner` is today's policy: a held identity wins; else the best qualifying blocker
  candidate; else create. The target is an `EntityOriginId`, never an entity id, so a decision made
  against an older corpus follows tombstones when it is applied.

The caller selects the batch. Already-linked observations are not the planner's problem; the helper
filters them before planning.

## Identities are caller-supplied

If an observation's platform columns should become an identity, the caller says so. The caller — today
the Link producer — reads `platform_link(observation)`; if its policy admits the handle, it passes an
`Identity` on `CreateEntity` or `AttachObservation`. An agent or operator may pass none.

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
- `AttestIdentity` is the same write without a membership change.

Why not derive internally: the admission rule is a policy about the data, and it may change. Keeping
it at the call site makes a change visible where it belongs and lets different callers hold different
rules without the graph growing a switch. Why not projection: the graph would need `platform_link`,
`illuminator`, and a policy function — exactly the coupling this removes.

One coupling remains: projection still reads member `platform_*` columns to fill the entity's
`platform_*` presence. That is a separate decision (open question 5); the identity path is clean.

## Provenance, not authority

`Link`, `Resolve` and `Replay` are code, not vocabulary. The graph never learns their names, and it
does not authenticate the caller.

- **Provenance is on the action.** An `Identity` carries `provider`; a decision carries `cause`.
  That is what the row stores, and it is enough for replay to re-emit the same action later.
- **The graph enforces the invariants of that provenance.** A lift may not delete a row whose
  `cause` is above its own (`Model` < `Repair`). A provider-scoped write is scoped by the identity's
  `provider`, and veto admission filters it.
- **The edge authenticates.** The agent adapter maps every model action to `provider = model` and
  `Cause::Model`, and exposes only the subset a model may call. Internal callers are trusted. There
  is no `Principal` in the graph and no capability table.
- **If the executor becomes a public boundary**, add a principal argument. That is authorization,
  not vocabulary, and it does not belong in the action.

What the code paths build:

- **Link**: a `Linker` plans the pending batch; `CreateEntity` and `AttachObservation` carry the
  `Identity` its policy admits. The graph never sees the platform link.
- **Resolve**: `AttestIdentity`, `RetractIdentity`, chunked per entity. That is how a provider
  replaces its set without a set-shaped action.
- **Replay**: `CreateEntity`, `AttachObservation`, `DetachObservation`, `MergeEntity`,
  `VetoIdentity`, with the recorded `cause`. It resolves origins to entities through the rebuilt
  graph and never re-emits identities.
- **Agent / User / Operator**: whatever the edge authorizes.

## Call rules

- **One call, one transaction.** Every action implementation takes `&C: ConnectionTrait`; the
  executor opens the transaction, applies in canonical order, and projects once at the end.
- **Rows are locked, not just written.** Resolving an origin takes `FOR UPDATE` on the live entity;
  the observation under a membership edit is locked too. Two writers serialize, and a fold cannot
  tombstone an entity another action is mutating.
- **All-or-nothing.** Any `Refuse` blocks the call. `NoChange` is a success, not a failure.
- **Canonical order**, not caller order:
  1. `UnvetoIdentity` — lift a standing veto before anything it would block
  2. `CreateEntity`
  3. `AttachObservation`, `DetachObservation`
  4. `MergeEntity` — membership is final before the shared-illumination check
  5. `AttestIdentity`, `RetractIdentity` — lands on the survivor after a fold
  6. `VetoIdentity`, `MarkDistinct` — assert a standing denial last
- **Contradictory plans are not yet detected.** Canonical order decides the outcome; a call that
  asserts and denies one pairing applies both in order. A validation pass is a follow-up (open
  question 9), not a rule the executor pretends to have.
- **Entity origins resolve during execution**, in order. Validation resolves against current state
  and accepts an origin that an earlier `CreateEntity` in the call will establish; the transaction's
  guards are authoritative.
- **Projection set** is the union of: the new entity (create); source and target (attach); source
  (detach); winner (merge); subject (attest, retract, veto, unveto). Distinct actions touch none.
  Dedup, project once.
- **Projection derives no identities.** The call refreshes `status` and `confidence` from the
  identity rows; name, type, description and platform are set when the entity is created from its
  origin. The blocking index belongs to the external blocker, never to `proc`.

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

Every action is safe to re-apply after a committed call, because the second pass lands on a
`NoChange` rather than a duplicate:

- create: the observation is on an entity whose origin is that observation → `NoChange`.
- attach: already on the target → `NoChange`.
- detach: already unlinked → `NoChange`.
- merge: loser already tombstoned into winner → `NoChange`.
- distinct / veto: already recorded → `NoChange`.
- attest: upsert.
- retract / unveto: already absent → `NoChange`.

## Open questions

1. `AttestIdentity` for `model`: an ephemeral identity annotation, or a logged decision? The cleaner
   split is that an agent's identities are a resolver run (recompute them), and only its decisions
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
   whether those columns stay as a presence hint, or the resolver reads identity rows instead.
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
10. The planner still uses knowledge1's `Heuristic` scoring. Port it into v2 when the corpus
    comparison is done, or move the scoring into a module both engines share.
