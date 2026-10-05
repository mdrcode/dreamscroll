# Test Coverage Strategy

**Status:** Initial plan.
**Last updated:** 2026-09-26.

## Current baseline

The repository uses two test tiers:

- **Unit tests:** pure logic, serialization, parsing, state machines, and in-memory behavior. These run without external services.
- **Database tests:** `#[tokio::test]` tests using the isolated-schema Postgres harness in `src/test_support/test_db.rs`. They are skipped when Postgres is unavailable, so a green local run does not necessarily prove that DB tests executed.

The current full suite passes with:

- 191 library tests
- 0 failures
- 0 ignored tests
- Binary targets compile, but currently contain no tests

Run the baseline with:

```text
cargo test --all-targets --no-fail-fast
```

The suite currently has strong coverage around authentication helpers, task identity and lifecycle, retry policy, local/cloud queue behavior, SSE serialization and stream helpers, configuration, search embedding validation, telemetry formatting, and template loading.

## Coverage tools

No quantitative coverage tool is currently installed or configured. The following commands were checked on 2026-09-26 and were unavailable:

- `cargo llvm-cov` / `cargo-llvm-cov`
- `cargo tarpaulin` / `cargo-tarpaulin`
- `grcov`

Therefore, the project currently reports test counts and behavior-focused coverage, but not line or branch coverage percentages.

### Options to evaluate later

- **`cargo-llvm-cov`** — likely the best default for Rust line and region coverage when the LLVM tooling is available. It can produce human-readable reports and LCOV output for CI.
- **`cargo-tarpaulin`** — convenient for many Linux-based CI environments, but native macOS support and instrumentation behavior should be verified before adopting it as the project standard.
- **`grcov`** — useful when integrating compiler coverage instrumentation into a broader CI/reporting pipeline, but more setup-heavy for local use.

Do not add a coverage dependency or CI requirement solely to obtain a percentage until we decide that the maintenance and toolchain cost is justified. If adopted, prefer a developer-installed tool or CI tool rather than an application dependency.

## Existing coverage gaps

The task/SSE testing review identified these priority gaps:

1. Authenticated partial routes: capture-card and detail rendering, missing/inaccessible captures, and ownership isolation.
2. TaskMaster notification lifecycle: queued, in-progress, retry/final outcome, and enqueue-failure events with complete identity and attempt fields.
3. DB-backed SSE catch-up: selected captures, owner scope, all latest task types/statuses, and subscribe-before-snapshot handoff.
4. Browser SSE lifecycle: three-minute inactivity close independent of visibility, no visibility/network-only reconnects, bounded retry budget with backoff, stable-open reset, and at most one EventSource/timer.
5. Listener/process lifecycle: listener failure visibility and WebUI startup/shutdown propagation.
6. Task edge cases: missing-run semantics, successful submissions for each task type, and enqueue failure followed by a later run.
7. SSE catch-up fan-out: one initial refresh hint per latest entity while live task updates remain individually delivered.

These are tracked as follow-up work rather than blockers for the current task framework.

## Task timing test plan

Timing persistence is a database behavior and should use the real isolated Postgres harness rather than a mocked database.

Required cases:

- A newly queued run has `processing_started_at`, `last_error_duration_ms`, and `success_duration_ms` set to `NULL`.
- Beginning an attempt sets `processing_started_at` and increments the attempt number.
- Successful completion stores a non-negative `success_duration_ms`.
- A failed attempt stores a non-negative `last_error_duration_ms`.
- A retry receives a new `processing_started_at` value.
- A later failure overwrites `last_error_duration_ms` with the most recent failed attempt duration.
- Success after a failure preserves `last_error_duration_ms` and sets `success_duration_ms`.
- A failure before a processing start still updates status but leaves the duration `NULL` and does not panic.
- Timing fields are independent between separate runs of the same logical task.

Avoid exact-duration assertions because scheduling and database latency make them brittle. Assert nullability, non-negativity, and timestamp ordering instead.

## Suggested adoption order

1. Keep behavior-focused unit and isolated-schema DB tests as the required baseline.
2. Complete the timing edge-case tests listed above.
3. Close the task notification and authenticated route gaps.
4. Add browser tests only when the reconnect/UI behavior is stable enough to justify a browser harness.
5. Evaluate `cargo-llvm-cov` for local and CI reporting; compare setup cost against the value of quantitative coverage.
6. If a tool is adopted, record the exact installation, command, and CI policy here.
