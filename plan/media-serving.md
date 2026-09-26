# Media serving: caching and private URLs

Status: implementation in progress (2026-09-25)

## Goals

- Avoid downloading immutable user media repeatedly.
- Keep the production media bucket private; an object URL must not be sufficient for unrestricted access.
- Make the operational configuration observable and repeatable from the command line.
- Preserve local filesystem and fake-GCS development workflows.

## Current state

- Production config uses `STORAGE_BACKEND=gcloud` and a project-specific media bucket.
- `src/storage/url_maker.rs` currently emits the public XML-style URL:
  `https://storage.googleapis.com/{bucket}/{shard}/{uuid}{extension}`.
- The URL maker explicitly says objects must be public and has a TODO for signed URLs.
- `src/storage/gcloud.rs` uploads objects through the Rust client, but does not currently set `Cache-Control` metadata.
- Media URLs are embedded in API/UI response models (`src/api/schema/infomaker.rs` and `src/webui/v2/r_masonry.rs`). The browser therefore fetches GCS directly rather than through Cloud Run.
- Object names are UUID-based and hard to guess, but obscurity is not authorization.
- The previous load balancer/CDN configuration is not present in the current repository. Do not assume it still supplies caching.

### Production inspection (2026-09-25, read-only)

- Active project: the configured Google Cloud project.
- Bucket: the production media bucket, regional `US-CENTRAL1`, `STANDARD` storage.
- Uniform bucket-level access is enabled.
- Public access prevention is `inherited`, not explicitly enforced at the bucket.
- The bucket IAM policy grants `roles/storage.objectViewer` to `allUsers`; the current `storage.googleapis.com/...` URLs are therefore publicly readable. UUID-like object names are not an access control.
- The production Cloud Run service uses its configured runtime service account.
- Object-level `Cache-Control` and HTTP headers required follow-up inspection.

### Upload implementation (2026-09-25)

- Confirmed `google-cloud-storage` 1.18's `write_object` builder supports both `set_cache_control` and `set_content_type`.
- Added `private, max-age=604800, immutable` to new uploads from both `store_bytes` and `store_from_local_path` in `src/storage/gcloud.rs`.
- The database records the inferred MIME type after upload, and the provider API now receives it during upload.
- Existing objects still need metadata inspection/migration.

The MIME-type refinement is now implemented: `insert_capture` passes the inferred MIME type through `StorageProvider`, and GCS upload builders set `Content-Type` when supplied. Local storage accepts and ignores this provider-only metadata.

The checked-in `util/update-media-metadata.sh` script was dry-run and then applied to all 1,078 filtered image objects. A representative object now reports `content-type: image/png` and `cache-control: private, max-age=604800, immutable`; its HTTP response carries the same cache policy and a seven-day `Expires` value.

### Safari verification follow-up (2026-09-26)

- Safari reported a production image object with the old headers; it returned `public, max-age=3600` and `application/octet-stream`.
- The object was included by the migration script's extension filter, but its stored metadata was still at `metageneration: 1`. This means the earlier bulk migration did not actually cover every intended object, despite the script reporting the expected candidate count.
- Repaired this exact object with `gcloud storage objects update`, setting `image/png` and `private, max-age=604800, immutable`.
- A cache-busting query immediately returned the corrected headers and `x-goog-metageneration: 2`. The query-free URL continued to return stale headers briefly, demonstrating that an already-populated intermediary cache can retain the previous response after metadata changes.
- Follow-up required: audit all objects by metadata, not only by listing/count, and repair any remaining objects with `cacheControl` absent or `contentType` equal to `application/octet-stream`. Do not treat the migration count alone as proof of completion.

### Random sample audit (2026-09-26)

- Sampled 50 random image object names from the bucket and inspected each with
  `gcloud storage objects describe --format=json`.
- All 50 sampled objects had no stored `contentType` and no stored
  `cacheControl`; this is a 100% sample failure rate, estimating approximately
  1,078 affected objects out of the current 1,078-object image inventory.
- This superseded the earlier optimistic spot check: the one representative
  object that looked correct had been individually repaired or was not
  representative. The prior bulk migration should not be considered
  successful based on its output/count.
- The Safari-reported object was repaired individually, but the bucket
  requires another complete metadata update. After that update, validation
  should use a random sample and correctly parse JSON field names, not rely on
  one HTTP response.

### Verified full migration completion (2026-09-26)

- The migration script's embedded verifier initially had two bugs: it used
  camelCase field names instead of the `gcloud` JSON names (`content_type` and
  `cache_control`), and its f-string quoting produced repeated `SyntaxError`s.
- Both issues were fixed. The script now materializes the object list, reports
  its total, updates each object, reads metadata back, verifies the expected
  values, counts failures, and exits unsuccessfully if any object fails.
- The corrected full run found and successfully verified all 1,079 image
  objects: `Updated: 1079; failed verification: 0; found: 1079`.
- The earlier random sample was rechecked: 50 checked, 0 failures.
- The Safari-reported object now has the expected metadata. The caching
  migration is complete; future uploads are covered by
  the application upload metadata changes.

### Production object/header inspection (2026-09-25)

- Representative production image object (identifier omitted).
- Bucket inventory: approximately 1,078 image objects.
- Stored `contentType`: absent in `gcloud storage objects describe`; HTTP response is `application/octet-stream`.
- Stored `cacheControl`: absent; HTTP response is `public, max-age=3600`.
- This confirms the original concern: production media currently receives only the one-hour default cache policy and an incorrect generic content type.

### Consequence for the next step

- Existing metadata must be migrated for roughly 1,078 objects; changing only future uploads would leave the current library at `public, max-age=3600` and `application/octet-stream`.
- The object names are all server-generated UUID paths under user-shard prefixes, so an extension-based metadata migration is suitable for the current data. The migration must still be dry-run first and must not infer authorization from the object path.

### Bucket-level defaults research (2026-09-25)

- Cloud Storage does not provide a bucket-level default `Cache-Control` or `Content-Type` object metadata value that newly created objects inherit. These are object metadata fields, not bucket metadata.
- The current approach—setting metadata in the application upload builder and maintaining a repeatable migration command for existing objects—is therefore appropriate.
- Cloud CDN can apply unified cache configuration across objects, but it requires an external Application Load Balancer and would reintroduce the infrastructure intentionally removed from this deployment. It is not needed for browser-local caching.
- GCS built-in caching is primarily useful for publicly accessible objects. With the intended private-media policy, `private` permits the browser's local cache but does not make the object eligible for shared GCS built-in caching. This is the safer privacy/performance tradeoff until signed URL behavior is implemented.

## Why not bucket-wide configuration?

It would be simpler if the bucket could declare a default `Cache-Control` and
have every object inherit it, but Cloud Storage does not support bucket-level
defaults for object metadata. `Cache-Control` and `Content-Type` belong to each
object's metadata record. An object with no explicit `Cache-Control` therefore
falls back to Google's default behavior rather than consulting a bucket policy.

That leaves two practical mechanisms:

1. Set the metadata during every upload. This is now done by the GCS provider,
  including the seven-day browser cache policy and inferred image MIME type.
2. Update existing objects or repair drift with a repeatable command. This is
  handled by `util/update-media-metadata.sh`, which supports a dry run and an
  explicit `--apply` mode.

Cloud CDN has a different kind of bucket-wide cache configuration, but it is a
cache layer in front of the bucket rather than inherited object metadata. It
requires an external Application Load Balancer and is not part of the current
direct-to-GCS design. It would also need a separate privacy design for this
application's per-user media.

Security implication: do not remove the `allUsers` binding until the application can issue working signed URLs and the runtime service account's object permissions have been verified. Removing it will intentionally break the current browser media path first.

## Findings from Google Cloud documentation

- Cloud Storage controls the response `Cache-Control` header from the object's stored metadata. It ignores a request's arbitrary `Cache-Control` header for this purpose. If unset, the normal default is only `public, max-age=3600`; explicitly set metadata is required for a longer lifetime.
- `Cache-Control` metadata is editable after upload, so existing objects can be migrated with `gcloud storage objects update`.
- GCS V4 signed URLs are bearer URLs: anyone possessing one can read the object until expiry. They are not a replacement for application authorization; the application must only issue them after checking the logged-in user's ownership.
- Signed URLs use Cloud Storage XML API endpoints such as `https://storage.googleapis.com/<bucket>/<object>`. The JSON API download endpoint is not the endpoint to use for signed URLs.
- GCS signed URL expiration is limited to 604800 seconds (7 days). A URL valid for weeks cannot be produced as one GCS V4 signed URL.
- Browser caching is keyed by the complete URL. If the application signs the same object with a different timestamp on every response, the changing query string can defeat cache reuse. The implementation must either cache/reuse signed URLs or use an application media endpoint with its own stable URL and authorization behavior.
- Long-lived caching means replacement/revocation is not immediate. This application creates immutable UUID object names, so the preferred policy is to never replace an object at the same name. If an object must be revoked, use a new object name and accept that already-cached copies may remain locally until expiry.

## Proposed target design

### Phase 1: bucket and object metadata

1. Verify the production bucket's IAM and public access state. Enable/retain uniform bucket-level access and confirm there is no `allUsers` object or bucket binding. The Cloud Run runtime service account needs object read access (and object create access for uploads); clients do not.
2. Choose an initial cache policy. Recommended prototype value: `private, max-age=604800, immutable` (7 days). `private` avoids shared-cache exposure while allowing the user's browser to reuse the image; `immutable` reflects the UUID/never-replace contract. Reconsider `private` versus `public` with signed URLs explicitly: signed URLs are bearer credentials, so shared caching must not be enabled casually.
3. Apply that metadata to existing objects with a reviewed, repeatable `gcloud storage objects update --cache-control=...` command. Prefer a script that lists the exact bucket and records the number of objects changed; do not run a broad command until a dry-run/listing has been inspected.
4. Set the same metadata at upload time in `src/storage/gcloud.rs`, alongside the correct `Content-Type` when known. This prevents new objects from reverting to the one-hour default. Confirm the `google-cloud-storage` 1.18 API used by this repository supports setting the final object metadata on `write_object`; if not, issue a metadata update immediately after upload.
5. Verify with both `gcloud storage objects describe gs://...` and `curl -sSI` against a test object. Record `Cache-Control`, `Content-Type`, `ETag`, `Age` (if present), and response status. Test a second request in browser devtools or with a cache-enabled client; repeated `curl` alone does not demonstrate browser-cache reuse.

Example command shapes (fill in the exact object or controlled prefix first):

```text
gcloud storage objects describe gs://BUCKET/OBJECT
gcloud storage objects update gs://BUCKET/OBJECT --cache-control='private,max-age=604800,immutable'
curl -sSI 'https://storage.googleapis.com/BUCKET/OBJECT'
```

For a migration, use `gcloud storage ls 'gs://BUCKET/**'` to inspect the scope, then run the update over the explicitly reviewed object list. Keep the migration command in a checked-in script once the scope is known.

### Phase 2: private signed URLs

1. Remove public access from the production bucket/object IAM and test that an unsigned `curl` returns `403` (or another non-success response), while the Cloud Run service account can still read through the authenticated client.
2. Add a signer to the GCloud storage provider using Application Default Credentials and the existing `google-cloud-storage` crate's `SignedUrlBuilder` (V4). The crate source confirms the builder accepts a `projects/_/buckets/...` bucket path, HTTP method, expiration, and `sign_with(&Signer)`.
3. Keep authorization in the application path that constructs the response: load the media record, verify the current user owns the associated capture/media, then sign only that object's GET URL. Do not sign arbitrary bucket/object strings supplied by a client.
4. Decide URL lifetime and reuse. A practical first version is a 7-day signed URL and an application-level cache or stable response generation strategy. If a fresh signature is generated every API request, the URL changes and client caching will be much less effective. Because the maximum is 7 days, users may need a refreshed URL after expiry; the image bytes can still remain in the browser cache only while the URL remains a cache hit.
5. Add tests for: correct canonical object path and URL encoding; unauthorized users not receiving a URL; unsigned production URL failing; signed GET succeeding; expired URL failing; local and emulator URL behavior remaining unchanged. Do not log full signed URLs because query strings contain bearer credentials.
6. Add metrics/logging that record bucket/object identity, URL generation success/failure, and expiry duration without recording the signature.

Potential implementation shape:

- Move URL generation from a synchronous `UrlMaker::make_url` into an async provider-backed service, or add a separate async `SignedUrlMaker`; signing may require async credential initialization.
- Keep local URLs unchanged.
- Keep the fake-GCS emulator on its existing JSON API URL; V4 signing is a production concern unless emulator support is specifically needed.
- Consider returning a media URL from a dedicated authenticated application endpoint as an alternative. That gives stable URLs and direct user authorization, but makes Cloud Run proxy all image bytes and loses the direct GCS download path. It should not be the default unless 7-day signed URL reuse proves awkward.

### Application integration design (to implement)

`InfoMaker` currently constructs URLs synchronously, while GCS V4 signing is asynchronous (`SignedUrlBuilder::sign_with`). It is used by both the authenticated user API and the service API, and its methods are nested through capture, spark, entity, and preview response construction. The implementation should therefore avoid putting signing inside a low-level synchronous formatter.

Preferred shape:

1. Introduce an async, cloneable storage URL/signing service initialized once in `dreamscroll_web` from ADC (`google_cloud_auth::credentials::Builder::default().build_signer()`).
2. Make the response-building methods that contain media URLs async, converting their iterator chains to explicit loops or `join_all` as appropriate. Keep the local and emulator branches fast and behaviorally unchanged.
3. Keep the existing service/user authorization boundaries intact. User API methods already load user-scoped captures; service API methods are trusted internal paths used by workers and must not become a client-facing authorization bypass.
4. Use a seven-day signed GET URL for GCS production. Do not log the query string. Initially accept signing per response as the simplest correct implementation, then measure whether URL churn defeats browser caching; if it does, add a short-lived in-process URL cache keyed by bucket/object and refresh before expiry.
5. Add a signer-free test constructor or injectable URL signer so unit tests do not require ADC. Do not make local development depend on production credentials.

This is a larger async call-graph change than the cache metadata change. It should be implemented in a focused branch/change set rather than mixed with bucket IAM changes.

## Repeatable command-line runbook

Before changing production:

```text
gcloud config get-value project
gcloud storage buckets describe gs://BUCKET
gcloud storage buckets get-iam-policy gs://BUCKET
gcloud storage objects list gs://BUCKET --format='value(name)' --limit=20
gcloud storage objects describe gs://BUCKET/KNOWN_OBJECT
curl -sSI 'CURRENT_URL'
```

Security checks:

```text
# The unsigned URL must fail after public access is removed.
curl -sS -o /dev/null -w '%{http_code}\n' 'https://storage.googleapis.com/BUCKET/OBJECT'

# Inspect only headers for a signed URL; never commit or log it.
curl -sSI 'SIGNED_URL'
```

After rollout, compare Cloud Run logs, GCS access/logging metrics, and browser network requests. A 200 from a browser's memory/disk cache is the useful signal; a request to GCS on every page render is not.

## Decisions still needed

- Exact cache lifetime: start at 7 days, or shorter during rollout (for example 1 day) and increase after verification.
- Whether “private” means browser-private only, or whether CDN/shared-cache delivery is desired later. For user-private media, browser-private is the safer default.
- Whether a 7-day signed URL is acceptable, including bearer-link sharing during its validity window.
- Which Cloud Run runtime service account is deployed and which minimal Storage IAM roles it currently has.
- Whether media records can ever be replaced at the same object name. If yes, do not use `immutable`; use versioned object names or a shorter cache policy.

## Reference links

- [Cloud Storage object metadata and Cache-Control](https://docs.cloud.google.com/storage/docs/metadata#caching_data)
- [Cloud Storage signed URLs](https://docs.cloud.google.com/storage/docs/access-control/signed-urls)
- [Create signed URLs with Cloud Storage tools](https://docs.cloud.google.com/storage/docs/access-control/signing-urls-with-helpers)
- [Cloud Storage request endpoints](https://docs.cloud.google.com/storage/docs/request-endpoints)
- [Cloud Storage IAM access control](https://docs.cloud.google.com/storage/docs/access-control/iam)
- [Uniform bucket-level access](https://docs.cloud.google.com/storage/docs/uniform-bucket-level-access)
- [Google Cloud CLI `gcloud storage objects update`](https://cloud.google.com/sdk/gcloud/reference/storage/objects/update)
- [Google Cloud Rust Storage crate documentation](https://docs.rs/google-cloud-storage/1.18.0/google_cloud_storage/)
- [Rust `SignedUrlBuilder` source/API](https://docs.rs/google-cloud-storage/1.18.0/google_cloud_storage/builder/storage/struct.SignedUrlBuilder.html)

## Implementation checklist

- [ ] Confirm current bucket IAM, public access prevention, and Cloud Run service account.
- [ ] Confirm current object `Cache-Control` and response headers for representative media.
- [ ] Choose and document cache lifetime and replacement/revocation policy.
- [x] Add a repeatable metadata migration script/command.
- [x] Set cache metadata for new uploads.
- [ ] Remove public access and prove unsigned requests fail.
- [ ] Implement ownership-checked V4 signed GET URLs.
- [ ] Prevent full signed URLs from logs and telemetry.
- [ ] Add unit/integration tests and a production smoke-test runbook.
- [ ] Re-measure browser cache hits and GCS request volume after deployment.
