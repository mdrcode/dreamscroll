# Database Clock and Snapshot Watermarks

**Status:** Implemented for task-status SSE card freshness.
**Scope:** Rules for comparing persisted task-status timestamps with rendered
capture-card snapshots. This is not a general database architecture guide.

## Why a DB clock

The WebUI and task worker can run on different Cloud Run instances. Their host
clocks can differ, so comparing an application-generated render time with an
application-generated task event time can produce false ordering. Both values
must be obtained from the same Postgres database clock.

## Render watermark

Before fetching the data used to render capture cards, the route queries
`SELECT CURRENT_TIMESTAMP`. It threads this value through the template context
to the card root as `data-snapshot-at`. A feed render shares one watermark
across its cards. Full index/detail responses, HTMX feed responses, capture-card
partials, and detail partials must all use this ordering: obtain the watermark
first, then read card data, then serialize both together.

The watermark is per-render metadata, not a model/database field. It marks the
start of the read window conservatively: if a task update commits after the
watermark while data is being loaded, its event is newer and may trigger a
follow-up read. It is not a content hash or an HTTP ETag.

## Task event timestamp

Task status rows receive database-managed `created_at` timestamps. On update,
`TaskRunTracker` sets `updated_at = CURRENT_TIMESTAMP` in Postgres and uses the
returned row's `updated_at` as the live `TaskStatusEvent.timestamp`. Snapshot
events use the persisted row's `updated_at` as well. Do not substitute
`Utc::now()` from an application instance for timestamps used in this client
comparison.

Postgres `CURRENT_TIMESTAMP` is fixed at transaction start. This is suitable
for the current short operations and gives both sides a shared clock/domain;
the watermark is taken in its own short query before card reads. If these
operations later move into long-lived explicit transactions or require strict
commit-order semantics, revisit whether `statement_timestamp()` or a database
sequence/version is a better watermark.

## Client rule

For an outcome status that can imply changed visible content, refresh a card
only when the event timestamp is strictly later than the card's watermark:

`event.timestamp > card.data-snapshot-at`

The client ignores events with missing or invalid timestamps rather than
issuing speculative requests. After a partial swap, the returned card carries
the next watermark. This prevents old settled catch-up rows from generating
one request per rendered capture, while preserving a refresh when an update
races with the original read or arrives afterward.

## User-visible leaves

The watermark is independent of a future `leaves_updated_at` model field. If
that field is added, its semantic scope should be user-visible data such as
illuminations, annotations, and future tags; invisible derived data such as
search indexes should not advance it. It is not needed for the current SSE
freshness comparison.

## Validation follow-up

- Old catch-up event timestamp: no card request.
- Event newer than the rendered watermark: one matching partial request.
- Returned partial carries a later watermark: replaying the same event does not
  request again.
- Full page, HTMX feed, feed card partial, and detail partial all serialize the
  DB-generated watermark.
- Live notification timestamp equals the persisted status row's `updated_at`;
  snapshot timestamps also come from that row.