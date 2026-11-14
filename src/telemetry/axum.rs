use axum::http;
use tower_http::trace::TraceLayer;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Seed each request span from Cloud Run's W3C `traceparent` header, so our
/// app spans become *children* of the infra-level request trace.
///
/// Not automatic: `tower_http::trace::TraceLayer` only builds `tracing` spans
/// and knows nothing about OpenTelemetry; and `opentelemetry` can't ship an
/// `http::HeaderMap` impl because it deliberately doesn't depend on `http`.
/// Hence the small `AxumHeaderExtractor` adapter below.
pub fn add_axum_trace_propagation(router: axum::Router) -> axum::Router {
    router.layer(
        TraceLayer::new_for_http().make_span_with(|request: &http::Request<_>| {
            // Extract the W3C traceparent header injected by Cloud Run so that
            // our spans are children of the infrastructure-level request traces
            let parent_cx = opentelemetry::global::get_text_map_propagator(|prop| {
                prop.extract(&AxumHeaderExtractor(request.headers()))
            });
            let span = tracing::info_span!(
                "http_request",
                method = %request.method(),
                path = %request.uri().path()
            );

            let _ = span.set_parent(parent_cx);
            span
        }),
    )
}

/// Adapts `http::HeaderMap` (from Axum) into OTel's transport-agnostic
/// `Extractor` trait, which abstracts over any carrier (HTTP, gRPC, queues).
struct AxumHeaderExtractor<'a>(&'a http::HeaderMap);

impl opentelemetry::propagation::Extractor for AxumHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::AxumHeaderExtractor;
    use axum::http::HeaderValue;
    use opentelemetry::propagation::Extractor;

    #[test]
    fn get_returns_none_for_non_utf8_header_instead_of_panicking() {
        // A header with invalid UTF-8 bytes must not panic; propagation is
        // simply skipped (returns None) so a malformed header can't crash a request.
        let mut map = axum::http::HeaderMap::new();
        map.insert(
            "traceparent",
            HeaderValue::from_bytes(&[0xFF, 0xFE]).unwrap(),
        );
        let extractor = AxumHeaderExtractor(&map);
        assert_eq!(extractor.get("traceparent"), None);
    }
}
