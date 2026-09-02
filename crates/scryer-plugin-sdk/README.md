# scryer-plugin-sdk

[![crates.io](https://img.shields.io/crates/v/scryer-plugin-sdk.svg)](https://crates.io/crates/scryer-plugin-sdk)
[![docs.rs](https://docs.rs/scryer-plugin-sdk/badge.svg)](https://docs.rs/scryer-plugin-sdk)

> [!WARNING]
> **Deprecated for direct plugin development.** New Scryer plugins should use
> [`scryer-plugin-pdk`](https://crates.io/crates/scryer-plugin-pdk), which
> provides the supported guest runtime and re-exports these contract types.

This crate remains published as the shared compatibility, wire-contract, and
schema layer used by the PDK, Scryer itself, and existing SDK-based plugins.

This crate defines Scryer's plugin descriptors, configuration fields,
capabilities, request and response payloads, compatibility checks, host-service
messages, and generated JSON Schema. It does not run a plugin by itself.

## Use the PDK for plugins

- New plugins should depend on `scryer-plugin-pdk`, not this crate directly.
- Host integrations, catalog tools, and schema consumers may still use this
  crate directly when they only need the underlying contract types.

## What is included

- `PluginDescriptor` and typed provider descriptors for indexers, download
  clients, notifications, subtitles, archive extraction, and subtitle sync.
- Configuration metadata, declared host permissions, provider capabilities,
  and scoring policies.
- Typed request, response, result, and error payloads for every plugin family.
- Torrent, networking, HTTP, notification, and native host-service contracts.
- SDK and host-version compatibility helpers.
- A generated JSON Schema bundle for registry validation and non-Rust tooling.

## Plugin runtime

Scryer loads plugins exclusively as WASI Preview 2 components built for
`wasm32-wasip2`, one WIT world per plugin family, with host services reaching
the guest through the shared `scryer:host/services@1.0.0` import. Older
guest transports have been removed from the host: a legacy artifact is refused
at load time with an upgrade diagnostic, and this crate carries no guest-side
transport at all.

Do not depend on this crate's transport for a new plugin. Use
[`scryer-plugin-pdk`](https://github.com/scryer-media/scryer-plugins/tree/main/pdk/scryer-plugin-pdk),
whose family entry macros generate the component exports and install the
host-call transport; this crate supplies the typed contracts those payloads
carry. The first-party plugins repository linked below contains a working,
conformance-tested example for every family.

For real implementations, declare the capabilities and configuration fields
you support, return typed results, and list every network destination the plugin
needs in its descriptor. Scryer enforces the declared host and socket
permissions at runtime.

## Compatibility

Always populate descriptors from the SDK instead of hard-coding compatibility
values:

```rust
sdk_version: SDK_VERSION.to_string(),
sdk_constraint: current_sdk_constraint(),
```

Scryer validates this contract before loading a plugin. A plugin should only
advertise exports and capabilities it actually implements.

## Schema and examples

- [API documentation](https://docs.rs/scryer-plugin-sdk)
- [SDK source and schema bundle](https://github.com/scryer-media/scryer/tree/main/crates/scryer-plugin-sdk)
- [First-party plugins and scaffolding](https://github.com/scryer-media/scryer-plugins)

Print the generated schema from a Scryer checkout with:

```console
cargo run -p scryer-plugin-sdk --example print-schema
```

## License

GPL-3.0-only
