# Entity attestations — references, not identities

**Status:** Proposed 2026-11-11. Design only; no schema or code change. Not scheduled, not
committed. This doc records a model worth keeping even if implementation waits. It predates schema
39; read "claim" below as an identity annotation on an entity, and `Claim` in the resolver sketch as
`Identity` — see [knowledge-identity-annotations.md](knowledge-identity-annotations.md).

**See also:** [knowledge-resolution.md](knowledge-resolution.md) (the resolver contract this would
extend), [knowledge-claim-management.md](archive/knowledge-claim-management.md) (the claim edge and its
provenance), [lexicon.md](lexicon.md) (current vocabulary), `src/knowledge/knowledge.md` (the model
as built), [pragmatism.md](pragmatism.md) (the trade-off ledger).

## The trigger

A web-search resolver reads pages and finds things such as:

- an Instagram profile
- an official website
- a NYT article
- a Reddit thread

Only some of those are identities. The current model has exactly one way to record a finding: a
`knowledge_claims` edge saying “this entity is that identity.” That is wrong for an article or a
thread, and it leaves a real gap:

> An entity can be documented by several sources and still have no canonical identity.

Today that entity stays `provisional`. For entities absent from Wikidata, that is the common case.
We do not want to invent a fake identity just to move the entity out of `provisional`.

## The distinction

| term | meaning | example |
| --- | --- | --- |
| **identity** | a stable `(scheme, external_id)` that names the referent | `instagram:billybob`, `wikidata:Q42` |
| **claim** | this entity is that identity | entity 42 → `wikidata:Q42` |
| **reference** | a source that is about the entity | `nytimes.com/.../billy-bob.html` |
| **attested** | the entity has references but no identity claim | a person with articles but no QID |

A page can be the identity of a document. It is not thereby an identity of the subject. The NYT
article is an identity of the article; it mentions Billy Bob. The graph does not need article
entities or a `mentions` relation to record that it is about him.

The rule of thumb:

> Does the page **name the referent**, or does it merely **mention the referent**?

Profiles, official presences, and authority entries name the referent. Articles, blog posts, and
threads mention it.

## Proposed model

### Entity-level references

Add one table. It deliberately has no `claim_id` and no `knowledge_identity_id`.

```sql
knowledge_references
  id
  knowledge_entity_id      -- the entity this source is about
  url                      -- canonical URL, not a raw search result
  title
  publisher
  role                     -- official | news | forum | social | other
  provider                 -- attestor / resolver id
  retrieved_at
  UNIQUE (knowledge_entity_id, url, provider)
```

Properties:

- **Entity-level, not claim-level.** A reference says “this source is about entity 42,” not “this
  source proves entity 42 is Q42.”
- **Append-only by default.** A URL disappearing from search results is not evidence that the
  entity was never documented. `retracted_at` can come later for manual rejection.
- **Deduped by canonical URL.** The resolver canonicalizes before writing; the table stores the
  canonical form. Same URL from a second provider may be a second row (provider is part of the
  key), but status only needs one.
- **No article entities.** Keeping the source as a reference avoids creating, blocking, and
  merging an entity for every article on the web.

### Status becomes a ladder

| status | condition | meaning |
| --- | --- | --- |
| `identified` | at least one live identity claim | the entity has a durable name |
| `attested` | no live claim, at least one live reference | the entity is documented, not named |
| `provisional` | neither | a cluster with nothing external about it |

Ordering: `identified` beats `attested` beats `provisional`. A claim retraction can fall back to
`attested` if references remain.

The projection becomes:

```rust
let status = Status::of(has_live_claim, has_live_reference);
```

References never make an entity `identified`. That is the whole point: a reference is corroboration
that the entity is discussed, not a canonical name for it.

### Confidence stays claim-derived

Do not let reference count change `confidence` implicitly. Today `combine` uses identity
confidence, and a text claim counts once per asserting provider. References are a different axis:
they justify `attested`, not a higher identity confidence.

If source quality should affect confidence later, that is a separate policy with its own risks
(SEO spam, duplicated syndication, five Reddit threads). Start with status only.

## Resolver / attestor seam

The current contract cannot express “I found references but no identity”:

```rust
async fn resolve(&self, entities, gateway) -> Vec<(i32, Claim)>;
```

Two shapes are worth considering when this becomes live.

### Option A — richer resolver result

```rust
struct Resolution {
    entity_id: i32,
    identity: Option<Claim>,
    references: Vec<Reference>,
}
```

One web-search pass can return both identity claims and references. This is the fewest calls when a
single agent does both. Selection becomes “ask this provider while it has neither a claim nor a
reference,” with an include-already flag for re-resolution.

### Option B — split `Resolver` and `Attestor`

```rust
trait Attestor {
    fn id(&self) -> &'static str;
    async fn attest(&self, entities, gateway) -> Vec<(i32, Reference)>;
}
```

Clearer vocabulary: `Resolver` means identity, `Attestor` means documentation. Selection is
naturally per provider and per kind: a resolver skips entities it already claimed, an attestor skips
entities it already referenced. The cost is a second pass if one agent owns both jobs.

**Proposed direction:** keep the distinction explicit. A “web-search identity resolver” is a weak
concept; a “web-search attestor that can also surface canonical identities” is the honest shape.
The exact seam can wait until implementation.

## What the web-search agent should emit

1. **Canonical identities** for pages that name the referent:
   - `instagram` + handle, not the full profile URL
   - `imdb` + id
   - `wikidata` + QID
   - `official_website` + registrable domain, if verified
2. **References** for pages that are about the referent:
   - title, publisher, URL, role
   - attach to the entity, not to a claim
3. **Nothing** for irrelevant results or namesakes.

The agent decides which category a page belongs to. The graph should not infer identity from a URL.
This matches the existing split: the resolver owns strategy; the graph stores claims and evidence.

## Examples

| finding | stored as | resulting status |
| --- | --- | --- |
| `instagram.com/billybob` | identity `instagram:billybob` + claim | `identified` |
| `imdb.com/name/nm...` | identity `imdb:nm...` + claim | `identified` |
| `billybob.com` | identity `official_website:billybob.com` + claim, or reference | `identified` if claimed |
| NYT article | reference on entity 42 | `attested` |
| Reddit thread | reference on entity 42 | `attested` |
| NYT + Reddit, no profile/QID | two references | `attested` |
| nothing | none | `provisional` |

## Non-goals

- Do not create entities for articles, blog posts, or threads solely to store mentions.
- Do not add a `mentions` edge in v1. It is a larger graph question with its own dedupe and
  proximity consequences.
- Do not make references identify an entity.
- Do not attach references to claims or identities in v1.
- Do not use reference count to inflate identity confidence.
- Do not automatically retract a reference because a later search did not repeat it.

## Consequences and plumbing

- **Schema:** new table, index on `knowledge_entity_id`, schema version bump, model, and sync.
- **Projection:** read claim existence and reference existence; add `Status::Attested`; update
  `stats`, UI badges, and tests.
- **Selection:** `awaiting_authority` still finds entities with no claim, so an `attested` entity
  remains eligible for an identity resolver. An attestor needs its own “no reference from this
  provider” selection.
- **Merge:** move references from loser to winner and dedupe by `(url, provider)`.
- **Reset:** decide whether references are part of the graph snapshot and emptied with it, or
  preserved across rebuilds. Entity ids change on reset; preserving them requires a durable key
  such as `origin_observation_id`. Start with snapshot/empty for consistency with claims.
- **Cap:** references need a per-entity/provider cap and canonical dedupe, so an agent cannot
  return a hundred URLs.
- **Audit:** an entity-level reference cannot answer “which source supports Q42?” That is the
  deliberate loss. If claim-level citation audit becomes necessary, add an optional link later
  without changing the status ladder.

## Acceptance tests when implemented

- An entity with only NYT/Reddit references is `attested`, not `identified`.
- Adding an Instagram claim moves it to `identified`.
- Retracting the claim with references left moves it back to `attested`.
- Deleting all references without a claim moves it to `provisional`.
- A reference does not change `confidence`.
- The same canonical URL from the same provider is a no-op.
- A merge moves and dedupes references.
- Reset either snapshots and clears references, or preserves them explicitly; the test names the
  chosen policy.
- A resolver returning references but no claim is reported, not treated as “found nothing.”

## Open questions

- Is one reference enough for `attested`, or does it require two independent providers/publishers?
- Should `role` be required, or inferred from the source and left optional?
- Should references be append-only forever, or gain `retracted_at` for bad sources?
- Should official website be an identity claim, a reference, both, or a policy choice per entity?
- Does `attested` belong in the entity list UI as a first-class status, or only in stats/detail?
- Does reset lose references, or are they external evidence that must survive a rebuild?
- If one agent both resolves identities and attests references, is the richer result worth
  avoiding two passes?

## Alternatives considered

**References linked to claims.** More audit power: “this article supports this identity.” It also
reintroduces claim ids into evidence, complicates merge/reset, and makes the source look like proof
of an identity. Deferred.

**References linked to identities.** Wrong subject: one identity may be shared by several entities,
and a source about the entity is not automatically about the identity.

**Model articles as entities.** Gives articles their own identity and status, but requires dedupe,
blocking, and a mentions relation that the graph does not have. Too much machinery for the value.

**Treat every search result as an identity claim.** Mechanically works today, but says the entity is
an article or a thread, flips status to `identified`, and can create false proximity candidates.
Rejected.
