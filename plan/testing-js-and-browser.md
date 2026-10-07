# Browser-Side JavaScript Testing — Follow-up Plan

**Status:** Planned follow-up; no browser test framework is currently configured.
**Scope:** Add focused coverage for browser behavior controlling SSE connection lifetime, event-driven partial refreshes, and upload feedback.

## Why consider this

`web/v2/static/webui-v2.js` is plain browser JavaScript with no build step. The SSE client combines a two-minute inactivity close with a heartbeat stale check; native EventSource owns transport retries. These rules affect Cloud Run request/concurrency usage and UI freshness.

## Recommended starting point

Prefer a real-browser test for behavior involving native `EventSource`, user input, background-tab behavior, and HTMX integration. A small Playwright test can load the local WebUI and verify the observable contract:

- one `/events` connection immediately on page load;
- the connection closes after two minutes without user interaction, regardless of visibility;
- while hidden, the client retains a live heartbeat connection and its inactivity timer, subject to host suspension/cancellation;
- returning to visible state via `visibilitychange`, or BFCache restoration via `pageshow`, checks heartbeat freshness and reconnects immediately only when stale;
- no heartbeat for 50 seconds (two expected 20-second beats plus 10 seconds of grace) triggers stale recovery;
- a delayed heartbeat resets the stale-connection deadline without opening a second EventSource;
- normal server stream closure and transport interruption reconnect through the browser's native EventSource policy, without constructing another source;
- `error` marks the connection indicator disconnected and the next `open` clears it;
- pointer down/over, keyboard, touch, and wheel input refresh the idle deadline; `pointermove` does not;
- user activity creates one source after an idle close; activity while connected does not replace it;
- changing visibility alone does not recycle a healthy stream; a stale stream reconnects on foreground;
- feed swaps do not open extra connections;
- each new source computes catch-up IDs from currently rendered capture cards without illumination;
- task-status events refresh only a matching rendered capture;
- a settled catch-up event older than the card's DB-clock snapshot watermark
	does not request a partial, while a newer event triggers exactly one refresh;
- after the refreshed card installs its new watermark, replaying the same event
	does not trigger another request;

- non-2xx and network upload failures leave an accessible failure notice visible after progress UI resets;

Keep tests focused on observable browser behavior rather than mirroring each implementation detail. Use controllable/fake timers or a short test-only duration seam instead of waiting several minutes.

## Lightweight alternative

If installing/running a browser toolchain is too heavy, extract the idle/heartbeat policy into a small dependency-free module and test it with mocked `EventSource`, timers, visibility, and user input under Node's built-in test runner. Native EventSource retries still require a browser test. Avoid turning this into a frontend build system by default.

Do not add both approaches initially. Pick the smallest approach that gives useful confidence when the client behavior is next changed.

## Non-goals

- No frontend framework or bundler solely for the current SSE client.
- No broad end-to-end test suite for every WebUI route.
- No dependency on an external test service; route/auth/database coverage is tracked separately in `plan/testing-auth-routes.md`.

## Revisit trigger

Start this work when SSE lifecycle behavior changes again, reconnect/idle behavior causes a user-visible regression, or browser-side code grows enough that manual browser checks are no longer comfortable.
