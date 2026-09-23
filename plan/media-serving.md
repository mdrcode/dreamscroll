# Media serving: caching and private URLs

Status: investigation / implementation plan (2026-09-23)

## Goals

- Avoid downloading immutable user media repeatedly.
- Keep the production media bucket private; an object URL must not be sufficient for unrestricted access.
- Make the operational configuration observable and repeatable from the command line.
- Preserve local filesystem and fake-GCS development workflows.

## Current state

- Production config uses `STORAGE_BACKEND=gcloud` and bucket `dreamscroll-prod-media1`.
- `src/storage/url_maker.rs` currently emits the public XML-style URL:
  `https://storage.googleapis.com/{bucket}/{shard}/{uuid}{extension}`.
- The URL maker explicitly says objects must be public and has a TODO for signed URLs.
- `src/storage/gcloud.rs` uploads objects through the Rust client, but does not currently set `Cache-Control` metadata.
- Media URLs are embedded in API/UI response models (`src/api/schema/infomaker.rs` and `src/webui/v2/r_masonry.rs`). The browser therefore fetches GCS directly rather than through Cloud Run.
- Object names are UUID-based and hard to guess, but obscurity is not authorization.
- The previous load balancer/CDN configuration is not present in the current repository. Do not assume it still supplies caching.

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
- [ ] Add a repeatable metadata migration script/command.
- [ ] Set cache metadata for new uploads.
- [ ] Remove public access and prove unsigned requests fail.
- [ ] Implement ownership-checked V4 signed GET URLs.
- [ ] Prevent full signed URLs from logs and telemetry.
- [ ] Add unit/integration tests and a production smoke-test runbook.
- [ ] Re-measure browser cache hits and GCS request volume after deployment.
