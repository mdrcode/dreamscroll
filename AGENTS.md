# Vision
This app empowers users to build a personal, AI-enriched knowledge graph from screenshots and share links.

# Velocity & Working Style
- **Velocity First:** We are rapidly exploring and validating our use case, not
  obsessing about production hardening. Trade robustness for speed, thoughtfully.
- **Log Technical Debt:** Document all shortcuts and tradeoffs in `plan/pragmatism.md`.
- **Confirm Big Changes:** Do not go down deep refactoring or architectural
  rabbit holes without first seeking confirmation.
- **Confirm New Dependencies:** If a new third-party crate is helpful, please
  explain your research/conclusion and seek confirmation first before adding it.
- **No Schema Or Migration Overhead:** Do not worry about data migrations or backward compatibility. Nuke the database or use crude hacks if needed for prototype speed.
- **Document Along The Way:** Update the various plan files as you go. If you
  are informed by useful third-party docs/websites, links to them in
  the plan.
- **Validate Intelligently:** Do not run unit tests for small naming or plumbing
  changes (prefer compilation and targeted checks). We will always do a full
  test pass before merging to main.

# Architecture & Module Conventions
- **Module Layout:** Follow the existing clean folder structure under `src/`.
- **File Organization:** Keep module root files (`mod.rs` or `lib.rs`) lean—use them only for module exports and declarations (`pub mod ...`). Put actual feature implementations in dedicated files (e.g., `src/foo/bar.rs`).
- **Imports and Use Statements:** Prefer high level `use crate::foo` (and then
  reference foo::bar in the code) instead of long, unwieldy import statements.
  Within a module, it's perfectly acceptable to just `use super::*` to import
  sibling types. Don't burn time on frivolous import specificity.
- **Comments:** Keep comments concise and high-value. Avoid superfluous prose;
  architecture and naming should explain themselves. Important decisions and
  logic should be documented so that they are salient and discoverable.

# Target Environments
Acknowledged hard dependencies:
- Postgres
- Google Cloud Run and Cloud Tasks
- HTMX

Only support these three topologies (ignore all others):
1. Local macOS development via `cargo run`
2. Local development inside Docker
3. Production on Google Cloud Run via Docker