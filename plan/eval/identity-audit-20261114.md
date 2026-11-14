# Identity audit — 2026-11-14 (guarded title credit)

**Build:** `TIER_PRIOR` title > label, title surface credited unless an item is labelled our name;
label corroboration unchanged (`fit > 0` or title). Graph rebuilt: 1,698 identities.

**Sample:** 200 live entities, deterministic random (`md5(id || '20261114')`), from 2,310 live rows.

| stratum | n |
| --- | ---: |
| identified via Wikidata | 141 |
| identified via platform handle only | 28 |
| provisional | 31 |

## Identification precision

| measured on | correct | wrong | precision | 95% CI (normal approx.) |
| --- | ---: | ---: | ---: | --- |
| Wikidata identities | 129 | 12 | **91.5%** | 86.9 – 96.1 |
| all identified (Wikidata + handle) | 157 | 12 | **92.9%** | 89.1 – 96.8 |

Handle-only identities are correct by construction: the annotation is the observation's own link.

### The 12 wrong identifications

| entity | held | why wrong | before this build? |
| --- | --- | --- | --- |
| The New York Times Opinion | Q128136966 | a scholarly article *about* the opinion pages, not the department | pre-existing |
| Suspicion | Q1197803 | Dürrenmatt's 1952 novel, not Matsumoto's | pre-existing |
| The Jokers | Q3209183 | the 1967 film, not Cossery's 1964 novel | pre-existing |
| Monocle | Q12721 | a corrective lens, not the magazine | pre-existing |
| August Vogel | Q17479554 | American businessman, not the German sculptor | pre-existing |
| Frank Long | Q5488004 | Tasmanian prospector, not the AI investor | pre-existing |
| Craig Lawrence | Q24851767 | British Army general, not the clean-tech VC | pre-existing |
| Pearl Rose | Q76132130 | a peerage record, not the comedian | pre-existing |
| Withnail | Q135920236 | a British comedian, not the *Withnail & I* character | pre-existing |
| Bill the Butcher | Q4346116 | the real William Poole; the entity is the film character | **reassigned by this build** (was the film item) |
| Group Relative Policy Optimization | Q113840014 | "policy-gradient method" — the class, not GRPO | **new (one of the 33 gains)** |
| Social defeat stress | Q4764954 | "animal models of depression" — related domain, not the paradigm | **new (one of the 33 gains)** |

Ten were wrong before the change and are untouched by it. Of the 33 gains the build produced, two are
granularity mismatches (a class instead of a method; a related research area) and one reassignment
(William Poole) is still not the fictional character. That is ~6% error among the gains.

**Dominant error class:** an exact or near-exact surface on a name shared by several referents —
people (`Frank Long`, `Craig Lawrence`, `August Vogel`, `Pearl Rose`), works and their adaptations
(`Suspicion`, `The Jokers`), and a common noun (`Monocle`).

## Provisional audit — "should this have been identified?"

31 provisionals, inspected individually.

| verdict | n | examples |
| --- | ---: | --- |
| correctly left provisional — no candidate, or only wrong candidates | 24 | `Honda EU2200i`, `MMapper`, `The People of Providence` |
| **should have been identified** | **5** | see below |
| genuinely ambiguous even for a person | 2 | `Social Security` (agency vs programme vs Act), `Saint Michael's Bridge` (Scotland vs Ghent) |

| entity | correct candidate | why it was missed |
| --- | --- | --- |
| 4chan /g/ | Q131144730 (4chan technology board) | tied 0.922 with Q63206994 (the site itself) — board vs parent at the same score |
| Joséy Wales (character) | Q6288557 (fictional human) | correct item at 0.750; a Jamaican singer item at 0.800 was refused too, and the eligible remainder produced a thin/tied result |
| Giorgio Armani (brand) | Q125182769 (brand owned by Giorgio Armani S.p.A.) | tied 0.045 with the fashion house and the designer |
| The Lowdown (TV show) | Q130601776 (2025 American series) | tied exactly with Q7749115 (British 1990s series); no year on the entity to separate them |
| Bring It In, Guys! (meme) | Q138753108 (track #95 in the Undertale soundtrack) | scored 0.500, on the floor; refused by a hair |

Missed rate: **5/31 = 16%** (2 more are disputed). If identified, coverage would go 169/200 → ~174/200.

**Dominant miss class:** a tie between related items at different granularity (board vs site, brand vs
company, series vs series, bridge vs bridge). The `MARGIN` rule correctly refuses when the evidence is
equal — the gap is that nothing prefers the granularity the entity type names.

## What this argues for next

1. **Granularity preference** — when the entity type names a class (`brand`, `online_community`,
   `television_show`), prefer the candidate whose classes match that granularity over its parent or
   sibling. Would fix `/g/` and `Giorgio Armani`; needs measurement.
2. **Year from the candidate description** to separate same-name works with different years
   (`The Lowdown`) — the entity usually carries no year, so this is a measure change, not just a gate.
3. **The label gate remains the coverage hole** (`Joséy Wales`, Jake/Elwood): exact label with no
   description corroboration, while a worse-scoring wrong item is also refused. The safe fix is
   semantic corroboration, not the search-agreement disjunct already measured and rejected.

## Status: parked (2026-11-14)

Identity scoring is at a local optimum for this phase. The audit above is the evidence: ~92%
precision on a fresh sample, fixed set unchanged, and the remaining errors split between
same-name disambiguation (needs semantic corroboration), granularity ties (needs a type/level term),
and the label gate's coverage hole. None of those is a threshold to nudge, and further rule work has
shown diminishing returns against the two instruments we have.

The next material lever is a different resolver shape, not more tuning: an LLM-agent identity
resolver that can read descriptions and compare them semantically. Reopen this file when that is
attempted, when a larger labelled set exists (the 200-row instrument is too small to tune against),
or if precision on a fresh sample drops below ~90%.

