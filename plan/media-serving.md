# Media serving

Status: caching implemented; private URL delivery deferred to the load-balancer/CDN architecture.

## Current caching policy — implemented

Media objects are immutable UUID-based images. The production policy is:

```text
Cache-Control: private, max-age=604800, immutable
```

Implemented safeguards:

- New GCS uploads set the cache policy and inferred image `Content-Type` in `src/storage/gcloud.rs`.
- Existing production objects were migrated with `util/update-media-metadata.sh`.
- The migration script supports a dry run, requires `STORAGE_GCLOUD_BUCKET_NAME` explicitly, and verifies each update by reading metadata back.
- The production migration completed successfully for 1,079 objects with zero verification failures.
- A random sample of 50 objects was rechecked successfully.

This policy supports browser-local caching for seven days. It does not make the private-media authorization problem go away.

## Why not bucket-wide defaults?

Cloud Storage does not provide inherited bucket-level defaults for object
`Cache-Control` or `Content-Type`. Those are object metadata fields, so the
application must set them during upload and a repeatable command must repair or
migrate existing objects.

## URL privacy decision

The current direct GCS URLs remain public because the bucket still grants
`roles/storage.objectViewer` to `allUsers`. UUID-like object names are not an
authorization mechanism.

We briefly evaluated GCS V4 signed URLs, but that approach is not suitable for
this application’s homepage path. It has several serious problems:

- Each image URL becomes hundreds of bytes longer; 50–100 images add tens of KB to the page response.
- Signing through the IAM `signBlob` API adds network latency.
- Signing each image serially made image-heavy pages take several seconds on the server.
- Signing concurrently reduces latency but introduces bursty IAM traffic and more complex response construction.
- Fresh signatures change the complete URL and can defeat browser cache reuse.
- In-process URL caching is not a reliable revocation mechanism and complicates multi-instance behavior.
- Signed URLs are bearer credentials, not user-bound authorization; anyone who obtains one can use it until expiry.

The signed-URL experiment was reverted. Do not remove the public bucket binding
until the replacement architecture is ready and verified.

## Ideal long-term state: load balancer + Cloud CDN signed cookies

Wait for the proper load-balancer architecture before implementing private URL
delivery. The intended design is:

1. Put the private media bucket behind an external Application Load Balancer with a backend bucket and Cloud CDN.
2. Keep ordinary media URLs stable and free of per-object signature query parameters.
3. After authenticating the user, Dreamscroll issues a Cloud CDN signed cookie scoped to that user’s media URL prefix.
4. Cloud CDN validates the cookie, authorizes the prefix, and caches image bytes at the edge.

This matches the desired model—“this user may access this URL prefix”—better
than per-object GCS signed URLs. Prefix design must be exact: a cookie covering
a broad prefix grants access to every object below it. Signed cookies remain
bearer credentials and require HTTPS, secure cookie settings, expiration, and a
revocation/key-rotation plan.

This architecture adds an external load balancer, backend-bucket configuration,
Cloud CDN signing keys, certificate/DNS setup, deployment automation, monitoring,
and cost. It is deferred until those benefits justify restoring the infrastructure.

## Next steps

- [x] Implement and verify the object cache policy.
- [ ] Design the media URL prefix layout for cookie authorization.
- [ ] Plan the external Application Load Balancer and backend bucket.
- [ ] Configure Cloud CDN signed-cookie key management.
- [ ] Issue scoped cookies from authenticated Dreamscroll sessions.
- [ ] Test private bucket access, CDN authorization, cache behavior, and revocation.
- [ ] Remove public bucket access only after the CDN path is proven.

## References

- [Cloud Storage object metadata and Cache-Control](https://docs.cloud.google.com/storage/docs/metadata#caching_data)
- [Cloud Storage caching](https://docs.cloud.google.com/storage/docs/caching)
- [Cloud CDN signed cookies](https://docs.cloud.google.com/cdn/docs/using-signed-cookies)
- [Cloud CDN signed URLs and cookies](https://docs.cloud.google.com/cdn/docs/using-signed-urls)
- [Cloud CDN with a backend bucket](https://docs.cloud.google.com/cdn/docs/setting-up-cdn-with-bucket)
- [External Application Load Balancer with Cloud Storage](https://docs.cloud.google.com/storage/docs/hosting-static-website)
- [Google Cloud CLI object updates](https://cloud.google.com/sdk/gcloud/reference/storage/objects/update)
