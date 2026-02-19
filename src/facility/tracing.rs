use tracing_subscriber::{
    EnvFilter,
    fmt::{format::FmtSpan, time::ChronoLocal},
};

pub fn init_tracing() {
    let timer = ChronoLocal::new("%H:%M:%S%.3f".to_string());

    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(EnvFilter::from_default_env())
        //.with_span_events(FmtSpan::NEW)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(true)
        .with_timer(timer)
        .init();
}
