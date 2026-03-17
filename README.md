<p align="center">
  <img src="docs/img/scryer-hero.png" alt="scryer" width="200"/>
</p>

<h1 align="center">scryer</h1>

<p align="center">
  <strong>One app for movies, TV series, and anime.</strong>
</p>

<p align="center">
  <a href="docs/getting-started.md">Getting Started</a> &middot;
  <a href="https://github.com/scryer-media/scryer/issues">Issues</a> &middot;
  <a href="CONTRIBUTORS.md">Contributing</a>
</p>

---

Scryer monitors your media library, automatically downloads movies and TV shows you want, upgrades them when better quality becomes available, renames and organizes files, and fetches subtitles — all from a single lightweight application.

## Features

- **Movies, TV, and anime in one place** — no need to run separate apps for each media type
- **Automatic downloads** — searches your indexers, grabs the best available release, and imports it into your library
- **Quality upgrades** — configurable quality profiles with scoring rules; scryer automatically replaces files when a better version appears
- **Subtitles** — finds and downloads matching subtitles from OpenSubtitles, with automatic timing correction
- **Anime franchise movies** — tracks movies that belong between anime seasons and places them in the right watch order for Plex and Jellyfin
- **Metadata** — artwork, episode data, and cross-referenced anime IDs (TVDB, TMDB, MAL, AniList, AniDB) fetched automatically
- **File organization** — renames and sorts files into clean folder structures that work with Plex, Jellyfin, and Emby
- **Post-processing** — run custom scripts after imports, with full release metadata available
- **Rules engine** — flexible policy rules for filtering releases, adjusting scores, and automating decisions
- **GraphQL API** — fully queryable API for building integrations or custom workflows
- **Plugin system** — extend scryer with plugins for additional indexers, download clients, and notification services
- **Single binary** — ~30 MB, ~60 MB RAM, runs on anything from a Raspberry Pi to a NAS
- **PWA** — installable progressive web app with a mobile-friendly interface

## Getting Started

See the **[Getting Started guide](docs/getting-started.md)** for setup instructions covering:

- One-command setup with `scryer init`
- Docker deployment
- Credentials and encryption key management
- Upgrading and backup/restore

## How It Compares

If you've used Sonarr, Radarr, or Bazarr, scryer does what all three do — in a single app that uses a fraction of the resources.

| | Scryer | Sonarr + Radarr + Bazarr | SickChill |
|---|---|---|---|
| **Media types** | Movies, series, and anime together | One app per media type, plus Bazarr for subtitles | Series and anime only |
| **Subtitles** | Built in | Separate service | Not included |
| **Memory** | ~60 MB | ~500+ MB combined | ~150-200 MB |
| **Anime** | First-class support with franchise movie tracking | Community-supported via Sonarr | Built-in |
| **Indexers** | Plugin-based, Newznab-compatible | Broad Newznab/Torznab support, Prowlarr integration | Built-in |
| **Maturity** | Active development | Mature, large ecosystem | Mature, smaller community |

**When Scryer fits:** You want one lightweight process for all media types including subtitles, you run on constrained hardware, or you want unified anime management without coordinating multiple services.

**When Sonarr/Radarr fits better:** You need broad torrent client support, you rely on the Prowlarr/Lidarr ecosystem, or you prefer the stability and community of a mature project.

## Architecture

```
┌─────────────────────────────────────────┐
│  scryer binary                          │
│  ┌───────────┐  ┌────────────────────┐  │
│  │ Web UI    │  │ GraphQL API        │  │
│  └───────────┘  └────────────────────┘  │
│  ┌────────────────────────────────────┐  │
│  │ Application layer                  │  │
│  │ acquisition · import · subtitles   │  │
│  │ rename · post-processing · rules   │  │
│  └────────────────────────────────────┘  │
│  ┌────────────────────────────────────┐  │
│  │ Storage (SQLite) + Plugins         │  │
│  └────────────────────────────────────┘  │
└─────────────────────────────────────────┘
         │                    │
    ┌────┴────┐        ┌─────┴──────┐
    │ Metadata│        │ Indexers & │
    │  API    │        │ Clients    │
    └─────────┘        └────────────┘
```

## Roadmap

- Additional subtitle providers (Podnapisi, Addic7ed)
- Torrent client support
- Calendar view for upcoming releases
- Import history and audit log improvements
- Mobile app refinements

---

### Reporting Issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab. Please include:
- Your platform (OS / architecture)
- The release version (`docker run --rm ghcr.io/scryer-media/scryer --version`)
- Relevant log output (`docker compose logs scryer`)

---

For development setup and contributing, see [CONTRIBUTORS.md](CONTRIBUTORS.md).
