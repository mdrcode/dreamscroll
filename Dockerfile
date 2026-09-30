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

# Step 1: Copy manifests and create a dummy src to cache deps.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin \
    && touch src/lib.rs \
    && echo 'fn main() { println!("dummy build for dependency caching"); }' > src/bin/dreamscroll_web.rs \
    && cargo build --release \
    && rm -rf src/lib.rs src/bin/dreamscroll_web.rs /app/target/release/deps/libdreamscroll* /app/target/release/deps/dreamscroll*

# Step 2: Copy real source and rebuild (reuses dep cache)
COPY src ./src/
RUN touch src/lib.rs src/bin/dreamscroll_web.rs \
    && cargo build --release --bin dreamscroll_web

# Runtime stage
FROM debian:trixie-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

COPY --from=builder /app/target/release/dreamscroll_web /app/dreamscroll_web
COPY web/v2 /app/web/v2

# Create non-root user
RUN groupadd --system --gid 1001 appgroup \
    && useradd --system --uid 1001 --gid 1001 --no-create-home --shell /usr/sbin/nologin appuser

USER appuser

CMD ["/app/dreamscroll_web"]