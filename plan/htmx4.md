# HTMX 4 Upgrade — Evaluation

**Status:** 2.0.11 pinned (`base.html.tera:14`, SRI verified 2026-10-23).
htmx 4.0.0 is not started but is intentionally `next` on npm; 2.x remains
`latest` and is supported indefinitely. No forced timeline.

**Scope:** Assess whether and how to move `web/v2` from htmx 2.0.11 to 4.0.0.
Covers the user-visible gains, the migration points that affect this codebase,
and a concrete checklist. Does not cover htmx 4 extensions we do not use.

---

## 1. Version facts

| Source | Version | Published |
|---|---|---|
| Pinned in `web/v2/templates/base.html.tera:14` | **2.0.11** (with SRI) | 2026-09-22 |
| npm `latest` | 2.0.11 | 2026-09-22 |
| npm `next` | **4.0.0** | 2026-08-28 |

The pin is explicit (no floating CDN URL), so nothing upgrades by accident.
`4.0.0` stays on `next` until roughly early 2027 to avoid force-upgrading apps
that reference unversioned CDN URLs.

The 2.0.11 patch baseline adds `<hx-partial>` (backported from 4.0), fixes OOB
target lookup for detached context elements, and fixes history path
normalization. Those reduce the reasons to jump to 4.0; `<hx-partial>` is
available now if the single OOB swap grows into multiple regions.

Sources:

- [htmx 4.0.0 release announcement](https://four.htmx.org/announcements/2026-08-28-htmx-4.0.0-is-released)
- [What's New in htmx 4](https://four.htmx.org/docs/whats-new-in-htmx-4)

## 2. Why upgrade (for us)

Our usage is deliberately small: five `hx-get`, two `hx-post`, seven
`hx-target`, seven `hx-swap`, two `hx-swap-oob`, one `hx-push-url`, one
`hx-trigger` — nearly all with explicit targets and swaps. We do not rely on
implicit attribute inheritance, `hx-headers`, `hx-include`, `hx-vars`, or
`hx-delete`.

Ordered by practical value here:

1. **Morph swaps (`innerMorph` / `outerMorph`).** The strongest reason.
   The SSE single-card refresh uses
   `htmx.ajax(..., { swap: 'outerHTML' })` (`webui-v2.js:116-134`), which destroys
   the card's DOM: open “More” disclosure, focus, scroll position, video state.
   Morph keeps node identity and listeners and patches only the diff.
   Caveat: morph preserves nodes/listeners, not arbitrary client-only classes
   or inline styles. Our disclosure state (`detailsExpanded`, `is-expanded`,
   inline `max-height`) may still need reconciliation or `hx-morph-skip` /
   `morphSkipChildren`.
2. **`<hx-partial>`.** Available on the 2.0.11 baseline, so not a reason to
   upgrade. It replaces `hx-swap-oob` with explicit per-target
   targeting/swap config. We have two OOB uses
   (`annotation_footer_left.html.tera`, `capture_metadata.html.tera`) and any
   future multi-region SSE update would benefit. Clearer than “magic” OOB
   elements.
3. **`fetch()` internals and streaming HTML.** Enables the rewritten `hx-sse`,
   `hx-multipart`, and `hx-download`. Our SSE is a hand-rolled `EventSource`
   (~60 lines in `webui-v2.js`). Not necessary to change, but it becomes a
   plausible simplification. `fetch()` also gives request abort/timeout.
4. **Explicit inheritance.** Removes “why did this child inherit `hx-target`?”
   bugs. Cost for us is ~zero: targets/swaps are already on the elements that
   use them, and the migration checker finds no real reliance (§3.5).
   `hx-disinherit` / `hx-inherit` disappear.
5. **Standardized events.** All events become
   `htmx:phase:action[:sub-action]`; errors consolidate into `htmx:error` /
   `htmx:response:error`. We listen to three events, so this is mechanical.
   `htmx.onLoad()` remains, now backed by `htmx:after:process` instead of
   `htmx:after:init`; our `onLoad` wiring is forward-compatible.
6. **No localStorage history cache.** Removes a class of stale-DOM bugs from
   third-party mutations. But see §3.3: our `hx-push-url` link changes back
   behavior.
7. **Modern defaults and extensions.** 60s request timeout (was none), 1ms
   settle delay (was 20ms), and extensions such as `hx-preload` (hover
   prefetch), `hx-pending`, `hx-history-cache`.
8. **Smaller core.** Class/lookup/removal helpers removed in favor of native
   DOM APIs. We do not use any of them.

## 3. Migration points (audited against current code)

### 3.1 Event renames

| htmx 2 | htmx 4 | Our site |
|---|---|---|
| `htmx:afterSwap` | `htmx:after:swap` | `webui-v2.js:27` |
| `htmx:configRequest` | `htmx:config:request` | `webui-v2.js:826` |
| `htmx:load` | `htmx:after:init` (via `htmx.onLoad`) | `webui-v2.js:22` |

### 3.2 Error responses now swap

htmx 4 swaps all responses except `204` and `304`. Our web handlers return
`api::ApiError` JSON for 4xx/5xx (e.g. `src/webui/v2/r_annotation.rs`). Today
those errors are ignored by htmx; after the upgrade the JSON body would be
swapped into `#annotation-block-*`. Mitigation: `hx-status:4xx="swap:none"` on
forms (or per-status behavior via `hx-status`), or return dedicated HTML error
partials.

### 3.3 History behavior

The header search form uses `hx-get="/cards" hx-target="#card-feed"
hx-push-url="true"`. htmx 2 restored a cached DOM snapshot. htmx 4 re-fetches
the page on back and swaps it into `<body>` (or `[hx-history-elt]`). Options:
add an `hx-history-elt` around the feed, load `hx-history-cache` (sessionStorage
extension), or drop `hx-push-url` for the feed.

### 3.4 `htmx:configRequest` shape

Our search routing mutates `e.detail.path` and `e.detail.parameters` to force
`/cards` plus feed state. In htmx 4 the event detail is request-context based
(`e.detail.ctx.request.action`, `ctx.request.body`) and GET parameters live in
the URL. This handler must be rewritten, not just renamed; it is the
highest-risk JS migration item.

### 3.5 Checker output (verified)

```
npx --yes htmx.org@4.0.0 upgrade-check ./web/v2/templates --ext .tera
```

Flags 4 sites in 2 files (`annotation_block.html.tera`,
`annotation_editor.html.tera`) as needing `:inherited` on `hx-target` /
`hx-swap`. All four are false positives: each flagged descendant already
declares its own `hx-target` and `hx-swap`, so nothing actually depends on
inheritance. No template changes required for these.

Note the CLI is argparse-based and requires the path **before** `--ext`; also
`.tera` must be added explicitly because the default list only matches
`.html`-suffixed files, and our templates are `.html.tera`.

### 3.6 Attribute changes we do not hit (but should keep in mind)

- `hx-delete` excludes enclosing form data (we use none).
- `hx-disable` means “disable during request”; `hx-ignore` is the old
  “skip processing” (we use neither).
- `hx-vars`, `hx-prompt`, `hx-params`, `hx-ext`, `hx-request`, `hx-history`
  removed.
- `hx-swap` `show`/`scroll` selector syntax split into
  `showTarget` / `scrollTarget` (we use no such modifiers).
- Request/response header formats changed (`HX-Trigger` value format,
  `HX-Trigger-After-Swap` / `HX-Trigger-After-Settle` removed); we do not use
  them.

### 3.7 Templates and delivery

- Update the single `<script>` tag in `web/v2/templates/base.html.tera` to
  `htmx.org@4.0.0` and recompute the SRI hash.
- Run the official checker as shown in §3.5 (`.tera` extension and path order
  matter).

## 4. Suggested migration order

1. Pin 4.0.0 in `base.html.tera` with SRI; run the upgrade checker.
2. Optionally load the `htmx-2-compat` extension first to keep 2.x event names
   and defaults, then migrate incrementally. Drop the extension when done.
3. Rename the three event listeners; rewrite the search `config:request`
   handler against `ctx.request`.
4. Add `hx-status:4xx="swap:none"` (or equivalent) where handlers can return
   JSON errors.
5. Decide history behavior for the feed (`hx-history-elt`, `hx-history-cache`,
   or no push).
6. Switch the SSE card refresh from `outerHTML` to `outerMorph` and verify
   disclosure/focus/scroll behavior; reconcile any client-only state that morph
   discards.
7. Smoke-test: upload → SSE card refresh, annotation add/edit/archive,
   search submit and back button, related-captures load, mobile upload.

## 5. Recommendation

**Not urgent, and cheap when we choose to do it.** We are on the 2.0.11 patch
baseline. Our templates avoid nearly all breaking attribute behavior; the
migration is mostly three event renames, one `config:request` rewrite,
error-swap policy, and the history decision.

The only feature that justifies upgrading on its own is **morph swaps on the
SSE card refresh**. If we want that benefit sooner without a major-version
change, the htmx 2 idiomorph extension can provide it; otherwise adopt 4.0 when
it moves to `latest` or when we next touch the SSE/card rendering path.

## 6. Verification note

The current JS was exercised in Chromium against **htmx 2.0.7**: initial render,
single-card `outerHTML` SSE swap, and `#card-feed` `innerHTML` swap all wire
“More”/metadata toggles via `htmx.onLoad`.

The 2.0.11 bump changes only the pinned URL and SRI; the SRI was recomputed from
the unpkg payload and matches `sha384-2OatzQy1H+Zd/IIrjr1TcuDGqLXeHhbooAyJY1KdQMKnr4LZ22k31GBLdYKHmVjg`.
Re-run the browser harness against 2.0.11 before relying on this note for new
integration work, and against 4.0.0 as part of step 6.
