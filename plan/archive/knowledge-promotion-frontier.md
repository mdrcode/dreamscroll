# Beyond the promotion gate — what the non-promoted 16% needs

**Status:** Analysis of the prod corpus (2026-10-30). Design input for
[knowledge-canonical-entities.md](knowledge-canonical-entities.md); evidence in
[knowledge-promotion-audit-prod.md](knowledge-promotion-audit-prod.md).

832 of 5,185 prod mentions (16.0%) are not promotion candidates. This document
asks what they actually are, and what evidence would let the graph identify them
later. Aggregate only; per-row detail stays local.

## What the 16% is made of

| Bucket | Rows | What it really is |
|---|---|---|
| `review_disambiguation` | 577 | **Not a disambiguation problem at all.** Zero of these rows collide with another type, and none carry a homograph cue. Every one was held because the *name string* is short: one token, ≤8 characters, or lower-case. The bucket contains Plato, Napoleon, Hergé, Elsevier, the Houthis, FFmpeg, PDP-11, Gimli, the Ardennes. |
| `hold_collision` | 112 | **Real** collisions: one string, several referents (a novel and its 2002 film; a magazine and its 1981 film; a company and its search engine; a book and a person named Augustus). |
| `review_thin_link` | 97 | Accounts and people with a **stable identifier but no description** — the model named them and said nothing else, or said too little. |
| `review_taxonomy` | 36 | Kinds: movements, techniques, theories, taxa. Already covered by the type/kind recommendation. |
| `review_thin` | 5 | Famous entities whose description is **empty**: World War I, World War II, the Vietnam War, Marvel Comics, and one critic. |
| `review_person` | 5 | Role-described creators with no identifier and no link captured. |

Three of the six buckets are therefore not "hard entities" but **defects in how
the gate judges a mention**: a name-shape heuristic standing in for a collision
test (577), a collision treated as a reason to reject instead of a reason to split
(112), and a model omission treated as evidence about the entity (5).

## The category error under the gate

The gate answers one question — *may we publish this text?* — but the 16% mostly
fail a different one: *can we say which thing this is?* Those are independent
axes, and the graph needs both:

| | Identified | Unidentified |
|---|---|---|
| **Described** | `promote` / `promote_linked` — 84.0% | the long tail: niche accounts and local people |
| **Undescribed** | World War I with an empty description; 97 handle-only people | genuinely unknown occurrences |

Only the top-left quadrant is "done" today. The top-right cell is the cheapest
win in the whole graph — these entities are *fully identified*; we simply have no
text to publish, and the gate treats missing text as missing identity.

## What evidence could answer "which thing is this?"

Ordered by cost, not by power.

**E0 — the entity's own identifier.** A platform handle is a stable, checkable
identifier (the analyser already verifies X accounts exist via oEmbed). 775 rows
carry one; 711 distinct names; 695 distinct handles. Identity is available for all
of them without any description.

**E1 — corpus-internal context.** The capture itself: what else appears in it, the
source URL if present, the capture's own summary, and whether the name recurs
across captures with the same co-mentions. This is free and underused — our
current classifier looks only at the single row.

**E2 — external authority records.** Wikidata or a domain authority.

An earlier draft of this section claimed that authority records were unreliable
because exact-label matches were often the wrong referent (Carhartt → "family
name", Odyssey → an instrumental). **That was a defect in the probe, not in the
records.** The probe searched the name alone, took the single top hit, and never
used `search_query`. Re-measured with a real strategy, 22 of the same 30 rows
(73%) resolve — and the records everyone assumed were missing exist: Carhartt's
brand is `Q527877` ("American brand of sturdy work clothing"), Homer's epic is
`Q35160`, the 1997 film is `Q1144476`.

Three modes are needed, in this order:

1. **Label search, then rank the candidate set.** `wbsearchentities` matches
   labels and aliases only — it is *not* full-text: `"Carhartt brand"` returns
   zero hits while `"Carhartt"` returns the company second. Reading only hit #1
   is what produced the false "unreliable records" conclusion.
2. **Full-text search** (`list=search`, CirrusSearch) for the cases where the
   label is qualified or absent: `search_query` values like "Cure 1997 film" and
   "Kirby Nintendo character" resolve exactly, and no label search can reach them.
3. **Verify the candidate** — type-compatible `P31`/description against our own
   type and description. This step is still required: the scripted run picked
   Kirby (a character from another franchise) and a Netflix duplicate item,
   because full-text returns a ranked list and the first line is not always the
   right one.

`KnowledgeEntity.search_query` is exactly the input mode 2 needs, it is populated
on **52% of prod rows** (2,691 of 5,185, differing from the name on 2,237), and
the analyser currently **ignores it entirely** — it is selected by the SQL and
never read. That is the single cheapest fix in the resolution path.

**E3 — reverse identity from the identifier.** Instead of searching a name, ask
which item *owns* the handle. Wikidata `haswbstatement:P2002=<handle>`:

| Handle class | Result |
|---|---|
| Established people (`elonmusk`, `tylercowen`, `ericweinstein`) | resolved to an item, one hit each |
| Long-tail accounts (3 tested) | no item |

3 of 6 resolved. This is the mechanism that converts `review_thin_link` rows into
identified entities without any text at all, and it is *verifiable* in a way that
name search is not.

**E4 — ask the model again.** The 5 `review_thin` rows are world-famous events
with an empty description: a re-run of the description step, or a description
derived from the authority record, resolves them completely. Missing descriptions
are a *transient* state, not a property of the entity.

**E5 — the user.** For the residual — an account with no authority record, no
description, and no recurrence — the only ground truth is the person who captured
it. Not a fallback: it is the highest-quality evidence in the system, and the only
kind that can never be derived from public data.

## Measured ceiling

| Mechanism | Rows recoverable | Confidence |
|---|---|---|
| Replace the name-shape test with a collision test | 577 | High — measured: 0 of them collide |
| Split collisions into referents, promote each | 112 → ~2× nodes | High |
| Re-describe empty rows | 5 (+ raises description quality generally) | High |
| Handle-first identity (E3) | 97, roughly half resolving | Medium — 3/6 sampled |
| Authority resolution with verification (E2) | ~73% of a 30-row sample, 3 needing full-text | Medium — measured; still needs candidate verification |
| User confirmation (E5) | the residual (~2%) | Certain, but human |

That puts an achievable steady state around **97% identified**, versus 84%
promotable today — and the last 3% staying deliberately private is a feature, not
a failure.

## What promotion should become

A binary gate cannot express the states above. The future shape is a **graded
identity state per canonical node**, with promotion as a consequence:

| State | Meaning | What the base layer may hold |
|---|---|---|
| `linked` | identifier or authority match, no usable description | the identity and the link; no universal description |
| `described` | identity plus a publishable definition | the definition, aliases, type, class |
| `collision` | a string with several identified referents | a disambiguation page, plus one node per referent |
| `local` | no public evidence | nothing shared; private graph only |

Two consequences worth stating now, because they shape the schema:

1. **Collisions become nodes, not holds.** A name shared by a novel and its film
   is *two* promotable entities. The current `hold_collision` verdict throws away
   two good nodes because the string is ambiguous.
2. **The name is not the key.** Every mechanism above produces an identity that is
   independent of the surface form. That is the same conclusion as finding 4 in the
   prod audit (name equality ≠ identity), reached from the other direction.

## What I would build, in order

1. **Collision test instead of name-shape test** — 577 rows, hours of work, and it
   removes the bucket that makes the gate look unreliable.
2. **Re-describe empty rows** — free accuracy on the most famous entities.
3. **Split collisions, promote each referent** — turns 112 rejections into ~220
   nodes.
4. **Handle-first identity (E3)** for accounts and people — recovers about half of
   the 97 thin links and needs no description at all.
5. **Type-verified authority resolution (E2)** — the big unlock; needs the
   verification step that Carhartt/Odyssey/Kirby prove is not optional.
6. **A user confirmation surface (E5)** — small, but it is the only path to the
   residual, and it also improves the private layer.

## Risks

- **Unverified top-1 selection introduces false identity.** The failure mode is
  not missing records — they exist — but accepting the first search hit (Kirby
  from the wrong franchise, a duplicate Netflix item). Verification is the
  difference between E2 raising coverage and E2 lowering quality.
- **Identity must not imply publishability.** A well-known person's handle is
  public; their contact details are not. The publishability rules in the design
  doc (S1–S3) still gate every field independently of identity.
- **Disambiguation pages need governance.** Once one string maps to several nodes,
  someone must arbitrate merges — that is the human-in-the-loop surface, not an
  afterthought.
- **Long-tail coverage is bounded by design.** Roughly 2–3% of a personal corpus
  will never be publicly describable. Optimising past that point costs more than
  it returns.
