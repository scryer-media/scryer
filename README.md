<p align="center">
  <img src="docs/img/scryer-hero.png" alt="scryer" width="200"/>
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

Scryer organizes movies, TV series, and anime in a single application. It monitors your library, tracks wanted media, manages quality upgrades, and handles file renaming and organization across your media directories. The backend is written in Rust with a React frontend embedded directly into the binary — no runtime dependencies, no external services, one process.

## Features

- **Unified library** — movies, TV series, and anime managed together with per-facet settings for quality profiles, naming conventions, and root folders
- **Subtitle management** — searches OpenSubtitles for missing subtitles, scores matches using release metadata and file hashing, and synchronizes timing using built-in audio analysis (no external tools required)
- **Quality-driven upgrades** — configurable quality profiles with scoring rules, delay profiles, and automatic upgrade tracking
- **Metadata integration** — centralized metadata from TVDB and TMDB via the Scryer Metadata Gateway, with anime ID cross-referencing (MAL, AniList, AniDB, Kitsu)
- **Background image management** — automatically fetches and caches posters, banners, and fanart with responsive image variants
- **Post-processing** — configurable per-facet scripts that run after media is organized, with full metadata passed via environment variables
- **Plugin framework** — WASM-based plugins for indexers, notification services, and acquisition clients
- **Custom rules engine** — Rego-based policy rules for release filtering, scoring adjustments, and automated decisions
- **Single binary** — Rust backend + embedded React UI, ~30 MB, ~60-70 MB RAM typical, no .NET/Python/Java runtime
- **PWA support** — installable progressive web app with mobile-optimized controls

## Getting Started

See the **[Getting Started guide](docs/getting-started.md)** for setup instructions covering:

- One-command setup with `scryer init`
- Default credentials and encryption key management
- Client configuration
- Upgrading and backup/restore

## Comparison with Other Tools

Scryer occupies the same space as Sonarr, Radarr, and Bazarr. Each tool makes different trade-offs:

| | Scryer | Sonarr + Radarr + Bazarr | SickChill |
|---|---|---|---|
| **Media types** | Movies, series, anime in one binary | Separate app per media type, plus Bazarr for subtitles | Series and anime only |
| **Subtitles** | Built-in (OpenSubtitles, timing sync) | Requires Bazarr as a separate service | Not included |
| **Runtime** | Rust binary (~30 MB), ~60-70 MB RAM | .NET + Python, ~500+ MB RAM combined | Python, ~150-200 MB RAM |
| **UI** | Embedded React app, no separate process | Embedded frontend per app | Web UI |
| **Anime** | First-class facet with cross-database ID mapping | Community-supported via Sonarr | Built-in |
| **Metadata** | Centrally cached via metadata gateway | Direct API calls per instance | Direct API calls |
| **Indexers** | Plugin-based (Newznab-compatible) | Broad Newznab/Torznab support, Prowlarr integration | Built-in |
| **Maturity** | Active development | Mature, large ecosystem | Mature, smaller community |

**When Scryer may fit:** You want one lightweight process for all media types including subtitles, you run on constrained hardware, or you want unified anime management without coordinating multiple services.

**When Sonarr/Radarr may fit better:** You need broad torrent client support, you rely on the Prowlarr/Lidarr/*arr ecosystem, or you need the stability of a mature project with a large community.

## Architecture

```
┌─────────────────────────────────────────┐
│  scryer binary                          │
│  ┌───────────┐  ┌────────────────────┐  │
│  │ React UI  │  │ GraphQL API        │  │
│  │ (embedded)│  │ (async-graphql)    │  │
│  └───────────┘  └────────────────────┘  │
│  ┌────────────────────────────────────┐  │
│  │ Application layer                  │  │
│  │ acquisition · import · subtitles   │  │
│  │ rename · post-processing · rules   │  │
│  └────────────────────────────────────┘  │
│  ┌────────────────────────────────────┐  │
│  │ Infrastructure (SQLite + plugins)  │  │
│  └────────────────────────────────────┘  │
└─────────────────────────────────────────┘
         │                    │
    ┌────┴────┐        ┌─────┴──────┐
    │ Metadata│        │ Indexers & │
    │ Gateway │        │ Clients    │
    │  (SMG)  │        │ (plugins)  │
    └─────────┘        └────────────┘
```

## Roadmap

- Additional subtitle providers (Podnapisi, Addic7ed)
- Torrent client support
- Calendar view for upcoming releases
- Import history and audit log improvements
- Mobile PWA refinements

---

### Reporting Issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab. Please include:
- Your platform (OS / architecture)
- The release version (`docker run --rm ghcr.io/scryer-media/scryer --version`)
- Relevant log output (`docker compose logs scryer`)

---

For development setup and contributing, see [CONTRIBUTORS.md](CONTRIBUTORS.md).
