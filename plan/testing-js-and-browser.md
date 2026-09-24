# Browser-Side JavaScript Testing — Follow-up Plan

**Status:** Planned follow-up; no browser test framework is currently configured.
**Scope:** Add focused coverage for the small amount of browser behavior that now controls SSE connection lifetime and event-driven partial refreshes.

## Why consider this

`web/v2/static/webui-v2.js` is plain browser JavaScript with no build step. The SSE client now has meaningful lifecycle behavior: it opens one `EventSource`, closes it while the page is hidden or idle, reconnects on user activity, and applies exponential backoff after transport errors. These rules are easy to regress and affect Cloud Run request/concurrency usage as well as UI freshness.

## Recommended starting point

Prefer a real-browser test for behavior involving native `EventSource`, page visibility, DOM events, and HTMX integration. A small Playwright test can load the local WebUI and verify the observable contract:

- one `/events` connection while the page is active;
- the connection closes after five minutes without user interaction and while the page is hidden;
- interaction or returning to a visible tab opens a new connection;
- feed swaps do not open extra connections;
- task-status events refresh only a matching rendered capture;
- transport failures use capped exponential backoff, while an ordinary server lifetime close reconnects normally.

Keep tests focused on observable browser behavior rather than mirroring each implementation detail. Use controllable/fake timers or a short test-only duration seam instead of waiting several minutes.

## Lightweight alternative

If installing/running a browser toolchain is too heavy for the current validation phase, extract the retry/idle policy into a small dependency-free module and test that logic under Node's built-in test runner with mocked `EventSource`, timers, and document visibility. This is less representative of browser integration and should not turn into a frontend build system by default.

Do not add both approaches initially. Pick the smallest approach that gives useful confidence when the client behavior is next changed.

## Non-goals

- No frontend framework or bundler solely for the current SSE client.
- No broad end-to-end test suite for every WebUI route.
- No dependency on an external test service; route/auth/database coverage is tracked separately in `plan/testing-auth-routes.md`.

## Revisit trigger

Start this work when SSE lifecycle behavior changes again, reconnect/idle behavior causes a user-visible regression, or browser-side code grows enough that manual browser checks are no longer comfortable.
