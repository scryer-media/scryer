# syntax=docker/dockerfile:1
FROM rust:1.89 AS build

WORKDIR /workspace

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      pkg-config \
      libssl-dev \
      libsqlite3-dev \
 && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/workspace/target \
    cargo build --locked -p scryer

FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

COPY --from=build /workspace/target/debug/scryer /usr/local/bin/scryer

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/scryer"]
