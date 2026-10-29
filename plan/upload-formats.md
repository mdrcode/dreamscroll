# Upload image formats

**Status:** policy implemented and deployed 2026-10-29 (blocklist, 415s, client
`accept` steering). Media transcode and client-side re-encode deferred.
**Last updated:** 2026-10-29.

> **See also:**
> - `plan/upload-attempts.md` — attempt tracking and the `upload_attempt` log events.
> - `src/api/user/insert_capture.rs` — the only place bytes become a media row.
> - `plan/README.md` — the index.

## The question

Two policies, and they simplify or complicate everything downstream:

- **Defer to the client.** Accept whatever the OS hands us, in standard formats,
  and let the browser decide what to encode.
- **Accept proprietary formats.** Take HEIC/HEIF as input and normalize it
  server-side.

Everything else — the upload cap, where transcoding lives, what the client does,
what `upload_attempts` records — is downstream of this choice.

## Evidence from production

Five photo uploads on 2026-10-29, after the cap was raised to 12 MiB:

```
02:19:12  stored .png  4.4 MB   Chrome iOS
02:21:58  stored .png  4.9 MB   Chrome iOS
02:23:44  stored .png  7.3 MB   Chrome iOS
02:25:34  stored .png  5.4 MB   Safari iOS
02:25:41  stored .png  6.5 MB   Safari iOS
```

Three facts, all measured rather than assumed:

1. **HEIC never reached the server.** The phone's originals were ~1.1–1.2 MB
   HEIC; the wire payloads were 5.4 and 6.5 MB PNG. iOS transcodes before the
   upload, so the server has never seen a HEIC byte in practice.
2. **The expansion is ~5×.** That makes the cap misleading: 12 MiB is roughly
   *2.2 MB of HEIC*, and modern iPhone photos are 1.7–2.5 MB. We are on the
   boundary, which is exactly why a 4.4 MB upload succeeded while 5.4 and 6.5 MB
   failed at the old 5 MiB cap.
3. **The browser is not the variable.** The same two files failed in Chrome
   before the cap change and succeeded in Safari after it; Chrome also uploaded
   4.4, 4.9 and 7.3 MB files successfully in between.

So today we already "defer to the client", and the client is expensive and
opaque: `accept="image/*"` produced PNG for photos, and there is no documented
contract governing that choice.

## What browsers actually support

HEIC/HEIF display support, per [caniuse](https://caniuse.com/heif):

| Engine | Support |
| --- | --- |
| Safari 17+ (macOS + iOS) | ✅ |
| Chrome / Edge / Opera (all platforms) | ❌ — including current releases |
| Firefox (all platforms) | ❌ |
| Android browsers, Samsung Internet | ❌ |

**HEIC is an ingest format, not a delivery format.** Storing it and serving it
back would render on Safari only — the feed would be broken images for every
desktop Chrome/Firefox user. This is the strongest constraint in the whole
decision.

Two further behaviours matter, both poorly specified:

- **iOS converts HEIC on upload, and `accept` steers it.** Community reports and
  Apple's own developer forums show `<input accept="image/jpeg,image/png">`
  yielding JPEG from a Photo Library HEIC pick. The heuristic is undocumented.
- **It drifts between versions.** [WebKit bug 303803](https://bugs.webkit.org/show_bug.cgi?id=303803)
  is titled "PNG converted to HEIC on file upload seemingly not accepted by
  `image/*` wildcard" — the OS converting *into* a proprietary format and then
  failing its own filter. `File.type` is advisory and must never be trusted;
  the server's byte sniffing (`infer`) is the only real check.

## What other products do

- **Normalize at ingest.** Laravel's image validation now accepts HEIC/AVIF
  uploads and serves WebP/AVIF derivatives; the `heic-normalize` library
  normalizes "at the input boundary" with a native Safari path and a lazy WASM
  fallback. The pattern is universal: accept broadly, convert once, serve
  standard.
- **Image services convert on ingest.** Cloudinary, Imgix and Cloudflare Images
  all treat HEIC as an input format and emit JPEG/WebP/AVIF.
- **Server-side conversion is libvips or ImageMagick** with `libheif` in
  practice — not a hand-rolled decoder.

The consensus matches the browser table: the input side is permissive, the
output side is standard, and the conversion point is the server.

## The decision, split into three

It is not one question. Each can be answered separately:

1. **Do we accept HEIC bytes?** (`infer` today says yes: `image/heif`.)
2. **Do we store them as-is?** Currently yes. This is the only clearly wrong
   answer, because of the Safari-only rendering.
3. **Do we normalize?** If yes, on the client, on the request path, or in a
   background task?

| Option | Accept | Store | Where normalize | Verdict |
| --- | --- | --- | --- | --- |
| **A. Defer to client** (today) | `image/*` | as sent | nowhere | Simple; payload size is an undocumented client choice, currently 5× |
| **B. Blocklist + steer** | anything except named proprietary formats | as sent | client-side `accept`, later canvas | **Chosen.** Predictable enough; still trusts the OS to convert |
| **C. Accept HEIC + server normalize** | any image | standard derivative only | server (`insert_capture` or a task) | Deterministic and universal; new decoder dependency + CPU |
| **D. Reject HEIC explicitly** | whitelist | standard only | none | Honest failure, but a user with a HEIC file simply cannot upload |

## Decision (2026-10-29): blocklist, not whitelist

A whitelist was considered and rejected. Its only real advantage over a
blocklist is failing closed on *unknown* formats — and that can be had directly,
without enumerating the set of formats that render in every browser, which is
large and grows (AVIF today, something else tomorrow).

Shipped together:

1. **A blocklist at the single policy point** (`supported_image` in
   `src/api/user/insert_capture.rs`, shared by the web upload and the REST
   import). HEIC/HEIF — which `infer` reports as `image/heif` — is refused with
   415, and the refusal explains itself in the log:
   `Refused 1153433 bytes of image/heif: no browser outside Safari renders it`.
   Surfacing that reason *to the user* is USER_FACING_LOGGING (see
   `plan/backlog.md`), not this change.
2. **Unidentified bytes are 415, not 500.** Previously an unrecognised format
   fell through `From<anyhow::Error>` to a 500, blaming the server for a file it
   simply did not support. This was the whitelist's one genuine benefit, captured
   without the whitelist.
3. **`accept="image/jpeg,image/png"`**, steering iOS toward JPEG for HEIC photos
   while leaving PNG pickable for screenshots (the app's primary input).
4. **A countable refusal.** `ApiError` logs `status_code=415` with the reason in
   the message, so HEIC refusals are one query away
   (`jsonPayload.status_code=415`). That count is the demand signal for option C —
   the refusal is not the end state, it is the instrumentation that earns the
   transcode.

The invariant that does not bend: **never store or serve a proprietary format.**
Whatever comes in, the bytes at rest are standard.

Option C becomes the right answer the moment either trigger fires:

- a HEIC (or any non-whitelisted) payload is observed reaching the server, or
- a client other than the browser bulk-uploads originals — `POST
  /api/captures/import`, the CLI, or a future share-link ingestion path, all of
  which bypass the browser's conversion entirely.

## If and when HEIC is accepted

Viable decoders, all without shelling out to ImageMagick:

| Crate | Approach | Note |
| --- | --- | --- |
| [`libheif-rs`](https://github.com/Cykooz/libheif-rs) | FFI to `libheif` | Battle-tested; needs the C library in the image |
| [`heic`](https://github.com/imazen/heic) | pure Rust | No C toolchain; smaller supply-chain surface |
| [`heic-rs`](https://github.com/tbraun96/heic-rs) | pure Rust, `no_std` | Young (0.1.x); verify before depending on it |

Two design constraints that do not change with the crate:

- **Decode off the request path.** A 12 MiB PNG decode is ~48 MB of RGBA. Put
  conversion in a Cloud Task — the infrastructure already exists (`illuminate`,
  `spark`, `search-index`). The upload stores the original, the task writes the
  derivative and updates the media row.
- **Hash the original, store the derivative.** `hash_blake3` in
  `insert_capture` must stay on the incoming bytes or duplicate detection breaks.

Licensing is a real consideration, not a footnote: HEVC patent licensing is the
reason browsers do not ship HEIC decode, and a server-side decoder inherits that
question. Worth a legal glance before it becomes a dependency.

## What this simplifies once decided

- **The cap** stops being a tuned guess: with JPEG delivery, 12 MiB covers ~8–12
  phone photos instead of ~2.
- **Client code** stays small — an `accept` attribute and, later, an optional
  canvas path — because normalization is not the client's job.
- **`upload_attempts`** records `declared_bytes` *and* `image_bytes`; their ratio
  is the expansion factor, which is the input to sizing the cap rather than
  guessing at it.
- **Media serving** has one rule: standard formats out, so caching and `<img>`
  work everywhere.

## MEDIA_TRANSCODE (deferred)

The bucket for everything that changes the bytes rather than the policy. Tracked
here, not started:

- **Client-side canvas re-encode to JPEG.** ~600 KB per photo instead of a
  5–7 MB PNG, which removes cap pressure rather than tuning it. Deferred
  2026-10-29; costs fidelity on text-heavy screenshots.
- **Server-side HEIC/HEIF decode** (option C above), triggered by
  `status_code=415` counts. Design constraints are in "If and when HEIC is
  accepted".
- **Cap sizing from measurement**, not guessing: `upload_attempts` records
  `declared_bytes` and `image_bytes`, and their ratio is the expansion factor.
- **Per-iOS-release measurement.** The ~5× conversion factor is an observation
  from one OS version; re-check when iOS majors move.
- **HEVC licensing check** before any decoder becomes a dependency.

## Open questions

1. Does iOS actually convert HEIC → JPEG when `accept="image/jpeg"` alone is set,
   on both Safari and Chrome iOS (which is WebKit but has its own picker)?
2. Are screenshots affected? They are natively PNG; forcing JPEG costs text
   fidelity, and text is what illumination reads.
3. Is the ~5× expansion stable, or iOS-version dependent? One measurement per
   iOS release is cheaper than trusting it.
4. Do we keep originals alongside derivatives (fidelity, storage cost), or
   discard them?
5. ~~Does the REST/CLI import path need its own limit?~~ Resolved in FIX_TONIGHT:
   the 5 MiB hard-code is gone, `RestState.max_upload_bytes` is the same value the
   body limit enforces, and an oversize import now returns 413 instead of "No
   image data found." (the old `while let Ok(Some(..))` loop swallowed the
   multipart error).
6. If HEIC refusals start appearing (`jsonPayload.status_code=415`), is the demand
   real (a user blocked) or theoretical (a one-off)? That count is the trigger
   for option C.

## References

- [Can I use: HEIF/HEIC](https://caniuse.com/heif) — Safari 17+ only; no Chrome,
  Edge, Firefox, or Android support.
- [WebKit bug 303803](https://bugs.webkit.org/show_bug.cgi?id=303803) — OS-side
  format conversion interacting badly with `accept` filtering.
- [Hacker News thread](https://news.ycombinator.com/item?id=23261216) —
  `accept="image/jpeg,image/png"` producing JPEG from Photo Library HEICs.
- [libheif-rs](https://github.com/Cykooz/libheif-rs), [imazen/heic](https://github.com/imazen/heic),
  [heic-rs](https://github.com/tbraun96/heic-rs) — decoder options.
- [heic-normalize](https://github.com/Araluma/heic-normalize) — input-boundary
  normalization with a WASM fallback.
