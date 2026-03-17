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
- **Quality upgrades** — set your preferred quality and scryer will automatically replace files when a better version appears
- **Subtitles** — finds and downloads matching subtitles from OpenSubtitles, with automatic timing correction
- **Anime franchise movies** — tracks movies that belong between anime seasons and places them in the right watch order
- **File organization** — renames and sorts files into clean folder structures that work with Plex, Jellyfin, and Emby
- **Lightweight** — a single ~30 MB download, uses ~60 MB of memory, runs on anything from a Raspberry Pi to a NAS
- **Installable on your phone** — works as a progressive web app with a mobile-friendly interface
- **Extensible** — supports plugins for additional indexers, download clients, and notification services

## Getting Started

See the **[Getting Started guide](docs/getting-started.md)** for setup instructions.

## How It Compares

If you've used Sonarr, Radarr, or Bazarr, scryer does what all three do — in a single app that uses a fraction of the resources.

| | Scryer | Sonarr + Radarr + Bazarr |
|---|---|---|
| **Media types** | Movies, series, and anime together | One app per media type, plus Bazarr for subtitles |
| **Subtitles** | Built in | Separate service |
| **Memory usage** | ~60 MB | ~500+ MB combined |
| **Anime** | First-class support with franchise movie tracking | Community-supported |

**When Sonarr/Radarr may be a better fit:** You need broad torrent client support, you rely on the Prowlarr/Lidarr ecosystem, or you prefer the stability and community of a mature project.

## Roadmap

- Additional subtitle providers
- Torrent client support
- Calendar view for upcoming releases
- Mobile app refinements

---

### Reporting Issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab. Please include:
- Your platform (OS / architecture)
- The release version (`docker run --rm ghcr.io/scryer-media/scryer --version`)
- Relevant log output (`docker compose logs scryer`)

---

For development setup and contributing, see [CONTRIBUTORS.md](CONTRIBUTORS.md).
