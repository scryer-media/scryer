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

SDK 3.2 adds first-class `path` and `tag` config field types and extends
download-client add requests with optional title slug, year, language, and
network metadata.

SDK 3.11 adds the `filtered_select` config field type, plus `visible_when`,
`required_when`, and `advanced` on config fields. Conditions are a closed set of
operators (`eq`, `ne`, `in`, `not_in`, `non_empty`) evaluated by the host, so a
plugin declares what a field depends on rather than running logic in the form.
All three are optional and default to the prior behaviour, so a descriptor
written against an earlier SDK keeps its exact meaning.
