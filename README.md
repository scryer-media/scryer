# scryer

Rust-backed replacement for Sonarr/Radarr capabilities in a single binary, designed as a
**spec-driven / intention-driven** codebase first.

## Why?

The Servarr products are incredible and such an evolutionary step beyond what we had before with sickbeard and couchpotato. However, their architecture doesn't lend itself well to low compute environments (aka NAS), which is unfortunately where a lot folks want to run these tools. The tools even poke fun at themselves with their cheeky loading screens of "i'd be faster if i was written in python".

The mission of Scryer is three fold:
  1. Use architecture that is consistent with the deployment environment
  2. Collapse the movie and series management into a single tool
  3. Add more native facility for anime management

The hope is not to replace servarr products, but offer another option that works better in different scenarios.

Current state, Scryer runs using <100MiB of RAM while a comparable sonarr/radarr installation is using ~400-500MiB of RAM between the two tools. Since the tool compiles natively to the target environment, it will run with much less overhead as well. The hope is that this means Scryer will perform much better in NAS environments and also scale well to higher powered servers.

We have been avid users of Servarr products for many years. As always, our opinions on how the app is organized will vary from the Servarr products. Certain ways the tools work have created friction that Scryer aims to solve.

Scryer is written and maintained by the assistance of agentic AI agents. The tool is overseen by seasoned software engineers that manage the agents. Multiple models are used with human reviews to ensure the best possible code.

### How Scryer is different

Scryer is a single binary that manages movies, TV series, and anime all in one place. Instead of running separate applications for each media type, you get one unified tool with a modern web UI. The backend is written in Rust for minimal resource usage, and the UI is a Next.js static app embedded directly into the binary -- no Node.js runtime needed at deploy time.

A separate companion service called **SMG (Scryer Metadata Gateway)** handles all communication with metadata providers (TVDB, TMDB). SMG aggressively caches metadata in PostgreSQL so that upstream APIs are not hammered, keeping you safely within rate limits. SMG is hosted centrally — you don't need to run it yourself.

## Getting Started

Scryer is deployed via Docker Compose. See the **[Getting Started guide](docs/deployment/getting-started.md)** for setup instructions covering:

- One-command setup with `scryer init`
- Default credentials and encryption key management
- Download client and metadata gateway configuration
- Upgrading and backup/restore

---

## Roadmap

### Downloaders
  1. NZBGet - first implementation and always preferred downloader. This is because the tool is built with the same mentality, lean for low compute environments.
  2. SabNZBD - Eventually

### Indexers
  1. NZBGeek - Maintainers use this indexer
  2. DogNZB - Maintainers use this indexer
  3. *TBD* - Will need to be voted on by community. Maintainers will need to be able to access the indexer

### Media Types
  1. Movies / TV / Anime

### Mobile

Mobile will be delivered via PWA as the app stores historically have not been open to media management apps. The goal is a first class PWA that behaves like a mobile app with UI controls that let users easily navigate the product on a phone.

### Other Integrations

Servarr products have a ton of integrations with other tooling. We will be adding the most requested ones over time, but do not ever intend to support such a broad diversity of integrations. We are a small team and do not have the time to support them.

### Plugin Framework

Eventually we want to create a plugin framework where community members can develop additional components for Scryer, like audio book or music management.

---

### Reporting issues

File bug reports and feature requests in the [GitHub Issues](https://github.com/scryer-media/scryer/issues) tab. Please include:
- Your platform (OS / architecture)
- The release version (`docker run --rm ghcr.io/scryer-media/scryer --version`)
- Relevant log output (`docker compose logs scryer`)

---

For development setup and contributing, see [CONTRIBUTORS.md](CONTRIBUTORS.md).
