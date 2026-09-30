# Cloud Build tagged-release deployment plan

**Status:** research / proposed; no Google Cloud resources changed.

## Goal

When the movable `prod` tag is pushed or updated in GitHub, Google Cloud should
build the exact commit it points to in Cloud Build, publish its container to
Artifact Registry, and deploy that image to the existing production Cloud Run
service. Semver tags (for example, `v1.2.3`) are informational only and do not
trigger builds. Local macOS development and Docker development remain unchanged.

## Current state

- `Dockerfile` builds the three Rust binaries and packages the active
  `web/v2` assets; Cloud Build can use this as-is.
- `gcloud/cloudbuild.yaml` currently builds and publishes `latest` plus a
  caller-supplied `_IMAGE_TAG`. It does not deploy to Cloud Run.
- `gcloud/docker-build-push.sh` is the laptop-side Docker build/push route.
- Production config identifies project `mdrcode`, region `us-central1`, and
  task/webhook URLs on the existing `dreamscroll-webui-api` Cloud Run service.
  This service name should be verified in the console before wiring deployment.
- `plan/static-serving-cdn.md` describes the current deployment as separate
  from image publishing; this plan proposes the first automated deploy path.

## Proposed production tag flow

1. Create signed, immutable version tags such as `v1.2.3` for release history;
  no Cloud Build trigger matches these tags.
2. Move the `prod` tag to the commit to deploy and push the tag ref to GitHub.
  Cloud Build's GitHub trigger matches only `^prod$` and runs on the commit
  associated with that ref update.
3. The trigger runs a dedicated production build/deploy config, proposed as
  `gcloud/cloudbuild-release.yaml`. It builds and pushes the image under the
  full `$COMMIT_SHA`, then deploys that exact image to the existing Cloud Run
  service in `us-central1`.
4. Cloud Run creates a new revision and shifts traffic according to the
  existing service behavior. The build fails visibly if build, push, or
  deploy fails.

Google's GitHub trigger documentation describes a **Push new tag** event and
states that tag changes matching the configured regex trigger builds. GitHub's
push webhook covers tag pushes, including forced ref updates (`forced: true`),
so force-updating the existing `prod` tag is expected to trigger a build. This
is supported by the documented event semantics, not merely an assumption that
new tag creation is the only event. Do not delete the tag as a deployment
mechanism; deletion is a different event and does not identify a commit to
build. Validate the first force-update end-to-end after creating the trigger.

Because this build can take a long time, do not move `prod` again until its
build/deploy finishes. Otherwise builds for successive commits can overlap and
finish out of order, allowing an older build to deploy after a newer one. A
pre-deploy check that the build's `$COMMIT_SHA` is still the commit referenced
by GitHub's `prod` tag would make rapid updates safer, but adds plumbing; for
the one-person workflow, serialize updates manually initially.

Keep this release config separate from `gcloud/cloudbuild.yaml`: manual builds
should not unexpectedly deploy production, and the existing manual image-push
workflow should remain available during rollout. Do not deploy `latest`; it is
mutable and can race across builds. The `prod` Git tag is the movable deployment
pointer, while the full commit-SHA image tag is the immutable build identity.

## Proposed Cloud Build configuration responsibilities

- Build with `docker build --platform linux/amd64` from the repository root.
- Tag the image with `$COMMIT_SHA` and publish it to the existing
  `us-central1-docker.pkg.dev/$PROJECT_ID/dreamscroll-repo/dreamscroll-web`
  Artifact Registry path.
- Run `gcloud run deploy` with that same image, the verified service name,
  project, and region. Avoid passing application secrets or duplicating the
  runtime environment configuration in the build config; retain the existing
  Cloud Run service settings.
- Prefer a dedicated, narrowly permissioned Cloud Build trigger service
  account over broad project-owner permissions. Grant only the Artifact
  Registry write access, permission to update the target Cloud Run service,
  permission to act as the service's runtime identity, and required log-write
  permissions. Confirm exact IAM roles against the current Cloud Build service
  account model before provisioning.

## GitHub connection implications

The recommended integration is Cloud Build's **2nd-generation GitHub App
connection**, not a GitHub Actions runner and not the older mirrored-repository
integration. A one-time repository connection authorizes Google's Cloud Build
GitHub App to access the selected repository or repositories. Google stores the
connection authorization token in Secret Manager; the Cloud Build service
agent accesses it. Prefer installing the app only on this repository and use a
shared/robot GitHub identity rather than making the connection depend on one
person's account. The app can be uninstalled or its access revoked from GitHub.

The main security implication is the **trigger's build service account**, not
the repository read connection itself. A trigger runs source-controlled build
instructions with that account's Google Cloud permissions. Anyone who can push
a matching tag can therefore cause those instructions to run; if the account
can deploy, treat tag creation rights and changes to the release build config as
production deployment authority. Protect release tags in GitHub, scope the
trigger to a narrow version-tag regex, use a dedicated least-privilege build
identity, and do not give ordinary PR builds production deployment rights.
Avoid an automatic pull-request trigger for the production deploy config:
untrusted PR source can modify build instructions. If CI-on-PR is later added,
make it a separate build with no deployment permissions and configure GitHub
comment control/approval for external contributors.

Cloud Build can report trigger status in GitHub's Checks UI. Sending logs to
GitHub is optional; Google documents sharing the Cloud project ID and trigger
name, and build logs only when that option is enabled. Enabling project-level
Cloud Build/GitHub data sharing is documented as irreversible, so leave it off
unless GitHub-side logs are wanted. Build logs remain available in Google Cloud
Logging regardless.

The GitHub connection and trigger are one-time Google Cloud setup, separate
from normal `git push`/tagging. The connection and trigger need a region; the
linked 2nd-generation repository and trigger must use matching regions. Review
the GitHub App repository permissions during installation and only grant the
access needed to read source and report checks/statuses for this pipeline.

## Build location, billing, and machine-size trade-offs

The `_LOCATION: us-central1` substitution in `gcloud/cloudbuild.yaml` selects
the Artifact Registry location; it does **not** choose where compilation runs.
The Cloud Build execution region is selected separately with `--region` for
manual builds or in the trigger configuration. Keep both in `us-central1` for
now: that co-locates builds with the existing image repository and Cloud Run
service. Moving regions is not a known way to reduce compilation cost and may
add cross-region image transfer charges or latency. Revisit only for data
residency, availability, or measured regional price/performance reasons.

Current Cloud Build pricing (verify before relying on it; Google can change
rates and promotions):

- The published free allowance is **2,500 build-minutes per billing account
  per month**, shared by projects on that billing account. It applies to
  `e2-standard-2` builds in the default pool, not larger machine types. Eligible
  usage consumes the allowance automatically when the project is linked to a
  billing account; after it is exhausted, additional eligible minutes are
  billed. Queued time is not charged; active build time is billed by the
  second.
- The listed `us-central1` default-pool rates are $0.006/minute for
  `e2-standard-2` and $0.0156/minute for `e2-highcpu-8`. A 10-minute build is
  approximately $0.06 and $0.156 respectively, before other charges. The
  higher-CPU machine has a 2.6x per-minute rate, so it must reduce a build's
  active duration by about 62% to lower its build-compute charge. It may still
  be worthwhile for shorter feedback time.
- Current listed `us-central1` default-pool machine rates (not private-pool
  rates; confirm on Google's pricing page before use):

  | Machine type              | vCPU / memory | Price per build-minute | 10 min | 30 min | 60 min |
  | ------------------------- | ------------: | ---------------------: | -----: | -----: | -----: |
  | `E2_MEDIUM`               |      1 / 4 GB |                $0.0030 | $0.030 | $0.090 | $0.180 |
  | `E2_STANDARD_2` (default) |      2 / 8 GB |                $0.0060 | $0.060 | $0.180 | $0.360 |
  | `E2_HIGHCPU_8`            |      8 / 8 GB |                $0.0156 | $0.156 | $0.468 | $0.936 |
  | `E2_HIGHCPU_32`           |    32 / 32 GB |                $0.0624 | $0.624 | $1.872 | $3.744 |

  These are build-compute estimates only, assuming the full duration is
  billable; the `E2_STANDARD_2` free allowance may cover eligible minutes.
- Selecting a larger machine does not draw from the `e2-standard-2` free
  allowance: it is billed at its applicable rate automatically when billing is
  enabled. A budget/alert can notify on spend but is not a hard spending cap.
- Build-minute allowance does not cover separate Artifact Registry storage,
  Cloud Logging/Storage log charges, vulnerability scanning, or network
  egress. Keep the registry in `us-central1` and clean up unneeded image tags
  according to retention needs.

Recommendation: begin with the default machine and compare completed build
durations/costs in Cloud Build. Try `E2_HIGHCPU_8` only as a measured experiment
if reducing wait time matters; compare active duration and actual billed cost,
not just CPU count. For example, pass `--machine-type=E2_HIGHCPU_8` on one
`gcloud builds submit` invocation rather than changing the shared config before
there is evidence it is worthwhile.

## Rust build caching: current behavior and options

The Dockerfile already uses a useful **intra-build dependency-layer cache**:
it copies `Cargo.toml`/`Cargo.lock`, compiles dependencies using dummy source,
then copies the real source and builds the binaries. A source-only change
should therefore reuse the dependency compilation within that Docker build.
Cloud Build workers are ephemeral, however, so that Docker layer cache is not
guaranteed to persist between independent builds.

Caching options and their likely trade-offs for this Rust image:

| Cache type                                                          | Likely win                                                                                                              | Costs / limitations                                                                                                                                                                                                                     | Assessment                                                                             |
| ------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Existing Dockerfile dependency layer                                | Avoid recompiling dependencies after source-only edits within a build                                                   | Does not avoid final application compilation; cache disappears with an ephemeral worker unless imported/exported                                                                                                                        | Keep; already implemented                                                              |
| Pull prior `latest` image + Docker `--cache-from`                   | Simple reuse of matching layers when a cached image contains them                                                       | Pull adds time/egress; ordinary image cache may not retain the intermediate Rust builder layers from this multi-stage Dockerfile, so it may not hit the expensive compile layer                                                         | Do not add blindly; measure cache hits first                                           |
| BuildKit registry cache (`buildx`, registry backend, `mode=max`)    | Can export/import intermediate stages, including builder stages; more promising for this multi-stage image              | Requires enabling/configuring BuildKit/buildx in Cloud Build; cache image takes registry storage and transfer; cache writes can contend between concurrent builds; extra setup/debug surface                                            | Candidate if logs show repeated dependency compilation is expensive                    |
| Persist Cargo `target/` between builds (e.g. Cloud Storage archive) | Can reuse Cargo compilation artifacts even when app source changes; targets the repeated Rust compilation work directly | Large archive upload/download and restore/save time; storage/egress costs; needs careful cache keying by Rust toolchain, lockfile, target, features, and flags; incompatible/stale contents can reduce hits or cause confusing failures | Potentially highest payoff if final compile dominates; benchmark before implementation |
| Remote Cargo compiler cache (e.g. `sccache`)                        | Reuses compiler outputs for unchanged crates across builds without archiving all of `target/`                           | Requires adding/configuring a cache service/backend, compiler-wrapper integration, credentials and cache policy; hit rate depends on deterministic compiler inputs                                                                      | More machinery than warranted until simpler options are measured                       |

Do not assume Docker `--cache-from` solves the Rust compile delay: it reuses
whole Docker instruction layers, while Cargo's `target/` cache can reuse
individual compiler artifacts after source edits. First run a normal build,
inspect its logs to see whether dependency compilation is skipped, and record
wall-clock duration and billed cost. If dependency compilation dominates,
evaluate a BuildKit registry cache; if application recompilation dominates,
prototype Cargo artifact caching or `sccache`. Include cache transfer/restore
time and cost in the comparison, and retain a clean build path that works when
the cache is cold or unavailable.

The cache is an optimization, not a source of truth: cache misses must only make
builds slower, never change build output. Avoid caching secrets or runtime
configuration. Use a dedicated cache reference/bucket and avoid allowing
untrusted pull-request builds to write a cache later consumed by privileged
release builds.

## One-time setup to research/confirm

- [ ] Verify the GitHub repository, production Cloud Run service name/region,
      Artifact Registry repository/image path, and confirm `prod` as the
      movable deployment tag; keep `vX.Y.Z` tags informational.
- [ ] Confirm Cloud Build, Cloud Run, Artifact Registry, and Resource Manager
      APIs are enabled; connect GitHub using Google's Cloud Build GitHub app and
      a 2nd-generation repository connection.
- [ ] Confirm the trigger region and repository connection region match, and
      choose whether the trigger requires approval before production deploy.
- [ ] Install the Cloud Build GitHub App only for the target repository; use a
  shared/robot identity where practical, and review the requested GitHub
  permissions. Note that the OAuth authorization is stored in Secret
  Manager and can be revoked by uninstalling the app.
- [ ] Create/use a dedicated trigger service account; verify its effective
      permissions for Artifact Registry push, Cloud Run deploy, `actAs` on the
      existing runtime service account, and build logging.
- [ ] Protect `prod` and restrict who can move it. Anyone able to update this
  matching tag can execute source-controlled build instructions with the
  trigger service account's permissions. Keep version tags immutable.
- [ ] Keep pull-request validation (if added) separate from production release
  deployment: no deploy permissions for PR builds; use comment control or
  approval for untrusted contributors.
- [ ] Decide whether to enable optional build-log sharing to GitHub. Project ID
  and trigger name are shared, and logs only if opted in; project-level data
  sharing is documented as irreversible.
- [ ] Add the dedicated release build config, then create a test tag on a
      harmless commit or use a staging service/project to validate the pipeline
      before enabling production traffic changes.
- [ ] Verify the deployed revision, image digest/commit identity, service
      health, and Cloud Tasks/webhook behavior after a test deployment.
- [ ] Document rollback: redeploy a known image digest/commit or shift Cloud Run
      traffic to a previous healthy revision. Do not rely on `latest` as the
      rollback identifier.
- [ ] Retain `gcloud/docker-build-push.sh` and the manual build config until
      the automated path has had a successful production release.

## Decisions still open

1. Should the production trigger require a manual approval gate, or deploy
  immediately after a `prod` tag update?
2. Is there a staging Cloud Run service/project for a future end-to-end test?

## Research sources

- [Build repositories from GitHub](https://docs.cloud.google.com/build/docs/automating-builds/github/build-repos-from-github)
  — GitHub App connection, 2nd-generation repository, tag-push triggers, and
  trigger service account selection.
- [Create and manage Cloud Build triggers](https://docs.cloud.google.com/build/docs/automating-builds/create-manage-triggers)
  — tag-change matching, push-to-remote behavior, and trigger setup.
- [GitHub webhook events](https://docs.github.com/en/webhooks/webhook-events-and-payloads#push)
  — push webhook payload for tag refs and forced updates.
- [GitHub REST API: update a reference](https://docs.github.com/en/rest/git/refs#update-a-reference)
  — GitHub ref updates can be forced to point a tag at another commit.
- [Deploying to Cloud Run using Cloud Build](https://docs.cloud.google.com/build/docs/deploying-builds/deploy-cloud-run)
  — build/push/deploy sequence, continuous deployment trigger, and IAM
  requirements.
- [Substituting variable values](https://docs.cloud.google.com/build/docs/configuring-builds/substitute-variable-values)
  — trigger-provided `$COMMIT_SHA` and `$TAG_NAME` values.
- [Cloud Build pricing](https://cloud.google.com/build/pricing) — free build
  minutes, machine-type rates, and additional charges.
- [Cloud Build locations](https://docs.cloud.google.com/build/docs/locations)
  — selecting a build region and location considerations.
- [Increase vCPU for builds](https://docs.cloud.google.com/build/docs/optimize-builds/increase-vcpu-for-builds)
  — choosing a larger default-pool machine for a measured speed experiment.
- [Cloud Build build-speed guidance](https://docs.cloud.google.com/build/docs/optimize-builds/speeding-up-builds)
  — Docker image caching and reducing uploaded build context.
- [Docker registry cache backend](https://docs.docker.com/build/cache/backends/registry/)
  — BuildKit registry cache, including intermediate-stage caching.
- [Artifact Registry pricing](https://cloud.google.com/artifact-registry/pricing)
  — storage and data-transfer charges.

## Initial assessment

This matches the desired workflow with minimal application changes: the current
Dockerfile and Artifact Registry image path are already in place. The main work
is a distinct release build config plus one-time GitHub connection, trigger,
and IAM setup. Deployment to the existing Cloud Run service should preserve its
runtime configuration, but that assumption must be verified in a test release.