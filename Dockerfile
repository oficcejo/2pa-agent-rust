# ==========================================
# Stage 1: Build & Compile
# ==========================================
FROM rust:1.80-slim-bookworm AS builder

WORKDIR /app

# Install basic build tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies layer
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn dummy() {}" > src/lib.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code and embedded assets
COPY src ./src
COPY static ./static
COPY prompt_engineering ./prompt_engineering
COPY experience ./experience

# Touch source files to invalidate dummy build artifacts and compile real release binary
RUN touch src/main.rs src/lib.rs && \
    cargo build --release --bin okx-2pa-agent

# ==========================================
# Stage 2: Minimal Production Runtime
# ==========================================
FROM debian:bookworm-slim AS runner

# Install ca-certificates (for HTTPS / SSL to OKX and LLM) and tzdata (for timezone support)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Set default timezone to Asia/Shanghai (user can override via TZ env variable)
ENV TZ=Asia/Shanghai \
    RUST_LOG=info

# Copy compiled binary from builder
COPY --from=builder /app/target/release/okx-2pa-agent /app/okx-2pa-agent

# Ensure persistence directories exist
RUN mkdir -p /app/records /app/config /app/experience /app/prompt_engineering /app/static

EXPOSE 8088

# Default entrypoint listens on 0.0.0.0 for container networking
ENTRYPOINT ["/app/okx-2pa-agent", "--host", "0.0.0.0", "--port", "8088"]
