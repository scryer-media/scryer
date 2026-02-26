# syntax=docker/dockerfile:1.7
FROM rust:1.89 AS rust-base

WORKDIR /workspace

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      pkg-config \
      libssl-dev \
      libsqlite3-dev \
 && rm -rf /var/lib/apt/lists/* \
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
