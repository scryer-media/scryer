# syntax=docker/dockerfile:1.7
FROM rust:1.94-slim-bookworm

WORKDIR /workspace

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      gdb \
      gawk \
      pkg-config \
      procps \
      libssl-dev \
      libsqlite3-dev \
      mold \
 && rm -rf /var/lib/apt/lists/*

# Use mold for faster links during iterative local builds.
ENV RUSTFLAGS="-C link-arg=-fuse-ld=mold"
