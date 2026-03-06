# syntax=docker/dockerfile:1.7
FROM rust:alpine AS rust-base

WORKDIR /workspace

RUN apk add --no-cache \
      ca-certificates \
      pkgconf \
      openssl-dev \
      sqlite-dev \
      musl-dev \
 && cargo install --locked cargo-chef

FROM rust-base AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo chef prepare --recipe-path recipe.json

FROM rust-base AS deps

COPY --from=planner /workspace/recipe.json /workspace/recipe.json

RUN cargo chef cook --locked --recipe-path recipe.json

# Final image: deps-warmed cache only, app compiles at runtime via cargo run
FROM rust-base

COPY --from=deps /usr/local/cargo /usr/local/cargo
COPY --from=deps /workspace/target /workspace/target
