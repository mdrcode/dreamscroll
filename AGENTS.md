# Vision
Our app Dreamscroll empowers users to build a personal, AI-enriched knowledge graph from
screenshots and share links.

# Philosophy
Be simple, idiomatic, robust, and flexible (in that order).

# Work Style
- **Velocity First:** We are rapidly exploring and validating our use case, not
  obsessing about production hardening. Trade robustness for speed,
  thoughtfully.
- **Log Technical Debt:** Document all shortcuts and tradeoffs in `plan/pragmatism.md`.
- **Strive for Coherent Commits:** Git is our journal and source of truth.
  Group related changes into one coherent unit of work. A whole discovery
  journey, feature slice, or migration is one commit—not one per file, phase,
  or edit.
- **Confirm Big Changes:** Do not go down deep refactoring or architectural
  rabbit holes without first seeking confirmation.
- **Confirm New Dependencies:** If a new third-party crate might be helpful,
  please explain your research/conclusion and seek confirmation before
  adding.
- **No Schema Or Migration Overhead:** Do not worry about data migrations or
  backward compatibility. We can nuke the database or use crude hacks if needed for
  development velocity.
- **Confirm Local DB Writes:** Do not change the local dev database (`sync_schema`, seeding, resets, manual SQL) without explicit approval.
- **Document Along The Way:** Update the various plan files as you go. Start at
  `plan/README.md` for what is live, deferred, and archived. If
  you are informed by useful third-party docs/websites, link to them.
- **Archive Means Inactive, Not Finished:** `plan/archive/` holds docs no longer
  receiving active attention — shipped, abandoned, or superseded — kept out of the
  default read path to save context tokens. Do not read or cite one unless the task names
  it or you need the rationale behind existing behavior, and never treat its
  contents as pending work. Shipping something does not by itself archive its doc:
  one with live open questions stays at top level. Restore a doc from `archive/`
  when its questions become active again.
- **Restart Local Dev Aggressively:** A long-running local process — `./target/debug/dreamscroll_web`,
  a worker, a container — is a stale binary the moment you rebuild. Rebuild, restart, *then* observe.
  Never trust a run of a process you did not just start, and never diagnose "new" behavior from an
  old process: binary skew looks exactly like a bug in the code you just wrote. This has already cost
  a debugging session (a task-level `proximity scan` that had been deleted still ran, because the
  process predated the rebuild).
- **Validate Intelligently:** Do not run unit tests for small naming or plumbing
  changes. We will always do a full test pass before merging to main. Do not run
  smoke tests without asking for approval first.
- **Build Narrowly, Run Debug:** Never reach for `--all-targets` (or a release
  build) out of habit — it recompiles every binary and test target for a check
  that almost never needed them. Build what you touched: `cargo check --lib`,
  `cargo test --lib <module>`, `cargo build --bin <one>`. `--all-targets` only
  when the change plausibly breaks a target you are not touching (a signature in
  a shared module, a schema change, a rename across modules), and say why. To
  exercise something, run the debug binary — `./target/debug/dreamscroll_knowledge`
  — never `--release` unless the task is about the release artifact.
- **Stand On the Shoulders of Giants:** Research best practices and existing
  solutions before reinventing the wheel. When you find excellent blogs, guides,
  or technical documentation, share that knowledge.

# Module & File Conventions
- **Module Layout:** Follow the existing clean folder structure under `src/`.
- **File Organization:** Keep module root files (`mod.rs` or `lib.rs`) lean—use them only for module exports and declarations (`pub mod ...`). Put actual feature implementations in dedicated files (e.g., `src/foo/bar.rs`).
- **Imports and Use Statements:** Prefer high level `use crate::foo` (and then
  reference foo::bar in the code) instead of long, unwieldy import statements.
  Within a module, it's perfectly acceptable to just `use super::*` to import
  sibling types. Don't burn time on frivolous import specificity. Always
  organize use directives in the order: (1) system/core first, (2) external crates, (3) local modules.
- **Comments:** Keep comments concise and high-value. Avoid superfluous prose;
  architecture and naming should explain themselves. Frequently, a single line
  comment is more useful and durable than multiple paragraphs. Important decisions and
  logic should be documented so that they are salient and discoverable.

# Derived Data
Anything computed from other data — a projected column, a cached match key, an index row — carries
three obligations. Skipping one turns a caching decision into a silent correctness bug:

- **Rebuildable, from the source, in one path.** There must be a way to recompute every row without
  touching what it was derived from; a column you can only fix with a migration is a copy, not a
  derivative. `knowledge_index` is rebuilt by truncating it and re-deriving.
- **Fingerprinted, not versioned by hand.** Record *what rule produced the value*, not when it was
  written. Fold a fixed corpus of probe strings, hash the result, store the hash: any change to the
  rule moves the fingerprint on its own, so nothing depends on someone remembering to bump a
  constant. A timestamp answers "when", never "what". Pin the fingerprint in a test, so a change to
  the rule appears in the diff instead of being discovered months later as a recall bug.
- **Checkable.** One query answers "how many rows disagree with the current rule", and a command or
  a `stats` line reports it. Staleness that cannot be counted surfaces as a mystery, not a bug.

The probe corpus is the one thing a fingerprint cannot protect you from: it has to cover every
branch the rule has, including the distinctions the rule must *preserve* (`C` / `C#` / `C++` are
three things). Add a probe when you add a branch.

# Target Environments
Acknowledged hard dependencies:
- Postgres
- SeaORM
- Google Cloud Run and Cloud Tasks
- HTMX

Three supported topologies (ignore all others):
1. Local macOS development via `cargo run`
2. Local development inside Docker
3. Production on Google Cloud Run via Docker