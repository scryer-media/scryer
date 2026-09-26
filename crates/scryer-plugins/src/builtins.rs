#[derive(Clone, Copy, Debug)]
pub struct BuiltinPluginAsset {
    pub wasm_zstd: &'static [u8],
    pub descriptor_json: &'static str,
    pub description: &'static str,
}

/// Built-in generic Newznab indexer plugin asset pair.
pub const NEWZNAB: BuiltinPluginAsset = BuiltinPluginAsset {
    wasm_zstd: include_bytes!("../builtins/newznab_indexer.wasm.zst"),
    descriptor_json: include_str!("../builtins/newznab_indexer.descriptor.json"),
    description: include_str!("../builtins/newznab_indexer.description.txt"),
};

/// Built-in Torznab indexer plugin asset pair.
pub const TORZNAB: BuiltinPluginAsset = BuiltinPluginAsset {
    wasm_zstd: include_bytes!("../builtins/torznab_indexer.wasm.zst"),
    descriptor_json: include_str!("../builtins/torznab_indexer.descriptor.json"),
    description: include_str!("../builtins/torznab_indexer.description.txt"),
};

pub const INDEXER_BUILTINS: &[BuiltinPluginAsset] = &[NEWZNAB, TORZNAB];
pub const SUBTITLE_BUILTINS: &[BuiltinPluginAsset] = &[];
pub const DOWNLOAD_CLIENT_BUILTINS: &[BuiltinPluginAsset] = &[];
pub const NOTIFICATION_BUILTINS: &[BuiltinPluginAsset] = &[];
pub const LIST_BUILTINS: &[BuiltinPluginAsset] = &[];

pub fn decode_builtin_wasm(asset: BuiltinPluginAsset) -> Result<Vec<u8>, String> {
    zstd::decode_all(asset.wasm_zstd)
        .map_err(|error| format!("failed to decompress built-in WASM asset: {error}"))
}

pub fn builtin_description_for_provider(provider_type: &str) -> Option<&'static str> {
    let key = provider_type.trim().to_ascii_lowercase();
    INDEXER_BUILTINS
        .iter()
        .chain(SUBTITLE_BUILTINS.iter())
        .chain(DOWNLOAD_CLIENT_BUILTINS.iter())
        .chain(NOTIFICATION_BUILTINS.iter())
        .chain(LIST_BUILTINS.iter())
        .find_map(|asset| {
            let descriptor: serde_json::Value = serde_json::from_str(asset.descriptor_json).ok()?;
            let provider = descriptor.get("provider")?;
            let actual = provider.get("provider_type")?.as_str()?;
            (actual.eq_ignore_ascii_case(&key)).then_some(asset.description.trim())
        })
}
