# syntax=docker/dockerfile:1.7

FROM rust:bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs rust-toolchain.toml ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release --locked --bin magnarr \
    && cp /app/target/release/magnarr /tmp/magnarr \
    && strip /tmp/magnarr

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

WORKDIR /app

COPY --from=builder /tmp/magnarr /usr/local/bin/magnarr

ENV MAGNARR_SERVER_LISTEN_ADDR=0.0.0.0:9393

EXPOSE 9393

CMD ["/usr/local/bin/magnarr", "start"]
