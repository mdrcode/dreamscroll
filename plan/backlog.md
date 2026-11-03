# Backlog

Small, startable tasks. Anything needing design work lives in its own plan; see
[Deferred](#deferred--already-scoped) below. Everything shipped is removed from
this file (git history keeps it) and recorded in `plan/archive/`.

## Next

- [ ] **Make `link_illumination` transactional.** Linking one reading writes several times per
  decision: create or load the entity, attach the observation, record the claim (or fold and
  discard the row it just made), then project. A crash between any two leaves an entity with
  members and no claim, or a claim on an entity the projection never recomputed — and nothing
  repairs either. `merge` already shows the shape: `merge_in_transaction<C>` takes the connection
  and its five writes share one transaction. Two things block doing the same here. The executor
  has to take the connection as a parameter rather than reach for `&self.db.conn`, and `Evidence`
  has to borrow that connection instead of owning one: a claim written outside the transaction
  would reference a `knowledge_entity_id` the transaction has not committed, which is a
  foreign-key violation rather than a torn read. `Evidence` holds a `DatabaseConnection` today and
  seven call sites build one, so this and "Give evidence claims a repository" below are the same
  change approached from opposite ends — do them together.
- [ ] **Assert the projection invariant.** `knowledge_stats` prints `platform_from_members` beside
  `with_platform`, and the field is documented as always equal: a divergence means `project` lost a
  presence that some member still carries. Nothing asserts it — a lost presence currently shows up
  only as two numbers a human has to compare. The cheap version is an assertion in the stats test;
  the useful one is a test that projects an entity whose *second* member carries a platform and
  asserts the column survives, since that is the order in which the blocker lag used to lose it.
- [ ] **Make `origin_observation_id` required.** It is `Option<i32>` and the column is nullable,
  but `create` is the only writer and it always sets it — every entity in the rebuilt corpus has
  one, 2,119 of 2,119. The nullability is vestigial: it was added to a table that already had
  rows, and the no-migration policy means a rebuild is the backfill. Making it `NOT NULL` would
  turn the premise of the whole projection rule ("the entity wears the values of the member it
  came from") into something the schema states rather than something `create` happens to honour,
  and it would delete `project`'s fallback to the first member — a branch that exists only for the
  null case and that reads as though origins were sometimes unknown. Two things move with it: the
  model field becomes `i32`, and `fixtures::stub_entity` passes `None` for entities that never had
  an observation behind them, so those stubs need either a real observation id or a shape that
  admits an entity with no origin at all. Worth deciding at the same time whether `project` should
  error on an entity with no members or keep its silent fallback — with the column required, "no
  members" is the only way to reach the fallback.
- [ ] **Unmerge.** A merge tombstones the loser and records `kind = 'merge'` with `child` and
  `method`, so reversing one is a pointer away — `merge`'s own doc says the decision is "one pointer
  away from being undone". Nothing implements it. `plan/knowledge-canonical-entities.md` §Repair
  describes `retire` and `relink`, and the CLI that drives a repair by hand is still "later". Settle
  one design question first: observations are the source of truth for membership, so re-pointing them
  by the ids the merge moved is probably enough, but the evidence rows that moved need their old owner
  back, and the `merge` row is the only record of which entities were involved.
- [ ] **Make the tree `rustfmt`-clean, and keep it that way.** `cargo fmt --check` reports ~87
  differences and there is no `rustfmt.toml`. Worse for this codebase: formatting a module root
  (`src/knowledge/mod.rs`) recurses into every child module, so a per-file `rustfmt` run silently
  reformats hundreds of unrelated lines — twice in one session that had to be reverted by hand.
  Either adopt the formatter repo-wide in a single commit and gate it in CI, or pin a `rustfmt.toml`
  whose style matches what is there closely enough that the first run is small. Formatting only the
  files you touched is the current workaround: it keeps diffs honest and leaves the drift in place.
- [ ] **Implement `SqlBlocker`.** `NameBlocker` is an in-memory index built by a full scan of
  `knowledge_entities` (~2.1k rows), so every `Knowledge::link_illumination` pays for the whole graph
  to link one reading. Fine for a capture arriving at a time; wrong for a backfill, which is why
  `link` still exists as "one blocker, every illumination". A `Blocker` that answers `candidates`
  with a query removes the scan and cannot go stale (the trait's `changed` is a no-op for anything
  reading the database). The obstacle is that the matching keys are not in the database:
  `normalize::comparable` and the token keys in `heuristic.rs` are Rust functions, and they are described as *the*
  definition of "the same name", so re-deriving them in SQL means two definitions that can drift.
  Cheapest shape: generated columns (or expression indexes) over `name`, `alt_names`,
  `platform_handle` and `platform_display_name`, fed by one shared normalisation — plus a check that
  `unnest`-ing `alt_names` per row is still cheaper than the scan it replaces. Pairs with the
  transaction work on `link_illumination`: both are about the link path no longer being a whole-graph
  operation.
- [ ] **Decide whether a claim match can be refused.** A platform link proves the link is the same;
  it says nothing about the thing behind it. Two captures claiming `X/@elonmusk` as
  "technologist and inventor" and as "random bullshitter" are today merged silently — the second is
  absorbed as corroboration, `project` picks one description, and nothing records the disagreement.
  The existing `collision` evidence kind and the `contested` status are the machinery; what is
  missing is a corroboration bar inside the linker for claim matches, and a decision about what the
  executor does with a refused match (link and mark contested, or leave unlinked and re-evaluate
  next run). Needs a real case before designing thresholds — a report-only count of claim matches
  that would fail a corroboration test is the first step. Shape is settled: a refused match needs no
  new decision variant — it is `Create` with the claim withheld and `best` naming the entity
  declined, which `project` then leaves `provisional` for free. Deferred with it: whether a refusal
  also writes a `collision` row (which would mark the established entity `contested`); today
  `contested` means "two things we could not merge", and diluting it is a separate decision.
- [ ] **Give evidence claims a repository.** `link` and `resolve` each hand-roll the same
  three outcomes around `idx-knowledge_evidence_identity`: insert, unique violation, and
  "another entity already holds this `(scheme, id)`". Both duplicate the violation check
  (`is_unique_violation` / `unique_violation`) and both call `identity_owner` directly.
  A `knowledge::evidence::Evidence<'a>` over a connection — `owner(scheme, id)`,
  `claim(entity, &Claim) -> Accepted | HeldBy(i32)` — would own the storage while callers keep
  their policy (`link` deletes the duplicate it just made; `resolve` merges or records a
  collision). Open question: whether `resolve::Claim` moves into `evidence` too, since the
  platform claim `link` writes is the same row shape with different fields filled.
- [ ] **Score the entity-resolution precision sheet.** 100 labelled rows sit in
  `localdev/analysis/entity-promotion/evidence/review/precision_sheet.csv`
  (45 identified / 25 ranked / 15 ambiguous / 15 unidentified). Precision ≥ 0.90
  lets the `identified` dimension drive promotion as-is; below 0.75 the
  verification step needs rework. See `plan/knowledge-evidence-prototype.md` §8.
- [ ] **Replace the keyword type-filter with `P31` class matching.** The filter
  rejected 444 rows whose exact-label item existed, 247 of them with a single
  candidate. A curated `type → {class QIDs}` map tested through
  `wdt:P31/wdt:P279*` is the fix. Same doc, §9.
- [ ] **Define `brand` vs `organization`, `product` vs `software`.** These two
  undefined type pairs account for 111 of 179 collision pairs.
- [ ] **Add `concept` and `organism` entity types, plus `referent_form` and
  `wikidata_class`.** Settled by the 2026-10-29 audit in `plan/knowledge-entity-types.md`:
  `unknown` splits into six Wikidata classes, 3 of them taxa, and named plant cultivars are
  currently typed `product`. Touches the v2 prompt, the type enum, and the P31 → type mapping.
  Unblocks 15 of the 17 `unknown` rows and the dahlia/cultivar corpus.
- [ ] Change the formatting pattern of the "commands" for both dreamscroll_api
  to use consistent - or _. Currently we use a mix.
- [ ] Expose the archived feed navigationally and paginate it. `/archived` works
  but is reachable only by typing the URL and caps at 100 captures, most recently
  archived first. See the Web UI table in `plan/pragmatism.md`.
- [ ] **Settle the agent smoke-environment story.** Today an agent can use the
  persistent local instance (see `plan/smoke.md`) or hack a test database, and test
  code carries a lot of ad hoc DB surgery. A deterministic in-memory search
  backend for throwaway runs was added and reverted the same day (`9cf0c70`,
  `fa9762f`), leaving only the manual procedure. Decide between: a supported
  throwaway profile, a seeded fixture harness, or making the
  persistent-instance procedure the only path.
- [ ] **Validate the Interactions API path from the Cloud Run service account.**
  Both Google backends pass a full-prompt/schema live smoke locally; the Vertex
  path under the deployed service account is unverified. See
  `plan/archive/google-ai-interactions-api.md`.
- [ ] **Add a schema audit to `dreamscroll_db`.** Compare the model registry
  against the live catalog (missing columns *and* retired columns), so a reset
  cannot leave the app unable to insert. The 2026-10-27 reset left
  `task_runs.user_id NOT NULL` behind and every illuminate enqueue failed on it.
- [ ] **Fail uploads when the illumination enqueue fails.** `Beacon::new_capture`
  errors are logged at `warn!` and swallowed, so the upload returns success with
  no illumination queued. Log at `error!` at minimum; consider failing the
  request.
- [ ] **Dispose of the renamed legacy tables** left in `public` by the
  2026-10-27 schema reset (old `illuminations` and friends). They are inert for
  SeaORM but clutter the schema; either drop them once the reset is trusted or
  move them to a `legacy` schema.
- [ ] **Remove or clearly demote `gcloud/cloudbuild.yaml`.** Nothing in the
  release path uses it any more (`prod-build-push.sh` + `prod-apply.sh`), it
  duplicates the identifiers that now live in `prod-config.sh`, and its deploy
  step bypasses the drift and invariant checks. Delete it — the archived
  research records what it was — or mark it unsupported and align its
  substitutions. Fix the two docs that still call it the build path:
  `plan/static-serving-cdn.md`, `plan/upgrade_3rd_party_crates.md`.

- [ ] **Separate interactive captures from bulk/backfill work in the task
  queues.** The illuminate queue runs `maxConcurrentDispatches: 1`, so a user
  upload waits behind every queued backfill task; during the 2026-10-27 backfill
  interactive captures sat behind hundreds of bulk runs. Decide between a
  dedicated interactive queue (higher concurrency, throttled bulk queue) or
  moving bulk work to Batch. See `plan/vertex-throughput.md` §5.

- [ ] **Re-evaluate sending Vertex a `gs://` file reference instead of inline
  base64.** Every image is base64-encoded into the request today
  (`InferenceInput::Image` in `src/llms/gemini/client.rs`), and the embedding path
  builds `inlineData` (`src/search/capture_data_object.rs`). Base64 adds ~33%, so a
  7 MB PNG ships as ~9.3 MB of request — and `MAX_UPLOAD_BYTES` is now 12 MiB,
  putting a single upload within reach of Vertex's ~20 MB inline request ceiling.

  We had this before: `GeminiPayloadMethod::{FileUri, Inline}` (default `Inline`)
  built `FileData { file_uri: gs://… }` through `StorageProvider::make_prod_uri`,
  and `eefdd8b` ("Unify Gemini inference through Interactions API") removed it.
  `make_prod_uri` still exists on every storage provider, so the URI half is intact.

  The real unknown is the API, not our code: the old path predates the unification,
  so confirm the **Interactions API** on Vertex accepts a GCS file reference at all.
  Also settle whether the Cloud Run service account already has `objectViewer` on
  `dreamscroll-prod-media1` (it writes objects there, so likely), whether the lever
  applies to embeddings as well as illumination/spark, and the latency delta — a
  provider-side GCS read replaces local encoding rather than being free.

  Moot if the Interactions API has no file-reference input; then the lever is
  shrinking uploads instead. See MEDIA_TRANSCODE in `plan/upload-formats.md`.

## Deferred — already scoped

Promote an item into **Next** when its trigger fires or it becomes the focus.

| Topic                                              | Plan                                                                   | Gate                                                          |
| -------------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------- |
| Incremental feed updates after upload              | [feed-view-incremental-updates.md](feed-view-incremental-updates.md)   | Feed refetch becomes user-visible cost                        |
| Protected-route tests through real auth middleware | [testing-auth-routes.md](testing-auth-routes.md)                       | Next change to auth or route-level authorization              |
| Browser tests for SSE/refresh/upload               | [testing-js-and-browser.md](testing-js-and-browser.md)                 | First client bug that unit tests miss                         |
| Coverage strategy beyond the two tiers             | [testing-coverage.md](testing-coverage.md)                             | Coverage discussions block a change                           |
| Private media URLs                                 | [media-serving.md](media-serving.md)                                   | Multi-user access or CDN/LB work starts                       |
| Provisioning the schema-sync Cloud Run Job         | [prod-deploy.md](prod-deploy.md)                                       | First production schema change                                |
| API token hardening                                | [api_token_hardening.md](api_token_hardening.md)                       | API usage grows beyond first-party clients                    |
| Entity taxonomy pass                               | [knowledge-entity-types.md](knowledge-entity-types.md)                 | Environment fully backfilled with v2                          |
| htmx 4.0 migration                                 | [htmx4.md](htmx4.md)                                                   | 4.x becomes `latest`, or 2.x breaks a behavior we rely on     |
| Wikidata entity linking                            | [knowledge-wikidata-recommender.md](knowledge-wikidata-recommender.md) | Entity identity across captures becomes a product requirement |
| Entity canonicalization and similarity             | [knowledge-canonical-entities.md](knowledge-canonical-entities.md)     | None — phase 1 is startable; public phases P-a…P-d follow it  |
| Screenshot provider selection                      | [screenshot_3rd_party.md](screenshot_3rd_party.md)                     | Link-preview latency or fidelity is measured                  |
| Static/CDN serving decision                        | [static-serving-cdn.md](static-serving-cdn.md)                         | Static bundle cost or latency justifies a network layer       |
| `/events` catch-up query reduction                 | [event-streaming-optimization.md](event-streaming-optimization.md)     | Connection count or DB load becomes material                  |
| Crate upgrade sweep                                | [upgrade_3rd_party_crates.md](upgrade_3rd_party_crates.md)             | Scheduled maintenance window                                  |
| Client pulse + `upload_attempts`                   | [upload-attempts.md](upload-attempts.md)                               | A client-side upload failure goes unexplained for a second time |
| Media transcode and format policy                  | [upload-formats.md](upload-formats.md)                                 | `rejected_format` fires for a real user, uploads stay multi-MB on cellular, or a non-browser ingestion path carries originals |

## Product direction — not yet scoped

Carried forward from `plan/archive/devlog.md`; each needs a design pass before it becomes a task.

- **Bifurcate the app loop** into `LearnMoreAbout`, `MoreLikeThis`, and `GoBeyond`.
- **Mind Map** view over the knowledge graph.
- **Derived media resolutions.** `thumbnail_url` points at the original asset, not a
  separately generated thumbnail; large screenshots are served at full size.
- **USER_FACING_LOGGING.** Decide how an actionable failure reaches the user.
  `ApiError` currently returns an opaque body (`Request failed. status=… report_id=…`),
  so every rejection reads as "something broke". Needs a design pass covering: what
  a safe-to-surface message is, how it travels (the `message=` body field prototyped
  then reverted), where the client renders it, and the known silent-failure bug where
  a session that expires mid-upload yields 307 → login → 200 HTML and the client
  treats it as success.
- **CLIENT_PULSE.** A quiet, low-impact channel for the browser to report telemetry
  the server cannot observe: an attempt beginning, and an attempt failing before any
  request leaves the device. Design questions: authentication (an expired session is
  itself a reported failure), ordering (pulse vs. completion), volume (what stops a
  loop from spamming it), and the `localStorage` queue that survives being offline.
  The prototype in `webui-v2.js` + `/upload/failed` answered these by accident and is
  deliberately unshipped; treat it as evidence, not a design.

