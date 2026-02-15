# Builder stage
FROM rust:slim AS builder

WORKDIR /app

# Step 1: Copy manifests and create dummy src to cache deps
COPY Cargo.toml Cargo.lock ./
RUN mkdir src/
RUN echo 'fn main() { println!("dummy build for dependency caching"); }' > src/main.rs  # (print to avoid silent failure)
RUN cargo build --release
RUN rm src/main.rs  # Clean up dummy

# Step 2: Copy real source and rebuild (reuses dep cache)
COPY src ./src/
# If you have other dirs (e.g., tests, benches), COPY them here too
RUN touch src/main.rs  # Optional: Force Cargo to detect changes if needed
RUN cargo build --release --bin your-app-name  # Replace with your binary name

# Runtime stage (unchanged)
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/your-app-name /usr/local/bin/your-app-name
EXPOSE $PORT
CMD ["your-app-name"]