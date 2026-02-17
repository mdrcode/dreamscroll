# Builder stage
FROM rust:slim AS builder

WORKDIR /app

# Step 1: Copy manifests and create a dummy src to cache deps. Note, we must
# create our dummy at src/bin/dreamscroll_localdev.rs because that's referenced
# in Cargo.toml as the default-run binary and cargo will fail if it doesn't
# exist. But below, when actually building the real source, we only build the 
# dreamscroll_cloudrun binary.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src/
RUN mkdir src/bin
RUN touch src/lib.rs
RUN echo 'fn main() { println!("dummy build for dependency caching"); }' > src/bin/dreamscroll_localdev.rs
RUN cargo build --release
RUN rm src/lib.rs src/bin/dreamscroll_localdev.rs  # clean up dummy

# Step 2: Copy real source and rebuild (reuses dep cache)
COPY src ./src/
RUN touch src/bin/dreamscroll_cloudrun.rs  # ensure timestamp is updated for cargo to detect changes
RUN touch src/lib.rs
RUN cargo build --release --bin dreamscroll_cloudrun

# Runtime stage
FROM debian:trixie-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

COPY --from=builder /app/target/release/dreamscroll_cloudrun /app/dreamscroll_cloudrun
COPY web/v1 /app/web/v1

# Create non-root user
RUN groupadd --system --gid 1001 appgroup \
    && useradd --system --uid 1001 --gid 1001 --no-create-home --shell /usr/sbin/nologin appuser

USER appuser

CMD ["/app/dreamscroll_cloudrun"]