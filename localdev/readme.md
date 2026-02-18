# Local Development "Environment"

When running the app outside Docker for local development (ie. via `cargo run`
and using Sqlite and the LocalStorageProvider), this folder will be used to
store all data. This content should be .gitignored.

Running the app locally should not produce content outside of this folder.

## Illumination queue architecture (current)

Dreamscroll now supports two complementary ways to drive illumination:

1. Local worker polling path (existing behavior).
2. Pub/Sub push webhook path (production-oriented behavior).

Both paths use the same service-level processing pipeline (ServiceApi-backed)
to avoid divergence in illumination behavior.

## Pub/Sub push endpoint mapping (important)

The handler for Pub/Sub push lives in `src/rest/r_pubsub.rs` and defines route
segment `/illumination/push`.

That segment is mounted by the internal router, and the cloudrun binary nests
that internal router at `/internal`.

So the effective production path is always:

- `/internal/illumination/push`

And the full Cloud Run URL shape is:

- `https://<your-cloud-run-host>/internal/illumination/push`

This is the URL you should configure as the Pub/Sub push subscription endpoint.

## Local Pub/Sub emulator (optional)

To test push-driven illumination locally with Docker Compose:

1. `docker compose up app pubsub-emulator`
2. `chmod +x ./localdev/pubsub_init.sh`
3. `./localdev/pubsub_init.sh`

The script creates a topic + push subscription in the emulator and targets the
internal webhook at `/internal/illumination/push` on the `app` service.

When a new capture is inserted, the app publishes an illumination task to the
configured Pub/Sub topic. For localdev, the internal webhook is intentionally
open (no auth token required).

## Production-grade Pub/Sub push auth (OIDC)

For Cloud Run / production, the internal webhook supports OIDC JWT validation
for Pub/Sub push subscriptions.

### Verification model

The webhook verifier (via Google's official `google-cloud-auth` Rust crate) checks:

1. JWT signature using Google-published JWKS keys.
2. Issuer (`iss`) is Google Accounts.
3. Audience (`aud`) matches configured expected audience.
4. Optional push service account email match.
5. Optional `email_verified=true` when email matching is enabled.
6. Expiration (`exp`) using standard JWT validation.

### Required env vars for OIDC mode

Set these in production runtime config:

- `DREAMSCROLL_PUBSUB_PUSH_OIDC_AUDIENCE` (required to enable OIDC mode)
- `DREAMSCROLL_PUBSUB_PUSH_OIDC_SERVICE_ACCOUNT_EMAIL` (recommended)
- `DREAMSCROLL_PUBSUB_PUSH_OIDC_JWKS_URL` (optional, defaults to Google certs)

If OIDC audience is configured, Cloudrun binary enables OIDC verification for
`/internal/illumination/push`.

### Fallback modes

If OIDC audience is not set:

- If `DREAMSCROLL_PUBSUB_WEBHOOK_BEARER_TOKEN` is set, static bearer auth is used.
- Otherwise cloudrun startup fails fast and refuses to run.

### Notes on localdev

The localdev binary intentionally keeps the internal webhook unauthenticated,
even if OIDC or bearer env vars exist, so emulator workflows remain frictionless.

## Cloud Run + Pub/Sub setup checklist

Use this sequence when wiring production:

1. Deploy Cloud Run service and note host URL.
2. Choose push endpoint URL:
	- `https://<cloud-run-host>/internal/illumination/push`
3. Configure runtime env vars on Cloud Run:
	- `DREAMSCROLL_PUBSUB_PROJECT_ID`
	- `DREAMSCROLL_PUBSUB_TOPIC_ID`
	- `DREAMSCROLL_PUBSUB_API_BASE_URL` (typically `https://pubsub.googleapis.com`)
	- `DREAMSCROLL_PUBSUB_PUSH_OIDC_AUDIENCE` (match subscription audience)
	- `DREAMSCROLL_PUBSUB_PUSH_OIDC_SERVICE_ACCOUNT_EMAIL` (recommended)
4. Create/update Pub/Sub push subscription:
	- Push endpoint: Cloud Run URL above
	- Push auth enabled with service account
	- Audience set to same value as runtime audience env var
5. Verify end-to-end:
	- Upload capture
	- Confirm task publish succeeded
	- Confirm webhook receives and processes push

## Security note

The `/internal` path is a naming convention, not an automatic network boundary.
Treat it as externally reachable unless ingress/network policy says otherwise.
The security boundary for this endpoint is the configured webhook auth mode.
