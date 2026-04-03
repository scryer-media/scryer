# syntax=docker/dockerfile:1.7

ARG NODE_VERSION=22
ARG RUST_VERSION=1.94

FROM node:${NODE_VERSION}-bookworm-slim AS web

WORKDIR /workspace/apps/scryer-web

COPY apps/scryer-web/package.json apps/scryer-web/package-lock.json apps/scryer-web/.npmrc ./
RUN npm ci

COPY apps/scryer-web ./

ENV SCRYER_GRAPHQL_URL=/graphql
ARG SCRYER_SMG_GRAPHQL_URL=https://smg.scryer.media/graphql
ENV SCRYER_METADATA_GATEWAY_GRAPHQL_URL=${SCRYER_SMG_GRAPHQL_URL}

RUN npm run lint
RUN npm run build
RUN test -f dist/index.html

FROM rust:${RUST_VERSION}-bookworm AS builder

ARG SCRYER_SMG_GRAPHQL_URL=https://smg.scryer.media/graphql
ARG SCRYER_SMG_REGISTRATION_SECRET=

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      libsqlite3-dev \
      libssl-dev \
      nasm \
      pkg-config \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

COPY . .
COPY --from=web /workspace/apps/scryer-web/dist /tmp/scryer-web-dist

ENV SCRYER_EMBED_UI_DIR=/tmp/scryer-web-dist
ENV SCRYER_SMG_GRAPHQL_URL=${SCRYER_SMG_GRAPHQL_URL}
ENV SCRYER_SMG_REGISTRATION_SECRET=${SCRYER_SMG_REGISTRATION_SECRET}

RUN cargo build -p scryer --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      libsqlite3-0 \
      libssl3 \
 && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --home-dir /home/scryer --uid 1000 scryer

COPY --from=builder /workspace/target/release/scryer /usr/local/bin/scryer
COPY docker/scryer-e2e-entrypoint.sh /usr/local/bin/scryer-e2e-entrypoint

RUN mkdir -p /data /weaver-downloads /nzbget-downloads /sabnzbd-downloads /scryer-data \
 && chown -R scryer:scryer /data /weaver-downloads /nzbget-downloads /sabnzbd-downloads /scryer-data /home/scryer \
 && chmod +x /usr/local/bin/scryer-e2e-entrypoint

WORKDIR /home/scryer

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/scryer-e2e-entrypoint"]
