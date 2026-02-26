FROM debian:12-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates \
      nzbget \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /config

CMD ["/bin/bash", "-lc"]
