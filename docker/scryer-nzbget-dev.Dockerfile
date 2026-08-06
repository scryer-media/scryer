FROM debian:13-slim@sha256:3a39a0592364683e6bab97937b72cad5a8fa6dcbbee90edb3bb48c7f8e94f258

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      nzbget \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /config

CMD ["/bin/bash", "-lc"]
