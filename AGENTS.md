# Vision
Our app Dreamscroll empowers users to build a personal, AI-enriched knowledge graph from
screenshots. We unlock the flywheel of curiosity by helping users capture,
understand, retrieve, and discover.

# Philosophy
Be simple, idiomatic, robust, and flexible (in that order).

# Work Style
- **Velocity First:** We are rapidly exploring and validating our use case, not
  obsessing about production hardening. Trade robustness for speed,
  thoughtfully.
- **Log Technical Debt:** Document all shortcuts and tradeoffs in `plan/pragmatism.md`.
- **Code for Readability:** Prioritize intuitability and legibility of
  the code. An unfamiliar (but technical) reader should find it illuminating and
  delightful to read the code. No headaches, only epiphanies. 
- **Strive for Coherent Commits:** Git is our journal and source of truth.
  Group related changes into one coherent unit of work. A whole discovery
  journey, feature slice, or migration is one commit—not one per file, phase,
  or edit.
- **Confirm Big Changes:** Do not go down deep refactoring or architectural
  rabbit holes without first seeking confirmation.
- **Confirm New Dependencies:** If a new third-party crate might be helpful,
  please explain your research/conclusion and seek confirmation before
  adding.
- **Confirm Local DB Writes:** Do not change the local dev database (`sync_schema`,
  seeding, resets, manual SQL) without explicit approval.
- **Document Along The Way:** Update the various plan files as you go. Start at
  `plan/README.md` for what is live, deferred, and archived. If
  you are informed by useful third-party docs/websites, link to them.
- **Archive Means Inactive, Not Finished:** `plan/archive/` holds docs no longer
  receiving active attention — shipped, abandoned, or superseded — kept out of the
  default read path to save context tokens. Do not read or cite from
  `plan/archive` unless specifically instructed.
- **Restart Local Dev Aggressively:** You are responsible for managing the local
  dev instance. When you rebuild, restart localdev to avoid binary drift/skew.
- **Stand On the Shoulders of Giants:** Research best practices and existing
  solutions before reinventing the wheel. When you find excellent blogs, guides,
  or technical documentation, share that knowledge.

# Module & File Conventions
- **Module Layout:** Follow the existing clean folder structure under `src/`.
- **File Organization:** Keep module root files (`mod.rs` or `lib.rs`) lean—use them only for module exports and declarations (`pub mod ...`).
- **Imports and Use Statements:** Prefer high level `use crate::foo` (and then
  reference foo::bar in the code) instead of long, unwieldy import statements.
  Within a module, it's perfectly acceptable to just `use super::*` to import
  sibling types. Don't burn time on frivolous import specificity. Always
  organize use directives in this order: (1) system/core first, (2) external crates, (3) local modules.
- **Comments:** Keep comments concise and high-value. Avoid superfluous prose;
  architecture and naming should explain themselves. Frequently, a single line
  comment is more useful and durable than multiple paragraphs.

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

# Build Loop
- **Iterate narrow:** `cargo test --lib <filter>` or `cargo check --lib`. Never `--all-targets` —
  every test lives in the lib, and the four `src/bin` harnesses run none.
- **Clippy once per commit**, immediately before it, not between edits: `clippy-driver` has a
  different fingerprint from `rustc`, so each switch costs a full non-incremental rebuild.
- **One line per suite run:** `484 ✓ · 16 s run · 8 s rebuild`, so timing regressions stay visible.
- **`[profile.dev] debug = 1`** is deliberate — fast links, still readable backtraces. `target/debug`
  still grows; `cargo clean` when it gets gross (and expect a full dependency rebuild after).
- **Format only what you touched.** The tree is not rustfmt-clean; a repo-wide `cargo fmt` rewrites
  unrelated files and buries the real diff.
