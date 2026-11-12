FROM rust:1.83-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/magpie-rs /app/magpie-rs
COPY --from=builder /app/config.toml.example /app/config.toml.example
EXPOSE 8080
CMD ["/app/magpie-rs"]
