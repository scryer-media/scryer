FROM alpine:latest

ARG TARGETARCH

RUN apk add --no-cache su-exec tzdata

WORKDIR /app

COPY ${TARGETARCH}/scryer /usr/local/bin/scryer
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 8080

# Persist the SQLite database across container upgrades.
RUN mkdir -p /data
VOLUME /data

ENV PUID=1000
ENV PGID=1000
ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/data/scryer.db
ENV WASMTIME_CACHE_ENABLED=0

# Graceful shutdown: let in-flight requests and background tasks finish
STOPSIGNAL SIGTERM

ENTRYPOINT ["/entrypoint.sh"]
