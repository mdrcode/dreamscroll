# Capture page route: `/detail` → `/capture`

**Status:** Shipped (2026-10-25).

## Why

`/detail` did not name the type being examined, unlike `/entity/{id}`. Page
routes should be type-specific and RESTful.

## What moved

| Before | After |
| --- | --- |
| `/detail/{id}` | `/capture/{id}` |
| `/detail/{id}/archive` | `/capture/{id}/archive` |
| `/detail/{id}/unarchive` | `/capture/{id}/unarchive` |
| `/detail/{id}/illumination` | `/capture/{id}/illumination` |
| `/detail/{id}/related/previews` | `/capture/{id}/related/previews` |

The upload JSON field `detail_url` became `capture_url` for the same reason: its
value is the capture page URL.

Renamed with the routes: the page handler moved from `r_detail.rs` into
`r_capture.rs`, which already owned the capture subroutes, and the page shell
`detail.html.tera` became `capture.html.tera`.

Deliberately not renamed: the card partials (`capture_detail.html.tera`,
`detail_illumination.html.tera`), `page_kind = "capture_detail"`,
`feed_content_mode = "detail"`, and the CSS class names. They name rendered
artifacts or presentation modes, not routes; renaming them would churn templates
and styles for no navigational gain.
`/feed/capture/{id}/illumination` is a feed fragment and keeps its path.

No legacy `/detail` alias: single-user prototype, and every consumer is ours
(templates, JS, archive/unarchive form actions, the upload response), all updated
here. An old bookmark now 404s.

## Verification

- `webui::v2::maker` renders the updated form actions and links
  (`action="/capture/7/archive"`, `href="/capture/7"`).
- Live: `GET /capture/1` is protected (redirects to login); `GET /detail/1` is
  404.
