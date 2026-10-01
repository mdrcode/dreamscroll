# Programmatic Screenshot API Landscape & Architecture Guide

## Overview

We want a user to submit a link and see a useful visual preview with as little waiting as practical. **Availability and end-to-end latency are primary selection criteria.** Image fidelity and cost matter, but should not undermine a responsive, dependable experience. Our initial shortlist is **Urlbox and ScreenshotOne**, with other providers considered only if these fail our latency, availability, metadata, or image-quality needs. Published vendor and third-party evidence helps prioritize, but does not replace a quick comparison on our own representative links.

## 1. Provider candidates and available speed evidence

Start with Urlbox and ScreenshotOne as the two candidates we want to evaluate. Both are dedicated screenshot APIs and can return Open Graph metadata alongside capture. Compare them on a handful of representative links, then choose the simpler option that meets our latency and reliability needs. Add another provider only if either candidate has a material issue. Do not expose provider credentials in the browser; requests and image storage should be mediated by our backend.

| Candidate | Open Graph / page metadata | Reported speed / availability evidence | Relevance and caveats |
| --- | --- | --- | --- |
| **Microlink** | Yes. Normalized metadata by default; custom extraction can target literal OG tags. Screenshot works on the same API. [Metadata](https://microlink.io/docs/api/parameters/meta), [custom extraction](https://microlink.io/docs/api/parameters/data), [screenshots](https://microlink.io/docs/api/parameters/screenshot) | Its own [March 2026 benchmark](https://microlink.io/benchmarks/screenshot-api) reports 4.112 s average, versus Urlbox 7.334 s, ScreenshotOne 7.711 s, and ApiFlash 9.463 s. Paid plans advertise a 99.9% uptime SLA; service credits only apply to Enterprise ([API overview](https://microlink.io/api)). | Not in the initial shortlist; reconsider only if Urlbox and ScreenshotOne miss our needs. Speed result is vendor-authored and measures response-header time, averages seven URLs, and does not establish cold browser starts or p95. |
| **Urlbox** | Yes. `save_metadata` extracts title, description, canonical URL, Open Graph, Twitter cards, and other tags during the screenshot page load. [Metadata docs](https://urlbox.com/docs/guides/side-renders#page-metadata) | Microlink’s benchmark reports 7.334 s average. Urlbox advertises 99.99% uptime SLA and ~3-second average render on its [official site](https://urlbox.com/). A [third-party developer project](https://github.com/avinoamMO/ScreenshotRace) lists rough 4–8 s timings, but adds a fixed 3 s delay and publishes no dataset. | Initial shortlist. Combined metadata and published SLA are useful; speed reports are not comparable p95 or per-target guarantees. |
| **ScreenshotOne** | Yes. `metadata_open_graph=true` returns title, description, and image alongside capture. [Options docs](https://screenshotone.com/docs/options/#metadata_open_graph) | A March 2026 developer comparison lists 2–4 s, but no targets, sample counts, region, raw data, or test code are provided. The author is linked to a competing screenshot product. [Comparison](https://dev.to/dennis-ddev/screenshot-api-comparison-2026-snaprender-vs-screenshotone-vs-urlbox-vs-scrapingbee-vs-capturekit-3egh) No numerical uptime SLA verified in reviewed docs. | Initial shortlist. Its speed claim is anecdotal and not a controlled, independent result. |
| **ApiFlash** | No parsed OG bundle verified. `extract_html` can return HTML alongside capture for us to parse. [Parameters](https://apiflash.com/documentation#parameters) | Microlink’s benchmark reports 9.463 s average. No numerical uptime SLA or comparable latency guarantee verified. | Not an initial test priority unless the first comparison gives a reason to expand. |
| **Firecrawl** | Yes. Scrape results include title, description, and OG fields, with screenshot among supported output formats. [Scrape docs](https://docs.firecrawl.dev/features/scrape) | No comparable latency benchmark found; advertises an SLA on custom plans without a numerical target in reviewed [pricing](https://www.firecrawl.dev/pricing). A [provider-authored comparison](https://www.firecrawl.dev/blog/best-website-screenshot-apis) does not time competitors under matching conditions. | Broader scraping product; test only if combined metadata/capture is valuable enough to justify it. Screenshot URLs expire, so copy output into our storage. |
| **Thum.io** | No documented structured page-metadata extraction found. [URL API](https://www.thum.io/documentation/api/url) | No comparable benchmark found. Its [product site](https://www.thum.io/) describes progressive image rendering, not a completion-time or uptime guarantee. | Lower priority for backend-controlled storage/status and structured metadata. |

**Evidence quality and scope:** The [Microlink benchmark source/data](https://github.com/microlinkhq/screenshot-benchmark) tests seven URLs and reports averages; it excludes the slowest run and non-200 errors, and stops timing at response headers rather than image download. It is useful directional evidence, not independent validation. A [third-party screenshot API guide](https://www.browserless.io/blog/best-screenshot-api) provides no cross-provider latency ranking and reports its own test captures only for Browserless and ScreenshotOne. We found no robust independent apples-to-apples winner; search coverage was incomplete. “Not verified” means no comparable claim was found in reviewed sources, not proof none exists. Verify current feature and plan availability before selection.

Browserless, Browserbase, and Scrapfly can provide configurable scraping or extraction capabilities, but require a separate capture/extraction operation or more integration work; keep them out of the initial shortlist unless the simple APIs fail requirements. [Browserless scrape](https://docs.browserless.io/rest-apis/scrape), [Browserbase Fetch](https://docs.browserbase.com/platform/fetch/overview), [Scrapfly extraction](https://scrapfly.io/docs/scrape-api/extraction).

**Evidence note:** Provider documentation was reviewed on 2026-10-01. “Not verified” means no numerical claim was found in the reviewed official material, not proof that none exists. Metadata may be normalized rather than literal OG values; missing or inaccurate tags are common. Confirm feature and plan availability before selection.

## 2. Latency and availability evaluation

Use the published evidence to move quickly, not to pretend we already know the winner. Compare Urlbox and ScreenshotOne on a small, fixed set of representative links, including screenshot plus metadata. Do not build a large benchmark harness. If one clearly meets our needs, ship with it and measure actual outcomes before adding more complexity.

Prioritize user-visible latency:

`link submitted → task accepted/queued → provider response → image stored → UI updated`

For the first comparison, record total elapsed time, whether the image and metadata succeeded, and obvious failures. If the candidate choice remains ambiguous or we see tail-latency problems, then collect p50/p95, queue/provider/storage breakdown, timeouts, rate limits, and repeat/cold-cache results at realistic concurrency. The benchmark must use the same representative public URLs and report its conditions; a provider's average render time is not the complete user journey or a p95 guarantee.

Availability has two useful signals: contractual SLA/status history (including exclusions, remedies, support, and rate limits) and our observed successful-capture rate. SLA claims inform a quick shortlist; monitor real success/failure once integrated. An SLA does not guarantee arbitrary target sites can be captured. Avoid delaying the MVP for elaborate provider failover or a comprehensive benchmark absent evidence that the simple path fails.

## 3. Proposed user and backend flow

1. Accept `http`/`https` links on the backend. Keep provider credentials out of the browser.
2. Create a pending capture task and return promptly. Keep the page responsive with progress/status updates; users may wait on the page, but do not hold one browser HTTP request open for the full render.
3. Have a bounded background task call the provider with a strict deadline and limited capture options. Retry transient failures a small number of times; do not retry invalid URLs or permanent blocks.
4. On success, validate and store the image in existing media storage, then update status for the UI. On failure, retain the link and provide a clear retry path.
5. Start with a simple synchronous provider call inside the worker; use provider-managed async jobs only if measurements or documented limits require them.

Align status delivery and image storage with the app’s existing task-status/SSE and media-serving designs rather than creating parallel mechanisms: [task-status timing plan](./task-status-timing-progress-bar.md), [SSE plan](./sse.md), [media-serving plan](./media-serving.md).

## 4. Metadata behavior and use

Open Graph tags are optional, page-authored metadata, commonly including `og:title`, `og:description`, and `og:image`; see the [Open Graph Protocol](https://ogp.me/). They can be absent, stale, inaccurate, or point to a different image than our screenshot. Keep metadata optional and untrusted. Use the submitted URL as the canonical user-provided source, and provide a title fallback when tags are missing.

Where supported, prefer extraction from the same page load as screenshot capture to avoid a second navigation. Benchmark metadata-enabled capture because it may add latency. For providers that only return raw HTML or require a separate metadata endpoint, compare the extra request and failure modes; optionally run it concurrently only if that is safe and improves the user-visible result. Do not fetch a metadata-provided `og:image` without applying the same URL safety checks as to user-submitted links.

## 5. URL safety and failure handling

User-submitted URLs are untrusted. Enforce allowed schemes, reject embedded credentials, constrain redirects and capture duration, and prevent access to localhost, private/link-local ranges, cloud metadata endpoints, and internal services. Verify provider-side protections but retain application-side validation; do not assume a remote browser is isolated. Limit and validate returned image content type, dimensions, and size before storage. Treat extracted metadata and image URLs as untrusted input.

Normalize outcomes into pending, success, retryable failure, and permanent failure. Bound retries and total waiting time so outages do not leave captures pending indefinitely. Capture sanitized diagnostics sufficient to compare provider health and latency; never log provider secrets or unnecessary page contents.

## 6. Decision and next steps

1. Compare Urlbox and ScreenshotOne on a small representative link set, with screenshot and metadata enabled.
2. Choose the simpler candidate that meets our availability, latency, metadata, and image-quality needs. Expand to another provider only if both have a material gap.
3. Use a bounded background task, responsive waiting UI, clear failure/retry behavior, and basic success/latency monitoring. Do not build automatic failover before production evidence warrants it.
4. Revisit provider choice if observed availability, user-visible latency, metadata completeness, or cost is unsatisfactory.

