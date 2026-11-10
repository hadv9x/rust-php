# syntax=docker/dockerfile:1
FROM rust:1.93.1-bookworm AS builder

WORKDIR /src
ARG PHP_RUST_VERSION

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN test "$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)" = "$PHP_RUST_VERSION" \
    && cargo build --locked --release -p php-cli

FROM debian:bookworm-slim AS runtime

ARG PHP_RUST_VERSION
ARG PHP_RUST_UID=1000
ARG PHP_RUST_GID=1000
LABEL org.opencontainers.image.title="php-rust" \
      org.opencontainers.image.version="$PHP_RUST_VERSION" \
      org.opencontainers.image.description="PHP subset runtime and async HTTP server written in Rust"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libmariadb3 \
        libmongoc-1.0-0 \
        libpq5 \
        unixodbc \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid "$PHP_RUST_GID" php-rust \
    && useradd --uid "$PHP_RUST_UID" --gid "$PHP_RUST_GID" --create-home --home-dir /home/php-rust --shell /usr/sbin/nologin php-rust

COPY --from=builder /src/target/release/php-rust /usr/local/bin/php-rust
COPY --from=builder /src/target/release/php-rust-server /usr/local/bin/php-rust-server

WORKDIR /app
USER ${PHP_RUST_UID}:${PHP_RUST_GID}
EXPOSE 3000
ENTRYPOINT ["php-rust-server"]
CMD ["--root", "/app/public", "--bind", "0.0.0.0:3000"]
