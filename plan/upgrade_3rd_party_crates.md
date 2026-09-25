# Third-party crate upgrade plan

**Status:** proposed. **Date:** 2026-09-18.

This is a pragmatic, repeatable process for updating Dreamscroll's Rust
dependencies safely without turning one maintenance task into an uncontrolled
rewrite.

## Core workflow

The practical loop is:

```text
# Establish a known-good baseline first
git status --short
cargo check --all-targets
cargo test --all-targets

# See what is stale
cargo outdated --root-deps-only

# Preview one crate or a small compatible batch
cargo upgrade -p <crate> --compatible --dry-run

# Apply the manifest requirement change, then resolve the lockfile
cargo upgrade -p <crate> --compatible
cargo update -p <crate>

# Review and verify
git diff -- Cargo.toml Cargo.lock
cargo tree -d
cargo fmt -- --check
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo audit
```

Repeat this loop package by package, or family by family when crates are
coupled. Do not upgrade the whole dependency graph blindly. The key distinction
is that `cargo upgrade` changes the version requirements in `Cargo.toml`, while
`cargo update` changes the concrete versions recorded in `Cargo.lock`. If the
existing requirement already permits the desired version, skip `cargo upgrade`
and use only `cargo update -p <crate>`.

The commands are a workflow, not a guarantee. Before upgrading a crate with a
public API, read its release notes and migration guide. Review every manifest
and lockfile diff, keep each batch easy to revert, and treat database, HTTP,
authentication, crypto, TLS, and generated-client crates as higher-risk than
ordinary utility crates. `cargo outdated` reports freshness; `cargo audit`
checks RustSec advisories; neither replaces tests or release-note review.

### Answering "what depends on X?"

Use `cargo tree --invert` to reverse the dependency graph. It answers the
practical question: *which direct or transitive dependencies are bringing this
crate into my project?*

```text
# Everything that depends on sqlx
cargo tree --invert sqlx

# Investigate a particular resolved version when duplicates exist
cargo tree --invert sqlx@0.8.6
cargo tree --invert sqlx@0.9.0

# Include normal, build, and development dependency edges
cargo tree --invert sqlx --edges all

# Show feature paths as well
cargo tree --invert sqlx --edges features

# Show only the first level of reverse dependencies
cargo tree --invert sqlx --depth 1

# Find packages that occur in multiple versions
cargo tree --duplicates
```

For a dependency upgrade, start with the version-specific reverse trees if
`cargo tree --duplicates` shows multiple versions. A result such as

```text
dreamscroll
└── tower-sessions-sqlx-store
  └── sqlx v0.8.6
```

means the session-store crate is the path that prevents that part of the graph
from using another SQLx version. Compare it with the reverse tree for the other
version, then decide whether to upgrade the parent crate, accept the duplicate,
or defer the upgrade. This investigation is often more useful than looking at
the top-level `Cargo.toml` alone.

## 1. What "update dependencies" means

Keep these activities separate:

1. **Lockfile refresh:** update `Cargo.lock` within the version ranges already
   declared in `Cargo.toml`. Usually low risk and does not require source edits.
2. **Manifest update:** change a direct dependency requirement in `Cargo.toml`.
   This may select a newer compatible release, but changes the minimum supported
   version and potentially the resolved feature graph.
3. **Breaking upgrade:** intentionally move across a SemVer-incompatible major
   or pre-release boundary. Expect release-note work and source changes.
4. **Security response:** fix a RustSec advisory or unacceptable transitive
   version, even if that means a targeted override or a larger upgrade.

Do not treat the newest version number as the goal. The goal is a reproducible,
tested, supportable dependency graph with no known unacceptable advisories.

## 2. Current baseline

At the time this plan was written:

- The service is a single Cargo package, edition 2024, with a checked-in
  `Cargo.lock` containing about 586 package entries.
- Rust/Cargo were `1.95.0` when this baseline was recorded; rerun
  `rustc --version` and `cargo --version` before starting an upgrade.
- `cargo-outdated` is installed; `cargo-audit` and `cargo-deny` are not.
- The direct dependency list contains several tightly coupled families:
  Google Cloud crates, OpenTelemetry crates, Axum/Tower crates, and SeaORM/
  SQLx crates.
- The current dependency inventory indicates many compatible updates plus
  several substantial jumps. In particular, do **not** combine all of these in
  one blind edit: Google Cloud generated clients, `sea-orm` (currently an RC),
  `sqlx`, `argon2`, `jsonwebtoken`, `keyring`, `tera`, and `tower-http` need
  individual review.
- The repository currently has untracked plan files. Before making code or
  lockfile changes, confirm the working tree and make a baseline commit or
  otherwise preserve the current state.

The inventory command showed (at this date) compatible updates such as
`reqwest`, `tokio`, `rustls`, `serde`, and `uuid`, and potentially breaking
updates such as `argon2`, `base64`, Google Cloud clients, `jsonwebtoken`,
`sqlx`, `strum`, `tera`, `tower-http`, and `tower-sessions`. Treat this output
as a snapshot, not as a prescription; rerun it immediately before upgrading.

## 3. Recommended strategy

### Phase A — Establish a known-good baseline

1. Ensure the working tree is clean enough to identify upgrade changes. Commit
  or stash unrelated work; do not discard it. If there are existing uncommitted
  dependency edits, decide explicitly whether they are the baseline or should
  be set aside before continuing.
2. Record the toolchain (`rustc --version`, `cargo --version`) and decide the
   project's MSRV policy. If the service has a deployment toolchain, test that
   toolchain too; local success on Rust 1.95 is not enough.
3. Run and record the baseline:

   ```text
   cargo fmt -- --check
   cargo check --all-targets
   cargo test --all-targets
   cargo clippy --all-targets --all-features -- -D warnings
   cargo tree -d
   cargo tree -e features
   ```

   Run the service's operational checks as well: database-backed tests (the
   project documents the Postgres setup in `plan/testing.md`), startup, health
   endpoint, authentication, uploads, Cloud Tasks/webhooks, search, and any
   smoke test used before a deployment.
4. Save or note the baseline failures. An upgrade PR must not accidentally
   become a debugging PR for pre-existing failures.

### Phase B — Audit before upgrading

Use more than one signal:

- `cargo outdated --root-deps-only` for direct dependencies and
  `cargo outdated` for the full graph.
- `cargo audit` for RustSec vulnerability, unsoundness, and unmaintained-crate
  advisories. Review each advisory instead of blindly suppressing it.
- Optionally install and initialize `cargo-deny` (`cargo deny init`) to enforce
  advisories, licenses, allowed sources, and duplicate-version policy in CI.
- `cargo tree -d` to find duplicate versions, and
  `cargo tree -e features -i <crate>` to understand why features are enabled.
- For a candidate release, read its changelog/release notes, MSRV, migration
  guide, feature changes, and known issues. For high-impact crates, inspect
  the crate's docs and repository issue tracker as well.

The RustSec audit database and dependency freshness are different concerns:
`cargo audit` can say "safe" while a crate is old; `cargo outdated` can report
an update that is incompatible or not suitable for this service.

### Phase C — Make small, reviewable upgrade batches

Use a branch and one logical change per commit/PR. A sensible order is:

1. **Compatible patch/minor refresh:** update the lockfile first with
   `cargo update`, or update a selected package conservatively:

   ```text
  cargo update --dry-run
  cargo update -p <crate>
   ```

   Review `Cargo.lock`; do not hand-edit it. If the manifest requirement must
   change, edit `Cargo.toml` deliberately and then use `cargo update`.
2. **Low-coupling direct crates:** update utilities one at a time or in a
   small batch, validating after each batch.
3. **Coordinated families:** upgrade Axum/Tower, Google Cloud, OpenTelemetry,
   and SeaORM/SQLx families together only when their compatibility requires it.
   Check that feature flags remain equivalent and that only one intended version
   of foundational crates is selected where practical.
4. **Breaking upgrades:** give each major migration its own commit/PR. Update
   the manifest, consult the migration guide, fix compilation errors, then fix
   behavior/regression failures. Do not use a large blanket search/replace for
   APIs or feature names.

`cargo update` changes resolved versions in `Cargo.lock`; it does not generally
rewrite the requirements in `Cargo.toml`. The default requirement syntax is a
caret range (`"1.2.3"` means compatible releases below the next incompatible
version), so avoid artificially narrow pins unless there is a documented reason.
For a production service, commit the lockfile and use `--locked` in CI/builds.

`cargo-edit`'s `cargo upgrade` can rewrite manifest requirements, but use its
compatible-only/dry-run modes first and review every manifest diff. It is an
optional convenience, not a substitute for release-note review. Recent Cargo
versions already provide `cargo add` and `cargo rm`; `cargo-edit` is mainly
useful here for `cargo upgrade`.

### Phase D — Validate each batch

After every meaningful batch, run:

```text
cargo fmt -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo audit
cargo tree -d
```

Also run the relevant feature/platform matrix. At minimum, verify the Linux
`linux/amd64` Docker build used by `gcloud/cloudbuild.yaml`; local macOS compilation
can miss Linux-only code, linker behavior, and system-library issues. Use
`cargo build --locked` (or the exact Docker build path) to ensure the checked-in
lockfile is honored. Run DB tests with Postgres available, and run API/auth,
Cloud integration, and persistence smoke tests where credentials and services
are safely available.

When a batch fails, classify the failure:

- resolver/feature conflict → inspect `cargo tree -e features` and package
  requirements;
- compile error → consult the dependency migration guide and change only the
  affected adapter/API boundary;
- test/behavior regression → compare defaults, serialization, TLS, timeouts,
  database SQL, and error handling;
- advisory → update the smallest safe path, or document a narrowly scoped,
  time-bounded exception with rationale and owner.

Do not “fix” a failing upgrade by weakening tests, removing security features,
or adding a broad `[patch]` override without understanding the graph.

### Phase E — Review and merge

For each PR, include:

- old/new direct versions and why each change is included;
- whether the change is lockfile-only, compatible, or breaking;
- notable migration-guide changes and source/API changes;
- `Cargo.lock` duplicate-version changes;
- audit result and any explicit exceptions;
- commands run, including Docker/Linux and DB-dependent checks;
- rollback plan (revert the single PR/commit).

Keep application changes separate from dependency-only churn where possible.
Merge low-risk batches first. Deploy a breaking batch with extra observability
and a quick rollback path.

## 4. Practical first pass for this repository

Do this as a sequence of separate branches or commits, not one command:

### 4.1 Baseline and security

```text
git status --short
cargo fmt -- --check
cargo check --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo outdated --root-deps-only
cargo audit
cargo tree -d
```

Use `cargo outdated` as an inventory, not as an instruction to upgrade every
line it reports. The `--root-deps-only` view is usually the best starting point;
inspect the full transitive graph when investigating advisories, duplicates, or
an unexpectedly large lockfile change.

Install missing tools with locked installs when appropriate:

```text
cargo install --locked cargo-audit
cargo install --locked cargo-deny
```

Do not install tools into the application dependency graph. They are developer/
CI tools. If adopting `cargo-deny`, review its generated `deny.toml` rather than
accepting all defaults blindly.

### 4.2 Conservative compatible refresh

First preview, then update only a small compatible group. If the manifest
requirement already permits the new version, this is a lockfile-only operation;
otherwise use `cargo upgrade` deliberately before `cargo update`:

```text
cargo upgrade -p anyhow -p async-trait -p blake3 --compatible --dry-run
cargo upgrade -p anyhow -p async-trait -p blake3 --compatible
cargo update -p anyhow -p async-trait -p blake3
cargo check --all-targets
cargo test --all-targets
```

The exact list should be regenerated from the current `cargo outdated` output.
Commit `Cargo.lock` separately if that makes review clearer. Do not assume a
transitive update is harmless: TLS, crypto, database drivers, and generated
clients deserve targeted tests.

### 4.3 Deliberate high-impact upgrades

Handle these separately and read release notes before editing:

- **Google Cloud:** upgrade the generated clients and shared auth/GAX/storage/
  tasks/vector-search crates as a compatibility family; test token creation,
  Cloud Storage, Cloud Tasks, and Vertex calls.
- **SeaORM + SQLx:** decide whether to leave the RC temporarily or move to the
  stable line; check schema-sync behavior, PostgreSQL types, runtime/TLS
  features, and the project's isolated-schema DB harness.
- **Axum/Tower/session stack:** check middleware signatures, extractors,
  response/body types, session-store behavior, and auth flows.
- **Crypto/auth:** `argon2`, `jsonwebtoken`, `keyring`, and `rustls` need
  focused review of algorithms, providers, key formats, platform behavior, and
  security defaults. Never silently change password-hash parameters or token
  validation semantics.
- **Telemetry:** upgrade OpenTelemetry crates together and verify exporter/span
  behavior and Cloud Trace output.

### 4.4 CI guardrails to add after the first cleanup

Make dependency hygiene continuous rather than repeating a ten-month jump:

- Run `cargo fmt -- --check`, `cargo check --locked --all-targets`, tests,
  Clippy, and `cargo audit` in CI.
- Run `cargo deny check` if the policy is adopted.
- Use Dependabot or Renovate for small grouped PRs. Configure separate groups
  for patch/minor updates and coordinated ecosystems; do not auto-merge major
  or security-sensitive changes without tests and review.
- Consider a scheduled weekly/monthly dependency PR and a separate immediate
  RustSec alert path.
- Keep `Cargo.lock` committed and make release builds fail when it is stale via
  `--locked`.

## 5. Ongoing cadence

A reasonable maintenance rhythm is:

- **Every PR/build:** locked resolution, formatting, check, tests, Clippy.
- **Weekly or fortnightly:** compatible updates and advisory scan.
- **Monthly:** review direct dependency freshness, duplicate versions, MSRV,
  licenses/sources, and Docker/Linux build.
- **Quarterly:** one planned breaking-upgrade window, with ecosystem families
  handled independently.
- **Immediately:** RustSec advisories affecting reachable production code,
  especially auth, crypto, TLS, HTTP, database, and serialization crates.

This cadence keeps the diff small and preserves the most valuable upgrade tool:
a recent, trustworthy test baseline.

## 6. Tool reference

| Tool                            | Use                                               | Important distinction                           |
| ------------------------------- | ------------------------------------------------- | ----------------------------------------------- |
| Cargo resolver / `cargo update` | Resolve versions and update `Cargo.lock`          | Does not replace API/migration review           |
| `cargo outdated`                | Find stale direct/transitive crates               | Freshness, not vulnerability analysis           |
| `cargo audit`                   | Check RustSec advisories                          | Advisory coverage, not general quality          |
| `cargo-deny`                    | Advisories, licenses, sources, duplicate versions | Policy enforcement; configure deliberately      |
| `cargo tree`                    | Inspect graph, duplicates, and features           | Explains why a version/feature is present       |
| `cargo-edit` / `cargo upgrade`  | Rewrite manifest requirements                     | Optional; review generated changes              |
| Dependabot/Renovate             | Open automated update PRs                         | Automation should be bounded by CI and grouping |

## Definition of done

The upgrade is complete when the desired version ranges are documented in
`Cargo.toml`, `Cargo.lock` is regenerated and committed, no unexplained duplicate
or advisory remains, all relevant checks pass on the supported toolchain and
Linux Docker target, and the PR records any deliberately deferred major upgrade
with a follow-up issue.
