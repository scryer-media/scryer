# syntax=docker/dockerfile:1.7
FROM rust:alpine AS rust-base

WORKDIR /workspace

RUN apk add --no-cache \
      ca-certificates \
      pkgconf \
      openssl-dev \
      sqlite-dev \
      musl-dev \
      mold \
 && cargo install --locked cargo-chef

# Use mold for faster linking in all stages (chef cook and cargo run).
# -fuse-ld=mold tells gcc (Alpine's default cc) to use mold as the linker.
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"

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
