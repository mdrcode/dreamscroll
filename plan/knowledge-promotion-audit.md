# Promotion audit — what the local corpus would actually put in the base graph

**Status:** Complete for the local corpus (2,518 mentions, 2026-10-29). Data and scripts are local
only; this doc holds the method and the aggregate findings.
**Feeds:** [knowledge-public-entities.md](knowledge-public-entities.md) (promotion gate, phases
P-a…P-d).

## Method

Three read-only scripts, no database writes, no product code:

| Step | Script | What it does |
| --- | --- | --- |
| 1 | `classify.py` | Reads every `knowledge_entities` row and scores three axes: **publicness** (public / person-behind-account / unverifiable / unclear), **name safety** (distinctive / needs disambiguation / collides across types), **quality** (definition / behavioural profile / thin). Emits a per-entity CSV plus aggregate summaries. |
| 2 | `probe.py` | Samples up to 30 distinct names per verdict bucket, queries Wikidata `wbsearchentities`, retries `cirrussearch-too-busy-error` with backoff, and records the best candidate plus alternatives. |
| 3 | `score.py` | Re-scores the probe offline with a *containment* metric (how much of Wikidata's short description appears in ours), replacing a Jaccard score that penalised correct matches for being terse. |
| 4 | `handles.py` | Samples distinct X handles and asks X's unauthenticated oEmbed endpoint whether the account exists (JSON = live, 404 / HTML error page = not). Proves the link is real; says nothing about attribution. |

Regenerate with `python3 classify.py && python3 probe.py && python3 score.py && python3 handles.py`
from `localdev/analysis/entity-promotion/`. The full CSV contains real user entity data and is
gitignored (`localdev/*`); this document contains only aggregates and redacted examples.

**The probe is a lower bound, not a verdict.** Description overlap cannot separate "wrong referent"
from "different wording": `S. Truett Cathy` → *American restaurateur, businessman, philanthropist* is
correct, but our description says "founder of Chick-fil-A", so containment is 1/3. A real gate needs
the P31 class mapping and an LLM yes/no, exactly as the design proposes.

## Data basis and known biases

Every number here is computed over **all** `knowledge_entities` rows, 2,518. That includes **149
superseded rows (5.9%)** left behind by re-illumination: 34 of 628 illuminated captures carry two or
three illuminations, and the read path (correctly) exposes only the latest. Current-only mentions are
**2,369**.

Consequences, in order of importance:

1. **The 84 "duplicated inside one capture" groups are all re-illumination artifacts.** Restricted to
   the latest illumination per capture, **zero** groups repeat a name within one run. The model does
   not duplicate entities; running illumination twice does. The fix is supersede-on-write, not a
   dedupe guard.
2. Bucket shares are inflated by at most ~6%, uniformly, so no conclusion changes. To reproduce on
   current-only rows, add
   `WHERE k.illumination_id = (SELECT max(i.id) FROM illuminations i WHERE i.capture_id = k.capture_id)`
   to `classify.py`'s query — the numbers below would shift by a point or two.

## Corpus shape

| Fact | Value |
| --- | --- |
| mentions | 2,518 |
| distinct normalized names | 2,099 |
| users with entities | 3 (one holds 2,486) |
| names appearing under more than one type | 19 |
| names seen by more than one user | 10 |

## Metadata verdicts

| Verdict | Rows | Share | Weighted by repetition | Share |
| --- | --- | --- | --- | --- |
| **promote** — public, distinctive name, definition | 1,756 | 69.7% | 2,976 | 66.2% |
| **promote_linked** — carries a platform link | 333 | 13.2% | 393 | 8.7% |
| **review_disambiguation** — single token / weak name | 304 | 12.1% | 559 | 12.4% |
| **hold_collision** — same name under two types | 63 | 2.5% | 502 | 11.2% |
| **review_thin_link** — linked but no description | 42 | 1.7% | 47 | 1.0% |
| **review_taxonomy** — legitimate concept, no suitable type | 17 | 0.7% | 17 | 0.4% |
| **review_thin** — name but no usable description | 2 | 0.1% | 3 | 0.1% |
| **review_person** — person-shaped, no identifier | 1 | 0.0% | 1 | 0.0% |

By distinct name: 1,480 promote, 307 linked, 242 disambiguation, 41 thin-link, 17 collision, 17
taxonomy, plus 3 thin or unclassified.

**This table reflects the platform-link rule**: a person-shaped entity backed by a platform handle is
a candidate, because the handle is a public, citable identifier (next section). Under the earlier rule
those rows were 158 `review_person` + 41 `reject_private_person`; both buckets are now empty of
person rows and the risk moved from *existence* to *attribution*.

Quality is the surprise: **2,334 of 2,518 descriptions (92.7%) are full definitions**; 136 are
behavioural profiles (all accounts); 48 are thin. The illumination prompt already writes
encyclopedic, third-person text.

## Platform links: the identifier the corpus actually has

**375 rows** carry `platform` + `platform_handle` — 184 `real_person`, 165 `social_media_account`, 14
`organization`, 8 `publication`, 3 `online_community`, 1 `brand`. No row has a platform without a
handle. `x_twitter` accounts for 322 of the 340 distinct handles.

| Property | Value | Consequence |
| --- | --- | --- |
| distinct `(platform, normalized handle)` | 340 for 375 rows | 35 rows are the same account, and reuse **crosses types** (`cremieuxrecueil`, `GraduatedBen` appear as both `real_person` and `social_media_account`) → the handle is a deterministic identity key, like a QID |
| `platform_display_name` present | 340 / 375 | the model usually captured it |
| display name matches the entity name | 251 / 375 exact-ish | strong corroboration when it matches |
| display name differs | 89 / 375 | almost all are accounts named by their handle (`@QuetzalPhoenix` → "Carlos That Notices Things") |
| sampled X handles that resolve | **55 / 60 (92%)** | the link is a real, live identifier, not a hallucinated string |
| ...that do not | 5 / 60, all HTTP 404 | accounts renamed or deleted since the screenshot — the corpus contains throwback posts, so an unverifiable handle is expected, not an error |
| handles violating the platform's own rules (X: ≤15 chars, `[A-Za-z0-9_]`) | **0 / 375** | a hallucinated handle would trip this eventually; none did |
| account descriptions asserting biography (age, location, job, relations) | **0 / 166** | the model describes what the account *publishes*, never who the person *is* |

An unverifiable handle is therefore **not** a downgrade: the screenshot may be a decade old. The existence
check is positive evidence when it succeeds and says nothing when it fails.

### Attribution: does the handle belong to the entity?

The failure mode that matters is not a dead link, it is a live link to the wrong account — a fan
account, a namesake, or the *poster* rather than the *subject* of the screenshot. The corpus has no
case of it:

| Test | Result |
| --- | --- |
| handles attached to two different people | **0** — 6 handles carry two names, and all 6 are the same account described two ways (`@GraduatedBen` / "Dr. Ben Braddock", `bo66ie29` / "Bobbie") |
| captures where two distinct entities share a handle | **0** — in all 7 captures with two linked rows, the rows are the same entity extracted twice |
| `real_person` links corroborated by `platform_display_name` | **168 / 184 (91%)**, once diacritics are normalised |
| `social_media_account` rows named by their handle with a *different* display name | **32** — correctly typed: the account is the entity, the display name is the operator's chosen name (`@QuetzalPhoenix` → "Carlos That Notices Things") |
| rows where the name is the handle, the display name differs, and the type is `real_person` | **1**, and it is a stylisation ("Græghama"), not a different person |

So the model does not invent handles, and it does not mis-type accounts as people either. The handle
names the account; the display name names whoever operates it. The one decision left to us is which
of those the base entity should be — and the answer is the account, because that is what we can cite.

The existence check uses X's unauthenticated oEmbed endpoint: JSON for a live account, an HTML error
page for a missing one. It proves the account exists; it does **not** prove the account belongs to the
entity — that is attribution, and it is checked separately (display name, role description).

## Risk 1 — privacy: the gate changes shape

The platform-link rule retires the 199-row person gate. What replaces it is not "no gate" but a
different one:

1. **The entity is the account, not a biography.** A `social_media_account` base record says "this is
   the account that posts book reviews", which is what the account publishes about itself. It must
   never say "a 34-year-old teacher in Ohio". The audit found zero biographical claims today; that is
   a property to keep, not one to assume.
2. **Attribution is the residual risk — and the corpus does not exhibit it.** No handle is attached to
   two different people, no capture gives two distinct entities the same handle, and no
   handle-named row is mistyped as a person. The model consistently uses the handle for the account
   and the display name for its operator; 168 of 184 person links are corroborated by that display
   name. What remains is the ordinary case of a stylised or variant display name
   (`Alexey Kutepov` → `tsoding` / "Tsøding", corroborated by the description), which the
   description check handles.
3. **Link rot is not an error signal.** 5 of 60 checked handles already 404, and all five are
   well-formed and plausible. Screenshots age faster than accounts; a handle that no longer resolves
   is consistent with a throwback post. Keep the description and provenance, re-check handles on a
   slow cycle, and never delete a base row because its URL died. What *would* be a signal is a
   malformed handle — and there are none (0 of 375 violate their platform's charset or length
   rules).

Bare persons — a name with no handle and no verifiable role — remain rejected in principle. This
corpus contains **one** such row: the extractor attaches a handle to essentially every person it pulls
from a post. So the old "199 rows at risk" was mostly a classification artifact, and the actual
privacy exposure is attribution, not existence.

Examples of the linked cohort (redacted): handles like `@feels…`, `@cardamom…`, `0x0S…`, `captg…`,
attached to a person's name. These are people who post publicly but are not public figures. Under the
rule they are promotable **as accounts**; the display-name check is what decides whether they are also
promotable under the person's name.

## Risk 2 — identity corruption: the larger, subtler problem

12.1% of rows need disambiguation and 19 names appear under two types. The probe shows what happens
when a single-token name resolves without a type check:

| Bucket | Found | Description-consistent | Wrong shape |
| --- | --- | --- | --- |
| review_disambiguation (n=304) | 29 / 30 | 11 | 3 |
| hold_collision (n=63) | 11 / 14 | **9** | 0 |

So single-token names *are* resolvable — but 14 of 29 sampled matches were not corroborated by the
description, and 3 resolved to a family name or given name. When the corpus itself already tells us
the name is ambiguous (`Train Dreams` as both a 2011 novella and a 2025 film; `Ditto`; `Heat`), the
description disambiguates well (9 of 11). **Use the corpus's own ambiguity signal; do not promote a
`needs_disambiguation` name without it.**

## Risk 3 — taxonomy: the `unknown` bucket is not junk, and not one kind

All 17 `unknown` entities are legitimate public things — but they are not all concepts. Fetching P31
for the 16 that resolved shows six distinct classes: 3 are **taxa** (`Dahlia`, *Clostridium
butyricum*, Northern cardinal), 2 are **painting techniques**, and the rest are a movement, an art
genre, a sport type, a demographic cohort, an anatomical class, a human activity, and a schema. Two of
the sixteen are **wrong referents** (`Sword in the Stone` → a 2019 film project, `Social defeat
stress` → a 1996 journal article).

They are not a promotion risk; they are a **promotion blocker**: a publishable base record needs a
type, and `unknown` is not one. Most of them are also *classes* rather than individuals (taxon, type
of sport, class of anatomical entity, art genre, and `dry stone wall`, which Wikidata models as a
subclass of `wall`) — the taxonomy assumes every row is an individual. See
[knowledge-entity-types.md](knowledge-entity-types.md) for the resulting recommendation (`concept` +
`organism` + a `referent_form` axis, and the precise `wikidata_class` kept on the base row).

## What this says about the gate

- **84.6% of rows are promotion candidates** once a platform link counts as a source (2,089 rows, plus
  42 thin-linked). The remaining 387 rows are 304 disambiguation, 63 collision, 17 taxonomy, and 3
  thin or unclassified.
- **Two different source checks, not one.** Wikidata (or a citable article) for the 69.7% public
  bucket — roughly 60–80% corroborated once the metric is fair — and the platform handle for the 13.2%
  linked bucket, which the 92% existence check says is a real identifier. The link is what covers the
  ground Wikidata misses: 307 distinct linked names that Wikidata largely does not know.
- **Attribution replaces existence as the person risk.** Promote the account; promote the *person*
  only when the display name or a role description corroborates the handle.
- **Collisions are rare but concentrated in repeated entities** (2.5% of rows, 11.2% of mention
  volume), because ambiguous names are the ones a user captures repeatedly.
- **Expected base graph from one corpus:** ~1,200–1,500 entities out of 2,099 distinct names — roughly
  900–1,200 Wikidata-corroborated plus ~300 account-backed. That is a real dictionary, and it is the
  shape the base graph should have.

## Open questions this raised

1. **What exactly is the account entity's label?** The handle (`@vdysseys`), the display name
   (`s ᰔ`), or the model's name (`vdysseys`)? Proposal: handle as the identity, display name as the
   label when it is not emoji or punctuation, model name as an alias.
2. **How often is the handle the *poster* rather than the subject?** A screenshot of a tweet about
   someone carries the poster's handle. The display-name check catches the obvious mismatches; a
   capture-level check ("was the handle in the post author slot?") would be better and is cheap if the
   model emits it.
3. **What is the acceptance rule for a single-token name?** The corpus gives a usable signal (name
   under two types, or description names a family/given name). Everything else needs a source that
   disambiguates.
4. **How do we audit promotions without a review queue?** Every promotion has `promotion_sources`; a
   sampled weekly audit of new rows is cheaper than a per-entity queue and catches the same failures.
