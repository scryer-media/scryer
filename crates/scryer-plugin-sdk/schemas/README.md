# Scryer Plugin SDK Schemas

`plugin-sdk-v3.schema.json` is the committed JSON Schema bundle for the
SDK 3 ABI family.
It contains `$defs` for the descriptor, config fields, and each request/response
payload family used by indexer, download-client, notification, and subtitle plugins.
SDK 3 is the catalog-v3 compatibility reset line and keeps the string-in/string-out
Wasm export shape while allowing source-breaking Rust SDK payload updates.

The Rust SDK remains the source of truth for first-party plugins. These schemas are
for registry validation, fixture tooling, and non-Rust plugin authors.

SDK 3.1 adds `supported_query_facets` on indexer capabilities so providers can
advertise facet-scoped title/freetext search separately from ID search support.
