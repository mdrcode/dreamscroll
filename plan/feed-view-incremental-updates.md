# Incremental Feed-View Updates

**Status:** Planned follow-up; substantial product and implementation work, not part of the current SSE scope.
**Goal:** Keep an open feed view synchronized with server-defined membership without reloading and retransferring the full feed.

## Problem

After a successful upload, the current timeline path calls `reloadFeedFrame()` and fetches the entire `/cards` feed. This is semantically correct—the server recomputes which cards belong in the current view—but can transfer hundreds of kilobytes when only one capture was added.

Simply fetching `/cards/capture/{id}` and prepending the result is not correct in general. The capture might be archived, outside the timeline's current limit, excluded by the selected feed-content mode, or not match the active search. Feed membership and ordering are defined by server-side query logic, not by the client.

## Desired behavior

- Server remains authoritative for membership, filtering, ordering, and limits.
- The client can learn the current ordered entity IDs for the active feed view without fetching all card content.
- The client incrementally fetches partials for newly included entities, removes entities no longer included, and reorders existing DOM cards to match server ordering.
- A local upload and an availability event from another device use the same reconciliation path.
- SSE task-status events continue to refresh only a matching rendered card when the status could reflect changed content.
- If incremental reconciliation fails or is unavailable, a full `/cards` refresh remains a correct fallback.

## Possible design to evaluate

1. Add an authenticated endpoint that returns the active view's ordered card identities (`kind`/entity type plus ID), using the same query parameters and `render_content` membership logic as `/cards` but omitting expensive card content.
2. On upload success or an entity-availability hint, request that lightweight identity list.
3. Diff the response against rendered card roots:
   - fetch a partial only for newly included cards;
   - remove cards no longer in the view;
   - reorder existing and newly fetched roots to the server-provided order.
4. Preserve search query, feed-content mode, timeline limit, and any other view state in the identity request.
5. Keep full-feed refresh as a simple correctness fallback until the incremental path has proven reliable.

This is a candidate, not a committed API shape. In particular, decide how sparks and other entity kinds participate, how concurrent reconciliation requests are coalesced, and whether a server-rendered lightweight list/fragment is simpler than JSON IDs.

## Out of scope for the current SSE work

Do not prepend an uploaded capture solely because the upload response contains its ID. Do not infer timeline membership from an availability event. Keep the existing full-feed refresh until this plan is taken up; correctness is more important than avoiding the current transfer while validating the use case.

## Validation plan

- Unit-test view identity generation against the same filters, modes, ordering, and limits used by `/cards`.
- Add authenticated route tests using the existing DB-backed route harness plan.
- Browser-test reconciliation: insertion, removal, ordering, duplicate event/upload races, filtering, and fallback behavior.
- Compare payload and request behavior against the existing full-refresh path during local testing.
