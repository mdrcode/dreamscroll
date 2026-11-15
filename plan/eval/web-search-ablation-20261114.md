# Web-search ablation — illumination quality, 2026-11-14

**Status:** Measured; recommendation below. The `web_search` task option is implemented and shipped behind a
default of `true`.

**Question.** `google_search` grounding is on for every illumination. It is billed per grounding query and
costs latency. What does turning it off actually cost in output quality?

## Method

| | |
|---|---|
| model | `gemini-3.8-flash`, Vertex global endpoint, thinking level unset |
| prompt | `capture_illumination_v2` (unchanged between arms) |
| ablated arm | `IlluminationTask.web_search = false` → request body carries no `tools` |
| persisted | `persist_projection = false`: every run wrote `illumination_raws` + `inference_metas` only, so no `illuminations` projection and no graph work |
| sample | 104 of the 105 captures that already had a v2 baseline (capture 719 has a run stuck `InProgress` since 2026-11-01, which refuses resubmission) |
| baselines | Oct 19 – Nov 15, all `gemini-3.8-flash`, all carry `params.tools = [{"type":"google_search"}]` (or predate the field); the baseline is the latest **grounded** raw before the batch, so the one pre-batch ablated smoke run (capture 781) is not used as a baseline |

Probe that motivated the design: the same body with and without `google_search` returns
`thought → google_search_call → google_search_result → model_output` vs `thought → model_output`.
`total_tool_use_tokens` is `0` in both, so `inference_metas` cannot tell you whether a run searched —
only the response steps can, and the client drops them.

## Cost, measured

| arm | n | mean wall | out tokens | thought tokens | in tokens | request carries `tools` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| grounded baseline | 104 | 26.8 s | 638 | 843 | 3161 | 104/104 |
| ablated re-run | 104 | 12.1 s | 585 | 692 | 3151 | 0/104 |

−55% wall time, −8% output tokens, −17% thinking tokens. The per-grounded-request fee is not in our
tables at all, so the real cost delta is this plus the provider's grounding charge — $14 per 1,000
queries after 5,000 free per month, measured at 0.75 queries per illumination with the real schema. See
`plan/eval/hydrogen-20261114.md`.

## The noise floor first

20 captures were re-run **grounded**, giving grounded-vs-grounded pairs:

| | |
| --- | ---: |
| entity recall of baseline set | 81/107 = **0.757** |
| pairs with identical entity sets | 8/20 |
| pairs differing only by aliases | most of the 12 differing pairs |

Examples: `Doc Brown` / `Dr. Emmett Brown`, `Jesus` / `Jesus Christ`, `Beat` / `English Beat`,
`Anthony the Great` / `Saint Anthony the Great`. Temperature is unset, so this is pure sampling
variance — and it is large. A single-capture difference is weak evidence; only a pattern across many
pairs means anything.

## Ablation, same metric

| | grounded baseline | ablated |
| --- | ---: | ---: |
| entity recall of baseline set | (control floor 0.757) | 306/498 = **0.614** |
| pairs with identical entity sets | 8/20 (40%) | 24/104 (23%) |
| pairs with any baseline-only entity | 9/20 (45%) | 73/104 (70%) |
| pairs with any new-only entity | 9/20 (45%) | 49/104 (47%) |
| `search_query` fields emitted | 245 | 218 |
| mean entities per capture | 4.08 (control 4.80→4.60) | 3.65 |
| captures returning zero entities | 0 | 2 |
| type agreement on shared names | 80/81 = 0.988 | 302/306 = 0.987 |

Recall drops below the noise floor; naming and typing do not.

## Pair-by-pair judgement

104 pairs, text compared side by side and the screenshot read wherever a claim was contested.
Verdicts in `web-search-ablation-20261114.tsv`.

| verdict | n | share |
| --- | ---: | ---: |
| equal | 73 | 70% |
| worse | 17 | 16% |
| better | 8 | 7.7% |
| mixed | 5 | 4.8% |
| unresolved | 1 | 1% |

### What "worse" is made of

| category | n | example |
| --- | ---: | --- |
| `missed_visible_entity` | 5 | capture 689: `@bo66ie29` legible in the post header, dropped; 671: `BRING IT IN, GUYS!!!` legible, dropped |
| `wrong_identification` | 3 | 683 *Apocalypse Now* → *Wake in Fright*; 704 Barclay → Data; 729 *Josey Wales* → *High Plains Drifter* |
| `missed_artwork_identity` | 2 | 743 Chopin Nocturne Op. 15 No. 1 → "time signature"/"polymeter", no composer; 712 dropped *The Justice of Emperor Otto III* |
| `missed_contextual_identity` / `missed_core_entity` / `missed_named_book` / `missed_all_entities` | 3 | 761 dropped *The End of Everything* (named in the post); 767 dropped the BlūMorpho transmitter / Project Monarch; 708 returned zero entities for the Palast der Republik interior |
| `type_error` | 2 | 726 Instagram account typed `real_person`; 757 *Araucaria angustifolia* typed `place` |
| `unsupported_specificity` | 1 | 669 asserted "Cotswolds honey-coloured Jurassic limestone" for a generic stone cottage |
| `wrong_identification` counted above | 3 | all three discussed under the attribution check |
| `minor_factual_error` | 1 | 695 *September, September* dated 1977 (it is 1978); Civil War volumes 1954–1974 (it is 1958–1974) |

### Attribution check: has this failed before, with grounding on?

`illumination_raws` only starts 2026-10-19; `illuminations` keeps every projection from 2026-10-16.
Picking the *preceding raw* as "the baseline" hid earlier readings, so each of the three
misidentifications was checked against the full projection history:

| capture | grounded readings | ablated | attributable to the ablation? |
| --- | --- | --- | --- |
| 683 | #4 correct (Oct 16), **#7 Wake in Fright (Oct 16)**, #13 correct (Oct 19), #147 correct (Nov 15) | #174 Wake in Fright | **no** — same error produced twice with grounding on |
| 729 | #91 correct (Nov 6) locally; the same *Josey Wales* → *High Plains Drifter* misidentification has been observed in **production** with grounding on | #213 High Plains Drifter | **no** — pre-existing, just not in this corpus |
| 704 | #30 correct (Oct 23), #46 correct (Oct 26) | #195 Data | **yes** — no grounded precedent, identity confirmed by eye (Barclay's human skin and curly hair over the yellow ops uniform, not the gold-skinned Data) |

So 2 of the 3 have grounded precedent; one (704) is a genuine ablation-only misidentification. With a
single attributable instance in 104 pairs the rate is not distinguishable from the grounded arm's own
background rate, but it is the one case where the ablated arm invented an identity set from scratch and
nothing in the grounded history does the same on that capture.

Capture 683 also has a plausible non-tool explanation: capture 697 in the same corpus is a genuine
*Wake in Fright* still (Gary Bond at a bar) that resembles 683's hotel frame, so the confusion is
between two images, and grounding did not prevent it either time.

The same check also **withdrew** one verdict: capture 728 was judged "worse" for dropping Jacob
Epstein, but the same-capture grounded control (raw 154) attributed the portrait to George Charles
Beresford instead. The baseline's attribution was itself unverified, so naming neither is not a loss.

The rest of the 18 "worse" pairs split cleanly under the same check: omissions that earlier grounded
readings did contain (172's two dropped entities, 671, 689, 708, 712, 726, 739, 743, 761, 767) —
those survive as ablation regressions — and mistakes only the ablated arm made (669's Cotswolds, 695's
dates, 726's and 757's types).

### What "better" is made of

| capture | why |
| --- | --- |
| 684 | baseline fabricated *John Singer Sargent* and *Caravaggio* for an anonymous painting; ablation returned no artist at all |
| 700 | baseline read the essay title "Who Watches The Watchmen?" as the Alan Moore comic; ablation named the actual *Stalker* still |
| 731 | Piranesi fan art: ablation named Susanna Clarke's *Piranesi*; baseline read generic capriccio / satyr / howdah |
| 768 | ablation named *Rafał Olbiński*; baseline invented a descriptive title and typed it `art_work` |
| 755 | *Damnation* is 1988; the baseline said 1987 |
| 702 | Pudgy Penguins avatar named by ablation, missed by baseline |
| 593, 682 | extra true cast/crew entities (Porechenkov; Boris Kaufman) |

The grounded arm has its own fabrication mode: attributing a painting to a famous artist, or an essay
title to the work it alludes to. Search does not remove invention; it changes which kind appears.

## Reading

- The demonstrated cost of turning search off is **recall of secondary entities** — handles, cast,
  series, artwork titles that are legible in the image or inferable from it. That is 6 `worse`
  verdicts on visible entities and 3 on a capture's own named subject, out of 104 pairs.
- The **catastrophic class is mostly not attributable to the ablation**: two of the three wholesale
  misidentifications reproduce with grounding on (683 locally, 729 in production). The third (704,
  Barclay → Data) is ablation-only and its true identity is confirmed by eye, so the risk is real in
  kind — but one instance in 104 pairs is not a measurable rate, and nothing here supports the claim
  that disabling search generally raises the factual-error rate.
- Turning search off sometimes **improves** output by refusing to over-attribute (5 of 8 `better`
  verdicts are attributions the grounded arm got wrong or invented).
- It is consistently cheaper and roughly twice as fast.

## Recommendation

1. Keep `web_search` on by default for capture-time illumination, on the recall evidence. Do not cite
   error-rate protection: that part is unproven and one counter-example exists.
   **Decided otherwise 2026-11-15:** grounding is now off by default. The recall finding here still
   stands — it is the trade-off being accepted, recorded in `plan/pragmatism.md` — and the
   thinking-level test (`plan/eval/thinking-levels-20261114.md`) showed `high` recovering most of the
   lost entity density without the per-request fee.
2. Use the flag for cheap re-runs, backfill ablations, and eval arms.
3. If we later want selective grounding, we cannot decide it from our data yet: `inference_metas` never
   records whether a run searched. Recording the interaction's step types (names only) in
   `provider_metadata` is a ~6-line change and would let us measure grounding rate per capture type.
4. Any future ablation should judge against the full `illuminations` history, not the latest raw, so a
   failure that already happens with grounding on cannot be scored as a regression.
