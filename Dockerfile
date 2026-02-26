FROM rust:1.85-bookworm AS builder

WORKDIR /build
# Cache dependencies by copying manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/ironclad-core/Cargo.toml crates/ironclad-core/Cargo.toml
COPY crates/ironclad-db/Cargo.toml crates/ironclad-db/Cargo.toml
COPY crates/ironclad-llm/Cargo.toml crates/ironclad-llm/Cargo.toml
COPY crates/ironclad-agent/Cargo.toml crates/ironclad-agent/Cargo.toml
COPY crates/ironclad-wallet/Cargo.toml crates/ironclad-wallet/Cargo.toml
COPY crates/ironclad-schedule/Cargo.toml crates/ironclad-schedule/Cargo.toml
COPY crates/ironclad-channels/Cargo.toml crates/ironclad-channels/Cargo.toml
COPY crates/ironclad-server/Cargo.toml crates/ironclad-server/Cargo.toml
COPY crates/ironclad-plugin-sdk/Cargo.toml crates/ironclad-plugin-sdk/Cargo.toml
COPY crates/ironclad-browser/Cargo.toml crates/ironclad-browser/Cargo.toml
COPY crates/ironclad-tests/Cargo.toml crates/ironclad-tests/Cargo.toml
# Create dummy lib.rs files so cargo fetch succeeds
RUN for d in crates/ironclad-*/; do mkdir -p "$d/src" && echo "" > "$d/src/lib.rs"; done && \
    mkdir -p crates/ironclad-server/src && echo "fn main() {}" > crates/ironclad-server/src/main.rs
RUN cargo fetch --locked
# Now copy the real source and build
COPY . .
RUN cargo build --release --locked --bin ironclad

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ironclad /usr/local/bin/ironclad

RUN groupadd --system ironclad && \
    useradd --system --gid ironclad --create-home ironclad && \
    mkdir -p /data/ironclad && \
    chown -R ironclad:ironclad /data/ironclad

ENV IRONCLAD_URL=http://0.0.0.0:18789

EXPOSE 18789

VOLUME ["/data/ironclad"]

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:18789/api/health || exit 1

USER ironclad

ENTRYPOINT ["ironclad"]
CMD ["serve", "--bind", "0.0.0.0", "--port", "18789"]
