# Getting Started

This guide walks you through setting up scryer with Docker Compose.

## Prerequisites

- Docker and Docker Compose installed
- A Usenet download client (NZBGet or SABnzbd) running and accessible
- Media directories on your host for movies and/or series

## Quick Start

The fastest way to get running is the built-in setup wizard. It generates a `docker-compose.yml` with a fresh encryption key and your media paths:

```bash
mkdir scryer && cd scryer

docker run --rm -it -v .:/output -w /output ghcr.io/scryer-media/scryer init
```

You'll be prompted for your host media directories:

```
Movies directory on this host [/media/movies]: /mnt/nas/movies
Series directory on this host [/media/series]: /mnt/nas/series
wrote docker-compose.yml

next steps:
  docker compose up -d
```

Then start it:

```bash
docker compose up -d
```

Open `http://localhost:8080` and log in with the default credentials:

| | |
|---|---|
| Username | `admin` |
| Password | `admin` |

Change your password in Settings after first login.

## Manual Setup

If you prefer to create the compose file yourself:

### 1. Generate an encryption key

The encryption key protects passwords and API keys stored in the database. Generate one before first run:

```bash
docker run --rm ghcr.io/scryer-media/scryer --generate-key
```

Copy the output — you'll paste it into your compose file.

### 2. Create docker-compose.yml

```yaml
services:
  scryer:
    image: ghcr.io/scryer-media/scryer:latest
    container_name: scryer
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - scryer-data:/data
      - /path/to/your/movies:/media/movies
      - /path/to/your/series:/media/series
    environment:
      SCRYER_ENCRYPTION_KEY: <paste your key here>
      SCRYER_MOVIES_PATH: /media/movies
      SCRYER_SERIES_PATH: /media/series
      SCRYER_METADATA_GATEWAY_GRAPHQL_URL: https://smg.scryer.media/graphql

volumes:
  scryer-data:
```

Replace `/path/to/your/movies` and `/path/to/your/series` with the actual directories on your host where your media lives (or where you want scryer to organize files into).

### 3. Start the service

```bash
docker compose up -d
```

## Configuration

Infrastructure settings are configured via environment variables in your `docker-compose.yml`. Application settings like download clients and indexers are configured through the web UI after startup.

### Metadata API

Scryer uses a hosted metadata service for title search, artwork, and metadata. The `init` wizard configures this automatically. For manual setup, point to the hosted instance:

```yaml
environment:
  SCRYER_METADATA_GATEWAY_GRAPHQL_URL: https://smg.scryer.media/graphql
```

### Media Paths

The `SCRYER_MOVIES_PATH` and `SCRYER_SERIES_PATH` environment variables tell scryer where to find media **inside the container**. These must match the right side of your volume mounts:

```yaml
volumes:
  - /mnt/nas/movies:/media/movies    # host path : container path
  - /mnt/nas/series:/media/series
environment:
  SCRYER_MOVIES_PATH: /media/movies   # matches container path
  SCRYER_SERIES_PATH: /media/series
```

### Download Clients and Indexers

Download clients (NZBGet, SABnzbd) and indexers are configured in **Settings** through the web UI after first login. No environment variables needed.

### Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `SCRYER_ENCRYPTION_KEY` | Recommended | Auto-generated | Encryption key for sensitive settings |
| `SCRYER_MOVIES_PATH` | Yes | — | Movies directory inside the container |
| `SCRYER_SERIES_PATH` | Yes | — | Series directory inside the container |
| `SCRYER_METADATA_GATEWAY_GRAPHQL_URL` | Yes | — | Metadata API endpoint URL |
| `SCRYER_BIND` | No | `0.0.0.0:8080` | Listen address and port |
| `SCRYER_BASE_PATH` | No | `/` | Optional reverse-proxy path prefix, for example `/scryer` |
| `SCRYER_TLS_CERT` | No | — | Path to TLS certificate (PEM) |
| `SCRYER_TLS_KEY` | No | — | Path to TLS private key (PEM) |

### Reverse Proxy Base Path

If your proxy publishes scryer under a path prefix instead of a dedicated host, set `SCRYER_BASE_PATH` to that prefix:

```yaml
environment:
  SCRYER_BASE_PATH: /scryer
```

With that setting, the UI, GraphQL endpoint, WebSocket endpoint, splash screen, and static assets are all served under `/scryer`. Configure your proxy to forward `/scryer` without stripping the prefix.

## Encryption Key

The encryption key protects passwords and API keys at rest in the database. There are two ways it can be managed:

1. **Set explicitly in docker-compose.yml** (recommended) — survives volume deletion, makes backup/restore straightforward
2. **Auto-generated on first run** — stored in the database inside the `scryer-data` volume. If you lose the volume, the key is gone and encrypted settings cannot be recovered

If you used the `init` wizard, the key is already in your compose file. If you see a warning on startup about a generated key, copy it into your `docker-compose.yml` `SCRYER_ENCRYPTION_KEY` variable.

## Resource Requirements

Scryer is lightweight and primarily I/O-bound (network requests to indexers, file imports). CPU usage is low outside of media analysis and subtitle timing sync.

### Baseline

| Resource | Idle / Monitoring | Active (imports, searches) |
|---|---|---|
| **Memory** | 60-80 MB | Up to ~150 MB |
| **CPU** | Negligible | Brief spikes during media analysis |
| **Disk** | ~5-20 MB (SQLite database) | 500 MB free required per import operation |

### Container Orchestration

For Kubernetes, Nomad, ECS, or similar schedulers:

| | Request (guaranteed minimum) | Limit (burst ceiling) |
|---|---|---|
| **Memory** | 128 Mi | 256 Mi |
| **CPU (Kubernetes)** | 100m (0.1 cores) | 500m (0.5 cores) |
| **CPU (Nomad)** | 100 MHz | 500 MHz |

These are conservative starting points. Scryer will run at lower CPU allocations — background tasks will just take longer. If you're managing a large library (1000+ titles) or running frequent searches, consider increasing the CPU limit.

There is no hard CPU floor. Scryer does not pin threads or require specific core counts. Constrained environments like Raspberry Pi (1 GHz ARM, 512 MB RAM) work fine.

### Example: Kubernetes

```yaml
resources:
  requests:
    memory: "128Mi"
    cpu: "100m"
  limits:
    memory: "256Mi"
    cpu: "500m"
```

### Example: Nomad

```hcl
resources {
  memory = 128
  cpu    = 100
}
```

### Example: ECS

```json
{
  "cpu": 256,
  "memory": 256,
  "memoryReservation": 128
}
```

## Upgrading

Pull the latest image and recreate the container:

```bash
docker compose pull
docker compose up -d
```

Database migrations run automatically on startup. The `scryer-data` volume preserves your database and all settings across upgrades. No manual steps required.

## Backup & Restore

Scryer stores everything in a single SQLite database file at `/data/scryer.db` inside the container, which lives on the `scryer-data` Docker volume.

**Quick backup:**

```bash
docker compose stop
docker run --rm -v scryer-data:/data -v .:/backup busybox cp /data/scryer.db /backup/scryer-backup.db
docker compose start
```

**Restore:**

```bash
docker compose stop
docker run --rm -v scryer-data:/data -v .:/backup busybox cp /backup/scryer-backup.db /data/scryer.db
docker compose start
```

## Troubleshooting

**Can't connect to the web UI**
- Verify the container is running: `docker compose ps`
- Check logs: `docker compose logs scryer`
- Ensure port 8080 isn't already in use on your host

**Download client errors**
- Verify the download client is reachable from the scryer container
- Check that the username and password are correct in Settings

**Media not appearing after import**
- Confirm the host directories are mounted correctly and contain media files
- Check that `SCRYER_MOVIES_PATH` / `SCRYER_SERIES_PATH` match the container-side mount paths

**Encryption key warning on startup**
- This means scryer auto-generated a key. Copy the key from the log output into your `docker-compose.yml` to persist it across volume recreations

**Check the version**

```bash
docker run --rm ghcr.io/scryer-media/scryer --version
```
