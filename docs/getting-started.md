# Getting Started

## macOS — Homebrew

The easiest way to run scryer on macOS is via Homebrew:

```bash
brew install scryer-media/scryer/scryer
```

Start scryer as a background service:

```bash
brew services start scryer
```

Open `http://localhost:8080` to access the web UI.

To stop the service:

```bash
brew services stop scryer
```

Configuration is at `$(brew --prefix)/etc/scryer/config.env`. Edit it to set your media paths, then restart the service.

---

## Docker Compose

For Linux, NAS devices, or if you prefer containers.

### Prerequisites

- Docker and Docker Compose installed
- A Usenet download client (NZBGet or SABnzbd) running and accessible
- Media directories on your host for movies and/or series

### Quick Start

The fastest way to get running is the built-in setup wizard. It generates a `docker-compose.yml` and a `scryer_encryption_key.txt` file with a fresh encryption key:

```bash
mkdir scryer && cd scryer

docker run --rm -it -v .:/output -w /output ghcr.io/scryer-media/scryer init
```

You'll be prompted for your host media directories:

```
Movies directory on this host [/data/movies]: /mnt/nas/movies
Series directory on this host [/data/series]: /mnt/nas/series
wrote docker-compose.yml
wrote scryer_encryption_key.txt (0600)

Your encryption key is stored in scryer_encryption_key.txt and mounted
as a Docker secret. Do not lose this file — you will need to reconfigure
passwords and API keys if it changes.

next steps:
  docker compose up -d
```

The encryption key is mounted as a [Docker secret](https://docs.docker.com/compose/how-tos/use-secrets/) — it never appears as an environment variable or in `docker inspect` output.

Then start it:

```bash
docker compose up -d
```

Open `http://localhost:8080` to access the web UI.

## Manual Setup

If you prefer to create the compose file yourself:

### 1. Generate an encryption key

The encryption key protects passwords and API keys stored in the database. Generate one before first run:

```bash
docker run --rm ghcr.io/scryer-media/scryer --generate-key > scryer_encryption_key.txt
chmod 600 scryer_encryption_key.txt
```

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
      - scryer-config:/config
      - /path/to/your/movies:/data/movies
      - /path/to/your/series:/data/series
    secrets:
      - scryer_encryption_key
    environment:
      SCRYER_METADATA_GATEWAY_GRAPHQL_URL: https://smg.scryer.media/graphql

secrets:
  scryer_encryption_key:
    file: ./scryer_encryption_key.txt

volumes:
  scryer-config:
```

Replace `/path/to/your/movies` and `/path/to/your/series` with the actual directories on your host where your media lives (or where you want scryer to organize files into).

The encryption key is mounted as a Docker secret at `/run/secrets/scryer_encryption_key` inside the container. It never appears as an environment variable.

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

Scryer defaults to `/data/movies`, `/data/series`, and `/data/anime` inside the container. Keep your volume mounts aligned to those container-side paths:

```yaml
volumes:
  - /mnt/nas/movies:/data/movies    # host path : container path
  - /mnt/nas/series:/data/series
```

### Download Clients and Indexers

Download clients (NZBGet, SABnzbd) and indexers are configured in **Settings** through the web UI after first login. No environment variables needed.

### Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `SCRYER_METADATA_GATEWAY_GRAPHQL_URL` | Yes | — | Metadata API endpoint URL |
| `SCRYER_ENCRYPTION_KEY` | No | Auto-managed | Override encryption key (see below) |
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

The encryption key protects passwords and API keys at rest in the database.

**macOS and Windows:** Managed automatically — no action needed. The key is generated on first startup and stored securely by the OS.

**Docker:** The key lives in `scryer_encryption_key.txt` and is mounted as a Docker secret. The `init` wizard creates this file for you. Do not lose it — you'll need to reconfigure passwords and API keys if it changes.

**Linux (bare metal):** The key is stored at `<data_dir>/encryption.key` on first startup.

The `SCRYER_ENCRYPTION_KEY` environment variable is supported as an explicit override on any platform.

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

## Upgrading

Pull the latest image and recreate the container:

```bash
docker compose pull
docker compose up -d
```

Database migrations run automatically on startup. The `scryer-config` volume preserves your database and all settings across upgrades. No manual steps required.

## Backup & Restore

Scryer stores everything in a single SQLite database file at `/config/scryer.db` inside the container, which lives on the `scryer-config` Docker volume. Back up your `scryer_encryption_key.txt` file as well — without it, encrypted settings cannot be recovered.

**Quick backup:**

```bash
docker compose stop
docker run --rm -v scryer-config:/config -v .:/backup busybox cp /config/scryer.db /backup/scryer-backup.db
cp scryer_encryption_key.txt scryer_encryption_key.txt.bak
docker compose start
```

**Restore:**

```bash
docker compose stop
docker run --rm -v scryer-config:/config -v .:/backup busybox cp /backup/scryer-backup.db /config/scryer.db
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
- Check that your volume mounts point to `/data/movies` and `/data/series` inside the container

**Encryption key warning on startup**
- On Docker, check that `scryer_encryption_key.txt` exists and is mounted via the `secrets:` section in your compose file
- On macOS/Windows/Linux bare metal, the key is managed automatically — this warning only appears on first run

**Check the version**

```bash
docker run --rm ghcr.io/scryer-media/scryer --version
```
