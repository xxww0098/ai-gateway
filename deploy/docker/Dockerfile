# --- build stage -------------------------------------------------------------
# Pinned to rust-toolchain.toml (1.97.1). The binary links rustls rather than
# OpenSSL and uses the pure-Rust sqlx Postgres driver, so it needs no system
# libraries beyond libc at runtime.
FROM rust:1.97 AS build
WORKDIR /src

# Cache the dependency build separately from our own sources: manifests and the
# lockfile first, so a source-only change does not rebuild the whole graph.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY tools ./tools
RUN cargo build --release --locked

COPY . .
RUN cargo build --release --locked && cp target/release/gw-server /out-ai-gateway

# --- runtime stage -----------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
COPY --from=build /out-ai-gateway /usr/local/bin/ai-gateway
# Ship the example config as a reference; the real config is mounted or supplied
# via env vars (see crates/gw-config). Secrets MUST come from the environment,
# never baked into the image.
COPY config.example.yaml /app/config.example.yaml

EXPOSE 8888
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/ai-gateway"]
CMD ["--config", "/app/config.yaml"]
