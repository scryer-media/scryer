/// Built-in NZBGeek indexer WASM plugin. Handles NZBGeek-specific metadata
/// (thumbs, subtitles, password) and the Newznab protocol.
pub const NZBGEEK_WASM: &[u8] = include_bytes!("../builtins/nzbgeek_indexer.wasm");

/// Built-in generic Newznab indexer WASM plugin. Handles the standard Newznab
/// protocol for DogNZB and other compatible indexers.
pub const NEWZNAB_WASM: &[u8] = include_bytes!("../builtins/newznab_indexer.wasm");
