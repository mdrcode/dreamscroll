# Promotion audit — prod corpus (second investigation)

**Status:** Complete for the prod corpus (5,185 mentions, 1,342 captures, 2026-10-30).
Standalone: this does **not** replace the local audit in
[knowledge-promotion-audit.md](knowledge-promotion-audit.md); it is a second
investigation at larger scale, and the comparison is in
[§ Local vs prod](#local-vs-prod).

**Data:** produced by `dreamscroll_api export-entities` against
`https://dreamscroll.ai/api/admin/export/entities`. The classifier in
`localdev/analysis/entity-promotion/classify.py` was imported **unchanged** — only
the loader differs (JSONL export instead of a database dump) — so the two
corpora are scored by identical rules. Raw data and per-row detail are local only.

## Corpus shape

| Fact | Value |
|---|---|
| Mentions | 5,185 |
| Captures | 1,342 |
| Distinct normalized names | 4,490 |
| Entity-producing users | 3 (one user holds 5,073 rows, 97.8%) |
| Entities per capture | mean 3.9, max 26 |
| Current occurrences | 5,157 (99.5%) |
| Stale occurrences | 28 (0.5%) |
| Captures with entities from >1 illumination | 7 |
| Rows in archived captures | 15 |
| Rows without a description | 97 (1.9%) |
| Platform-linked rows | 775 (14.9%) — X 723, Instagram 16, Reddit 15, YouTube 10, Threads 5, TikTok 1, other 5 |

## Verdicts

| Verdict | Rows | Share | Weighted mentions | Share |
|---|---|---|---|---|
| `promote` | 3,675 | 70.9% | 5,096 | 66.5% |
| `promote_linked` | 678 | 13.1% | 861 | 11.2% |
| `review_disambiguation` | 577 | 11.1% | 1,071 | 14.0% |
| `hold_collision` | 112 | 2.2% | 463 | 6.0% |
| `review_thin_link` | 97 | 1.9% | 111 | 1.4% |
| `review_taxonomy` | 36 | 0.7% | 38 | 0.5% |
| `review_thin` | 5 | 0.1% | 14 | 0.2% |
| `review_person` | 5 | 0.1% | 5 | 0.1% |

Promotion candidates (`promote` + `promote_linked`): **4,353 rows, 84.0%**.

## Redundancy

| Shape | Count |
|---|---|
| Names repeated across captures | 408 |
| …of those, type agrees | 376 (92.2%) |
| …of those, type conflicts | 32 (7.8%) |
| Names repeated within one capture | 28 |
| Excess mentions (repeat occurrences) | 695 |
| — cross-capture | 665 (95.7%) |
| — re-illumination | 22 (3.2%) |
| — same illumination | 8 (1.2%) |

## Types, and what each row needs

Column meanings: **today** = `promote` + `promote_linked`; **gate** = held only for
name shape (+ 5 empty descriptions), so recoverable by replacing the shape test;
**enum** = needs `organism`/`concept`; **split** = a colliding string that becomes
two or more nodes; **ident** = has a platform handle but no description.

| Type | Rows | Today | Gate fix | Enum | Split | Ident |
|---|---:|---:|---:|---:|---:|---:|
| `real_person` | 1,829 | 1,753 | 18 | — | 4 | 54 |
| `organization` | 455 | 373 | 61 | — | 21 | — |
| `book` | 442 | 403 | 32 | — | 7 | — |
| `place` | 409 | 327 | 81 | — | 1 | — |
| `social_media_account` | 333 | 285 | — | — | — | 48 |
| `software` | 285 | 129 | 128 | — | 28 | — |
| `movie` | 235 | 183 | 45 | — | 7 | — |
| `fictional_character` | 208 | 149 | 55 | — | 4 | — |
| `publication` | 158 | 124 | 28 | — | 6 | — |
| `product` | 151 | 114 | 27 | — | 10 | — |
| `brand` | 131 | 77 | 40 | — | 14 | — |
| `event` | 120 | 112 | 7 | — | 1 | — |
| `art_work` | 107 | 98 | 7 | — | 2 | — |
| `television_show` | 92 | 81 | 8 | — | 3 | — |
| `music` | 74 | 62 | 12 | — | — | — |
| `meme` | 49 | 39 | 10 | — | — | — |
| `unknown` | 40 | — | — | 36 | 4 | — |
| `online_community` | 32 | 24 | 8 | — | — | — |
| `religious_figure` | 23 | 10 | 13 | — | — | — |
| `financial` | 12 | 10 | 2 | — | — | — |
| **Total** | **5,185** | **4,353** | **582** | **36** | **112** | **102** |

Read as a progression: **84.0%** promotable today → **95.2%** after the shape test
is replaced and empty descriptions are re-asked → **95.9%** once the enum covers
kinds. The 112 collision rows are not lost either; they become ≥2 nodes each. What
remains is the 102 identifier-only rows, about half of which resolve by handle
lookup.

Where the gate bites is structural, not random: `software` (128), `place` (81),
`organization` (61), `fictional_character` (55) — all types whose names are
frequently single short tokens (Zoom, Git, Rust, Ardennes, Ghent, Elsevier).

## Local vs prod

| Metric | Local (2,518) | Prod (5,185) | Delta |
|---|---|---|---|
| `promote` | 69.7% | 70.9% | +1.2pp |
| `promote_linked` | 13.2% | 13.1% | −0.1pp |
| Candidates | 83.0% | 84.0% | +1.0pp |
| `review_disambiguation` | 12.1% | 11.1% | −1.0pp |
| `hold_collision` | 2.5% | 2.2% | −0.3pp |
| `review_taxonomy` | 0.7% | 0.7% | 0 |
| Platform-linked | 14.9% | 14.9% | 0 |
| `unknown` share | 0.68% | 0.77% | +0.09pp |
| **Stale occurrences** | **5.9%** | **0.5%** | **−5.4pp** |
| Excess mentions | 384 (15.2%) | 695 (13.4%) | −1.8pp |
| …cross-capture | 293 | 665 | — |
| …re-illumination | 91 | 22 | — |
| …same illumination | 0 | 8 | +8 |
| Names per capture | 4.1 | 3.9 | −0.2 |

## Findings

1. **The gate is stable across a 2× corpus.** Every verdict share moved by ≤1.2pp.
   The classifier is measuring the model's output distribution, not an artefact
   of one person's capture history — which is what the design needed before
   trusting the promotion bar. A one-corpus result would not have settled this.

2. **The stale-row problem largely disappears in prod (5.9% → 0.5%).** The local
   figure was inflated by a dev environment that had been re-illuminated
   repeatedly (34 captures with 2–3 illuminations); prod has 7. Consequence for
   the design: resolving only the latest illumination is still correct, but it is
   a **0.5% cleanup, not a 6% one** — it should not carry weight in the phase-1
   payoff estimate.

3. **Cross-capture redundancy is the real duplicate mass: 12.8% of rows** (665 of
   5,185) sit in 408 repeated-name groups. This is consistent with the local
   11.6%, so the phase-1 "collapse duplicate mentions" estimate holds at scale.

4. **Name equality is not identity — now demonstrated within one capture.** 32 of
   the 408 repeated names (7.8%) carry conflicting types across captures, and all
   8 same-illumination repeats are *homonyms*, not extraction duplicates: a novel
   and its film adaptation, a magazine and its film, a city and a film named
   after it. The local corpus's "0 duplicates inside one illumination" was luck,
   not a property of the prompt. Canonicalization must never merge on normalized
   name alone, even inside a single capture.

5. **The taxonomy gap is kind-shaped, not instance-shaped.** `unknown` is only
   0.77% of rows, but its content is almost entirely *kinds*: art and design
   movements, techniques, principles, theories, fallacies, metrics (≈30 rows),
   and organisms — genera, species, cultivars, a racehorse (≈8 rows). No row needs
   a new *instance* type. This is the case for `concept` + `organism` +
   `referent_form`, and against growing the enum with more concrete nouns.

6. **Two existing types have no definition, and the model notices.** `brand` vs
   `organization` (6 conflicting groups: streaming services, carmakers,
   broadcasters, cloud suites) and `product` vs `software` (7 groups: chat apps,
   code hosts, games, platforms) account for 13 of the 32 type conflicts. Both
   are cheap to settle with prompt rules; neither needs a new type.

7. **Organisms are systematically misfiled as `product`.** A single capture family
   (dahlias and asters) yields 7 cultivar rows typed `product` and 4 genus/species
   rows split between `product` and `unknown`. Under `product`, a plant collection
   reads as merchandise and would merge against real products.

8. **Attribution behaviour is stable.** Platform-link coverage is identical
   (14.9%) in both corpora, and the account-vs-person split behaves the same way
   (~5 conflicting groups, all pairs where the account and the person are the same
   referent). The attribution rules transfer; the fix is aliasing, not typing.

## Type/kind recommendation

**Add `organism`.** 8 rows in the `unknown` bucket and 11 more misfiled under
`product`/`brand`, all of them taxa, cultivars or a named animal. Genus and
species are `referent_form=kind`; a named cultivar is `kind`; a named racehorse is
`individual`.

**Add `concept`.** ≈30 of the 40 `unknown` rows are movements, styles,
techniques, principles, theories, fallacies, metrics, cohorts, a sport and a legal
category. `knowledge-entity-types.md` reached the same conclusion on a 17-row
base; the prod corpus makes it a 30-row, multi-genre case, and confirms the shape:
kinds rather than instances, which is what `referent_form` records.

Evidence — fifteen targeted Wikidata lookups (14 resolved):

| Row | Class | `referent_form` |
|---|---|---|
| Tyrannosaurus rex, Northern cardinal, Dahlia, Callistephus chinensis, Allium schubertii | fossil taxon / taxon (`Q23038290`, `Q16521`) | `kind` |
| Dahlia 'Kelsey Annie Joy' (cultivar) | **no Wikidata item** | `kind` |
| Up to the Mark (racehorse) | racehorse (`Q726`) | `individual` |
| Gothic Revival, Streamline Moderne | architectural movement / art movement | `kind` |
| Fencing | sport (`Q31629`) | `kind` |
| Generation Z | cohort (`Q15925549`) | `kind` |
| H-1B visa | visa category (`Q170404`) | `kind` |
| Kaizen | word/term | `kind` |

Two things this settles. First, the kinds are real classes with different
Wikidata parents (art movement, sport, cohort, visa category) — which is exactly
why `wikidata_class` earns its place: the UI can say "architectural movement"
without a type per noun. Second, the same lookup demonstrates finding 4 from the
other side: `Chiaroscuro` resolved to a **live album**, not the painting
technique. Naive search-based identity has the same homonym problem as merging on
names.

**Add `referent_form` and `wikidata_class`.** Every disputed row above is a
kind/instance question (the Dahlia genus vs a named dahlia cultivar; "Space art"
the genre vs a work), not a question of a missing noun.

**Do not add** a `sport`, `document`, `paper` or `cohort` type for the handful of
rows that suggest them; `concept` (or `publication` for a paper) absorbs them.
Full per-row detail lives in the local review queue
(`localdev/analysis/entity-promotion/type-review.md`).

## What I would do next

1. **Prompt rules for `brand` vs `organization` and `product` vs `software`** —
   cheapest change with the largest consistency win (13 of 32 conflicts).
2. **Ship `organism` + `concept` + `referent_form`** and re-run this audit; the
   `unknown` bucket should fall under 0.2% and the dahlia cluster should type
   cleanly.
3. **Use `search_query` in entity resolution** — it is populated on 52% of rows
   and currently read by nothing; it carries exactly the disambiguation context
   ("Cure 1997 film") that label search cannot reach. See
   [knowledge-promotion-frontier.md](knowledge-promotion-frontier.md).
4. **Fix the analyser's `name_is_handle` flag** — it normalizes away spaces, so
   "First Last" matches the handle `firstlast`; 167 of 348 prod hits are human
   names. It inflates the account-shaped count by roughly half.
5. **Score duplicates on `(name, type)`**, not name alone: 8 same-illumination
   pairs and 32 cross-capture groups are homonyms, and counting them as
   duplicates overstates extraction noise.
6. **Disambiguate before merging.** The merge path needs a similarity signal
   (description, co-mentioned entities, platform link) in addition to the
   normalized name — finding 4 is the evidence.

## Caveats

- **One user.** 97.8% of rows belong to a single account, so no multi-user claim
  is supportable; cross-user overlap remains unmeasured by design.
- **v2 only.** Every prod capture is a v2 illumination, which is why the stale
  rate is so low compared with the mixed local environment.
- **No LLM calls, no media.** The export carries entity text only.
- **Wikidata sampled, not swept.** Fifteen targeted lookups (14 resolved), not a
  corpus-wide probe.
