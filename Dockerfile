# Multi-stage build for coco-gateway
# Stage 1: cargo-chef — compute recipe for dependency caching
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Build dependencies (cached layer)
FROM chef AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libdbus-1-dev \
    && rm -rf /var/lib/apt/lists/*
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Stage 3: Build the binary
COPY . .
RUN cargo build --release -p coco-gateway && \
    strip /app/target/release/coco-gateway

# Stage 4: Minimal runtime image
FROM debian:trixie-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libdbus-1-3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/coco-gateway /usr/local/bin/coco-gateway

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/coco-gateway"]
