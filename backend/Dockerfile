# syntax=docker/dockerfile:1.7
# ---------------------------------------------------------------- build
FROM rust:1.98-bookworm AS build
WORKDIR /src

# Cache dependencies: build once with stub sources, then with the real code.
COPY Cargo.toml Cargo.lock ./
COPY crates/engine/Cargo.toml crates/engine/Cargo.toml
COPY crates/api/Cargo.toml crates/api/Cargo.toml
RUN mkdir -p crates/engine/src/bin crates/api/src \
 && echo "pub const BUNDLED_POLICY_JSONL: &str = \"\";" > crates/engine/src/lib.rs \
 && echo "fn main() {}" > crates/engine/src/bin/eval.rs \
 && echo "fn main() {}" > crates/api/src/main.rs \
 && touch crates/api/src/lib.rs \
 && cargo build --release --locked -p claim-api || true

COPY data ./data
COPY crates ./crates
RUN touch crates/engine/src/lib.rs crates/api/src/lib.rs crates/api/src/main.rs \
 && cargo build --release --locked -p claim-api -p claim-engine \
 && mkdir -p /out/data \
 && cp target/release/claim-api target/release/claim-eval /out/

# ---------------------------------------------------------------- runtime
# Distroless: no shell, no package manager; runs as uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
COPY --from=build --chown=65532:65532 /out/claim-api /out/claim-eval /app/
COPY --from=build --chown=65532:65532 /out/data /data
ENV BIND_ADDR=0.0.0.0:8000 \
    DATABASE_URL=sqlite:///data/claims.db?mode=rwc \
    RUST_LOG=info
EXPOSE 8000
USER 65532:65532
HEALTHCHECK --interval=15s --timeout=4s --start-period=10s --retries=3 CMD ["/app/claim-api", "healthcheck"]
ENTRYPOINT ["/app/claim-api"]
CMD ["serve"]
