use axum::{
    body::Body,
    http::{HeaderValue, Request, header},
    middleware::Next,
    response::Response,
};

// We "version" asset links by applying a ?v= query parameter.
// Such versioned assets are safe to keep for a year because a new deployment
// changes the `?v=` value in generated HTML. Unversioned references get a
// shorter TTL. The service worker is handled separately with `no-cache` because
// its URL must remain stable while browsers check for updates.
pub(crate) const VERSIONED_ASSET_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
pub(crate) const UNVERSIONED_ASSET_CACHE_CONTROL: &str = "public, max-age=3600";

/// Choose the cache lifetime for a `/static/*` request.
///
/// The query string is already part of the browser/cache key. We only inspect
/// it here to distinguish deployment-versioned URLs from unversioned asset
/// references; this is deliberately not a general query-string parser.
fn asset_cache_control(query: Option<&str>) -> &'static str {
    let is_versioned = query.is_some_and(|query| {
        query.split('&').any(|part| {
            let mut pieces = part.splitn(2, '=');
            pieces.next() == Some("v") && pieces.next().is_some_and(|value| !value.is_empty())
        })
    });

    if is_versioned {
        VERSIONED_ASSET_CACHE_CONTROL
    } else {
        UNVERSIONED_ASSET_CACHE_CONTROL
    }
}

/// Apply the static asset policy to responses served from `/static`. Nothing
/// dynamic should pass through this.
pub(crate) async fn static_asset_cache_headers(request: Request<Body>, next: Next) -> Response {
    let cache_control = asset_cache_control(request.uri().query());

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}

/// The manifest URL includes `?v=<revision>`, so it can use the same immutable
/// policy as versioned CSS, JavaScript, and image URLs.
pub(crate) async fn manifest_cache_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(VERSIONED_ASSET_CACHE_CONTROL),
    );
    response
}

/// Keep the service-worker URL stable and revalidate it on each browser check.
///
/// Unlike ordinary static assets, a long immutable TTL here could prevent the
/// browser from discovering a new service worker after deployment.
pub(crate) async fn service_worker_cache_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

#[cfg(test)]
mod tests {
    use super::{
        UNVERSIONED_ASSET_CACHE_CONTROL, VERSIONED_ASSET_CACHE_CONTROL, asset_cache_control,
    };

    #[test]
    fn unversioned_assets_use_short_cache() {
        assert_eq!(asset_cache_control(None), UNVERSIONED_ASSET_CACHE_CONTROL);
        assert_eq!(
            asset_cache_control(Some("view=masonry-v2")),
            UNVERSIONED_ASSET_CACHE_CONTROL
        );
        assert_eq!(
            asset_cache_control(Some("v=")),
            UNVERSIONED_ASSET_CACHE_CONTROL
        );
    }

    #[test]
    fn versioned_assets_use_immutable_cache() {
        assert_eq!(
            asset_cache_control(Some("v=revision-123")),
            VERSIONED_ASSET_CACHE_CONTROL
        );
        assert_eq!(
            asset_cache_control(Some("v=revision-123&view=masonry-v2")),
            VERSIONED_ASSET_CACHE_CONTROL
        );
        assert_eq!(
            asset_cache_control(Some("view=masonry-v2&v=revision-123")),
            VERSIONED_ASSET_CACHE_CONTROL
        );
    }
}
