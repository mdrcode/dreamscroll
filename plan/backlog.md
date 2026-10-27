# Backlog

Small, startable tasks. Anything needing design work lives in its own plan; see
[Deferred](#deferred--already-scoped) below. Everything shipped is removed from
this file (git history keeps it) and recorded in `plan/archive/`.

## Next

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
| Screenshot provider selection                      | [screenshot_3rd_party.md](screenshot_3rd_party.md)                     | Link-preview latency or fidelity is measured                  |
| Static/CDN serving decision                        | [static-serving-cdn.md](static-serving-cdn.md)                         | Static bundle cost or latency justifies a network layer       |
| `/events` catch-up query reduction                 | [event-streaming-optimization.md](event-streaming-optimization.md)     | Connection count or DB load becomes material                  |
| Crate upgrade sweep                                | [upgrade_3rd_party_crates.md](upgrade_3rd_party_crates.md)             | Scheduled maintenance window                                  |

## Product direction — not yet scoped

Carried forward from `plan/devlog.md`; each needs a design pass before it becomes a task.

- **Bifurcate the app loop** into `LearnMoreAbout`, `MoreLikeThis`, and `GoBeyond`.
- **Mind Map** view over the knowledge graph.
- **Derived media resolutions.** `thumbnail_url` points at the original asset, not a
  separately generated thumbnail; large screenshots are served at full size.

