use axum::http;
use tower_http::trace::TraceLayer;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub fn add_axum_trace_propagation(router: axum::Router) -> axum::Router {
    router.layer(
        TraceLayer::new_for_http().make_span_with(|request: &http::Request<_>| {
            // Extract the W3C traceparent header injected by Cloud Run so that
            // our spans are children of the infrastructure-level request traces
            let parent_cx = opentelemetry::global::get_text_map_propagator(|prop| {
                prop.extract(&AxumHeaderExtractor(request.headers()))
            });
            let span = tracing::info_span!("http_request");

            let _ = span.set_parent(parent_cx);
            span
        }),
    )
}

struct AxumHeaderExtractor<'a>(&'a http::HeaderMap);

impl opentelemetry::propagation::Extractor for AxumHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}
