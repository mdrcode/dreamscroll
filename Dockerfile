# Builder stage
FROM rust:slim AS builder

WORKDIR /app

# reqwest and the Google client dependencies currently include native-tls on
# Linux, so openssl-sys needs the OpenSSL development files while compiling.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Step 1: Copy manifests and create a dummy src to cache deps. Note, we must
# create our dummy at src/bin/dreamscroll_web.rs because that's referenced in
# Cargo.toml as the default-run binary and cargo will fail if it doesn't
# exist. The real source build below explicitly builds each binary shipped in
# the runtime image.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src/
RUN mkdir src/bin
RUN touch src/lib.rs
RUN echo 'fn main() { println!("dummy build for dependency caching"); }' > src/bin/dreamscroll_web.rs
RUN cargo build --release
RUN rm src/lib.rs src/bin/dreamscroll_web.rs  # clean up dummy

# Step 2: Copy real source and rebuild (reuses dep cache)
COPY src ./src/
RUN touch src/bin/dreamscroll_web.rs  # ensure timestamp is updated for cargo to detect changes
RUN touch src/lib.rs
RUN cargo build --release --bin dreamscroll_web --bin dreamscroll_api --bin dreamscroll_admin

# Runtime stage
FROM debian:trixie-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

COPY --from=builder /app/target/release/dreamscroll_web /app/dreamscroll_web
COPY --from=builder /app/target/release/dreamscroll_api /app/dreamscroll_api
COPY --from=builder /app/target/release/dreamscroll_admin /app/dreamscroll_admin
COPY web/v1 /app/web/v1
COPY web/v2 /app/web/v2

# Create non-root user
RUN groupadd --system --gid 1001 appgroup \
    && useradd --system --uid 1001 --gid 1001 --no-create-home --shell /usr/sbin/nologin appuser

USER appuser

CMD ["/app/dreamscroll_web"]