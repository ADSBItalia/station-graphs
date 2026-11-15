# Multi-stage build for multi-architecture Docker images (amd64, arm64, arm/v7)
FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends librrd-dev pkg-config && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# Final lightweight runtime image
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends rrdtool ca-certificates tzdata curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/adsbitalia-station-graphs /app/adsbitalia-station-graphs
COPY web /app/web
ENV PORT=8085
EXPOSE 8085
CMD ["/app/adsbitalia-station-graphs"]
