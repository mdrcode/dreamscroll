# Vision
This app empowers users to build a personal, AI-enriched knowledge graph from screenshots and share links.

# Velocity & Working Style
- **Velocity First:** We are rapidly exploring and validating our use case, not obsessing about production hardening. Trade robustness for speed.
- **Log Technical Debt:** Document all shortcuts and tradeoffs in `plan/pragmatism.md`.
- **Confirm Big Changes:** Do not go down deep refactoring or architectural rabbit holes without seeking confirmation first.
- **No Schema Overhead:** Do not worry about data migrations or backward compatibility. Nuke the database or use crude hacks if needed for prototype speed.

# Architecture & Module Conventions
- **Module Layout:** Follow the existing clean folder structure under `src/`.
- **File Organization:** Keep module root files (`mod.rs` or `lib.rs`) lean—use them only for module exports and declarations (`pub mod ...`). Put actual feature implementations in dedicated files (e.g., `src/foo/bar.rs`).
- **Comments:** Keep comments concise and high-value. Avoid superfluous prose; architecture and naming should explain themselves.

# Target Environments
Only support these three topologies (ignore all others):
1. Local macOS development via `cargo run`
2. Local development inside Docker
3. Production on Google Cloud Run via Docker