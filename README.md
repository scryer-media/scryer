<p align="center">
  <img src="docs/scryer-hero.svg" alt="scryer" width="200"/>
</p>

<h1 align="center">scryer</h1>

<p align="center">
  <strong>A single-binary media manager for movies, TV series, and anime.</strong>
</p>

<p align="center">
  <a href="docs/getting-started.md">Getting Started</a> &middot;
  <a href="https://github.com/scryer-media/scryer/issues">Issues</a> &middot;
  <a href="CONTRIBUTORS.md">Contributing</a>
</p>

---

Scryer manages movies, TV series, and anime in one place. It monitors your library, searches indexers for releases, sends downloads to your Usenet client, and organizes the results into your media directories. The backend is written in Rust and the UI is a Vite + React static app embedded directly into the binary — no runtime dependencies at deploy time.

A companion service called **SMG (Scryer Metadata Gateway)** handles all communication with metadata providers (TVDB, TMDB). SMG caches metadata in PostgreSQL so upstream APIs are not hammered, keeping you within rate limits. SMG is hosted centrally — you don't need to run it yourself.

## Getting Started

See the **[Getting Started guide](docs/getting-started.md)** for setup instructions covering:

- One-command setup with `scryer init`
- Default credentials and encryption key management
- Download client and metadata gateway configuration
- Upgrading and backup/restore

## Comparison with Other Tools

Scryer occupies the same space as Sonarr, Radarr, and SickChill. Each tool makes different trade-offs:

| | Scryer | Sonarr / Radarr | SickChill |
|---|---|---|---|
| **Media types** | Movies, series, anime in one binary | Separate app per media type (Radarr for movies, Sonarr for series) | Series and anime only |
| **Runtime** | Rust binary (~30 MB), <100 MB RAM typical | .NET, ~200-300 MB RAM each | Python, ~150-200 MB RAM |
| **UI** | Embedded static React app, no separate runtime | Embedded frontend | Web UI |
| **Anime support** | First-class facet with anime-specific metadata mapping | Community-supported via Sonarr | Built-in |
| **Metadata** | Centrally cached via SMG (TVDB, TMDB) | Direct API calls per instance | Direct API calls (TVDB) |
| **Download clients** | NZBGet, SABnzbd | NZBGet, SABnzbd, qBittorrent, Deluge, and others | NZBGet, SABnzbd, and torrent clients |
| **Indexer support** | Plugin-based (NZBGeek, Newznab-compatible) | Broad Newznab/Torznab support, Prowlarr integration | Built-in indexer support |
| **Maturity** | Early, active development | Mature, large community and ecosystem | Mature, smaller community |
| **Configuration** | Docker Compose, single binary | Docker or native install, per-app config | Docker or native install |

**When Scryer may be a good fit:** You want a single lightweight process for all media types, you're running on a NAS or low-power hardware, or you want unified anime management without separate tools.

**When Sonarr/Radarr may be a better fit:** You need broad torrent client support, you rely on the Prowlarr/Lidarr/*arr ecosystem, or you need the stability of a mature project with a large community.

**When SickChill may be a better fit:** You only manage series/anime and prefer a Python-based tool with a different approach to show management.

## Roadmap

### Download Clients
- NZBGet
- SABnzbd

### Indexers
- Plugin-based architecture with built-in support for NZBGeek and Newznab-compatible indexers (DogNZB, etc.)

### Mobile
Mobile will be delivered via PWA. The goal is a first-class PWA that behaves like a native app with UI controls designed for phone navigation.

### Plugin Framework
A WASM-based plugin framework enables community-developed components for additional indexers, download clients, and notification services.

---

### Reporting Issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab. Please include:
- Your platform (OS / architecture)
- The release version (`docker run --rm ghcr.io/scryer-media/scryer --version`)
- Relevant log output (`docker compose logs scryer`)

---

For development setup and contributing, see [CONTRIBUTORS.md](CONTRIBUTORS.md).
