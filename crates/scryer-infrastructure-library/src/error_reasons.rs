//! Stable reasons for conditions raised at this infrastructure boundary.

/// artwork worker count must be two or four
pub const MEDIA_ARTWORK_WORKER_COUNT_INVALID: &str = "MEDIA_ARTWORK_WORKER_COUNT_INVALID";

/// batch size must be greater than 0
pub const MEDIA_SCAN_BATCH_SIZE_INVALID: &str = "MEDIA_SCAN_BATCH_SIZE_INVALID";

/// cached image is not a JPEG
pub const MEDIA_CACHED_IMAGE_NOT_JPEG: &str = "MEDIA_CACHED_IMAGE_NOT_JPEG";

/// cached JPEG encoder returned invalid AVIF
pub const MEDIA_CACHED_AVIF_INVALID: &str = "MEDIA_CACHED_AVIF_INVALID";

/// cached JPEG exceeds source size limit
pub const MEDIA_CACHED_JPEG_TOO_LARGE: &str = "MEDIA_CACHED_JPEG_TOO_LARGE";

/// changing a title facet is not supported because titles cannot move between libraries
pub const MEDIA_TITLE_FACET_CHANGE_UNSUPPORTED: &str = "MEDIA_TITLE_FACET_CHANGE_UNSUPPORTED";

/// claim extension must be in the future
pub const MEDIA_CLAIM_EXTENSION_NOT_FUTURE: &str = "MEDIA_CLAIM_EXTENSION_NOT_FUTURE";

/// claim extension must lengthen a live finite lease
pub const MEDIA_CLAIM_EXTENSION_NOT_LONGER: &str = "MEDIA_CLAIM_EXTENSION_NOT_LONGER";

/// collection {collection_id}
pub const MEDIA_COLLECTION_NOT_FOUND: &str = "MEDIA_COLLECTION_NOT_FOUND";

/// disc episode belongs to a different title or no longer exists
pub const MEDIA_DISC_EPISODE_TITLE_MISMATCH: &str = "MEDIA_DISC_EPISODE_TITLE_MISMATCH";

/// only a dismissed media request can be reopened
pub const MEDIA_REQUEST_NOT_DISMISSED: &str = "MEDIA_REQUEST_NOT_DISMISSED";

/// duplicate disc episode mapping
pub const MEDIA_DISC_EPISODE_MAPPING_DUPLICATE: &str = "MEDIA_DISC_EPISODE_MAPPING_DUPLICATE";

/// episode {episode_id}
pub const MEDIA_EPISODE_NOT_FOUND: &str = "MEDIA_EPISODE_NOT_FOUND";

/// external ids already map to multiple titles
pub const MEDIA_EXTERNAL_ID_AMBIGUOUS: &str = "MEDIA_EXTERNAL_ID_AMBIGUOUS";

/// failed to decode cached JPEG: {error}
pub const MEDIA_CACHED_JPEG_DECODE_FAILED: &str = "MEDIA_CACHED_JPEG_DECODE_FAILED";

/// failed to decode image: {err}
pub const MEDIA_IMAGE_DECODE_FAILED: &str = "MEDIA_IMAGE_DECODE_FAILED";

/// failed to detect image format: {err}
pub const MEDIA_IMAGE_FORMAT_DETECTION_FAILED: &str = "MEDIA_IMAGE_FORMAT_DETECTION_FAILED";

/// image dimensions must be non-zero
pub const MEDIA_IMAGE_DIMENSIONS_EMPTY: &str = "MEDIA_IMAGE_DIMENSIONS_EMPTY";

/// image proxy response bytes do not match its image content type
pub const MEDIA_IMAGE_PROXY_CONTENT_TYPE_MISMATCH: &str = "MEDIA_IMAGE_PROXY_CONTENT_TYPE_MISMATCH";

/// image proxy response is too large
pub const MEDIA_IMAGE_PROXY_RESPONSE_TOO_LARGE: &str = "MEDIA_IMAGE_PROXY_RESPONSE_TOO_LARGE";

/// invalid cached JPEG size or target width
pub const MEDIA_CACHED_JPEG_DIMENSIONS_INVALID: &str = "MEDIA_CACHED_JPEG_DIMENSIONS_INVALID";

/// invalid or incomplete disc episode mapping
pub const MEDIA_DISC_EPISODE_MAPPING_INVALID: &str = "MEDIA_DISC_EPISODE_MAPPING_INVALID";

/// Invalid recovery operation
pub const MEDIA_RECOVERY_OPERATION_INVALID: &str = "MEDIA_RECOVERY_OPERATION_INVALID";

/// invalid title image URL: {error}
pub const MEDIA_TITLE_IMAGE_URL_INVALID: &str = "MEDIA_TITLE_IMAGE_URL_INVALID";

/// at least one collection field must be provided
pub const MEDIA_COLLECTION_UPDATE_EMPTY: &str = "MEDIA_COLLECTION_UPDATE_EMPTY";

/// at least one episode field must be provided
pub const MEDIA_EPISODE_UPDATE_EMPTY: &str = "MEDIA_EPISODE_UPDATE_EMPTY";

/// at least one title field must be provided
pub const MEDIA_TITLE_UPDATE_EMPTY: &str = "MEDIA_TITLE_UPDATE_EMPTY";

/// library {library_id} does not exist
pub const MEDIA_DESTINATION_LIBRARY_NOT_FOUND: &str = "MEDIA_DESTINATION_LIBRARY_NOT_FOUND";

/// library {} does not exist for facet {}
pub const MEDIA_FACET_LIBRARY_NOT_FOUND: &str = "MEDIA_FACET_LIBRARY_NOT_FOUND";

/// library {library_id}
pub const MEDIA_LIBRARY_NOT_FOUND: &str = "MEDIA_LIBRARY_NOT_FOUND";

/// library path must be a directory
pub const MEDIA_LIBRARY_PATH_NOT_DIRECTORY: &str = "MEDIA_LIBRARY_PATH_NOT_DIRECTORY";

/// library path error: {err}
pub const MEDIA_LIBRARY_PATH_UNAVAILABLE: &str = "MEDIA_LIBRARY_PATH_UNAVAILABLE";

/// library root '{path}' is already configured as root {conflicting}
pub const MEDIA_LIBRARY_ROOT_ALREADY_CONFIGURED: &str = "MEDIA_LIBRARY_ROOT_ALREADY_CONFIGURED";

/// library root {root_id}
pub const MEDIA_LIBRARY_ROOT_NOT_FOUND: &str = "MEDIA_LIBRARY_ROOT_NOT_FOUND";

/// a library root path cannot be empty
pub const MEDIA_LIBRARY_ROOT_PATH_EMPTY: &str = "MEDIA_LIBRARY_ROOT_PATH_EMPTY";

/// library root cannot be removed while titles reference it
pub const MEDIA_LIBRARY_ROOT_REFERENCED: &str = "MEDIA_LIBRARY_ROOT_REFERENCED";

/// lifecycle claim is no longer live
pub const MEDIA_LIFECYCLE_CLAIM_NOT_LIVE: &str = "MEDIA_LIFECYCLE_CLAIM_NOT_LIVE";

/// list account {id}
pub const MEDIA_LIST_ACCOUNT_NOT_FOUND: &str = "MEDIA_LIST_ACCOUNT_NOT_FOUND";

/// list exclusion {id}
pub const MEDIA_LIST_EXCLUSION_NOT_FOUND: &str = "MEDIA_LIST_EXCLUSION_NOT_FOUND";

/// list movie selection is missing
pub const MEDIA_LIST_MOVIE_SELECTION_MISSING: &str = "MEDIA_LIST_MOVIE_SELECTION_MISSING";

/// list subscription {id}
pub const MEDIA_LIST_SUBSCRIPTION_NOT_FOUND: &str = "MEDIA_LIST_SUBSCRIPTION_NOT_FOUND";

/// location operation {operation_id}
pub const MEDIA_LOCATION_OPERATION_NOT_FOUND: &str = "MEDIA_LOCATION_OPERATION_NOT_FOUND";

/// Media changed during recovery
pub const MEDIA_RECOVERY_MEDIA_CHANGED: &str = "MEDIA_RECOVERY_MEDIA_CHANGED";

/// media file ownership changed
pub const MEDIA_FILE_OWNERSHIP_CHANGED: &str = "MEDIA_FILE_OWNERSHIP_CHANGED";

/// media request {request_id}
pub const MEDIA_REQUEST_NOT_FOUND: &str = "MEDIA_REQUEST_NOT_FOUND";

/// media request is no longer pending
pub const MEDIA_REQUEST_STATE_NOT_PENDING: &str = "MEDIA_REQUEST_STATE_NOT_PENDING";

/// media server connection {id}
pub const MEDIA_SERVER_CONNECTION_NOT_FOUND: &str = "MEDIA_SERVER_CONNECTION_NOT_FOUND";

/// Missing recovery root
pub const MEDIA_RECOVERY_ROOT_MISSING: &str = "MEDIA_RECOVERY_ROOT_MISSING";

/// pending import {pending_import_id} could not be resolved for title {}
pub const MEDIA_PENDING_IMPORT_UNRESOLVED: &str = "MEDIA_PENDING_IMPORT_UNRESOLVED";

/// playback mapping connection ID does not match persistence target
pub const MEDIA_PLAYBACK_CONNECTION_MISMATCH: &str = "MEDIA_PLAYBACK_CONNECTION_MISMATCH";

/// Recovery canceled before catalog commit; original mappings preserved
pub const MEDIA_RECOVERY_CANCELED_BEFORE_COMMIT: &str = "MEDIA_RECOVERY_CANCELED_BEFORE_COMMIT";

/// Recovery catalog evidence limit exceeded; no changes made
pub const MEDIA_RECOVERY_CATALOG_EVIDENCE_LIMIT_EXCEEDED: &str =
    "MEDIA_RECOVERY_CATALOG_EVIDENCE_LIMIT_EXCEEDED";

/// Recovery file has no durable verification receipt
pub const MEDIA_RECOVERY_VERIFICATION_RECEIPT_REQUIRED: &str =
    "MEDIA_RECOVERY_VERIFICATION_RECEIPT_REQUIRED";

/// Recovery identity changed
pub const MEDIA_RECOVERY_IDENTITY_CHANGED: &str = "MEDIA_RECOVERY_IDENTITY_CHANGED";

/// Recovery operation no longer exists
pub const MEDIA_RECOVERY_OPERATION_NOT_FOUND: &str = "MEDIA_RECOVERY_OPERATION_NOT_FOUND";

/// Recovery plan changed or remains unresolved
pub const MEDIA_RECOVERY_PLAN_UNRESOLVED: &str = "MEDIA_RECOVERY_PLAN_UNRESOLVED";

/// Recovery requires matching full verification receipts
pub const MEDIA_RECOVERY_VERIFICATION_MISMATCH: &str = "MEDIA_RECOVERY_VERIFICATION_MISMATCH";

/// Recovery staging evidence limit exceeded
pub const MEDIA_RECOVERY_STAGING_EVIDENCE_LIMIT_EXCEEDED: &str =
    "MEDIA_RECOVERY_STAGING_EVIDENCE_LIMIT_EXCEEDED";

/// Recovery cannot be replanned in its current state
pub const MEDIA_RECOVERY_NOT_REPLANNABLE: &str = "MEDIA_RECOVERY_NOT_REPLANNABLE";

/// rename target path is required for move actions
pub const MEDIA_RENAME_TARGET_PATH_REQUIRED: &str = "MEDIA_RENAME_TARGET_PATH_REQUIRED";

/// request is no longer pending
pub const MEDIA_REQUEST_NOT_PENDING: &str = "MEDIA_REQUEST_NOT_PENDING";

/// request title changed during approval; retry
pub const MEDIA_REQUEST_APPROVAL_TITLE_CHANGED: &str = "MEDIA_REQUEST_APPROVAL_TITLE_CHANGED";

/// selected movie has no supported identity
pub const MEDIA_MOVIE_IDENTITY_UNSUPPORTED: &str = "MEDIA_MOVIE_IDENTITY_UNSUPPORTED";

/// Selected root changed
pub const MEDIA_SELECTED_ROOT_CHANGED: &str = "MEDIA_SELECTED_ROOT_CHANGED";

/// series movie link {link_id}
pub const MEDIA_SERIES_MOVIE_LINK_NOT_FOUND: &str = "MEDIA_SERIES_MOVIE_LINK_NOT_FOUND";

/// slug '{slug}' maps to multiple titles
pub const MEDIA_TITLE_SLUG_AMBIGUOUS: &str = "MEDIA_TITLE_SLUG_AMBIGUOUS";

/// a title can carry at most {MAX_USER_TAGS_PER_TITLE} tags; this change would leave it with {user_tag_count}
pub const MEDIA_TITLE_TAG_LIMIT_EXCEEDED: &str = "MEDIA_TITLE_TAG_LIMIT_EXCEEDED";

/// title image exceeds max size of {} bytes
pub const MEDIA_TITLE_IMAGE_TOO_LARGE: &str = "MEDIA_TITLE_IMAGE_TOO_LARGE";

/// title image redirects are not allowed
pub const MEDIA_TITLE_IMAGE_REDIRECT_FORBIDDEN: &str = "MEDIA_TITLE_IMAGE_REDIRECT_FORBIDDEN";

/// title image URL must not be empty
pub const MEDIA_TITLE_IMAGE_URL_EMPTY: &str = "MEDIA_TITLE_IMAGE_URL_EMPTY";

/// title image URL must not include credentials
pub const MEDIA_TITLE_IMAGE_URL_CREDENTIALS_FORBIDDEN: &str =
    "MEDIA_TITLE_IMAGE_URL_CREDENTIALS_FORBIDDEN";

/// title image URL must use http or https
pub const MEDIA_TITLE_IMAGE_URL_SCHEME_UNSUPPORTED: &str =
    "MEDIA_TITLE_IMAGE_URL_SCHEME_UNSUPPORTED";

/// title image URL must include a host
pub const MEDIA_TITLE_IMAGE_URL_HOST_REQUIRED: &str = "MEDIA_TITLE_IMAGE_URL_HOST_REQUIRED";

/// title name cannot be empty
pub const MEDIA_TITLE_NAME_EMPTY: &str = "MEDIA_TITLE_NAME_EMPTY";

/// Title no longer exists
pub const MEDIA_RECOVERY_TITLE_NOT_FOUND: &str = "MEDIA_RECOVERY_TITLE_NOT_FOUND";

/// title tag {id}
pub const MEDIA_TITLE_TAG_NOT_FOUND: &str = "MEDIA_TITLE_TAG_NOT_FOUND";

/// a transfer needs both a destination library and a destination root
pub const MEDIA_TRANSFER_DESTINATION_REQUIRED: &str = "MEDIA_TRANSFER_DESTINATION_REQUIRED";

/// unapproved image proxy source
pub const MEDIA_IMAGE_PROXY_SOURCE_UNAPPROVED: &str = "MEDIA_IMAGE_PROXY_SOURCE_UNAPPROVED";

/// unsupported cached JPEG kind
pub const MEDIA_CACHED_JPEG_KIND_UNSUPPORTED: &str = "MEDIA_CACHED_JPEG_KIND_UNSUPPORTED";

/// unsupported image format
pub const MEDIA_IMAGE_FORMAT_UNSUPPORTED: &str = "MEDIA_IMAGE_FORMAT_UNSUPPORTED";

/// unsupported image proxy content type
pub const MEDIA_IMAGE_PROXY_CONTENT_TYPE_UNSUPPORTED: &str =
    "MEDIA_IMAGE_PROXY_CONTENT_TYPE_UNSUPPORTED";

/// unsupported title image content type: {content_type}
pub const MEDIA_TITLE_IMAGE_CONTENT_TYPE_UNSUPPORTED: &str =
    "MEDIA_TITLE_IMAGE_CONTENT_TYPE_UNSUPPORTED";

/// title {id}
pub const TITLE_NOT_FOUND: &str = "TITLE_NOT_FOUND";

/// The image source failed public HTTP URL validation.
pub const MEDIA_IMAGE_SOURCE_REJECTED: &str = "MEDIA_IMAGE_SOURCE_REJECTED";

/// The selected titles cannot be merged.
pub const MEDIA_TITLE_MERGE_BLOCKED: &str = "MEDIA_TITLE_MERGE_BLOCKED";

/// The title image host is not an approved artwork provider.
pub const MEDIA_TITLE_IMAGE_HOST_UNAPPROVED: &str = "MEDIA_TITLE_IMAGE_HOST_UNAPPROVED";

/// A title tag with the normalized label already exists.
pub const TAG_ALREADY_EXISTS: &str = "TAG_ALREADY_EXISTS";

#[cfg(test)]
const ALL: &[&str] = &[
    TAG_ALREADY_EXISTS,
    MEDIA_IMAGE_SOURCE_REJECTED,
    MEDIA_TITLE_MERGE_BLOCKED,
    MEDIA_TITLE_IMAGE_HOST_UNAPPROVED,
    MEDIA_ARTWORK_WORKER_COUNT_INVALID,
    MEDIA_SCAN_BATCH_SIZE_INVALID,
    MEDIA_CACHED_IMAGE_NOT_JPEG,
    MEDIA_CACHED_AVIF_INVALID,
    MEDIA_CACHED_JPEG_TOO_LARGE,
    MEDIA_TITLE_FACET_CHANGE_UNSUPPORTED,
    MEDIA_CLAIM_EXTENSION_NOT_FUTURE,
    MEDIA_CLAIM_EXTENSION_NOT_LONGER,
    MEDIA_COLLECTION_NOT_FOUND,
    MEDIA_DISC_EPISODE_TITLE_MISMATCH,
    MEDIA_REQUEST_NOT_DISMISSED,
    MEDIA_DISC_EPISODE_MAPPING_DUPLICATE,
    MEDIA_EPISODE_NOT_FOUND,
    MEDIA_EXTERNAL_ID_AMBIGUOUS,
    MEDIA_CACHED_JPEG_DECODE_FAILED,
    MEDIA_IMAGE_DECODE_FAILED,
    MEDIA_IMAGE_FORMAT_DETECTION_FAILED,
    MEDIA_IMAGE_DIMENSIONS_EMPTY,
    MEDIA_IMAGE_PROXY_CONTENT_TYPE_MISMATCH,
    MEDIA_IMAGE_PROXY_RESPONSE_TOO_LARGE,
    MEDIA_CACHED_JPEG_DIMENSIONS_INVALID,
    MEDIA_DISC_EPISODE_MAPPING_INVALID,
    MEDIA_RECOVERY_OPERATION_INVALID,
    MEDIA_TITLE_IMAGE_URL_INVALID,
    MEDIA_COLLECTION_UPDATE_EMPTY,
    MEDIA_EPISODE_UPDATE_EMPTY,
    MEDIA_TITLE_UPDATE_EMPTY,
    MEDIA_DESTINATION_LIBRARY_NOT_FOUND,
    MEDIA_FACET_LIBRARY_NOT_FOUND,
    MEDIA_LIBRARY_NOT_FOUND,
    MEDIA_LIBRARY_PATH_NOT_DIRECTORY,
    MEDIA_LIBRARY_PATH_UNAVAILABLE,
    MEDIA_LIBRARY_ROOT_ALREADY_CONFIGURED,
    MEDIA_LIBRARY_ROOT_NOT_FOUND,
    MEDIA_LIBRARY_ROOT_PATH_EMPTY,
    MEDIA_LIBRARY_ROOT_REFERENCED,
    MEDIA_LIFECYCLE_CLAIM_NOT_LIVE,
    MEDIA_LIST_ACCOUNT_NOT_FOUND,
    MEDIA_LIST_EXCLUSION_NOT_FOUND,
    MEDIA_LIST_MOVIE_SELECTION_MISSING,
    MEDIA_LIST_SUBSCRIPTION_NOT_FOUND,
    MEDIA_LOCATION_OPERATION_NOT_FOUND,
    MEDIA_RECOVERY_MEDIA_CHANGED,
    MEDIA_FILE_OWNERSHIP_CHANGED,
    MEDIA_REQUEST_NOT_FOUND,
    MEDIA_REQUEST_STATE_NOT_PENDING,
    MEDIA_SERVER_CONNECTION_NOT_FOUND,
    MEDIA_RECOVERY_ROOT_MISSING,
    MEDIA_PENDING_IMPORT_UNRESOLVED,
    MEDIA_PLAYBACK_CONNECTION_MISMATCH,
    MEDIA_RECOVERY_CANCELED_BEFORE_COMMIT,
    MEDIA_RECOVERY_CATALOG_EVIDENCE_LIMIT_EXCEEDED,
    MEDIA_RECOVERY_VERIFICATION_RECEIPT_REQUIRED,
    MEDIA_RECOVERY_IDENTITY_CHANGED,
    MEDIA_RECOVERY_OPERATION_NOT_FOUND,
    MEDIA_RECOVERY_PLAN_UNRESOLVED,
    MEDIA_RECOVERY_VERIFICATION_MISMATCH,
    MEDIA_RECOVERY_STAGING_EVIDENCE_LIMIT_EXCEEDED,
    MEDIA_RECOVERY_NOT_REPLANNABLE,
    MEDIA_RENAME_TARGET_PATH_REQUIRED,
    MEDIA_REQUEST_NOT_PENDING,
    MEDIA_REQUEST_APPROVAL_TITLE_CHANGED,
    MEDIA_MOVIE_IDENTITY_UNSUPPORTED,
    MEDIA_SELECTED_ROOT_CHANGED,
    MEDIA_SERIES_MOVIE_LINK_NOT_FOUND,
    MEDIA_TITLE_SLUG_AMBIGUOUS,
    MEDIA_TITLE_TAG_LIMIT_EXCEEDED,
    MEDIA_TITLE_IMAGE_TOO_LARGE,
    MEDIA_TITLE_IMAGE_REDIRECT_FORBIDDEN,
    MEDIA_TITLE_IMAGE_URL_EMPTY,
    MEDIA_TITLE_IMAGE_URL_CREDENTIALS_FORBIDDEN,
    MEDIA_TITLE_IMAGE_URL_SCHEME_UNSUPPORTED,
    MEDIA_TITLE_IMAGE_URL_HOST_REQUIRED,
    MEDIA_TITLE_NAME_EMPTY,
    MEDIA_RECOVERY_TITLE_NOT_FOUND,
    MEDIA_TITLE_TAG_NOT_FOUND,
    MEDIA_TRANSFER_DESTINATION_REQUIRED,
    MEDIA_IMAGE_PROXY_SOURCE_UNAPPROVED,
    MEDIA_CACHED_JPEG_KIND_UNSUPPORTED,
    MEDIA_IMAGE_FORMAT_UNSUPPORTED,
    MEDIA_IMAGE_PROXY_CONTENT_TYPE_UNSUPPORTED,
    MEDIA_TITLE_IMAGE_CONTENT_TYPE_UNSUPPORTED,
    TITLE_NOT_FOUND,
];

#[cfg(test)]
mod tests {
    #[test]
    fn reason_codes_are_distinct_and_stable_format() {
        let mut seen = std::collections::HashSet::new();
        for code in super::ALL {
            assert!(seen.insert(*code), "duplicate reason: {code}");
            assert!(
                ["MEDIA_", "TAG_", "TITLE_"]
                    .iter()
                    .any(|prefix| code.starts_with(prefix))
            );
            assert!(
                code.bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            );
        }
    }
}
