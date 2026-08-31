FROM alpine:3@sha256:28bd5fe8b56d1bd048e5babf5b10710ebe0bae67db86916198a6eec434943f8b

ARG TARGETARCH

RUN apk add --no-cache ca-certificates tzdata

WORKDIR /app

COPY ${TARGETARCH}/scryer ${TARGETARCH}/scryer-launcher /opt/scryer/

RUN chmod +x /opt/scryer/scryer /opt/scryer/scryer-launcher \
    && mkdir -p /config /data

USER 0:0

EXPOSE 8080
VOLUME /config

ENV PUID=1000
ENV PGID=1000
ENV SCRYER_PACKAGE=docker
ENV TZ=Etc/UTC
ENV UMASK=022
ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/config/scryer.db
ENV EXTISM_CACHE_CONFIG=
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt

# Graceful shutdown: let in-flight requests and background tasks finish
STOPSIGNAL SIGTERM

ENTRYPOINT ["/opt/scryer/scryer-launcher"]
