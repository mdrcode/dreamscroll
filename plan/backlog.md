# Backlog

Small, startable tasks. Anything needing design work lives in its own plan; see
[Deferred](#deferred--already-scoped) below. Everything shipped is removed from
this file (git history keeps it) and recorded in `plan/archive/`.

## Next

### Knowledge identity resolver

- [x] **Unify the heuristic tokenizer.** The linker tokenises with `text::content_tokens`, the
  resolver's own definition, instead of splitting on whitespace. Measured over the full rebuild:
  two link decisions move, both correct merges (`Nintendo` × 2, `The Criterion Collection` × 2), no
  wrong merges, no losses.
- [x] **`content_tokens` cannot see short tokens.** Two-character runs that carry a digit (`M4`,
  `5G`) are now words; general two-letter runs stay dropped. Measured: `M4 carbine` resolves to
  Q214688, 7 confidences move, nothing lost.
- [ ] **The stemmer disagrees with itself on plurals.** `carbines` → `carbin` while `carbine` →
  `carbine`. A trailing-`e` trim fixes it and was **measured then reverted**: 23 decisions moved,
  and merging `force`/`forces` let a *painting* ("Royal Air Force Museum") outrank the B-17 whose
  description is thin. Blocked on the thin-description/margin problem, not on the stem.
- [x] **`identity inspect` truncates each rung to four candidates.** The decided candidate is now
  always shown and the rest are counted (`… 5 more`), in text and JSON.

- [ ] **Overhaul `entity_type`, driven by data.** It is capture vocabulary — the illuminator names a type
  in free text — and it currently acts as a *veto* in three places: `types::incompatible` (rejecting
  candidates by `P31`), the link decision (declining an attachment), and `merge`.

  What the veto actually costs, measured on the rebuilt graph 2026-11-13: two of the three cases it was
  cited for are **not** stranded. `Pokémon Pokopia` (`software`/`product`) is raised by `proximity scan`
  at 0.85 and folded by `auto-merge` — a happy case, recorded here so it stops being cited as damage.
  `The Criterion Collection` (`organization`/`brand`) is repaired by the verdict log instead, because the
  scorer will not fold two surface texts that differ that much. Both are one live entity afterwards.

  `Lucifer` is **not a duplicate**: the two captures resolved to different items — `Q185498`, the
  name/concept, and `Q11689748`, the `Supernatural` character — and the type difference is what correctly
  kept them apart. They share no identity, so no proximity is raised, and none should be.

  So the cost is a *transient* duplicate the reconciler repairs, not three permanent ones. The design
  argument below stands without a body count: the veto still gives a second provider nowhere to stand,
  and still blocks a person from their own account (`Andra` `real_person` vs `@bioavailablend`
  `social_media_account`).

  The vocabulary is 20 values assigned by hand-in-prompt, so it drifts: `television_show` vs
  `television_series` silently disabled a whole rule for 23 series, and `types.rs` deliberately leaves
  `product`, `publication`, `place`, `organization` and `event` **unmapped** because mapping them rejected
  correct claims (`Rotten Tomatoes` is a business, the `Locarno Film Festival` an organization).
  **Wikidata is one identity resolver provider, so `P31` must not be the arbiter.** A provider's classification
  is evidence *scoped to that provider*; the entity's type is ours. Today `types::incompatible` compares
  the two directly, which means the second provider (IMDb, OpenLibrary, MusicBrainz) has no place to
  stand: it would need its own N×M table of our types against its vocabulary, maintained by hand and
  drifting, and a disagreement between providers would have no representation at all.
  Direction: keep the entity type as a small, closed, **provider-neutral** vocabulary (person, work,
  place, organization, concept, event, artifact, name, …). Each provider owns a mapping from its own
  classifications into that vocabulary, carrying its own confidence, and a disagreement **ranks** rather
  than refuses. A hard refusal stays possible but needs both sides confident: our type unambiguous and
  the provider's classification confidently in a disjoint family. The same rule then serves merge and
  admission without a second table. Needs design; see the merge item below, which is the same defect
  caught one layer later.

- [ ] **Refuse a merge between incompatible types.** `merge` is driven by identity equality alone:
  `resolve` finds two entities holding the same `(scheme, id)` and folds them, with no check that they
  agree about what kind of thing they are. Six of the eight live collisions are exactly that — a
  `fictional_character` holding a film's item, a `book` holding a movie's — stopped only because the
  same-illumination veto happened to fire; in separate captures they would have merged silently.
  Settle first whether the guard belongs here (an `ensure!` on type compatibility, the shape of the
  tombstone guard) or upstream in resolution (a character should not resolve to a work's item). The
  answer decides whether the collision record's `reason` needs a `type_incompatible` value.
- [ ] **Make `link_illumination` transactional.** Linking one reading writes several times per
  decision: create or load the entity, attach the observation, add the capture's identity (or fold
  and discard the row it just made), then project. A crash between any two leaves an entity with
  members and no identity, or an identity on an entity the projection never recomputed — and
  nothing repairs either. `merge` already shows the shape: `merge_in_transaction<C>` takes the
  connection and its five writes share one transaction. Two things block doing the same here. The
  executor has to take the connection as a parameter rather than reach for `&self.db.conn`, and
  `Evidence` has to borrow that connection instead of owning one: an identity written outside the
  transaction would reference a `knowledge_entity_id` the transaction has not committed, which is a
  foreign-key violation rather than a torn read. `Evidence` holds a `DatabaseConnection` today and
  seven call sites build one.
  **Observed consequence, from the 2026-11 audit:** the identities are written by
  `write_observation_identities`, and only for the observations the linker decided *in that call*
  (`linked`, `helper.rs`). A failure after the links land leaves the observations attached and the
  identity unwritten; the retry finds nothing pending, so `linked` is empty and the handle identity
  is never written. A torn read heals on the next projection — a lost identity does not: the entity
  stays unfindable by the handle that named it until someone re-links it by hand.
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
  away from being undone". Nothing implements it. `plan/archive/knowledge-canonical-entities.md` §Repair
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
- [ ] **Decide whether a handle match can be refused.** A platform link proves the link is the same;
  it says nothing about the thing behind it. Two captures naming `X/@elonmusk` as
  "technologist and inventor" and as "random bullshitter" are today merged silently — the second is
  absorbed as corroboration, `project` picks one description, and nothing records the disagreement.
  The proximity record is the machinery; what is missing is a corroboration bar inside the linker
  for handle matches, and a decision about what the executor does with a refused match (link and
  note a proximity, or leave unlinked and re-evaluate next run). Needs a real case before designing
  thresholds — a report-only count of handle matches that would fail a corroboration test is the
  first step. Shape is settled: a refused match needs no new decision variant — it is `Create` with
  the identity withheld and `best` naming the entity declined, which `project` then leaves
  `provisional` for free. Deferred with it: whether a refusal also writes a proximity (which would
  flag the established entity for a look); today "shares an identity" is a proximity, and diluting
  it is a separate decision.
- [ ] **Score the entity-resolution precision sheet.** 100 labelled rows sit in
  `localdev/analysis/entity-promotion/evidence/review/precision_sheet.csv`
  (45 identified / 25 ranked / 15 ambiguous / 15 unidentified). Precision ≥ 0.90
  lets the `identified` dimension drive promotion as-is; below 0.75 the
  verification step needs rework. See `plan/archive/knowledge-evidence-prototype.md` §8.
- [ ] **Replace the keyword type-filter with `P31` class matching.** The filter
  rejected 444 rows whose exact-label item existed, 247 of them with a single
  candidate. A curated `type → {class QIDs}` map tested through
  `wdt:P31/wdt:P279*` is the fix. Same doc, §9.
- [ ] **Define `brand` vs `organization`, `product` vs `software`.** These two
  undefined type pairs account for 111 of 179 collision pairs.
- [ ] **Add `concept` and `organism` entity types, plus `referent_form` and
  `wikidata_class`.** Settled by the 2026-10-29 audit in `plan/archive/knowledge-entity-types.md`:
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
  `plan/archive/static-serving-cdn.md`, `plan/archive/upgrade_3rd_party_crates.md`.

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

- [ ] **Make the scoring agree with itself.** An audit of `src/knowledge` (2026-11-04) found four
  places where a function contradicts its own rule. Item 2 is fixed; item 1 needs a before/after diff
  (`reset` → `link` → `identity resolve --offline` → `compare`) to prove it moves, not just a unit test.

  1. **Two fits for one candidate.** `wikidata/policy.rs::parse_search` builds IDF weights over all
     of the answer's candidates; `decide` builds them over the *admitted* subset. One candidate is
     therefore scored against two weight sets inside one `parse`, so a search answer can be accepted
     on one fit and ranked on another. `ranking.rs`'s own doc says the corpus is "the answers the
     question returned", which sides with `parse_search` — so the fix is for `decide` to rank the
     admitted subset against the full answer's weights, not for `parse_search` to narrow its corpus.
  2. ~~**The margin guards the wrong competitor.**~~ **Fixed** (2026-11-12): `Ranking::rank` sorts
     by `fit` and lets the blend break ties only, so `decide`'s `runner_up` is the closest competitor
     *by the measure it margins*. Locked by
     `policy::tests::the_runner_up_is_measured_in_the_same_order_as_the_winner`.
  3. ~~**Two orderings for "most substantiated".**~~ **Fixed by construction** (2026-11-04): the SQL
     `ORDER BY` is gone, `holders_of_identity` sorts with `Worth::order`, and `merge_open` asks the same
     function — one comparator, no NULL rule to keep in sync.
  4. **The third holder's pair is never asked.** With three or more holders `scan` pairs every other
     holder against `holders.first()`, so `(h2, h3)` never becomes a proximity and that question is
     invisible in `proximity list`. The corpus never exceeds two holders, so nothing is missing yet.

  1 is ranker policy and belongs with the resolver, not the linker. Full statement, options and the
  rebuild recipe: [wikidata-cleanup-20261112.md](wikidata-cleanup-20261112.md). Note that the scoring
  split (`scoring.rs` verbs, `linker/heuristic/scoring.rs` policy) makes it cheaper to state:
  `parse_search` builds `Weights` itself instead of asking the policy, which is the visible oddity
  rather than a hidden one.

- [ ] **`snapshot::reset::tests::two_resets_keep_two_separate_snapshots` is racy.** Observed once
  2026-11-12 under parallel test execution: `relation "knowledge_entity_membership__<ts>" does not
  exist`. `reset` keeps a bounded number of snapshots and prunes by age, so one test's reset can drop
  another's snapshot tables while they are being read; the test passes alone and on rerun. Either
  serialise the snapshot tests or scope the prune to past the snapshot under test. Found while
  verifying the wikidata review (`plan/wikidata-cleanup-20261112.md`), not caused by it.

- [ ] **Normalisation at the query boundary — watch, do not build.** Entity names reach Wikidata as
  `trim()`ed surfaces and nothing else. That is correct: the label rung is an exact `rdfs:label`
  match (folding case or collapsing inner spaces would *cause* misses), the Wikipedia title API
  normalises case and underscores itself, and search is tokenised. Measured 2026-11-12 over 2,281
  entities: 0 names with leading/trailing whitespace, 0 not in NFC, 0 containing invisible
  characters (ZWSP/ZWNJ/soft hyphen/BOM/NBSP/word joiner), 0 with doubled inner spaces.
  **Tripwire:** if an illuminator ever emits zero-width or NFD text the symptom is a *silent
  duplicate entity* — the exact-label query misses **and** `text::comparable` cannot fold the
  string either (its table is precomposed-only, per the note in `text/fold.rs`). Fix it in
  `text/fold.rs` (NFC, strip `Cf`) and bump `norm_fingerprint`; not in `plan.rs`, because comparison
  needs it at least as much as querying does.

- [ ] **Decide how admin privilege reaches the common APIs.** Today an admin can do a thing only if a
  route was written for it. `POST /api/queues/illuminate` resolves the capture through the caller's own
  list, so an admin asking for someone else's capture gets a 404 — *before*
  `ensure_task_submission_allowed` (`is_admin() || owner`) is ever consulted. The authority already
  exists; the lookup above it hides the capture. A parallel route
  (`POST /api/admin/captures/{id}/illuminate`) was prototyped on 2026-11-07, worked end to end, and
  was **abandoned**: it cost a client method, a CLI flag, a route handler, a model-id field threaded
  through `AdminApiClient` and eight test constructors, for one operation. Reverted rather than
  carried.
  **Debate:** parallel `/admin` routes per operation (explicit; duplicates each handler and drifts), or
  admin credentials accepted on the base routes with the ownership lookup made admin-aware (one
  handler; every read path must then know about admin). The second matches where the authority already
  lives — the task layer. Not urgent until a second admin-only operation appears; that one will show
  which shape repeats.

- [ ] **Decide whether a person and their social account are one entity.** `social_media_account` is
  a type of its own (148 live entities), distinct from `real_person`, so a capture that names the
  person and one that names the account become two entities. Live case: `Andra` (`real_person`,
  identity `x_twitter:bioavailablend`) and `@bioavailablend` (`social_media_account`, identity
  `instagram:bioavailablend`) — same handle, same subject, two entities, and `proximity` cannot pair
  them because the identities are in different platform namespaces. Blocking now offers each to the other
  and the link decider declines **on type**, which is consistent rather than buggy.
  Settle the vocabulary before any mechanism. If they are one thing, the type signal is wrong for a
  whole class, and the typing rule (not the decider) is what has to change. If they are two, the split
  stays, and `alt_types` is where "this person's item also covers their account" should be recorded.
  Do not special-case the link decider for one handle — that is the hack this item exists to avoid.

- [ ] **`cargo fmt` the world, then gate it.** 47 files are dirty under `cargo fmt --check`, so every
  change carries formatter noise next to its real diff and a reviewer cannot tell them apart. Nothing
  enforces it — not CI, not a pre-push hook — so each commit can leave a hunk behind and the backlog
  only grows. Do it as one behaviourless commit: `cargo fmt` and nothing else, so `git diff -w` against
  the parent is empty and `cargo test --lib` proves it. Then add `cargo fmt --check` to whatever runs
  before a merge, or the same drift starts again the next day.

- [ ] **Make illumination retries append rather than replace.** `insert_illumination` upserts by
  `task_run_id`, then deletes and re-inserts its observations, so a retry of one run discards the
  previous attempt before `link` ever sees it — the graph's "every reading survives" rule silently
  loses the reading. A re-illumination from a *new* run is unaffected. This is the supersede change
  and the only part of the illumination-history work that touches ingest.

- [ ] **Surface the ghost set.** "Every member of this entity is superseded" is a one-line query and a
  decent drift signal, and nothing reports it; `knowledge_stats` is where it belongs.

- [ ] **Rebuild the resolution gold-set harness.** The 13 verified rows survive in
  `localdev/analysis/entity-promotion/evidence/resolve-gold.tsv`, but the harness that scored them
  (`examples/eval_resolve`) went with the replay driver. Without it a scoring change is measured on
  the whole corpus only, so a regression on a specifically hard case stays invisible until it moves
  a claim count.

- [ ] **Fetch enrichment for an identity.** `enrichment` is reserved vocabulary on
  `knowledge_identities` (label, description, classes) and an identity resolver fills them on the way past,
  but nothing reads them and no pass fetches them by identity alone. Decide whether it is a request
  kind in the resolver ladder or a separate pass over identified entities. `project` is where a
  reader would appear.

## Deferred — already scoped

Promote an item into **Next** when its trigger fires or it becomes the focus.

| Topic                                              | Plan                                                                   | Gate                                                          |
| -------------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------- |
| Incremental feed updates after upload              | [feed-view-incremental-updates.md](archive/feed-view-incremental-updates.md)   | Feed refetch becomes user-visible cost                        |
| Protected-route tests through real auth middleware | [testing-auth-routes.md](archive/testing-auth-routes.md)                       | Next change to auth or route-level authorization              |
| Browser tests for SSE/refresh/upload               | [testing-js-and-browser.md](archive/testing-js-and-browser.md)                 | First client bug that unit tests miss                         |
| Coverage strategy beyond the two tiers             | [testing-coverage.md](archive/testing-coverage.md)                             | Coverage discussions block a change                           |
| Private media URLs                                 | [media-serving.md](media-serving.md)                                   | Multi-user access or CDN/LB work starts                       |
| API token hardening                                | [api_token_hardening.md](archive/api_token_hardening.md)                       | API usage grows beyond first-party clients                    |
| Entity taxonomy pass                               | [knowledge-entity-types.md](archive/knowledge-entity-types.md)                 | Environment fully backfilled with v2                          |
| htmx 4.0 migration                                 | [htmx4.md](archive/htmx4.md)                                                   | 4.x becomes `latest`, or 2.x breaks a behavior we rely on     |
| Wikidata entity linking                            | [knowledge-wikidata-recommender.md](archive/knowledge-wikidata-recommender.md) | Entity identity across captures becomes a product requirement |
| Knowledge entity disclosure policy                 | [knowledge-canonical-entities.md](archive/knowledge-canonical-entities.md)     | Multi-user knowledge pages become a product requirement      |
| Screenshot provider selection                      | [screenshot_3rd_party.md](archive/screenshot_3rd_party.md)                     | Link-preview latency or fidelity is measured                  |
| Static/CDN serving decision                        | [static-serving-cdn.md](archive/static-serving-cdn.md)                         | Static bundle cost or latency justifies a network layer       |
| `/events` catch-up query reduction                 | [event-streaming-optimization.md](archive/event-streaming-optimization.md)     | Connection count or DB load becomes material                  |
| Crate upgrade sweep                                | [upgrade_3rd_party_crates.md](archive/upgrade_3rd_party_crates.md)             | Scheduled maintenance window                                  |
| Client pulse + `upload_attempts`                   | [upload-attempts.md](archive/upload-attempts.md)                               | A client-side upload failure goes unexplained for a second time |
| Identity `confidence`: label or probability         | [wikidata-cleanup-20261112.md](wikidata-cleanup-20261112.md)  | Anything starts sorting entities by confidence again |
| The title rung's own name-relatedness rule           | [wikidata-cleanup-20261112.md](wikidata-cleanup-20261112.md)  | A second provider answers title queries                          |
| `SurfaceTitle` as a deferred rule                    | [wikidata-cleanup-20261112.md](wikidata-cleanup-20261112.md)  | `inspect` needs a change, or a third consumer wants the ladder    |
| Media transcode and format policy                  | [upload-formats.md](upload-formats.md)                                 | `rejected_format` fires for a real user, uploads stay multi-MB on cellular, or a non-browser ingestion path carries originals |

## Product direction — not yet scoped

Carried forward from `plan/archive/devlog.md`; each needs a design pass before it becomes a task.

- **Pin which illumination of a capture is current.** The view shows `max(illumination_id)` — recency,
  not quality, and in all three re-reads examined the newer reading was the worse one (captures 683,
  697 and 684). A capture-level pointer to the chosen reading is the natural home, and it would be
  the first labelled data in the system: every pinned reading says which run was right, which is the
  only way to answer whether re-illumination helps.
- **Bifurcate the app loop** into `LearnMoreAbout`, `MoreLikeThis`, and `GoBeyond`.
- **Entity pages, and permanent redirects from tombstones.** One page per knowledge entity, and a
  merged entity answers `301` to its canonical one: `merged_into_id` is a permanent forwarding
  pointer, kept append-only so unmerging stays a single pointer restore. Following stops at a fixed
  hop cap and errors — exhaustion means a cycle, which nothing else guards. `301` not `302`, because
  the identity moved for good: a browser that cached the redirect will keep following it after an
  unmerge.
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

