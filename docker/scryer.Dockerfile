FROM alpine:latest

ARG TARGETARCH

RUN apk add --no-cache su-exec tzdata

WORKDIR /app

COPY ${TARGETARCH}/scryer /usr/local/bin/scryer
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 8080

# /config holds app state: database, WASM cache, logs.
# /data is conventionally where users mount their media library.
RUN mkdir -p /config /data
VOLUME /config

ENV PUID=1000
ENV PGID=1000
ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/config/scryer.db

# Graceful shutdown: let in-flight requests and background tasks finish
STOPSIGNAL SIGTERM

ENTRYPOINT ["/entrypoint.sh"]
