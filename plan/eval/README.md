# Identity evaluation sets

Two files, two jobs. Do not merge them.

## `identity-labels.tsv` — the instrument

A random sample of entities, judged once with their claimed identity. It measures precision:
`kept` / `avoided` / `still wrong`. Frozen on purpose — if we add cases we happen to care about,
the sample stops being random and the confidence interval stops meaning anything.

## `identity-hard-cases.tsv` — curated hard cases

Curated, one row per reviewed capture-time failure. It is **not** a precision estimate: it is a
memory of specific things the resolver got wrong, with the reason, so a scoring change can be
checked against them and so the same failure is not rediscovered.

Columns:

| column | meaning |
| --- | --- |
| `capture_id` | the capture that surfaced it |
| `entity_id` | knowledge entity at judgement time (ids are generation-scoped; the name is the stable key) |
| `name`, `entity_type` | what the graph held |
| `expected_qid` | the item a person judges correct, `-` when none exists, `?` when the case is genuinely ambiguous |
| `observed_qid` | what the resolver wrote, `-` when it refused |
| `category` | why it failed, from the list below |
| `note` | the mechanism, with scores, so it can be argued with |
| `judged_at` | date of the human judgement |

Categories:

- `title_not_credited` — a title answer found the item, but the surface term compared the item's
  **label** to the name, so a synonym/redirect ("Battle of Singapore" for "Fall of Singapore")
  scored 0 and the candidate became ineligible. Fixed 2026-11-14: a page title now carries the
  surface, but only when no item is literally labelled our name.
- `title_miscredited` — the inverse: the page title is a redirect to a different concept ("QMD" →
  quantum electrodynamics), and title credit hands it the surface it should not have.
- `label_needs_description` — an exact label (often from label + search) was refused because the
  label rule wants description corroboration (`fit > 0`), and the descriptions share no tokens
  ("lead vocalist" vs "fictional musician").
- `margin_tie` — two or more candidates fit within `MARGIN` (0.05), so none is chosen. Track which
  ones had the correct item on top.
- `no_candidate` — the answers contained nothing name-bearing.
- `thin_evidence` — a candidate was admissible but scored below `SCORE_FLOOR`.
- `account_rule` — a profile identified only by its platform link; a name-based QID would be wrong.
- `accepted_loss` — reviewed and accepted; no correct item exists, or the ambiguity is real.

Add a row when a capture is inspected and something looks wrong. Judge it once; do not rewrite a
row because a later build changes the outcome — add a new row for the new build if it matters.

# Illumination output ablations

Separate from the identity sets above, because it measures the *illuminator*, not the resolver.

| file | job |
| --- | --- |
| `web-search-ablation-20261114.tsv` | one row per capture: the grounded baseline vs the same capture re-run with `web_search: false` |
| `web-search-ablation-20261114.md` | the method, the cost table, the grounded-vs-grounded noise floor, and what each verdict class is made of |

Reproduce an arm with `dreamscroll_api illuminate <capture_id> --web-search false
--persist-projection false`; nothing projected, so the graph is untouched. Keep the baseline raw
in place — the pair is the instrument.

# Thinking level

| file | job |
| --- | --- |
| `thinking-levels-20261114.md` | low / medium / high on `gemini-3.8-flash` with grounding off: cost ladder, cross-arm agreement, and the confusable-still swap |
| `illumination-readings.jsonl` | every illumination reading in one shape — `capture_id`, `arm`, `model_id`, `thinking_level`, `web_search`, tokens, duration, and the parsed entities/summary/details. **Generated, not committed** (~2 MB); rebuild it with the script below |

Regenerate the readings with `util/export-illumination-readings.py` (read-only, local dev DB). New
arms — including a different model — append in the same shape, and comparisons are joins on
`capture_id`.

# Model comparison

| file | job |
| --- | --- |
| `hydrogen-20261114.md` | Flash 3.8 medium (prod flow) vs Gemini 4 Hydrogen low/medium on all 104 captures: token totals, per-capture and per-1,000 price, latency, and what the outputs look like |

The readings file already carries `model_id`, so a model comparison is a filter on that column, not a
separate dataset. Prices used are in the doc; they are list prices and must be checked against the
Vertex bill.
