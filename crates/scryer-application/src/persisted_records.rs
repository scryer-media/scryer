use scryer_domain::Title;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PersistedTitleReadMode {
    #[default]
    Presentation,
    Canonical,
    Matching,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PersistedTitleDecodeOptions<'a> {
    pub mode: PersistedTitleReadMode,
    pub include_external_ids: bool,
    pub base_path: &'a str,
    pub poster_local_path: Option<&'a str>,
    pub background_local_path: Option<&'a str>,
}

pub fn finalize_persisted_title(
    mut title: Title,
    options: PersistedTitleDecodeOptions<'_>,
) -> Title {
    if !options.include_external_ids {
        title.external_ids.clear();
    }

    if matches!(options.mode, PersistedTitleReadMode::Presentation) {
        apply_local_image(
            &mut title.poster_url,
            &mut title.poster_source_url,
            options.base_path,
            options.poster_local_path,
        );
        apply_local_image(
            &mut title.background_url,
            &mut title.background_source_url,
            options.base_path,
            options.background_local_path,
        );
    }

    title
}

pub fn external_plugin_installation_is_supported_shape(
    wasm_bytes: Option<&[u8]>,
    wasm_encoding: &str,
    wasm_digest_algo: Option<&str>,
    wasm_digest: Option<&str>,
    descriptor_present: bool,
) -> bool {
    external_plugin_installation_shape_is_supported(
        wasm_bytes.is_some(),
        wasm_encoding,
        wasm_digest_algo,
        wasm_digest,
        descriptor_present,
    )
}

pub fn external_plugin_installation_shape_is_supported(
    wasm_bytes_present: bool,
    wasm_encoding: &str,
    wasm_digest_algo: Option<&str>,
    wasm_digest: Option<&str>,
    descriptor_present: bool,
) -> bool {
    wasm_bytes_present
        && matches!(wasm_encoding, "zstd" | "brotli")
        && matches!(
            wasm_digest_algo.map(|value| value.trim().to_ascii_lowercase()),
            Some(value) if value == "blake3"
        )
        && wasm_digest.is_some_and(is_hex_digest)
        && descriptor_present
}

fn apply_local_image(
    primary_url: &mut Option<String>,
    source_url: &mut Option<String>,
    base_path: &str,
    local_path: Option<&str>,
) {
    let Some(local_path) = local_path else {
        return;
    };

    *source_url = primary_url.take();
    *primary_url = Some(prefix_local_title_image_path(base_path, local_path));
}

fn prefix_local_title_image_path(base_path: &str, local_path: &str) -> String {
    if base_path.is_empty() {
        local_path.to_string()
    } else {
        format!("{base_path}{local_path}")
    }
}

fn is_hex_digest(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{
        PersistedTitleDecodeOptions, PersistedTitleReadMode,
        external_plugin_installation_is_supported_shape, finalize_persisted_title,
    };
    use chrono::{DateTime, Utc};
    use scryer_domain::{ExternalId, MediaFacet, Title};

    #[test]
    fn finalize_persisted_title_promotes_local_images_and_preserves_sources() {
        let decoded = finalize_persisted_title(
            sample_title(),
            PersistedTitleDecodeOptions {
                mode: PersistedTitleReadMode::Presentation,
                include_external_ids: true,
                base_path: "/scryer",
                poster_local_path: Some("/images/titles/title-1/poster/w500"),
                background_local_path: None,
            },
        );

        assert_eq!(
            decoded.poster_url.as_deref(),
            Some("/scryer/images/titles/title-1/poster/w500")
        );
        assert_eq!(
            decoded.poster_source_url.as_deref(),
            Some("https://example.com/poster.jpg")
        );
        assert_eq!(
            decoded.background_url.as_deref(),
            Some("https://example.com/background.jpg")
        );
        assert!(decoded.background_source_url.is_none());
    }

    #[test]
    fn finalize_persisted_title_matching_mode_preserves_remote_urls_and_can_strip_external_ids() {
        let decoded = finalize_persisted_title(
            sample_title(),
            PersistedTitleDecodeOptions {
                mode: PersistedTitleReadMode::Matching,
                include_external_ids: false,
                base_path: "/ignored",
                poster_local_path: Some("/images/titles/title-1/poster/w500"),
                background_local_path: None,
            },
        );

        assert_eq!(
            decoded.poster_url.as_deref(),
            Some("https://example.com/poster.jpg")
        );
        assert!(decoded.poster_source_url.is_none());
        assert!(decoded.external_ids.is_empty());
    }

    #[test]
    fn plugin_support_shape_requires_expected_artifact_fields() {
        assert!(external_plugin_installation_is_supported_shape(
            Some(&[1, 2, 3]),
            "zstd",
            Some("blake3"),
            Some("abc123"),
            true,
        ));
        assert!(!external_plugin_installation_is_supported_shape(
            Some(&[1, 2, 3]),
            "identity",
            Some("blake3"),
            Some("abc123"),
            true,
        ));
        assert!(!external_plugin_installation_is_supported_shape(
            Some(&[1, 2, 3]),
            "zstd",
            Some("sha256"),
            Some("abc123"),
            true,
        ));
        assert!(!external_plugin_installation_is_supported_shape(
            Some(&[1, 2, 3]),
            "zstd",
            Some("blake3"),
            Some(""),
            true,
        ));
        assert!(!external_plugin_installation_is_supported_shape(
            Some(&[1, 2, 3]),
            "zstd",
            Some("blake3"),
            Some("abc123"),
            false,
        ));
    }

    fn sample_title() -> Title {
        Title {
            id: "title-1".to_string(),
            library_id: "library-1".to_string(),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            name: "Example".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec!["tag".to_string()],
            canonical_tags: vec![],
            external_ids: vec![ExternalId::new("tvdb".to_string(), "123".to_string())],
            created_by: None,
            created_at: parse_time("2026-01-01T00:00:00Z"),
            year: Some(2026),
            overview: Some("Overview".to_string()),
            poster_url: Some("https://example.com/poster.jpg".to_string()),
            poster_source_url: None,
            background_url: Some("https://example.com/background.jpg".to_string()),
            background_source_url: None,
            sort_title: Some("Example".to_string()),
            catalog_sort_key: String::new(),
            slug: Some("example".to_string()),
            imdb_id: Some("tt123".to_string()),
            runtime_minutes: Some(120),
            popularity: None,
            content_status: Some("released".to_string()),
            language: Some("en".to_string()),
            first_aired: Some("2026-01-01".to_string()),
            network: None,
            studio: None,
            country: None,
            aliases: vec!["Alias".to_string()],
            tagged_aliases: Vec::new(),
            metadata_language: Some("en".to_string()),
            metadata_fetched_at: Some(parse_time("2026-01-02T00:00:00Z")),
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn parse_time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }
}
