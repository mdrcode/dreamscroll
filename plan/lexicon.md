# Lexicon — the names the knowledge graph uses

**Status:** Authoritative (2026-11-01), adopted with schema version 16, which renames the graph's
tables, columns and verbs. The map from the previous names is at the end, and so is the list of names
that were considered and rejected — read that before proposing a rename.

**See also:** [knowledge-canonical-entities.md](archive/knowledge-canonical-entities.md) (the model),
[knowledge-resolution.md](knowledge-resolution.md) (the resolver).

## The sentence

> A **capture** is illuminated into **observations**; **linking** clusters observations into
> **knowledge entities**; **resolution** asks the authorities about them and records the answers as
> **evidence**.

Everything the graph does is a clause of that sentence. A verb that is not in it is an implementation
detail and should not be named after one.

## Scope

These names govern **every** identifier in the graph — public types, private types, locals, fields
— not just the schema. `core` is retired everywhere; the surviving uses are historical DDL naming
tables being dropped (`core_knowledge_*` in the retirement step) and unrelated English.

The schema-16 cutover renamed tables, columns, exported types, CLI verbs and docs, but left the
internal vocabulary (`scoring::Core`, `CoreView`, `CoreState`, `merge_core`, `cores_resolved`) in place.
The compiler cannot see a name that is merely wrong, so the gap surfaced only when a design
discussion repeated the old word. Treat "no internal identifiers either" as part of any rename.

## Two layers

| Layer | Noun | Belongs to | Rewritten? |
| --- | --- | --- | --- |
| personal | **observation** | one user | never |
| shared | **knowledge entity** | nobody | projected, any time |

An observation is a record of what the extractor said — trusted because we trust the extractor, not
because the graph agreed with it. That is why it stands alone unlinked, why `reset` can drop every
entity and lose nothing, and why a wrong link is repaired by re-linking rather than by editing the
observation.

## Models

| Noun | Rust | Table | Columns |
| --- | --- | --- | --- |
| **observation** | `UserObservedEntity` | `user_observed_entities` | `id`, `user_id`, `capture_id`, `illumination_id`, `position`, `created_at`, `name`, `entity_type`, `description`, `search_query`, `platform`, `platform_handle`, `platform_display_name`, `knowledge_entity_id` |
| **knowledge entity** | `KnowledgeEntity` | `knowledge_entities` | `id`, `name`, `alt_names`, `entity_type`, `alt_types`, `description`, `alt_descriptions`, `status`, `confidence`, `merged_into_id`, timestamps |
| **identity** (schema 23) | `KnowledgeIdentity` | `knowledge_identities` | `id`, `scheme`, `external_id`, `label`, `description`, `classes`, `provider`, `method`, `confidence`, `first_seen` |
| **claim** | `KnowledgeClaim` | `knowledge_claims` | `id`, `knowledge_entity_id`, `knowledge_identity_id`, `provider`, `method`, `user_observed_entity_id`, `retracted_at`, `observed_at` |
| **merge** | `KnowledgeVerdict` | `knowledge_verdicts` | `id`, `generation`, `cause`, `winner_origin`, `loser_origin`, `winner_label`, `loser_label`, `at` |
| **proximity** | `KnowledgeProximity` | `knowledge_proximities` | `id`, `holder_id`, `counterpart_id`, `knowledge_identity_id`, `basis`, `score`, `verdict`, `raised_at`, `settled_at` |

Two things have no table: an **identity** is `(scheme, external_id)` on a claim, and a **snapshot**
is what `reset` saves and `compare` diffs.

### Column conventions

- `name` / `alt_names`, `entity_type` / `alt_types`, `description` / `alt_descriptions` — three pairs
  of the same shape: singular primary, plural alternatives, `alt_` meaning *the complement of the
  primary*. Do not rename one without the other two.
- `knowledge_entity_id` is spelled out on observations because the row is itself an observed entity,
  so a bare `entity_id` would be ambiguous. Inside `knowledge_entities`, `merged_into_id` is short
  because `id` there can only mean a knowledge entity.
- An observation's `name` is what the capture said, verbatim. A knowledge entity's `name` is the
  surface `project` chose to represent it, and `alt_names` is everything else it has been seen as.

## Two verbs that both mean "connect"

> **Link** asks *are these the same thing?* — it clusters observations into one knowledge entity.
> **Resolve** asks *what can the authorities tell us about this thing?* — it records the answers as
> evidence.

Resolution emits claims, and also two things that are **about the graph rather than the world**: a
collision (two entities holding one identity that cannot be merged, awaiting a decision) and a merge
(a mutation, recorded for whoever reverses it). Those live in their own tables — see below — because
one table answering three questions needs a `kind` discriminator, three payload shapes and a
per-kind index.

## Verbs

| Verb | Direction | Notes |
| --- | --- | --- |
| **illuminate** | capture → observations | the model. Never "extract" — that word is for pulling fields out of a response |
| **link** | observations → knowledge entities | pure, offline, idempotent |
| **resolve** | knowledge entities → evidence | network, cache-first |
| **merge** | knowledge entity → knowledge entity | tombstones the loser, never deletes |
| **project** | observations + evidence → entity columns | pure |
| **retract** | a claim stops being believed | sets `retracted_at` |
| **reset** | drop derived state, keep observations | pure |
| **compare** | snapshot → snapshot diff | pure |

## Claims, proximities and merges

| Table | Is | Written by |
| --- | --- | --- |
| `knowledge_claims` | a fact about the world: this entity has this identity, on whose authority | `link` (a capture's handle), `resolve` (an authority's answer) |
| `knowledge_proximities` | a judgement about our data: two entities look close enough to ask about, and what was decided — a record, never deleted | `proximity::scan`, when a merge is refused; a human, with a verdict |
| `knowledge_verdicts` | a mutation of our data: one thing folded into another, what decided it (`repair`, `proximity_auto`, `replayed`), and which generation — a log, keyed by origin so it survives a rebuild | `resolve`, the repair command, and `verdicts replay` |

`enrichment` and `rejection` remain reserved kinds in the vocabulary, not tables: enrichment is the
authority's answer about an identity — its `label`, `description`, and `classes` — and it lives on
`knowledge_identities`, not on the claim. `project` does not read it yet.

An identity is owned by one provider at a time. Its metadata is that provider's answer, not a
reconciliation of several; a provider answer may take the row over from another. Claims keep their
own `provider`, so takeover changes the identity without rewriting edge history.

## Status

`provisional` → `identified`. `identified` describes a knowledge entity that holds a live authority
claim; it is not the past participle of `resolve`, which may abstain.

There is no `contested` status. "This entity shares an identity with another and nobody has decided"
is a **proximity** — a pair-level record with a score and a verdict — and folding it into one field
alongside substantiation conflated two questions and had to be re-projected on every scan.

## Names considered and rejected

Grep before you argue. **A word's meaning in this repo is established by use, not by the dictionary**,
and twice the decisive objection to a candidate was that it already meant something else here: an
`oracle` is our measurement ground truth, and a `quota` is the *provider's* limit. Check the candidate
against the codebase first; this table is the record.

| Name | Why not |
| --- | --- |
| *mention* | implies a string whose meaning is fixed by its referent; an observation can be wrong and still be a faithful record |
| *core* / *CoreEntity* | a cluster has a core; entities are not derived from clusters, and "core" names a mechanism that no longer exists |
| *CoreKnowledgeEvidence* / *evidence* | one table with a `kind` discriminator for three different statements; split into claims, collisions and merges |
| *knowledge_evidence* (for the identity table, schema 23) | a QID is the referent, not a statement about it, and half of the identities are platform handles read off a capture — neither has an external assertion to be evidence *of*. `identity` is already this lexicon's word for `(scheme, external_id)`, so the table adds no vocabulary. Reconsider only as a fourth, statement-level table (`knowledge_evidence` = one row per authority answer, with identities derived), which is a different and larger model |
| *label* / *alt_labels* | technical jargon for the same idea; `name`/`alt_names` is plain and keeps the singular/plural pattern |
| *identify* | presupposes success, and resolution abstains often; also too narrow — it names only one of the kinds |
| *related_core_id* | one column with two meanings selected by `kind`; deleted, the role lives in the payload |
| *contested* (status) | a derived boolean stored beside substantiation; the proximity record already says it at pair granularity |
| *knowledge_id* | names a layer, not a thing; there is no `knowledge` table, and `knowledge_evidence` invites the wrong join |
| *oracle* (for a provider, or for `Request::provider`) | already this project's word for the measurement ground truth — *"an exhaustive scan over a corpus is the oracle"* (`plan/archive/knowledge-blocking.md`), and the deleted in-memory blocker was *the parity oracle*. It also connotes infallibility, which is the opposite of the stance: `found` means candidates came back, not that one is the referent. Kept `provider` |
| *quota* (for what a run may spend) | already the provider's limit, in code: `"vertex: quota exceeded"`. A quota is granted by someone else; `max_calls` is our own anti-spam budget, and the provider's real quota is what a `429` reports. One word for both would erase the distinction `Refusal::RateLimited` exists to draw. Kept `Allowance`, whose two refusals are `Budget` and `RateLimited` |
Schema 34 renamed `knowledge_identities.asserted_by` to `provider` and changed the capture value from
`platform` to `illuminator`. The old objection to `provider` was the *value* `platform`: the platform
answered nothing. `provider` is now the source of the row and a **superset** of `Request::provider` —
for an authority it is the same id; `illuminator` and `model` never appear in `fetch_cache`. The
reserved ids are `illuminator`, `platform`, `model`, `operator` and `user`; the engine refuses a
resolver that takes one.

## Rename map (schema 16)

| Before | After |
| --- | --- |
| `knowledge_user` (legacy) | deleted — folded into `user_observed_entities` |
| `user_knowledge_entities` | `user_observed_entities` |
| `core_knowledge_entities` | `knowledge_entities` |
| `core_knowledge_evidence` | `knowledge_evidence` (later split into the three below) |
| `model::knowledge_entity` (observation) | `model::user_observed_entity` |
| `model::user_knowledge_entity` | merged into the above |
| `model::core_knowledge_entity` | `model::knowledge_entity` |
| `model::core_knowledge_evidence` | `model::knowledge_evidence` → split, schema 22 |
| `core_id` | `knowledge_entity_id` |
| `user_entity_id` | `user_observed_entity_id` |
| `label` / `alt_labels` | `name` / `alt_names` |
| `superseded_by_id` | `merged_into_id` |
| kind `ontology` + `platform_link` | kind `claim` (differing only in the identity's `provider`) |
| kinds `description` / `identity_rejection` / `identity_claim` | `enrichment` / `rejection` / deleted |
| `build_core` | `link` |
| `merge_core_entities` | `merge` |
| `reset_core` | `reset` |
| `migrate` | deleted |
| `KnowledgeEntityInfo` (API) | `ObservedEntityInfo` |
| `Request.kind` / `cached.kind` | `Request.route` / `cached.provider` |

Unchanged on purpose: `entity_type` (the reconciliation contract's "type", and the parallel of
`alt_types`), the `resolve` module and doc filenames, and the `/entity/{id}` REST route, which still
serves an observation id and is due a revisit once knowledge entities are user-visible.
