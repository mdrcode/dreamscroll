# Backlog

Small, startable tasks. Anything needing design work lives in its own plan; see
[Deferred](#deferred--already-scoped) below. Everything shipped is removed from
this file (git history keeps it) and recorded in `plan/archive/`.

## Next

> **Out of scope today:** **K3, provider-neutral entity types** below. It stays prominent in this
> backlog as the next design item, but do not start it in this run.

> **Knowledge item ids:** stable handles for discussion. Severity is current triage, not a promise
> that the order will not change.

| id | severity | item |
| --- | --- | --- |
| K1 | S2 | Person vs social account |
| K3 | S2 | Provider-neutral entity types (boundaries, coverage, mechanism) |
| K10 | S4 | Normalisation watch |
| K11 | S2 | Illumination retries append rather than replace |

> Former K2 and K6 are folded into K3; their ids are retired. K4 is retired; the incomplete mutation
> grammar is documented in `src/knowledge/knowledge.md`. K5 shipped 2026-11-18. K7 is retired;
> nothing wrote `Cause::Model`. K8 shipped 2026-11-18. K9 is accepted in `plan/pragmatism.md`.

### Production ergonomics (see `plan/prod-ops.md`)

- [ ] **Survey and add the remaining production metrics**, following the pattern established for
  provider rate limits: a structured event in `src/telemetry/prod_event_registry.rs`
  (`provider.rate_limited`, lowercase-dotted), a log-based counter in Cloud Logging named in
  snake_case (`provider_rate_limited`) with a `provider` label extracted from `jsonPayload`, and the
  query + metric filter + alert condition written into `plan/prod-deploy.md` next to the event.
  Survey first, then add one metric per operational question and keep the set small. Candidates
  from the 2026-11-18 cutover: provider request failures (`provider.request_failed`), schema marker
  gap (`applied` vs `required`), knowledge link/identity backlog depth, upload rejections by reason
  (after P5's error codes), task failures by task type.
- [ ] **`gcloud/prod-status.sh`.** One read-only snapshot: drift, revision/digest/traffic, job
  executions, all five queues with expected concurrency, latest backup, error count, optional DB
  status. Exit 1 on anything unexpected.
- [ ] **Release preflight for what manifests only name.** On `--apply` (and `--check`), assert every
  `TASK_QUEUE_NAME_*` exists with the expected concurrency and every `secretKeyRef` has an enabled
  version. The manifest is the next state by design; the failure to avoid is a deploy that looks
  successful and then cannot submit tasks or read a secret.
- [ ] **`Config::required_env_names()` + manifest assertion.** One source for required env, tested
  against `prod-dreamscroll-web.yaml`, instead of `config_def.rs` + manifest + the shell list +
  gitignored `config_prod.env` drifting apart.
- [ ] **Stable API error codes.** A non-sensitive `code` on every `ApiError` response so the UI can
  say "that image is too large" instead of "Upload failed", without exposing internal messages.
- [ ] **Provider health visibility (the rest).** The `provider.rate_limited` event, `prod_event_registry.rs`
  and the `provider_rate_limited` log metric shipped 2026-11-18. Remaining: fetch counters and the
  refusal reason on the online resolve log, a per-provider
  `found`/`empty`/`failed`/`degraded` block in `status`, and the alerting policy (needs a
  notification channel).

### Knowledge cutover (2026-11-18)

- [ ] **Rotate identity resolution per provider by attempt time.** `identity resolve --newest
  --limit N` re-selects the newest N awaiting entities, so unfindable ones keep the window and the
  older tail is never asked (2026-11-18: ~800 entities were starved; a full-scope sweep added 584
  identities). A single `last_attempted_at` column is not enough once a second provider exists: one
  provider's no-match would push an entity back for every provider. Add a per-provider attempt table
  — `knowledge_identity_attempts (knowledge_entity_id, provider, last_attempted_at, attempts,
  last_outcome)` with unique `(knowledge_entity_id, provider)` — stamp every `(entity, provider)`
  pair a run considered with one batched upsert, and order awaiting entities by the joined
  `last_attempted_at NULLS FIRST, id ASC` (earliest-due provider when a run covers several). Outcome
  enables backoff: retry `failed` soon, retry `no_match` only after its cached negative expires.
  Selection is already provider-scoped (`awaiting_identity(&[provider])`), so the change is
  localized to the schema, the call site and the ordering.
- [ ] **Confirm the legacy `illuminations_20261027` table still has no reader** once the new revision
  has run for a week; it is the last pre-reset table in `public`.

### Illumination inference

- [ ] **Revisit the illumination model on 2027-01-01.** Flash 3.8's introductory rate ($0.75 input /
  $3.75 output per 1M) ends 2026-12-31 and becomes $1.50 / $7.50 — identical to Hydrogen. Measured
  on the corpus at the then-shipped prompt (`capture_illumination_v2`, ungrounded; the prompt is
  now v2.4, whose 4,000-character `details` cap does not change this cost shape):

  | cell | $/1,000 now | $/1,000 from 2027 | empty lists | entities/capture | p50 |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | Flash default (then-shipped) | **$7.18** | $14.36 | 2 of 106 | 3.63 | 10.9 s |
  | Hydrogen low | $13.71 | **$13.71** | 8 of 104 | 3.38 | 9.2 s |
  | Hydrogen medium | $19.07 | $19.07 | 10 of 104 | 4.14 | 12.6 s |

  **Decided 2026-11-15: stay on Flash.** It is 1.91× cheaper than the best Hydrogen cell today, and
  Hydrogen never matches it on emission. Hydrogen medium is not the cell to compare: its thinking
  more than doubles billed output (1,725 vs 868 tokens per capture), which is why it looks expensive
  despite identical rates.

  The entity-completeness rule was measured and **reverted**; it took Flash to 0 empty lists and 5.27
  entities per capture but cost 6 points of per-entity connectedness (82% → 76%), with 2 in 5 of the
  additions unrelated to the image. It did not change the ordering: Flash $8.27, Hydrogen low $15.32,
  Hydrogen medium $23.78. A narrower four-line variant fixed Flash's two empty lists at v2-level
  per-entity quality on 22 captures, but was never run at corpus scale.

  After the rate change the paper ranking flips — Hydrogen **low** undercuts Flash by ~5% and is 1.7 s
  faster, giving up 8% of lists and 0.25 entities per capture. Take that decision when the rate
  lands, not now.

  If Hydrogen does become the default, pin `thinking_level: Some(Medium)` in the capture path — its
  provider default is `high`, not `medium`, so an omitted level silently buys the expensive cell.
  Verify with one smoke capture that `params.generation_config.thinking_level` is `medium` in
  `inference_metas`.

### Knowledge identity resolver
- [ ] **Provider-neutral entity types.** It is capture vocabulary — the illuminator names a type
  in free text — and it currently acts as a *veto* in three places: `classes::incompatible` (rejecting
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
  `television_series` silently disabled a whole rule for 23 series, and `classes.rs` deliberately leaves
  `product`, `publication`, `place`, `organization` and `event` **unmapped** because mapping them rejected
  correct claims (`Rotten Tomatoes` is a business, the `Locarno Film Festival` an organization).
  **Wikidata is one identity resolver provider, so `P31` must not be the arbiter.** A provider's classification
  is evidence *scoped to that provider*; the entity's type is ours. Today `classes::incompatible` compares
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

  Concrete gaps folded into this design: define `brand` vs `organization` and `product` vs `software`
  (111 of 179 collision pairs); add `concept`, `organism`, `referent_form`, and `wikidata_class`
  (unblocks 15 of the 17 `unknown` rows and the dahlia/cultivar corpus).

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

- [ ] **Normalisation at the query boundary — watch, do not build.** Entity names reach Wikidata as
  `trim()`ed surfaces and nothing else. That is correct: the label question is an exact `rdfs:label`
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

- [ ] **Make illumination retries append rather than replace.** `insert_illumination_v2` upserts by
  `task_run_id`, then deletes and re-inserts its observations; `insert_illumination_raw` replaces the
  raw row the same way. A retry of one run therefore discards the previous attempt. If
  `knowledge_link` already ran, the graph can keep entities/evidence derived from observations that
  no longer exist; if it had not run, the attempt is still lost from history. A re-illumination from
  a *new* run is unaffected. This is the supersede change and the only part of the illumination-history
  work that touches ingest. Tracked as a tolerated tradeoff in `plan/pragmatism.md`.

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
| Identity `confidence`: label or probability         | [wikidata-cleanup-20261112.md](archive/wikidata-cleanup-20261112.md)  | Anything starts sorting entities by confidence again |
| The title step's own name-relatedness rule            | [wikidata-cleanup-20261112.md](archive/wikidata-cleanup-20261112.md)  | A second provider answers title queries                          |
| Identity fetch enrichment                             | —                                                                       | A consumer needs provider label/description/classes by identity alone |
| Verdict authorship (user/agent)                       | —                                                                       | Graph management is fully supported by the API. Add authorship to verdicts and any mutation audit log: which user/agent made each decision. |
| Empty handle/search cache TTL                         | —                                                                       | Handle/search-empty volume is visible in `fetch_cache` or 429s |
| Media transcode and format policy                  | [upload-formats.md](upload-formats.md)                                 | `rejected_format` fires for a real user, uploads stay multi-MB on cellular, or a non-browser ingestion path carries originals |
| Abandoned task-run status                          | [task-abandoned-status.md](task-abandoned-status.md)                   | Abandonment becomes a production operation                    |
| Projection canonical shape diff                      | [knowledge-projection.md](archive/knowledge-projection.md)             | Projection rule changes again                                  |
| Snapshot catalog tightening (Phase 2)                | [knowledge-snapshot-redesign.md](archive/knowledge-snapshot-redesign.md) | Snapshot retention, validation, or catalog work becomes active |

## Product direction — not yet scoped

Carried forward from `plan/archive/devlog.md`; each needs a design pass before it becomes a task.

- **Pin which illumination of a capture is current.** The view shows `max(illumination_id)` — recency,
  not quality, and in all three re-reads examined the newer reading was the worse one (captures 683,
  697 and 684). A capture-level pointer to the chosen reading is the natural home, and it would be
  the first labelled data in the system: every pinned reading says which run was right, which is the
  only way to answer whether re-illumination helps.
- **Bifurcate the app loop** into `LearnMoreAbout`, `MoreLikeThis`, and `GoBeyond`.
- **Permanent redirects from tombstones.** A merged entity answers `301` to its canonical one:
  `merged_into_id` is a permanent forwarding pointer, kept append-only so unmerging stays a single
  pointer restore. Following stops at a fixed hop cap and errors — exhaustion means a cycle, which
  nothing else guards. `301` not `302`, because the identity moved for good: a browser that cached
  the redirect will keep following it after an unmerge. Entity pages themselves shipped 2026-11-19;
  see [`plan/entity-page.md`](entity-page.md).
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

