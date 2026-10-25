# Feed fragment route rename: `/cards` → `/feed`

**Status:** Shipped 2026-10-24. No open tasks.

## Why

The handler (`r_feed::fragment`), the fragment template
(`partials/feed.html.tera`), the swap target (`#card-feed`), and the page kind
(`data-page-kind="feed"`) all said *feed*; only the URL said *cards*. One name
for one concept.

## What changed

| Surface | Before | After |
| --- | --- | --- |
| `src/webui/v2/maker.rs` | `/cards`, `/cards/capture/{id}/illumination` | `/feed`, `/feed/capture/{id}/illumination` |
| `web/v2/templates/partials/header.html.tera` | feed-mode `action` and `hx-get` | `/feed` |
| `web/v2/static/webui-v2.js` | search `e.detail.path`, `buildFeedUrlFromCurrentState`, feed-illumination refresh | `/feed` |

The non-HTMX canonical redirect is unchanged in behavior: `r_feed::fragment`
still redirects such requests to `/` (and `/` is what a full page load uses),
so only the fragment URL moved.

## Deliberately not renamed

- `partials/cards/*.tera` and the `cards` template variable. They name the
  rendered artifacts, not the route, and every card partial (detail, spark,
  annotation) lives there, not just feed ones.
- No legacy `/cards` alias. Single-user prototype; the only consumers are our
  own template and JS, both updated here.

## Verification

- `cargo test --lib webui::v2::maker` — rendered header assertions pin
  `hx-get="/feed"` on the feed page and its absence on the detail page.
- Live plan docs referencing the old path were updated
  (`plan/htmx4.md`, `plan/feed-view-incremental-updates.md`,
  `plan/testing-auth-routes.md`, `plan/README.md`); `plan/archive/` records
  keep the historical path.
