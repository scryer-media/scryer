# Crates

The workspace follows the layering in [ARCHITECTURE.md](../ARCHITECTURE.md):
domain → application → infrastructure/interface adapters → the `scryer` binary.

| Crate | Role |
| --- | --- |
| `scryer-domain` | Shared entities, value objects, and invariants (titles, media facets, downloads, imports, settings types). No IO. |
| `scryer-application` | Use cases, workflows, and ports (repository/client/provider traits); `AppServices`/`AppUseCase`; background workflow entry points. See its README for a reading guide. |
| `scryer-infrastructure-*` | Adapters behind the application ports: SQL stores (SQLite/PostgreSQL), download-client and indexer clients, metadata/subtitle providers, filesystem and media helpers. |
| `scryer-interface*` | The GraphQL API. `scryer-interface` composes the schema; `-core` holds context/loaders/error mapping; `-query`, `-subscription`, `-media(-types)`, `-acquisition`, `-import`, `-metadata`, `-security`, `-settings`, `-system` hold the resolvers and payload types per area. |
| `scryer` | The service binary: HTTP server, embedded web UI, migrations, jobs, tray/desktop entry points, integration tests. |
| `scryer-plugins` | Plugin host: Wasm loading, descriptor/ABI validation, indexer/download-client/notification/subtitle/archive adapters, permission-enforced HTTP and socket hosts. |
| `scryer-plugin-sdk` | Published wire contracts and JSON Schema for plugins (deprecated for direct plugin development; use `scryer-plugin-pdk`). |
| `scryer-release-parser` | Deterministic release-name parser with title-context targeting. |
| `scryer-rules` | Rego rule evaluation (via `regorus`) and built-in rule sets. |
| `scryer-outbound-http` | Outbound HTTP client with rate-limit, retry, cooldown, and redirect policy. |
| `scryer-mediainfo` | Container/codec probing for media files (MKV, MP4, AVI, TS, OGG, FLV, ASF). |
| `scryer-webauthn` | WebAuthn/passkey helpers used by the security workflows. |
| `scryer-runtime-info` | CPU feature / build-lane detection for release binaries. |
| `scryer-launcher` | Container entry point that drops privileges and `exec`s `scryer`. |
| `scryer-mock-apis` | Mock NZBGet/Newznab/SMG servers for tests and e2e fixtures. |
