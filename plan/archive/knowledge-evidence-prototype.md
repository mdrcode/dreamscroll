# Entity evidence model — prototype report

**Status:** prototype, complete enough to judge feasibility. **Not** a production
design. The owner's verdict (2026-10-31): the direction has merit; a robust
implementation will be built later. Nothing here is shipped; the analysis code is
local-only (see [Where the code lives](#where-the-code-lives)).

**Question this prototype answers:** can we reliably identify the entities our
model extracts against external authorities (Wikidata, platform identities), and
what evidence does each entity carry?

**Supersedes nothing.** [knowledge-promotion-audit.md](knowledge-promotion-audit.md)
and [knowledge-promotion-audit-prod.md](knowledge-promotion-audit-prod.md) remain
the records of the *verdict classifier*; this is the different model that replaced
it for prototyping. [knowledge-promotion-frontier.md](knowledge-promotion-frontier.md)
is the design argument this implements.

---

## 1. Why a prototype at all

The first classifier collapsed everything into one verdict
(`promote` / `review_*` / `hold_collision` / `reject_*`). That is one ordinal score
for several independent facts, and it failed in a way that took a week to notice:
**577 rows were held for "needs disambiguation", and zero of them collided.** They
were held because the name was a short string (≤8 characters), which the gate used
as a proxy for "common word".

A stored verdict cannot be re-derived when the rule turns out to be wrong. So the
prototype stores *evidence* and derives the judgement: change a rule, re-derive.

## 2. The model

Four kinds of thing, deliberately kept apart:

| Kind | Examples | Where | Stored? |
|---|---|---|---|
| **Evidence** — facts we hold | `ontology_match(source, qid, method, confidence)`, `platform_link(site, handle)`, `description(text, authored_by)` | per node | yes, append-only |
| **Aggregate** — computed over a cluster | `multi_source(distinct_users)` | derived | no |
| **Relation** — between two candidates | `collision(a, b, reason, score)` | pairwise | yes, but as pairs |
| **Decision** — per field | can we publish the *name*? the *description*? the *identifier*? | derived | no |

Per-field matters because the fields disagree: 624 rows are identified by platform
handle with no authority match, and 13 are identified with no description at all. A
single score has to throw one of those away.

## 3. Measured on prod (2026-10-31, 5,245 mentions)

Corpus: 5,245 rows · 1,360 captures · 3 users · 4,500 name groups. One user holds
5,113 rows; a second holds 48 (a different content genre — see §6).

| Dimension | Value | Rows |
|---|---|---|
| platform link | handle present | 785 (15.0%) |
| platform link | handle → Wikidata (`P2002`) | 107 (2.0%) |
| ontology | **identified** (one plausible candidate) | **2,131 (40.6%)** |
| ontology | **ranked** (several; description chose one) | 519 (9.9%) |
| ontology | ambiguous (several; nothing separated them) | 591 (11.3%) |
| ontology | none | 1,816 (34.6%) |
| collision | participates in a pair | 220 (4.2%) |
| multi-source | >1 user behind the name | 75 (1.4%) |
| description | publishable | 5,137 (97.9%) |
| description | missing | 97 (1.8%) |
| description | private identifier found | **0** |

**Identified + described** (ready for a base layer): 2,116 (40.3%).
**Identified or ranked**: 50.5%.

Per user: 43.2% identified for the primary account, **14.6%** for the aesthetic
corpus — the long tail is concentrated exactly where the content genre is
style/craft (cultivars, small farms, studios).

Collisions split cleanly, which is the framework's main dividend:

| Collision reason | Pairs |
|---|---|
| **type instability** (same referent, disagreeing type) | 111 |
| genuine homonym evidence (different referents) | 68 |

So 62% of "collisions" are the `brand`/`organization` and `product`/`software`
problem, not duplicates.

## 4. Resolution strategy

Three candidate sources, merged rather than ordered:

1. **Exact label, batched.** One SPARQL query per 50 names returns candidates *with*
   their descriptions and `P31` classes, so the type filter can run before any cap.
2. **Full-text on `search_query`.** `KnowledgeEntity.search_query` is populated on
   ~52% of rows and was previously read by nothing. It carries exactly the
   disambiguation the label index cannot reach: `"Cure 1997 film"` → Q1144476.
   This API call is per-query (not batchable), so it is only used where a name has
   no type-plausible candidate.
3. **Handle → item** via `P2002` (batched SPARQL). Identity with no text at all.

Then **rank** the plausible candidates by description-token overlap, and only call
one when it clears the runner-up by a margin. That produces the `ranked` tier;
without it, several candidates were simply `[0]` — arbitrary, and wrong (`Django`,
a Python web framework, resolved to a 2021 video game).

Measured: exact-label coverage 68% of names; full-text adds candidates for ~45% of
the names that lack one; 107 of 785 handle-bearing rows resolve through `P2002`.

## 5. The failure catalogue — the most valuable output

Every one of these was found *after* the pipeline looked like it worked. They share
one shape: **a value trusted outside the context that produced it.**

| # | Heuristic | Symptom | Root cause | Rule |
|---|---|---|---|---|
| 1 | Name length (`≤8 chars`) ⇒ "needs disambiguation" | 577 rows held; **0** true collisions | length used as a proxy for commonness | never proxy identity with a cheap string property |
| 2 | Take the top authority hit | Carhartt → "family name"; Odyssey → an album; Kirby → a given name | `wbsearchentities` matches labels only, and hit #1 is the most *popular*, not the most *correct* | use the whole candidate set, then verify |
| 3 | Query the lowercased name | label hit rate 23% instead of 68% | grouping normalization leaked into a case-sensitive index | normalize for grouping, **never** for lookup |
| 4 | `"handle"@en` in SPARQL | 0 of 646 handles resolved | `P2002` values are plain literals, not language-tagged | know the property's datatype |
| 5 | Cap candidates at 10, then filter by type | Django → a video game; the right item fell outside the cap | cap applied before evidence | filter first, cap last |
| 6 | `human` in the `religious_figure` keyword set | Jesus Christ → "American Internet personality" | a predicate true of every person filters nothing | a filter word that matches everything is not a filter |
| 7 | Cache keyed by query text | one new name refetched every batch; a stale negative outlived its fixed query | cached the *question*, not the *fact* | cache facts, tag them with the query shape that produced them |

Each is now guarded: the analyser warns when the label hit rate is implausibly low;
cache rows carry a `src` tag so a negative cannot outlive its query; and the
re-run assertion (`requests_this_run == 0`) proves the cache is warm.

## 6. What the framework already changed

- **Genres differ.** The primary account resolves at 43%; the aesthetic account at
  15%. Corpus-wide averages hide this entirely, and it is the strongest argument for
  per-user evidence rather than a global promotion rate.
- **Type instability dominates collisions** (111 of 179 pairs), which reframes the
  merge problem as a taxonomy problem.
- **The long tail is real but smaller than it looked.** Of the 1,816 rows with no
  plausible candidate, 444 had exact-label items that the keyword filter rejected,
  and 247 of those had exactly **one** item with that label. The "unidentified"
  figure is inflated by the filter, not by Wikidata's coverage.

## 7. Where the code lives

Everything is under `localdev/analysis/entity-promotion/` (gitignored; entity text
must never enter the public `plan/` tree):

| Path | Contents |
|---|---|
| `prod/entities.jsonl` | the export (via `dreamscroll_api export-entities`) |
| `evidence/wd.py` | Wikidata access + item-level cache (`wikidata_cache.sqlite`) |
| `evidence/resolve.py` | label → full-text → handle resolution; writes `resolved.jsonl` |
| `evidence/rules.py` | type ↔ authority-class plausibility (**the weak part**, see §9) |
| `evidence/analyze.py` | derives dimensions + per-field publishability |
| `evidence/make_sheet.py`, `score.py` | the precision-measurement harness |
| `evidence/README.md` | run recipe and cache invariants |
| `type-review.md` | per-row type queue (taxonomy), gitignored for the same reason |

## 8. Measurement in progress

A 100-row hand-labelling sheet (`review/precision_sheet.csv`) is generated but
**unlabelled**. Strata: 45 identified, 25 ranked, 15 ambiguous, 15 unidentified.
Filling `verdict` (y/n/?) and `answer_qid` then running `score.py` produces
precision, a ranking-vs-retrieval split of the errors, the false-negative rate, and
ambiguity recall.

Decision rule agreed in advance: **precision ≥ 0.90** → the dimension can drive
promotion; 0.75–0.90 → keep it but add a confidence field; **< 0.75** → the
verification step needs rework before anything is published.

**Until that sheet is scored, every number in §3 is unvalidated.**

## 9. What productionising would require

1. **Replace the keyword filter with `P31` class matching** — a curated
   `type → {class QIDs}` map tested with a `wdt:P31/wdt:P279*` closure query. The
   keyword list is the largest remaining source of false negatives (444 rows) and
   of permissiveness (`video game` counts as `software`).
2. **Confidence per dimension**, not booleans: `unique` from one candidate and
   `unique` after a filter are not the same claim.
3. **Evidence storage with provenance** — `(node, dimension, source, method,
   observed_at, confidence)` — plus a revalidation TTL. Items merge, handles are
   renamed, accounts are deleted.
4. **A user-assertion surface**: the only evidence for the 1,816-row tail, and the
   only evidence that can never be derived from public data.
5. **Rate-limit and etiquette policy.** Caching makes steady state free; a cold
   backfill of a large corpus is a different matter on donated infrastructure.
6. **Attribution policy.** Wikidata statements are CC0; Wikipedia prose is CC-BY-SA.
   The intent is strong attribution, but "universal description" needs a licensing
   decision before it is written into a shared layer.
7. **`multi_source` must stay a ranking signal.** One user today (75 rows of
   cross-user overlap); volume is popularity, never identity.

## 10. Open questions

- Is `organism`/`concept` the right enum change, and does `referent_form` cover the
  remaining kind/instance cases? See [knowledge-entity-types.md](../knowledge-entity-types.md).
- Do we split collisions into per-referent nodes, or hold them? The type-instability
  finding (111 of 179) argues for fixing types first.
- What is the promotion bar for the long tail — user assertion alone, or nothing?
- Which domain authorities per type (RHS for cultivars, TMDB for film, MusicBrainz
  for music, OpenLibrary for books)?

## References

- [knowledge-canonical-entities.md](../knowledge-canonical-entities.md) — the design this feeds
- [knowledge-promotion-frontier.md](knowledge-promotion-frontier.md) — why the gate was reframed
- [knowledge-promotion-audit-prod.md](knowledge-promotion-audit-prod.md) — the classifier-era numbers
- [knowledge-entity-types.md](../knowledge-entity-types.md) — taxonomy
