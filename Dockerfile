# syntax=docker/dockerfile:1.7

FROM rust:bookworm AS builder

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        libssl-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs rust-toolchain.toml ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --locked --bin magnarr \
    && install -D /app/target/release/magnarr /tmp/magnarr \
    && strip /tmp/magnarr

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 1000 magnarr \
    && useradd --system --uid 1000 --gid magnarr --home-dir /app --shell /usr/sbin/nologin magnarr \
    && install --directory --owner magnarr --group magnarr \
        /app \
        /app/data \
        /app/downloads

WORKDIR /app

COPY --from=builder /tmp/magnarr /usr/local/bin/magnarr

ENV MAGNARR_SERVER_LISTEN_ADDR=0.0.0.0:9393

EXPOSE 9393

USER magnarr:magnarr

ENTRYPOINT ["/usr/local/bin/magnarr"]
CMD ["start"]
