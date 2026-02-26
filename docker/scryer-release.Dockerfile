# syntax=docker/dockerfile:1
FROM rust:1.93 AS build

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
    cargo build --locked -p scryer --release \
 && cp /workspace/target/release/scryer /tmp/scryer

FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

COPY --from=build /tmp/scryer /usr/local/bin/scryer

EXPOSE 8080

VOLUME /data

ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/data/scryer.db

STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/local/bin/scryer"]
