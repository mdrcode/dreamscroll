# Media serving: caching and private URLs

Status: near-term signed URL implementation in progress (2026-09-25)

## Decision summary

- **Caching:** `private, max-age=604800, immutable` for immutable image objects.
- **Near term:** direct Cloud Storage URLs signed with GCS V4 for seven days.
- **Security:** Dreamscroll authorizes the user before issuing a signed URL; remove `allUsers` object access only after verification.
- **Long term:** Cloud CDN behind an external Application Load Balancer, using signed cookies for prefix-scoped authorization and stable media URLs.
- **Not in scope:** restoring the load balancer/CDN for the prototype before direct signed URLs are tested.

## Why not bucket-wide defaults?

Cloud Storage does not support bucket-level defaults inherited by objects for
`Cache-Control` or `Content-Type`; these are object metadata fields.

The project therefore uses two mechanisms:

1. `src/storage/gcloud.rs` sets the cache policy and inferred MIME type on every new upload.
2. `util/update-media-metadata.sh` updates existing objects. It defaults to dry-run, requires `STORAGE_GCLOUD_BUCKET_NAME` explicitly, and `--apply` performs per-object verification.

Cloud CDN can apply unified cache behavior, but that is a separate edge-cache
layer requiring an external Application Load Balancer, not bucket metadata.

## Completed caching work

- New GCS uploads set `private, max-age=604800, immutable` and the inferred `Content-Type`.
- The migration script materializes its object list, reports totals, reads metadata back, verifies `content_type` and `cache_control`, counts failures, and exits unsuccessfully if verification fails.
- The final production run updated and verified 1,079 objects with zero failures. A random sample of 50 was then rechecked with zero failures.
- Browser verification confirmed the corrected response headers after stale intermediary responses from the old one-hour policy had expired or were bypassed.

## Near-term privacy design: direct GCS signed URLs

`UrlMaker::make_url` now returns one of:

- A local filesystem URL.
- A configured fake-GCS emulator URL.
- A seven-day GCS V4-signed GET URL in production.

There is no unsigned production fallback. `UrlMaker::from_config` initializes
the ADC signer for production GCS and returns an error if signer construction
fails. Local and emulator configurations do not require ADC credentials.

Media response construction is asynchronous because signing is asynchronous.
Existing user-scoped capture queries provide the application authorization
boundary: only authorized media records should reach the URL maker. The URL
maker does not know users or query the database, so callers must not pass it
untrusted storage metadata.

Signed URLs are bearer credentials. Do not log them. Anyone who obtains one can
read that object until expiry. Browser caching is keyed by the complete URL, so
regenerating a different signature on every response may reduce cache reuse;
add a short-lived URL cache only if production measurement shows this matters.

### Implementation status

- [x] ADC-backed V4 signer using existing Google Cloud crates.
- [x] Async signed URL response integration.
- [x] No unsigned production fallback.
- [x] Configured emulator endpoint handling.
- [x] Unit tests for local URLs, extension handling, emulator behavior, unknown providers, and missing production signer.
- [ ] Deploy and verify signed URLs while the bucket remains public.
- [ ] Audit all media-producing response paths for user authorization.
- [ ] Remove the bucket's `allUsers` object-viewer binding.
- [ ] Verify unsigned requests fail and signed requests succeed.
- [ ] Add broader URL/path and authorization integration tests.

## Ideal long-term state: Cloud CDN signed cookies

If stable media URLs, edge caching, prefix-scoped authorization, centralized
revocation, or bandwidth economics become important, evaluate:

1. A private Cloud Storage bucket behind an external Application Load Balancer with Cloud CDN and a backend bucket.
2. Dreamscroll authenticates the user and issues a Cloud CDN signed cookie scoped to that user's media URL prefix.
3. The browser uses stable media URLs without per-object GCS signature query parameters.
4. Cloud CDN validates the cookie and caches image bytes at the edge.

This is closer to “the user may access this URL prefix” than per-object GCS
signatures. It requires careful path design: a cookie for a broad prefix grants
access to every object under it. Signed cookies remain bearer credentials and do
not replace Dreamscroll authentication when the cookie is issued.

Tradeoffs include an external load balancer, backend-bucket configuration,
certificate/DNS setup, CDN signing-key management, deployment automation,
monitoring, and additional cost. Do not restore this infrastructure solely for
the current prototype unless those benefits justify it.

Migration path: introduce a CDN media hostname and backend bucket, configure
signed-cookie key management and prefix claims, issue cookies from the
authenticated application, test private-bucket access, migrate media URLs, and
then remove the direct GCS URL path.

## Operational runbook

Before changing production IAM:

```text
gcloud config get-value project
gcloud storage buckets describe gs://BUCKET
gcloud storage buckets get-iam-policy gs://BUCKET
gcloud storage objects describe gs://BUCKET/KNOWN_OBJECT
```

After deployment, inspect a media URL in browser developer tools. A production
URL should contain `X-Goog-Algorithm`, `X-Goog-Credential`, `X-Goog-Expires`,
and `X-Goog-Signature`. Do not paste complete signed URLs into logs, commits, or
the plan.

Only after signed delivery works:

```text
curl -sS -o /dev/null -w '%{http_code}\n' 'https://storage.googleapis.com/BUCKET/OBJECT'
curl -sSI 'SIGNED_URL'
```

Expected results are a non-success response for the unsigned URL and `200` for
the signed URL. Also verify the Cloud Run runtime identity can still read and
write objects through the authenticated client.

## References

- [Cloud Storage object metadata and Cache-Control](https://docs.cloud.google.com/storage/docs/metadata#caching_data)
- [Cloud Storage caching](https://docs.cloud.google.com/storage/docs/caching)
- [Cloud Storage signed URLs](https://docs.cloud.google.com/storage/docs/access-control/signed-urls)
- [Cloud Storage request endpoints](https://docs.cloud.google.com/storage/docs/request-endpoints)
- [Cloud Storage IAM](https://docs.cloud.google.com/storage/docs/access-control/iam)
- [Uniform bucket-level access](https://docs.cloud.google.com/storage/docs/uniform-bucket-level-access)
- [Cloud CDN signed cookies](https://docs.cloud.google.com/cdn/docs/using-signed-cookies)
- [Cloud CDN signed URLs and cookies](https://docs.cloud.google.com/cdn/docs/using-signed-urls)
- [Cloud CDN with a backend bucket](https://docs.cloud.google.com/cdn/docs/setting-up-cdn-with-bucket)
- [Google Cloud CLI object updates](https://cloud.google.com/sdk/gcloud/reference/storage/objects/update)
- [Google Cloud Rust Storage crate](https://docs.rs/google-cloud-storage/1.18.0/google_cloud_storage/)
