//! Stable reasons for conditions raised at this infrastructure boundary.

/// archive plugin path '{}' is not a safe relative path
pub const PLUGIN_ARCHIVE_PATH_UNSAFE: &str = "PLUGIN_ARCHIVE_PATH_UNSAFE";

/// archive plugin path '{}' is outside allowed root '{}'
pub const PLUGIN_ARCHIVE_PATH_OUTSIDE_ROOT: &str = "PLUGIN_ARCHIVE_PATH_OUTSIDE_ROOT";

/// indexer_host_not_allowed: Host {host} is missing permission for this indexer instance
pub const PLUGIN_INDEXER_HOST_FORBIDDEN: &str = "PLUGIN_INDEXER_HOST_FORBIDDEN";

/// Initialized component descriptor differs from embedded metadata
pub const PLUGIN_COMPONENT_DESCRIPTOR_MISMATCH: &str = "PLUGIN_COMPONENT_DESCRIPTOR_MISMATCH";

/// list provider {} reads only a member's own lists and needs that member's linked account
pub const PLUGIN_LIST_LINKED_ACCOUNT_REQUIRED: &str = "PLUGIN_LIST_LINKED_ACCOUNT_REQUIRED";

/// Multiple enabled archive extraction plugins support this format. Disable one in Settings → Plugins.
pub const PLUGIN_ARCHIVE_EXTRACTOR_CONFLICT: &str = "PLUGIN_ARCHIVE_EXTRACTOR_CONFLICT";

/// network_endpoint_resolution_failed: Cannot resolve this indexer's configured endpoints. Check the selected definition and its endpoint settings; authorities must be HTTP(S) origins available from configuration.
pub const PLUGIN_NETWORK_ENDPOINT_RESOLUTION_FAILED: &str =
    "PLUGIN_NETWORK_ENDPOINT_RESOLUTION_FAILED";

/// subtitle generator input exceeds duration limit ({} > {})
pub const PLUGIN_SUBTITLE_INPUT_TOO_LONG: &str = "PLUGIN_SUBTITLE_INPUT_TOO_LONG";

/// subtitle generator input exceeds size limit ({} > {})
pub const PLUGIN_SUBTITLE_INPUT_TOO_LARGE: &str = "PLUGIN_SUBTITLE_INPUT_TOO_LARGE";

/// subtitle generator input path '{}' has no file name
pub const PLUGIN_SUBTITLE_INPUT_FILENAME_REQUIRED: &str = "PLUGIN_SUBTITLE_INPUT_FILENAME_REQUIRED";

/// subtitle provider config_json must be a JSON object: {error}
pub const PLUGIN_SUBTITLE_CONFIG_NOT_OBJECT: &str = "PLUGIN_SUBTITLE_CONFIG_NOT_OBJECT";

/// subtitle provider '{}' is missing required host bindings: {binding_names}
pub const PLUGIN_SUBTITLE_HOST_BINDING_REQUIRED: &str = "PLUGIN_SUBTITLE_HOST_BINDING_REQUIRED";

/// subtitle sync path '{}' has no file name
pub const PLUGIN_SUBTITLE_SYNC_FILENAME_REQUIRED: &str = "PLUGIN_SUBTITLE_SYNC_FILENAME_REQUIRED";

/// subtitle sync plugin output escapes the output directory: {}
pub const PLUGIN_SUBTITLE_SYNC_OUTPUT_OUTSIDE_DIRECTORY: &str =
    "PLUGIN_SUBTITLE_SYNC_OUTPUT_OUTSIDE_DIRECTORY";

/// subtitle sync plugin output exceeds {MAX_REWRITTEN_SUBTITLE_BYTES} bytes
pub const PLUGIN_SUBTITLE_SYNC_OUTPUT_TOO_LARGE: &str = "PLUGIN_SUBTITLE_SYNC_OUTPUT_TOO_LARGE";

/// subtitle sync plugin output is not a regular file: {}
pub const PLUGIN_SUBTITLE_SYNC_OUTPUT_NOT_FILE: &str = "PLUGIN_SUBTITLE_SYNC_OUTPUT_NOT_FILE";

/// subtitle sync plugin returned unexpected rewritten subtitle path '{}'
pub const PLUGIN_SUBTITLE_SYNC_OUTPUT_PATH_MISMATCH: &str =
    "PLUGIN_SUBTITLE_SYNC_OUTPUT_PATH_MISMATCH";

/// The selected Newznab provider profile is invalid.
pub const PLUGIN_NEWZNAB_PROFILE_INVALID: &str = "PLUGIN_NEWZNAB_PROFILE_INVALID";

#[cfg(test)]
const ALL: &[&str] = &[
    PLUGIN_NEWZNAB_PROFILE_INVALID,
    PLUGIN_ARCHIVE_PATH_UNSAFE,
    PLUGIN_ARCHIVE_PATH_OUTSIDE_ROOT,
    PLUGIN_INDEXER_HOST_FORBIDDEN,
    PLUGIN_COMPONENT_DESCRIPTOR_MISMATCH,
    PLUGIN_LIST_LINKED_ACCOUNT_REQUIRED,
    PLUGIN_ARCHIVE_EXTRACTOR_CONFLICT,
    PLUGIN_NETWORK_ENDPOINT_RESOLUTION_FAILED,
    PLUGIN_SUBTITLE_INPUT_TOO_LONG,
    PLUGIN_SUBTITLE_INPUT_TOO_LARGE,
    PLUGIN_SUBTITLE_INPUT_FILENAME_REQUIRED,
    PLUGIN_SUBTITLE_CONFIG_NOT_OBJECT,
    PLUGIN_SUBTITLE_HOST_BINDING_REQUIRED,
    PLUGIN_SUBTITLE_SYNC_FILENAME_REQUIRED,
    PLUGIN_SUBTITLE_SYNC_OUTPUT_OUTSIDE_DIRECTORY,
    PLUGIN_SUBTITLE_SYNC_OUTPUT_TOO_LARGE,
    PLUGIN_SUBTITLE_SYNC_OUTPUT_NOT_FILE,
    PLUGIN_SUBTITLE_SYNC_OUTPUT_PATH_MISMATCH,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(["PLUGIN_"].iter().any(|prefix| code.starts_with(prefix)));
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
