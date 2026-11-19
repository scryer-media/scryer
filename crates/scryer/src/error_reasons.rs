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

/// Downloading diagnostics needs a signed-in, fully authenticated session or an API key.
pub const HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED: &str =
    "HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED";

/// Downloading diagnostics needs the system settings permission.
pub const HTTP_DIAGNOSTICS_PERMISSION_DENIED: &str = "HTTP_DIAGNOSTICS_PERMISSION_DENIED";

/// The diagnostics bundle could not be built.
pub const HTTP_DIAGNOSTICS_BUNDLE_FAILED: &str = "HTTP_DIAGNOSTICS_BUNDLE_FAILED";

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

/// The MCP endpoint was sent an Origin it does not accept.
pub const HTTP_MCP_ORIGIN_NOT_ALLOWED: &str = "HTTP_MCP_ORIGIN_NOT_ALLOWED";

/// The MCP endpoint is turned off in the security settings.
pub const HTTP_MCP_DISABLED: &str = "HTTP_MCP_DISABLED";

/// The MCP endpoint accepts only an API key, and none was offered: no
/// credentials, a session token or an OAuth access token.
pub const HTTP_MCP_API_KEY_REQUIRED: &str = "HTTP_MCP_API_KEY_REQUIRED";

/// The API key offered to the MCP endpoint is unknown, expired or revoked.
pub const HTTP_MCP_API_KEY_INVALID: &str = "HTTP_MCP_API_KEY_INVALID";

/// The MCP request body is larger than the endpoint accepts.
pub const HTTP_MCP_REQUEST_TOO_LARGE: &str = "HTTP_MCP_REQUEST_TOO_LARGE";

/// The MCP request body could not be read.
pub const HTTP_MCP_REQUEST_BODY_UNREADABLE: &str = "HTTP_MCP_REQUEST_BODY_UNREADABLE";

/// A GraphQL error behind a tool carried no code of its own, such as a parse
/// or validation failure.
pub const HTTP_MCP_GRAPHQL_ERROR: &str = "HTTP_MCP_GRAPHQL_ERROR";

/// A tool call named a tool the endpoint does not serve.
pub const HTTP_MCP_TOOL_UNKNOWN: &str = "HTTP_MCP_TOOL_UNKNOWN";

/// A tool call's arguments are unknown, of the wrong type or out of range.
pub const HTTP_MCP_ARGUMENTS_INVALID: &str = "HTTP_MCP_ARGUMENTS_INVALID";

/// A log search pattern is not a valid regular expression, or compiles past
/// the size limits.
pub const HTTP_MCP_LOG_PATTERN_INVALID: &str = "HTTP_MCP_LOG_PATTERN_INVALID";

/// A diagnostics tool could not gather what it reports.
pub const HTTP_MCP_DIAGNOSTICS_FAILED: &str = "HTTP_MCP_DIAGNOSTICS_FAILED";

/// A run tool named an operation the catalog does not hold.
pub const HTTP_MCP_OPERATION_UNKNOWN: &str = "HTTP_MCP_OPERATION_UNKNOWN";

/// An operation name differs between skills and the call named no skill.
pub const HTTP_MCP_OPERATION_AMBIGUOUS: &str = "HTTP_MCP_OPERATION_AMBIGUOUS";

/// A query was sent to the mutation tool, or a mutation to the query tool.
pub const HTTP_MCP_OPERATION_KIND_MISMATCH: &str = "HTTP_MCP_OPERATION_KIND_MISMATCH";

/// An operation's result is larger than one tool result may carry.
pub const HTTP_MCP_RESULT_TOO_LARGE: &str = "HTTP_MCP_RESULT_TOO_LARGE";

/// The served schema could not be read to describe an operation.
pub const HTTP_MCP_SCHEMA_UNREADABLE: &str = "HTTP_MCP_SCHEMA_UNREADABLE";

/// The operation list was filtered by a skill that has no served operation.
pub const HTTP_MCP_SKILL_UNKNOWN: &str = "HTTP_MCP_SKILL_UNKNOWN";

/// A mutation's `confirm_token` is not one this key holds: never issued,
/// expired, already used or another key's.
pub const HTTP_MCP_CONFIRMATION_UNKNOWN: &str = "HTTP_MCP_CONFIRMATION_UNKNOWN";

/// A mutation's `confirm_token` was issued for a different operation or
/// variables; it is spent.
pub const HTTP_MCP_CONFIRMATION_MISMATCH: &str = "HTTP_MCP_CONFIRMATION_MISMATCH";

#[cfg(test)]
const ALL: &[&str] = &[
    HTTP_MCP_ORIGIN_NOT_ALLOWED,
    HTTP_MCP_CONFIRMATION_UNKNOWN,
    HTTP_MCP_CONFIRMATION_MISMATCH,
    HTTP_MCP_DISABLED,
    HTTP_MCP_API_KEY_REQUIRED,
    HTTP_MCP_API_KEY_INVALID,
    HTTP_MCP_REQUEST_TOO_LARGE,
    HTTP_MCP_REQUEST_BODY_UNREADABLE,
    HTTP_MCP_GRAPHQL_ERROR,
    HTTP_MCP_TOOL_UNKNOWN,
    HTTP_MCP_ARGUMENTS_INVALID,
    HTTP_MCP_LOG_PATTERN_INVALID,
    HTTP_MCP_DIAGNOSTICS_FAILED,
    HTTP_MCP_OPERATION_UNKNOWN,
    HTTP_MCP_OPERATION_AMBIGUOUS,
    HTTP_MCP_OPERATION_KIND_MISMATCH,
    HTTP_MCP_RESULT_TOO_LARGE,
    HTTP_MCP_SCHEMA_UNREADABLE,
    HTTP_MCP_SKILL_UNKNOWN,
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
    HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED,
    HTTP_DIAGNOSTICS_PERMISSION_DENIED,
    HTTP_DIAGNOSTICS_BUNDLE_FAILED,
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
