# Browser-Side JavaScript Testing — Follow-up Plan

**Status:** Planned follow-up; no browser test framework is currently configured.
**Scope:** Add focused coverage for the small amount of browser behavior that now controls SSE connection lifetime and event-driven partial refreshes.

## Why consider this

`web/v2/static/webui-v2.js` is plain browser JavaScript with no build step. The SSE client now has meaningful lifecycle behavior: it opens one `EventSource`, keeps it open while backgrounded when the host allows, closes it after three minutes without recognized activity, and applies a bounded retry budget with exponential backoff after transport errors. These rules are easy to regress and affect Cloud Run request/concurrency usage as well as UI freshness.

## Recommended starting point

Prefer a real-browser test for behavior involving native `EventSource`, user input, background-tab behavior, and HTMX integration. A small Playwright test can load the local WebUI and verify the observable contract:

- one `/events` connection immediately on page load;
- the connection closes after three minutes without user interaction, regardless of visibility;
- while hidden, the client keeps the stream and activity timer behavior unchanged, subject to host suspension/cancellation;
- the server stream lifetime exceeds the three-minute client activity window but is shorter than the configured Cloud Run request timeout;
- a `stream-ending` event immediately replaces the stream without consuming the failure retry budget;
- transport failures retry with capped exponential backoff, at most five times per retry budget and only while the last interaction is recent;
- a retry timer firing after the activity window expires does not reconnect;
- exhausting retries waits for new activity; ordinary activity does not reset an active retry budget;
- a connection open for 10 seconds resets the retry budget; a short-lived open does not;
- pointer down/over, keyboard, touch, and wheel input refresh the idle deadline; `pointermove` does not;
- user input does not cancel a pending retry or reset an active retry budget;
- a page in the background keeps the same connection and inactivity timer, subject to browser/OS suspension;
- feed swaps do not open extra connections;
- each new connection computes catch-up IDs from currently rendered capture cards without illumination;
- task-status events refresh only a matching rendered capture;
- a settled catch-up event older than the card's DB-clock snapshot watermark
	does not request a partial, while a newer event triggers exactly one refresh;
- after the refreshed card installs its new watermark, replaying the same event
	does not trigger another request;

Keep tests focused on observable browser behavior rather than mirroring each implementation detail. Use controllable/fake timers or a short test-only duration seam instead of waiting several minutes.

## Lightweight alternative

If installing/running a browser toolchain is too heavy for the current validation phase, extract the retry/idle policy into a small dependency-free module and test that logic under Node's built-in test runner with mocked `EventSource`, timers, user input, and document events. This is less representative of browser integration and should not turn into a frontend build system by default.

Do not add both approaches initially. Pick the smallest approach that gives useful confidence when the client behavior is next changed.

## Non-goals

- No frontend framework or bundler solely for the current SSE client.
- No broad end-to-end test suite for every WebUI route.
- No dependency on an external test service; route/auth/database coverage is tracked separately in `plan/testing-auth-routes.md`.

## Revisit trigger

Start this work when SSE lifecycle behavior changes again, reconnect/idle behavior causes a user-visible regression, or browser-side code grows enough that manual browser checks are no longer comfortable.
