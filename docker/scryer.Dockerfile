FROM busybox:1.37 AS prep
RUN mkdir -p /data && chown 65532:65532 /data

FROM gcr.io/distroless/static-debian12:nonroot

ARG TARGETARCH

WORKDIR /app

COPY ${TARGETARCH}/scryer /usr/local/bin/scryer

EXPOSE 8080

# Persist the SQLite database across container upgrades.
# Pre-create /data owned by nonroot (65532) so SQLite can write.
COPY --from=prep --chown=65532:65532 /data /data
VOLUME /data

ENV SCRYER_BIND=0.0.0.0:8080
ENV SCRYER_DB_PATH=/data/scryer.db
ENV WASMTIME_CACHE_ENABLED=0

# Graceful shutdown: let in-flight requests and background tasks finish
STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/local/bin/scryer"]
