# Entity page: from observation to knowledge entity

Status: **Partly implemented 2026-11-19.** Phase 1 (observations carry their entity on the wire) is
in. Decisions in “Decisions”.

The entity page is where a user's personal slice meets the shared graph. Today it shows only the
user's own **observation** (`user_observed_entities`) and never the **knowledge entity** it was
linked to. This doc plans the transition, including the window between illumination and linking when
no knowledge entity exists yet.

## What exists today

- `/entity/:id` (`src/webui/v2/mod.rs`) → `r_entity::get` (`src/webui/v2/r_entity.rs:15`).
  `:id` is a **`user_observed_entities.id`**, not a `knowledge_entities.id`. The lexicon already
  records this as due a revisit (`plan/lexicon.md:175`).
- `get_knowledge_entity` (`src/api/user/get_entity.rs:15`) is the misnamed loader: it finds the
  observation scoped to the user, an unarchived capture, and the capture's **current** illumination.
  A superseded observation therefore 404s.
- Chips link `/entity/{{ entity.id }}` where `entity` is an `ObservedEntityInfo` — observation ids,
  from `capture_illumination.html.tera:31` and `capture_metadata.html.tera:14`.
- The wire shape is already composite: `ObservedEntityInfo.knowledge_entity: Option<KnowledgeEntityInfo>`
  (`src/api/schema/observedentityinfo.rs:40`), including a `KnowledgeEntityInfo` projection of the
  entity row (`src/api/schema/knowledgeentityinfo.rs`). **It is dead**: both constructors hardwire
  `None` (`observedentityinfo.rs:72`, `illuminationinfo.rs:95`).
- The page template renders observation fields only: name, type, platform, description,
  `search_query`, “Appears in” the source capture, and sibling observations in the same illumination
  (`web/v2/templates/entity.html.tera`).

Linking is fast but asynchronous:

- `LinkHelper::link_illumination` (`src/knowledge/helper.rs:61`) plans `AttachObservation` /
  `CreateEntity` mutations for observations with `knowledge_entity_id IS NULL`; provisional entities
  are created at link time, identification comes later.
- `knowledge_link` runs on its own queue, fed by the illumination webhook; its webhook chains
  `identity_resolve`. Link latency is seconds normally, longer under queue retries.
- Merge repoints observations (`UPDATE user_observed_entities SET knowledge_entity_id = winner …`,
  `src/knowledge/graph/sql.rs:470`), so a *linked* observation does not go cold after a merge.

So the three states the user sees are exactly:

| State | Page can show |
| --- | --- |
| before illumination | nothing (no observation) |
| illuminated, not linked | observation only — the “cold” reading |
| linked | observation + knowledge entity (provisional or identified) |

## Target

- The page is **an occurrence plus its knowledge entity when it has one**.
- Cold: render the observation verbatim, visually neutral, explicitly “not yet in the graph”.
- Warm: the knowledge entity is the subject; the observation is “this capture said”.
- The URL is stable across the transition.

The composite API shape is the right one; it simply has to be populated.

## Addressing

`/entity/:observation_id` is already the route, and it is already stable through linking — the id
does not change when `knowledge_entity_id` is set. What it cannot be is *shared across users*:
observation ids are owner-scoped, and the graph's disclosure policy is deliberately deferred
(`src/api/schema/knowledgeentityinfo.rs:1`). So:

| Option | Shape | Trade-off |
| --- | --- | --- |
| A | Keep `/entity/:observation_id` as the owner view; resolve the knowledge entity server-side | Stable through the window; matches existing chips; per-user and private by construction. Multiple observations can produce several URLs for one knowledge entity. |
| B | Redirect `/entity/:observation_id` → `/entity/:knowledge_entity_id` once linked | One URL per entity, but the URL changes mid-window and the id space becomes global/leaky before a disclosure policy exists. |
| C | A now, plus canonical `/k/:knowledge_entity_id` later | Owner view stays stable; a shareable canonical page arrives with the disclosure policy. |

Recommendation taken: **A** — `/entity/:observation_id` stays the owner view and the knowledge
entity is resolved server-side. B's redirect is avoided until the disclosure policy exists; C
(`/k/:knowledge_entity_id`) is the later canonical page.

Within A, the page needs no redirect; it renders cold or warm depending on whether
`knowledge_entity_id` is set and resolvable. Re-illumination is the one remaining URL hazard: the
observation belongs to a superseded illumination and 404s. Optional later fix: redirect by
`(capture_id, position)` to the current illumination's observation.

## Data flow

1. `get_entity` returns a composite: the observation (`ObservedEntityInfo`, with
   `knowledge_entity` populated) plus the user's **occurrences** of that knowledge entity.
2. `ObservedEntityInfo.knowledge_entity` is populated wherever observations are rendered —
   illumination cards too, so chips can later show status. One batched query per illumination:
   collect non-null `knowledge_entity_id`s, fetch those rows (filtering `merged_into_id IS NULL` once
   merges repoint), map back. No N+1.
3. Occurrences: observations owned by the caller, attached to the same knowledge entity, on each
   capture's current illumination, grouped by capture. Clicked occurrence first.
4. Identities: `KnowledgeEntityInfo` grows `identities: Vec<IdentityInfo>` (`scheme`,
   `external_id`, `provider`, `method`, `confidence`), loaded for the entity ids in one query and
   ordered by confidence. Same batched pattern as the entity join.
5. `relationship` between observation and entity is read **through the owned observation**
   (`get_entity.rs:57`), preserving the access rule.

Cold page does not query the graph at all — there is nothing to ask.

## Rendering

One template, two modes, chosen by `knowledge_entity`:

**Cold** (observation only)

- Name, type, description, platform, canonical search — verbatim.
- A neutral state: “Not linked to the graph yet”, no status pill, no alt names, no occurrences
  beyond the source capture.
- Refresh or navigation picks up the linked view (see “Transient window”).

**Warm** (entity present)

- Knowledge entity as the subject: `name`, `status` (`provisional` / `identified`), `confidence`,
  alt names/types when they differ from the observation.
- The observation as evidence: “this capture said …”, kept visibly secondary.
- “Appears in”: the user's occurrences of the entity across captures (the cross-capture payoff),
  with the clicked one marked.
- Platform link and canonical search prefer the entity's projected values, falling back to the
  observation's.

- Identity chips from `KnowledgeEntityInfo.identities`: `scheme:external_id` (Wikidata links to
  `wikidata.org/wiki/<id>`), with provider/method as the tooltip and confidence as a muted suffix.
  Provisional entities simply have none.
- A “provisional” entity is not a lesser page: the chip states what is known, not what is missing.

## Transient window

The gap is real and short: illumination completes, the link queue runs. The page may be opened
mid-window and must not look broken.

- **Passive:** the page renders once; a refresh or navigation shows the linked view. No polling.
  The cold state says so (“not linked yet”) so the user knows the page is current, not stuck.
- Provisional → identified is likewise a normal snapshot; refresh is acceptable.

## Edge cases

- **Multiple observations of one illumination link to the same knowledge entity.** Page shows the
  clicked observation; occurrences dedupe per capture. Chip dedupe is out of scope.
- **Merge while open.** Observation is repointed; refresh resolves to the winner. No live update.
- **Merged pointer that survived some path other than `merge`.** Resolve through `merged_into_id`
  and render cold if it dead-ends; never show a tombstone.
- **Link failure/retry.** Cold state must be honest (“not linked yet”), not an error. No status
  API for link-run failure is exposed; acceptable for now.
- **Entity with zero identities** is still a knowledge entity (`provisional`); that is the normal
  post-link state and must render as the warm view.
- **Deep links from old feeds.** `current_illumination` filter may 404 a superseded observation;
  accepted until the redirect above.

## Phases

1. Populate `knowledge_entity` in the wire type (batched; illumination cards + get_entity); rename
   the misnamed `get_knowledge_entity` to `get_observation`. Tests for: linked, unlinked, merged.
2. Extend `KnowledgeEntityInfo` with `identities` (batched query; Wikidata/illuminator rows) and
   populate it through the same projection path.
3. Entity page composite: cold/warm branches; siblings → occurrences; entity-first rendering.
4. Occurrences section (“appears in”, per-capture dedupe, clicked-first).
5. Optional: superseded-illumination redirect; canonical `/k/:id` when disclosure lands.

## Non-goals

- Cross-user visibility, global counts, and a shareable canonical entity page: blocked on the
  disclosure policy.
- Chip dedupe/rollup within one illumination.
- Live merge/provisional→identified updates.

## Decisions

1. **Addressing — A.** `/entity/:observation_id` is the owner view; the knowledge entity is resolved
   server-side. Canonical `/k/:knowledge_entity_id` waits for the disclosure policy.
2. **Scope — all of the user's occurrences** of the entity, across captures, clicked one marked.
   No global counts.
3. **Cold upgrade — passive.** One render; refresh or navigation shows the linked view.
4. **Identities — extend `KnowledgeEntityInfo`** with source identities (scheme, external id,
   provider, method, confidence) and render them as links.
