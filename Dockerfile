FROM rust:1.85-bookworm AS builder

WORKDIR /build
COPY . .
RUN cargo build --release --bin ironclad

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ironclad /usr/local/bin/ironclad

RUN mkdir -p /data/ironclad

ENV IRONCLAD_URL=http://0.0.0.0:18789

EXPOSE 18789

VOLUME ["/data/ironclad"]

ENTRYPOINT ["ironclad"]
CMD ["serve", "--bind", "0.0.0.0", "--port", "18789"]
