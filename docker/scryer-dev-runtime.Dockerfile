# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e
FROM rust:1.97.1-slim-bookworm@sha256:96c0af8cf054fd006435089f0076729716784ec9be485bd655de59c55df105ce

WORKDIR /workspace

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
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
