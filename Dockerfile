# ---- build stage ------------------------------------------------------------
FROM rust:1-slim AS builder
WORKDIR /app
# the admin SPA (web/) is embedded at compile time via rust-embed, so it must be present
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web
RUN cargo build --release

# ---- runtime stage ----------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 cortiq \
    && mkdir -p /app/config
COPY --from=builder /app/target/release/cortiq-gateway /usr/local/bin/cortiq-gateway
# ship the example as the default config so `docker run` works out of the box
COPY config/gateway.example.toml /app/config/gateway.toml
RUN chown -R 10001:10001 /app
WORKDIR /app
USER cortiq
EXPOSE 9000
ENTRYPOINT ["cortiq-gateway"]
CMD ["--config", "/app/config/gateway.toml"]
