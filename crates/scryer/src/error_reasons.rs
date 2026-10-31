//! Stable reasons for conditions raised at this infrastructure boundary.

/// API explorer is disabled
pub const HTTP_API_EXPLORER_DISABLED: &str = "HTTP_API_EXPLORER_DISABLED";

/// API explorer requires a web session
pub const HTTP_API_EXPLORER_WEB_SESSION_REQUIRED: &str = "HTTP_API_EXPLORER_WEB_SESSION_REQUIRED";

/// API explorer session changed
pub const HTTP_API_EXPLORER_SESSION_CHANGED: &str = "HTTP_API_EXPLORER_SESSION_CHANGED";

/// invalid authorization header
pub const HTTP_AUTHORIZATION_HEADER_INVALID: &str = "HTTP_AUTHORIZATION_HEADER_INVALID";

/// backup bundle not found: {error}
pub const HTTP_BACKUP_BUNDLE_NOT_FOUND: &str = "HTTP_BACKUP_BUNDLE_NOT_FOUND";

/// backup download authorization is invalid
pub const HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID: &str =
    "HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID";

/// backup download authorization is required
pub const HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_REQUIRED: &str =
    "HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_REQUIRED";

/// backup download ticket query is not accepted
pub const HTTP_BACKUP_TICKET_QUERY_FORBIDDEN: &str = "HTTP_BACKUP_TICKET_QUERY_FORBIDDEN";

/// backup metadata not found: {error}
pub const HTTP_BACKUP_METADATA_NOT_FOUND: &str = "HTTP_BACKUP_METADATA_NOT_FOUND";

/// Duplicate API explorer access mode
pub const HTTP_API_EXPLORER_MODE_DUPLICATE: &str = "HTTP_API_EXPLORER_MODE_DUPLICATE";

/// Invalid API explorer access mode
pub const HTTP_API_EXPLORER_MODE_INVALID: &str = "HTTP_API_EXPLORER_MODE_INVALID";

/// invalid backup filename
pub const HTTP_BACKUP_FILENAME_INVALID: &str = "HTTP_BACKUP_FILENAME_INVALID";

/// invalid release download request: {error}
pub const HTTP_RELEASE_DOWNLOAD_REQUEST_INVALID: &str = "HTTP_RELEASE_DOWNLOAD_REQUEST_INVALID";

/// API keys are not supported for WebSocket connections.
pub const HTTP_WEBSOCKET_API_KEY_UNSUPPORTED: &str = "HTTP_WEBSOCKET_API_KEY_UNSUPPORTED";

/// Scryer web client proof is required for unauthenticated WebSocket access.
pub const HTTP_WEBSOCKET_CLIENT_PROOF_REQUIRED: &str = "HTTP_WEBSOCKET_CLIENT_PROOF_REQUIRED";

#[cfg(test)]
const ALL: &[&str] = &[
    HTTP_WEBSOCKET_API_KEY_UNSUPPORTED,
    HTTP_WEBSOCKET_CLIENT_PROOF_REQUIRED,
    HTTP_API_EXPLORER_DISABLED,
    HTTP_API_EXPLORER_WEB_SESSION_REQUIRED,
    HTTP_API_EXPLORER_SESSION_CHANGED,
    HTTP_AUTHORIZATION_HEADER_INVALID,
    HTTP_BACKUP_BUNDLE_NOT_FOUND,
    HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID,
    HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_REQUIRED,
    HTTP_BACKUP_TICKET_QUERY_FORBIDDEN,
    HTTP_BACKUP_METADATA_NOT_FOUND,
    HTTP_API_EXPLORER_MODE_DUPLICATE,
    HTTP_API_EXPLORER_MODE_INVALID,
    HTTP_BACKUP_FILENAME_INVALID,
    HTTP_RELEASE_DOWNLOAD_REQUEST_INVALID,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(["HTTP_"].iter().any(|prefix| code.starts_with(prefix)));
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
