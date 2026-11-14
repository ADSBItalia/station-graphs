FROM rust:1.80-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM alpine:3.20
RUN apk add --no-cache rrdtool ca-certificates tzdata
WORKDIR /app
COPY --from=builder /app/target/release/adsbitalia-station-graphs /app/
COPY web ./web
ENV PORT=8085
EXPOSE 8085
CMD ["/app/adsbitalia-station-graphs"]
