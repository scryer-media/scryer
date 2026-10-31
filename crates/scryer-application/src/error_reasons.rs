//! Stable conditions owned by application policy. Sentences remain display-only.

pub mod metadata_gateway {
    /// The system clock is outside the metadata gateway's request-signing window.
    pub const CLOCK_SKEW: &str = "METADATA_GATEWAY_CLOCK_SKEW";
    pub const ALL: &[&str] = &[CLOCK_SKEW];
}

pub mod indexer_clients {
    /// Client management requires an authenticated interactive session.
    pub const INTERACTIVE_SESSION_REQUIRED: &str = "INDEXER_CLIENTS_INTERACTIVE_SESSION_REQUIRED";
    /// The acting user does not hold every application permission.
    pub const ALL_PERMISSIONS_REQUIRED: &str = "INDEXER_CLIENTS_ALL_PERMISSIONS_REQUIRED";
    /// A client whose key was revoked cannot be enabled until a key is rotated in.
    pub const ENABLE_REQUIRES_KEY: &str = "INDEXER_CLIENTS_ENABLE_REQUIRES_KEY";
    /// The submitted client key is malformed or does not verify.
    pub const KEY_INVALID: &str = "INDEXER_CLIENTS_KEY_INVALID";
    /// The client repository is unavailable in this runtime.
    pub const UNAVAILABLE: &str = "INDEXER_CLIENTS_UNAVAILABLE";
    /// A management target no longer exists.
    pub const NOT_FOUND: &str = "INDEXER_CLIENTS_NOT_FOUND";
    /// A concurrent management mutation changed the expected revision.
    pub const CONFIGURATION_CHANGED: &str = "INDEXER_CLIENTS_CONFIGURATION_CHANGED";
    /// A label is empty, too long, or contains a control character.
    pub const NAME_INVALID: &str = "INDEXER_CLIENTS_NAME_INVALID";
    /// A grant is duplicated, missing, targets a manager, or has an invalid category.
    pub const GRANT_INVALID: &str = "INDEXER_CLIENTS_GRANT_INVALID";
    /// The persistent revision counter cannot advance further.
    pub const REVISION_EXHAUSTED: &str = "INDEXER_CLIENTS_REVISION_EXHAUSTED";
    /// This client is not granted the requested compatibility protocol.
    pub const PROTOCOL_FORBIDDEN: &str = "INDEXER_CLIENTS_PROTOCOL_FORBIDDEN";
    /// No granted source can honor the complete operation and parameter set.
    pub const SEARCH_UNSUPPORTED: &str = "INDEXER_CLIENTS_SEARCH_UNSUPPORTED";
    /// A compatibility request contains a non-numeric category identifier.
    pub const CATEGORY_INVALID: &str = "INDEXER_CLIENTS_CATEGORY_INVALID";
    /// Source configuration changed while a page was being produced.
    pub const SOURCE_CHANGED: &str = "INDEXER_CLIENTS_SOURCE_CHANGED";
    /// Compatibility parameters cannot be parsed without changing their meaning.
    pub const REQUEST_INVALID: &str = "INDEXER_CLIENTS_REQUEST_INVALID";
    /// More than one credential transport supplied different key values.
    pub const CREDENTIALS_AMBIGUOUS: &str = "INDEXER_CLIENTS_CREDENTIALS_AMBIGUOUS";
    /// The retrieval reference is malformed, unauthenticated or no longer authorized.
    pub const REFERENCE_INVALID: &str = "INDEXER_CLIENTS_REFERENCE_INVALID";
    pub const ALL: &[&str] = &[
        INTERACTIVE_SESSION_REQUIRED,
        ALL_PERMISSIONS_REQUIRED,
        ENABLE_REQUIRES_KEY,
        KEY_INVALID,
        UNAVAILABLE,
        NOT_FOUND,
        CONFIGURATION_CHANGED,
        NAME_INVALID,
        GRANT_INVALID,
        REVISION_EXHAUSTED,
        PROTOCOL_FORBIDDEN,
        SEARCH_UNSUPPORTED,
        CATEGORY_INVALID,
        SOURCE_CHANGED,
        REQUEST_INVALID,
        CREDENTIALS_AMBIGUOUS,
        REFERENCE_INVALID,
    ];

    #[cfg(test)]
    #[test]
    fn reasons_are_distinct_and_area_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for reason in ALL {
            assert!(reason.starts_with("INDEXER_CLIENTS_"));
            assert!(
                reason
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            );
            assert!(seen.insert(reason));
        }
    }
}

pub mod raw_search {
    /// Retaining the complete source page would exceed the shared byte budget.
    pub const RETAINED_STATE_FULL: &str = "RAW_SEARCH_RETAINED_STATE_FULL";
    /// The source returned a continuation already consumed by this generation.
    pub const CURSOR_REPEATED: &str = "RAW_SEARCH_CURSOR_REPEATED";
    /// The source continuation is empty or exceeds the existing cursor byte bound.
    pub const CURSOR_INVALID: &str = "RAW_SEARCH_CURSOR_INVALID";
    /// A requested offset cannot be represented by this server.
    pub const OFFSET_INVALID: &str = "RAW_SEARCH_OFFSET_INVALID";
    /// The owner explicitly cancelled this generation.
    pub const CANCELLED: &str = "RAW_SEARCH_CANCELLED";
    /// The requested earlier page must be rebuilt from upstream sources.
    pub const HISTORY_EXPIRED: &str = "RAW_SEARCH_HISTORY_EXPIRED";
    /// Every eligible source is currently limited by its existing quota rules.
    pub const ALL_SOURCES_LIMITED: &str = "RAW_SEARCH_ALL_SOURCES_LIMITED";
    /// No eligible source produced a successful response.
    pub const NO_SOURCE_SUCCEEDED: &str = "RAW_SEARCH_NO_SOURCE_SUCCEEDED";
    /// The raw search coordinator could not complete the page.
    pub const SEARCH_FAILED: &str = "RAW_SEARCH_FAILED";
    /// A plugin search asked for recent paging and an RSS catch-up marker at
    /// once; the host refuses it before the plugin runs.
    pub const RECENT_PAGING_WITH_RSS_CATCH_UP: &str = "RAW_SEARCH_RECENT_PAGING_WITH_RSS_CATCH_UP";
    pub const ALL: &[&str] = &[
        RETAINED_STATE_FULL,
        CURSOR_REPEATED,
        CURSOR_INVALID,
        OFFSET_INVALID,
        CANCELLED,
        HISTORY_EXPIRED,
        ALL_SOURCES_LIMITED,
        NO_SOURCE_SUCCEEDED,
        SEARCH_FAILED,
        RECENT_PAGING_WITH_RSS_CATCH_UP,
    ];

    #[cfg(test)]
    #[test]
    fn reasons_are_distinct_and_area_prefixed() {
        let mut seen = std::collections::HashSet::new();
        for reason in ALL {
            assert!(reason.starts_with("RAW_SEARCH_"));
            assert!(
                reason
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
            );
            assert!(seen.insert(reason));
        }
    }
}

pub mod library {
    /// A library scan is already running.
    pub const SCAN_ALREADY_RUNNING: &str = "LIBRARY_SCAN_ALREADY_RUNNING";
    /// Library roots must be unique within a library.
    pub const ROOT_DUPLICATED: &str = "LIBRARY_ROOT_DUPLICATED";
    /// Library root the selected item is already used by the selected value.
    pub const ROOT_ALREADY_ASSIGNED: &str = "LIBRARY_ROOT_ALREADY_ASSIGNED";
    /// Library root the selected item cannot be used.
    pub const ROOT_CANNOT_USED: &str = "LIBRARY_ROOT_CANNOT_USED";
    /// Library was not found.
    pub const RESOURCE_NOT_FOUND: &str = "LIBRARY_RESOURCE_NOT_FOUND";
    /// Library name is required.
    pub const NAME_REQUIRED: &str = "LIBRARY_NAME_REQUIRED";
    /// Libraries require at least one root folder.
    pub const ROOT_REQUIRED: &str = "LIBRARY_ROOT_REQUIRED";
    /// Default libraries cannot be deleted.
    pub const DEFAULT_LIBRARIES_CANNOT_DELETED: &str = "LIBRARY_DEFAULT_LIBRARIES_CANNOT_DELETED";
    /// Library scan session was not found.
    pub const SCAN_SESSION_NOT_FOUND: &str = "LIBRARY_SCAN_SESSION_NOT_FOUND";
    /// Library scan session is not cancelable.
    pub const SCAN_CANCEL_UNAVAILABLE: &str = "LIBRARY_SCAN_CANCEL_UNAVAILABLE";
    /// The requested value library roots are not configured.
    pub const ROOTS_NOT_CONFIGURED: &str = "LIBRARY_ROOTS_NOT_CONFIGURED";
    /// Library path is not a directory.
    pub const PATH_NOT_DIRECTORY: &str = "LIBRARY_PATH_NOT_DIRECTORY";
    /// Default  library was not found.
    pub const DEFAULT_LIBRARY_NOT_FOUND: &str = "LIBRARY_DEFAULT_LIBRARY_NOT_FOUND";
    /// Requested facet does not match title facet.
    pub const REQUESTED_FACET_DOES_NOT_MATCH_TITLE_FACET: &str =
        "LIBRARY_REQUESTED_FACET_DOES_NOT_MATCH_TITLE_FACET";
    /// Renamer_disabled.
    pub const RENAMER_DISABLED: &str = "LIBRARY_RENAMER_DISABLED";
    /// Rename_stale_plan.
    pub const RENAME_STALE_PLAN: &str = "LIBRARY_RENAME_STALE_PLAN";
    /// Destination folder already contains other title folders; it is an organizing folder, not a title folder.
    pub const FOLDER_NOT_TITLE_BOUNDARY: &str = "LIBRARY_FOLDER_NOT_TITLE_BOUNDARY";
    /// Refusing to delete the selected value because it is outside the configured media roots.
    pub const PATH_OUTSIDE_LIBRARY_ROOT: &str = "LIBRARY_PATH_OUTSIDE_LIBRARY_ROOT";
    /// The requested value must not contain . or .. components.
    pub const PATH_COMPONENT_INVALID: &str = "LIBRARY_PATH_COMPONENT_INVALID";
    /// The requested value template must include {{the selected value}}.
    pub const TEMPLATE_IDENTITY_TOKEN_REQUIRED: &str = "LIBRARY_TEMPLATE_IDENTITY_TOKEN_REQUIRED";
    /// Invalid episode folder optional group.
    pub const EPISODE_FOLDER_OPTIONAL_GROUP_INVALID: &str =
        "LIBRARY_EPISODE_FOLDER_OPTIONAL_GROUP_INVALID";
    /// Unclosed episode folder token.
    pub const UNCLOSED_EPISODE_FOLDER_TOKEN: &str = "LIBRARY_UNCLOSED_EPISODE_FOLDER_TOKEN";
    /// Invalid episode folder pattern.
    pub const EPISODE_FOLDER_PATTERN_INVALID: &str = "LIBRARY_EPISODE_FOLDER_PATTERN_INVALID";
    /// The requested value template contains an unmatched '}}'.
    pub const TEMPLATE_BRACES_UNBALANCED: &str = "LIBRARY_TEMPLATE_BRACES_UNBALANCED";
    /// Unsupported the selected value template token: {{the selected value}}.
    pub const TEMPLATE_TOKEN_UNSUPPORTED: &str = "LIBRARY_TEMPLATE_TOKEN_UNSUPPORTED";
    /// Rename template is required.
    pub const TEMPLATE_REQUIRED: &str = "LIBRARY_TEMPLATE_REQUIRED";
    /// Pending import path is missing or invalid.
    pub const PENDING_IMPORT_PATH_INVALID: &str = "LIBRARY_PENDING_IMPORT_PATH_INVALID";
    /// Pending import file is unavailable.
    pub const PENDING_IMPORT_FILE_UNAVAILABLE: &str = "LIBRARY_PENDING_IMPORT_FILE_UNAVAILABLE";
    /// Pending import path is not a file.
    pub const PENDING_IMPORT_PATH_NOT_FILE: &str = "LIBRARY_PENDING_IMPORT_PATH_NOT_FILE";
    /// Folder ownership conflicts cannot be bound or adopted.
    pub const FOLDER_OWNERSHIP_CONFLICT: &str = "LIBRARY_FOLDER_OWNERSHIP_CONFLICT";
    /// Pending import the selected value is already being resolved.
    pub const PENDING_IMPORT_ALREADY_BEING_RESOLVED: &str =
        "LIBRARY_PENDING_IMPORT_ALREADY_BEING_RESOLVED";
    /// Pending import path is a folder, not a file, so it cannot be bound to episodes; its stale title link was cleared, so reload pending imports and attach the folder to a title again.
    pub const PENDING_IMPORT_PATH_FOLDER_NOT_FILE_SO_CANNOT_BOUND_EPISODES: &str =
        "LIBRARY_PENDING_IMPORT_PATH_FOLDER_NOT_FILE_SO_CANNOT_BOUND_EPISODES";
    /// Pending import id is required.
    pub const PENDING_IMPORT_ID_REQUIRED: &str = "LIBRARY_PENDING_IMPORT_ID_REQUIRED";
    /// Pending import was not found.
    pub const PENDING_IMPORT_NOT_FOUND: &str = "LIBRARY_PENDING_IMPORT_NOT_FOUND";
    /// Pending import requires explicit episode binding.
    pub const PENDING_IMPORT_REQUIRES_EXPLICIT_EPISODE_BINDING: &str =
        "LIBRARY_PENDING_IMPORT_REQUIRES_EXPLICIT_EPISODE_BINDING";
    /// A title identity is required.
    pub const TITLE_IDENTITY_REQUIRED: &str = "LIBRARY_TITLE_IDENTITY_REQUIRED";
    /// Title already exists in this library.
    pub const TITLE_ALREADY_EXISTS_LIBRARY: &str = "LIBRARY_TITLE_ALREADY_EXISTS_LIBRARY";
    /// Pending import path is unavailable.
    pub const PENDING_IMPORT_PATH_UNAVAILABLE: &str = "LIBRARY_PENDING_IMPORT_PATH_UNAVAILABLE";
    /// Pending import path is not a file or directory.
    pub const PENDING_IMPORT_PATH_NOT_FILE_DIRECTORY: &str =
        "LIBRARY_PENDING_IMPORT_PATH_NOT_FILE_DIRECTORY";
    /// Pending import directory contains no files to attach.
    pub const PENDING_IMPORT_DIRECTORY_CONTAINS_NO_FILES_ATTACH: &str =
        "LIBRARY_PENDING_IMPORT_DIRECTORY_CONTAINS_NO_FILES_ATTACH";
    /// Pending import path has no parent folder.
    pub const PENDING_IMPORT_PATH_NO_PARENT_FOLDER: &str =
        "LIBRARY_PENDING_IMPORT_PATH_NO_PARENT_FOLDER";
    /// Failed to attach pending import file to selected movie.
    pub const FAILED_ATTACH_PENDING_IMPORT_FILE_SELECTED_MOVIE: &str =
        "LIBRARY_FAILED_ATTACH_PENDING_IMPORT_FILE_SELECTED_MOVIE";
    /// Pending import does not have a known title.
    pub const PENDING_IMPORT_DOES_NOT_KNOWN_TITLE: &str =
        "LIBRARY_PENDING_IMPORT_DOES_NOT_KNOWN_TITLE";
    /// Collection the selected value does not belong to title the selected value.
    pub const COLLECTION_DOES_NOT_BELONG_TITLE: &str = "LIBRARY_COLLECTION_DOES_NOT_BELONG_TITLE";
    /// At least one episode must be selected.
    pub const EPISODE_SELECTION_REQUIRED: &str = "LIBRARY_EPISODE_SELECTION_REQUIRED";
    /// One or more selected episodes do not belong to the target title.
    pub const ONE_MORE_SELECTED_EPISODES_DO_NOT_BELONG_TARGET_TITLE: &str =
        "LIBRARY_ONE_MORE_SELECTED_EPISODES_DO_NOT_BELONG_TARGET_TITLE";
    /// Failed to bind pending import file to selected episodes.
    pub const FAILED_BIND_PENDING_IMPORT_FILE_SELECTED_EPISODES: &str =
        "LIBRARY_FAILED_BIND_PENDING_IMPORT_FILE_SELECTED_EPISODES";
    /// Recycle entry id is not a valid opaque id.
    pub const RECYCLE_ENTRY_ID_NOT_VALID_OPAQUE_ID: &str =
        "LIBRARY_RECYCLE_ENTRY_ID_NOT_VALID_OPAQUE_ID";
    /// Refusing to restore recycled file the selected value because it is a symlink.
    pub const SYMLINK_REFUSED: &str = "LIBRARY_SYMLINK_REFUSED";
    /// Refusing to restore recycled file the selected value because it is not a regular file.
    pub const REGULAR_FILE_REQUIRED: &str = "LIBRARY_REGULAR_FILE_REQUIRED";
    /// Refusing to recycle the selected value because the recycle bin path is unsafe.
    pub const RECYCLE_BIN_PATH_UNSAFE: &str = "LIBRARY_RECYCLE_BIN_PATH_UNSAFE";
    /// Refusing to delete the selected value because no configured media roots are available.
    pub const MEDIA_ROOT_REQUIRED: &str = "LIBRARY_MEDIA_ROOT_REQUIRED";
    /// Refusing to restore recycled file the selected value because destination is occupied.
    pub const DESTINATION_OCCUPIED: &str = "LIBRARY_DESTINATION_OCCUPIED";
    /// At least one episode id is required.
    pub const EPISODE_ID_REQUIRED: &str = "LIBRARY_EPISODE_ID_REQUIRED";
    /// Scoped policy deletion cannot remove directories.
    pub const SCOPED_POLICY_DELETION_CANNOT_REMOVE_DIRECTORIES: &str =
        "LIBRARY_SCOPED_POLICY_DELETION_CANNOT_REMOVE_DIRECTORIES";
    /// Maintenance deletion targets changed; re-evaluation required.
    pub const MAINTENANCE_DELETION_TARGETS_CHANGED: &str =
        "LIBRARY_MAINTENANCE_DELETION_TARGETS_CHANGED";
    /// Maintenance deletion file changed; re-evaluation required.
    pub const MAINTENANCE_DELETION_FILE_CHANGED: &str = "LIBRARY_MAINTENANCE_DELETION_FILE_CHANGED";
    /// Maintenance deletion fingerprint changed.
    pub const MAINTENANCE_DELETION_FINGERPRINT_CHANGED: &str =
        "LIBRARY_MAINTENANCE_DELETION_FINGERPRINT_CHANGED";
    /// External subtitle was not found.
    pub const EXTERNAL_SUBTITLE_NOT_FOUND: &str = "LIBRARY_EXTERNAL_SUBTITLE_NOT_FOUND";
    /// Only provider-backed downloaded subtitles can be blocklisted.
    pub const ONLY_PROVIDER_BACKED_DOWNLOADED_SUBTITLES_BLOCKLISTED: &str =
        "LIBRARY_ONLY_PROVIDER_BACKED_DOWNLOADED_SUBTITLES_BLOCKLISTED";
    /// Media file was not found.
    pub const MEDIA_FILE_NOT_FOUND: &str = "LIBRARY_MEDIA_FILE_NOT_FOUND";
    /// Recorded media path is outside its title's owned folder.
    pub const RECORDED_MEDIA_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER: &str =
        "LIBRARY_RECORDED_MEDIA_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER";
    /// Title folder overlaps another title; refusing media deletion.
    pub const TITLE_FOLDER_OVERLAPS_ANOTHER_TITLE: &str =
        "LIBRARY_TITLE_FOLDER_OVERLAPS_ANOTHER_TITLE";
    /// Recorded subtitle path is outside its title's owned folder.
    pub const RECORDED_SUBTITLE_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER: &str =
        "LIBRARY_RECORDED_SUBTITLE_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER";
    /// Recorded media path has no folder.
    pub const RECORDED_MEDIA_PATH_NO_FOLDER: &str = "LIBRARY_RECORDED_MEDIA_PATH_NO_FOLDER";
    /// Title has no recorded media.
    pub const TITLE_NO_RECORDED_MEDIA: &str = "LIBRARY_TITLE_NO_RECORDED_MEDIA";
    /// Batch size must be positive.
    pub const BATCH_SIZE_OUT_OF_RANGE: &str = "LIBRARY_BATCH_SIZE_OUT_OF_RANGE";
    /// Title directory is not a directory.
    pub const TITLE_DIRECTORY_NOT_DIRECTORY: &str = "LIBRARY_TITLE_DIRECTORY_NOT_DIRECTORY";
    /// Title has no owned folder.
    pub const TITLE_NO_OWNED_FOLDER: &str = "LIBRARY_TITLE_NO_OWNED_FOLDER";
    /// Owned folder is unavailable.
    pub const OWNED_FOLDER_UNAVAILABLE: &str = "LIBRARY_OWNED_FOLDER_UNAVAILABLE";
    /// Library scan is not supported for this facet.
    pub const SCAN_UNSUPPORTED: &str = "LIBRARY_SCAN_UNSUPPORTED";
    /// Title library scan is only supported for episodic titles.
    pub const TITLE_LIBRARY_SCAN_ONLY_SUPPORTED_EPISODIC_TITLES: &str =
        "LIBRARY_TITLE_LIBRARY_SCAN_ONLY_SUPPORTED_EPISODIC_TITLES";
    /// Selected folder is an organizing folder or mixed title layout; select the title folder containing its media.
    pub const SELECTED_FOLDER_ORGANIZING_FOLDER_MIXED_TITLE_LAYOUT: &str =
        "LIBRARY_SELECTED_FOLDER_ORGANIZING_FOLDER_MIXED_TITLE_LAYOUT";
    /// Media identity conflicts with the proposed title folder.
    pub const MEDIA_IDENTITY_CONFLICTS_PROPOSED_TITLE_FOLDER: &str =
        "LIBRARY_MEDIA_IDENTITY_CONFLICTS_PROPOSED_TITLE_FOLDER";
    /// Linked folder requires explicit ownership resolution before scanning.
    pub const LINKED_FOLDER_REQUIRES_EXPLICIT_OWNERSHIP_RESOLUTION_BEFORE_SCANNING: &str =
        "LIBRARY_LINKED_FOLDER_REQUIRES_EXPLICIT_OWNERSHIP_RESOLUTION_BEFORE_SCANNING";
    /// Automatic title discovery depth limit reached.
    pub const AUTOMATIC_TITLE_DISCOVERY_DEPTH_LIMIT_REACHED: &str =
        "LIBRARY_AUTOMATIC_TITLE_DISCOVERY_DEPTH_LIMIT_REACHED";
    /// Destination folder contains an independent or unresolved title boundary.
    pub const DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY: &str =
        "LIBRARY_DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY";
    /// At least one title is required for renaming.
    pub const TITLE_REQUIRED: &str = "LIBRARY_TITLE_REQUIRED";
    /// Duplicate title id in rename request.
    pub const TITLE_ID_RENAME_REQUEST_DUPLICATED: &str =
        "LIBRARY_TITLE_ID_RENAME_REQUEST_DUPLICATED";
    /// A rename is already running for the selected value.
    pub const RENAME_ALREADY_RUNNING: &str = "LIBRARY_RENAME_ALREADY_RUNNING";
    /// Refusing to delete the selected value the selected value because it has no parent directory.
    pub const DELETE_TARGET_PARENT_REQUIRED: &str = "LIBRARY_DELETE_TARGET_PARENT_REQUIRED";
    /// Refusing to delete the selected value the selected value because it is outside the configured root folders.
    pub const DELETE_TARGET_OUTSIDE_ROOTS: &str = "LIBRARY_DELETE_TARGET_OUTSIDE_ROOTS";
    /// Configured the selected value root folder the selected value is invalid.
    pub const CONFIGURED_ROOT_FOLDER_INVALID: &str = "LIBRARY_CONFIGURED_ROOT_FOLDER_INVALID";
    /// No configured the selected value root folders are available for safe deletion.
    pub const NO_ROOT_FOLDERS_UNAVAILABLE: &str = "LIBRARY_NO_ROOT_FOLDERS_UNAVAILABLE";
    /// Path must be absolute.
    pub const PATH_INVALID: &str = "LIBRARY_PATH_INVALID";
    /// Path cannot escape its root.
    pub const PATH_CANNOT_ESCAPE_ROOT: &str = "LIBRARY_PATH_CANNOT_ESCAPE_ROOT";
    /// Refusing to delete directory path the selected value from a single-file delete flow.
    pub const DELETE_TARGET_NOT_FILE: &str = "LIBRARY_DELETE_TARGET_NOT_FILE";
    /// The proposed rename path lies outside its library root.
    pub const RENAME_PATH_OUTSIDE_ROOT: &str = "LIBRARY_RENAME_PATH_OUTSIDE_ROOT";
    /// The selected title folders overlap.
    pub const RENAME_FOLDERS_OVERLAP: &str = "LIBRARY_RENAME_FOLDERS_OVERLAP";
    /// The template optional group is invalid.
    pub const TEMPLATE_OPTIONAL_GROUP_INVALID: &str = "LIBRARY_TEMPLATE_OPTIONAL_GROUP_INVALID";
    /// The folder is not a valid title boundary for scanning.
    pub const SCAN_FOLDER_BOUNDARY_INVALID: &str = "LIBRARY_SCAN_FOLDER_BOUNDARY_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SCAN_ALREADY_RUNNING,
        ROOT_DUPLICATED,
        ROOT_ALREADY_ASSIGNED,
        ROOT_CANNOT_USED,
        RESOURCE_NOT_FOUND,
        NAME_REQUIRED,
        ROOT_REQUIRED,
        DEFAULT_LIBRARIES_CANNOT_DELETED,
        SCAN_SESSION_NOT_FOUND,
        SCAN_CANCEL_UNAVAILABLE,
        ROOTS_NOT_CONFIGURED,
        PATH_NOT_DIRECTORY,
        DEFAULT_LIBRARY_NOT_FOUND,
        REQUESTED_FACET_DOES_NOT_MATCH_TITLE_FACET,
        RENAMER_DISABLED,
        RENAME_STALE_PLAN,
        FOLDER_NOT_TITLE_BOUNDARY,
        PATH_OUTSIDE_LIBRARY_ROOT,
        PATH_COMPONENT_INVALID,
        TEMPLATE_IDENTITY_TOKEN_REQUIRED,
        EPISODE_FOLDER_OPTIONAL_GROUP_INVALID,
        UNCLOSED_EPISODE_FOLDER_TOKEN,
        EPISODE_FOLDER_PATTERN_INVALID,
        TEMPLATE_BRACES_UNBALANCED,
        TEMPLATE_TOKEN_UNSUPPORTED,
        TEMPLATE_REQUIRED,
        PENDING_IMPORT_PATH_INVALID,
        PENDING_IMPORT_FILE_UNAVAILABLE,
        PENDING_IMPORT_PATH_NOT_FILE,
        FOLDER_OWNERSHIP_CONFLICT,
        PENDING_IMPORT_ALREADY_BEING_RESOLVED,
        PENDING_IMPORT_PATH_FOLDER_NOT_FILE_SO_CANNOT_BOUND_EPISODES,
        PENDING_IMPORT_ID_REQUIRED,
        PENDING_IMPORT_NOT_FOUND,
        PENDING_IMPORT_REQUIRES_EXPLICIT_EPISODE_BINDING,
        TITLE_IDENTITY_REQUIRED,
        TITLE_ALREADY_EXISTS_LIBRARY,
        PENDING_IMPORT_PATH_UNAVAILABLE,
        PENDING_IMPORT_PATH_NOT_FILE_DIRECTORY,
        PENDING_IMPORT_DIRECTORY_CONTAINS_NO_FILES_ATTACH,
        PENDING_IMPORT_PATH_NO_PARENT_FOLDER,
        FAILED_ATTACH_PENDING_IMPORT_FILE_SELECTED_MOVIE,
        PENDING_IMPORT_DOES_NOT_KNOWN_TITLE,
        COLLECTION_DOES_NOT_BELONG_TITLE,
        EPISODE_SELECTION_REQUIRED,
        ONE_MORE_SELECTED_EPISODES_DO_NOT_BELONG_TARGET_TITLE,
        FAILED_BIND_PENDING_IMPORT_FILE_SELECTED_EPISODES,
        RECYCLE_ENTRY_ID_NOT_VALID_OPAQUE_ID,
        SYMLINK_REFUSED,
        REGULAR_FILE_REQUIRED,
        RECYCLE_BIN_PATH_UNSAFE,
        MEDIA_ROOT_REQUIRED,
        DESTINATION_OCCUPIED,
        EPISODE_ID_REQUIRED,
        SCOPED_POLICY_DELETION_CANNOT_REMOVE_DIRECTORIES,
        MAINTENANCE_DELETION_TARGETS_CHANGED,
        MAINTENANCE_DELETION_FILE_CHANGED,
        MAINTENANCE_DELETION_FINGERPRINT_CHANGED,
        EXTERNAL_SUBTITLE_NOT_FOUND,
        ONLY_PROVIDER_BACKED_DOWNLOADED_SUBTITLES_BLOCKLISTED,
        MEDIA_FILE_NOT_FOUND,
        RECORDED_MEDIA_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER,
        TITLE_FOLDER_OVERLAPS_ANOTHER_TITLE,
        RECORDED_SUBTITLE_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER,
        RECORDED_MEDIA_PATH_NO_FOLDER,
        TITLE_NO_RECORDED_MEDIA,
        BATCH_SIZE_OUT_OF_RANGE,
        TITLE_DIRECTORY_NOT_DIRECTORY,
        TITLE_NO_OWNED_FOLDER,
        OWNED_FOLDER_UNAVAILABLE,
        SCAN_UNSUPPORTED,
        TITLE_LIBRARY_SCAN_ONLY_SUPPORTED_EPISODIC_TITLES,
        SELECTED_FOLDER_ORGANIZING_FOLDER_MIXED_TITLE_LAYOUT,
        MEDIA_IDENTITY_CONFLICTS_PROPOSED_TITLE_FOLDER,
        LINKED_FOLDER_REQUIRES_EXPLICIT_OWNERSHIP_RESOLUTION_BEFORE_SCANNING,
        AUTOMATIC_TITLE_DISCOVERY_DEPTH_LIMIT_REACHED,
        DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY,
        TITLE_REQUIRED,
        TITLE_ID_RENAME_REQUEST_DUPLICATED,
        RENAME_ALREADY_RUNNING,
        DELETE_TARGET_PARENT_REQUIRED,
        DELETE_TARGET_OUTSIDE_ROOTS,
        CONFIGURED_ROOT_FOLDER_INVALID,
        NO_ROOT_FOLDERS_UNAVAILABLE,
        PATH_INVALID,
        PATH_CANNOT_ESCAPE_ROOT,
        DELETE_TARGET_NOT_FILE,
        RENAME_PATH_OUTSIDE_ROOT,
        RENAME_FOLDERS_OVERLAP,
        TEMPLATE_OPTIONAL_GROUP_INVALID,
        SCAN_FOLDER_BOUNDARY_INVALID,
    ];
}

pub mod external_import {
    /// The import session no longer exists. Connect again.
    pub const SESSION_GONE: &str = "EXTERNAL_IMPORT_SESSION_GONE";
    /// Could not determine the selected value version from the selected item.
    pub const COULD_NOT_DETERMINE_VERSION: &str = "EXTERNAL_IMPORT_COULD_NOT_DETERMINE_VERSION";
    /// Unsupported the selected value version the selected item; expected major the selected value or newer.
    pub const VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED: &str =
        "EXTERNAL_IMPORT_VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED";
    /// The requested value did not report an API version.
    pub const DID_NOT_REPORT_API_VERSION: &str = "EXTERNAL_IMPORT_DID_NOT_REPORT_API_VERSION";
    /// Unsupported the selected value API version the selected item; expected one of the selected value.
    pub const API_VERSION_EXPECTED_ONE_UNSUPPORTED: &str =
        "EXTERNAL_IMPORT_API_VERSION_EXPECTED_ONE_UNSUPPORTED";
    /// The requested value must not include a query string or fragment.
    pub const SOURCE_URL_QUERY_OR_FRAGMENT: &str = "EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT";
    /// The requested value has an invalid host.
    pub const INVALID_HOST: &str = "EXTERNAL_IMPORT_INVALID_HOST";
    /// The source URL belongs to a different application.
    pub const SOURCE_APPLICATION_MISMATCH: &str = "EXTERNAL_IMPORT_SOURCE_APPLICATION_MISMATCH";
    /// The source URL is invalid.
    pub const SOURCE_URL_INVALID: &str = "EXTERNAL_IMPORT_SOURCE_URL_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SESSION_GONE,
        COULD_NOT_DETERMINE_VERSION,
        VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED,
        DID_NOT_REPORT_API_VERSION,
        API_VERSION_EXPECTED_ONE_UNSUPPORTED,
        SOURCE_URL_QUERY_OR_FRAGMENT,
        INVALID_HOST,
        SOURCE_APPLICATION_MISMATCH,
        SOURCE_URL_INVALID,
    ];
}

pub mod catalog {
    /// The title was not found.
    pub const TITLE_NOT_FOUND: &str = "TITLE_NOT_FOUND";
    /// Search query is required.
    pub const SEARCH_QUERY_REQUIRED: &str = "CATALOG_SEARCH_QUERY_REQUIRED";
    /// A raw page read must explicitly select exactly one source.
    pub const RAW_SEARCH_PAGE_REQUIRES_SINGLE_INDEXER: &str =
        "CATALOG_RAW_SEARCH_PAGE_REQUIRES_SINGLE_INDEXER";
    /// The adapter does not implement raw page continuation.
    pub const RAW_SEARCH_PAGING_UNSUPPORTED: &str = "CATALOG_RAW_SEARCH_PAGING_UNSUPPORTED";
    /// Provide exactly one of title id or query.
    pub const PROVIDE_EXACTLY_ONE_TITLE_ID_QUERY: &str =
        "CATALOG_PROVIDE_EXACTLY_ONE_TITLE_ID_QUERY";
    /// Unknown indexer the selected value.
    pub const INDEXER_UNKNOWN: &str = "CATALOG_INDEXER_UNKNOWN";
    /// Too many concurrent interactive searches.
    pub const TOO_MANY_CONCURRENT_INTERACTIVE_SEARCHES: &str =
        "CATALOG_TOO_MANY_CONCURRENT_INTERACTIVE_SEARCHES";
    /// Release has no usable download source.
    pub const RELEASE_NO_USABLE_DOWNLOAD_SOURCE: &str = "CATALOG_RELEASE_NO_USABLE_DOWNLOAD_SOURCE";
    /// Download client the selected value does not exist.
    pub const DOWNLOAD_CLIENT_DOES_NOT_EXIST: &str = "CATALOG_DOWNLOAD_CLIENT_DOES_NOT_EXIST";
    /// Download client the selected value is disabled.
    pub const DOWNLOAD_CLIENT_DISABLED: &str = "CATALOG_DOWNLOAD_CLIENT_DISABLED";
    /// Select at least one release to download.
    pub const RELEASE_DOWNLOAD_REQUIRED: &str = "CATALOG_RELEASE_DOWNLOAD_REQUIRED";
    /// At most the selected value releases can be downloaded at once.
    pub const MOST_RELEASES_DOWNLOADED_ONCE: &str = "CATALOG_MOST_RELEASES_DOWNLOADED_ONCE";
    /// The requested value has no usable download source.
    pub const NO_USABLE_DOWNLOAD_SOURCE: &str = "CATALOG_NO_USABLE_DOWNLOAD_SOURCE";
    /// The requested value is a magnet link and has no file to download.
    pub const MAGNET_LINK_NO_FILE_DOWNLOAD: &str = "CATALOG_MAGNET_LINK_NO_FILE_DOWNLOAD";
    /// Selected release files exceed the 64 MiB download bundle limit.
    pub const SELECTED_RELEASE_FILES_EXCEED_64_MIB_DOWNLOAD_BUNDLE_LIMIT: &str =
        "CATALOG_SELECTED_RELEASE_FILES_EXCEED_64_MIB_DOWNLOAD_BUNDLE_LIMIT";
    /// Interactive release search was not found.
    pub const INTERACTIVE_RELEASE_SEARCH_NOT_FOUND: &str =
        "CATALOG_INTERACTIVE_RELEASE_SEARCH_NOT_FOUND";
    /// Release is no longer in this search was not found.
    pub const RELEASE_NO_LONGER_SEARCH_NOT_FOUND: &str =
        "CATALOG_RELEASE_NO_LONGER_SEARCH_NOT_FOUND";
    /// Movie searches cannot include season or episode.
    pub const MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE: &str =
        "CATALOG_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE";
    /// Series movie was not found.
    pub const SERIES_MOVIE_NOT_FOUND: &str = "CATALOG_SERIES_MOVIE_NOT_FOUND";
    /// Series movie does not belong to title.
    pub const SERIES_MOVIE_DOES_NOT_BELONG_TITLE: &str =
        "CATALOG_SERIES_MOVIE_DOES_NOT_BELONG_TITLE";
    /// Episode searches require a season.
    pub const EPISODE_SEARCHES_REQUIRE_SEASON: &str = "CATALOG_EPISODE_SEARCHES_REQUIRE_SEASON";
    /// Series movie searches cannot include season or episode.
    pub const SERIES_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE: &str =
        "CATALOG_SERIES_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE";
    /// The requested value titles have no series metadata.
    pub const TITLES_NO_SERIES_METADATA: &str = "CATALOG_TITLES_NO_SERIES_METADATA";
    /// Media file was not found.
    pub const MEDIA_FILE_NOT_FOUND: &str = "CATALOG_MEDIA_FILE_NOT_FOUND";
    /// Additional-file title queueing supports only movie titles.
    pub const ADDITIONAL_FILE_TITLE_QUEUEING_SUPPORTS_ONLY_MOVIE_TITLES: &str =
        "CATALOG_ADDITIONAL_FILE_TITLE_QUEUEING_SUPPORTS_ONLY_MOVIE_TITLES";
    /// Additional-file queueing supports only title and single-episode scopes.
    pub const ADDITIONAL_FILE_QUEUEING_SUPPORTS_ONLY_TITLE_SINGLE_EPISODE_SCOPES: &str =
        "CATALOG_ADDITIONAL_FILE_QUEUEING_SUPPORTS_ONLY_TITLE_SINGLE_EPISODE_SCOPES";
    /// Additional-file queueing does not support collection scopes yet.
    pub const ADDITIONAL_FILE_QUEUEING_DOES_NOT_SUPPORT_COLLECTION_SCOPES_YET: &str =
        "CATALOG_ADDITIONAL_FILE_QUEUEING_DOES_NOT_SUPPORT_COLLECTION_SCOPES_YET";
    /// Additional-file queueing requires a title or episode scope.
    pub const ADDITIONAL_FILE_QUEUEING_REQUIRES_TITLE_EPISODE_SCOPE: &str =
        "CATALOG_ADDITIONAL_FILE_QUEUEING_REQUIRES_TITLE_EPISODE_SCOPE";
    /// Blocklist entry was not found.
    pub const BLOCKLIST_ENTRY_NOT_FOUND: &str = "CATALOG_BLOCKLIST_ENTRY_NOT_FOUND";
    /// Additional-file queueing cannot replace existing media.
    pub const ADDITIONAL_FILE_QUEUEING_CANNOT_REPLACE_EXISTING_MEDIA: &str =
        "CATALOG_ADDITIONAL_FILE_QUEUEING_CANNOT_REPLACE_EXISTING_MEDIA";
    /// Release size does not match the signed candidate.
    pub const RELEASE_SIZE_DOES_NOT_MATCH_SIGNED_CANDIDATE: &str =
        "CATALOG_RELEASE_SIZE_DOES_NOT_MATCH_SIGNED_CANDIDATE";
    /// Release has no protocol.
    pub const RELEASE_NO_PROTOCOL: &str = "CATALOG_RELEASE_NO_PROTOCOL";
    /// Selected download client is not eligible.
    pub const DOWNLOAD_CLIENT_UNAVAILABLE: &str = "CATALOG_DOWNLOAD_CLIENT_UNAVAILABLE";
    /// A download is already queued for this title.
    pub const DOWNLOAD_ALREADY_QUEUED_TITLE: &str = "CATALOG_DOWNLOAD_ALREADY_QUEUED_TITLE";
    /// Episode was not found.
    pub const EPISODE_NOT_FOUND: &str = "CATALOG_EPISODE_NOT_FOUND";
    /// Episode is missing season number.
    pub const EPISODE_MISSING_SEASON_NUMBER: &str = "CATALOG_EPISODE_MISSING_SEASON_NUMBER";
    /// Episode is missing episode number.
    pub const EPISODE_MISSING_EPISODE_NUMBER: &str = "CATALOG_EPISODE_MISSING_EPISODE_NUMBER";
    /// Best-release search is not supported for collection scopes.
    pub const BEST_RELEASE_SEARCH_UNSUPPORTED: &str = "CATALOG_BEST_RELEASE_SEARCH_UNSUPPORTED";
    /// Series movie link was not found.
    pub const SERIES_MOVIE_LINK_NOT_FOUND: &str = "CATALOG_SERIES_MOVIE_LINK_NOT_FOUND";
    /// Invalid snapshot JSON.
    pub const SNAPSHOT_JSON_COLUMN_INVALID: &str = "CATALOG_SNAPSHOT_JSON_COLUMN_INVALID";
    /// Metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn.
    pub const METADATA_LANGUAGE_INVALID: &str = "CATALOG_METADATA_LANGUAGE_INVALID";
    /// At least one title field must be provided.
    pub const TITLE_FIELD_REQUIRED: &str = "CATALOG_TITLE_FIELD_REQUIRED";
    /// Changing a title facet is not supported because titles cannot move between libraries.
    pub const CHANGING_TITLE_FACET_UNSUPPORTED: &str = "CATALOG_CHANGING_TITLE_FACET_UNSUPPORTED";
    /// Minimum availability is only valid for movie titles.
    pub const MINIMUM_AVAILABILITY_ONLY_VALID_MOVIE_TITLES: &str =
        "CATALOG_MINIMUM_AVAILABILITY_ONLY_VALID_MOVIE_TITLES";
    /// Minimum availability must be announced, in_cinemas, or released.
    pub const MINIMUM_AVAILABILITY_INVALID: &str = "CATALOG_MINIMUM_AVAILABILITY_INVALID";
    /// Primary movie file can only be set for movie titles, series movie files, or episode files.
    pub const PRIMARY_MOVIE_FILE_ONLY_SET_MOVIE_TITLES_SERIES_MOVIE_FILES_EPISODE_FILES: &str =
        "CATALOG_PRIMARY_MOVIE_FILE_ONLY_SET_MOVIE_TITLES_SERIES_MOVIE_FILES_EPISODE_FILES";
    /// Movie title does not have a canonical folder path.
    pub const MOVIE_TITLE_DOES_NOT_CANONICAL_FOLDER_PATH: &str =
        "CATALOG_MOVIE_TITLE_DOES_NOT_CANONICAL_FOLDER_PATH";
    /// Selected file is outside the title's canonical movie folder.
    pub const SELECTED_FILE_OUTSIDE_TITLE_S_CANONICAL_MOVIE_FOLDER: &str =
        "CATALOG_SELECTED_FILE_OUTSIDE_TITLE_S_CANONICAL_MOVIE_FOLDER";
    /// Tvdb id must be numeric.
    pub const TVDB_ID_INVALID: &str = "CATALOG_TVDB_ID_INVALID";
    /// Smg id must be positive.
    pub const SMG_ID_OUT_OF_RANGE: &str = "CATALOG_SMG_ID_OUT_OF_RANGE";
    /// A title identity is required.
    pub const TITLE_IDENTITY_REQUIRED: &str = "CATALOG_TITLE_IDENTITY_REQUIRED";
    /// Movie metadata response missing title was not found.
    pub const MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND: &str =
        "CATALOG_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND";
    /// The requested value id the selected value is already assigned to title the selected value.
    pub const ID_ALREADY_ASSIGNED_TITLE: &str = "CATALOG_ID_ALREADY_ASSIGNED_TITLE";
    /// Unsupported title facet.
    pub const TITLE_FACET_UNSUPPORTED: &str = "CATALOG_TITLE_FACET_UNSUPPORTED";
    /// A media file deletion is already running for the selected value.
    pub const MEDIA_FILE_DELETION_ALREADY_RUNNING: &str =
        "CATALOG_MEDIA_FILE_DELETION_ALREADY_RUNNING";
    /// Delete preview confirmation is required before deleting files on disk.
    pub const DELETE_PREVIEW_CONFIRMATION_REQUIRED_BEFORE_DELETING_FILES_DISK: &str =
        "CATALOG_DELETE_PREVIEW_CONFIRMATION_REQUIRED_BEFORE_DELETING_FILES_DISK";
    /// Library was not found.
    pub const LIBRARY_NOT_FOUND: &str = "CATALOG_LIBRARY_NOT_FOUND";
    /// RootFolderId must reference a root on the title library.
    pub const ROOTFOLDERID_MUST_REFERENCE_ROOT_TITLE_LIBRARY: &str =
        "CATALOG_ROOTFOLDERID_MUST_REFERENCE_ROOT_TITLE_LIBRARY";
    /// Title library must have at least one root folder.
    pub const ROOT_REQUIRED: &str = "CATALOG_ROOT_REQUIRED";
    /// Pending rename refers to missing title tag the selected value.
    pub const PENDING_RENAME_REFERS_MISSING_TITLE_TAG: &str =
        "CATALOG_PENDING_RENAME_REFERS_MISSING_TITLE_TAG";
    /// Title tag the selected value changed while its rename was pending.
    pub const TITLE_TAG_CHANGED_WHILE_RENAME_PENDING: &str =
        "CATALOG_TITLE_TAG_CHANGED_WHILE_RENAME_PENDING";
    /// At least one tag to add or remove must be provided.
    pub const TAG_ADD_REMOVE_REQUIRED: &str = "CATALOG_TAG_ADD_REMOVE_REQUIRED";
    /// At least one title must be provided.
    pub const TITLE_REQUIRED: &str = "CATALOG_TITLE_REQUIRED";
    /// At least one series movie must be provided.
    pub const SERIES_MOVIE_REQUIRED: &str = "CATALOG_SERIES_MOVIE_REQUIRED";
    /// The selected item is not a defined tag; an administrator has to add it in Settings before it can be applied.
    pub const NOT_DEFINED_TAG: &str = "CATALOG_NOT_DEFINED_TAG";
    /// The selected item is listed both to add and to remove; choose one.
    pub const LISTED_BOTH_ADD_REMOVE: &str = "CATALOG_LISTED_BOTH_ADD_REMOVE";
    /// A title can carry at most the selected value tags; this change would leave it with the selected value.
    pub const TITLE_CARRY_MOST_TAGS: &str = "CATALOG_TITLE_CARRY_MOST_TAGS";
    /// Collection was not found.
    pub const COLLECTION_NOT_FOUND: &str = "CATALOG_COLLECTION_NOT_FOUND";
    /// Configured quality profile the selected item from the selected value is missing from the catalog.
    pub const CONFIGURED_QUALITY_PROFILE_MISSING_CATALOG: &str =
        "CATALOG_CONFIGURED_QUALITY_PROFILE_MISSING_CATALOG";
    /// No quality profile is configured; choose a global, category, library, or title profile.
    pub const NO_QUALITY_PROFILE_NOT_CONFIGURED: &str = "CATALOG_NO_QUALITY_PROFILE_NOT_CONFIGURED";
    /// The existing download is no longer safe to replace.
    pub const EXISTING_DOWNLOAD_NO_LONGER_SAFE_REPLACE: &str =
        "CATALOG_EXISTING_DOWNLOAD_NO_LONGER_SAFE_REPLACE";
    /// Snapshot title is missing or ambiguous.
    pub const SNAPSHOT_TITLE_MISSING_AMBIGUOUS: &str = "CATALOG_SNAPSHOT_TITLE_MISSING_AMBIGUOUS";
    /// Snapshot title no longer exists.
    pub const SNAPSHOT_TITLE_NO_LONGER_EXISTS: &str = "CATALOG_SNAPSHOT_TITLE_NO_LONGER_EXISTS";
    /// Collection type is required.
    pub const COLLECTION_TYPE_REQUIRED: &str = "CATALOG_COLLECTION_TYPE_REQUIRED";
    /// Unknown collection type.
    pub const COLLECTION_TYPE_UNKNOWN: &str = "CATALOG_COLLECTION_TYPE_UNKNOWN";
    /// Collection index is required.
    pub const COLLECTION_INDEX_REQUIRED: &str = "CATALOG_COLLECTION_INDEX_REQUIRED";
    /// At least one collection field must be provided.
    pub const COLLECTION_FIELD_REQUIRED: &str = "CATALOG_COLLECTION_FIELD_REQUIRED";
    /// Episode type is required.
    pub const EPISODE_TYPE_REQUIRED: &str = "CATALOG_EPISODE_TYPE_REQUIRED";
    /// Unknown episode type.
    pub const EPISODE_TYPE_UNKNOWN: &str = "CATALOG_EPISODE_TYPE_UNKNOWN";
    /// At least one episode field must be provided.
    pub const EPISODE_FIELD_REQUIRED: &str = "CATALOG_EPISODE_FIELD_REQUIRED";
    /// Title name is required.
    pub const TITLE_NAME_REQUIRED: &str = "CATALOG_TITLE_NAME_REQUIRED";
    /// The requested value: SMG has no series for title id the selected value.
    pub const SMG_NO_SERIES_NOT_FOUND: &str = "CATALOG_SMG_NO_SERIES_NOT_FOUND";
    /// Movie entity was not found.
    pub const MOVIE_ENTITY_NOT_FOUND: &str = "CATALOG_MOVIE_ENTITY_NOT_FOUND";
    /// The title tag is invalid.
    pub const TAG_INVALID: &str = "CATALOG_TAG_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        TAG_INVALID,
        TITLE_NOT_FOUND,
        SEARCH_QUERY_REQUIRED,
        RAW_SEARCH_PAGE_REQUIRES_SINGLE_INDEXER,
        RAW_SEARCH_PAGING_UNSUPPORTED,
        PROVIDE_EXACTLY_ONE_TITLE_ID_QUERY,
        INDEXER_UNKNOWN,
        TOO_MANY_CONCURRENT_INTERACTIVE_SEARCHES,
        RELEASE_NO_USABLE_DOWNLOAD_SOURCE,
        DOWNLOAD_CLIENT_DOES_NOT_EXIST,
        DOWNLOAD_CLIENT_DISABLED,
        RELEASE_DOWNLOAD_REQUIRED,
        MOST_RELEASES_DOWNLOADED_ONCE,
        NO_USABLE_DOWNLOAD_SOURCE,
        MAGNET_LINK_NO_FILE_DOWNLOAD,
        SELECTED_RELEASE_FILES_EXCEED_64_MIB_DOWNLOAD_BUNDLE_LIMIT,
        INTERACTIVE_RELEASE_SEARCH_NOT_FOUND,
        RELEASE_NO_LONGER_SEARCH_NOT_FOUND,
        MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE,
        SERIES_MOVIE_NOT_FOUND,
        SERIES_MOVIE_DOES_NOT_BELONG_TITLE,
        EPISODE_SEARCHES_REQUIRE_SEASON,
        SERIES_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE,
        TITLES_NO_SERIES_METADATA,
        MEDIA_FILE_NOT_FOUND,
        ADDITIONAL_FILE_TITLE_QUEUEING_SUPPORTS_ONLY_MOVIE_TITLES,
        ADDITIONAL_FILE_QUEUEING_SUPPORTS_ONLY_TITLE_SINGLE_EPISODE_SCOPES,
        ADDITIONAL_FILE_QUEUEING_DOES_NOT_SUPPORT_COLLECTION_SCOPES_YET,
        ADDITIONAL_FILE_QUEUEING_REQUIRES_TITLE_EPISODE_SCOPE,
        BLOCKLIST_ENTRY_NOT_FOUND,
        ADDITIONAL_FILE_QUEUEING_CANNOT_REPLACE_EXISTING_MEDIA,
        RELEASE_SIZE_DOES_NOT_MATCH_SIGNED_CANDIDATE,
        RELEASE_NO_PROTOCOL,
        DOWNLOAD_CLIENT_UNAVAILABLE,
        DOWNLOAD_ALREADY_QUEUED_TITLE,
        EPISODE_NOT_FOUND,
        EPISODE_MISSING_SEASON_NUMBER,
        EPISODE_MISSING_EPISODE_NUMBER,
        BEST_RELEASE_SEARCH_UNSUPPORTED,
        SERIES_MOVIE_LINK_NOT_FOUND,
        SNAPSHOT_JSON_COLUMN_INVALID,
        METADATA_LANGUAGE_INVALID,
        TITLE_FIELD_REQUIRED,
        CHANGING_TITLE_FACET_UNSUPPORTED,
        MINIMUM_AVAILABILITY_ONLY_VALID_MOVIE_TITLES,
        MINIMUM_AVAILABILITY_INVALID,
        PRIMARY_MOVIE_FILE_ONLY_SET_MOVIE_TITLES_SERIES_MOVIE_FILES_EPISODE_FILES,
        MOVIE_TITLE_DOES_NOT_CANONICAL_FOLDER_PATH,
        SELECTED_FILE_OUTSIDE_TITLE_S_CANONICAL_MOVIE_FOLDER,
        TVDB_ID_INVALID,
        SMG_ID_OUT_OF_RANGE,
        TITLE_IDENTITY_REQUIRED,
        MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND,
        ID_ALREADY_ASSIGNED_TITLE,
        TITLE_FACET_UNSUPPORTED,
        MEDIA_FILE_DELETION_ALREADY_RUNNING,
        DELETE_PREVIEW_CONFIRMATION_REQUIRED_BEFORE_DELETING_FILES_DISK,
        LIBRARY_NOT_FOUND,
        ROOTFOLDERID_MUST_REFERENCE_ROOT_TITLE_LIBRARY,
        ROOT_REQUIRED,
        PENDING_RENAME_REFERS_MISSING_TITLE_TAG,
        TITLE_TAG_CHANGED_WHILE_RENAME_PENDING,
        TAG_ADD_REMOVE_REQUIRED,
        TITLE_REQUIRED,
        SERIES_MOVIE_REQUIRED,
        NOT_DEFINED_TAG,
        LISTED_BOTH_ADD_REMOVE,
        TITLE_CARRY_MOST_TAGS,
        COLLECTION_NOT_FOUND,
        CONFIGURED_QUALITY_PROFILE_MISSING_CATALOG,
        NO_QUALITY_PROFILE_NOT_CONFIGURED,
        EXISTING_DOWNLOAD_NO_LONGER_SAFE_REPLACE,
        SNAPSHOT_TITLE_MISSING_AMBIGUOUS,
        SNAPSHOT_TITLE_NO_LONGER_EXISTS,
        COLLECTION_TYPE_REQUIRED,
        COLLECTION_TYPE_UNKNOWN,
        COLLECTION_INDEX_REQUIRED,
        COLLECTION_FIELD_REQUIRED,
        EPISODE_TYPE_REQUIRED,
        EPISODE_TYPE_UNKNOWN,
        EPISODE_FIELD_REQUIRED,
        TITLE_NAME_REQUIRED,
        SMG_NO_SERIES_NOT_FOUND,
        MOVIE_ENTITY_NOT_FOUND,
    ];
}

pub mod security {
    /// The search results expired. Search again.
    pub const RELEASE_TOKEN_EXPIRED: &str = "RELEASE_TOKEN_EXPIRED";
    /// The release selection could not be verified. Search again.
    pub const RELEASE_TOKEN_INVALID: &str = "RELEASE_TOKEN_INVALID";
    /// This account is already linked to another user.
    pub const EXTERNAL_ACCOUNT_LINKED_ELSEWHERE: &str = "EXTERNAL_ACCOUNT_LINKED_ELSEWHERE";
    /// This account is linked to another provider identity.
    pub const EXTERNAL_ACCOUNT_IDENTITY_MISMATCH: &str = "EXTERNAL_ACCOUNT_IDENTITY_MISMATCH";
    /// The requested value connection is not configured for external auth.
    pub const CONNECTION_NOT_CONFIGURED: &str = "SECURITY_CONNECTION_NOT_CONFIGURED";
    /// The requested value connection is disabled.
    pub const CONNECTION_DISABLED: &str = "SECURITY_CONNECTION_DISABLED";
    /// The requested value is not enabled for the selected value.
    pub const NOT_ENABLED: &str = "SECURITY_NOT_ENABLED";
    /// Plex server discovery is required before using Plex for auth.
    pub const PLEX_SERVER_DISCOVERY_REQUIRED_BEFORE_USING_PLEX_AUTH: &str =
        "SECURITY_PLEX_SERVER_DISCOVERY_REQUIRED_BEFORE_USING_PLEX_AUTH";
    /// Verified external identity provider did not match the requested provider.
    pub const VERIFIED_EXTERNAL_IDENTITY_PROVIDER_DID_NOT_MATCH_REQUESTED_PROVIDER: &str =
        "SECURITY_VERIFIED_EXTERNAL_IDENTITY_PROVIDER_DID_NOT_MATCH_REQUESTED_PROVIDER";
    /// Verified external identity connection did not match the requested connection.
    pub const VERIFIED_EXTERNAL_IDENTITY_CONNECTION_DID_NOT_MATCH_REQUESTED_CONNECTION: &str =
        "SECURITY_VERIFIED_EXTERNAL_IDENTITY_CONNECTION_DID_NOT_MATCH_REQUESTED_CONNECTION";
    /// User was not found.
    pub const USER_NOT_FOUND: &str = "SECURITY_USER_NOT_FOUND";
    /// Jellyfin invites require selecting a Jellyfin user from the picker.
    pub const JELLYFIN_INVITES_REQUIRE_SELECTING_JELLYFIN_USER_PICKER: &str =
        "SECURITY_JELLYFIN_INVITES_REQUIRE_SELECTING_JELLYFIN_USER_PICKER";
    /// Jellyfin invites require a saved Jellyfin API key so Scryer can resolve the immutable user id.
    pub const JELLYFIN_INVITE_API_KEY_REQUIRED: &str = "SECURITY_JELLYFIN_INVITE_API_KEY_REQUIRED";
    /// Selected Jellyfin user was not found on the configured server.
    pub const SELECTED_JELLYFIN_USER_NOT_FOUND_CONFIGURED_SERVER: &str =
        "SECURITY_SELECTED_JELLYFIN_USER_NOT_FOUND_CONFIGURED_SERVER";
    /// Jellyfin account is already linked or invited.
    pub const JELLYFIN_ACCOUNT_ALREADY_LINKED_INVITED: &str =
        "SECURITY_JELLYFIN_ACCOUNT_ALREADY_LINKED_INVITED";
    /// Plex invites require selecting a Plex user from the picker.
    pub const PLEX_INVITES_REQUIRE_SELECTING_PLEX_USER_PICKER: &str =
        "SECURITY_PLEX_INVITES_REQUIRE_SELECTING_PLEX_USER_PICKER";
    /// Plex invites require a saved Plex token so Scryer can resolve the immutable user id.
    pub const PLEX_INVITE_TOKEN_REQUIRED: &str = "SECURITY_PLEX_INVITE_TOKEN_REQUIRED";
    /// Selected Plex user was not found on the configured account.
    pub const SELECTED_PLEX_USER_NOT_FOUND_CONFIGURED_ACCOUNT: &str =
        "SECURITY_SELECTED_PLEX_USER_NOT_FOUND_CONFIGURED_ACCOUNT";
    /// Plex account is already linked or invited.
    pub const PLEX_ACCOUNT_ALREADY_LINKED_INVITED: &str =
        "SECURITY_PLEX_ACCOUNT_ALREADY_LINKED_INVITED";
    /// Emby invites require selecting an Emby user from the picker.
    pub const EMBY_INVITES_REQUIRE_SELECTING_EMBY_USER_PICKER: &str =
        "SECURITY_EMBY_INVITES_REQUIRE_SELECTING_EMBY_USER_PICKER";
    /// Emby invites require a saved integration API key.
    pub const EMBY_INVITES_REQUIRE_SAVED_INTEGRATION_API_KEY: &str =
        "SECURITY_EMBY_INVITES_REQUIRE_SAVED_INTEGRATION_API_KEY";
    /// Selected Emby user was not found on the configured server.
    pub const SELECTED_EMBY_USER_NOT_FOUND_CONFIGURED_SERVER: &str =
        "SECURITY_SELECTED_EMBY_USER_NOT_FOUND_CONFIGURED_SERVER";
    /// Emby account is already linked or invited.
    pub const EMBY_ACCOUNT_ALREADY_LINKED_INVITED: &str =
        "SECURITY_EMBY_ACCOUNT_ALREADY_LINKED_INVITED";
    /// Emby sign-in failed.
    pub const EMBY_SIGN_FAILED: &str = "SECURITY_EMBY_SIGN_FAILED";
    /// Emby connection has no verified server identity.
    pub const EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY: &str =
        "SECURITY_EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY";
    /// Emby Connect sign-in is disabled for this connection.
    pub const EMBY_CONNECT_SIGN_DISABLED_CONNECTION: &str =
        "SECURITY_EMBY_CONNECT_SIGN_DISABLED_CONNECTION";
    /// External account is disabled and must be repaired by an administrator.
    pub const EXTERNAL_ACCOUNT_DISABLED_INVALID: &str =
        "SECURITY_EXTERNAL_ACCOUNT_DISABLED_INVALID";
    /// Jellyfin user ID is invalid.
    pub const JELLYFIN_USER_ID_INVALID: &str = "SECURITY_JELLYFIN_USER_ID_INVALID";
    /// OAuth grant is not eligible for Jellyfin linking.
    pub const OAUTH_GRANT_UNAVAILABLE: &str = "SECURITY_OAUTH_GRANT_UNAVAILABLE";
    /// OAuth client is unavailable.
    pub const OAUTH_CLIENT_UNAVAILABLE: &str = "SECURITY_OAUTH_CLIENT_UNAVAILABLE";
    /// Jellyfin connection is unavailable.
    pub const JELLYFIN_CONNECTION_UNAVAILABLE: &str = "SECURITY_JELLYFIN_CONNECTION_UNAVAILABLE";
    /// Jellyfin connection authorization changed.
    pub const JELLYFIN_CONNECTION_AUTHORIZATION_CHANGED: &str =
        "SECURITY_JELLYFIN_CONNECTION_AUTHORIZATION_CHANGED";
    /// Jellyfin user verification failed.
    pub const JELLYFIN_USER_VERIFICATION_FAILED: &str =
        "SECURITY_JELLYFIN_USER_VERIFICATION_FAILED";
    /// Jellyfin link authorization changed.
    pub const JELLYFIN_LINK_AUTHORIZATION_CHANGED: &str =
        "SECURITY_JELLYFIN_LINK_AUTHORIZATION_CHANGED";
    /// Invalid Jellyfin credentials.
    pub const JELLYFIN_CREDENTIALS_INVALID: &str = "SECURITY_JELLYFIN_CREDENTIALS_INVALID";
    /// External account is not invited.
    pub const EXTERNAL_ACCOUNT_NOT_INVITED: &str = "SECURITY_EXTERNAL_ACCOUNT_NOT_INVITED";
    /// Credentials unavailable.
    pub const CREDENTIALS_UNAVAILABLE: &str = "SECURITY_CREDENTIALS_UNAVAILABLE";
    /// External account is disabled.
    pub const EXTERNAL_ACCOUNT_DISABLED: &str = "SECURITY_EXTERNAL_ACCOUNT_DISABLED";
    /// Could not allocate a Scryer username for the external account.
    pub const COULD_NOT_ALLOCATE_SCRYER_USERNAME_EXTERNAL_ACCOUNT: &str =
        "SECURITY_COULD_NOT_ALLOCATE_SCRYER_USERNAME_EXTERNAL_ACCOUNT";
    /// External account was not found.
    pub const EXTERNAL_ACCOUNT_NOT_FOUND: &str = "SECURITY_EXTERNAL_ACCOUNT_NOT_FOUND";
    /// Cannot unlink the last external login without a local password or passkey.
    pub const LAST_EXTERNAL_LOGIN_REMOVAL_FORBIDDEN: &str =
        "SECURITY_LAST_EXTERNAL_LOGIN_REMOVAL_FORBIDDEN";
    /// Password is required.
    pub const PASSWORD_REQUIRED: &str = "SECURITY_PASSWORD_REQUIRED";
    /// Release candidate expired; search again.
    pub const RELEASE_CANDIDATE_EXPIRED: &str = "SECURITY_RELEASE_CANDIDATE_EXPIRED";
    /// Password must be at least the selected value characters.
    pub const PASSWORD_OUT_OF_RANGE: &str = "SECURITY_PASSWORD_OUT_OF_RANGE";
    /// Invalid v2 password hash.
    pub const V2_PASSWORD_HASH_INVALID: &str = "SECURITY_V2_PASSWORD_HASH_INVALID";
    /// Unsupported password hash version.
    pub const PASSWORD_HASH_VERSION_UNSUPPORTED: &str =
        "SECURITY_PASSWORD_HASH_VERSION_UNSUPPORTED";
    /// Unknown actor capability claim.
    pub const ACTOR_CAPABILITY_CLAIM_UNKNOWN: &str = "SECURITY_ACTOR_CAPABILITY_CLAIM_UNKNOWN";
    /// Token subject no longer exists.
    pub const TOKEN_SUBJECT_NO_LONGER_EXISTS: &str = "SECURITY_TOKEN_SUBJECT_NO_LONGER_EXISTS";
    /// Session duration must be between 1 and 365 days.
    pub const SESSION_DURATION_OUT_OF_RANGE: &str = "SECURITY_SESSION_DURATION_OUT_OF_RANGE";
    /// Authentication session was invalidated.
    pub const AUTHENTICATION_SESSION_INVALIDATED: &str =
        "SECURITY_AUTHENTICATION_SESSION_INVALIDATED";
    /// Cannot resolve backup download signing key for actor the selected value.
    pub const CANNOT_RESOLVE_BACKUP_DOWNLOAD_SIGNING_KEY_ACTOR: &str =
        "SECURITY_CANNOT_RESOLVE_BACKUP_DOWNLOAD_SIGNING_KEY_ACTOR";
    /// Cannot resolve release candidate signing key for actor the selected value.
    pub const CANNOT_RESOLVE_RELEASE_CANDIDATE_SIGNING_KEY_ACTOR: &str =
        "SECURITY_CANNOT_RESOLVE_RELEASE_CANDIDATE_SIGNING_KEY_ACTOR";
    /// Release candidate token missing episode scope id.
    pub const RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SCOPE_ID: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SCOPE_ID";
    /// Release candidate token missing episode-set scope id.
    pub const RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SET_SCOPE_ID: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SET_SCOPE_ID";
    /// Release candidate token has empty episode-set scope.
    pub const RELEASE_CANDIDATE_TOKEN_EMPTY_EPISODE_SET_SCOPE: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_EMPTY_EPISODE_SET_SCOPE";
    /// Release candidate token missing collection scope id.
    pub const RELEASE_CANDIDATE_TOKEN_MISSING_COLLECTION_SCOPE_ID: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_COLLECTION_SCOPE_ID";
    /// Release candidate token missing series movie scope id.
    pub const RELEASE_CANDIDATE_TOKEN_MISSING_SERIES_MOVIE_SCOPE_ID: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_SERIES_MOVIE_SCOPE_ID";
    /// Release candidate token has unknown scope.
    pub const RELEASE_CANDIDATE_TOKEN_UNKNOWN_SCOPE: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_UNKNOWN_SCOPE";
    /// Release candidate token requires a source hint.
    pub const RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_HINT: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_HINT";
    /// Release candidate token requires a source title.
    pub const RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_TITLE: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_TITLE";
    /// Release candidate token scope does not match request.
    pub const RELEASE_CANDIDATE_TOKEN_SCOPE_DOES_NOT_MATCH_REQUEST: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_SCOPE_DOES_NOT_MATCH_REQUEST";
    /// Malformed backup download token.
    pub const MALFORMED_BACKUP_DOWNLOAD_TOKEN: &str = "SECURITY_MALFORMED_BACKUP_DOWNLOAD_TOKEN";
    /// Backup download token subject is empty.
    pub const BACKUP_DOWNLOAD_TOKEN_SUBJECT_EMPTY: &str =
        "SECURITY_BACKUP_DOWNLOAD_TOKEN_SUBJECT_EMPTY";
    /// Invalid backup download token.
    pub const BACKUP_DOWNLOAD_TOKEN_INVALID: &str = "SECURITY_BACKUP_DOWNLOAD_TOKEN_INVALID";
    /// Invalid backup download token kind.
    pub const BACKUP_DOWNLOAD_TOKEN_KIND_INVALID: &str =
        "SECURITY_BACKUP_DOWNLOAD_TOKEN_KIND_INVALID";
    /// Backup download token subject does not match actor.
    pub const BACKUP_DOWNLOAD_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR: &str =
        "SECURITY_BACKUP_DOWNLOAD_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR";
    /// Backup download token filename does not match request.
    pub const BACKUP_DOWNLOAD_TOKEN_FILENAME_DOES_NOT_MATCH_REQUEST: &str =
        "SECURITY_BACKUP_DOWNLOAD_TOKEN_FILENAME_DOES_NOT_MATCH_REQUEST";
    /// Unknown backup download subject.
    pub const BACKUP_DOWNLOAD_SUBJECT_UNKNOWN: &str = "SECURITY_BACKUP_DOWNLOAD_SUBJECT_UNKNOWN";
    /// Invalid release candidate token kind.
    pub const RELEASE_CANDIDATE_TOKEN_KIND_INVALID: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_KIND_INVALID";
    /// Release candidate token subject does not match actor.
    pub const RELEASE_CANDIDATE_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR";
    /// Release candidate token title does not match request.
    pub const RELEASE_CANDIDATE_TOKEN_TITLE_DOES_NOT_MATCH_REQUEST: &str =
        "SECURITY_RELEASE_CANDIDATE_TOKEN_TITLE_DOES_NOT_MATCH_REQUEST";
    /// Release reports the selected value seeders, below the minimum of the selected value for this indexer.
    pub const RELEASE_REPORTS_SEEDERS_BELOW_MINIMUM_INDEXER: &str =
        "SECURITY_RELEASE_REPORTS_SEEDERS_BELOW_MINIMUM_INDEXER";
    /// Malformed token.
    pub const MALFORMED_TOKEN: &str = "SECURITY_MALFORMED_TOKEN";
    /// Unknown token subject.
    pub const TOKEN_SUBJECT_UNKNOWN: &str = "SECURITY_TOKEN_SUBJECT_UNKNOWN";
    /// Invalid token.
    pub const TOKEN_INVALID: &str = "SECURITY_TOKEN_INVALID";
    /// Invalid OAuth token claims.
    pub const OAUTH_TOKEN_CLAIMS_INVALID: &str = "SECURITY_OAUTH_TOKEN_CLAIMS_INVALID";
    /// OAuth tokens cannot carry app permissions.
    pub const OAUTH_TOKENS_CANNOT_CARRY_APP_PERMISSIONS: &str =
        "SECURITY_OAUTH_TOKENS_CANNOT_CARRY_APP_PERMISSIONS";
    /// Invalid password-change enrollment token claims.
    pub const PASSWORD_CHANGE_ENROLLMENT_TOKEN_CLAIMS_INVALID: &str =
        "SECURITY_PASSWORD_CHANGE_ENROLLMENT_TOKEN_CLAIMS_INVALID";
    /// OAuth tokens cannot carry actor capabilities.
    pub const OAUTH_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES: &str =
        "SECURITY_OAUTH_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES";
    /// Restricted authentication tokens cannot carry actor capabilities.
    pub const RESTRICTED_AUTHENTICATION_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES: &str =
        "SECURITY_RESTRICTED_AUTHENTICATION_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES";
    /// Username is required.
    pub const USERNAME_REQUIRED: &str = "SECURITY_USERNAME_REQUIRED";
    /// Invalid credentials.
    pub const CREDENTIALS_INVALID: &str = "SECURITY_CREDENTIALS_INVALID";
    /// Current password is required for reauthentication.
    pub const CURRENT_PASSWORD_REQUIRED_REAUTHENTICATION: &str =
        "SECURITY_CURRENT_PASSWORD_REQUIRED_REAUTHENTICATION";
    /// Password reauthentication is unavailable for this account.
    pub const PASSWORD_REAUTHENTICATION_UNAVAILABLE_ACCOUNT: &str =
        "SECURITY_PASSWORD_REAUTHENTICATION_UNAVAILABLE_ACCOUNT";
    /// Invalid current password.
    pub const CURRENT_PASSWORD_INVALID: &str = "SECURITY_CURRENT_PASSWORD_INVALID";
    /// TOTP is already enabled.
    pub const TOTP_ALREADY_ENABLED: &str = "SECURITY_TOTP_ALREADY_ENABLED";
    /// Totp challenge was not found.
    pub const TOTP_CHALLENGE_NOT_FOUND: &str = "SECURITY_TOTP_CHALLENGE_NOT_FOUND";
    /// TOTP enrollment challenge has expired.
    pub const TOTP_ENROLLMENT_CHALLENGE_EXPIRED: &str =
        "SECURITY_TOTP_ENROLLMENT_CHALLENGE_EXPIRED";
    /// Backup password is required for full backups.
    pub const BACKUP_PASSWORD_REQUIRED_FULL_BACKUPS: &str =
        "SECURITY_BACKUP_PASSWORD_REQUIRED_FULL_BACKUPS";
    /// Backup export requires a configured encryption master key.
    pub const BACKUP_EXPORT_REQUIRES_CONFIGURED_ENCRYPTION_MASTER_KEY: &str =
        "SECURITY_BACKUP_EXPORT_REQUIRES_CONFIGURED_ENCRYPTION_MASTER_KEY";
    /// A manual backup is already running.
    pub const MANUAL_BACKUP_ALREADY_RUNNING: &str = "SECURITY_MANUAL_BACKUP_ALREADY_RUNNING";
    /// Invalid backup filename.
    pub const BACKUP_FILENAME_INVALID: &str = "SECURITY_BACKUP_FILENAME_INVALID";
    /// Backup metadata was not found.
    pub const BACKUP_METADATA_NOT_FOUND: &str = "SECURITY_BACKUP_METADATA_NOT_FOUND";
    /// Backup bundle is still being created.
    pub const BACKUP_BUNDLE_STILL_BEING_CREATED: &str =
        "SECURITY_BACKUP_BUNDLE_STILL_BEING_CREATED";
    /// Backup bundle was not found.
    pub const BACKUP_BUNDLE_NOT_FOUND: &str = "SECURITY_BACKUP_BUNDLE_NOT_FOUND";
    /// Failed to resolve the configured local backup time.
    pub const FAILED_RESOLVE_CONFIGURED_LOCAL_BACKUP_TIME: &str =
        "SECURITY_FAILED_RESOLVE_CONFIGURED_LOCAL_BACKUP_TIME";
    /// Failed to compute next backup day.
    pub const FAILED_COMPUTE_NEXT_BACKUP_DAY: &str = "SECURITY_FAILED_COMPUTE_NEXT_BACKUP_DAY";
    /// Backup encryption metadata is invalid.
    pub const BACKUP_ENCRYPTION_METADATA_INVALID: &str =
        "SECURITY_BACKUP_ENCRYPTION_METADATA_INVALID";
    /// Encrypted backup payload is truncated or invalid.
    pub const ENCRYPTED_BACKUP_PAYLOAD_TRUNCATED_INVALID: &str =
        "SECURITY_ENCRYPTED_BACKUP_PAYLOAD_TRUNCATED_INVALID";
    /// Failed to decrypt backup bundle: wrong password or corrupted data.
    pub const FAILED_DECRYPT_BACKUP_BUNDLE: &str = "SECURITY_FAILED_DECRYPT_BACKUP_BUNDLE";
    /// Backup bundle table set does not match the current restore catalog: missing [the selected value], unexpected [the selected value]. the selected value.
    pub const BACKUP_BUNDLE_TABLE_SET_DOES_NOT_MATCH_CURRENT_RESTORE_CATALOG: &str =
        "SECURITY_BACKUP_BUNDLE_TABLE_SET_DOES_NOT_MATCH_CURRENT_RESTORE_CATALOG";
    /// Backup export requires a non-empty password.
    pub const BACKUP_EXPORT_REQUIRES_NON_EMPTY_PASSWORD: &str =
        "SECURITY_BACKUP_EXPORT_REQUIRES_NON_EMPTY_PASSWORD";
    /// This backup bundle is encrypted and requires a password.
    pub const BACKUP_BUNDLE_ENCRYPTED_REQUIRES_PASSWORD: &str =
        "SECURITY_BACKUP_BUNDLE_ENCRYPTED_REQUIRES_PASSWORD";
    /// Login verification challenge is unavailable.
    pub const LOGIN_VERIFICATION_CHALLENGE_UNAVAILABLE: &str =
        "SECURITY_LOGIN_VERIFICATION_CHALLENGE_UNAVAILABLE";
    /// Login verification challenge has expired.
    pub const LOGIN_VERIFICATION_CHALLENGE_EXPIRED: &str =
        "SECURITY_LOGIN_VERIFICATION_CHALLENGE_EXPIRED";
    /// Requested login verification factor is unavailable.
    pub const REQUESTED_LOGIN_VERIFICATION_FACTOR_UNAVAILABLE: &str =
        "SECURITY_REQUESTED_LOGIN_VERIFICATION_FACTOR_UNAVAILABLE";
    /// Login verification user is unavailable.
    pub const LOGIN_VERIFICATION_USER_UNAVAILABLE: &str =
        "SECURITY_LOGIN_VERIFICATION_USER_UNAVAILABLE";
    /// Login verification challenge was invalidated.
    pub const LOGIN_VERIFICATION_CHALLENGE_INVALIDATED: &str =
        "SECURITY_LOGIN_VERIFICATION_CHALLENGE_INVALIDATED";
    /// Login verification challenge was already used.
    pub const LOGIN_VERIFICATION_CHALLENGE_ALREADY_USED: &str =
        "SECURITY_LOGIN_VERIFICATION_CHALLENGE_ALREADY_USED";
    /// Passkey authentication is not configured.
    pub const PASSKEY_AUTHENTICATION_NOT_CONFIGURED: &str =
        "SECURITY_PASSKEY_AUTHENTICATION_NOT_CONFIGURED";
    /// Passkey authentication is unavailable while form login is disabled.
    pub const PASSKEY_AUTHENTICATION_UNAVAILABLE_WHILE_FORM_LOGIN_DISABLED: &str =
        "SECURITY_PASSKEY_AUTHENTICATION_UNAVAILABLE_WHILE_FORM_LOGIN_DISABLED";
    /// Failed to start passkey registration.
    pub const FAILED_START_PASSKEY_REGISTRATION: &str =
        "SECURITY_FAILED_START_PASSKEY_REGISTRATION";
    /// Passkey challenge was not found.
    pub const PASSKEY_CHALLENGE_NOT_FOUND: &str = "SECURITY_PASSKEY_CHALLENGE_NOT_FOUND";
    /// Passkey challenge does not belong to the current user.
    pub const PASSKEY_CHALLENGE_DOES_NOT_BELONG_CURRENT_USER: &str =
        "SECURITY_PASSKEY_CHALLENGE_DOES_NOT_BELONG_CURRENT_USER";
    /// Passkey challenge is not a registration ceremony.
    pub const PASSKEY_CHALLENGE_NOT_REGISTRATION_CEREMONY: &str =
        "SECURITY_PASSKEY_CHALLENGE_NOT_REGISTRATION_CEREMONY";
    /// Passkey challenge has an unexpected registration purpose.
    pub const PASSKEY_CHALLENGE_UNEXPECTED_REGISTRATION_PURPOSE: &str =
        "SECURITY_PASSKEY_CHALLENGE_UNEXPECTED_REGISTRATION_PURPOSE";
    /// Passkey challenge has expired.
    pub const PASSKEY_CHALLENGE_EXPIRED: &str = "SECURITY_PASSKEY_CHALLENGE_EXPIRED";
    /// Invalid passkey registration response payload.
    pub const PASSKEY_REGISTRATION_RESPONSE_PAYLOAD_INVALID: &str =
        "SECURITY_PASSKEY_REGISTRATION_RESPONSE_PAYLOAD_INVALID";
    /// Passkey challenge stored an authentication ceremony.
    pub const PASSKEY_CHALLENGE_STORED_AUTHENTICATION_CEREMONY: &str =
        "SECURITY_PASSKEY_CHALLENGE_STORED_AUTHENTICATION_CEREMONY";
    /// Failed to finish passkey registration.
    pub const FAILED_FINISH_PASSKEY_REGISTRATION: &str =
        "SECURITY_FAILED_FINISH_PASSKEY_REGISTRATION";
    /// A passkey with this credential id is already registered.
    pub const PASSKEY_CREDENTIAL_ID_ALREADY_REGISTERED: &str =
        "SECURITY_PASSKEY_CREDENTIAL_ID_ALREADY_REGISTERED";
    /// Passkey reauthentication is unavailable for this account.
    pub const PASSKEY_REAUTHENTICATION_UNAVAILABLE_ACCOUNT: &str =
        "SECURITY_PASSKEY_REAUTHENTICATION_UNAVAILABLE_ACCOUNT";
    /// Failed to start passkey authentication.
    pub const FAILED_START_PASSKEY_AUTHENTICATION: &str =
        "SECURITY_FAILED_START_PASSKEY_AUTHENTICATION";
    /// Failed to start discoverable passkey authentication.
    pub const FAILED_START_DISCOVERABLE_PASSKEY_AUTHENTICATION: &str =
        "SECURITY_FAILED_START_DISCOVERABLE_PASSKEY_AUTHENTICATION";
    /// Passkey reauthentication challenge belongs to another user.
    pub const PASSKEY_REAUTHENTICATION_CHALLENGE_BELONGS_ANOTHER_USER: &str =
        "SECURITY_PASSKEY_REAUTHENTICATION_CHALLENGE_BELONGS_ANOTHER_USER";
    /// Passkey challenge is not an authentication ceremony.
    pub const PASSKEY_CHALLENGE_NOT_AUTHENTICATION_CEREMONY: &str =
        "SECURITY_PASSKEY_CHALLENGE_NOT_AUTHENTICATION_CEREMONY";
    /// Passkey challenge has an unexpected authentication purpose.
    pub const PASSKEY_CHALLENGE_UNEXPECTED_AUTHENTICATION_PURPOSE: &str =
        "SECURITY_PASSKEY_CHALLENGE_UNEXPECTED_AUTHENTICATION_PURPOSE";
    /// Passkey challenge does not belong to this login verification.
    pub const PASSKEY_CHALLENGE_DOES_NOT_BELONG_LOGIN_VERIFICATION: &str =
        "SECURITY_PASSKEY_CHALLENGE_DOES_NOT_BELONG_LOGIN_VERIFICATION";
    /// Passkey challenge is not actor-bound.
    pub const PASSKEY_CHALLENGE_NOT_ACTOR_BOUND: &str =
        "SECURITY_PASSKEY_CHALLENGE_NOT_ACTOR_BOUND";
    /// Passkey challenge session has been invalidated.
    pub const PASSKEY_CHALLENGE_SESSION_INVALIDATED: &str =
        "SECURITY_PASSKEY_CHALLENGE_SESSION_INVALIDATED";
    /// Invalid passkey authentication response payload.
    pub const PASSKEY_AUTHENTICATION_RESPONSE_PAYLOAD_INVALID: &str =
        "SECURITY_PASSKEY_AUTHENTICATION_RESPONSE_PAYLOAD_INVALID";
    /// Passkey challenge stored a registration ceremony.
    pub const PASSKEY_CHALLENGE_STORED_REGISTRATION_CEREMONY: &str =
        "SECURITY_PASSKEY_CHALLENGE_STORED_REGISTRATION_CEREMONY";
    /// Passkey credential was not found.
    pub const PASSKEY_CREDENTIAL_NOT_FOUND: &str = "SECURITY_PASSKEY_CREDENTIAL_NOT_FOUND";
    /// Passkey credential does not belong to the challenge user.
    pub const PASSKEY_CREDENTIAL_DOES_NOT_BELONG_CHALLENGE_USER: &str =
        "SECURITY_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_CHALLENGE_USER";
    /// Failed to finish passkey authentication.
    pub const FAILED_FINISH_PASSKEY_AUTHENTICATION: &str =
        "SECURITY_FAILED_FINISH_PASSKEY_AUTHENTICATION";
    /// Passkey credential changed during authentication.
    pub const PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION: &str =
        "SECURITY_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION";
    /// Failed to identify discoverable passkey authentication.
    pub const FAILED_IDENTIFY_DISCOVERABLE_PASSKEY_AUTHENTICATION: &str =
        "SECURITY_FAILED_IDENTIFY_DISCOVERABLE_PASSKEY_AUTHENTICATION";
    /// Discoverable passkey credential was not found.
    pub const DISCOVERABLE_PASSKEY_CREDENTIAL_NOT_FOUND: &str =
        "SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_NOT_FOUND";
    /// Discoverable passkey credential does not belong to the resolved user.
    pub const DISCOVERABLE_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_RESOLVED_USER: &str =
        "SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_RESOLVED_USER";
    /// Failed to finish discoverable passkey authentication.
    pub const FAILED_FINISH_DISCOVERABLE_PASSKEY_AUTHENTICATION: &str =
        "SECURITY_FAILED_FINISH_DISCOVERABLE_PASSKEY_AUTHENTICATION";
    /// Discoverable passkey credential changed during authentication.
    pub const DISCOVERABLE_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION: &str =
        "SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION";
    /// Passkey factor is no longer available.
    pub const PASSKEY_FACTOR_UNAVAILABLE: &str = "SECURITY_PASSKEY_FACTOR_UNAVAILABLE";
    /// Passkey credential does not belong to this login verification.
    pub const PASSKEY_CREDENTIAL_DOES_NOT_BELONG_LOGIN_VERIFICATION: &str =
        "SECURITY_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_LOGIN_VERIFICATION";
    /// Anonymous is reserved for authless audit attribution.
    pub const ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION: &str =
        "SECURITY_ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION";
    /// The requested value is reserved for instance recovery.
    pub const RESERVED_INSTANCE_RECOVERY: &str = "SECURITY_RESERVED_INSTANCE_RECOVERY";
    /// Admin username is required.
    pub const ADMIN_USERNAME_REQUIRED: &str = "SECURITY_ADMIN_USERNAME_REQUIRED";
    /// Admin password is required.
    pub const ADMIN_PASSWORD_REQUIRED: &str = "SECURITY_ADMIN_PASSWORD_REQUIRED";
    /// Admin is disabled by SCRYER_DISABLE_DEFAULT_ADMIN; remove that setting and restart before using the default administrator.
    pub const ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN: &str =
        "SECURITY_ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN";
    /// SCRYER_ADMIN_USERNAME must name a non-reserved local administrator.
    pub const SCRYER_ADMIN_USERNAME_MUST_NAME_NON_RESERVED_LOCAL_ADMINISTRATOR: &str =
        "SECURITY_SCRYER_ADMIN_USERNAME_MUST_NAME_NON_RESERVED_LOCAL_ADMINISTRATOR";
    /// SCRYER_ADMIN_PASSWORD_RESET requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE.
    pub const ADMIN_RESET_PASSWORD_REQUIRED: &str = "SECURITY_ADMIN_RESET_PASSWORD_REQUIRED";
    /// Cannot provision or select admin while SCRYER_DISABLE_DEFAULT_ADMIN is true.
    pub const CANNOT_PROVISION_SELECT_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE: &str =
        "SECURITY_CANNOT_PROVISION_SELECT_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE";
    /// SCRYER_ADMIN_USERNAME conflicts with an existing username; use its exact stored spelling.
    pub const SCRYER_ADMIN_USERNAME_CONFLICTS_EXISTING_USERNAME: &str =
        "SECURITY_SCRYER_ADMIN_USERNAME_CONFLICTS_EXISTING_USERNAME";
    /// Selected bootstrap account must already be an enabled local full administrator; use recovery for lockouts.
    pub const SELECTED_BOOTSTRAP_ACCOUNT_MUST_ALREADY_ENABLED_LOCAL_FULL_ADMINISTRATOR: &str =
        "SECURITY_SELECTED_BOOTSTRAP_ACCOUNT_MUST_ALREADY_ENABLED_LOCAL_FULL_ADMINISTRATOR";
    /// New bootstrap administrator requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE.
    pub const BOOTSTRAP_ADMIN_PASSWORD_REQUIRED: &str =
        "SECURITY_BOOTSTRAP_ADMIN_PASSWORD_REQUIRED";
    /// SCRYER_DISABLE_DEFAULT_ADMIN requires another enabled local full administrator with a usable password.
    pub const ALTERNATIVE_LOCAL_ADMIN_REQUIRED: &str = "SECURITY_ALTERNATIVE_LOCAL_ADMIN_REQUIRED";
    /// Bootstrap administrator has no usable credentials.
    pub const BOOTSTRAP_ADMINISTRATOR_NO_USABLE_CREDENTIALS: &str =
        "SECURITY_BOOTSTRAP_ADMINISTRATOR_NO_USABLE_CREDENTIALS";
    /// Cannot disable admin without another usable administrator.
    pub const CANNOT_DISABLE_ADMIN_WITHOUT_ANOTHER_USABLE_ADMINISTRATOR: &str =
        "SECURITY_CANNOT_DISABLE_ADMIN_WITHOUT_ANOTHER_USABLE_ADMINISTRATOR";
    /// Recovery admin user is externally managed and cannot be repaired for local login.
    pub const RECOVERY_ADMIN_USER_EXTERNALLY_MANAGED_CANNOT_REPAIRED_LOCAL_LOGIN: &str =
        "SECURITY_RECOVERY_ADMIN_USER_EXTERNALLY_MANAGED_CANNOT_REPAIRED_LOCAL_LOGIN";
    /// Cannot create admin while SCRYER_DISABLE_DEFAULT_ADMIN is true.
    pub const CANNOT_CREATE_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE: &str =
        "SECURITY_CANNOT_CREATE_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE";
    /// User the selected value already exists.
    pub const USER_ALREADY_EXISTS: &str = "SECURITY_USER_ALREADY_EXISTS";
    /// Externally managed users cannot set a Scryer password.
    pub const EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD: &str =
        "SECURITY_EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD";
    /// Account has no password set.
    pub const ACCOUNT_NO_PASSWORD_SET: &str = "SECURITY_ACCOUNT_NO_PASSWORD_SET";
    /// Current password is incorrect.
    pub const CURRENT_PASSWORD_INCORRECT: &str = "SECURITY_CURRENT_PASSWORD_INCORRECT";
    /// Current password is required.
    pub const CURRENT_PASSWORD_REQUIRED: &str = "SECURITY_CURRENT_PASSWORD_REQUIRED";
    /// Use change_own_password to update your own password.
    pub const USE_CHANGE_OWN_PASSWORD_UPDATE_OWN_PASSWORD: &str =
        "SECURITY_USE_CHANGE_OWN_PASSWORD_UPDATE_OWN_PASSWORD";
    /// New password must differ from the temporary password.
    pub const NEW_PASSWORD_MUST_DIFFER_TEMPORARY_PASSWORD: &str =
        "SECURITY_NEW_PASSWORD_MUST_DIFFER_TEMPORARY_PASSWORD";
    /// Cannot remove your own user-management or permission-management access.
    pub const CANNOT_REMOVE_OWN_USER_MANAGEMENT_PERMISSION_MANAGEMENT_ACCESS: &str =
        "SECURITY_CANNOT_REMOVE_OWN_USER_MANAGEMENT_PERMISSION_MANAGEMENT_ACCESS";
    /// Cannot demote the last full administrator.
    pub const CANNOT_DEMOTE_LAST_FULL_ADMINISTRATOR: &str =
        "SECURITY_CANNOT_DEMOTE_LAST_FULL_ADMINISTRATOR";
    /// Cannot change login status for the current user.
    pub const CANNOT_CHANGE_LOGIN_STATUS_CURRENT_USER: &str =
        "SECURITY_CANNOT_CHANGE_LOGIN_STATUS_CURRENT_USER";
    /// Recovery-admin login is managed by the environment.
    pub const RECOVERY_ADMIN_LOGIN_MANAGED_ENVIRONMENT: &str =
        "SECURITY_RECOVERY_ADMIN_LOGIN_MANAGED_ENVIRONMENT";
    /// Cannot disable the last usable full administrator.
    pub const CANNOT_DISABLE_LAST_USABLE_FULL_ADMINISTRATOR: &str =
        "SECURITY_CANNOT_DISABLE_LAST_USABLE_FULL_ADMINISTRATOR";
    /// Cannot delete current user.
    pub const CANNOT_DELETE_CURRENT_USER: &str = "SECURITY_CANNOT_DELETE_CURRENT_USER";
    /// Cannot delete the default admin; disable its login instead.
    pub const CANNOT_DELETE_DEFAULT_ADMIN: &str = "SECURITY_CANNOT_DELETE_DEFAULT_ADMIN";
    /// Cannot delete the last full administrator.
    pub const CANNOT_DELETE_LAST_FULL_ADMINISTRATOR: &str =
        "SECURITY_CANNOT_DELETE_LAST_FULL_ADMINISTRATOR";
    /// Cannot reset your own authentication factors.
    pub const CANNOT_RESET_OWN_AUTHENTICATION_FACTORS: &str =
        "SECURITY_CANNOT_RESET_OWN_AUTHENTICATION_FACTORS";
    /// Encrypted backup header is truncated.
    pub const ENCRYPTED_BACKUP_HEADER_TRUNCATED: &str =
        "SECURITY_ENCRYPTED_BACKUP_HEADER_TRUNCATED";
    /// Unsupported encrypted backup version the selected value.
    pub const ENCRYPTED_BACKUP_VERSION_UNSUPPORTED: &str =
        "SECURITY_ENCRYPTED_BACKUP_VERSION_UNSUPPORTED";
    /// Encrypted backup metadata header is truncated.
    pub const ENCRYPTED_BACKUP_METADATA_HEADER_TRUNCATED: &str =
        "SECURITY_ENCRYPTED_BACKUP_METADATA_HEADER_TRUNCATED";
    /// Encrypted backup metadata is truncated.
    pub const ENCRYPTED_BACKUP_METADATA_TRUNCATED: &str =
        "SECURITY_ENCRYPTED_BACKUP_METADATA_TRUNCATED";
    /// Failed to derive backup encryption key from password.
    pub const FAILED_DERIVE_BACKUP_ENCRYPTION_KEY_PASSWORD: &str =
        "SECURITY_FAILED_DERIVE_BACKUP_ENCRYPTION_KEY_PASSWORD";
    /// Encrypted backup payload length is invalid.
    pub const ENCRYPTED_BACKUP_PAYLOAD_LENGTH_INVALID: &str =
        "SECURITY_ENCRYPTED_BACKUP_PAYLOAD_LENGTH_INVALID";
    /// Encrypted backup payload length is truncated.
    pub const ENCRYPTED_BACKUP_PAYLOAD_LENGTH_TRUNCATED: &str =
        "SECURITY_ENCRYPTED_BACKUP_PAYLOAD_LENGTH_TRUNCATED";
    /// Backup manifest missing.
    pub const BACKUP_MANIFEST_MISSING: &str = "SECURITY_BACKUP_MANIFEST_MISSING";
    /// Backup manifest is invalid.
    pub const BACKUP_MANIFEST_INVALID: &str = "SECURITY_BACKUP_MANIFEST_INVALID";
    /// Backup secrets payload missing.
    pub const BACKUP_SECRETS_PAYLOAD_MISSING: &str = "SECURITY_BACKUP_SECRETS_PAYLOAD_MISSING";
    /// Backup secrets payload is invalid.
    pub const BACKUP_SECRETS_PAYLOAD_INVALID: &str = "SECURITY_BACKUP_SECRETS_PAYLOAD_INVALID";
    /// Prepared backup directory contains unsupported entry the selected value.
    pub const PREPARED_BACKUP_DIRECTORY_CONTAINS_UNSUPPORTED_ENTRY: &str =
        "SECURITY_PREPARED_BACKUP_DIRECTORY_CONTAINS_UNSUPPORTED_ENTRY";
    /// Unsupported backup format version the selected value.
    pub const BACKUP_FORMAT_VERSION_UNSUPPORTED: &str =
        "SECURITY_BACKUP_FORMAT_VERSION_UNSUPPORTED";
    /// Backup part path must be a regular file.
    pub const BACKUP_PART_PATH_INVALID: &str = "SECURITY_BACKUP_PART_PATH_INVALID";
    /// Backup part path escapes the bundle root.
    pub const BACKUP_PART_PATH_ESCAPES_BUNDLE_ROOT: &str =
        "SECURITY_BACKUP_PART_PATH_ESCAPES_BUNDLE_ROOT";
    /// Backup part path is unreadable.
    pub const BACKUP_PART_PATH_UNREADABLE: &str = "SECURITY_BACKUP_PART_PATH_UNREADABLE";
    /// Backup checksum mismatch for the selected value.
    pub const BACKUP_CHECKSUM_MISMATCH: &str = "SECURITY_BACKUP_CHECKSUM_MISMATCH";
    /// Backup checksum manifest is incomplete: missing [the selected value], unexpected [the selected value].
    pub const BACKUP_CHECKSUM_MANIFEST_INCOMPLETE: &str =
        "SECURITY_BACKUP_CHECKSUM_MANIFEST_INCOMPLETE";
    /// The backup could not be created.
    pub const BACKUP_CREATION_FAILED: &str = "SECURITY_BACKUP_CREATION_FAILED";
    /// The backup bundle is invalid.
    pub const BACKUP_BUNDLE_INVALID: &str = "SECURITY_BACKUP_BUNDLE_INVALID";
    /// The external account could not be linked.
    pub const EXTERNAL_ACCOUNT_LINK_FAILED: &str = "EXTERNAL_ACCOUNT_LINK_FAILED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        EXTERNAL_ACCOUNT_LINK_FAILED,
        RELEASE_TOKEN_EXPIRED,
        RELEASE_TOKEN_INVALID,
        EXTERNAL_ACCOUNT_LINKED_ELSEWHERE,
        EXTERNAL_ACCOUNT_IDENTITY_MISMATCH,
        CONNECTION_NOT_CONFIGURED,
        CONNECTION_DISABLED,
        NOT_ENABLED,
        PLEX_SERVER_DISCOVERY_REQUIRED_BEFORE_USING_PLEX_AUTH,
        VERIFIED_EXTERNAL_IDENTITY_PROVIDER_DID_NOT_MATCH_REQUESTED_PROVIDER,
        VERIFIED_EXTERNAL_IDENTITY_CONNECTION_DID_NOT_MATCH_REQUESTED_CONNECTION,
        USER_NOT_FOUND,
        JELLYFIN_INVITES_REQUIRE_SELECTING_JELLYFIN_USER_PICKER,
        JELLYFIN_INVITE_API_KEY_REQUIRED,
        SELECTED_JELLYFIN_USER_NOT_FOUND_CONFIGURED_SERVER,
        JELLYFIN_ACCOUNT_ALREADY_LINKED_INVITED,
        PLEX_INVITES_REQUIRE_SELECTING_PLEX_USER_PICKER,
        PLEX_INVITE_TOKEN_REQUIRED,
        SELECTED_PLEX_USER_NOT_FOUND_CONFIGURED_ACCOUNT,
        PLEX_ACCOUNT_ALREADY_LINKED_INVITED,
        EMBY_INVITES_REQUIRE_SELECTING_EMBY_USER_PICKER,
        EMBY_INVITES_REQUIRE_SAVED_INTEGRATION_API_KEY,
        SELECTED_EMBY_USER_NOT_FOUND_CONFIGURED_SERVER,
        EMBY_ACCOUNT_ALREADY_LINKED_INVITED,
        EMBY_SIGN_FAILED,
        EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY,
        EMBY_CONNECT_SIGN_DISABLED_CONNECTION,
        EXTERNAL_ACCOUNT_DISABLED_INVALID,
        JELLYFIN_USER_ID_INVALID,
        OAUTH_GRANT_UNAVAILABLE,
        OAUTH_CLIENT_UNAVAILABLE,
        JELLYFIN_CONNECTION_UNAVAILABLE,
        JELLYFIN_CONNECTION_AUTHORIZATION_CHANGED,
        JELLYFIN_USER_VERIFICATION_FAILED,
        JELLYFIN_LINK_AUTHORIZATION_CHANGED,
        JELLYFIN_CREDENTIALS_INVALID,
        EXTERNAL_ACCOUNT_NOT_INVITED,
        CREDENTIALS_UNAVAILABLE,
        EXTERNAL_ACCOUNT_DISABLED,
        COULD_NOT_ALLOCATE_SCRYER_USERNAME_EXTERNAL_ACCOUNT,
        EXTERNAL_ACCOUNT_NOT_FOUND,
        LAST_EXTERNAL_LOGIN_REMOVAL_FORBIDDEN,
        PASSWORD_REQUIRED,
        RELEASE_CANDIDATE_EXPIRED,
        PASSWORD_OUT_OF_RANGE,
        V2_PASSWORD_HASH_INVALID,
        PASSWORD_HASH_VERSION_UNSUPPORTED,
        ACTOR_CAPABILITY_CLAIM_UNKNOWN,
        TOKEN_SUBJECT_NO_LONGER_EXISTS,
        SESSION_DURATION_OUT_OF_RANGE,
        AUTHENTICATION_SESSION_INVALIDATED,
        CANNOT_RESOLVE_BACKUP_DOWNLOAD_SIGNING_KEY_ACTOR,
        CANNOT_RESOLVE_RELEASE_CANDIDATE_SIGNING_KEY_ACTOR,
        RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SCOPE_ID,
        RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SET_SCOPE_ID,
        RELEASE_CANDIDATE_TOKEN_EMPTY_EPISODE_SET_SCOPE,
        RELEASE_CANDIDATE_TOKEN_MISSING_COLLECTION_SCOPE_ID,
        RELEASE_CANDIDATE_TOKEN_MISSING_SERIES_MOVIE_SCOPE_ID,
        RELEASE_CANDIDATE_TOKEN_UNKNOWN_SCOPE,
        RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_HINT,
        RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_TITLE,
        RELEASE_CANDIDATE_TOKEN_SCOPE_DOES_NOT_MATCH_REQUEST,
        MALFORMED_BACKUP_DOWNLOAD_TOKEN,
        BACKUP_DOWNLOAD_TOKEN_SUBJECT_EMPTY,
        BACKUP_DOWNLOAD_TOKEN_INVALID,
        BACKUP_DOWNLOAD_TOKEN_KIND_INVALID,
        BACKUP_DOWNLOAD_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR,
        BACKUP_DOWNLOAD_TOKEN_FILENAME_DOES_NOT_MATCH_REQUEST,
        BACKUP_DOWNLOAD_SUBJECT_UNKNOWN,
        RELEASE_CANDIDATE_TOKEN_KIND_INVALID,
        RELEASE_CANDIDATE_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR,
        RELEASE_CANDIDATE_TOKEN_TITLE_DOES_NOT_MATCH_REQUEST,
        RELEASE_REPORTS_SEEDERS_BELOW_MINIMUM_INDEXER,
        MALFORMED_TOKEN,
        TOKEN_SUBJECT_UNKNOWN,
        TOKEN_INVALID,
        OAUTH_TOKEN_CLAIMS_INVALID,
        OAUTH_TOKENS_CANNOT_CARRY_APP_PERMISSIONS,
        PASSWORD_CHANGE_ENROLLMENT_TOKEN_CLAIMS_INVALID,
        OAUTH_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES,
        RESTRICTED_AUTHENTICATION_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES,
        USERNAME_REQUIRED,
        CREDENTIALS_INVALID,
        CURRENT_PASSWORD_REQUIRED_REAUTHENTICATION,
        PASSWORD_REAUTHENTICATION_UNAVAILABLE_ACCOUNT,
        CURRENT_PASSWORD_INVALID,
        TOTP_ALREADY_ENABLED,
        TOTP_CHALLENGE_NOT_FOUND,
        TOTP_ENROLLMENT_CHALLENGE_EXPIRED,
        BACKUP_PASSWORD_REQUIRED_FULL_BACKUPS,
        BACKUP_EXPORT_REQUIRES_CONFIGURED_ENCRYPTION_MASTER_KEY,
        MANUAL_BACKUP_ALREADY_RUNNING,
        BACKUP_FILENAME_INVALID,
        BACKUP_METADATA_NOT_FOUND,
        BACKUP_BUNDLE_STILL_BEING_CREATED,
        BACKUP_BUNDLE_NOT_FOUND,
        FAILED_RESOLVE_CONFIGURED_LOCAL_BACKUP_TIME,
        FAILED_COMPUTE_NEXT_BACKUP_DAY,
        BACKUP_ENCRYPTION_METADATA_INVALID,
        ENCRYPTED_BACKUP_PAYLOAD_TRUNCATED_INVALID,
        FAILED_DECRYPT_BACKUP_BUNDLE,
        BACKUP_BUNDLE_TABLE_SET_DOES_NOT_MATCH_CURRENT_RESTORE_CATALOG,
        BACKUP_EXPORT_REQUIRES_NON_EMPTY_PASSWORD,
        BACKUP_BUNDLE_ENCRYPTED_REQUIRES_PASSWORD,
        LOGIN_VERIFICATION_CHALLENGE_UNAVAILABLE,
        LOGIN_VERIFICATION_CHALLENGE_EXPIRED,
        REQUESTED_LOGIN_VERIFICATION_FACTOR_UNAVAILABLE,
        LOGIN_VERIFICATION_USER_UNAVAILABLE,
        LOGIN_VERIFICATION_CHALLENGE_INVALIDATED,
        LOGIN_VERIFICATION_CHALLENGE_ALREADY_USED,
        PASSKEY_AUTHENTICATION_NOT_CONFIGURED,
        PASSKEY_AUTHENTICATION_UNAVAILABLE_WHILE_FORM_LOGIN_DISABLED,
        FAILED_START_PASSKEY_REGISTRATION,
        PASSKEY_CHALLENGE_NOT_FOUND,
        PASSKEY_CHALLENGE_DOES_NOT_BELONG_CURRENT_USER,
        PASSKEY_CHALLENGE_NOT_REGISTRATION_CEREMONY,
        PASSKEY_CHALLENGE_UNEXPECTED_REGISTRATION_PURPOSE,
        PASSKEY_CHALLENGE_EXPIRED,
        PASSKEY_REGISTRATION_RESPONSE_PAYLOAD_INVALID,
        PASSKEY_CHALLENGE_STORED_AUTHENTICATION_CEREMONY,
        FAILED_FINISH_PASSKEY_REGISTRATION,
        PASSKEY_CREDENTIAL_ID_ALREADY_REGISTERED,
        PASSKEY_REAUTHENTICATION_UNAVAILABLE_ACCOUNT,
        FAILED_START_PASSKEY_AUTHENTICATION,
        FAILED_START_DISCOVERABLE_PASSKEY_AUTHENTICATION,
        PASSKEY_REAUTHENTICATION_CHALLENGE_BELONGS_ANOTHER_USER,
        PASSKEY_CHALLENGE_NOT_AUTHENTICATION_CEREMONY,
        PASSKEY_CHALLENGE_UNEXPECTED_AUTHENTICATION_PURPOSE,
        PASSKEY_CHALLENGE_DOES_NOT_BELONG_LOGIN_VERIFICATION,
        PASSKEY_CHALLENGE_NOT_ACTOR_BOUND,
        PASSKEY_CHALLENGE_SESSION_INVALIDATED,
        PASSKEY_AUTHENTICATION_RESPONSE_PAYLOAD_INVALID,
        PASSKEY_CHALLENGE_STORED_REGISTRATION_CEREMONY,
        PASSKEY_CREDENTIAL_NOT_FOUND,
        PASSKEY_CREDENTIAL_DOES_NOT_BELONG_CHALLENGE_USER,
        FAILED_FINISH_PASSKEY_AUTHENTICATION,
        PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION,
        FAILED_IDENTIFY_DISCOVERABLE_PASSKEY_AUTHENTICATION,
        DISCOVERABLE_PASSKEY_CREDENTIAL_NOT_FOUND,
        DISCOVERABLE_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_RESOLVED_USER,
        FAILED_FINISH_DISCOVERABLE_PASSKEY_AUTHENTICATION,
        DISCOVERABLE_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION,
        PASSKEY_FACTOR_UNAVAILABLE,
        PASSKEY_CREDENTIAL_DOES_NOT_BELONG_LOGIN_VERIFICATION,
        ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION,
        RESERVED_INSTANCE_RECOVERY,
        ADMIN_USERNAME_REQUIRED,
        ADMIN_PASSWORD_REQUIRED,
        ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN,
        SCRYER_ADMIN_USERNAME_MUST_NAME_NON_RESERVED_LOCAL_ADMINISTRATOR,
        ADMIN_RESET_PASSWORD_REQUIRED,
        CANNOT_PROVISION_SELECT_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE,
        SCRYER_ADMIN_USERNAME_CONFLICTS_EXISTING_USERNAME,
        SELECTED_BOOTSTRAP_ACCOUNT_MUST_ALREADY_ENABLED_LOCAL_FULL_ADMINISTRATOR,
        BOOTSTRAP_ADMIN_PASSWORD_REQUIRED,
        ALTERNATIVE_LOCAL_ADMIN_REQUIRED,
        BOOTSTRAP_ADMINISTRATOR_NO_USABLE_CREDENTIALS,
        CANNOT_DISABLE_ADMIN_WITHOUT_ANOTHER_USABLE_ADMINISTRATOR,
        RECOVERY_ADMIN_USER_EXTERNALLY_MANAGED_CANNOT_REPAIRED_LOCAL_LOGIN,
        CANNOT_CREATE_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE,
        USER_ALREADY_EXISTS,
        EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD,
        ACCOUNT_NO_PASSWORD_SET,
        CURRENT_PASSWORD_INCORRECT,
        CURRENT_PASSWORD_REQUIRED,
        USE_CHANGE_OWN_PASSWORD_UPDATE_OWN_PASSWORD,
        NEW_PASSWORD_MUST_DIFFER_TEMPORARY_PASSWORD,
        CANNOT_REMOVE_OWN_USER_MANAGEMENT_PERMISSION_MANAGEMENT_ACCESS,
        CANNOT_DEMOTE_LAST_FULL_ADMINISTRATOR,
        CANNOT_CHANGE_LOGIN_STATUS_CURRENT_USER,
        RECOVERY_ADMIN_LOGIN_MANAGED_ENVIRONMENT,
        CANNOT_DISABLE_LAST_USABLE_FULL_ADMINISTRATOR,
        CANNOT_DELETE_CURRENT_USER,
        CANNOT_DELETE_DEFAULT_ADMIN,
        CANNOT_DELETE_LAST_FULL_ADMINISTRATOR,
        CANNOT_RESET_OWN_AUTHENTICATION_FACTORS,
        ENCRYPTED_BACKUP_HEADER_TRUNCATED,
        ENCRYPTED_BACKUP_VERSION_UNSUPPORTED,
        ENCRYPTED_BACKUP_METADATA_HEADER_TRUNCATED,
        ENCRYPTED_BACKUP_METADATA_TRUNCATED,
        FAILED_DERIVE_BACKUP_ENCRYPTION_KEY_PASSWORD,
        ENCRYPTED_BACKUP_PAYLOAD_LENGTH_INVALID,
        ENCRYPTED_BACKUP_PAYLOAD_LENGTH_TRUNCATED,
        BACKUP_MANIFEST_MISSING,
        BACKUP_MANIFEST_INVALID,
        BACKUP_SECRETS_PAYLOAD_MISSING,
        BACKUP_SECRETS_PAYLOAD_INVALID,
        PREPARED_BACKUP_DIRECTORY_CONTAINS_UNSUPPORTED_ENTRY,
        BACKUP_FORMAT_VERSION_UNSUPPORTED,
        BACKUP_PART_PATH_INVALID,
        BACKUP_PART_PATH_ESCAPES_BUNDLE_ROOT,
        BACKUP_PART_PATH_UNREADABLE,
        BACKUP_CHECKSUM_MISMATCH,
        BACKUP_CHECKSUM_MANIFEST_INCOMPLETE,
        BACKUP_CREATION_FAILED,
        BACKUP_BUNDLE_INVALID,
    ];
}

pub mod rules {
    /// The rule pack is already up to date.
    pub const PACK_UP_TO_DATE: &str = "RULE_PACK_UP_TO_DATE";
    /// The rule uses a retired input field.
    pub const INPUT_RETIRED: &str = "RULE_INPUT_RETIRED";
    /// Request rules cannot target a person.
    pub const TARGETS_PERSON: &str = "RULE_TARGETS_PERSON";
    /// Rule validation failed.
    pub const VALIDATION_FAILED: &str = "RULE_VALIDATION_FAILED";
    /// At least one rule set field must be provided.
    pub const SET_FIELD_REQUIRED: &str = "RULE_SET_FIELD_REQUIRED";
    /// Rule set was not found.
    pub const SET_NOT_FOUND: &str = "RULE_SET_NOT_FOUND";
    /// This rule is tracked by a community pack. Change its settings through the tracked pack so membership revisions remain atomic, or copy it to customize its content.
    pub const TRACKED_COMMUNITY_PACK: &str = "RULE_TRACKED_COMMUNITY_PACK";
    /// This rule is managed by a convenience setting. Change the setting instead of editing the rule directly.
    pub const MANAGED_CONVENIENCE_SETTING: &str = "RULE_MANAGED_CONVENIENCE_SETTING";
    /// Managed TRaSH Guides locale packs use their predefined locale policy and cannot be filtered.
    pub const MANAGED_LOCALE_FILTER_FORBIDDEN: &str = "RULE_MANAGED_LOCALE_FILTER_FORBIDDEN";
    /// A tag filter only applies to managed rule sets.
    pub const TAG_FILTER_ONLY_APPLIES_MANAGED_RULE_SETS: &str =
        "RULE_TAG_FILTER_ONLY_APPLIES_MANAGED_RULE_SETS";
    /// Rule validation error.
    pub const VALIDATION_ERROR: &str = "RULE_VALIDATION_ERROR";
    /// Only one enabled rule may use exclusive group the selected value.
    pub const ONLY_ONE_ENABLED_RULE_MAY_USE_EXCLUSIVE_GROUP: &str =
        "RULE_ONLY_ONE_ENABLED_RULE_MAY_USE_EXCLUSIVE_GROUP";
    /// Failed to build rules engine.
    pub const FAILED_BUILD_RULES_ENGINE: &str = "RULE_FAILED_BUILD_RULES_ENGINE";
    /// Media file  of title was not found.
    pub const MEDIA_FILE_NOT_FOUND: &str = "RULE_MEDIA_FILE_NOT_FOUND";
    /// Media file the selected value does not cover episode the selected value.
    pub const MEDIA_FILE_DOES_NOT_COVER_EPISODE: &str = "RULE_MEDIA_FILE_DOES_NOT_COVER_EPISODE";
    /// Episode was not found.
    pub const EPISODE_NOT_FOUND: &str = "RULE_EPISODE_NOT_FOUND";
    /// Episode the selected value does not belong to title the selected value.
    pub const EPISODE_DOES_NOT_BELONG_TITLE: &str = "RULE_EPISODE_DOES_NOT_BELONG_TITLE";
    /// An episode cannot be selected when testing a movie rule.
    pub const EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE: &str =
        "RULE_EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE";
    /// An episode is required when testing a series or anime rule.
    pub const EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE: &str =
        "RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE";
    /// Testing a saved rule cannot include a draft, edit, or copy context.
    pub const TESTING_SAVED_RULE_CANNOT_INCLUDE_DRAFT_EDIT_COPY_CONTEXT: &str =
        "RULE_TESTING_SAVED_RULE_CANNOT_INCLUDE_DRAFT_EDIT_COPY_CONTEXT";
    /// A preview requires either a draft or a saved rule set.
    pub const PREVIEW_REQUIRES_EITHER_DRAFT_SAVED_RULE_SET: &str =
        "RULE_PREVIEW_REQUIRES_EITHER_DRAFT_SAVED_RULE_SET";
    /// A preview cannot edit and copy a rule at the same time.
    pub const PREVIEW_CANNOT_EDIT_COPY_RULE_SAME_TIME: &str =
        "RULE_PREVIEW_CANNOT_EDIT_COPY_RULE_SAME_TIME";
    /// Copy_disables_source requires a copy source rule set.
    pub const COPY_DISABLES_SOURCE_REQUIRES_COPY_SOURCE_RULE_SET: &str =
        "RULE_COPY_DISABLES_SOURCE_REQUIRES_COPY_SOURCE_RULE_SET";
    /// Managed or tracked rules must be copied before editing.
    pub const MANAGED_TRACKED_RULES_INVALID: &str = "RULE_MANAGED_TRACKED_RULES_INVALID";
    /// Rules from this pack cannot be copied.
    pub const RULES_PACK_CANNOT_COPIED: &str = "RULE_RULES_PACK_CANNOT_COPIED";
    /// Copying a tracked rule requires disabling its source.
    pub const COPYING_TRACKED_RULE_REQUIRES_DISABLING_SOURCE: &str =
        "RULE_COPYING_TRACKED_RULE_REQUIRES_DISABLING_SOURCE";
    /// Rule source uses retired fields.
    pub const SOURCE_USES_RETIRED_FIELDS: &str = "RULE_SOURCE_USES_RETIRED_FIELDS";
    /// A title is required.
    pub const TITLE_REQUIRED: &str = "RULE_TITLE_REQUIRED";
    /// Test either a release name or a stored media file, not both.
    pub const TEST_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_NOT_BOTH: &str =
        "RULE_TEST_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_NOT_BOTH";
    /// Title and either a release name or a stored media file are required.
    pub const TITLE_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_REQUIRED: &str =
        "RULE_TITLE_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_REQUIRED";
    /// Release name is too large for a scoring preview.
    pub const RELEASE_NAME_TOO_LARGE_SCORING_PREVIEW: &str =
        "RULE_RELEASE_NAME_TOO_LARGE_SCORING_PREVIEW";
    /// A stored media file is scored from its own size and listing facts; size and listing inputs apply only to a release name.
    pub const STORED_MEDIA_FILE_SCORED_OWN_SIZE_LISTING_FACTS: &str =
        "RULE_STORED_MEDIA_FILE_SCORED_OWN_SIZE_LISTING_FACTS";
    /// Size bytes cannot be negative.
    pub const SIZE_BYTES_CANNOT_NEGATIVE: &str = "RULE_SIZE_BYTES_CANNOT_NEGATIVE";
    /// Published at must be an RFC 2822 or RFC 3339 timestamp, got the selected value.
    pub const PUBLISHED_INVALID: &str = "RULE_PUBLISHED_INVALID";
    /// Votes cannot be negative.
    pub const VOTES_CANNOT_NEGATIVE: &str = "RULE_VOTES_CANNOT_NEGATIVE";
    /// Indexer languages are too large for a scoring preview.
    pub const INDEXER_LANGUAGES_TOO_LARGE_SCORING_PREVIEW: &str =
        "RULE_INDEXER_LANGUAGES_TOO_LARGE_SCORING_PREVIEW";
    /// Bundled rule pack installation conflicted with another change.
    pub const BUNDLED_RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE: &str =
        "RULE_BUNDLED_RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE";
    /// Enabled templates must be unique members of the selected pack.
    pub const ENABLED_TEMPLATES_INVALID: &str = "RULE_ENABLED_TEMPLATES_INVALID";
    /// Rule pack the selected value is already installed.
    pub const PACK_ALREADY_INSTALLED: &str = "RULE_PACK_ALREADY_INSTALLED";
    /// Rule pack installation conflicted with another change.
    pub const PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE: &str =
        "RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE";
    /// Rule pack version or digest did not match the verified catalog release.
    pub const PACK_VERSION_DIGEST_DID_NOT_MATCH_VERIFIED_CATALOG_RELEASE: &str =
        "RULE_PACK_VERSION_DIGEST_DID_NOT_MATCH_VERIFIED_CATALOG_RELEASE";
    /// Tracked rule pack was not found.
    pub const TRACKED_RULE_PACK_NOT_FOUND: &str = "RULE_TRACKED_RULE_PACK_NOT_FOUND";
    /// Duplicate rule pack template ID.
    pub const PACK_TEMPLATE_ID_DUPLICATED: &str = "RULE_PACK_TEMPLATE_ID_DUPLICATED";
    /// Duplicate rule pack priority template ID.
    pub const PACK_PRIORITY_TEMPLATE_ID_DUPLICATED: &str =
        "RULE_PACK_PRIORITY_TEMPLATE_ID_DUPLICATED";
    /// Rule pack settings reference an unknown or removed template.
    pub const PACK_SETTINGS_REFERENCE_UNKNOWN_REMOVED_TEMPLATE: &str =
        "RULE_PACK_SETTINGS_REFERENCE_UNKNOWN_REMOVED_TEMPLATE";
    /// Tracked rule the selected value is missing.
    pub const TRACKED_RULE_REQUIRED: &str = "RULE_TRACKED_RULE_REQUIRED";
    /// Rule set is not tracked by a rule pack.
    pub const SET_NOT_TRACKED_RULE_PACK: &str = "RULE_SET_NOT_TRACKED_RULE_PACK";
    /// Tracked rule pack automatic updates are disabled.
    pub const TRACKED_RULE_PACK_AUTOMATIC_UPDATES_DISABLED: &str =
        "RULE_TRACKED_RULE_PACK_AUTOMATIC_UPDATES_DISABLED";
    /// Rule pack update must be a newer version.
    pub const PACK_UPDATE_INVALID: &str = "RULE_PACK_UPDATE_INVALID";
    /// Automatic rule pack updates require a stable patch on the installed major/minor.
    pub const AUTOMATIC_RULE_PACK_UPDATES_REQUIRE_STABLE_PATCH_INSTALLED_MAJOR_MINOR: &str =
        "RULE_AUTOMATIC_RULE_PACK_UPDATES_REQUIRE_STABLE_PATCH_INSTALLED_MAJOR_MINOR";
    /// The requested value version is not valid semantic version.
    pub const VERSION_INVALID: &str = "RULE_VERSION_INVALID";
    /// Cannot adopt duplicate legacy locale rules for the selected value.
    pub const CANNOT_ADOPT_DUPLICATE_LEGACY_LOCALE_RULES: &str =
        "RULE_CANNOT_ADOPT_DUPLICATE_LEGACY_LOCALE_RULES";
    /// Invalid rule pack facet the selected value.
    pub const PACK_FACET_INVALID: &str = "RULE_PACK_FACET_INVALID";
    /// Tracked rule pack changed; reload and retry.
    pub const TRACKED_RULE_PACK_CHANGED: &str = "RULE_TRACKED_RULE_PACK_CHANGED";
    /// Invalid bundled TRaSH pack.
    pub const BUNDLED_TRASH_PACK_INVALID: &str = "RULE_BUNDLED_TRASH_PACK_INVALID";
    /// Bundled TRaSH pack has an unsupported identity or schema.
    pub const BUNDLED_TRASH_PACK_UNSUPPORTED_IDENTITY_SCHEMA: &str =
        "RULE_BUNDLED_TRASH_PACK_UNSUPPORTED_IDENTITY_SCHEMA";
    /// Bundled TRaSH pack contains no templates.
    pub const BUNDLED_TRASH_PACK_CONTAINS_NO_TEMPLATES: &str =
        "RULE_BUNDLED_TRASH_PACK_CONTAINS_NO_TEMPLATES";
    /// The rule source is invalid.
    pub const SOURCE_INVALID: &str = "RULE_SOURCE_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        PACK_UP_TO_DATE,
        INPUT_RETIRED,
        TARGETS_PERSON,
        VALIDATION_FAILED,
        SET_FIELD_REQUIRED,
        SET_NOT_FOUND,
        TRACKED_COMMUNITY_PACK,
        MANAGED_CONVENIENCE_SETTING,
        MANAGED_LOCALE_FILTER_FORBIDDEN,
        TAG_FILTER_ONLY_APPLIES_MANAGED_RULE_SETS,
        VALIDATION_ERROR,
        ONLY_ONE_ENABLED_RULE_MAY_USE_EXCLUSIVE_GROUP,
        FAILED_BUILD_RULES_ENGINE,
        MEDIA_FILE_NOT_FOUND,
        MEDIA_FILE_DOES_NOT_COVER_EPISODE,
        EPISODE_NOT_FOUND,
        EPISODE_DOES_NOT_BELONG_TITLE,
        EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE,
        EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE,
        TESTING_SAVED_RULE_CANNOT_INCLUDE_DRAFT_EDIT_COPY_CONTEXT,
        PREVIEW_REQUIRES_EITHER_DRAFT_SAVED_RULE_SET,
        PREVIEW_CANNOT_EDIT_COPY_RULE_SAME_TIME,
        COPY_DISABLES_SOURCE_REQUIRES_COPY_SOURCE_RULE_SET,
        MANAGED_TRACKED_RULES_INVALID,
        RULES_PACK_CANNOT_COPIED,
        COPYING_TRACKED_RULE_REQUIRES_DISABLING_SOURCE,
        SOURCE_USES_RETIRED_FIELDS,
        TITLE_REQUIRED,
        TEST_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_NOT_BOTH,
        TITLE_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_REQUIRED,
        RELEASE_NAME_TOO_LARGE_SCORING_PREVIEW,
        STORED_MEDIA_FILE_SCORED_OWN_SIZE_LISTING_FACTS,
        SIZE_BYTES_CANNOT_NEGATIVE,
        PUBLISHED_INVALID,
        VOTES_CANNOT_NEGATIVE,
        INDEXER_LANGUAGES_TOO_LARGE_SCORING_PREVIEW,
        BUNDLED_RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE,
        ENABLED_TEMPLATES_INVALID,
        PACK_ALREADY_INSTALLED,
        PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE,
        PACK_VERSION_DIGEST_DID_NOT_MATCH_VERIFIED_CATALOG_RELEASE,
        TRACKED_RULE_PACK_NOT_FOUND,
        PACK_TEMPLATE_ID_DUPLICATED,
        PACK_PRIORITY_TEMPLATE_ID_DUPLICATED,
        PACK_SETTINGS_REFERENCE_UNKNOWN_REMOVED_TEMPLATE,
        TRACKED_RULE_REQUIRED,
        SET_NOT_TRACKED_RULE_PACK,
        TRACKED_RULE_PACK_AUTOMATIC_UPDATES_DISABLED,
        PACK_UPDATE_INVALID,
        AUTOMATIC_RULE_PACK_UPDATES_REQUIRE_STABLE_PATCH_INSTALLED_MAJOR_MINOR,
        VERSION_INVALID,
        CANNOT_ADOPT_DUPLICATE_LEGACY_LOCALE_RULES,
        PACK_FACET_INVALID,
        TRACKED_RULE_PACK_CHANGED,
        BUNDLED_TRASH_PACK_INVALID,
        BUNDLED_TRASH_PACK_UNSUPPORTED_IDENTITY_SCHEMA,
        BUNDLED_TRASH_PACK_CONTAINS_NO_TEMPLATES,
        SOURCE_INVALID,
    ];
}

pub mod oauth {
    /// The requested authorization scope is not supported.
    pub const INVALID_SCOPE: &str = "OAUTH_INVALID_SCOPE";
    /// Unknown OAuth client.
    pub const CLIENT_UNKNOWN: &str = "OAUTH_CLIENT_UNKNOWN";
    /// Invalid OAuth redirect URI.
    pub const REDIRECT_URI_INVALID: &str = "OAUTH_REDIRECT_URI_INVALID";
    /// Redirect_uri is not allowed for this OAuth client.
    pub const REDIRECT_URI_NOT_ALLOWED_OAUTH_CLIENT: &str =
        "OAUTH_REDIRECT_URI_NOT_ALLOWED_OAUTH_CLIENT";
    /// Oauth client was not found.
    pub const CLIENT_NOT_FOUND: &str = "OAUTH_CLIENT_NOT_FOUND";
    /// OAuth client is disabled or unavailable.
    pub const CLIENT_DISABLED_UNAVAILABLE: &str = "OAUTH_CLIENT_DISABLED_UNAVAILABLE";
    /// OAuth grant is revoked or unavailable.
    pub const GRANT_REVOKED_UNAVAILABLE: &str = "OAUTH_GRANT_REVOKED_UNAVAILABLE";
    /// Code_challenge_method must be S256.
    pub const CODE_CHALLENGE_METHOD_INVALID: &str = "OAUTH_CODE_CHALLENGE_METHOD_INVALID";
    /// Authorization code is invalid.
    pub const AUTHORIZATION_CODE_INVALID: &str = "OAUTH_AUTHORIZATION_CODE_INVALID";
    /// Authorization code is expired or already used.
    pub const AUTHORIZATION_CODE_EXPIRED_ALREADY_USED: &str =
        "OAUTH_AUTHORIZATION_CODE_EXPIRED_ALREADY_USED";
    /// Authorization code binding mismatch.
    pub const AUTHORIZATION_CODE_BINDING_MISMATCH: &str =
        "OAUTH_AUTHORIZATION_CODE_BINDING_MISMATCH";
    /// Authless authorization code is no longer allowed.
    pub const AUTHLESS_AUTHORIZATION_CODE_NO_LONGER_ALLOWED: &str =
        "OAUTH_AUTHLESS_AUTHORIZATION_CODE_NO_LONGER_ALLOWED";
    /// PKCE verification failed.
    pub const PKCE_VERIFICATION_FAILED: &str = "OAUTH_PKCE_VERIFICATION_FAILED";
    /// OAuth user no longer exists.
    pub const USER_NO_LONGER_EXISTS: &str = "OAUTH_USER_NO_LONGER_EXISTS";
    /// Authorization code session is no longer valid.
    pub const AUTHORIZATION_CODE_SESSION_UNAVAILABLE: &str =
        "OAUTH_AUTHORIZATION_CODE_SESSION_UNAVAILABLE";
    /// Authorization code is no longer valid.
    pub const AUTHORIZATION_CODE_UNAVAILABLE: &str = "OAUTH_AUTHORIZATION_CODE_UNAVAILABLE";
    /// Refresh token is invalid.
    pub const REFRESH_TOKEN_INVALID: &str = "OAUTH_REFRESH_TOKEN_INVALID";
    /// Refresh token client mismatch.
    pub const REFRESH_TOKEN_CLIENT_MISMATCH: &str = "OAUTH_REFRESH_TOKEN_CLIENT_MISMATCH";
    /// Refresh token is revoked.
    pub const REFRESH_TOKEN_REVOKED: &str = "OAUTH_REFRESH_TOKEN_REVOKED";
    /// Authless refresh grant is no longer allowed.
    pub const AUTHLESS_REFRESH_GRANT_NO_LONGER_ALLOWED: &str =
        "OAUTH_AUTHLESS_REFRESH_GRANT_NO_LONGER_ALLOWED";
    /// Refresh token was reused.
    pub const REFRESH_TOKEN_REUSED: &str = "OAUTH_REFRESH_TOKEN_REUSED";
    /// Refresh token session is no longer valid.
    pub const REFRESH_TOKEN_SESSION_UNAVAILABLE: &str = "OAUTH_REFRESH_TOKEN_SESSION_UNAVAILABLE";
    /// OAuth client display name is required.
    pub const CLIENT_DISPLAY_NAME_REQUIRED: &str = "OAUTH_CLIENT_DISPLAY_NAME_REQUIRED";
    /// OAuth client display name must not exceed the selected value characters.
    pub const CLIENT_DISPLAY_NAME_TOO_LONG: &str = "OAUTH_CLIENT_DISPLAY_NAME_TOO_LONG";
    /// At least one OAuth redirect URI is required.
    pub const REDIRECT_URI_REQUIRED: &str = "OAUTH_REDIRECT_URI_REQUIRED";
    /// Custom OAuth redirect URIs must be absolute HTTPS URLs without credentials.
    pub const CUSTOM_OAUTH_REDIRECT_URIS_INVALID: &str = "OAUTH_CUSTOM_OAUTH_REDIRECT_URIS_INVALID";
    /// OAuth redirect URIs must be unique.
    pub const REDIRECT_URIS_INVALID: &str = "OAUTH_REDIRECT_URIS_INVALID";
    /// OAuth token is malformed.
    pub const TOKEN_MALFORMED: &str = "OAUTH_TOKEN_MALFORMED";
    /// Code_challenge must be a valid S256 base64url value.
    pub const CODE_CHALLENGE_INVALID: &str = "OAUTH_CODE_CHALLENGE_INVALID";
    /// Code_verifier must be 43-128 RFC7636 unreserved characters.
    pub const CODE_VERIFIER_INVALID: &str = "OAUTH_CODE_VERIFIER_INVALID";
    /// Redirect_uri must not contain a fragment.
    pub const REDIRECT_URI_FRAGMENT_FORBIDDEN: &str = "OAUTH_REDIRECT_URI_FRAGMENT_FORBIDDEN";
    /// More than one Jellyfin connection matches. Choose a connection.
    pub const JELLYFIN_CONNECTION_AMBIGUOUS: &str = "OAUTH_JELLYFIN_CONNECTION_AMBIGUOUS";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        INVALID_SCOPE,
        CLIENT_UNKNOWN,
        REDIRECT_URI_INVALID,
        REDIRECT_URI_NOT_ALLOWED_OAUTH_CLIENT,
        CLIENT_NOT_FOUND,
        CLIENT_DISABLED_UNAVAILABLE,
        GRANT_REVOKED_UNAVAILABLE,
        CODE_CHALLENGE_METHOD_INVALID,
        AUTHORIZATION_CODE_INVALID,
        AUTHORIZATION_CODE_EXPIRED_ALREADY_USED,
        AUTHORIZATION_CODE_BINDING_MISMATCH,
        AUTHLESS_AUTHORIZATION_CODE_NO_LONGER_ALLOWED,
        PKCE_VERIFICATION_FAILED,
        USER_NO_LONGER_EXISTS,
        AUTHORIZATION_CODE_SESSION_UNAVAILABLE,
        AUTHORIZATION_CODE_UNAVAILABLE,
        REFRESH_TOKEN_INVALID,
        REFRESH_TOKEN_CLIENT_MISMATCH,
        REFRESH_TOKEN_REVOKED,
        AUTHLESS_REFRESH_GRANT_NO_LONGER_ALLOWED,
        REFRESH_TOKEN_REUSED,
        REFRESH_TOKEN_SESSION_UNAVAILABLE,
        CLIENT_DISPLAY_NAME_REQUIRED,
        CLIENT_DISPLAY_NAME_TOO_LONG,
        REDIRECT_URI_REQUIRED,
        CUSTOM_OAUTH_REDIRECT_URIS_INVALID,
        REDIRECT_URIS_INVALID,
        TOKEN_MALFORMED,
        CODE_CHALLENGE_INVALID,
        CODE_VERIFIER_INVALID,
        REDIRECT_URI_FRAGMENT_FORBIDDEN,
        JELLYFIN_CONNECTION_AMBIGUOUS,
    ];
}

pub mod maintenance {
    /// The candidate count changed. Review the current count before arming.
    pub const ARM_COUNT_MISMATCH: &str = "MAINTENANCE_ARM_COUNT_MISMATCH";
    /// Maintenance exclusion was not found.
    pub const EXCLUSION_NOT_FOUND: &str = "MAINTENANCE_EXCLUSION_NOT_FOUND";
    /// Maintenance rule the selected value failed to compile.
    pub const RULE_FAILED_COMPILE: &str = "MAINTENANCE_RULE_FAILED_COMPILE";
    /// Ambiguous maintenance file coverage.
    pub const AMBIGUOUS_MAINTENANCE_FILE_COVERAGE: &str =
        "MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE";
    /// Child deletion requires an episode or season.
    pub const CHILD_DELETION_REQUIRES_EPISODE_SEASON: &str =
        "MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON";
    /// Maintenance unmonitoring candidate identity does not match its title.
    pub const UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE: &str =
        "MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE";
    /// Maintenance file targets changed; another evaluation is required.
    pub const FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED: &str =
        "MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED";
    /// Maintenance file ownership changed.
    pub const FILE_OWNERSHIP_CHANGED: &str = "MAINTENANCE_FILE_OWNERSHIP_CHANGED";
    /// This rule ships with Scryer: its matcher and actions cannot be changed. Its grace period, libraries and state can.
    pub const RULE_SHIPS_SCRYER: &str = "MAINTENANCE_RULE_SHIPS_SCRYER";
    /// This rule ships with Scryer and cannot be renamed.
    pub const RULE_SHIPS_SCRYER_CANNOT_RENAMED: &str =
        "MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED";
    /// This rule ships with Scryer and cannot be deleted. Set it to Disabled instead.
    pub const RULE_SHIPS_SCRYER_CANNOT_DELETED: &str =
        "MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED";
    /// The selected item has already run actions on your library, and deleting it would erase the record of what it did. Set it to Disabled instead: the rule stops immediately and its history is kept.
    pub const ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID: &str =
        "MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID";
    /// The selected item is currently tracking the selected value title(s), and deleting it would drop them without a trace. Set it to Disabled instead, or clear the candidates first.
    pub const CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE: &str =
        "MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE";
    /// Rule validation failed.
    pub const RULE_VALIDATION_FAILED: &str = "MAINTENANCE_RULE_VALIDATION_FAILED";
    /// Preview requires between one and fifty subjects.
    pub const PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS: &str =
        "MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS";
    /// Rule evaluation failed.
    pub const RULE_EVALUATION_FAILED: &str = "MAINTENANCE_RULE_EVALUATION_FAILED";
    /// Preview subject is missing, outside the rule libraries, or does not belong to the selected titles and scope.
    pub const PREVIEW_SUBJECT_OUTSIDE_SCOPE: &str = "MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE";
    /// This rule reads facts about specific people, so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.
    pub const PERSON_TARGETING_PERMISSION_REQUIRED: &str =
        "MAINTENANCE_PERSON_TARGETING_PERMISSION_REQUIRED";
    /// A matcher that reads storage facts must select a configured library root.
    pub const MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT: &str =
        "MAINTENANCE_MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT";
    /// Storage root the selected item is not configured.
    pub const STORAGE_ROOT_NOT_CONFIGURED: &str = "MAINTENANCE_STORAGE_ROOT_NOT_CONFIGURED";
    /// Maintenance rule set was not found.
    pub const RULE_SET_NOT_FOUND: &str = "MAINTENANCE_RULE_SET_NOT_FOUND";
    /// Preview accepts at most the selected value titles, the selected value were requested.
    pub const PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED: &str =
        "MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED";
    /// Grace period must be zero or more days.
    pub const GRACE_PERIOD_INVALID: &str = "MAINTENANCE_GRACE_PERIOD_INVALID";
    /// Grace period must be at most the selected value days (ten years).
    pub const GRACE_PERIOD_OUT_OF_RANGE: &str = "MAINTENANCE_GRACE_PERIOD_OUT_OF_RANGE";
    /// Maintenance action is not storable.
    pub const ACTION_NOT_STORABLE: &str = "MAINTENANCE_ACTION_NOT_STORABLE";
    /// This maintenance action is not supported for titles.
    pub const ACTION_UNSUPPORTED: &str = "MAINTENANCE_ACTION_UNSUPPORTED";
    /// Maintenance action is not valid for a title-scoped rule.
    pub const ACTION_INVALID: &str = "MAINTENANCE_ACTION_INVALID";
    /// The requested value must not be empty.
    pub const FIELD_REQUIRED: &str = "MAINTENANCE_FIELD_REQUIRED";
    /// Movie titles do not have an episode inventory.
    pub const MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY: &str =
        "MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY";
    /// Invalid maintenance subject kind.
    pub const SUBJECT_KIND_INVALID: &str = "MAINTENANCE_SUBJECT_KIND_INVALID";
    /// Maintenance subject ID is required.
    pub const SUBJECT_ID_REQUIRED: &str = "MAINTENANCE_SUBJECT_ID_REQUIRED";
    /// Maintenance subject no longer belongs to this title was not found.
    pub const SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND: &str =
        "MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND";
    /// This rule's action never changes anything, so it can only run as a dry run.
    pub const RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN: &str =
        "MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN";
    /// The rule configuration must be reviewed again before arming deletion; reopen the confirmation.
    pub const RULE_CONFIGURATION_INVALID: &str = "MAINTENANCE_RULE_CONFIGURATION_INVALID";
    /// The rule changed while confirming; review it again before arming deletion.
    pub const RULE_CHANGED_WHILE_CONFIRMING: &str = "MAINTENANCE_RULE_CHANGED_WHILE_CONFIRMING";
    /// Sequence actions must use the sequence executor.
    pub const SEQUENCE_ACTIONS_INVALID: &str = "MAINTENANCE_SEQUENCE_ACTIONS_INVALID";
    /// Tag action has no tags configured.
    pub const TAG_ACTION_NO_TAGS_CONFIGURED: &str = "MAINTENANCE_TAG_ACTION_NO_TAGS_CONFIGURED";
    /// Quality-profile action has no target profile.
    pub const QUALITY_PROFILE_ACTION_NO_TARGET_PROFILE: &str =
        "MAINTENANCE_QUALITY_PROFILE_ACTION_NO_TARGET_PROFILE";
    /// Target quality profile the selected item does not exist.
    pub const TARGET_QUALITY_PROFILE_DOES_NOT_EXIST: &str =
        "MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST";
    /// Maintenance sequence step parameters are invalid.
    pub const SEQUENCE_STEP_PARAMETERS_INVALID: &str =
        "MAINTENANCE_SEQUENCE_STEP_PARAMETERS_INVALID";
    /// DeleteFiles requires an earlier Unmonitor step.
    pub const DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP: &str =
        "MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP";
    /// DeleteFiles has an incompatible Unmonitor prerequisite.
    pub const DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE: &str =
        "MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE";
    /// The maintenance rule source is invalid.
    pub const RULE_SOURCE_INVALID: &str = "MAINTENANCE_RULE_SOURCE_INVALID";
    /// The maintenance action configuration is invalid.
    pub const ACTION_CONFIGURATION_INVALID: &str = "MAINTENANCE_ACTION_CONFIGURATION_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        ARM_COUNT_MISMATCH,
        EXCLUSION_NOT_FOUND,
        RULE_FAILED_COMPILE,
        AMBIGUOUS_MAINTENANCE_FILE_COVERAGE,
        CHILD_DELETION_REQUIRES_EPISODE_SEASON,
        UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE,
        FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED,
        FILE_OWNERSHIP_CHANGED,
        RULE_SHIPS_SCRYER,
        RULE_SHIPS_SCRYER_CANNOT_RENAMED,
        RULE_SHIPS_SCRYER_CANNOT_DELETED,
        ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID,
        CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE,
        RULE_VALIDATION_FAILED,
        PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS,
        RULE_EVALUATION_FAILED,
        PREVIEW_SUBJECT_OUTSIDE_SCOPE,
        PERSON_TARGETING_PERMISSION_REQUIRED,
        MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT,
        STORAGE_ROOT_NOT_CONFIGURED,
        RULE_SET_NOT_FOUND,
        PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED,
        GRACE_PERIOD_INVALID,
        GRACE_PERIOD_OUT_OF_RANGE,
        ACTION_NOT_STORABLE,
        ACTION_UNSUPPORTED,
        ACTION_INVALID,
        FIELD_REQUIRED,
        MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY,
        SUBJECT_KIND_INVALID,
        SUBJECT_ID_REQUIRED,
        SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND,
        RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN,
        RULE_CONFIGURATION_INVALID,
        RULE_CHANGED_WHILE_CONFIRMING,
        SEQUENCE_ACTIONS_INVALID,
        TAG_ACTION_NO_TAGS_CONFIGURED,
        QUALITY_PROFILE_ACTION_NO_TARGET_PROFILE,
        TARGET_QUALITY_PROFILE_DOES_NOT_EXIST,
        SEQUENCE_STEP_PARAMETERS_INVALID,
        DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP,
        DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE,
        RULE_SOURCE_INVALID,
        ACTION_CONFIGURATION_INVALID,
    ];
}

pub mod acquisition {
    /// An acquisition search is already running.
    pub const SEARCH_RUNNING: &str = "ACQUISITION_SEARCH_RUNNING";
    /// The indexer resolved a different item; refusing to submit.
    pub const INDEXER_RESOLVED_DIFFERENT_ITEM: &str = "ACQUISITION_INDEXER_RESOLVED_DIFFERENT_ITEM";
    /// The indexer resolved a different page; refusing to submit.
    pub const INDEXER_RESOLVED_DIFFERENT_PAGE: &str = "ACQUISITION_INDEXER_RESOLVED_DIFFERENT_PAGE";
    /// Canonical download submission requires a download id.
    pub const CANONICAL_DOWNLOAD_SUBMISSION_REQUIRES_DOWNLOAD_ID: &str =
        "ACQUISITION_CANONICAL_DOWNLOAD_SUBMISSION_REQUIRES_DOWNLOAD_ID";
    /// The selected library is unavailable or incompatible with this endpoint.
    pub const SELECTED_LIBRARY_UNAVAILABLE_INCOMPATIBLE_ENDPOINT: &str =
        "ACQUISITION_SELECTED_LIBRARY_UNAVAILABLE_INCOMPATIBLE_ENDPOINT";
    /// Matched title was not found.
    pub const MATCHED_TITLE_NO_LONGER_EXISTS_NOT_FOUND: &str =
        "ACQUISITION_MATCHED_TITLE_NO_LONGER_EXISTS_NOT_FOUND";
    /// Release URL or flag list exceeds the input limit.
    pub const RELEASE_URL_FLAG_LIST_EXCEEDS_INPUT_LIMIT: &str =
        "ACQUISITION_RELEASE_URL_FLAG_LIST_EXCEEDS_INPUT_LIMIT";
    /// Release name and source are required and must be bounded.
    pub const RELEASE_NAME_SOURCE_REQUIRED_INVALID: &str =
        "ACQUISITION_RELEASE_NAME_SOURCE_REQUIRED_INVALID";
    /// Release size must not be negative.
    pub const RELEASE_SIZE_OUT_OF_RANGE: &str = "ACQUISITION_RELEASE_SIZE_OUT_OF_RANGE";
    /// A magnet requires the torrent protocol.
    pub const MAGNET_REQUIRES_TORRENT_PROTOCOL: &str =
        "ACQUISITION_MAGNET_REQUIRES_TORRENT_PROTOCOL";
    /// Release download URL is invalid.
    pub const RELEASE_DOWNLOAD_URL_INVALID: &str = "ACQUISITION_RELEASE_DOWNLOAD_URL_INVALID";
    /// Release URL must use HTTP, HTTPS, or a valid magnet.
    pub const RELEASE_URL_INVALID: &str = "ACQUISITION_RELEASE_URL_INVALID";
    /// Unsupported release flag.
    pub const RELEASE_FLAG_UNSUPPORTED: &str = "ACQUISITION_RELEASE_FLAG_UNSUPPORTED";
    /// Download submission was not found.
    pub const DOWNLOAD_SUBMISSION_NOT_FOUND: &str = "ACQUISITION_DOWNLOAD_SUBMISSION_NOT_FOUND";
    /// Wanted item was not found.
    pub const WANTED_ITEM_NOT_FOUND: &str = "ACQUISITION_WANTED_ITEM_NOT_FOUND";
    /// Title was not found.
    pub const TITLE_NOT_FOUND: &str = "ACQUISITION_TITLE_NOT_FOUND";
    /// Acquisition is paused for content covered by this release. Resume it before searching.
    pub const PAUSED_CONTENT_COVERED_RELEASE: &str = "ACQUISITION_PAUSED_CONTENT_COVERED_RELEASE";
    /// Title has no name or external IDs.
    pub const TITLE_NO_NAME_EXTERNAL_IDS: &str = "ACQUISITION_TITLE_NO_NAME_EXTERNAL_IDS";
    /// Season and episode are required.
    pub const SEASON_EPISODE_REQUIRED: &str = "ACQUISITION_SEASON_EPISODE_REQUIRED";
    /// Season and episode must include numeric values.
    pub const SEASON_EPISODE_INVALID: &str = "ACQUISITION_SEASON_EPISODE_INVALID";
    /// Invalid season value.
    pub const SEASON_VALUE_INVALID: &str = "ACQUISITION_SEASON_VALUE_INVALID";
    /// Invalid episode value.
    pub const EPISODE_VALUE_INVALID: &str = "ACQUISITION_EPISODE_VALUE_INVALID";
    /// Season pack search subject has no searchable title or external IDs.
    pub const SEASON_PACK_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE_EXTERNAL_IDS: &str =
        "ACQUISITION_SEASON_PACK_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE_EXTERNAL_IDS";
    /// Series movie search subject has no searchable title.
    pub const SERIES_MOVIE_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE: &str =
        "ACQUISITION_SERIES_MOVIE_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE";
    /// Season must be a whole number.
    pub const SEASON_INVALID: &str = "ACQUISITION_SEASON_INVALID";
    /// Automatic search requires a title and cannot select a Wanted item.
    pub const AUTOMATIC_SEARCH_REQUIRES_TITLE_CANNOT_SELECT_WANTED_ITEM: &str =
        "ACQUISITION_AUTOMATIC_SEARCH_REQUIRES_TITLE_CANNOT_SELECT_WANTED_ITEM";
    /// Another search is running. Wait for it to finish or cancel it before starting this search.
    pub const ANOTHER_SEARCH_RUNNING: &str = "ACQUISITION_ANOTHER_SEARCH_RUNNING";
    /// Maintenance Search dispatch requires a preallocated job run id.
    pub const MAINTENANCE_SEARCH_DISPATCH_REQUIRES_PREALLOCATED_JOB_RUN_ID: &str =
        "ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_PREALLOCATED_JOB_RUN_ID";
    /// Maintenance Search dispatch requires an accepted receipt intent.
    pub const MAINTENANCE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT: &str =
        "ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT";
    /// Recovery search job was not found.
    pub const RECOVERY_SEARCH_JOB_NOT_FOUND: &str = "ACQUISITION_RECOVERY_SEARCH_JOB_NOT_FOUND";
    /// Unsupported saved search version.
    pub const SEARCH_VERSION_UNSUPPORTED: &str = "ACQUISITION_SEARCH_VERSION_UNSUPPORTED";
    /// Invalid saved wanted kind.
    pub const WANTED_KIND_INVALID: &str = "ACQUISITION_WANTED_KIND_INVALID";
    /// Invalid saved search facet.
    pub const SEARCH_FACET_INVALID: &str = "ACQUISITION_SEARCH_FACET_INVALID";
    /// Search actor is unavailable.
    pub const SEARCH_ACTOR_UNAVAILABLE: &str = "ACQUISITION_SEARCH_ACTOR_UNAVAILABLE";
    /// Search actor no longer exists.
    pub const SEARCH_ACTOR_NO_LONGER_EXISTS: &str = "ACQUISITION_SEARCH_ACTOR_NO_LONGER_EXISTS";
    /// Search actor is disabled.
    pub const SEARCH_ACTOR_DISABLED: &str = "ACQUISITION_SEARCH_ACTOR_DISABLED";
    /// Automatic search requires a title.
    pub const AUTOMATIC_SEARCH_REQUIRES_TITLE: &str = "ACQUISITION_AUTOMATIC_SEARCH_REQUIRES_TITLE";
    /// The selected title was not found.
    pub const TITLE_NO_LONGER_EXISTS_NOT_FOUND: &str =
        "ACQUISITION_TITLE_NO_LONGER_EXISTS_NOT_FOUND";
    /// This title is being moved. Search is available when the move finishes.
    pub const TITLE_BEING_MOVED_SEARCH_UNAVAILABLE: &str =
        "ACQUISITION_TITLE_BEING_MOVED_SEARCH_UNAVAILABLE";
    /// The selected season no longer matches this title. Refresh the title and try again.
    pub const SELECTED_SEASON_NO_LONGER_MATCHES_TITLE: &str =
        "ACQUISITION_SELECTED_SEASON_NO_LONGER_MATCHES_TITLE";
    /// The matching episodes are paused or already downloading. Resume paused acquisition or wait for the download to finish.
    pub const MATCHING_EPISODES_PAUSED_ALREADY_DOWNLOADING: &str =
        "ACQUISITION_MATCHING_EPISODES_PAUSED_ALREADY_DOWNLOADING";
    /// No acquisition scope for was not found.
    pub const SCOPE_NOT_FOUND: &str = "ACQUISITION_SCOPE_NOT_FOUND";
    /// Acquisition was paused while this search was waiting. Resume it before searching.
    pub const PAUSED_WHILE_SEARCH_WAITING: &str = "ACQUISITION_PAUSED_WHILE_SEARCH_WAITING";
    /// The download client is still finishing an earlier operation that holds this entry, such as its own delete.
    pub const DOWNLOAD_CLIENT_DELETE_IN_PROGRESS: &str =
        "ACQUISITION_DOWNLOAD_CLIENT_DELETE_IN_PROGRESS";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SEARCH_RUNNING,
        INDEXER_RESOLVED_DIFFERENT_ITEM,
        INDEXER_RESOLVED_DIFFERENT_PAGE,
        CANONICAL_DOWNLOAD_SUBMISSION_REQUIRES_DOWNLOAD_ID,
        SELECTED_LIBRARY_UNAVAILABLE_INCOMPATIBLE_ENDPOINT,
        MATCHED_TITLE_NO_LONGER_EXISTS_NOT_FOUND,
        RELEASE_URL_FLAG_LIST_EXCEEDS_INPUT_LIMIT,
        RELEASE_NAME_SOURCE_REQUIRED_INVALID,
        RELEASE_SIZE_OUT_OF_RANGE,
        MAGNET_REQUIRES_TORRENT_PROTOCOL,
        RELEASE_DOWNLOAD_URL_INVALID,
        RELEASE_URL_INVALID,
        RELEASE_FLAG_UNSUPPORTED,
        DOWNLOAD_SUBMISSION_NOT_FOUND,
        WANTED_ITEM_NOT_FOUND,
        TITLE_NOT_FOUND,
        PAUSED_CONTENT_COVERED_RELEASE,
        TITLE_NO_NAME_EXTERNAL_IDS,
        SEASON_EPISODE_REQUIRED,
        SEASON_EPISODE_INVALID,
        SEASON_VALUE_INVALID,
        EPISODE_VALUE_INVALID,
        SEASON_PACK_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE_EXTERNAL_IDS,
        SERIES_MOVIE_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE,
        SEASON_INVALID,
        AUTOMATIC_SEARCH_REQUIRES_TITLE_CANNOT_SELECT_WANTED_ITEM,
        ANOTHER_SEARCH_RUNNING,
        MAINTENANCE_SEARCH_DISPATCH_REQUIRES_PREALLOCATED_JOB_RUN_ID,
        MAINTENANCE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT,
        RECOVERY_SEARCH_JOB_NOT_FOUND,
        SEARCH_VERSION_UNSUPPORTED,
        WANTED_KIND_INVALID,
        SEARCH_FACET_INVALID,
        SEARCH_ACTOR_UNAVAILABLE,
        SEARCH_ACTOR_NO_LONGER_EXISTS,
        SEARCH_ACTOR_DISABLED,
        AUTOMATIC_SEARCH_REQUIRES_TITLE,
        TITLE_NO_LONGER_EXISTS_NOT_FOUND,
        TITLE_BEING_MOVED_SEARCH_UNAVAILABLE,
        SELECTED_SEASON_NO_LONGER_MATCHES_TITLE,
        MATCHING_EPISODES_PAUSED_ALREADY_DOWNLOADING,
        SCOPE_NOT_FOUND,
        PAUSED_WHILE_SEARCH_WAITING,
        DOWNLOAD_CLIENT_DELETE_IN_PROGRESS,
    ];
}

pub mod location {
    /// The plan changed. Preview it again.
    pub const STALE_PLAN: &str = "LOCATION_STALE_PLAN";
    /// The merge has no valid identity mapping.
    pub const MERGE_UNAVAILABLE: &str = "LOCATION_MERGE_UNAVAILABLE";
    /// The destination did not pass verification.
    pub const VERIFICATION_MISMATCH: &str = "LOCATION_VERIFICATION_MISMATCH";
    /// Library was not found.
    pub const LIBRARY_NOT_FOUND: &str = "LOCATION_LIBRARY_NOT_FOUND";
    /// Root  in library was not found.
    pub const ROOT_NOT_FOUND: &str = "LOCATION_ROOT_NOT_FOUND";
    /// Choose a new path for this root.
    pub const ROOT_NEW_PATH_REQUIRED: &str = "LOCATION_ROOT_NEW_PATH_REQUIRED";
    /// Library root was not found.
    pub const LIBRARY_ROOT_NOT_FOUND: &str = "LOCATION_LIBRARY_ROOT_NOT_FOUND";
    /// Location operation was not found.
    pub const OPERATION_NOT_FOUND: &str = "LOCATION_OPERATION_NOT_FOUND";
    /// Verification of the selected value failedthe selected value.
    pub const VERIFICATION_FAILED: &str = "LOCATION_VERIFICATION_FAILED";
    /// The transferred file the selected value needs to be verified again before its catalog entry can be updated.
    pub const TRANSFERRED_FILE_NEEDS_VERIFIED_AGAIN_BEFORE_CATALOG_ENTRY_UPDATED: &str =
        "LOCATION_TRANSFERRED_FILE_NEEDS_VERIFIED_AGAIN_BEFORE_CATALOG_ENTRY_UPDATED";
    /// The transferred file the selected value has not finished verification. Its catalog entry was left unchanged.
    pub const TRANSFERRED_FILE_NOT_FINISHED_VERIFICATION: &str =
        "LOCATION_TRANSFERRED_FILE_NOT_FINISHED_VERIFICATION";
    /// The destination staging path the selected value is occupied by a file this move did not write. Both files were preserved; move or remove that file and try again.
    pub const DESTINATION_OCCUPIED: &str = "LOCATION_DESTINATION_OCCUPIED";
    /// Path was not found.
    pub const PATH_NOT_FOUND: &str = "LOCATION_PATH_NOT_FOUND";
    /// This selection has nothing to move.
    pub const SELECTION_NOTHING_MOVE: &str = "LOCATION_SELECTION_NOTHING_MOVE";
    /// Location operation the selected value is still running; cancel it instead of abandoning it.
    pub const OPERATION_STILL_RUNNING: &str = "LOCATION_OPERATION_STILL_RUNNING";
    /// Location operation source libraries are unavailable.
    pub const OPERATION_SOURCE_LIBRARIES_UNAVAILABLE: &str =
        "LOCATION_OPERATION_SOURCE_LIBRARIES_UNAVAILABLE";
    /// Select at least one title to move.
    pub const TITLE_SELECTION_REQUIRED: &str = "LOCATION_TITLE_SELECTION_REQUIRED";
    /// Choose a destination library or root.
    pub const DESTINATION_REQUIRED: &str = "LOCATION_DESTINATION_REQUIRED";
    /// A root move spans one library; choose a destination library for a selection that spans several.
    pub const ROOT_MOVE_SPANS_ONE_LIBRARY: &str = "LOCATION_ROOT_MOVE_SPANS_ONE_LIBRARY";
    /// The catalog cannot map a file outside the recorded folder for the selected value. Correct its folder mapping first.
    pub const CATALOG_CANNOT_MAP_FILE_OUTSIDE_RECORDED_FOLDER: &str =
        "LOCATION_CATALOG_CANNOT_MAP_FILE_OUTSIDE_RECORDED_FOLDER";
    /// The catalog path for the selected value is ambiguous. Correct its folder mapping before moving it.
    pub const CATALOG_PATH_AMBIGUOUS: &str = "LOCATION_CATALOG_PATH_AMBIGUOUS";
    /// More than one catalog record would point at the selected value. Resolve that catalog conflict before confirming the manual move.
    pub const MORE_ONE_CATALOG_RECORD_WOULD_POINT: &str =
        "LOCATION_MORE_ONE_CATALOG_RECORD_WOULD_POINT";
    /// Destination title was not found.
    pub const DESTINATION_TITLE_NOT_FOUND: &str = "LOCATION_DESTINATION_TITLE_NOT_FOUND";
    /// Folder path is required.
    pub const FOLDER_PATH_REQUIRED: &str = "LOCATION_FOLDER_PATH_REQUIRED";
    /// Folder the selected value must not contain . or .. components.
    pub const PATH_COMPONENT_INVALID: &str = "LOCATION_PATH_COMPONENT_INVALID";
    /// Folder the selected value is not inside a root of library the selected value.
    pub const FOLDER_NOT_INSIDE_ROOT_LIBRARY: &str = "LOCATION_FOLDER_NOT_INSIDE_ROOT_LIBRARY";
    /// Folder the selected value is a library root, not a title folder.
    pub const FOLDER_NOT_TITLE_BOUNDARY: &str = "LOCATION_FOLDER_NOT_TITLE_BOUNDARY";
    /// Selected folder resolves outside its library root.
    pub const PATH_OUTSIDE_LIBRARY_ROOT: &str = "LOCATION_PATH_OUTSIDE_LIBRARY_ROOT";
    /// Selected folder overlaps another title's folder; no ownership was changed.
    pub const SELECTED_FOLDER_OVERLAPS_ANOTHER_TITLE_S_FOLDER: &str =
        "LOCATION_SELECTED_FOLDER_OVERLAPS_ANOTHER_TITLE_S_FOLDER";
    /// Folder the selected value is not owned by another title; resolution the selected value does not apply.
    pub const FOLDER_NOT_OWNED_ANOTHER_TITLE: &str = "LOCATION_FOLDER_NOT_OWNED_ANOTHER_TITLE";
    /// Folder the selected value is already owned by title the selected value; choose swap or take over.
    pub const FOLDER_ALREADY_OWNED_TITLE: &str = "LOCATION_FOLDER_ALREADY_OWNED_TITLE";
    /// Title the selected value owns no folder to swap; take over the folder instead.
    pub const TITLE_OWNS_NO_FOLDER_SWAP: &str = "LOCATION_TITLE_OWNS_NO_FOLDER_SWAP";
    /// The recorded folder for the selected value is a library root, not a folder to swap; take over the folder instead.
    pub const RECORDED_FOLDER_LIBRARY_ROOT_NOT_FOLDER_SWAP: &str =
        "LOCATION_RECORDED_FOLDER_LIBRARY_ROOT_NOT_FOLDER_SWAP";
    /// Merge of title the selected value into the selected value is blocked.
    pub const MERGE_TITLE_INTO_BLOCKED: &str = "LOCATION_MERGE_TITLE_INTO_BLOCKED";
    /// Source and destination are the same path.
    pub const SOURCE_DESTINATION_SAME_PATH: &str = "LOCATION_SOURCE_DESTINATION_SAME_PATH";
    /// Source changed during file placement.
    pub const SOURCE_CHANGED_DURING_FILE_PLACEMENT: &str =
        "LOCATION_SOURCE_CHANGED_DURING_FILE_PLACEMENT";
    /// The confirmed plan has no instructions for title the selected value.
    pub const CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE: &str =
        "LOCATION_CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE";
    /// A file changed before source cleanup. The source was preserved; retry to compare it again.
    pub const FILE_CHANGED_BEFORE_SOURCE_CLEANUP: &str =
        "LOCATION_FILE_CHANGED_BEFORE_SOURCE_CLEANUP";
    /// The destination copy no longer holds the content this move proved identical to the source. The source was preserved; retry to compare the files again.
    pub const DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE: &str =
        "LOCATION_DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE";
    /// Source or destination changed before cleanup.
    pub const SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP: &str =
        "LOCATION_SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP";
    /// Source changed before cleanup; preserved it.
    pub const SOURCE_CHANGED_BEFORE_CLEANUP: &str = "LOCATION_SOURCE_CHANGED_BEFORE_CLEANUP";
    /// The related media file could not be transferred. Its companion files were preserved.
    pub const RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED: &str =
        "LOCATION_RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED";
    /// The related media transfer was interrupted. The source was preserved.
    pub const RELATED_MEDIA_TRANSFER_INTERRUPTED: &str =
        "LOCATION_RELATED_MEDIA_TRANSFER_INTERRUPTED";
    /// The destination changed during comparison. The source was preserved; try again when the files are no longer changing.
    pub const DESTINATION_CHANGED_DURING_COMPARISON: &str =
        "LOCATION_DESTINATION_CHANGED_DURING_COMPARISON";
    /// The source changed during transfer. It was preserved; try again when the file is no longer changing.
    pub const SOURCE_CHANGED_DURING_TRANSFER: &str = "LOCATION_SOURCE_CHANGED_DURING_TRANSFER";
    /// Could not compare the selected value. Check that it is readable and try again; the source was preserved.
    pub const COULD_NOT_COMPARE: &str = "LOCATION_COULD_NOT_COMPARE";
    /// An existing path is not a regular file. Resolve the conflicting path and try again; the source was preserved.
    pub const EXISTING_PATH_NOT_REGULAR_FILE: &str = "LOCATION_EXISTING_PATH_NOT_REGULAR_FILE";
    /// A file changed while it was being compared. The source was preserved; try again when the files are no longer changing.
    pub const FILE_CHANGED_WHILE_BEING_COMPARED: &str =
        "LOCATION_FILE_CHANGED_WHILE_BEING_COMPARED";
    /// Full-hash backfill is already running.
    pub const FULL_HASH_BACKFILL_ALREADY_RUNNING: &str =
        "LOCATION_FULL_HASH_BACKFILL_ALREADY_RUNNING";
    /// Full-hash backfill cancelled.
    pub const FULL_HASH_BACKFILL_CANCELLED: &str = "LOCATION_FULL_HASH_BACKFILL_CANCELLED";
    /// File changed while hashing.
    pub const FILE_CHANGED_WHILE_HASHING: &str = "LOCATION_FILE_CHANGED_WHILE_HASHING";
    /// A location operation currently owns these items.
    pub const OWNERSHIP_BUSY: &str = "LOCATION_OWNERSHIP_BUSY";
    /// Library roots changed; recovery needs a new plan.
    pub const RECOVERY_ROOTS_CHANGED: &str = "LOCATION_RECOVERY_ROOTS_CHANGED";
    /// Title was not found.
    pub const RECOVERY_TITLE_NOT_FOUND: &str = "LOCATION_RECOVERY_TITLE_NOT_FOUND";
    /// Title ownership changed; originals are preserved.
    pub const RECOVERY_OWNERSHIP_CHANGED: &str = "LOCATION_RECOVERY_OWNERSHIP_CHANGED";
    /// Tracked media changed; originals are preserved.
    pub const RECOVERY_TRACKED_MEDIA_CHANGED: &str = "LOCATION_RECOVERY_TRACKED_MEDIA_CHANGED";
    /// No proven destination.
    pub const RECOVERY_DESTINATION_UNPROVEN: &str = "LOCATION_RECOVERY_DESTINATION_UNPROVEN";
    /// No proven destination root.
    pub const RECOVERY_DESTINATION_ROOT_UNPROVEN: &str =
        "LOCATION_RECOVERY_DESTINATION_ROOT_UNPROVEN";
    /// Recovery destination is not a valid title folder.
    pub const RECOVERY_DESTINATION_BOUNDARY_INVALID: &str =
        "LOCATION_RECOVERY_DESTINATION_BOUNDARY_INVALID";
    /// Another title owns an overlapping destination folder.
    pub const RECOVERY_DESTINATION_OWNERSHIP_CONFLICT: &str =
        "LOCATION_RECOVERY_DESTINATION_OWNERSHIP_CONFLICT";
    /// Another title tracks a source or destination file.
    pub const RECOVERY_FILE_OWNERSHIP_CONFLICT: &str = "LOCATION_RECOVERY_FILE_OWNERSHIP_CONFLICT";
    /// A destination is already associated with another media record.
    pub const RECOVERY_DESTINATION_RECORD_CONFLICT: &str =
        "LOCATION_RECOVERY_DESTINATION_RECORD_CONFLICT";
    /// Recovery file destinations collide or escape the title folder.
    pub const RECOVERY_DESTINATION_PATH_INVALID: &str =
        "LOCATION_RECOVERY_DESTINATION_PATH_INVALID";
    /// Recovery requires an absolute native filesystem path.
    pub const RECOVERY_NATIVE_ABSOLUTE_PATH_REQUIRED: &str =
        "LOCATION_RECOVERY_NATIVE_ABSOLUTE_PATH_REQUIRED";
    /// Recovery does not follow dot-segment paths.
    pub const RECOVERY_DOT_SEGMENT_PATH_FORBIDDEN: &str =
        "LOCATION_RECOVERY_DOT_SEGMENT_PATH_FORBIDDEN";
    /// Recovery does not follow reparse points.
    pub const RECOVERY_REPARSE_POINT_FORBIDDEN: &str = "LOCATION_RECOVERY_REPARSE_POINT_FORBIDDEN";
    /// Recovery does not follow symbolic links.
    pub const RECOVERY_SYMLINK_FORBIDDEN: &str = "LOCATION_RECOVERY_SYMLINK_FORBIDDEN";
    /// Recovery path is unavailable.
    pub const RECOVERY_PATH_UNAVAILABLE: &str = "LOCATION_RECOVERY_PATH_UNAVAILABLE";
    /// Recovery requires a regular file.
    pub const RECOVERY_REGULAR_FILE_REQUIRED: &str = "LOCATION_RECOVERY_REGULAR_FILE_REQUIRED";
    /// Filesystem identity is unavailable.
    pub const RECOVERY_FILE_IDENTITY_UNAVAILABLE: &str =
        "LOCATION_RECOVERY_FILE_IDENTITY_UNAVAILABLE";
    /// Filesystem identity is unsupported.
    pub const RECOVERY_FILE_IDENTITY_UNSUPPORTED: &str =
        "LOCATION_RECOVERY_FILE_IDENTITY_UNSUPPORTED";
    /// Identity sidecar is unsupported or too large.
    pub const RECOVERY_IDENTITY_SIDECAR_INVALID: &str =
        "LOCATION_RECOVERY_IDENTITY_SIDECAR_INVALID";
    /// Folder identity evidence conflicts.
    pub const RECOVERY_IDENTITY_EVIDENCE_CONFLICT: &str =
        "LOCATION_RECOVERY_IDENTITY_EVIDENCE_CONFLICT";
    /// Tracked folder identity contradicts the title.
    pub const RECOVERY_FOLDER_TITLE_IDENTITY_MISMATCH: &str =
        "LOCATION_RECOVERY_FOLDER_TITLE_IDENTITY_MISMATCH";
    /// Content discovery limit reached.
    pub const RECOVERY_CONTENT_DISCOVERY_LIMIT_REACHED: &str =
        "LOCATION_RECOVERY_CONTENT_DISCOVERY_LIMIT_REACHED";
    /// Destination changed during content discovery.
    pub const RECOVERY_DESTINATION_CHANGED: &str = "LOCATION_RECOVERY_DESTINATION_CHANGED";
    /// Multiple destination files match the missing source hash.
    pub const RECOVERY_DESTINATION_HASH_AMBIGUOUS: &str =
        "LOCATION_RECOVERY_DESTINATION_HASH_AMBIGUOUS";
    /// No destination matches the missing source's full-content hash was not found.
    pub const RECOVERY_DESTINATION_HASH_NOT_FOUND: &str =
        "LOCATION_RECOVERY_DESTINATION_HASH_NOT_FOUND";
    /// Title ownership is not a root corruption case.
    pub const RECOVERY_ROOT_CORRUPTION_REQUIRED: &str =
        "LOCATION_RECOVERY_ROOT_CORRUPTION_REQUIRED";
    /// Title library has no configured roots.
    pub const RECOVERY_ROOTS_REQUIRED: &str = "LOCATION_RECOVERY_ROOTS_REQUIRED";
    /// Folder discovery limit reached; no automatic repair.
    pub const RECOVERY_FOLDER_DISCOVERY_LIMIT_REACHED: &str =
        "LOCATION_RECOVERY_FOLDER_DISCOVERY_LIMIT_REACHED";
    /// Multiple folders identify this title.
    pub const RECOVERY_FOLDER_IDENTITY_AMBIGUOUS: &str =
        "LOCATION_RECOVERY_FOLDER_IDENTITY_AMBIGUOUS";
    /// No media or exact-ID folder establishes ownership.
    pub const RECOVERY_FOLDER_OWNERSHIP_UNPROVEN: &str =
        "LOCATION_RECOVERY_FOLDER_OWNERSHIP_UNPROVEN";
    /// Selected root was not found.
    pub const RECOVERY_SELECTED_ROOT_NOT_FOUND: &str = "LOCATION_RECOVERY_SELECTED_ROOT_NOT_FOUND";
    /// Selected root is not below the corrupt ancestor.
    pub const RECOVERY_SELECTED_ROOT_OUTSIDE_ANCESTOR: &str =
        "LOCATION_RECOVERY_SELECTED_ROOT_OUTSIDE_ANCESTOR";
    /// Media has no parent folder.
    pub const RECOVERY_MEDIA_PARENT_REQUIRED: &str = "LOCATION_RECOVERY_MEDIA_PARENT_REQUIRED";
    /// Media is not loose at the corrupt ancestor.
    pub const RECOVERY_MEDIA_NOT_LOOSE_AT_ANCESTOR: &str =
        "LOCATION_RECOVERY_MEDIA_NOT_LOOSE_AT_ANCESTOR";
    /// Recovery destination overlaps configured recycle storage.
    pub const RECOVERY_RECYCLE_STORAGE_OVERLAP: &str = "LOCATION_RECOVERY_RECYCLE_STORAGE_OVERLAP";
    /// Destination is outside the title library.
    pub const RECOVERY_DESTINATION_OUTSIDE_LIBRARY: &str =
        "LOCATION_RECOVERY_DESTINATION_OUTSIDE_LIBRARY";
    /// Source parent is unavailable.
    pub const RECOVERY_SOURCE_PARENT_UNAVAILABLE: &str =
        "LOCATION_RECOVERY_SOURCE_PARENT_UNAVAILABLE";
    /// Tracked source is neither in the proven folder nor loose at the corrupt ancestor.
    pub const RECOVERY_SOURCE_LOCATION_UNPROVEN: &str =
        "LOCATION_RECOVERY_SOURCE_LOCATION_UNPROVEN";
    /// Source filename is unavailable.
    pub const RECOVERY_SOURCE_FILENAME_REQUIRED: &str =
        "LOCATION_RECOVERY_SOURCE_FILENAME_REQUIRED";
    /// Source changed while verifying.
    pub const RECOVERY_SOURCE_CHANGED: &str = "LOCATION_RECOVERY_SOURCE_CHANGED";
    /// Missing source has no attested full hash.
    pub const RECOVERY_SOURCE_FULL_HASH_REQUIRED: &str =
        "LOCATION_RECOVERY_SOURCE_FULL_HASH_REQUIRED";
    /// Missing full hash.
    pub const RECOVERY_FULL_HASH_REQUIRED: &str = "LOCATION_RECOVERY_FULL_HASH_REQUIRED";
    /// Recovery destination missing was not found.
    pub const RECOVERY_DESTINATION_NOT_FOUND: &str = "LOCATION_RECOVERY_DESTINATION_NOT_FOUND";
    /// Generated destination is not a directory.
    pub const RECOVERY_GENERATED_DESTINATION_NOT_DIRECTORY: &str =
        "LOCATION_RECOVERY_GENERATED_DESTINATION_NOT_DIRECTORY";
    /// Generated destination already exists without ownership evidence.
    pub const RECOVERY_GENERATED_DESTINATION_UNCLAIMED: &str =
        "LOCATION_RECOVERY_GENERATED_DESTINATION_UNCLAIMED";
    /// Generated destination is not this recovery's recorded folder.
    pub const RECOVERY_GENERATED_DESTINATION_IDENTITY_MISMATCH: &str =
        "LOCATION_RECOVERY_GENERATED_DESTINATION_IDENTITY_MISMATCH";
    /// Generated destination inventory limit exceeded.
    pub const RECOVERY_DESTINATION_INVENTORY_LIMIT_REACHED: &str =
        "LOCATION_RECOVERY_DESTINATION_INVENTORY_LIMIT_REACHED";
    /// Generated destination contains unrelated content; ownership remains unresolved.
    pub const RECOVERY_DESTINATION_UNRELATED_CONTENT: &str =
        "LOCATION_RECOVERY_DESTINATION_UNRELATED_CONTENT";
    /// Recovery is already running.
    pub const RECOVERY_ALREADY_RUNNING: &str = "LOCATION_RECOVERY_ALREADY_RUNNING";
    /// Recovery plan was not found.
    pub const RECOVERY_PLAN_NOT_FOUND: &str = "LOCATION_RECOVERY_PLAN_NOT_FOUND";
    /// Unsupported recovery plan.
    pub const RECOVERY_PLAN_UNSUPPORTED: &str = "LOCATION_RECOVERY_PLAN_UNSUPPORTED";
    /// Recovery root missing was not found.
    pub const RECOVERY_ROOT_NOT_FOUND: &str = "LOCATION_RECOVERY_ROOT_NOT_FOUND";
    /// Exact folder identity changed; ownership remains unresolved.
    pub const RECOVERY_FOLDER_IDENTITY_CHANGED: &str = "LOCATION_RECOVERY_FOLDER_IDENTITY_CHANGED";
    /// Recovery already committed; a missing receipt cannot authorize another copy.
    pub const RECOVERY_COMMIT_RECEIPT_REQUIRED: &str = "LOCATION_RECOVERY_COMMIT_RECEIPT_REQUIRED";
    /// Recovery file is not in the persisted plan.
    pub const RECOVERY_FILE_NOT_IN_PLAN: &str = "LOCATION_RECOVERY_FILE_NOT_IN_PLAN";
    /// Both source and destination are missing was not found.
    pub const RECOVERY_SOURCE_AND_DESTINATION_NOT_FOUND: &str =
        "LOCATION_RECOVERY_SOURCE_AND_DESTINATION_NOT_FOUND";
    /// Destination has no parent.
    pub const RECOVERY_DESTINATION_PARENT_REQUIRED: &str =
        "LOCATION_RECOVERY_DESTINATION_PARENT_REQUIRED";
    /// Destination could not be claimed; original and staged files preserved.
    pub const RECOVERY_DESTINATION_CLAIM_FAILED: &str =
        "LOCATION_RECOVERY_DESTINATION_CLAIM_FAILED";
    /// Recovery destination parent is not a directory.
    pub const RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY: &str =
        "LOCATION_RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY";
    /// Recovery destination has no existing ancestor was not found.
    pub const RECOVERY_DESTINATION_ANCESTOR_NOT_FOUND: &str =
        "LOCATION_RECOVERY_DESTINATION_ANCESTOR_NOT_FOUND";
    /// Recovery requires an independent destination copy, not a hardlink.
    pub const RECOVERY_INDEPENDENT_COPY_REQUIRED: &str =
        "LOCATION_RECOVERY_INDEPENDENT_COPY_REQUIRED";
    /// Missing source has no full-content proof.
    pub const RECOVERY_SOURCE_CONTENT_PROOF_REQUIRED: &str =
        "LOCATION_RECOVERY_SOURCE_CONTENT_PROOF_REQUIRED";
    /// Destination content does not match; both files preserved.
    pub const RECOVERY_DESTINATION_CONTENT_MISMATCH: &str =
        "LOCATION_RECOVERY_DESTINATION_CONTENT_MISMATCH";
    /// Title changed during recovery; original mappings preserved.
    pub const RECOVERY_TITLE_CHANGED: &str = "LOCATION_RECOVERY_TITLE_CHANGED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        STALE_PLAN,
        MERGE_UNAVAILABLE,
        VERIFICATION_MISMATCH,
        LIBRARY_NOT_FOUND,
        ROOT_NOT_FOUND,
        ROOT_NEW_PATH_REQUIRED,
        LIBRARY_ROOT_NOT_FOUND,
        OPERATION_NOT_FOUND,
        VERIFICATION_FAILED,
        TRANSFERRED_FILE_NEEDS_VERIFIED_AGAIN_BEFORE_CATALOG_ENTRY_UPDATED,
        TRANSFERRED_FILE_NOT_FINISHED_VERIFICATION,
        DESTINATION_OCCUPIED,
        PATH_NOT_FOUND,
        SELECTION_NOTHING_MOVE,
        OPERATION_STILL_RUNNING,
        OPERATION_SOURCE_LIBRARIES_UNAVAILABLE,
        TITLE_SELECTION_REQUIRED,
        DESTINATION_REQUIRED,
        ROOT_MOVE_SPANS_ONE_LIBRARY,
        CATALOG_CANNOT_MAP_FILE_OUTSIDE_RECORDED_FOLDER,
        CATALOG_PATH_AMBIGUOUS,
        MORE_ONE_CATALOG_RECORD_WOULD_POINT,
        DESTINATION_TITLE_NOT_FOUND,
        FOLDER_PATH_REQUIRED,
        PATH_COMPONENT_INVALID,
        FOLDER_NOT_INSIDE_ROOT_LIBRARY,
        FOLDER_NOT_TITLE_BOUNDARY,
        PATH_OUTSIDE_LIBRARY_ROOT,
        SELECTED_FOLDER_OVERLAPS_ANOTHER_TITLE_S_FOLDER,
        FOLDER_NOT_OWNED_ANOTHER_TITLE,
        FOLDER_ALREADY_OWNED_TITLE,
        TITLE_OWNS_NO_FOLDER_SWAP,
        RECORDED_FOLDER_LIBRARY_ROOT_NOT_FOLDER_SWAP,
        MERGE_TITLE_INTO_BLOCKED,
        SOURCE_DESTINATION_SAME_PATH,
        SOURCE_CHANGED_DURING_FILE_PLACEMENT,
        CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE,
        FILE_CHANGED_BEFORE_SOURCE_CLEANUP,
        DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE,
        SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP,
        SOURCE_CHANGED_BEFORE_CLEANUP,
        RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED,
        RELATED_MEDIA_TRANSFER_INTERRUPTED,
        DESTINATION_CHANGED_DURING_COMPARISON,
        SOURCE_CHANGED_DURING_TRANSFER,
        COULD_NOT_COMPARE,
        EXISTING_PATH_NOT_REGULAR_FILE,
        FILE_CHANGED_WHILE_BEING_COMPARED,
        FULL_HASH_BACKFILL_ALREADY_RUNNING,
        FULL_HASH_BACKFILL_CANCELLED,
        FILE_CHANGED_WHILE_HASHING,
        OWNERSHIP_BUSY,
        RECOVERY_ROOTS_CHANGED,
        RECOVERY_TITLE_NOT_FOUND,
        RECOVERY_OWNERSHIP_CHANGED,
        RECOVERY_TRACKED_MEDIA_CHANGED,
        RECOVERY_DESTINATION_UNPROVEN,
        RECOVERY_DESTINATION_ROOT_UNPROVEN,
        RECOVERY_DESTINATION_BOUNDARY_INVALID,
        RECOVERY_DESTINATION_OWNERSHIP_CONFLICT,
        RECOVERY_FILE_OWNERSHIP_CONFLICT,
        RECOVERY_DESTINATION_RECORD_CONFLICT,
        RECOVERY_DESTINATION_PATH_INVALID,
        RECOVERY_NATIVE_ABSOLUTE_PATH_REQUIRED,
        RECOVERY_DOT_SEGMENT_PATH_FORBIDDEN,
        RECOVERY_REPARSE_POINT_FORBIDDEN,
        RECOVERY_SYMLINK_FORBIDDEN,
        RECOVERY_PATH_UNAVAILABLE,
        RECOVERY_REGULAR_FILE_REQUIRED,
        RECOVERY_FILE_IDENTITY_UNAVAILABLE,
        RECOVERY_FILE_IDENTITY_UNSUPPORTED,
        RECOVERY_IDENTITY_SIDECAR_INVALID,
        RECOVERY_IDENTITY_EVIDENCE_CONFLICT,
        RECOVERY_FOLDER_TITLE_IDENTITY_MISMATCH,
        RECOVERY_CONTENT_DISCOVERY_LIMIT_REACHED,
        RECOVERY_DESTINATION_CHANGED,
        RECOVERY_DESTINATION_HASH_AMBIGUOUS,
        RECOVERY_DESTINATION_HASH_NOT_FOUND,
        RECOVERY_ROOT_CORRUPTION_REQUIRED,
        RECOVERY_ROOTS_REQUIRED,
        RECOVERY_FOLDER_DISCOVERY_LIMIT_REACHED,
        RECOVERY_FOLDER_IDENTITY_AMBIGUOUS,
        RECOVERY_FOLDER_OWNERSHIP_UNPROVEN,
        RECOVERY_SELECTED_ROOT_NOT_FOUND,
        RECOVERY_SELECTED_ROOT_OUTSIDE_ANCESTOR,
        RECOVERY_MEDIA_PARENT_REQUIRED,
        RECOVERY_MEDIA_NOT_LOOSE_AT_ANCESTOR,
        RECOVERY_RECYCLE_STORAGE_OVERLAP,
        RECOVERY_DESTINATION_OUTSIDE_LIBRARY,
        RECOVERY_SOURCE_PARENT_UNAVAILABLE,
        RECOVERY_SOURCE_LOCATION_UNPROVEN,
        RECOVERY_SOURCE_FILENAME_REQUIRED,
        RECOVERY_SOURCE_CHANGED,
        RECOVERY_SOURCE_FULL_HASH_REQUIRED,
        RECOVERY_FULL_HASH_REQUIRED,
        RECOVERY_DESTINATION_NOT_FOUND,
        RECOVERY_GENERATED_DESTINATION_NOT_DIRECTORY,
        RECOVERY_GENERATED_DESTINATION_UNCLAIMED,
        RECOVERY_GENERATED_DESTINATION_IDENTITY_MISMATCH,
        RECOVERY_DESTINATION_INVENTORY_LIMIT_REACHED,
        RECOVERY_DESTINATION_UNRELATED_CONTENT,
        RECOVERY_ALREADY_RUNNING,
        RECOVERY_PLAN_NOT_FOUND,
        RECOVERY_PLAN_UNSUPPORTED,
        RECOVERY_ROOT_NOT_FOUND,
        RECOVERY_FOLDER_IDENTITY_CHANGED,
        RECOVERY_COMMIT_RECEIPT_REQUIRED,
        RECOVERY_FILE_NOT_IN_PLAN,
        RECOVERY_SOURCE_AND_DESTINATION_NOT_FOUND,
        RECOVERY_DESTINATION_PARENT_REQUIRED,
        RECOVERY_DESTINATION_CLAIM_FAILED,
        RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY,
        RECOVERY_DESTINATION_ANCESTOR_NOT_FOUND,
        RECOVERY_INDEPENDENT_COPY_REQUIRED,
        RECOVERY_SOURCE_CONTENT_PROOF_REQUIRED,
        RECOVERY_DESTINATION_CONTENT_MISMATCH,
        RECOVERY_TITLE_CHANGED,
    ];
}

pub mod plugins {
    /// This plugin is already installed.
    pub const ALREADY_INSTALLED: &str = "PLUGIN_ALREADY_INSTALLED";
    /// Plugin  not installed was not found.
    pub const RESOURCE_NOT_FOUND: &str = "PLUGIN_RESOURCE_NOT_FOUND";
    /// Unknown plugin setting.
    pub const SETTING_UNKNOWN: &str = "PLUGIN_SETTING_UNKNOWN";
    /// Installed plugin settings descriptor is unavailable.
    pub const INSTALLED_PLUGIN_SETTINGS_DESCRIPTOR_UNAVAILABLE: &str =
        "PLUGIN_INSTALLED_PLUGIN_SETTINGS_DESCRIPTOR_UNAVAILABLE";
    /// Required plugin setting is missing.
    pub const REQUIRED_PLUGIN_SETTING_REQUIRED: &str = "PLUGIN_REQUIRED_PLUGIN_SETTING_REQUIRED";
    /// Invalid plugin setting value.
    pub const SETTING_VALUE_INVALID: &str = "PLUGIN_SETTING_VALUE_INVALID";
    /// Component replacement changed plugin identity.
    pub const COMPONENT_REPLACEMENT_CHANGED_PLUGIN_IDENTITY: &str =
        "PLUGIN_COMPONENT_REPLACEMENT_CHANGED_PLUGIN_IDENTITY";
    /// Component descriptor differs from its persisted metadata.
    pub const COMPONENT_DESCRIPTOR_DIFFERS_PERSISTED_METADATA: &str =
        "PLUGIN_COMPONENT_DESCRIPTOR_DIFFERS_PERSISTED_METADATA";
    /// The the selected item plugin is blocked on this version of Scryer, so it cannot be tested. the selected value.
    pub const BLOCKED_VERSION_SCRYER_SO_CANNOT_TESTED: &str =
        "PLUGIN_BLOCKED_VERSION_SCRYER_SO_CANNOT_TESTED";
    /// Manual plugin repo the selected item must publish exactly one plugin entry.
    pub const MANUAL_PLUGIN_REPO_MUST_PUBLISH_EXACTLY_ONE_PLUGIN_ENTRY: &str =
        "PLUGIN_MANUAL_PLUGIN_REPO_MUST_PUBLISH_EXACTLY_ONE_PLUGIN_ENTRY";
    /// Manual plugin repo the selected item published plugin the selected item from source repo the selected item.
    pub const MANUAL_PLUGIN_REPO_PUBLISHED_PLUGIN_SOURCE_REPO: &str =
        "PLUGIN_MANUAL_PLUGIN_REPO_PUBLISHED_PLUGIN_SOURCE_REPO";
    /// Manual plugin upload must be a .wasm or .wasm.zst file.
    pub const MANUAL_PLUGIN_UPLOAD_INVALID: &str = "PLUGIN_MANUAL_PLUGIN_UPLOAD_INVALID";
    /// Manual plugin repo the selected item has no SDK-compatible release.
    pub const MANUAL_PLUGIN_REPO_NO_SDK_COMPATIBLE_RELEASE: &str =
        "PLUGIN_MANUAL_PLUGIN_REPO_NO_SDK_COMPATIBLE_RELEASE";
    /// Manual plugin catalog is not UTF-8.
    pub const MANUAL_PLUGIN_CATALOG_NOT_UTF_8: &str = "PLUGIN_MANUAL_PLUGIN_CATALOG_NOT_UTF_8";
    /// Manual plugin upload requires explicit risk acknowledgement.
    pub const MANUAL_PLUGIN_UPLOAD_REQUIRES_EXPLICIT_RISK_ACKNOWLEDGEMENT: &str =
        "PLUGIN_MANUAL_PLUGIN_UPLOAD_REQUIRES_EXPLICIT_RISK_ACKNOWLEDGEMENT";
    /// Manual plugin upload file name is required.
    pub const MANUAL_PLUGIN_UPLOAD_FILE_NAME_REQUIRED: &str =
        "PLUGIN_MANUAL_PLUGIN_UPLOAD_FILE_NAME_REQUIRED";
    /// Manual plugin upload payload is not valid base64.
    pub const MANUAL_PLUGIN_UPLOAD_PAYLOAD_INVALID: &str =
        "PLUGIN_MANUAL_PLUGIN_UPLOAD_PAYLOAD_INVALID";
    /// Provider type the selected item is reserved for first-party code.
    pub const PROVIDER_TYPE_RESERVED_FIRST_PARTY_CODE: &str =
        "PLUGIN_PROVIDER_TYPE_RESERVED_FIRST_PARTY_CODE";
    /// Plugin the selected item is a bundled plugin; uninstall any downloaded override before uploading a local build.
    pub const BUNDLED_PLUGIN: &str = "PLUGIN_BUNDLED_PLUGIN";
    /// Rule pack catalog changed during download.
    pub const RULE_PACK_CATALOG_CHANGED_DURING_DOWNLOAD: &str =
        "PLUGIN_RULE_PACK_CATALOG_CHANGED_DURING_DOWNLOAD";
    /// Unsupported plugin_type the selected item for built-in preparation.
    pub const TYPE_BUILT_PREPARATION_UNSUPPORTED: &str =
        "PLUGIN_TYPE_BUILT_PREPARATION_UNSUPPORTED";
    /// Unsupported plugin_type the selected item for built-in restore.
    pub const TYPE_BUILT_RESTORE_UNSUPPORTED: &str = "PLUGIN_TYPE_BUILT_RESTORE_UNSUPPORTED";
    /// The requested value does not include a blake3 digest.
    pub const DOES_NOT_INCLUDE_BLAKE3_DIGEST: &str = "PLUGIN_DOES_NOT_INCLUDE_BLAKE3_DIGEST";
    /// Plugin the selected item is missing persisted wasm digest algorithm.
    pub const MISSING_PERSISTED_WASM_DIGEST_ALGORITHM: &str =
        "PLUGIN_MISSING_PERSISTED_WASM_DIGEST_ALGORITHM";
    /// Plugin the selected item is missing persisted wasm digest value.
    pub const MISSING_PERSISTED_WASM_DIGEST_VALUE: &str =
        "PLUGIN_MISSING_PERSISTED_WASM_DIGEST_VALUE";
    /// Plugin the selected item is missing persisted descriptor_json.
    pub const MISSING_PERSISTED_DESCRIPTOR_JSON: &str = "PLUGIN_MISSING_PERSISTED_DESCRIPTOR_JSON";
    /// Plugin the selected item has invalid persisted descriptor_json.
    pub const INVALID_PERSISTED_DESCRIPTOR_JSON: &str = "PLUGIN_INVALID_PERSISTED_DESCRIPTOR_JSON";
    /// Plugin the selected item requests the host-process capability, which is reserved for official plugins.
    pub const REQUESTS_HOST_PROCESS_CAPABILITY_WHICH_RESERVED_OFFICIAL_PLUGINS: &str =
        "PLUGIN_REQUESTS_HOST_PROCESS_CAPABILITY_WHICH_RESERVED_OFFICIAL_PLUGINS";
    /// Downloaded plugin descriptor id the selected item does not match registry id the selected item.
    pub const DESCRIPTOR_ID_MISMATCH: &str = "PLUGIN_DESCRIPTOR_ID_MISMATCH";
    /// Downloaded plugin the selected item has plugin_type the selected item but registry expects the selected item.
    pub const DOWNLOADED_PLUGIN_PLUGIN_TYPE_BUT_REGISTRY_EXPECTS: &str =
        "PLUGIN_DOWNLOADED_PLUGIN_PLUGIN_TYPE_BUT_REGISTRY_EXPECTS";
    /// Downloaded plugin the selected item has provider_type the selected item but registry expects the selected item.
    pub const DOWNLOADED_PLUGIN_PROVIDER_TYPE_BUT_REGISTRY_EXPECTS: &str =
        "PLUGIN_DOWNLOADED_PLUGIN_PROVIDER_TYPE_BUT_REGISTRY_EXPECTS";
    /// Downloaded plugin the selected item has version the selected item but the selected release is the selected item.
    pub const DOWNLOADED_PLUGIN_VERSION_BUT_SELECTED_RELEASE: &str =
        "PLUGIN_DOWNLOADED_PLUGIN_VERSION_BUT_SELECTED_RELEASE";
    /// Downloaded plugin the selected item has sdk_version the selected item but the selected release is the selected item.
    pub const DOWNLOADED_PLUGIN_SDK_VERSION_BUT_SELECTED_RELEASE: &str =
        "PLUGIN_DOWNLOADED_PLUGIN_SDK_VERSION_BUT_SELECTED_RELEASE";
    /// Plugin the selected item no longer has a host-compatible release for this Scryer version.
    pub const NO_LONGER_HOST_COMPATIBLE_RELEASE_SCRYER_VERSION: &str =
        "PLUGIN_NO_LONGER_HOST_COMPATIBLE_RELEASE_SCRYER_VERSION";
    /// Plugin the selected item is missing persisted WASM payload.
    pub const MISSING_PERSISTED_WASM_PAYLOAD: &str = "PLUGIN_MISSING_PERSISTED_WASM_PAYLOAD";
    /// Unsupported plugin_type the selected item for runtime removal.
    pub const TYPE_RUNTIME_REMOVAL_UNSUPPORTED: &str = "PLUGIN_TYPE_RUNTIME_REMOVAL_UNSUPPORTED";
    /// Plugin  is not available from the plugin catalog was not found.
    pub const NOT_AVAILABLE_PLUGIN_CATALOG_NOT_FOUND: &str =
        "PLUGIN_NOT_AVAILABLE_PLUGIN_CATALOG_NOT_FOUND";
    /// Plugin the selected item WASM artifact declared size the selected value exceeds maximum size of the selected value bytes.
    pub const WASM_ARTIFACT_TOO_LARGE: &str = "PLUGIN_WASM_ARTIFACT_TOO_LARGE";
    /// Plugin the selected item selected artifact the selected item has unsupported encoding.
    pub const SELECTED_ARTIFACT_UNSUPPORTED_ENCODING: &str =
        "PLUGIN_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING";
    /// Plugin the selected item selected artifact the selected item is too large to validate bytes.
    pub const SELECTED_ARTIFACT_TOO_LARGE_VALIDATE_BYTES: &str =
        "PLUGIN_SELECTED_ARTIFACT_TOO_LARGE_VALIDATE_BYTES";
    /// Plugin the selected item selected artifact the selected item bytes mismatch: expected the selected value, got the selected value.
    pub const SELECTED_ARTIFACT_BYTES_MISMATCH: &str = "PLUGIN_SELECTED_ARTIFACT_BYTES_MISMATCH";
    /// Invalid catalog version the selected item.
    pub const CATALOG_VERSION_INVALID: &str = "PLUGIN_CATALOG_VERSION_INVALID";
    /// Invalid installed version the selected item.
    pub const INSTALLED_VERSION_INVALID: &str = "PLUGIN_INSTALLED_VERSION_INVALID";
    /// Plugin the selected item is already at version the selected value.
    pub const ALREADY_VERSION_SELECTED_RELEASE: &str = "PLUGIN_ALREADY_VERSION_SELECTED_RELEASE";
    /// Official nzbgeek plugin is not available for builtin migration.
    pub const OFFICIAL_NZBGEEK_PLUGIN_NOT_AVAILABLE_BUILTIN_MIGRATION_NOT_FOUND: &str =
        "PLUGIN_OFFICIAL_NZBGEEK_PLUGIN_NOT_AVAILABLE_BUILTIN_MIGRATION_NOT_FOUND";
    /// Manual plugin the selected item is missing source repo.
    pub const MANUAL_PLUGIN_MISSING_SOURCE_REPO: &str = "PLUGIN_MANUAL_PLUGIN_MISSING_SOURCE_REPO";
    /// Unsupported plugin_type the selected item for runtime upsert.
    pub const TYPE_RUNTIME_UPSERT_UNSUPPORTED: &str = "PLUGIN_TYPE_RUNTIME_UPSERT_UNSUPPORTED";
    /// Plugin is no longer installed was not found.
    pub const NO_LONGER_INSTALLED_NOT_FOUND: &str = "PLUGIN_NO_LONGER_INSTALLED_NOT_FOUND";
    /// Plugin changed while its runtime was being prepared.
    pub const CHANGED_WHILE_RUNTIME_BEING_PREPARED: &str =
        "PLUGIN_CHANGED_WHILE_RUNTIME_BEING_PREPARED";
    /// Cannot uninstall built-in plugins; disable them instead.
    pub const CANNOT_UNINSTALL_BUILT_PLUGINS: &str = "PLUGIN_CANNOT_UNINSTALL_BUILT_PLUGINS";
    /// Cannot revert built-in plugin the selected item because no bundled definition is available.
    pub const CANNOT_REVERT_BUILT_PLUGIN_NO_BUNDLED_DEFINITION_UNAVAILABLE: &str =
        "PLUGIN_CANNOT_REVERT_BUILT_PLUGIN_NO_BUNDLED_DEFINITION_UNAVAILABLE";
    /// Proxy configuration was not found.
    pub const PROXY_CONFIGURATION_NOT_FOUND: &str = "PLUGIN_PROXY_CONFIGURATION_NOT_FOUND";
    /// Proxy is disabled for this download client.
    pub const PROXY_DISABLED_DOWNLOAD_CLIENT: &str = "PLUGIN_PROXY_DISABLED_DOWNLOAD_CLIENT";
    /// Test connection is not supported for client type the selected item.
    pub const TEST_CONNECTION_UNSUPPORTED: &str = "PLUGIN_TEST_CONNECTION_UNSUPPORTED";
    /// Invalid client config_json.
    pub const CLIENT_CONFIG_JSON_INVALID: &str = "PLUGIN_CLIENT_CONFIG_JSON_INVALID";
    /// Invalid GitHub repository URL.
    pub const GITHUB_REPOSITORY_URL_INVALID: &str = "PLUGIN_GITHUB_REPOSITORY_URL_INVALID";
    /// Manual plugin repositories must be hosted on github.com.
    pub const MANUAL_PLUGIN_REPOSITORIES_INVALID: &str =
        "PLUGIN_MANUAL_PLUGIN_REPOSITORIES_INVALID";
    /// GitHub owner is missing.
    pub const GITHUB_OWNER_REQUIRED: &str = "PLUGIN_GITHUB_OWNER_REQUIRED";
    /// GitHub repo name is missing.
    pub const GITHUB_REPO_NAME_REQUIRED: &str = "PLUGIN_GITHUB_REPO_NAME_REQUIRED";
    /// GitHub repository must include owner and repo.
    pub const GITHUB_REPOSITORY_INVALID: &str = "PLUGIN_GITHUB_REPOSITORY_INVALID";
    /// Invalid plugin catalog v3 JSON.
    pub const CATALOG_V3_JSON_INVALID: &str = "PLUGIN_CATALOG_V3_JSON_INVALID";
    /// Invalid plugin catalog redirect JSON.
    pub const CATALOG_REDIRECT_JSON_INVALID: &str = "PLUGIN_CATALOG_REDIRECT_JSON_INVALID";
    /// Plugin signature verification is not compiled into this target.
    pub const SIGNATURE_VERIFICATION_NOT_COMPILED_INTO_TARGET: &str =
        "PLUGIN_SIGNATURE_VERIFICATION_NOT_COMPILED_INTO_TARGET";
    /// The requested value decompressed size limit the selected value exceeds this platform's maximum buffer size.
    pub const DECOMPRESSED_SIZE_LIMIT_EXCEEDS_PLATFORM_S_MAXIMUM_BUFFER_SIZE: &str =
        "PLUGIN_DECOMPRESSED_SIZE_LIMIT_EXCEEDS_PLATFORM_S_MAXIMUM_BUFFER_SIZE";
    /// The requested value decompressed payload exceeds maximum size of the selected value bytes.
    pub const DECOMPRESSED_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES: &str =
        "PLUGIN_DECOMPRESSED_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES";
    /// The requested value payload is too large to validate decompressed size.
    pub const PAYLOAD_TOO_LARGE_VALIDATE_DECOMPRESSED_SIZE: &str =
        "PLUGIN_PAYLOAD_TOO_LARGE_VALIDATE_DECOMPRESSED_SIZE";
    /// The requested value payload exceeds maximum size of the selected value bytes.
    pub const PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES: &str =
        "PLUGIN_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES";
    /// Plugin zstd decompression is not compiled into this target.
    pub const ZSTD_DECOMPRESSION_NOT_COMPILED_INTO_TARGET: &str =
        "PLUGIN_ZSTD_DECOMPRESSION_NOT_COMPILED_INTO_TARGET";
    /// Plugin brotli decompression is not compiled into this target.
    pub const BROTLI_DECOMPRESSION_NOT_COMPILED_INTO_TARGET: &str =
        "PLUGIN_BROTLI_DECOMPRESSION_NOT_COMPILED_INTO_TARGET";
    /// Plugin zstd compression is not compiled into this target.
    pub const ZSTD_COMPRESSION_NOT_COMPILED_INTO_TARGET: &str =
        "PLUGIN_ZSTD_COMPRESSION_NOT_COMPILED_INTO_TARGET";
    /// Invalid digest string the selected item.
    pub const DIGEST_STRING_INVALID: &str = "PLUGIN_DIGEST_STRING_INVALID";
    /// Digest algorithm is missing.
    pub const DIGEST_ALGORITHM_REQUIRED: &str = "PLUGIN_DIGEST_ALGORITHM_REQUIRED";
    /// The requested value digest mismatch: expected blake3.
    pub const DIGEST_MISMATCH: &str = "PLUGIN_DIGEST_MISMATCH";
    /// The requested value uses unsupported digest algorithm the selected item.
    pub const USES_UNSUPPORTED_DIGEST_ALGORITHM: &str = "PLUGIN_USES_UNSUPPORTED_DIGEST_ALGORITHM";
    /// The requested value is missing a usable blake3 digest.
    pub const MISSING_USABLE_BLAKE3_DIGEST: &str = "PLUGIN_MISSING_USABLE_BLAKE3_DIGEST";
    /// The requested value must include at least one digest.
    pub const ARTIFACT_DIGEST_REQUIRED: &str = "PLUGIN_ARTIFACT_DIGEST_REQUIRED";
    /// Unsupported plugin catalog redirect schema the selected item.
    pub const CATALOG_REDIRECT_SCHEMA_UNSUPPORTED: &str =
        "PLUGIN_CATALOG_REDIRECT_SCHEMA_UNSUPPORTED";
    /// Plugin catalog redirect catalog_version must be greater than zero.
    pub const CATALOG_REDIRECT_CATALOG_VERSION_OUT_OF_RANGE: &str =
        "PLUGIN_CATALOG_REDIRECT_CATALOG_VERSION_OUT_OF_RANGE";
    /// Plugin catalog redirect must include at least one artifact.
    pub const CATALOG_REDIRECT_OUT_OF_RANGE: &str = "PLUGIN_CATALOG_REDIRECT_OUT_OF_RANGE";
    /// Unsupported plugin catalog schema the selected item.
    pub const CATALOG_SCHEMA_UNSUPPORTED: &str = "PLUGIN_CATALOG_SCHEMA_UNSUPPORTED";
    /// Plugin catalog version must be greater than zero.
    pub const CATALOG_VERSION_OUT_OF_RANGE: &str = "PLUGIN_CATALOG_VERSION_OUT_OF_RANGE";
    /// Duplicate plugin id the selected item in plugin catalog.
    pub const ID_PLUGIN_CATALOG_DUPLICATED: &str = "PLUGIN_ID_PLUGIN_CATALOG_DUPLICATED";
    /// Plugin the selected item signer repo the selected item does not match source repo the selected item.
    pub const SIGNER_REPO_DOES_NOT_MATCH_SOURCE_REPO: &str =
        "PLUGIN_SIGNER_REPO_DOES_NOT_MATCH_SOURCE_REPO";
    /// Community source the selected item support tier must be verified_community.
    pub const COMMUNITY_SOURCE_SUPPORT_TIER_INVALID: &str =
        "PLUGIN_COMMUNITY_SOURCE_SUPPORT_TIER_INVALID";
    /// Duplicate community source id the selected item in plugin catalog.
    pub const COMMUNITY_SOURCE_ID_PLUGIN_CATALOG_DUPLICATED: &str =
        "PLUGIN_COMMUNITY_SOURCE_ID_PLUGIN_CATALOG_DUPLICATED";
    /// Duplicate rule pack id the selected item in plugin catalog.
    pub const RULE_PACK_ID_PLUGIN_CATALOG_DUPLICATED: &str =
        "PLUGIN_RULE_PACK_ID_PLUGIN_CATALOG_DUPLICATED";
    /// Duplicate rule pack release version the selected item for the selected item.
    pub const RULE_PACK_RELEASE_VERSION_DUPLICATED: &str =
        "PLUGIN_RULE_PACK_RELEASE_VERSION_DUPLICATED";
    /// Rule pack the selected item has invalid min_scryer_version the selected item.
    pub const RULE_PACK_INVALID_MIN_SCRYER_VERSION: &str =
        "PLUGIN_RULE_PACK_INVALID_MIN_SCRYER_VERSION";
    /// Rule pack the selected item release the selected item must include rule_pack_digests.
    pub const RULE_PACK_RELEASE_INVALID: &str = "PLUGIN_RULE_PACK_RELEASE_INVALID";
    /// Invalid sdk_constraint the selected item for plugin the selected item.
    pub const SDK_CONSTRAINT_PLUGIN_INVALID: &str = "PLUGIN_SDK_CONSTRAINT_PLUGIN_INVALID";
    /// Plugin the selected item release the selected item has invalid min_scryer_version.
    pub const RELEASE_INVALID_MIN_SCRYER_VERSION: &str =
        "PLUGIN_RELEASE_INVALID_MIN_SCRYER_VERSION";
    /// Plugin the selected item release the selected item has invalid max_scryer_version.
    pub const RELEASE_INVALID_MAX_SCRYER_VERSION: &str =
        "PLUGIN_RELEASE_INVALID_MAX_SCRYER_VERSION";
    /// Plugin the selected item release the selected item has min_scryer_version greater than max_scryer_version.
    pub const RELEASE_MIN_SCRYER_VERSION_GREATER_MAX_SCRYER_VERSION: &str =
        "PLUGIN_RELEASE_MIN_SCRYER_VERSION_GREATER_MAX_SCRYER_VERSION";
    /// Duplicate release version the selected item for plugin the selected item.
    pub const RELEASE_VERSION_PLUGIN_DUPLICATED: &str = "PLUGIN_RELEASE_VERSION_PLUGIN_DUPLICATED";
    /// The selected item release the selected item must include at least one artifact.
    pub const RELEASE_OUT_OF_RANGE: &str = "PLUGIN_RELEASE_OUT_OF_RANGE";
    /// Plugin the selected item release the selected item has duplicate artifact variant/encoding rows.
    pub const RELEASE_DUPLICATE_ARTIFACT_VARIANT_ENCODING_ROWS: &str =
        "PLUGIN_RELEASE_DUPLICATE_ARTIFACT_VARIANT_ENCODING_ROWS";
    /// Plugin the selected item release the selected item artifact the selected item must include wasm_digests.
    pub const RELEASE_ARTIFACT_INVALID: &str = "PLUGIN_RELEASE_ARTIFACT_INVALID";
    /// Plugin the selected item release the selected item artifact the selected item bytes must be greater than zero.
    pub const RELEASE_ARTIFACT_BYTES_OUT_OF_RANGE: &str =
        "PLUGIN_RELEASE_ARTIFACT_BYTES_OUT_OF_RANGE";
    /// The requested value URL the selected item must use http or https, got the selected item.
    pub const URL_INVALID: &str = "PLUGIN_URL_INVALID";
    /// The requested value is required.
    pub const FIELD_REQUIRED: &str = "PLUGIN_FIELD_REQUIRED";
    /// Invalid plugin release version the selected item.
    pub const RELEASE_VERSION_INVALID: &str = "PLUGIN_RELEASE_VERSION_INVALID";
    /// The requested value must use a supported <algorithm>:<hex> digest.
    pub const MUST_USE_SUPPORTED_ALGORITHM_HEX_DIGEST: &str =
        "PLUGIN_MUST_USE_SUPPORTED_ALGORITHM_HEX_DIGEST";
    /// Digest value is missing.
    pub const DIGEST_VALUE_REQUIRED: &str = "PLUGIN_DIGEST_VALUE_REQUIRED";
    /// Digest value the selected item must be hexadecimal.
    pub const DIGEST_VALUE_INVALID: &str = "PLUGIN_DIGEST_VALUE_INVALID";
    /// Rule pack the selected item selected artifact the selected item has unsupported encoding.
    pub const RULE_PACK_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING: &str =
        "PLUGIN_RULE_PACK_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING";
    /// Community catalog source the selected item approved tier must be verified_community.
    pub const COMMUNITY_CATALOG_SOURCE_APPROVED_TIER_INVALID: &str =
        "PLUGIN_COMMUNITY_CATALOG_SOURCE_APPROVED_TIER_INVALID";
    /// Community catalog source the selected item must publish exactly one plugin.
    pub const COMMUNITY_CATALOG_SOURCE_MUST_PUBLISH_EXACTLY_ONE_PLUGIN: &str =
        "PLUGIN_COMMUNITY_CATALOG_SOURCE_MUST_PUBLISH_EXACTLY_ONE_PLUGIN";
    /// Community catalog source the selected item must not publish rule packs.
    pub const COMMUNITY_RULE_PACK_PUBLISH_FORBIDDEN: &str =
        "PLUGIN_COMMUNITY_RULE_PACK_PUBLISH_FORBIDDEN";
    /// Community catalog source the selected item must not publish nested community sources.
    pub const COMMUNITY_SOURCE_NESTING_FORBIDDEN: &str =
        "PLUGIN_COMMUNITY_SOURCE_NESTING_FORBIDDEN";
    /// Community catalog source the selected item published plugin the selected item.
    pub const COMMUNITY_CATALOG_SOURCE_PUBLISHED_PLUGIN: &str =
        "PLUGIN_COMMUNITY_CATALOG_SOURCE_PUBLISHED_PLUGIN";
    /// Community catalog source the selected item support tier does not match approved tier.
    pub const COMMUNITY_CATALOG_SOURCE_SUPPORT_TIER_DOES_NOT_MATCH_APPROVED_TIER: &str =
        "PLUGIN_COMMUNITY_CATALOG_SOURCE_SUPPORT_TIER_DOES_NOT_MATCH_APPROVED_TIER";
    /// Community catalog source the selected item signer repo the selected item does not match approved repo the selected item.
    pub const COMMUNITY_CATALOG_SOURCE_SIGNER_REPO_DOES_NOT_MATCH_APPROVED_REPO: &str =
        "PLUGIN_COMMUNITY_CATALOG_SOURCE_SIGNER_REPO_DOES_NOT_MATCH_APPROVED_REPO";
    /// Community catalog source the selected item source repo the selected item does not match approved repo the selected item.
    pub const COMMUNITY_CATALOG_SOURCE_SOURCE_REPO_DOES_NOT_MATCH_APPROVED_REPO: &str =
        "PLUGIN_COMMUNITY_CATALOG_SOURCE_SOURCE_REPO_DOES_NOT_MATCH_APPROVED_REPO";
    /// Manual plugin catalog source the selected item is missing github repo.
    pub const MANUAL_PLUGIN_CATALOG_SOURCE_MISSING_GITHUB_REPO: &str =
        "PLUGIN_MANUAL_PLUGIN_CATALOG_SOURCE_MISSING_GITHUB_REPO";
    /// Plugin catalog redirect did not contain any artifacts.
    pub const CATALOG_ARTIFACTS_MISSING: &str = "PLUGIN_CATALOG_ARTIFACTS_MISSING";
    /// Plugin catalog redirect did not yield a usable catalog.
    pub const CATALOG_REDIRECT_DID_NOT_YIELD_USABLE_CATALOG: &str =
        "PLUGIN_CATALOG_REDIRECT_DID_NOT_YIELD_USABLE_CATALOG";
    /// Invalid installed pack version.
    pub const INSTALLED_PACK_VERSION_INVALID: &str = "PLUGIN_INSTALLED_PACK_VERSION_INVALID";
    /// Rule pack was not found.
    pub const RULE_PACK_NOT_FOUND: &str = "PLUGIN_RULE_PACK_NOT_FOUND";
    /// Rule pack the selected item has no compatible artifact.
    pub const RULE_PACK_NO_COMPATIBLE_ARTIFACT: &str = "PLUGIN_RULE_PACK_NO_COMPATIBLE_ARTIFACT";
    /// Rule pack the selected item uses unsupported schema version the selected value.
    pub const RULE_PACK_USES_UNSUPPORTED_SCHEMA_VERSION: &str =
        "PLUGIN_RULE_PACK_USES_UNSUPPORTED_SCHEMA_VERSION";
    /// Rule pack the selected item does not match the catalog id/version.
    pub const RULE_PACK_DOES_NOT_MATCH_CATALOG_ID_VERSION: &str =
        "PLUGIN_RULE_PACK_DOES_NOT_MATCH_CATALOG_ID_VERSION";
    /// Rule pack the selected item has an empty the selected value.
    pub const RULE_PACK_EMPTY: &str = "PLUGIN_RULE_PACK_EMPTY";
    /// Rule pack the selected item contains no rules.
    pub const RULE_PACK_CONTAINS_NO_RULES: &str = "PLUGIN_RULE_PACK_CONTAINS_NO_RULES";
    /// Rule pack the selected item contains a rule with required fields missing.
    pub const RULE_PACK_CONTAINS_RULE_REQUIRED_FIELDS_MISSING: &str =
        "PLUGIN_RULE_PACK_CONTAINS_RULE_REQUIRED_FIELDS_MISSING";
    /// Rule pack the selected item contains duplicate rule id the selected item.
    pub const RULE_PACK_CONTAINS_DUPLICATE_RULE_ID: &str =
        "PLUGIN_RULE_PACK_CONTAINS_DUPLICATE_RULE_ID";
    /// Plugin artifact redirects are not allowed for the selected value.
    pub const ARTIFACT_REDIRECTS_NOT_ALLOWED: &str = "PLUGIN_ARTIFACT_REDIRECTS_NOT_ALLOWED";
    /// Plugin artifact redirect limit exceeded for the selected value.
    pub const ARTIFACT_REDIRECT_LIMIT_EXCEEDED: &str = "PLUGIN_ARTIFACT_REDIRECT_LIMIT_EXCEEDED";
    /// Plugin artifact redirect for the selected value did not include a Location header.
    pub const ARTIFACT_REDIRECT_DID_NOT_INCLUDE_LOCATION_HEADER: &str =
        "PLUGIN_ARTIFACT_REDIRECT_DID_NOT_INCLUDE_LOCATION_HEADER";
    /// Plugin artifact redirect for the selected value included an invalid Location header.
    pub const ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_HEADER: &str =
        "PLUGIN_ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_HEADER";
    /// Plugin artifact redirect for the selected value included an invalid Location URL.
    pub const ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_URL: &str =
        "PLUGIN_ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_URL";
    /// The plugin artifact could not be trusted.
    pub const ARTIFACT_UNTRUSTED: &str = "PLUGIN_ARTIFACT_UNTRUSTED";
    /// The plugin upgrade is blocked by installed components.
    pub const COMPONENT_UPGRADE_BLOCKED: &str = "PLUGIN_COMPONENT_UPGRADE_BLOCKED";
    /// The artifact URL resolves to a destination that policy forbids.
    pub const ARTIFACT_DESTINATION_FORBIDDEN: &str = "PLUGIN_ARTIFACT_DESTINATION_FORBIDDEN";
    /// The artifact host could not be resolved.
    pub const ARTIFACT_HOST_UNAVAILABLE: &str = "PLUGIN_ARTIFACT_HOST_UNAVAILABLE";
    /// The plugin descriptor is incompatible with this SDK.
    pub const DESCRIPTOR_SDK_INCOMPATIBLE: &str = "PLUGIN_DESCRIPTOR_SDK_INCOMPATIBLE";
    /// The plugin descriptor requests invalid host permissions.
    pub const DESCRIPTOR_HOST_PERMISSIONS_INVALID: &str =
        "PLUGIN_DESCRIPTOR_HOST_PERMISSIONS_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        DESCRIPTOR_SDK_INCOMPATIBLE,
        DESCRIPTOR_HOST_PERMISSIONS_INVALID,
        ARTIFACT_DESTINATION_FORBIDDEN,
        ARTIFACT_HOST_UNAVAILABLE,
        ALREADY_INSTALLED,
        RESOURCE_NOT_FOUND,
        SETTING_UNKNOWN,
        INSTALLED_PLUGIN_SETTINGS_DESCRIPTOR_UNAVAILABLE,
        REQUIRED_PLUGIN_SETTING_REQUIRED,
        SETTING_VALUE_INVALID,
        COMPONENT_REPLACEMENT_CHANGED_PLUGIN_IDENTITY,
        COMPONENT_DESCRIPTOR_DIFFERS_PERSISTED_METADATA,
        BLOCKED_VERSION_SCRYER_SO_CANNOT_TESTED,
        MANUAL_PLUGIN_REPO_MUST_PUBLISH_EXACTLY_ONE_PLUGIN_ENTRY,
        MANUAL_PLUGIN_REPO_PUBLISHED_PLUGIN_SOURCE_REPO,
        MANUAL_PLUGIN_UPLOAD_INVALID,
        MANUAL_PLUGIN_REPO_NO_SDK_COMPATIBLE_RELEASE,
        MANUAL_PLUGIN_CATALOG_NOT_UTF_8,
        MANUAL_PLUGIN_UPLOAD_REQUIRES_EXPLICIT_RISK_ACKNOWLEDGEMENT,
        MANUAL_PLUGIN_UPLOAD_FILE_NAME_REQUIRED,
        MANUAL_PLUGIN_UPLOAD_PAYLOAD_INVALID,
        PROVIDER_TYPE_RESERVED_FIRST_PARTY_CODE,
        BUNDLED_PLUGIN,
        RULE_PACK_CATALOG_CHANGED_DURING_DOWNLOAD,
        TYPE_BUILT_PREPARATION_UNSUPPORTED,
        TYPE_BUILT_RESTORE_UNSUPPORTED,
        DOES_NOT_INCLUDE_BLAKE3_DIGEST,
        MISSING_PERSISTED_WASM_DIGEST_ALGORITHM,
        MISSING_PERSISTED_WASM_DIGEST_VALUE,
        MISSING_PERSISTED_DESCRIPTOR_JSON,
        INVALID_PERSISTED_DESCRIPTOR_JSON,
        REQUESTS_HOST_PROCESS_CAPABILITY_WHICH_RESERVED_OFFICIAL_PLUGINS,
        DESCRIPTOR_ID_MISMATCH,
        DOWNLOADED_PLUGIN_PLUGIN_TYPE_BUT_REGISTRY_EXPECTS,
        DOWNLOADED_PLUGIN_PROVIDER_TYPE_BUT_REGISTRY_EXPECTS,
        DOWNLOADED_PLUGIN_VERSION_BUT_SELECTED_RELEASE,
        DOWNLOADED_PLUGIN_SDK_VERSION_BUT_SELECTED_RELEASE,
        NO_LONGER_HOST_COMPATIBLE_RELEASE_SCRYER_VERSION,
        MISSING_PERSISTED_WASM_PAYLOAD,
        TYPE_RUNTIME_REMOVAL_UNSUPPORTED,
        NOT_AVAILABLE_PLUGIN_CATALOG_NOT_FOUND,
        WASM_ARTIFACT_TOO_LARGE,
        SELECTED_ARTIFACT_UNSUPPORTED_ENCODING,
        SELECTED_ARTIFACT_TOO_LARGE_VALIDATE_BYTES,
        SELECTED_ARTIFACT_BYTES_MISMATCH,
        CATALOG_VERSION_INVALID,
        INSTALLED_VERSION_INVALID,
        ALREADY_VERSION_SELECTED_RELEASE,
        OFFICIAL_NZBGEEK_PLUGIN_NOT_AVAILABLE_BUILTIN_MIGRATION_NOT_FOUND,
        MANUAL_PLUGIN_MISSING_SOURCE_REPO,
        TYPE_RUNTIME_UPSERT_UNSUPPORTED,
        NO_LONGER_INSTALLED_NOT_FOUND,
        CHANGED_WHILE_RUNTIME_BEING_PREPARED,
        CANNOT_UNINSTALL_BUILT_PLUGINS,
        CANNOT_REVERT_BUILT_PLUGIN_NO_BUNDLED_DEFINITION_UNAVAILABLE,
        PROXY_CONFIGURATION_NOT_FOUND,
        PROXY_DISABLED_DOWNLOAD_CLIENT,
        TEST_CONNECTION_UNSUPPORTED,
        CLIENT_CONFIG_JSON_INVALID,
        GITHUB_REPOSITORY_URL_INVALID,
        MANUAL_PLUGIN_REPOSITORIES_INVALID,
        GITHUB_OWNER_REQUIRED,
        GITHUB_REPO_NAME_REQUIRED,
        GITHUB_REPOSITORY_INVALID,
        CATALOG_V3_JSON_INVALID,
        CATALOG_REDIRECT_JSON_INVALID,
        SIGNATURE_VERIFICATION_NOT_COMPILED_INTO_TARGET,
        DECOMPRESSED_SIZE_LIMIT_EXCEEDS_PLATFORM_S_MAXIMUM_BUFFER_SIZE,
        DECOMPRESSED_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES,
        PAYLOAD_TOO_LARGE_VALIDATE_DECOMPRESSED_SIZE,
        PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES,
        ZSTD_DECOMPRESSION_NOT_COMPILED_INTO_TARGET,
        BROTLI_DECOMPRESSION_NOT_COMPILED_INTO_TARGET,
        ZSTD_COMPRESSION_NOT_COMPILED_INTO_TARGET,
        DIGEST_STRING_INVALID,
        DIGEST_ALGORITHM_REQUIRED,
        DIGEST_MISMATCH,
        USES_UNSUPPORTED_DIGEST_ALGORITHM,
        MISSING_USABLE_BLAKE3_DIGEST,
        ARTIFACT_DIGEST_REQUIRED,
        CATALOG_REDIRECT_SCHEMA_UNSUPPORTED,
        CATALOG_REDIRECT_CATALOG_VERSION_OUT_OF_RANGE,
        CATALOG_REDIRECT_OUT_OF_RANGE,
        CATALOG_SCHEMA_UNSUPPORTED,
        CATALOG_VERSION_OUT_OF_RANGE,
        ID_PLUGIN_CATALOG_DUPLICATED,
        SIGNER_REPO_DOES_NOT_MATCH_SOURCE_REPO,
        COMMUNITY_SOURCE_SUPPORT_TIER_INVALID,
        COMMUNITY_SOURCE_ID_PLUGIN_CATALOG_DUPLICATED,
        RULE_PACK_ID_PLUGIN_CATALOG_DUPLICATED,
        RULE_PACK_RELEASE_VERSION_DUPLICATED,
        RULE_PACK_INVALID_MIN_SCRYER_VERSION,
        RULE_PACK_RELEASE_INVALID,
        SDK_CONSTRAINT_PLUGIN_INVALID,
        RELEASE_INVALID_MIN_SCRYER_VERSION,
        RELEASE_INVALID_MAX_SCRYER_VERSION,
        RELEASE_MIN_SCRYER_VERSION_GREATER_MAX_SCRYER_VERSION,
        RELEASE_VERSION_PLUGIN_DUPLICATED,
        RELEASE_OUT_OF_RANGE,
        RELEASE_DUPLICATE_ARTIFACT_VARIANT_ENCODING_ROWS,
        RELEASE_ARTIFACT_INVALID,
        RELEASE_ARTIFACT_BYTES_OUT_OF_RANGE,
        URL_INVALID,
        FIELD_REQUIRED,
        RELEASE_VERSION_INVALID,
        MUST_USE_SUPPORTED_ALGORITHM_HEX_DIGEST,
        DIGEST_VALUE_REQUIRED,
        DIGEST_VALUE_INVALID,
        RULE_PACK_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING,
        COMMUNITY_CATALOG_SOURCE_APPROVED_TIER_INVALID,
        COMMUNITY_CATALOG_SOURCE_MUST_PUBLISH_EXACTLY_ONE_PLUGIN,
        COMMUNITY_RULE_PACK_PUBLISH_FORBIDDEN,
        COMMUNITY_SOURCE_NESTING_FORBIDDEN,
        COMMUNITY_CATALOG_SOURCE_PUBLISHED_PLUGIN,
        COMMUNITY_CATALOG_SOURCE_SUPPORT_TIER_DOES_NOT_MATCH_APPROVED_TIER,
        COMMUNITY_CATALOG_SOURCE_SIGNER_REPO_DOES_NOT_MATCH_APPROVED_REPO,
        COMMUNITY_CATALOG_SOURCE_SOURCE_REPO_DOES_NOT_MATCH_APPROVED_REPO,
        MANUAL_PLUGIN_CATALOG_SOURCE_MISSING_GITHUB_REPO,
        CATALOG_ARTIFACTS_MISSING,
        CATALOG_REDIRECT_DID_NOT_YIELD_USABLE_CATALOG,
        INSTALLED_PACK_VERSION_INVALID,
        RULE_PACK_NOT_FOUND,
        RULE_PACK_NO_COMPATIBLE_ARTIFACT,
        RULE_PACK_USES_UNSUPPORTED_SCHEMA_VERSION,
        RULE_PACK_DOES_NOT_MATCH_CATALOG_ID_VERSION,
        RULE_PACK_EMPTY,
        RULE_PACK_CONTAINS_NO_RULES,
        RULE_PACK_CONTAINS_RULE_REQUIRED_FIELDS_MISSING,
        RULE_PACK_CONTAINS_DUPLICATE_RULE_ID,
        ARTIFACT_REDIRECTS_NOT_ALLOWED,
        ARTIFACT_REDIRECT_LIMIT_EXCEEDED,
        ARTIFACT_REDIRECT_DID_NOT_INCLUDE_LOCATION_HEADER,
        ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_HEADER,
        ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_URL,
        ARTIFACT_UNTRUSTED,
        COMPONENT_UPGRADE_BLOCKED,
    ];
}

pub mod ports {
    /// Multiple titles found for slug lookup.
    pub const MULTIPLE_TITLES_FOUND_SLUG_LOOKUP: &str = "PORTS_MULTIPLE_TITLES_FOUND_SLUG_LOOKUP";
    /// Cached JPEG encoding is unavailable.
    pub const CACHED_JPEG_ENCODING_UNAVAILABLE: &str = "PORTS_CACHED_JPEG_ENCODING_UNAVAILABLE";
    /// Indexer configuration was not found.
    pub const INDEXER_CONFIGURATION_NOT_FOUND: &str = "PORTS_INDEXER_CONFIGURATION_NOT_FOUND";
    /// Indexer settings changed during validation; reload and try again.
    pub const INDEXER_SETTINGS_CHANGED_DURING_VALIDATION: &str =
        "PORTS_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION";
    /// Structural diagnostics are unsupported by this analyzer.
    pub const STRUCTURAL_DIAGNOSTICS_UNSUPPORTED_ANALYZER: &str =
        "PORTS_STRUCTURAL_DIAGNOSTICS_UNSUPPORTED_ANALYZER";
    /// Disc title selection is unsupported by this analyzer.
    pub const DISC_TITLE_SELECTION_UNSUPPORTED_ANALYZER: &str =
        "PORTS_DISC_TITLE_SELECTION_UNSUPPORTED_ANALYZER";
    /// Select a download client.
    pub const SELECT_DOWNLOAD_CLIENT: &str = "PORTS_SELECT_DOWNLOAD_CLIENT";
    /// Category must be at most 255 bytes without control characters.
    pub const CATEGORY_OUT_OF_RANGE: &str = "PORTS_CATEGORY_OUT_OF_RANGE";
    /// Client routing discovery is unsupported.
    pub const CLIENT_ROUTING_DISCOVERY_UNSUPPORTED: &str =
        "PORTS_CLIENT_ROUTING_DISCOVERY_UNSUPPORTED";
    /// Artifact download is not supported by this client.
    pub const ARTIFACT_DOWNLOAD_UNSUPPORTED: &str = "PORTS_ARTIFACT_DOWNLOAD_UNSUPPORTED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        MULTIPLE_TITLES_FOUND_SLUG_LOOKUP,
        CACHED_JPEG_ENCODING_UNAVAILABLE,
        INDEXER_CONFIGURATION_NOT_FOUND,
        INDEXER_SETTINGS_CHANGED_DURING_VALIDATION,
        STRUCTURAL_DIAGNOSTICS_UNSUPPORTED_ANALYZER,
        DISC_TITLE_SELECTION_UNSUPPORTED_ANALYZER,
        SELECT_DOWNLOAD_CLIENT,
        CATEGORY_OUT_OF_RANGE,
        CLIENT_ROUTING_DISCOVERY_UNSUPPORTED,
        ARTIFACT_DOWNLOAD_UNSUPPORTED,
    ];
}

pub mod folder_ownership {
    /// Title folder must be an absolute path without parent traversal.
    pub const TITLE_FOLDER_INVALID: &str = "FOLDER_OWNERSHIP_TITLE_FOLDER_INVALID";
    /// Cannot resolve title folder the selected value.
    pub const CANNOT_RESOLVE_TITLE_FOLDER: &str = "FOLDER_OWNERSHIP_CANNOT_RESOLVE_TITLE_FOLDER";
    /// Title folder has no existing ancestor.
    pub const TITLE_FOLDER_NO_EXISTING_ANCESTOR: &str =
        "FOLDER_OWNERSHIP_TITLE_FOLDER_NO_EXISTING_ANCESTOR";
    /// Cannot inspect title folder the selected value.
    pub const CANNOT_INSPECT_TITLE_FOLDER: &str = "FOLDER_OWNERSHIP_CANNOT_INSPECT_TITLE_FOLDER";
    /// Cannot claim a title folder outside its library roots.
    pub const PATH_OUTSIDE_LIBRARY_ROOT: &str = "FOLDER_OWNERSHIP_PATH_OUTSIDE_LIBRARY_ROOT";
    /// The recorded folder for the selected item is a library root, not a title folder; correct the title's folder first.
    pub const FOLDER_NOT_TITLE_BOUNDARY: &str = "FOLDER_OWNERSHIP_FOLDER_NOT_TITLE_BOUNDARY";
    /// Cannot inspect folder the selected value owned by another title.
    pub const CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE: &str =
        "FOLDER_OWNERSHIP_CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE";
    /// Title folder path is required.
    pub const TITLE_FOLDER_PATH_REQUIRED: &str = "FOLDER_OWNERSHIP_TITLE_FOLDER_PATH_REQUIRED";
    /// Cannot compare folder the selected value with folder the selected value owned by title the selected value.
    pub const CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE: &str =
        "FOLDER_OWNERSHIP_CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE";
    /// Folder the selected value overlaps the folder owned by title the selected value.
    pub const FOLDER_OVERLAPS_FOLDER_OWNED_TITLE: &str =
        "FOLDER_OWNERSHIP_FOLDER_OVERLAPS_FOLDER_OWNED_TITLE";
    /// Recycle restore path is outside the title's current owned folder; resolve the folder change before restoring.
    pub const RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER: &str =
        "FOLDER_OWNERSHIP_RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER";
    /// Recycle restore destination overlaps another title's owned folder; recycled files were preserved.
    pub const RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER: &str =
        "FOLDER_OWNERSHIP_RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER";
    /// Folder the selected value is already owned by title the selected value.
    pub const FOLDER_ALREADY_OWNED_TITLE: &str = "FOLDER_OWNERSHIP_FOLDER_ALREADY_OWNED_TITLE";
    /// Title the selected value already owns another folder.
    pub const TITLE_ALREADY_OWNS_ANOTHER_FOLDER: &str =
        "FOLDER_OWNERSHIP_TITLE_ALREADY_OWNS_ANOTHER_FOLDER";
    /// Cannot claim a library root as a title folder.
    pub const CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER: &str =
        "FOLDER_OWNERSHIP_CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        TITLE_FOLDER_INVALID,
        CANNOT_RESOLVE_TITLE_FOLDER,
        TITLE_FOLDER_NO_EXISTING_ANCESTOR,
        CANNOT_INSPECT_TITLE_FOLDER,
        PATH_OUTSIDE_LIBRARY_ROOT,
        FOLDER_NOT_TITLE_BOUNDARY,
        CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE,
        TITLE_FOLDER_PATH_REQUIRED,
        CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE,
        FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
        RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER,
        RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER,
        FOLDER_ALREADY_OWNED_TITLE,
        TITLE_ALREADY_OWNS_ANOTHER_FOLDER,
        CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER,
    ];
}

pub mod indexer_category {
    /// The indexer category does not match the release subject.
    pub const MISMATCH: &str = "INDEXER_CATEGORY_MISMATCH";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[MISMATCH];
}

pub mod download_client_path_mappings {
    /// Remote path mappings line the selected value must use REMOTE => LOCAL format.
    pub const REMOTE_PATH_MAPPINGS_LINE_INVALID: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_INVALID";
    /// Remote path mappings line the selected value requires a local path.
    pub const REMOTE_PATH_MAPPINGS_LINE_REQUIRES_LOCAL_PATH: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REQUIRES_LOCAL_PATH";
    /// Remote path mappings line the selected value local path must be absolute.
    pub const REMOTE_PATH_MAPPINGS_LINE_LOCAL_PATH_INVALID: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_LOCAL_PATH_INVALID";
    /// Remote path mappings line the selected value duplicates an existing remote path root.
    pub const REMOTE_PATH_MAPPINGS_LINE_DUPLICATES_EXISTING_REMOTE_PATH_ROOT: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_DUPLICATES_EXISTING_REMOTE_PATH_ROOT";
    /// Invalid download client config JSON.
    pub const DOWNLOAD_CLIENT_CONFIG_JSON_INVALID: &str =
        "DOWNLOAD_PATH_MAPPING_DOWNLOAD_CLIENT_CONFIG_JSON_INVALID";
    /// Remote_path_mappings must be a string.
    pub const REMOTE_PATH_MAPPINGS_INVALID: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_INVALID";
    /// Remote path mappings line the selected value requires a remote path.
    pub const REMOTE_PATH_MAPPINGS_LINE_REQUIRES_REMOTE_PATH: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REQUIRES_REMOTE_PATH";
    /// Remote path mappings line the selected value remote path cannot start with whitespace.
    pub const REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_START_WHITESPACE: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_START_WHITESPACE";
    /// Remote path mappings line the selected value remote path cannot end with whitespace.
    pub const REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_END_WHITESPACE: &str =
        "DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_END_WHITESPACE";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        REMOTE_PATH_MAPPINGS_LINE_INVALID,
        REMOTE_PATH_MAPPINGS_LINE_REQUIRES_LOCAL_PATH,
        REMOTE_PATH_MAPPINGS_LINE_LOCAL_PATH_INVALID,
        REMOTE_PATH_MAPPINGS_LINE_DUPLICATES_EXISTING_REMOTE_PATH_ROOT,
        DOWNLOAD_CLIENT_CONFIG_JSON_INVALID,
        REMOTE_PATH_MAPPINGS_INVALID,
        REMOTE_PATH_MAPPINGS_LINE_REQUIRES_REMOTE_PATH,
        REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_START_WHITESPACE,
        REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_END_WHITESPACE,
    ];
}

pub mod media_requests {
    /// External ids map to multiple titles.
    pub const EXTERNAL_IDS_MAP_MULTIPLE_TITLES: &str =
        "MEDIA_REQUEST_EXTERNAL_IDS_MAP_MULTIPLE_TITLES";
    /// Title is being modified; retry approval.
    pub const TITLE_BEING_MODIFIED: &str = "MEDIA_REQUEST_TITLE_BEING_MODIFIED";
    /// Title changed during approval; retry.
    pub const TITLE_CHANGED_DURING_APPROVAL: &str = "MEDIA_REQUEST_TITLE_CHANGED_DURING_APPROVAL";
    /// Request title is required.
    pub const REQUEST_TITLE_REQUIRED: &str = "MEDIA_REQUEST_REQUEST_TITLE_REQUIRED";
    /// Media requests must include a searchable SMG identifier.
    pub const MEDIA_REQUESTS_INVALID: &str = "MEDIA_REQUEST_MEDIA_REQUESTS_INVALID";
    /// Library was not found.
    pub const LIBRARY_NOT_FOUND: &str = "MEDIA_REQUEST_LIBRARY_NOT_FOUND";
    /// Library facet does not match requested media facet.
    pub const LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET: &str =
        "MEDIA_REQUEST_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET";
    /// A list movie approval must retain its selected movie.
    pub const LIST_MOVIE_APPROVAL_MUST_RETAIN_SELECTED_MOVIE: &str =
        "MEDIA_REQUEST_LIST_MOVIE_APPROVAL_MUST_RETAIN_SELECTED_MOVIE";
    /// Media request id is required.
    pub const ID_REQUIRED: &str = "MEDIA_REQUEST_ID_REQUIRED";
    /// Media request was not found.
    pub const RESOURCE_NOT_FOUND: &str = "MEDIA_REQUEST_RESOURCE_NOT_FOUND";
    /// Only a dismissed media request can be reopened.
    pub const ONLY_DISMISSED_MEDIA_REQUEST_REOPENED: &str =
        "MEDIA_REQUEST_ONLY_DISMISSED_MEDIA_REQUEST_REOPENED";
    /// A list movie request must retain its selected movie.
    pub const LIST_MOVIE_REQUEST_MUST_RETAIN_SELECTED_MOVIE: &str =
        "MEDIA_REQUEST_LIST_MOVIE_REQUEST_MUST_RETAIN_SELECTED_MOVIE";
    /// Approved quality profile is required.
    pub const APPROVED_QUALITY_PROFILE_REQUIRED: &str =
        "MEDIA_REQUEST_APPROVED_QUALITY_PROFILE_REQUIRED";
    /// Approved quality profile name is required.
    pub const APPROVED_QUALITY_PROFILE_NAME_REQUIRED: &str =
        "MEDIA_REQUEST_APPROVED_QUALITY_PROFILE_NAME_REQUIRED";
    /// Title was not found.
    pub const TITLE_NOT_FOUND: &str = "MEDIA_REQUEST_TITLE_NOT_FOUND";
    /// Claim extension must lengthen a finite lease into the future.
    pub const CLAIM_EXTENSION_MUST_LENGTHEN_FINITE_LEASE_INTO_FUTURE: &str =
        "MEDIA_REQUEST_CLAIM_EXTENSION_MUST_LENGTHEN_FINITE_LEASE_INTO_FUTURE";
    /// A release reason is required.
    pub const RELEASE_REASON_REQUIRED: &str = "MEDIA_REQUEST_RELEASE_REASON_REQUIRED";
    /// Lifecycle claim was not found.
    pub const LIFECYCLE_CLAIM_NOT_FOUND: &str = "MEDIA_REQUEST_LIFECYCLE_CLAIM_NOT_FOUND";
    /// Media request is no longer pending.
    pub const NO_LONGER_PENDING: &str = "MEDIA_REQUEST_NO_LONGER_PENDING";
    /// You do not own this media request.
    pub const DO_NOT_OWN_MEDIA_REQUEST: &str = "MEDIA_REQUEST_DO_NOT_OWN_MEDIA_REQUEST";
    /// Title already exists in the target library.
    pub const TITLE_ALREADY_EXISTS_TARGET_LIBRARY: &str =
        "MEDIA_REQUEST_TITLE_ALREADY_EXISTS_TARGET_LIBRARY";
    /// Requested quality profile is not allowed for this library.
    pub const REQUESTED_QUALITY_PROFILE_NOT_ALLOWED_LIBRARY: &str =
        "MEDIA_REQUEST_REQUESTED_QUALITY_PROFILE_NOT_ALLOWED_LIBRARY";
    /// Quality profile id is required.
    pub const QUALITY_PROFILE_ID_REQUIRED: &str = "MEDIA_REQUEST_QUALITY_PROFILE_ID_REQUIRED";
    /// Unknown quality profile the selected value.
    pub const QUALITY_PROFILE_UNKNOWN: &str = "MEDIA_REQUEST_QUALITY_PROFILE_UNKNOWN";
    /// Movie metadata response missing title was not found.
    pub const MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND: &str =
        "MEDIA_REQUEST_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND";
    /// Advanced monitoring is only valid for series and anime titles.
    pub const ADVANCED_MONITORING_ONLY_VALID_SERIES_ANIME_TITLES: &str =
        "MEDIA_REQUEST_ADVANCED_MONITORING_ONLY_VALID_SERIES_ANIME_TITLES";
    /// Unsupported request monitor type the selected value.
    pub const REQUEST_MONITOR_TYPE_UNSUPPORTED: &str =
        "MEDIA_REQUEST_REQUEST_MONITOR_TYPE_UNSUPPORTED";
    /// Advanced monitoring requires at least one season or series movie.
    pub const MONITORING_TARGET_REQUIRED: &str = "MEDIA_REQUEST_MONITORING_TARGET_REQUIRED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        EXTERNAL_IDS_MAP_MULTIPLE_TITLES,
        TITLE_BEING_MODIFIED,
        TITLE_CHANGED_DURING_APPROVAL,
        REQUEST_TITLE_REQUIRED,
        MEDIA_REQUESTS_INVALID,
        LIBRARY_NOT_FOUND,
        LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET,
        LIST_MOVIE_APPROVAL_MUST_RETAIN_SELECTED_MOVIE,
        ID_REQUIRED,
        RESOURCE_NOT_FOUND,
        ONLY_DISMISSED_MEDIA_REQUEST_REOPENED,
        LIST_MOVIE_REQUEST_MUST_RETAIN_SELECTED_MOVIE,
        APPROVED_QUALITY_PROFILE_REQUIRED,
        APPROVED_QUALITY_PROFILE_NAME_REQUIRED,
        TITLE_NOT_FOUND,
        CLAIM_EXTENSION_MUST_LENGTHEN_FINITE_LEASE_INTO_FUTURE,
        RELEASE_REASON_REQUIRED,
        LIFECYCLE_CLAIM_NOT_FOUND,
        NO_LONGER_PENDING,
        DO_NOT_OWN_MEDIA_REQUEST,
        TITLE_ALREADY_EXISTS_TARGET_LIBRARY,
        REQUESTED_QUALITY_PROFILE_NOT_ALLOWED_LIBRARY,
        QUALITY_PROFILE_ID_REQUIRED,
        QUALITY_PROFILE_UNKNOWN,
        MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND,
        ADVANCED_MONITORING_ONLY_VALID_SERIES_ANIME_TITLES,
        REQUEST_MONITOR_TYPE_UNSUPPORTED,
        MONITORING_TARGET_REQUIRED,
    ];
}

pub mod api_keys {
    /// Environment-seeded API keys are managed by SCRYER_DEV_API_KEYS.
    pub const ENVIRONMENT_SEEDED_API_KEYS_MANAGED_SCRYER_DEV_API_KEYS: &str =
        "API_KEY_ENVIRONMENT_SEEDED_API_KEYS_MANAGED_SCRYER_DEV_API_KEYS";
    /// Invalid API key.
    pub const VALUE_INVALID: &str = "API_KEY_VALUE_INVALID";
    /// API key is expired or revoked.
    pub const EXPIRED_REVOKED: &str = "API_KEY_EXPIRED_REVOKED";
    /// API key owner no longer exists.
    pub const OWNER_NO_LONGER_EXISTS: &str = "API_KEY_OWNER_NO_LONGER_EXISTS";
    /// API key owner is disabled.
    pub const OWNER_DISABLED: &str = "API_KEY_OWNER_DISABLED";
    /// Invalid development API key declaration.
    pub const DEVELOPMENT_API_KEY_DECLARATION_INVALID: &str =
        "API_KEY_DEVELOPMENT_API_KEY_DECLARATION_INVALID";
    /// Development API key declarations must have unique lookup IDs.
    pub const DEVELOPMENT_API_KEY_DECLARATIONS_INVALID: &str =
        "API_KEY_DEVELOPMENT_API_KEY_DECLARATIONS_INVALID";
    /// Development API key owner does not exist.
    pub const DEVELOPMENT_API_KEY_OWNER_DOES_NOT_EXIST: &str =
        "API_KEY_DEVELOPMENT_API_KEY_OWNER_DOES_NOT_EXIST";
    /// Development API key lookup ID collides with a user-managed key.
    pub const DEVELOPMENT_API_KEY_LOOKUP_ID_COLLIDES_USER_MANAGED_KEY: &str =
        "API_KEY_DEVELOPMENT_API_KEY_LOOKUP_ID_COLLIDES_USER_MANAGED_KEY";
    /// Development API key lookup ID has a different secret.
    pub const DEVELOPMENT_API_KEY_LOOKUP_ID_DIFFERENT_SECRET: &str =
        "API_KEY_DEVELOPMENT_API_KEY_LOOKUP_ID_DIFFERENT_SECRET";
    /// API keys are restricted to users who manage system settings.
    pub const API_KEYS_RESTRICTED_USERS_WHO_MANAGE_SYSTEM_SETTINGS: &str =
        "API_KEY_API_KEYS_RESTRICTED_USERS_WHO_MANAGE_SYSTEM_SETTINGS";
    /// API key label is required.
    pub const LABEL_REQUIRED: &str = "API_KEY_LABEL_REQUIRED";
    /// API key label must not exceed the selected value characters.
    pub const LABEL_TOO_LONG: &str = "API_KEY_LABEL_TOO_LONG";
    /// API key label must not contain control characters.
    pub const LABEL_CONTROL_CHARACTER: &str = "API_KEY_LABEL_CONTROL_CHARACTER";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        ENVIRONMENT_SEEDED_API_KEYS_MANAGED_SCRYER_DEV_API_KEYS,
        VALUE_INVALID,
        EXPIRED_REVOKED,
        OWNER_NO_LONGER_EXISTS,
        OWNER_DISABLED,
        DEVELOPMENT_API_KEY_DECLARATION_INVALID,
        DEVELOPMENT_API_KEY_DECLARATIONS_INVALID,
        DEVELOPMENT_API_KEY_OWNER_DOES_NOT_EXIST,
        DEVELOPMENT_API_KEY_LOOKUP_ID_COLLIDES_USER_MANAGED_KEY,
        DEVELOPMENT_API_KEY_LOOKUP_ID_DIFFERENT_SECRET,
        API_KEYS_RESTRICTED_USERS_WHO_MANAGE_SYSTEM_SETTINGS,
        LABEL_REQUIRED,
        LABEL_TOO_LONG,
        LABEL_CONTROL_CHARACTER,
    ];
}

pub mod fs_safety {
    /// Configured media root the selected value is unavailable.
    pub const CONFIGURED_MEDIA_ROOT_UNAVAILABLE: &str =
        "FS_SAFETY_CONFIGURED_MEDIA_ROOT_UNAVAILABLE";
    /// Configured media root the selected value is a symlink.
    pub const CONFIGURED_MEDIA_ROOT_SYMLINK: &str = "FS_SAFETY_CONFIGURED_MEDIA_ROOT_SYMLINK";
    /// Configured media root the selected value is not a directory.
    pub const CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY: &str =
        "FS_SAFETY_CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY";
    /// Configured media root the selected value is unreadable.
    pub const CONFIGURED_MEDIA_ROOT_UNREADABLE: &str = "FS_SAFETY_CONFIGURED_MEDIA_ROOT_UNREADABLE";
    /// Configured media root the selected value is empty.
    pub const CONFIGURED_MEDIA_ROOT_EMPTY: &str = "FS_SAFETY_CONFIGURED_MEDIA_ROOT_EMPTY";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        CONFIGURED_MEDIA_ROOT_UNAVAILABLE,
        CONFIGURED_MEDIA_ROOT_SYMLINK,
        CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY,
        CONFIGURED_MEDIA_ROOT_UNREADABLE,
        CONFIGURED_MEDIA_ROOT_EMPTY,
    ];
}

/// Why the folder picker could not create a new folder. Every way the
/// operating system can refuse lands on one of these, so a client never sees
/// raw OS error text.
pub mod folder_create {
    /// No name was given, or only spaces.
    pub const NAME_REQUIRED: &str = "FOLDER_CREATE_NAME_REQUIRED";
    /// The name is `.`, `..` or a device name Windows reserves (CON, NUL, COM1 and the like).
    pub const NAME_RESERVED: &str = "FOLDER_CREATE_NAME_RESERVED";
    /// The name holds a path separator, a character Windows, SMB or FAT/exFAT
    /// drives reject, or a control character.
    pub const NAME_INVALID_CHARACTERS: &str = "FOLDER_CREATE_NAME_INVALID_CHARACTERS";
    /// The name ends with a dot, which Windows and SMB shares silently drop.
    pub const NAME_TRAILING_DOT: &str = "FOLDER_CREATE_NAME_TRAILING_DOT";
    /// The name starts with a dot, so the folder would be hidden from the picker.
    pub const NAME_HIDDEN: &str = "FOLDER_CREATE_NAME_HIDDEN";
    /// The name, or the path it makes, is longer than the drive allows.
    pub const NAME_TOO_LONG: &str = "FOLDER_CREATE_NAME_TOO_LONG";
    /// The drive's file system refused the name for its own rules.
    pub const NAME_NOT_ALLOWED: &str = "FOLDER_CREATE_NAME_NOT_ALLOWED";
    /// The folder to create in is not an absolute path.
    pub const PARENT_NOT_ABSOLUTE: &str = "FOLDER_CREATE_PARENT_NOT_ABSOLUTE";
    /// The folder to create in no longer exists or cannot be reached.
    pub const PARENT_MISSING: &str = "FOLDER_CREATE_PARENT_MISSING";
    /// The path to create in is a file, not a folder.
    pub const PARENT_NOT_DIRECTORY: &str = "FOLDER_CREATE_PARENT_NOT_DIRECTORY";
    /// A file or folder with that name is already there.
    pub const ALREADY_EXISTS: &str = "FOLDER_CREATE_ALREADY_EXISTS";
    /// The user Scryer runs as may not write in that folder.
    pub const PERMISSION_DENIED: &str = "FOLDER_CREATE_PERMISSION_DENIED";
    /// The drive or mount is read-only.
    pub const READ_ONLY: &str = "FOLDER_CREATE_READ_ONLY";
    /// The drive is full, or a storage quota is used up.
    pub const DISK_FULL: &str = "FOLDER_CREATE_DISK_FULL";
    /// The folder already holds as many subfolders as its file system allows.
    pub const FOLDER_FULL: &str = "FOLDER_CREATE_FOLDER_FULL";
    /// The location does not support creating folders.
    pub const NOT_SUPPORTED: &str = "FOLDER_CREATE_NOT_SUPPORTED";
    /// The drive or network share is disconnected, not responding or failing.
    pub const DRIVE_UNAVAILABLE: &str = "FOLDER_CREATE_DRIVE_UNAVAILABLE";
    /// The folder could not be created for a reason none of the others name.
    pub const FAILED: &str = "FOLDER_CREATE_FAILED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        NAME_REQUIRED,
        NAME_RESERVED,
        NAME_INVALID_CHARACTERS,
        NAME_TRAILING_DOT,
        NAME_HIDDEN,
        NAME_TOO_LONG,
        NAME_NOT_ALLOWED,
        PARENT_NOT_ABSOLUTE,
        PARENT_MISSING,
        PARENT_NOT_DIRECTORY,
        ALREADY_EXISTS,
        PERMISSION_DENIED,
        READ_ONLY,
        DISK_FULL,
        FOLDER_FULL,
        NOT_SUPPORTED,
        DRIVE_UNAVAILABLE,
        FAILED,
    ];

    #[cfg(test)]
    mod tests {
        #[test]
        fn reasons_are_distinct_prefixed_and_screaming_snake_case() {
            let mut unique = std::collections::HashSet::new();
            for reason in super::ALL {
                assert!(reason.starts_with("FOLDER_CREATE_"));
                assert!(
                    reason
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                );
                assert!(unique.insert(reason));
            }
        }
    }
}

pub mod authorization {
    /// You do not have permission to perform this action.
    pub const ACTION_PERMISSION_DENIED: &str = "AUTHORIZATION_ACTION_PERMISSION_DENIED";
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "AUTHORIZATION_LIBRARY_ACCESS_DENIED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[ACTION_PERMISSION_DENIED, LIBRARY_ACCESS_DENIED];
}

pub mod download_client_config {
    /// Invalid download client config JSON.
    pub const JSON_INVALID: &str = "DOWNLOAD_CLIENT_CONFIG_JSON_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[JSON_INVALID];
}

pub mod settings {
    /// Catalog view columns must match the saved device class and facet.
    pub const CATALOG_VIEW_COLUMNS_INVALID: &str = "SETTINGS_CATALOG_VIEW_COLUMNS_INVALID";
    /// A dashboard layout's column count is outside the supported range.
    pub const DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE: &str =
        "SETTINGS_DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE";
    /// A dashboard layout has more panels than one layout may hold.
    pub const DASHBOARD_LAYOUT_TOO_MANY_ITEMS: &str = "SETTINGS_DASHBOARD_LAYOUT_TOO_MANY_ITEMS";
    /// A dashboard panel id is empty, too long, or uses characters other than lowercase letters, digits and hyphens.
    pub const DASHBOARD_PANEL_ID_INVALID: &str = "SETTINGS_DASHBOARD_PANEL_ID_INVALID";
    /// A dashboard layout places the same panel more than once.
    pub const DASHBOARD_PANEL_ID_DUPLICATED: &str = "SETTINGS_DASHBOARD_PANEL_ID_DUPLICATED";
    /// A dashboard panel starts at a negative row or column.
    pub const DASHBOARD_ITEM_POSITION_OUT_OF_RANGE: &str =
        "SETTINGS_DASHBOARD_ITEM_POSITION_OUT_OF_RANGE";
    /// A dashboard panel is less than one cell wide or tall.
    pub const DASHBOARD_ITEM_SIZE_OUT_OF_RANGE: &str = "SETTINGS_DASHBOARD_ITEM_SIZE_OUT_OF_RANGE";
    /// A dashboard panel extends past the layout's last column.
    pub const DASHBOARD_ITEM_EXCEEDS_COLUMNS: &str = "SETTINGS_DASHBOARD_ITEM_EXCEEDS_COLUMNS";
    /// Language must be a language code of at most the selected value letters, digits or hyphens.
    pub const LANGUAGE_OUT_OF_RANGE: &str = "SETTINGS_LANGUAGE_OUT_OF_RANGE";
    /// The requested value must be a #RRGGBB hex color.
    pub const MUST_RRGGBB_HEX_COLOR: &str = "SETTINGS_MUST_RRGGBB_HEX_COLOR";
    /// Duplicate UI table column setting for the selected value the selected value the selected value column the selected value.
    pub const UI_TABLE_COLUMN_SETTING_COLUMN_DUPLICATED: &str =
        "SETTINGS_UI_TABLE_COLUMN_SETTING_COLUMN_DUPLICATED";
    /// Table column order must be greater than or equal to 0.
    pub const TABLE_COLUMN_ORDER_OUT_OF_RANGE: &str = "SETTINGS_TABLE_COLUMN_ORDER_OUT_OF_RANGE";
    /// Unsupported the selected value table column the selected value.
    pub const TABLE_COLUMN_UNSUPPORTED: &str = "SETTINGS_TABLE_COLUMN_UNSUPPORTED";
    /// Movies table settings cannot include the episodes column.
    pub const MOVIES_TABLE_SETTINGS_CANNOT_INCLUDE_EPISODES_COLUMN: &str =
        "SETTINGS_MOVIES_TABLE_SETTINGS_CANNOT_INCLUDE_EPISODES_COLUMN";
    /// Title image cache refresh is already running.
    pub const TITLE_IMAGE_CACHE_REFRESH_ALREADY_RUNNING: &str =
        "SETTINGS_TITLE_IMAGE_CACHE_REFRESH_ALREADY_RUNNING";
    /// Session duration must be between 1 and 365 days.
    pub const SESSION_DURATION_OUT_OF_RANGE: &str = "SETTINGS_SESSION_DURATION_OUT_OF_RANGE";
    /// Password minimum length must be at least the selected value.
    pub const PASSWORD_MINIMUM_LENGTH_OUT_OF_RANGE: &str =
        "SETTINGS_PASSWORD_MINIMUM_LENGTH_OUT_OF_RANGE";
    /// Change the default admin password before enabling form login.
    pub const CHANGE_DEFAULT_ADMIN_PASSWORD_BEFORE_ENABLING_FORM_LOGIN: &str =
        "SETTINGS_CHANGE_DEFAULT_ADMIN_PASSWORD_BEFORE_ENABLING_FORM_LOGIN";
    /// Configure an enabled full administrator login before enabling form login.
    pub const CONFIGURE_ENABLED_FULL_ADMINISTRATOR_LOGIN_BEFORE_ENABLING_FORM_LOGIN: &str =
        "SETTINGS_CONFIGURE_ENABLED_FULL_ADMINISTRATOR_LOGIN_BEFORE_ENABLING_FORM_LOGIN";
    /// Form login cannot be turned off while SCRYER_DISABLE_DEFAULT_ADMIN is true, because running without login uses the default admin account; remove that setting and restart first.
    pub const FORM_LOGIN_REQUIRED_DEFAULT_ADMIN_DISABLED: &str =
        "SETTINGS_FORM_LOGIN_REQUIRED_DEFAULT_ADMIN_DISABLED";
    /// Cannot save and reset trusted proxies together.
    pub const CANNOT_SAVE_RESET_TRUSTED_PROXIES_TOGETHER: &str =
        "SETTINGS_CANNOT_SAVE_RESET_TRUSTED_PROXIES_TOGETHER";
    /// Default movie availability must be announced, in_cinemas, or released.
    pub const DEFAULT_MOVIE_AVAILABILITY_INVALID: &str =
        "SETTINGS_DEFAULT_MOVIE_AVAILABILITY_INVALID";
    /// Movie release market must be a two-letter ISO-3166 country code.
    pub const MOVIE_RELEASE_MARKET_INVALID: &str = "SETTINGS_MOVIE_RELEASE_MARKET_INVALID";
    /// Root folder path is required.
    pub const ROOT_FOLDER_PATH_REQUIRED: &str = "SETTINGS_ROOT_FOLDER_PATH_REQUIRED";
    /// At least one root folder is required.
    pub const ROOT_REQUIRED: &str = "SETTINGS_ROOT_REQUIRED";
    /// The requested value must be an absolute path or a command name without path separators.
    pub const MUST_ABSOLUTE_PATH_COMMAND_NAME_WITHOUT_PATH_SEPARATORS: &str =
        "SETTINGS_MUST_ABSOLUTE_PATH_COMMAND_NAME_WITHOUT_PATH_SEPARATORS";
    /// You do not have permission to view verification settings.
    pub const VIEW_VERIFICATION_SETTINGS_PERMISSION_DENIED: &str =
        "SETTINGS_VIEW_VERIFICATION_SETTINGS_PERMISSION_DENIED";
    /// Subtitle search interval must be at least 1 hour.
    pub const SUBTITLE_SEARCH_INTERVAL_OUT_OF_RANGE: &str =
        "SETTINGS_SUBTITLE_SEARCH_INTERVAL_OUT_OF_RANGE";
    /// Subtitle minimum score percentages must be between 0 and 100.
    pub const SUBTITLE_MINIMUM_SCORE_PERCENTAGES_OUT_OF_RANGE: &str =
        "SETTINGS_SUBTITLE_MINIMUM_SCORE_PERCENTAGES_OUT_OF_RANGE";
    /// Subtitle sync threshold percentages must be between 0 and 100.
    pub const SUBTITLE_SYNC_THRESHOLD_PERCENTAGES_OUT_OF_RANGE: &str =
        "SETTINGS_SUBTITLE_SYNC_THRESHOLD_PERCENTAGES_OUT_OF_RANGE";
    /// Subtitle sync max offset cannot be negative.
    pub const SUBTITLE_SYNC_MAX_OFFSET_CANNOT_NEGATIVE: &str =
        "SETTINGS_SUBTITLE_SYNC_MAX_OFFSET_CANNOT_NEGATIVE";
    /// Invalid the selected value setting value the selected item: expected a 3 or 4 digit octal mask.
    pub const SETTING_VALUE_INVALID: &str = "SETTINGS_SETTING_VALUE_INVALID";
    /// Invalid permissions.chown_group setting value the selected item: numeric gid is outside range.
    pub const PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID: &str =
        "SETTINGS_PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID";
    /// Season folder templates are only supported for series and anime.
    pub const SEASON_FOLDER_TEMPLATES_ONLY_SUPPORTED_SERIES_ANIME: &str =
        "SETTINGS_SEASON_FOLDER_TEMPLATES_ONLY_SUPPORTED_SERIES_ANIME";
    /// Movie availability settings require scope movie.
    pub const MOVIE_AVAILABILITY_SETTINGS_REQUIRE_SCOPE_MOVIE: &str =
        "SETTINGS_MOVIE_AVAILABILITY_SETTINGS_REQUIRE_SCOPE_MOVIE";
    /// Plexmatch_write_on_import is only valid for series and anime.
    pub const PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME: &str =
        "SETTINGS_PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME";
    /// Anime-specific settings require scope anime.
    pub const ANIME_SPECIFIC_SETTINGS_REQUIRE_SCOPE_ANIME: &str =
        "SETTINGS_ANIME_SPECIFIC_SETTINGS_REQUIRE_SCOPE_ANIME";
    /// At least one media setting change is required.
    pub const MEDIA_SETTING_CHANGE_REQUIRED: &str = "SETTINGS_MEDIA_SETTING_CHANGE_REQUIRED";
    /// Automatic backup key must be at least the selected value non-whitespace characters.
    pub const AUTOMATIC_BACKUP_KEY_OUT_OF_RANGE: &str =
        "SETTINGS_AUTOMATIC_BACKUP_KEY_OUT_OF_RANGE";
    /// Automatic backup key cannot be replaced and cleared in the same request.
    pub const AUTOMATIC_BACKUP_KEY_CANNOT_REPLACED_CLEARED_SAME_REQUEST: &str =
        "SETTINGS_AUTOMATIC_BACKUP_KEY_CANNOT_REPLACED_CLEARED_SAME_REQUEST";
    /// Automatic backup key cannot be cleared while automatic backups are enabled.
    pub const AUTOMATIC_BACKUP_KEY_CANNOT_CLEARED_WHILE_AUTOMATIC_BACKUPS_DISABLED: &str =
        "SETTINGS_AUTOMATIC_BACKUP_KEY_CANNOT_CLEARED_WHILE_AUTOMATIC_BACKUPS_DISABLED";
    /// Automatic backups require a backup key.
    pub const AUTOMATIC_BACKUPS_REQUIRE_BACKUP_KEY: &str =
        "SETTINGS_AUTOMATIC_BACKUPS_REQUIRE_BACKUP_KEY";
    /// Trusted certificate bundle must contain PEM-encoded X.509 certificates.
    pub const TRUSTED_CERTIFICATE_BUNDLE_INVALID: &str =
        "SETTINGS_TRUSTED_CERTIFICATE_BUNDLE_INVALID";
    /// Trusted certificate bundle may only contain X.509 certificate PEM blocks.
    pub const TRUSTED_CERTIFICATE_BUNDLE_MAY_ONLY_CONTAIN_X_509_CERTIFICATE_PEM_BLOCKS: &str =
        "SETTINGS_TRUSTED_CERTIFICATE_BUNDLE_MAY_ONLY_CONTAIN_X_509_CERTIFICATE_PEM_BLOCKS";
    /// Trusted certificate bundle did not contain a readable X.509 certificate.
    pub const CERTIFICATE_BUNDLE_EMPTY: &str = "SETTINGS_CERTIFICATE_BUNDLE_EMPTY";
    /// Failed to parse trusted certificate PEM block.
    pub const FAILED_PARSE_TRUSTED_CERTIFICATE_PEM_BLOCK: &str =
        "SETTINGS_FAILED_PARSE_TRUSTED_CERTIFICATE_PEM_BLOCK";
    /// Failed to parse trailing PEM content for trusted certificate.
    pub const FAILED_PARSE_TRAILING_PEM_CONTENT_TRUSTED_CERTIFICATE: &str =
        "SETTINGS_FAILED_PARSE_TRAILING_PEM_CONTENT_TRUSTED_CERTIFICATE";
    /// Each trusted certificate entry must contain exactly one X.509 certificate.
    pub const EACH_TRUSTED_CERTIFICATE_ENTRY_INVALID: &str =
        "SETTINGS_EACH_TRUSTED_CERTIFICATE_ENTRY_INVALID";
    /// Backup path must be a directory.
    pub const BACKUP_PATH_INVALID: &str = "SETTINGS_BACKUP_PATH_INVALID";
    /// Failed to create backup directory.
    pub const FAILED_CREATE_BACKUP_DIRECTORY: &str = "SETTINGS_FAILED_CREATE_BACKUP_DIRECTORY";
    /// Backup path is not writable.
    pub const BACKUP_PATH_NOT_WRITABLE: &str = "SETTINGS_BACKUP_PATH_NOT_WRITABLE";
    /// Failed to remove backup path probe.
    pub const FAILED_REMOVE_BACKUP_PATH_PROBE: &str = "SETTINGS_FAILED_REMOVE_BACKUP_PATH_PROBE";
    /// History retention days must be at least 1.
    pub const HISTORY_RETENTION_DAYS_OUT_OF_RANGE: &str =
        "SETTINGS_HISTORY_RETENTION_DAYS_OUT_OF_RANGE";
    /// Image cache maximum size must be at least 1 MiB.
    pub const IMAGE_CACHE_MAXIMUM_SIZE_OUT_OF_RANGE: &str =
        "SETTINGS_IMAGE_CACHE_MAXIMUM_SIZE_OUT_OF_RANGE";
    /// Acquisition thresholds cannot be negative.
    pub const ACQUISITION_THRESHOLDS_CANNOT_NEGATIVE: &str =
        "SETTINGS_ACQUISITION_THRESHOLDS_CANNOT_NEGATIVE";
    /// Acquisition poll interval must be at least 1 second.
    pub const ACQUISITION_POLL_INTERVAL_OUT_OF_RANGE: &str =
        "SETTINGS_ACQUISITION_POLL_INTERVAL_OUT_OF_RANGE";
    /// Acquisition walk interval must be at least 1 second.
    pub const ACQUISITION_WALK_INTERVAL_OUT_OF_RANGE: &str =
        "SETTINGS_ACQUISITION_WALK_INTERVAL_OUT_OF_RANGE";
    /// Convergence per-cycle scope ceiling must be at least 1.
    pub const CONVERGENCE_PER_CYCLE_SCOPE_CEILING_OUT_OF_RANGE: &str =
        "SETTINGS_CONVERGENCE_PER_CYCLE_SCOPE_CEILING_OUT_OF_RANGE";
    /// Re-converge backstop cannot be negative.
    pub const RE_CONVERGE_BACKSTOP_CANNOT_NEGATIVE: &str =
        "SETTINGS_RE_CONVERGE_BACKSTOP_CANNOT_NEGATIVE";
    /// Quality profile id the selected item is ambiguous because profile ids differ only by ASCII case.
    pub const QUALITY_PROFILE_ID_AMBIGUOUS_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE: &str =
        "SETTINGS_QUALITY_PROFILE_ID_AMBIGUOUS_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE";
    /// Quality profile id is required.
    pub const QUALITY_PROFILE_ID_REQUIRED: &str = "SETTINGS_QUALITY_PROFILE_ID_REQUIRED";
    /// Quality profile ids the selected item and the selected item differ only by ASCII case.
    pub const QUALITY_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE: &str =
        "SETTINGS_QUALITY_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE";
    /// Delay profile the selected item names an unknown library the selected item.
    pub const DELAY_PROFILE_NAMES_UNKNOWN_LIBRARY: &str =
        "SETTINGS_DELAY_PROFILE_NAMES_UNKNOWN_LIBRARY";
    /// You do not have permission to view quality profiles.
    pub const VIEW_QUALITY_PROFILES_PERMISSION_DENIED: &str =
        "SETTINGS_VIEW_QUALITY_PROFILES_PERMISSION_DENIED";
    /// Unknown quality profile the selected item.
    pub const QUALITY_PROFILE_UNKNOWN: &str = "SETTINGS_QUALITY_PROFILE_UNKNOWN";
    /// Cannot remove quality profile the selected item because it is the global default.
    pub const CANNOT_REMOVE_QUALITY_PROFILE_GLOBAL_DEFAULT: &str =
        "SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_GLOBAL_DEFAULT";
    /// Cannot remove quality profile the selected item because it is configured for the selected value.
    pub const CANNOT_REMOVE_QUALITY_PROFILE_NOT_CONFIGURED: &str =
        "SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_NOT_CONFIGURED";
    /// Cannot remove quality profile the selected item because library the selected item references it.
    pub const CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_REFERENCES: &str =
        "SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_REFERENCES";
    /// Cannot remove quality profile the selected item because library the selected item allows it for requests.
    pub const CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_ALLOWS_REQUESTS: &str =
        "SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_ALLOWS_REQUESTS";
    /// Cannot remove quality profile the selected item because the selected value title(s) reference it.
    pub const CANNOT_REMOVE_QUALITY_PROFILE_TITLE_S_REFERENCE: &str =
        "SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_TITLE_S_REFERENCE";
    /// Cannot remove quality profile the selected item because the selected value pending media request(s) reference it.
    pub const CANNOT_REMOVE_QUALITY_PROFILE_PENDING_MEDIA_REQUEST_S_REFERENCE: &str =
        "SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_PENDING_MEDIA_REQUEST_S_REFERENCE";
    /// Profile_id is required when inherit_global is false.
    pub const PROFILE_ID_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE: &str =
        "SETTINGS_PROFILE_ID_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE";
    /// Persona is required when inherit_global is false.
    pub const PERSONA_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE: &str =
        "SETTINGS_PERSONA_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE";
    /// Profile_id is required.
    pub const PROFILE_ID_REQUIRED: &str = "SETTINGS_PROFILE_ID_REQUIRED";
    /// Quality profile was not found.
    pub const QUALITY_PROFILE_NOT_FOUND: &str = "SETTINGS_QUALITY_PROFILE_NOT_FOUND";
    /// Delay profile id is required.
    pub const DELAY_PROFILE_ID_REQUIRED: &str = "SETTINGS_DELAY_PROFILE_ID_REQUIRED";
    /// Delay profile was not found.
    pub const DELAY_PROFILE_NOT_FOUND: &str = "SETTINGS_DELAY_PROFILE_NOT_FOUND";
    /// Download client routing entry requires client_id.
    pub const DOWNLOAD_CLIENT_ROUTING_ENTRY_REQUIRES_CLIENT_ID: &str =
        "SETTINGS_DOWNLOAD_CLIENT_ROUTING_ENTRY_REQUIRES_CLIENT_ID";
    /// Indexer routing entry requires indexer_id.
    pub const INDEXER_ROUTING_ENTRY_REQUIRES_INDEXER_ID: &str =
        "SETTINGS_INDEXER_ROUTING_ENTRY_REQUIRES_INDEXER_ID";
    /// You do not have permission to view recycle bin settings.
    pub const VIEW_RECYCLE_BIN_SETTINGS_PERMISSION_DENIED: &str =
        "SETTINGS_VIEW_RECYCLE_BIN_SETTINGS_PERMISSION_DENIED";
    /// Recycle bin retention must be between 1 and the selected value days.
    pub const RECYCLE_BIN_RETENTION_OUT_OF_RANGE: &str =
        "SETTINGS_RECYCLE_BIN_RETENTION_OUT_OF_RANGE";
    /// Limit is required and must be greater than zero.
    pub const LIMIT_REQUIRED_OUT_OF_RANGE: &str = "SETTINGS_LIMIT_REQUIRED_OUT_OF_RANGE";
    /// Source is required.
    pub const SOURCE_REQUIRED: &str = "SETTINGS_SOURCE_REQUIRED";
    /// Default  library was not found.
    pub const DEFAULT_LIBRARY_NOT_FOUND: &str = "SETTINGS_DEFAULT_LIBRARY_NOT_FOUND";
    /// Library was not found.
    pub const LIBRARY_NOT_FOUND: &str = "SETTINGS_LIBRARY_NOT_FOUND";
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "SETTINGS_LIBRARY_ACCESS_DENIED";
    /// Monitor_specials is only valid for anime libraries.
    pub const MONITOR_SPECIALS_ONLY_VALID_ANIME_LIBRARIES: &str =
        "SETTINGS_MONITOR_SPECIALS_ONLY_VALID_ANIME_LIBRARIES";
    /// Plexmatch_write_on_import is only valid for series and anime libraries.
    pub const PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME_LIBRARIES: &str =
        "SETTINGS_PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME_LIBRARIES";
    /// Metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn.
    pub const METADATA_LANGUAGE_INVALID: &str = "SETTINGS_METADATA_LANGUAGE_INVALID";
    /// Anime-specific settings require an anime library.
    pub const ANIME_SPECIFIC_SETTINGS_REQUIRE_ANIME_LIBRARY: &str =
        "SETTINGS_ANIME_SPECIFIC_SETTINGS_REQUIRE_ANIME_LIBRARY";
    /// Season folders are only valid for series and anime libraries.
    pub const SEASON_FOLDERS_ONLY_VALID_SERIES_ANIME_LIBRARIES: &str =
        "SETTINGS_SEASON_FOLDERS_ONLY_VALID_SERIES_ANIME_LIBRARIES";
    /// Movie availability settings require a movie library.
    pub const MOVIE_AVAILABILITY_SETTINGS_REQUIRE_MOVIE_LIBRARY: &str =
        "SETTINGS_MOVIE_AVAILABILITY_SETTINGS_REQUIRE_MOVIE_LIBRARY";
    /// Unknown request quality profile the selected value.
    pub const REQUEST_QUALITY_PROFILE_UNKNOWN: &str = "SETTINGS_REQUEST_QUALITY_PROFILE_UNKNOWN";
    /// The recycle bin path is unsafe or overlaps managed media.
    pub const RECYCLE_BIN_PATH_UNSAFE: &str = "SETTINGS_RECYCLE_BIN_PATH_UNSAFE";
    /// The delay profile catalog is invalid.
    pub const DELAY_PROFILE_CATALOG_INVALID: &str = "SETTINGS_DELAY_PROFILE_CATALOG_INVALID";
    /// The trusted proxy policy is invalid.
    pub const TRUSTED_PROXY_POLICY_INVALID: &str = "SETTINGS_TRUSTED_PROXY_POLICY_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        DELAY_PROFILE_CATALOG_INVALID,
        TRUSTED_PROXY_POLICY_INVALID,
        CATALOG_VIEW_COLUMNS_INVALID,
        DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE,
        DASHBOARD_LAYOUT_TOO_MANY_ITEMS,
        DASHBOARD_PANEL_ID_INVALID,
        DASHBOARD_PANEL_ID_DUPLICATED,
        DASHBOARD_ITEM_POSITION_OUT_OF_RANGE,
        DASHBOARD_ITEM_SIZE_OUT_OF_RANGE,
        DASHBOARD_ITEM_EXCEEDS_COLUMNS,
        LANGUAGE_OUT_OF_RANGE,
        MUST_RRGGBB_HEX_COLOR,
        UI_TABLE_COLUMN_SETTING_COLUMN_DUPLICATED,
        TABLE_COLUMN_ORDER_OUT_OF_RANGE,
        TABLE_COLUMN_UNSUPPORTED,
        MOVIES_TABLE_SETTINGS_CANNOT_INCLUDE_EPISODES_COLUMN,
        TITLE_IMAGE_CACHE_REFRESH_ALREADY_RUNNING,
        SESSION_DURATION_OUT_OF_RANGE,
        PASSWORD_MINIMUM_LENGTH_OUT_OF_RANGE,
        CHANGE_DEFAULT_ADMIN_PASSWORD_BEFORE_ENABLING_FORM_LOGIN,
        CONFIGURE_ENABLED_FULL_ADMINISTRATOR_LOGIN_BEFORE_ENABLING_FORM_LOGIN,
        FORM_LOGIN_REQUIRED_DEFAULT_ADMIN_DISABLED,
        CANNOT_SAVE_RESET_TRUSTED_PROXIES_TOGETHER,
        DEFAULT_MOVIE_AVAILABILITY_INVALID,
        MOVIE_RELEASE_MARKET_INVALID,
        ROOT_FOLDER_PATH_REQUIRED,
        ROOT_REQUIRED,
        MUST_ABSOLUTE_PATH_COMMAND_NAME_WITHOUT_PATH_SEPARATORS,
        VIEW_VERIFICATION_SETTINGS_PERMISSION_DENIED,
        SUBTITLE_SEARCH_INTERVAL_OUT_OF_RANGE,
        SUBTITLE_MINIMUM_SCORE_PERCENTAGES_OUT_OF_RANGE,
        SUBTITLE_SYNC_THRESHOLD_PERCENTAGES_OUT_OF_RANGE,
        SUBTITLE_SYNC_MAX_OFFSET_CANNOT_NEGATIVE,
        SETTING_VALUE_INVALID,
        PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID,
        SEASON_FOLDER_TEMPLATES_ONLY_SUPPORTED_SERIES_ANIME,
        MOVIE_AVAILABILITY_SETTINGS_REQUIRE_SCOPE_MOVIE,
        PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME,
        ANIME_SPECIFIC_SETTINGS_REQUIRE_SCOPE_ANIME,
        MEDIA_SETTING_CHANGE_REQUIRED,
        AUTOMATIC_BACKUP_KEY_OUT_OF_RANGE,
        AUTOMATIC_BACKUP_KEY_CANNOT_REPLACED_CLEARED_SAME_REQUEST,
        AUTOMATIC_BACKUP_KEY_CANNOT_CLEARED_WHILE_AUTOMATIC_BACKUPS_DISABLED,
        AUTOMATIC_BACKUPS_REQUIRE_BACKUP_KEY,
        TRUSTED_CERTIFICATE_BUNDLE_INVALID,
        TRUSTED_CERTIFICATE_BUNDLE_MAY_ONLY_CONTAIN_X_509_CERTIFICATE_PEM_BLOCKS,
        CERTIFICATE_BUNDLE_EMPTY,
        FAILED_PARSE_TRUSTED_CERTIFICATE_PEM_BLOCK,
        FAILED_PARSE_TRAILING_PEM_CONTENT_TRUSTED_CERTIFICATE,
        EACH_TRUSTED_CERTIFICATE_ENTRY_INVALID,
        BACKUP_PATH_INVALID,
        FAILED_CREATE_BACKUP_DIRECTORY,
        BACKUP_PATH_NOT_WRITABLE,
        FAILED_REMOVE_BACKUP_PATH_PROBE,
        HISTORY_RETENTION_DAYS_OUT_OF_RANGE,
        IMAGE_CACHE_MAXIMUM_SIZE_OUT_OF_RANGE,
        ACQUISITION_THRESHOLDS_CANNOT_NEGATIVE,
        ACQUISITION_POLL_INTERVAL_OUT_OF_RANGE,
        ACQUISITION_WALK_INTERVAL_OUT_OF_RANGE,
        CONVERGENCE_PER_CYCLE_SCOPE_CEILING_OUT_OF_RANGE,
        RE_CONVERGE_BACKSTOP_CANNOT_NEGATIVE,
        QUALITY_PROFILE_ID_AMBIGUOUS_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE,
        QUALITY_PROFILE_ID_REQUIRED,
        QUALITY_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE,
        DELAY_PROFILE_NAMES_UNKNOWN_LIBRARY,
        VIEW_QUALITY_PROFILES_PERMISSION_DENIED,
        QUALITY_PROFILE_UNKNOWN,
        CANNOT_REMOVE_QUALITY_PROFILE_GLOBAL_DEFAULT,
        CANNOT_REMOVE_QUALITY_PROFILE_NOT_CONFIGURED,
        CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_REFERENCES,
        CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_ALLOWS_REQUESTS,
        CANNOT_REMOVE_QUALITY_PROFILE_TITLE_S_REFERENCE,
        CANNOT_REMOVE_QUALITY_PROFILE_PENDING_MEDIA_REQUEST_S_REFERENCE,
        PROFILE_ID_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE,
        PERSONA_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE,
        PROFILE_ID_REQUIRED,
        QUALITY_PROFILE_NOT_FOUND,
        DELAY_PROFILE_ID_REQUIRED,
        DELAY_PROFILE_NOT_FOUND,
        DOWNLOAD_CLIENT_ROUTING_ENTRY_REQUIRES_CLIENT_ID,
        INDEXER_ROUTING_ENTRY_REQUIRES_INDEXER_ID,
        VIEW_RECYCLE_BIN_SETTINGS_PERMISSION_DENIED,
        RECYCLE_BIN_RETENTION_OUT_OF_RANGE,
        LIMIT_REQUIRED_OUT_OF_RANGE,
        SOURCE_REQUIRED,
        DEFAULT_LIBRARY_NOT_FOUND,
        LIBRARY_NOT_FOUND,
        LIBRARY_ACCESS_DENIED,
        MONITOR_SPECIALS_ONLY_VALID_ANIME_LIBRARIES,
        PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME_LIBRARIES,
        METADATA_LANGUAGE_INVALID,
        ANIME_SPECIFIC_SETTINGS_REQUIRE_ANIME_LIBRARY,
        SEASON_FOLDERS_ONLY_VALID_SERIES_ANIME_LIBRARIES,
        MOVIE_AVAILABILITY_SETTINGS_REQUIRE_MOVIE_LIBRARY,
        REQUEST_QUALITY_PROFILE_UNKNOWN,
        RECYCLE_BIN_PATH_UNSAFE,
    ];
}

pub mod request_rules {
    /// Rule validation failed.
    pub const RULE_VALIDATION_FAILED: &str = "REQUEST_RULE_RULE_VALIDATION_FAILED";
    /// Sample requester was not found.
    pub const SAMPLE_REQUESTER_NOT_FOUND: &str = "REQUEST_RULE_SAMPLE_REQUESTER_NOT_FOUND";
    /// Sample library was not found.
    pub const SAMPLE_LIBRARY_NOT_FOUND: &str = "REQUEST_RULE_SAMPLE_LIBRARY_NOT_FOUND";
    /// Rule failed to compile.
    pub const RULE_FAILED_COMPILE: &str = "REQUEST_RULE_RULE_FAILED_COMPILE";
    /// Rule evaluation failed.
    pub const RULE_EVALUATION_FAILED: &str = "REQUEST_RULE_RULE_EVALUATION_FAILED";
    /// This rule reads facts about specific people, so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.
    pub const PERSON_TARGETING_PERMISSION_REQUIRED: &str =
        "REQUEST_RULE_PERSON_TARGETING_PERMISSION_REQUIRED";
    /// Request rule set was not found.
    pub const SET_NOT_FOUND: &str = "REQUEST_RULE_SET_NOT_FOUND";
    /// The requested value must not be empty.
    pub const FIELD_REQUIRED: &str = "REQUEST_RULE_FIELD_REQUIRED";
    /// A request may carry at most the selected value tags, the selected value were supplied.
    pub const REQUEST_MAY_CARRY_MOST_TAGS_SUPPLIED: &str =
        "REQUEST_RULE_REQUEST_MAY_CARRY_MOST_TAGS_SUPPLIED";
    /// Tag must not be empty.
    pub const TAG_REQUIRED: &str = "REQUEST_RULE_TAG_REQUIRED";
    /// Tag is the selected value characters, at most the selected value are allowed.
    pub const TAG_CHARACTERS_MOST_ALLOWED: &str = "REQUEST_RULE_TAG_CHARACTERS_MOST_ALLOWED";
    /// Tag the selected item uses the reserved the selected item prefix.
    pub const TAG_USES_RESERVED_PREFIX: &str = "REQUEST_RULE_TAG_USES_RESERVED_PREFIX";
    /// Tag the selected item contains unsupported character the selected item; use letters, digits, '.', '_', '-', or spaces.
    pub const TAG_CONTAINS_UNSUPPORTED_CHARACTER: &str =
        "REQUEST_RULE_TAG_CONTAINS_UNSUPPORTED_CHARACTER";
    /// A lease must be at most the selected value days (ten years), or forever.
    pub const LEASE_OUT_OF_RANGE: &str = "REQUEST_RULE_LEASE_OUT_OF_RANGE";
    /// Library was not found.
    pub const LIBRARY_NOT_FOUND: &str = "REQUEST_RULE_LIBRARY_NOT_FOUND";
    /// Library facet does not match requested media facet.
    pub const LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET: &str =
        "REQUEST_RULE_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET";
    /// Media requests must include SMG external identifiers.
    pub const MEDIA_REQUESTS_INVALID: &str = "REQUEST_RULE_MEDIA_REQUESTS_INVALID";
    /// Media request id is required.
    pub const MEDIA_REQUEST_ID_REQUIRED: &str = "REQUEST_RULE_MEDIA_REQUEST_ID_REQUIRED";
    /// Media request was not found.
    pub const MEDIA_REQUEST_NOT_FOUND: &str = "REQUEST_RULE_MEDIA_REQUEST_NOT_FOUND";
    /// You may not read this request's decision.
    pub const MAY_NOT_READ_REQUEST_S_DECISION: &str =
        "REQUEST_RULE_MAY_NOT_READ_REQUEST_S_DECISION";
    /// Failed to build the request rules engine.
    pub const FAILED_BUILD_REQUEST_RULES_ENGINE: &str =
        "REQUEST_RULE_FAILED_BUILD_REQUEST_RULES_ENGINE";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        RULE_VALIDATION_FAILED,
        SAMPLE_REQUESTER_NOT_FOUND,
        SAMPLE_LIBRARY_NOT_FOUND,
        RULE_FAILED_COMPILE,
        RULE_EVALUATION_FAILED,
        PERSON_TARGETING_PERMISSION_REQUIRED,
        SET_NOT_FOUND,
        FIELD_REQUIRED,
        REQUEST_MAY_CARRY_MOST_TAGS_SUPPLIED,
        TAG_REQUIRED,
        TAG_CHARACTERS_MOST_ALLOWED,
        TAG_USES_RESERVED_PREFIX,
        TAG_CONTAINS_UNSUPPORTED_CHARACTER,
        LEASE_OUT_OF_RANGE,
        LIBRARY_NOT_FOUND,
        LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET,
        MEDIA_REQUESTS_INVALID,
        MEDIA_REQUEST_ID_REQUIRED,
        MEDIA_REQUEST_NOT_FOUND,
        MAY_NOT_READ_REQUEST_S_DECISION,
        FAILED_BUILD_REQUEST_RULES_ENGINE,
    ];
}

pub mod quality {
    /// Score refresh user no longer exists.
    pub const SCORE_REFRESH_USER_NO_LONGER_EXISTS: &str =
        "QUALITY_SCORE_REFRESH_USER_NO_LONGER_EXISTS";
    /// Media file was not found.
    pub const MEDIA_FILE_NOT_FOUND: &str = "QUALITY_MEDIA_FILE_NOT_FOUND";
    /// Media title was not found.
    pub const MEDIA_TITLE_NOT_FOUND: &str = "QUALITY_MEDIA_TITLE_NOT_FOUND";
    /// File score evaluation failed; previous evaluation retained.
    pub const FILE_SCORE_EVALUATION_FAILED: &str = "QUALITY_FILE_SCORE_EVALUATION_FAILED";
    /// File scoring inputs changed; refresh again.
    pub const FILE_SCORING_INPUTS_CHANGED: &str = "QUALITY_FILE_SCORING_INPUTS_CHANGED";
    /// Select between 1 and 1000 files, seasons, or titles.
    pub const SELECT_BETWEEN_1_1000_FILES_SEASONS_TITLES: &str =
        "QUALITY_SELECT_BETWEEN_1_1000_FILES_SEASONS_TITLES";
    /// Selected file was not found.
    pub const FILE_NOT_FOUND: &str = "QUALITY_FILE_NOT_FOUND";
    /// Selected season was not found.
    pub const SEASON_NOT_FOUND: &str = "QUALITY_SEASON_NOT_FOUND";
    /// Selected title was not found.
    pub const TITLE_NOT_FOUND: &str = "QUALITY_TITLE_NOT_FOUND";
    /// Score refresh queue is full.
    pub const SCORE_REFRESH_QUEUE_FULL: &str = "QUALITY_SCORE_REFRESH_QUEUE_FULL";
    /// Score refresh belongs to another user.
    pub const SCORE_REFRESH_BELONGS_ANOTHER_USER: &str =
        "QUALITY_SCORE_REFRESH_BELONGS_ANOTHER_USER";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SCORE_REFRESH_USER_NO_LONGER_EXISTS,
        MEDIA_FILE_NOT_FOUND,
        MEDIA_TITLE_NOT_FOUND,
        FILE_SCORE_EVALUATION_FAILED,
        FILE_SCORING_INPUTS_CHANGED,
        SELECT_BETWEEN_1_1000_FILES_SEASONS_TITLES,
        FILE_NOT_FOUND,
        SEASON_NOT_FOUND,
        TITLE_NOT_FOUND,
        SCORE_REFRESH_QUEUE_FULL,
        SCORE_REFRESH_BELONGS_ANOTHER_USER,
    ];
}

pub mod lists {
    /// List owner was not found.
    pub const OWNER_NOT_FOUND: &str = "LIST_OWNER_NOT_FOUND";
    /// List owner is disabled.
    pub const OWNER_DISABLED: &str = "LIST_OWNER_DISABLED";
    /// Title was not found.
    pub const TITLE_NOT_FOUND: &str = "LIST_TITLE_NOT_FOUND";
    /// List provider was not found.
    pub const PROVIDER_NOT_FOUND: &str = "LIST_PROVIDER_NOT_FOUND";
    /// List subscription was not found.
    pub const SUBSCRIPTION_NOT_FOUND: &str = "LIST_SUBSCRIPTION_NOT_FOUND";
    /// List exclusion was not found.
    pub const EXCLUSION_NOT_FOUND: &str = "LIST_EXCLUSION_NOT_FOUND";
    /// User was not found.
    pub const USER_NOT_FOUND: &str = "LIST_USER_NOT_FOUND";
    /// List subscription changed was not found.
    pub const SUBSCRIPTION_CHANGED_NOT_FOUND: &str = "LIST_SUBSCRIPTION_CHANGED_NOT_FOUND";
    /// Add and search need title management permission.
    pub const ADD_SEARCH_NEED_TITLE_MANAGEMENT_PERMISSION: &str =
        "LIST_ADD_SEARCH_NEED_TITLE_MANAGEMENT_PERMISSION";
    /// Departure changes need title management permission.
    pub const DEPARTURE_CHANGES_NEED_TITLE_MANAGEMENT_PERMISSION: &str =
        "LIST_DEPARTURE_CHANGES_NEED_TITLE_MANAGEMENT_PERMISSION";
    /// List account was not found.
    pub const ACCOUNT_NOT_FOUND: &str = "LIST_ACCOUNT_NOT_FOUND";
    /// List account link was not found.
    pub const ACCOUNT_LINK_NOT_FOUND: &str = "LIST_ACCOUNT_LINK_NOT_FOUND";
    /// Could not start account link.
    pub const COULD_NOT_START_ACCOUNT_LINK: &str = "LIST_COULD_NOT_START_ACCOUNT_LINK";
    /// Account is disabled.
    pub const ACCOUNT_DISABLED: &str = "LIST_ACCOUNT_DISABLED";
    /// Personal lists are not allowed for this member.
    pub const PERSONAL_LISTS_NOT_ALLOWED_MEMBER: &str = "LIST_PERSONAL_LISTS_NOT_ALLOWED_MEMBER";
    /// The list provider asked for fewer requests.
    pub const SOURCE_RATE_LIMITED: &str = "LIST_SOURCE_RATE_LIMITED";
    /// The list provider could not be reached.
    pub const SOURCE_UNAVAILABLE: &str = "LIST_SOURCE_UNAVAILABLE";
    /// The provider did not return a readable list.
    pub const SOURCE_FAILED: &str = "LIST_SOURCE_FAILED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SOURCE_RATE_LIMITED,
        SOURCE_UNAVAILABLE,
        SOURCE_FAILED,
        OWNER_NOT_FOUND,
        OWNER_DISABLED,
        TITLE_NOT_FOUND,
        PROVIDER_NOT_FOUND,
        SUBSCRIPTION_NOT_FOUND,
        EXCLUSION_NOT_FOUND,
        USER_NOT_FOUND,
        SUBSCRIPTION_CHANGED_NOT_FOUND,
        ADD_SEARCH_NEED_TITLE_MANAGEMENT_PERMISSION,
        DEPARTURE_CHANGES_NEED_TITLE_MANAGEMENT_PERMISSION,
        ACCOUNT_NOT_FOUND,
        ACCOUNT_LINK_NOT_FOUND,
        COULD_NOT_START_ACCOUNT_LINK,
        ACCOUNT_DISABLED,
        PERSONAL_LISTS_NOT_ALLOWED_MEMBER,
    ];
}

pub mod media_servers {
    /// Media server base URL is required.
    pub const BASE_URL_REQUIRED: &str = "MEDIA_SERVER_BASE_URL_REQUIRED";
    /// Media server base URL must be an HTTP or HTTPS URL.
    pub const BASE_URL_INVALID: &str = "MEDIA_SERVER_BASE_URL_INVALID";
    /// Media server base URL must not include a query or fragment.
    pub const BASE_URL_QUERY_OR_FRAGMENT: &str = "MEDIA_SERVER_BASE_URL_QUERY_OR_FRAGMENT";
    /// Media server external URL must be an HTTP or HTTPS URL.
    pub const EXTERNAL_URL_INVALID: &str = "MEDIA_SERVER_EXTERNAL_URL_INVALID";
    /// Media server external URL must not include credentials.
    pub const EXTERNAL_URL_CREDENTIALS_FORBIDDEN: &str =
        "MEDIA_SERVER_EXTERNAL_URL_CREDENTIALS_FORBIDDEN";
    /// Media server external URL must not include a query or fragment.
    pub const EXTERNAL_URL_QUERY_OR_FRAGMENT: &str = "MEDIA_SERVER_EXTERNAL_URL_QUERY_OR_FRAGMENT";
    /// Media server path mappings must have unique source paths.
    pub const PATH_MAPPINGS_INVALID: &str = "MEDIA_SERVER_PATH_MAPPINGS_INVALID";
    /// Plex did not return any accessible servers for that account.
    pub const PLEX_DID_NOT_RETURN_ANY_ACCESSIBLE_SERVERS_ACCOUNT: &str =
        "MEDIA_SERVER_PLEX_DID_NOT_RETURN_ANY_ACCESSIBLE_SERVERS_ACCOUNT";
    /// The selected Plex server was not returned by Plex discovery.
    pub const SELECTED_PLEX_SERVER_NOT_RETURNED_PLEX_DISCOVERY: &str =
        "MEDIA_SERVER_SELECTED_PLEX_SERVER_NOT_RETURNED_PLEX_DISCOVERY";
    /// Select a Plex server before saving this connection.
    pub const SELECT_PLEX_SERVER_BEFORE_SAVING_CONNECTION: &str =
        "MEDIA_SERVER_SELECT_PLEX_SERVER_BEFORE_SAVING_CONNECTION";
    /// Sign in with Plex before selecting a Plex server.
    pub const SIGN_PLEX_BEFORE_SELECTING_PLEX_SERVER: &str =
        "MEDIA_SERVER_SIGN_PLEX_BEFORE_SELECTING_PLEX_SERVER";
    /// Media server connection id is required.
    pub const CONNECTION_ID_REQUIRED: &str = "MEDIA_SERVER_CONNECTION_ID_REQUIRED";
    /// Media server connection was not found.
    pub const CONNECTION_NOT_FOUND: &str = "MEDIA_SERVER_CONNECTION_NOT_FOUND";
    /// Cannot change provider for a connection with linked accounts.
    pub const CANNOT_CHANGE_PROVIDER_CONNECTION_LINKED_ACCOUNTS: &str =
        "MEDIA_SERVER_CANNOT_CHANGE_PROVIDER_CONNECTION_LINKED_ACCOUNTS";
    /// An enabled Emby connection must retain a verified API key.
    pub const ENABLED_EMBY_CONNECTION_MUST_RETAIN_VERIFIED_API_KEY: &str =
        "MEDIA_SERVER_ENABLED_EMBY_CONNECTION_MUST_RETAIN_VERIFIED_API_KEY";
    /// Changing an Emby server URL requires a stored API key.
    pub const CHANGING_EMBY_SERVER_URL_REQUIRES_STORED_API_KEY: &str =
        "MEDIA_SERVER_CHANGING_EMBY_SERVER_URL_REQUIRES_STORED_API_KEY";
    /// Emby Connect login requires a verified server identity.
    pub const EMBY_CONNECT_LOGIN_REQUIRES_VERIFIED_SERVER_IDENTITY: &str =
        "MEDIA_SERVER_EMBY_CONNECT_LOGIN_REQUIRES_VERIFIED_SERVER_IDENTITY";
    /// Media server connection is referenced by linked accounts; disable it instead.
    pub const CONNECTION_REFERENCED_LINKED_ACCOUNTS: &str =
        "MEDIA_SERVER_CONNECTION_REFERENCED_LINKED_ACCOUNTS";
    /// Media server connection is referenced by notification channels; disable it instead.
    pub const CONNECTION_REFERENCED_NOTIFICATION_CHANNELS: &str =
        "MEDIA_SERVER_CONNECTION_REFERENCED_NOTIFICATION_CHANNELS";
    /// Emby connection test requires a saved integration API key.
    pub const EMBY_CONNECTION_TEST_REQUIRES_SAVED_INTEGRATION_API_KEY: &str =
        "MEDIA_SERVER_EMBY_CONNECTION_TEST_REQUIRES_SAVED_INTEGRATION_API_KEY";
    /// Discover and select a Plex server before enabling login, linking, or auto-add.
    pub const DISCOVER_SELECT_PLEX_SERVER_BEFORE_ENABLING_LOGIN_LINKING_AUTO_ADD: &str =
        "MEDIA_SERVER_DISCOVER_SELECT_PLEX_SERVER_BEFORE_ENABLING_LOGIN_LINKING_AUTO_ADD";
    /// Sign in with Plex to test this connection.
    pub const SIGN_PLEX_TEST_CONNECTION: &str = "MEDIA_SERVER_SIGN_PLEX_TEST_CONNECTION";
    /// Plex account does not have access to the selected server.
    pub const PLEX_ACCOUNT_DOES_NOT_ACCESS_SELECTED_SERVER: &str =
        "MEDIA_SERVER_PLEX_ACCOUNT_DOES_NOT_ACCESS_SELECTED_SERVER";
    /// Emby connection requires a verified integration API key.
    pub const EMBY_CONNECTION_REQUIRES_VERIFIED_INTEGRATION_API_KEY: &str =
        "MEDIA_SERVER_EMBY_CONNECTION_REQUIRES_VERIFIED_INTEGRATION_API_KEY";
    /// Emby server identity does not match the saved connection.
    pub const EMBY_SERVER_IDENTITY_DOES_NOT_MATCH_SAVED_CONNECTION: &str =
        "MEDIA_SERVER_EMBY_SERVER_IDENTITY_DOES_NOT_MATCH_SAVED_CONNECTION";
    /// Episode was not found.
    pub const EPISODE_NOT_FOUND: &str = "MEDIA_SERVER_EPISODE_NOT_FOUND";
    /// Jellyfin user listing requires a Jellyfin connection.
    pub const JELLYFIN_USER_LISTING_REQUIRES_JELLYFIN_CONNECTION: &str =
        "MEDIA_SERVER_JELLYFIN_USER_LISTING_REQUIRES_JELLYFIN_CONNECTION";
    /// Jellyfin user listing requires a saved API key; save an API key to load Jellyfin users.
    pub const JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY: &str =
        "MEDIA_SERVER_JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY";
    /// Choose either a Jellyfin API key or Jellyfin admin login credentials, not both.
    pub const CHOOSE_EITHER_JELLYFIN_API_KEY_JELLYFIN_ADMIN_LOGIN_CREDENTIALS_NOT_BOTH: &str =
        "MEDIA_SERVER_CHOOSE_EITHER_JELLYFIN_API_KEY_JELLYFIN_ADMIN_LOGIN_CREDENTIALS_NOT_BOTH";
    /// Both Jellyfin admin username and password are required.
    pub const BOTH_JELLYFIN_ADMIN_USERNAME_PASSWORD_REQUIRED: &str =
        "MEDIA_SERVER_BOTH_JELLYFIN_ADMIN_USERNAME_PASSWORD_REQUIRED";
    /// Emby Connect test requires an Emby connection.
    pub const EMBY_CONNECT_TEST_REQUIRES_EMBY_CONNECTION: &str =
        "MEDIA_SERVER_EMBY_CONNECT_TEST_REQUIRES_EMBY_CONNECTION";
    /// Emby Connect sign-in is disabled for this connection.
    pub const EMBY_CONNECT_SIGN_DISABLED_CONNECTION: &str =
        "MEDIA_SERVER_EMBY_CONNECT_SIGN_DISABLED_CONNECTION";
    /// Emby connection has no verified server identity.
    pub const EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY: &str =
        "MEDIA_SERVER_EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY";
    /// Emby user listing requires an Emby connection.
    pub const EMBY_USER_LISTING_REQUIRES_EMBY_CONNECTION: &str =
        "MEDIA_SERVER_EMBY_USER_LISTING_REQUIRES_EMBY_CONNECTION";
    /// Emby user listing requires a saved integration API key.
    pub const EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY: &str =
        "MEDIA_SERVER_EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY";
    /// Emby avatar was not found.
    pub const EMBY_AVATAR_NOT_FOUND: &str = "MEDIA_SERVER_EMBY_AVATAR_NOT_FOUND";
    /// Local Emby setup does not accept Emby Connect credentials.
    pub const LOCAL_EMBY_SETUP_DOES_NOT_ACCEPT_EMBY_CONNECT_CREDENTIALS: &str =
        "MEDIA_SERVER_LOCAL_EMBY_SETUP_DOES_NOT_ACCEPT_EMBY_CONNECT_CREDENTIALS";
    /// Choose either an Emby API key or local administrator credentials, not both.
    pub const CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH: &str =
        "MEDIA_SERVER_CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH";
    /// Emby API key is required.
    pub const EMBY_API_KEY_REQUIRED: &str = "MEDIA_SERVER_EMBY_API_KEY_REQUIRED";
    /// Both Emby administrator username and password are required.
    pub const BOTH_EMBY_ADMINISTRATOR_USERNAME_PASSWORD_REQUIRED: &str =
        "MEDIA_SERVER_BOTH_EMBY_ADMINISTRATOR_USERNAME_PASSWORD_REQUIRED";
    /// Emby Connect setup does not accept a pasted API key or local administrator credentials.
    pub const EMBY_CONNECT_LOCAL_CREDENTIALS_FORBIDDEN: &str =
        "MEDIA_SERVER_EMBY_CONNECT_LOCAL_CREDENTIALS_FORBIDDEN";
    /// Emby Connect username or email is required.
    pub const EMBY_CONNECT_USERNAME_EMAIL_REQUIRED: &str =
        "MEDIA_SERVER_EMBY_CONNECT_USERNAME_EMAIL_REQUIRED";
    /// Emby Connect password is required.
    pub const EMBY_CONNECT_PASSWORD_REQUIRED: &str = "MEDIA_SERVER_EMBY_CONNECT_PASSWORD_REQUIRED";
    /// Select an Emby Connect server.
    pub const SELECT_EMBY_CONNECT_SERVER: &str = "MEDIA_SERVER_SELECT_EMBY_CONNECT_SERVER";
    /// Selected Emby Connect server identity does not match the reachable server.
    pub const SELECTED_EMBY_CONNECT_SERVER_IDENTITY_DOES_NOT_MATCH_REACHABLE_SERVER: &str =
        "MEDIA_SERVER_SELECTED_EMBY_CONNECT_SERVER_IDENTITY_DOES_NOT_MATCH_REACHABLE_SERVER";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        BASE_URL_REQUIRED,
        BASE_URL_INVALID,
        BASE_URL_QUERY_OR_FRAGMENT,
        EXTERNAL_URL_INVALID,
        EXTERNAL_URL_CREDENTIALS_FORBIDDEN,
        EXTERNAL_URL_QUERY_OR_FRAGMENT,
        PATH_MAPPINGS_INVALID,
        PLEX_DID_NOT_RETURN_ANY_ACCESSIBLE_SERVERS_ACCOUNT,
        SELECTED_PLEX_SERVER_NOT_RETURNED_PLEX_DISCOVERY,
        SELECT_PLEX_SERVER_BEFORE_SAVING_CONNECTION,
        SIGN_PLEX_BEFORE_SELECTING_PLEX_SERVER,
        CONNECTION_ID_REQUIRED,
        CONNECTION_NOT_FOUND,
        CANNOT_CHANGE_PROVIDER_CONNECTION_LINKED_ACCOUNTS,
        ENABLED_EMBY_CONNECTION_MUST_RETAIN_VERIFIED_API_KEY,
        CHANGING_EMBY_SERVER_URL_REQUIRES_STORED_API_KEY,
        EMBY_CONNECT_LOGIN_REQUIRES_VERIFIED_SERVER_IDENTITY,
        CONNECTION_REFERENCED_LINKED_ACCOUNTS,
        CONNECTION_REFERENCED_NOTIFICATION_CHANNELS,
        EMBY_CONNECTION_TEST_REQUIRES_SAVED_INTEGRATION_API_KEY,
        DISCOVER_SELECT_PLEX_SERVER_BEFORE_ENABLING_LOGIN_LINKING_AUTO_ADD,
        SIGN_PLEX_TEST_CONNECTION,
        PLEX_ACCOUNT_DOES_NOT_ACCESS_SELECTED_SERVER,
        EMBY_CONNECTION_REQUIRES_VERIFIED_INTEGRATION_API_KEY,
        EMBY_SERVER_IDENTITY_DOES_NOT_MATCH_SAVED_CONNECTION,
        EPISODE_NOT_FOUND,
        JELLYFIN_USER_LISTING_REQUIRES_JELLYFIN_CONNECTION,
        JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY,
        CHOOSE_EITHER_JELLYFIN_API_KEY_JELLYFIN_ADMIN_LOGIN_CREDENTIALS_NOT_BOTH,
        BOTH_JELLYFIN_ADMIN_USERNAME_PASSWORD_REQUIRED,
        EMBY_CONNECT_TEST_REQUIRES_EMBY_CONNECTION,
        EMBY_CONNECT_SIGN_DISABLED_CONNECTION,
        EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY,
        EMBY_USER_LISTING_REQUIRES_EMBY_CONNECTION,
        EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY,
        EMBY_AVATAR_NOT_FOUND,
        LOCAL_EMBY_SETUP_DOES_NOT_ACCEPT_EMBY_CONNECT_CREDENTIALS,
        CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH,
        EMBY_API_KEY_REQUIRED,
        BOTH_EMBY_ADMINISTRATOR_USERNAME_PASSWORD_REQUIRED,
        EMBY_CONNECT_LOCAL_CREDENTIALS_FORBIDDEN,
        EMBY_CONNECT_USERNAME_EMAIL_REQUIRED,
        EMBY_CONNECT_PASSWORD_REQUIRED,
        SELECT_EMBY_CONNECT_SERVER,
        SELECTED_EMBY_CONNECT_SERVER_IDENTITY_DOES_NOT_MATCH_REACHABLE_SERVER,
    ];
}

pub mod indexer_caps {
    /// Capability discovery failed; the previous snapshot remains usable.
    pub const REFRESH_FAILED: &str = "INDEXER_CAPS_REFRESH_FAILED";
    /// Discovery did not provide a usable replacement for missing or stale caps.
    pub const REFRESH_DEFERRED: &str = "INDEXER_CAPS_REFRESH_DEFERRED";
    /// Every reason owned by capability resolution.
    pub const ALL: &[&str] = &[REFRESH_FAILED, REFRESH_DEFERRED];

    #[cfg(test)]
    mod tests {
        #[test]
        fn reasons_are_distinct_prefixed_and_screaming_snake_case() {
            let mut unique = std::collections::HashSet::new();
            for reason in super::ALL {
                assert!(reason.starts_with("INDEXER_CAPS_"));
                assert!(
                    reason
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                );
                assert!(unique.insert(reason));
            }
        }
    }
}

pub mod integration {
    /// Indexer provider is unavailable.
    pub const INDEXER_PROVIDER_UNAVAILABLE: &str = "INTEGRATION_INDEXER_PROVIDER_UNAVAILABLE";
    /// Plugin returned invalid config fields.
    pub const PLUGIN_RETURNED_INVALID_CONFIG_FIELDS: &str =
        "INTEGRATION_PLUGIN_RETURNED_INVALID_CONFIG_FIELDS";
    /// Indexer settings changed during validation; reload and test again.
    pub const INDEXER_SETTINGS_CHANGED_DURING_VALIDATION: &str =
        "INTEGRATION_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION";
    /// No indexer management client available for provider type the selected item.
    pub const NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE: &str =
        "INTEGRATION_NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE";
    /// No indexer provider available for provider type the selected item.
    pub const NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE: &str =
        "INTEGRATION_NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE";
    /// Indexer connection test did not return a valid Newznab caps document.
    pub const INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT: &str =
        "INTEGRATION_INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT";
    /// API key appears to be a masked placeholder — enter the real key.
    pub const API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY: &str =
        "INTEGRATION_API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY";
    /// Proxy configuration was not found.
    pub const PROXY_CONFIGURATION_NOT_FOUND: &str = "INTEGRATION_PROXY_CONFIGURATION_NOT_FOUND";
    /// Proxy is disabled for this indexer.
    pub const PROXY_DISABLED_INDEXER: &str = "INTEGRATION_PROXY_DISABLED_INDEXER";
    /// Prowlarr indexers cannot use challenge solvers; Prowlarr owns challenge handling.
    pub const PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS: &str =
        "INTEGRATION_PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS";
    /// Provider type the selected item does not support managed child sync.
    pub const PROVIDER_TYPE_DOES_NOT_SUPPORT_MANAGED_CHILD_SYNC: &str =
        "INTEGRATION_PROVIDER_TYPE_DOES_NOT_SUPPORT_MANAGED_CHILD_SYNC";
    /// Indexer connection test failed.
    pub const INDEXER_CONNECTION_TEST_FAILED: &str = "INTEGRATION_INDEXER_CONNECTION_TEST_FAILED";
    /// Base URL must use http or https.
    pub const BASE_URL_INVALID: &str = "INTEGRATION_BASE_URL_INVALID";
    /// Base URL must include a host.
    pub const BASE_URL_HOST_REQUIRED: &str = "INTEGRATION_BASE_URL_HOST_REQUIRED";
    /// Base URL must not include embedded credentials.
    pub const BASE_URL_CREDENTIALS_FORBIDDEN: &str = "INTEGRATION_BASE_URL_CREDENTIALS_FORBIDDEN";
    /// Download submission was not found.
    pub const DOWNLOAD_SUBMISSION_NOT_FOUND: &str = "INTEGRATION_DOWNLOAD_SUBMISSION_NOT_FOUND";
    /// This download client does not support password retries.
    pub const DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_PASSWORD_RETRIES: &str =
        "INTEGRATION_DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_PASSWORD_RETRIES";
    /// Password retry requires an exact download client.
    pub const PASSWORD_RETRY_REQUIRES_EXACT_DOWNLOAD_CLIENT: &str =
        "INTEGRATION_PASSWORD_RETRY_REQUIRES_EXACT_DOWNLOAD_CLIENT";
    /// Download is no longer available for manual import.
    pub const DOWNLOAD_UNAVAILABLE: &str = "INTEGRATION_DOWNLOAD_UNAVAILABLE";
    /// Seeding profile id is required.
    pub const SEEDING_PROFILE_ID_REQUIRED: &str = "INTEGRATION_SEEDING_PROFILE_ID_REQUIRED";
    /// At least one seeding profile field must be provided.
    pub const SEEDING_PROFILE_FIELD_REQUIRED: &str = "INTEGRATION_SEEDING_PROFILE_FIELD_REQUIRED";
    /// Seeding profile was not found.
    pub const SEEDING_PROFILE_NOT_FOUND: &str = "INTEGRATION_SEEDING_PROFILE_NOT_FOUND";
    /// Seeding profile is still assigned to the selected value.
    pub const SEEDING_PROFILE_STILL_ASSIGNED: &str = "INTEGRATION_SEEDING_PROFILE_STILL_ASSIGNED";
    /// Indexer id is required.
    pub const INDEXER_ID_REQUIRED: &str = "INTEGRATION_INDEXER_ID_REQUIRED";
    /// Indexer config was not found.
    pub const INDEXER_CONFIG_NOT_FOUND: &str = "INTEGRATION_INDEXER_CONFIG_NOT_FOUND";
    /// Prowlarr management parents cannot be assigned a seeding profile.
    pub const PROWLARR_MANAGEMENT_PARENTS_CANNOT_ASSIGNED_SEEDING_PROFILE: &str =
        "INTEGRATION_PROWLARR_MANAGEMENT_PARENTS_CANNOT_ASSIGNED_SEEDING_PROFILE";
    /// Indexer provider the selected item does not declare a supported protocol family.
    pub const INDEXER_PROVIDER_DOES_NOT_DECLARE_SUPPORTED_PROTOCOL_FAMILY: &str =
        "INTEGRATION_INDEXER_PROVIDER_DOES_NOT_DECLARE_SUPPORTED_PROTOCOL_FAMILY";
    /// Indexer the selected item does not support torrents, so seeding profile the selected item cannot be assigned.
    pub const INDEXER_DOES_NOT_SUPPORT_TORRENTS_SO_SEEDING_PROFILE_CANNOT_ASSIGNED: &str =
        "INTEGRATION_INDEXER_DOES_NOT_SUPPORT_TORRENTS_SO_SEEDING_PROFILE_CANNOT_ASSIGNED";
    /// Minimum seeders floor cannot be negative.
    pub const MINIMUM_SEEDERS_FLOOR_CANNOT_NEGATIVE: &str =
        "INTEGRATION_MINIMUM_SEEDERS_FLOOR_CANNOT_NEGATIVE";
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "INTEGRATION_LIBRARY_ACCESS_DENIED";
    /// Client type is required.
    pub const CLIENT_TYPE_REQUIRED: &str = "INTEGRATION_CLIENT_TYPE_REQUIRED";
    /// Unsupported download client type the selected item.
    pub const DOWNLOAD_CLIENT_TYPE_UNSUPPORTED: &str =
        "INTEGRATION_DOWNLOAD_CLIENT_TYPE_UNSUPPORTED";
    /// You do not have permission to perform this action.
    pub const ACTION_PERMISSION_DENIED: &str = "INTEGRATION_ACTION_PERMISSION_DENIED";
    /// Client id is required.
    pub const CLIENT_ID_REQUIRED: &str = "INTEGRATION_CLIENT_ID_REQUIRED";
    /// Proxy config id is required.
    pub const PROXY_CONFIG_ID_REQUIRED: &str = "INTEGRATION_PROXY_CONFIG_ID_REQUIRED";
    /// Proxy is disabled for this download client.
    pub const PROXY_DISABLED_DOWNLOAD_CLIENT: &str = "INTEGRATION_PROXY_DISABLED_DOWNLOAD_CLIENT";
    /// Download client name is required.
    pub const DOWNLOAD_CLIENT_NAME_REQUIRED: &str = "INTEGRATION_DOWNLOAD_CLIENT_NAME_REQUIRED";
    /// At least one download client field must be provided.
    pub const DOWNLOAD_CLIENT_FIELD_REQUIRED: &str = "INTEGRATION_DOWNLOAD_CLIENT_FIELD_REQUIRED";
    /// Client name cannot be empty.
    pub const CLIENT_NAME_REQUIRED: &str = "INTEGRATION_CLIENT_NAME_REQUIRED";
    /// Download client config was not found.
    pub const DOWNLOAD_CLIENT_CONFIG_NOT_FOUND: &str =
        "INTEGRATION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND";
    /// Manual import selection is unavailable; reopen the import dialog.
    pub const MANUAL_IMPORT_SELECTION_UNAVAILABLE: &str =
        "INTEGRATION_MANUAL_IMPORT_SELECTION_UNAVAILABLE";
    /// Manual import selection title is unavailable.
    pub const MANUAL_IMPORT_SELECTION_TITLE_UNAVAILABLE: &str =
        "INTEGRATION_MANUAL_IMPORT_SELECTION_TITLE_UNAVAILABLE";
    /// Manual import title does not match the Scryer submission that grabbed this download.
    pub const MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD: &str =
        "INTEGRATION_MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD";
    /// Manual import files are no longer available.
    pub const MANUAL_IMPORT_FILES_UNAVAILABLE: &str = "INTEGRATION_MANUAL_IMPORT_FILES_UNAVAILABLE";
    /// Manual import selection changed; reopen the import dialog.
    pub const MANUAL_IMPORT_SELECTION_CHANGED: &str = "INTEGRATION_MANUAL_IMPORT_SELECTION_CHANGED";
    /// Manual import candidate is unavailable.
    pub const MANUAL_IMPORT_CANDIDATE_UNAVAILABLE: &str =
        "INTEGRATION_MANUAL_IMPORT_CANDIDATE_UNAVAILABLE";
    /// Download client item id is required.
    pub const DOWNLOAD_CLIENT_ITEM_ID_REQUIRED: &str =
        "INTEGRATION_DOWNLOAD_CLIENT_ITEM_ID_REQUIRED";
    /// This download is not in a state that can be imported by hand right now.
    pub const DOWNLOAD_NOT_STATE_IMPORTED_HAND_RIGHT_NOW: &str =
        "INTEGRATION_DOWNLOAD_NOT_STATE_IMPORTED_HAND_RIGHT_NOW";
    /// Stored proxy protocol does not match its provider.
    pub const STORED_PROXY_PROTOCOL_DOES_NOT_MATCH_PROVIDER: &str =
        "INTEGRATION_STORED_PROXY_PROTOCOL_DOES_NOT_MATCH_PROVIDER";
    /// At least one proxy field must be provided.
    pub const PROXY_FIELD_REQUIRED: &str = "INTEGRATION_PROXY_FIELD_REQUIRED";
    /// Proxy config was not found.
    pub const PROXY_CONFIG_NOT_FOUND: &str = "INTEGRATION_PROXY_CONFIG_NOT_FOUND";
    /// Proxy config is assigned to the selected value enabled indexer(s).
    pub const PROXY_CONFIG_ASSIGNED_ENABLED_INDEXER_S: &str =
        "INTEGRATION_PROXY_CONFIG_ASSIGNED_ENABLED_INDEXER_S";
    /// Proxy config is assigned to the selected value enabled download client(s).
    pub const PROXY_CONFIG_ASSIGNED_ENABLED_DOWNLOAD_CLIENT_S: &str =
        "INTEGRATION_PROXY_CONFIG_ASSIGNED_ENABLED_DOWNLOAD_CLIENT_S";
    /// Proxy config is assigned to one or more indexers.
    pub const PROXY_CONFIG_ASSIGNED_ONE_MORE_INDEXERS: &str =
        "INTEGRATION_PROXY_CONFIG_ASSIGNED_ONE_MORE_INDEXERS";
    /// Proxy config is assigned to one or more download clients.
    pub const PROXY_CONFIG_ASSIGNED_ONE_MORE_DOWNLOAD_CLIENTS: &str =
        "INTEGRATION_PROXY_CONFIG_ASSIGNED_ONE_MORE_DOWNLOAD_CLIENTS";
    /// Only SSH tunnel proxies pin a host key.
    pub const ONLY_SSH_TUNNEL_PROXIES_PIN_HOST_KEY: &str =
        "INTEGRATION_ONLY_SSH_TUNNEL_PROXIES_PIN_HOST_KEY";
    /// WireGuard tunnels have no host key to reset.
    pub const WIREGUARD_TUNNELS_NO_HOST_KEY_RESET: &str =
        "INTEGRATION_WIREGUARD_TUNNELS_NO_HOST_KEY_RESET";
    /// Enable this proxy before testing its outbound route.
    pub const ENABLE_PROXY_BEFORE_TESTING_OUTBOUND_ROUTE: &str =
        "INTEGRATION_ENABLE_PROXY_BEFORE_TESTING_OUTBOUND_ROUTE";
    /// Assign an indexer to test HTTP/3 CONNECT through its destination.
    pub const ASSIGN_INDEXER_TEST_HTTP_3_CONNECT_THROUGH_DESTINATION: &str =
        "INTEGRATION_ASSIGN_INDEXER_TEST_HTTP_3_CONNECT_THROUGH_DESTINATION";
    /// Test URL must use HTTP or HTTPS and contain no credentials.
    pub const TEST_URL_INVALID: &str = "INTEGRATION_TEST_URL_INVALID";
    /// Proxy name is required.
    pub const PROXY_NAME_REQUIRED: &str = "INTEGRATION_PROXY_NAME_REQUIRED";
    /// Proxy base URL is required.
    pub const PROXY_BASE_URL_REQUIRED: &str = "INTEGRATION_PROXY_BASE_URL_REQUIRED";
    /// Proxy base URL must include a port.
    pub const PROXY_BASE_URL_INVALID: &str = "INTEGRATION_PROXY_BASE_URL_INVALID";
    /// Proxy base URL must include a host.
    pub const PROXY_BASE_URL_HOST_REQUIRED: &str = "INTEGRATION_PROXY_BASE_URL_HOST_REQUIRED";
    /// Proxy base URL must not embed credentials; use the username and password fields.
    pub const PROXY_URL_CREDENTIALS_FORBIDDEN: &str = "INTEGRATION_PROXY_URL_CREDENTIALS_FORBIDDEN";
    /// HTTP/3 CONNECT endpoint must be https://host[:port] without a path, query or fragment.
    pub const HTTP_3_CONNECT_ENDPOINT_INVALID: &str = "INTEGRATION_HTTP_3_CONNECT_ENDPOINT_INVALID";
    /// HTTP/3 CONNECT requires an https endpoint.
    pub const HTTP_3_CONNECT_REQUIRES_HTTPS_ENDPOINT: &str =
        "INTEGRATION_HTTP_3_CONNECT_REQUIRES_HTTPS_ENDPOINT";
    /// SSH tunnel base URL must use ssh.
    pub const SSH_TUNNEL_BASE_URL_INVALID: &str = "INTEGRATION_SSH_TUNNEL_BASE_URL_INVALID";
    /// WireGuard base URL must use wireguard.
    pub const WIREGUARD_BASE_URL_INVALID: &str = "INTEGRATION_WIREGUARD_BASE_URL_INVALID";
    /// HTTP proxy base URL must use http or https.
    pub const HTTP_PROXY_BASE_URL_INVALID: &str = "INTEGRATION_HTTP_PROXY_BASE_URL_INVALID";
    /// SOCKS4 proxy base URL must use socks4 or socks4a.
    pub const SOCKS4_PROXY_BASE_URL_INVALID: &str = "INTEGRATION_SOCKS4_PROXY_BASE_URL_INVALID";
    /// SOCKS5 proxy base URL must use socks5 or socks5h.
    pub const SOCKS5_PROXY_BASE_URL_INVALID: &str = "INTEGRATION_SOCKS5_PROXY_BASE_URL_INVALID";
    /// The requested value proxies do not use a challenge-solver protocol.
    pub const PROXIES_DO_NOT_USE_CHALLENGE_SOLVER_PROTOCOL: &str =
        "INTEGRATION_PROXIES_DO_NOT_USE_CHALLENGE_SOLVER_PROTOCOL";
    /// Challenge-solver proxies do not accept a username or password.
    pub const CHALLENGE_SOLVER_PROXIES_DO_NOT_ACCEPT_USERNAME_PASSWORD: &str =
        "INTEGRATION_CHALLENGE_SOLVER_PROXIES_DO_NOT_ACCEPT_USERNAME_PASSWORD";
    /// SOCKS4 proxies do not carry credentials; use SOCKS5 for an authenticated proxy.
    pub const SOCKS4_PROXIES_DO_NOT_CARRY_CREDENTIALS: &str =
        "INTEGRATION_SOCKS4_PROXIES_DO_NOT_CARRY_CREDENTIALS";
    /// Proxy password requires a username.
    pub const PROXY_PASSWORD_REQUIRES_USERNAME: &str =
        "INTEGRATION_PROXY_PASSWORD_REQUIRES_USERNAME";
    /// Private key must be a PEM block beginning with -----BEGIN and ending with -----.
    pub const PRIVATE_KEY_INVALID: &str = "INTEGRATION_PRIVATE_KEY_INVALID";
    /// Only WireGuard tunnels take a peer public key, preshared key, tunnel addresses, DNS servers, MTU or keepalive.
    pub const WIREGUARD_FIELDS_WRONG_TUNNEL_KIND: &str =
        "INTEGRATION_WIREGUARD_FIELDS_WRONG_TUNNEL_KIND";
    /// Only SSH tunnels take a private key passphrase.
    pub const ONLY_SSH_TUNNELS_TAKE_PRIVATE_KEY_PASSPHRASE: &str =
        "INTEGRATION_ONLY_SSH_TUNNELS_TAKE_PRIVATE_KEY_PASSPHRASE";
    /// Only tunnels take a private key.
    pub const ONLY_TUNNELS_TAKE_PRIVATE_KEY: &str = "INTEGRATION_ONLY_TUNNELS_TAKE_PRIVATE_KEY";
    /// SSH tunnels require a username.
    pub const SSH_TUNNELS_REQUIRE_USERNAME: &str = "INTEGRATION_SSH_TUNNELS_REQUIRE_USERNAME";
    /// SSH tunnels do not accept passwords; use an Ed25519 private key.
    pub const SSH_TUNNELS_DO_NOT_ACCEPT_PASSWORDS: &str =
        "INTEGRATION_SSH_TUNNELS_DO_NOT_ACCEPT_PASSWORDS";
    /// SSH tunnels require an Ed25519 private key.
    pub const SSH_TUNNELS_REQUIRE_ED25519_PRIVATE_KEY: &str =
        "INTEGRATION_SSH_TUNNELS_REQUIRE_ED25519_PRIVATE_KEY";
    /// WireGuard tunnels authenticate with keys, not a username or password.
    pub const WIREGUARD_TUNNELS_AUTHENTICATE_KEYS_NOT_USERNAME_PASSWORD: &str =
        "INTEGRATION_WIREGUARD_TUNNELS_AUTHENTICATE_KEYS_NOT_USERNAME_PASSWORD";
    /// WireGuard tunnels require a private key; the selected value.
    pub const WIREGUARD_TUNNELS_REQUIRE_PRIVATE_KEY: &str =
        "INTEGRATION_WIREGUARD_TUNNELS_REQUIRE_PRIVATE_KEY";
    /// WireGuard tunnels require the peer's public key from the `[Peer]` section.
    pub const WIREGUARD_TUNNELS_REQUIRE_PEER_S_PUBLIC_KEY_PEER_SECTION: &str =
        "INTEGRATION_WIREGUARD_TUNNELS_REQUIRE_PEER_S_PUBLIC_KEY_PEER_SECTION";
    /// WireGuard tunnels require at least one interface address; copy the `Address` line from the WireGuard configuration (for example `10.6.0.2/32`).
    pub const WIREGUARD_ADDRESS_REQUIRED: &str = "INTEGRATION_WIREGUARD_ADDRESS_REQUIRED";
    /// `the selected value` is not a DNS server address; the `DNS` line takes IP addresses.
    pub const NOT_DNS_SERVER_ADDRESS: &str = "INTEGRATION_NOT_DNS_SERVER_ADDRESS";
    /// The tunnel MTU must be between the selected value and the selected value; the selected value is outside that range.
    pub const TUNNEL_MTU_OUT_OF_RANGE: &str = "INTEGRATION_TUNNEL_MTU_OUT_OF_RANGE";
    /// Socks5h:// and socks4a:// resolve names at the proxy; use socks5:// or socks4:// to resolve them locally.
    pub const SOCKS5H_SOCKS4A_RESOLVE_NAMES_PROXY: &str =
        "INTEGRATION_SOCKS5H_SOCKS4A_RESOLVE_NAMES_PROXY";
    /// Remote DNS applies only to SOCKS proxies.
    pub const REMOTE_DNS_APPLIES_ONLY_SOCKS_PROXIES: &str =
        "INTEGRATION_REMOTE_DNS_APPLIES_ONLY_SOCKS_PROXIES";
    /// Proxy timeout must be between 1 and the selected value seconds.
    pub const PROXY_TIMEOUT_OUT_OF_RANGE: &str = "INTEGRATION_PROXY_TIMEOUT_OUT_OF_RANGE";
    /// Download already resolved as the selected value; nothing to ignore.
    pub const DOWNLOAD_ALREADY_RESOLVED: &str = "INTEGRATION_DOWNLOAD_ALREADY_RESOLVED";
    /// Tracked download was not found.
    pub const TRACKED_DOWNLOAD_NOT_FOUND: &str = "INTEGRATION_TRACKED_DOWNLOAD_NOT_FOUND";
    /// Tracked download the selected value is busy processing.
    pub const TRACKED_DOWNLOAD_BUSY_PROCESSING: &str =
        "INTEGRATION_TRACKED_DOWNLOAD_BUSY_PROCESSING";
    /// Max queries per minute must be at least 1, or empty for no budget.
    pub const MAX_QUERIES_PER_MINUTE_OUT_OF_RANGE: &str =
        "INTEGRATION_MAX_QUERIES_PER_MINUTE_OUT_OF_RANGE";
    /// Max queries per minute is set on each managed indexer, not on the indexer that syncs them.
    pub const MAX_QUERIES_PER_MINUTE_SET_EACH_MANAGED_INDEXER_NOT_INDEXER_SYNCS_THEM: &str =
        "INTEGRATION_MAX_QUERIES_PER_MINUTE_SET_EACH_MANAGED_INDEXER_NOT_INDEXER_SYNCS_THEM";
    /// Indexer config_json must be a JSON object.
    pub const INDEXER_CONFIG_JSON_INVALID: &str = "INTEGRATION_INDEXER_CONFIG_JSON_INVALID";
    /// Indexer provider declares multiple connection_url config fields.
    pub const INDEXER_PROVIDER_DECLARES_MULTIPLE_CONNECTION_URL_CONFIG_FIELDS: &str =
        "INTEGRATION_INDEXER_PROVIDER_DECLARES_MULTIPLE_CONNECTION_URL_CONFIG_FIELDS";
    /// Indexer connection URL is required.
    pub const INDEXER_CONNECTION_URL_REQUIRED: &str = "INTEGRATION_INDEXER_CONNECTION_URL_REQUIRED";
    /// The requested value is required.
    pub const FIELD_REQUIRED: &str = "INTEGRATION_FIELD_REQUIRED";
    /// Unsupported indexer provider type the selected item.
    pub const INDEXER_PROVIDER_TYPE_UNSUPPORTED: &str =
        "INTEGRATION_INDEXER_PROVIDER_TYPE_UNSUPPORTED";
    /// Indexer name cannot be empty.
    pub const INDEXER_NAME_REQUIRED: &str = "INTEGRATION_INDEXER_NAME_REQUIRED";
    /// Provider type cannot be empty.
    pub const PROVIDER_TYPE_REQUIRED: &str = "INTEGRATION_PROVIDER_TYPE_REQUIRED";
    /// Indexer config id is required.
    pub const INDEXER_CONFIG_ID_REQUIRED: &str = "INTEGRATION_INDEXER_CONFIG_ID_REQUIRED";
    /// At least one indexer field must be provided.
    pub const INDEXER_FIELD_REQUIRED: &str = "INTEGRATION_INDEXER_FIELD_REQUIRED";
    /// Managed child indexers are controlled by their parent sync and cannot be edited directly.
    pub const MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC_CANNOT_EDITED_DIRECTLY: &str =
        "INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC_CANNOT_EDITED_DIRECTLY";
    /// Managed indexers cannot use a proxy; the managing application owns challenge solving.
    pub const MANAGED_INDEXERS_CANNOT_USE_PROXY: &str =
        "INTEGRATION_MANAGED_INDEXERS_CANNOT_USE_PROXY";
    /// Mapped download client the selected item no longer exists.
    pub const MAPPED_DOWNLOAD_CLIENT_NO_LONGER_EXISTS: &str =
        "INTEGRATION_MAPPED_DOWNLOAD_CLIENT_NO_LONGER_EXISTS";
    /// Managed child indexers are controlled by their parent sync.
    pub const MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC: &str =
        "INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC";
    /// Managed child indexers cannot be synced directly.
    pub const MANAGED_CHILD_INDEXERS_CANNOT_SYNCED_DIRECTLY: &str =
        "INTEGRATION_MANAGED_CHILD_INDEXERS_CANNOT_SYNCED_DIRECTLY";
    /// Managed child indexer requires a parent.
    pub const MANAGED_CHILD_INDEXER_REQUIRES_PARENT: &str =
        "INTEGRATION_MANAGED_CHILD_INDEXER_REQUIRES_PARENT";
    /// Managed child indexer requires a child key.
    pub const MANAGED_CHILD_INDEXER_REQUIRES_CHILD_KEY: &str =
        "INTEGRATION_MANAGED_CHILD_INDEXER_REQUIRES_CHILD_KEY";
    /// Prowlarr management parents cannot be mapped to a download client.
    pub const PROWLARR_MANAGEMENT_PARENTS_CANNOT_MAPPED_DOWNLOAD_CLIENT: &str =
        "INTEGRATION_PROWLARR_MANAGEMENT_PARENTS_CANNOT_MAPPED_DOWNLOAD_CLIENT";
    /// Download client the selected item does not support all protocol families required by indexer the selected item.
    pub const DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_ALL_PROTOCOL_FAMILIES_REQUIRED_INDEXER: &str =
        "INTEGRATION_DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_ALL_PROTOCOL_FAMILIES_REQUIRED_INDEXER";
    /// Managed child routing scope the selected item is not supported.
    pub const MANAGED_CHILD_ROUTING_SCOPE_UNSUPPORTED: &str =
        "INTEGRATION_MANAGED_CHILD_ROUTING_SCOPE_UNSUPPORTED";
    /// Managed child routing contains duplicate scope the selected item.
    pub const MANAGED_CHILD_ROUTING_CONTAINS_DUPLICATE_SCOPE: &str =
        "INTEGRATION_MANAGED_CHILD_ROUTING_CONTAINS_DUPLICATE_SCOPE";
    /// Managed child plan entries require child_key.
    pub const MANAGED_CHILD_PLAN_ENTRIES_REQUIRE_CHILD_KEY: &str =
        "INTEGRATION_MANAGED_CHILD_PLAN_ENTRIES_REQUIRE_CHILD_KEY";
    /// Managed child plan contains duplicate child_key the selected item.
    pub const MANAGED_CHILD_PLAN_CONTAINS_DUPLICATE_CHILD_KEY: &str =
        "INTEGRATION_MANAGED_CHILD_PLAN_CONTAINS_DUPLICATE_CHILD_KEY";
    /// Managed child the selected item requires a name.
    pub const MANAGED_CHILD_REQUIRES_NAME: &str = "INTEGRATION_MANAGED_CHILD_REQUIRES_NAME";
    /// Managed child the selected item requires provider_type.
    pub const MANAGED_CHILD_REQUIRES_PROVIDER_TYPE: &str =
        "INTEGRATION_MANAGED_CHILD_REQUIRES_PROVIDER_TYPE";
    /// This download is already being processed.
    pub const DOWNLOAD_ALREADY_BEING_PROCESSED: &str =
        "INTEGRATION_DOWNLOAD_ALREADY_BEING_PROCESSED";
    /// Held sources could not be settled.
    pub const HELD_SOURCES_COULD_NOT_SETTLED: &str = "INTEGRATION_HELD_SOURCES_COULD_NOT_SETTLED";
    /// The indexer configuration is invalid.
    pub const INDEXER_CONFIG_INVALID: &str = "INTEGRATION_INDEXER_CONFIG_INVALID";
    /// The indexer could not be reached.
    pub const INDEXER_CONNECTION_UNREACHABLE: &str = "INTEGRATION_INDEXER_CONNECTION_UNREACHABLE";
    /// The indexer rejected the request.
    pub const INDEXER_REQUEST_REJECTED: &str = "INTEGRATION_INDEXER_REQUEST_REJECTED";
    /// The download client configuration is invalid.
    pub const DOWNLOAD_CLIENT_CONFIG_INVALID: &str = "INTEGRATION_DOWNLOAD_CLIENT_CONFIG_INVALID";
    /// The proxy credentials are invalid.
    pub const PROXY_CREDENTIALS_INVALID: &str = "INTEGRATION_PROXY_CREDENTIALS_INVALID";
    /// The SSH private key could not be read.
    pub const SSH_PRIVATE_KEY_INVALID: &str = "INTEGRATION_SSH_PRIVATE_KEY_INVALID";
    /// The WireGuard keys are invalid.
    pub const WIREGUARD_KEYS_INVALID: &str = "INTEGRATION_WIREGUARD_KEYS_INVALID";
    /// The WireGuard interface address is invalid.
    pub const WIREGUARD_ADDRESS_INVALID: &str = "INTEGRATION_WIREGUARD_ADDRESS_INVALID";
    /// The seeding profile is invalid.
    pub const SEEDING_PROFILE_INVALID: &str = "INTEGRATION_SEEDING_PROFILE_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SEEDING_PROFILE_INVALID,
        INDEXER_PROVIDER_UNAVAILABLE,
        PLUGIN_RETURNED_INVALID_CONFIG_FIELDS,
        INDEXER_SETTINGS_CHANGED_DURING_VALIDATION,
        NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE,
        NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE,
        INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT,
        API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY,
        PROXY_CONFIGURATION_NOT_FOUND,
        PROXY_DISABLED_INDEXER,
        PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS,
        PROVIDER_TYPE_DOES_NOT_SUPPORT_MANAGED_CHILD_SYNC,
        INDEXER_CONNECTION_TEST_FAILED,
        BASE_URL_INVALID,
        BASE_URL_HOST_REQUIRED,
        BASE_URL_CREDENTIALS_FORBIDDEN,
        DOWNLOAD_SUBMISSION_NOT_FOUND,
        DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_PASSWORD_RETRIES,
        PASSWORD_RETRY_REQUIRES_EXACT_DOWNLOAD_CLIENT,
        DOWNLOAD_UNAVAILABLE,
        SEEDING_PROFILE_ID_REQUIRED,
        SEEDING_PROFILE_FIELD_REQUIRED,
        SEEDING_PROFILE_NOT_FOUND,
        SEEDING_PROFILE_STILL_ASSIGNED,
        INDEXER_ID_REQUIRED,
        INDEXER_CONFIG_NOT_FOUND,
        PROWLARR_MANAGEMENT_PARENTS_CANNOT_ASSIGNED_SEEDING_PROFILE,
        INDEXER_PROVIDER_DOES_NOT_DECLARE_SUPPORTED_PROTOCOL_FAMILY,
        INDEXER_DOES_NOT_SUPPORT_TORRENTS_SO_SEEDING_PROFILE_CANNOT_ASSIGNED,
        MINIMUM_SEEDERS_FLOOR_CANNOT_NEGATIVE,
        LIBRARY_ACCESS_DENIED,
        CLIENT_TYPE_REQUIRED,
        DOWNLOAD_CLIENT_TYPE_UNSUPPORTED,
        ACTION_PERMISSION_DENIED,
        CLIENT_ID_REQUIRED,
        PROXY_CONFIG_ID_REQUIRED,
        PROXY_DISABLED_DOWNLOAD_CLIENT,
        DOWNLOAD_CLIENT_NAME_REQUIRED,
        DOWNLOAD_CLIENT_FIELD_REQUIRED,
        CLIENT_NAME_REQUIRED,
        DOWNLOAD_CLIENT_CONFIG_NOT_FOUND,
        MANUAL_IMPORT_SELECTION_UNAVAILABLE,
        MANUAL_IMPORT_SELECTION_TITLE_UNAVAILABLE,
        MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD,
        MANUAL_IMPORT_FILES_UNAVAILABLE,
        MANUAL_IMPORT_SELECTION_CHANGED,
        MANUAL_IMPORT_CANDIDATE_UNAVAILABLE,
        DOWNLOAD_CLIENT_ITEM_ID_REQUIRED,
        DOWNLOAD_NOT_STATE_IMPORTED_HAND_RIGHT_NOW,
        STORED_PROXY_PROTOCOL_DOES_NOT_MATCH_PROVIDER,
        PROXY_FIELD_REQUIRED,
        PROXY_CONFIG_NOT_FOUND,
        PROXY_CONFIG_ASSIGNED_ENABLED_INDEXER_S,
        PROXY_CONFIG_ASSIGNED_ENABLED_DOWNLOAD_CLIENT_S,
        PROXY_CONFIG_ASSIGNED_ONE_MORE_INDEXERS,
        PROXY_CONFIG_ASSIGNED_ONE_MORE_DOWNLOAD_CLIENTS,
        ONLY_SSH_TUNNEL_PROXIES_PIN_HOST_KEY,
        WIREGUARD_TUNNELS_NO_HOST_KEY_RESET,
        ENABLE_PROXY_BEFORE_TESTING_OUTBOUND_ROUTE,
        ASSIGN_INDEXER_TEST_HTTP_3_CONNECT_THROUGH_DESTINATION,
        TEST_URL_INVALID,
        PROXY_NAME_REQUIRED,
        PROXY_BASE_URL_REQUIRED,
        PROXY_BASE_URL_INVALID,
        PROXY_BASE_URL_HOST_REQUIRED,
        PROXY_URL_CREDENTIALS_FORBIDDEN,
        HTTP_3_CONNECT_ENDPOINT_INVALID,
        HTTP_3_CONNECT_REQUIRES_HTTPS_ENDPOINT,
        SSH_TUNNEL_BASE_URL_INVALID,
        WIREGUARD_BASE_URL_INVALID,
        HTTP_PROXY_BASE_URL_INVALID,
        SOCKS4_PROXY_BASE_URL_INVALID,
        SOCKS5_PROXY_BASE_URL_INVALID,
        PROXIES_DO_NOT_USE_CHALLENGE_SOLVER_PROTOCOL,
        CHALLENGE_SOLVER_PROXIES_DO_NOT_ACCEPT_USERNAME_PASSWORD,
        SOCKS4_PROXIES_DO_NOT_CARRY_CREDENTIALS,
        PROXY_PASSWORD_REQUIRES_USERNAME,
        PRIVATE_KEY_INVALID,
        WIREGUARD_FIELDS_WRONG_TUNNEL_KIND,
        ONLY_SSH_TUNNELS_TAKE_PRIVATE_KEY_PASSPHRASE,
        ONLY_TUNNELS_TAKE_PRIVATE_KEY,
        SSH_TUNNELS_REQUIRE_USERNAME,
        SSH_TUNNELS_DO_NOT_ACCEPT_PASSWORDS,
        SSH_TUNNELS_REQUIRE_ED25519_PRIVATE_KEY,
        WIREGUARD_TUNNELS_AUTHENTICATE_KEYS_NOT_USERNAME_PASSWORD,
        WIREGUARD_TUNNELS_REQUIRE_PRIVATE_KEY,
        WIREGUARD_TUNNELS_REQUIRE_PEER_S_PUBLIC_KEY_PEER_SECTION,
        WIREGUARD_ADDRESS_REQUIRED,
        NOT_DNS_SERVER_ADDRESS,
        TUNNEL_MTU_OUT_OF_RANGE,
        SOCKS5H_SOCKS4A_RESOLVE_NAMES_PROXY,
        REMOTE_DNS_APPLIES_ONLY_SOCKS_PROXIES,
        PROXY_TIMEOUT_OUT_OF_RANGE,
        DOWNLOAD_ALREADY_RESOLVED,
        TRACKED_DOWNLOAD_NOT_FOUND,
        TRACKED_DOWNLOAD_BUSY_PROCESSING,
        MAX_QUERIES_PER_MINUTE_OUT_OF_RANGE,
        MAX_QUERIES_PER_MINUTE_SET_EACH_MANAGED_INDEXER_NOT_INDEXER_SYNCS_THEM,
        INDEXER_CONFIG_JSON_INVALID,
        INDEXER_PROVIDER_DECLARES_MULTIPLE_CONNECTION_URL_CONFIG_FIELDS,
        INDEXER_CONNECTION_URL_REQUIRED,
        FIELD_REQUIRED,
        INDEXER_PROVIDER_TYPE_UNSUPPORTED,
        INDEXER_NAME_REQUIRED,
        PROVIDER_TYPE_REQUIRED,
        INDEXER_CONFIG_ID_REQUIRED,
        INDEXER_FIELD_REQUIRED,
        MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC_CANNOT_EDITED_DIRECTLY,
        MANAGED_INDEXERS_CANNOT_USE_PROXY,
        MAPPED_DOWNLOAD_CLIENT_NO_LONGER_EXISTS,
        MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC,
        MANAGED_CHILD_INDEXERS_CANNOT_SYNCED_DIRECTLY,
        MANAGED_CHILD_INDEXER_REQUIRES_PARENT,
        MANAGED_CHILD_INDEXER_REQUIRES_CHILD_KEY,
        PROWLARR_MANAGEMENT_PARENTS_CANNOT_MAPPED_DOWNLOAD_CLIENT,
        DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_ALL_PROTOCOL_FAMILIES_REQUIRED_INDEXER,
        MANAGED_CHILD_ROUTING_SCOPE_UNSUPPORTED,
        MANAGED_CHILD_ROUTING_CONTAINS_DUPLICATE_SCOPE,
        MANAGED_CHILD_PLAN_ENTRIES_REQUIRE_CHILD_KEY,
        MANAGED_CHILD_PLAN_CONTAINS_DUPLICATE_CHILD_KEY,
        MANAGED_CHILD_REQUIRES_NAME,
        MANAGED_CHILD_REQUIRES_PROVIDER_TYPE,
        DOWNLOAD_ALREADY_BEING_PROCESSED,
        HELD_SOURCES_COULD_NOT_SETTLED,
        INDEXER_CONFIG_INVALID,
        INDEXER_CONNECTION_UNREACHABLE,
        INDEXER_REQUEST_REJECTED,
        DOWNLOAD_CLIENT_CONFIG_INVALID,
        PROXY_CREDENTIALS_INVALID,
        SSH_PRIVATE_KEY_INVALID,
        WIREGUARD_KEYS_INVALID,
        WIREGUARD_ADDRESS_INVALID,
    ];
}

pub mod discovery {
    /// Pending discovery removal the selected value is missing subject key.
    pub const PENDING_DISCOVERY_REMOVAL_MISSING_SUBJECT_KEY: &str =
        "DISCOVERY_PENDING_DISCOVERY_REMOVAL_MISSING_SUBJECT_KEY";
    /// Pending discovery rematch the selected value is missing previous subject key.
    pub const PENDING_DISCOVERY_REMATCH_MISSING_PREVIOUS_SUBJECT_KEY: &str =
        "DISCOVERY_PENDING_DISCOVERY_REMATCH_MISSING_PREVIOUS_SUBJECT_KEY";
    /// Pending discovery change the selected value is missing subject key.
    pub const PENDING_DISCOVERY_CHANGE_MISSING_SUBJECT_KEY: &str =
        "DISCOVERY_PENDING_DISCOVERY_CHANGE_MISSING_SUBJECT_KEY";
    /// Pending discovery change the selected value is missing raw subject JSON.
    pub const PENDING_DISCOVERY_CHANGE_MISSING_RAW_SUBJECT_JSON: &str =
        "DISCOVERY_PENDING_DISCOVERY_CHANGE_MISSING_RAW_SUBJECT_JSON";
    /// Pending discovery change the selected value has invalid raw subject JSON.
    pub const PENDING_DISCOVERY_CHANGE_INVALID_RAW_SUBJECT_JSON: &str =
        "DISCOVERY_PENDING_DISCOVERY_CHANGE_INVALID_RAW_SUBJECT_JSON";
    /// Pending discovery change the selected value has invalid previous subject JSON.
    pub const PENDING_DISCOVERY_CHANGE_INVALID_PREVIOUS_SUBJECT_JSON: &str =
        "DISCOVERY_PENDING_DISCOVERY_CHANGE_INVALID_PREVIOUS_SUBJECT_JSON";
    /// Unsupported discovery context change type the selected value.
    pub const CONTEXT_CHANGE_TYPE_UNSUPPORTED: &str = "DISCOVERY_CONTEXT_CHANGE_TYPE_UNSUPPORTED";
    /// Discovery home hero was not found.
    pub const HOME_HERO_NOT_FOUND: &str = "DISCOVERY_HOME_HERO_NOT_FOUND";
    /// Discovery incremental reload resolves to the selected value changed subjects, above SMG limit the selected value.
    pub const INCREMENTAL_RELOAD_RESOLVES_CHANGED_SUBJECTS_ABOVE_SMG_LIMIT: &str =
        "DISCOVERY_INCREMENTAL_RELOAD_RESOLVES_CHANGED_SUBJECTS_ABOVE_SMG_LIMIT";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        PENDING_DISCOVERY_REMOVAL_MISSING_SUBJECT_KEY,
        PENDING_DISCOVERY_REMATCH_MISSING_PREVIOUS_SUBJECT_KEY,
        PENDING_DISCOVERY_CHANGE_MISSING_SUBJECT_KEY,
        PENDING_DISCOVERY_CHANGE_MISSING_RAW_SUBJECT_JSON,
        PENDING_DISCOVERY_CHANGE_INVALID_RAW_SUBJECT_JSON,
        PENDING_DISCOVERY_CHANGE_INVALID_PREVIOUS_SUBJECT_JSON,
        CONTEXT_CHANGE_TYPE_UNSUPPORTED,
        HOME_HERO_NOT_FOUND,
        INCREMENTAL_RELOAD_RESOLVES_CHANGED_SUBJECTS_ABOVE_SMG_LIMIT,
    ];
}

pub mod application_upgrade {
    /// Invalid installer digest.
    pub const INSTALLER_DIGEST_INVALID: &str = "APPLICATION_UPGRADE_INSTALLER_DIGEST_INVALID";
    /// ExpectedTag must not be empty.
    pub const EXPECTEDTAG_REQUIRED: &str = "APPLICATION_UPGRADE_EXPECTEDTAG_REQUIRED";
    /// ExpectedVersion must not be empty.
    pub const EXPECTEDVERSION_REQUIRED: &str = "APPLICATION_UPGRADE_EXPECTEDVERSION_REQUIRED";
    /// No application update notice is available.
    pub const NO_APPLICATION_UPDATE_NOTICE_UNAVAILABLE: &str =
        "APPLICATION_UPGRADE_NO_APPLICATION_UPDATE_NOTICE_UNAVAILABLE";
    /// The application update notice is not available.
    pub const APPLICATION_UPDATE_NOTICE_UNAVAILABLE: &str =
        "APPLICATION_UPGRADE_APPLICATION_UPDATE_NOTICE_UNAVAILABLE";
    /// ExpectedTag does not match the current application update notice.
    pub const EXPECTEDTAG_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE: &str =
        "APPLICATION_UPGRADE_EXPECTEDTAG_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE";
    /// ExpectedVersion does not match the current application update notice.
    pub const EXPECTEDVERSION_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE: &str =
        "APPLICATION_UPGRADE_EXPECTEDVERSION_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE";
    /// ExpectedVersion must be strictly newer than the running version.
    pub const EXPECTEDVERSION_INVALID: &str = "APPLICATION_UPGRADE_EXPECTEDVERSION_INVALID";
    /// Application upgrade installation is not eligible.
    pub const INSTALLATION_UNAVAILABLE: &str = "APPLICATION_UPGRADE_INSTALLATION_UNAVAILABLE";
    /// An application upgrade job is already running.
    pub const JOB_ALREADY_RUNNING: &str = "APPLICATION_UPGRADE_JOB_ALREADY_RUNNING";
    /// Upgrade manifest tag does not match expectedTag.
    pub const UPGRADE_MANIFEST_TAG_DOES_NOT_MATCH_EXPECTEDTAG: &str =
        "APPLICATION_UPGRADE_UPGRADE_MANIFEST_TAG_DOES_NOT_MATCH_EXPECTEDTAG";
    /// Upgrade manifest version does not match expectedVersion.
    pub const UPGRADE_MANIFEST_VERSION_DOES_NOT_MATCH_EXPECTEDVERSION: &str =
        "APPLICATION_UPGRADE_UPGRADE_MANIFEST_VERSION_DOES_NOT_MATCH_EXPECTEDVERSION";
    /// Running executable has no parent directory.
    pub const RUNNING_EXECUTABLE_NO_PARENT_DIRECTORY: &str =
        "APPLICATION_UPGRADE_RUNNING_EXECUTABLE_NO_PARENT_DIRECTORY";
    /// Refusing to overwrite existing application backup the selected item.
    pub const BACKUP_ALREADY_EXISTS: &str = "APPLICATION_UPGRADE_BACKUP_ALREADY_EXISTS";
    /// Unsupported application upgrade journal schema the selected item.
    pub const JOURNAL_SCHEMA_UNSUPPORTED: &str = "APPLICATION_UPGRADE_JOURNAL_SCHEMA_UNSUPPORTED";
    /// Unsupported application upgrade journal phase the selected item.
    pub const JOURNAL_PHASE_UNSUPPORTED: &str = "APPLICATION_UPGRADE_JOURNAL_PHASE_UNSUPPORTED";
    /// Application upgrade run was not found.
    pub const RUN_NOT_FOUND: &str = "APPLICATION_UPGRADE_RUN_NOT_FOUND";
    /// Insufficient free space for application upgrade: need the selected value bytes, have the selected value bytes.
    pub const INSUFFICIENT_FREE_SPACE_APPLICATION_UPGRADE: &str =
        "APPLICATION_UPGRADE_INSUFFICIENT_FREE_SPACE_APPLICATION_UPGRADE";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        INSTALLER_DIGEST_INVALID,
        EXPECTEDTAG_REQUIRED,
        EXPECTEDVERSION_REQUIRED,
        NO_APPLICATION_UPDATE_NOTICE_UNAVAILABLE,
        APPLICATION_UPDATE_NOTICE_UNAVAILABLE,
        EXPECTEDTAG_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE,
        EXPECTEDVERSION_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE,
        EXPECTEDVERSION_INVALID,
        INSTALLATION_UNAVAILABLE,
        JOB_ALREADY_RUNNING,
        UPGRADE_MANIFEST_TAG_DOES_NOT_MATCH_EXPECTEDTAG,
        UPGRADE_MANIFEST_VERSION_DOES_NOT_MATCH_EXPECTEDVERSION,
        RUNNING_EXECUTABLE_NO_PARENT_DIRECTORY,
        BACKUP_ALREADY_EXISTS,
        JOURNAL_SCHEMA_UNSUPPORTED,
        JOURNAL_PHASE_UNSUPPORTED,
        RUN_NOT_FOUND,
        INSUFFICIENT_FREE_SPACE_APPLICATION_UPGRADE,
    ];
}

pub mod subtitles {
    /// Invalid subtitle language code.
    pub const SUBTITLE_LANGUAGE_CODE_INVALID: &str = "SUBTITLES_SUBTITLE_LANGUAGE_CODE_INVALID";
    /// Unsupported subtitle format.
    pub const SUBTITLE_FORMAT_UNSUPPORTED: &str = "SUBTITLES_SUBTITLE_FORMAT_UNSUPPORTED";
    /// Subtitle provider config name must not be empty.
    pub const SUBTITLE_PROVIDER_CONFIG_NAME_REQUIRED: &str =
        "SUBTITLES_SUBTITLE_PROVIDER_CONFIG_NAME_REQUIRED";
    /// Subtitle provider type must not be empty.
    pub const SUBTITLE_PROVIDER_TYPE_REQUIRED: &str = "SUBTITLES_SUBTITLE_PROVIDER_TYPE_REQUIRED";
    /// At least one subtitle provider config field must be provided.
    pub const SUBTITLE_PROVIDER_CONFIG_FIELD_REQUIRED: &str =
        "SUBTITLES_SUBTITLE_PROVIDER_CONFIG_FIELD_REQUIRED";
    /// Subtitle provider config was not found.
    pub const SUBTITLE_PROVIDER_CONFIG_NOT_FOUND: &str =
        "SUBTITLES_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND";
    /// No subtitle plugin for provider type was not found.
    pub const SUBTITLE_PLUGIN_PROVIDER_TYPE_NOT_FOUND: &str =
        "SUBTITLES_SUBTITLE_PLUGIN_PROVIDER_TYPE_NOT_FOUND";
    /// Subtitle provider config_json must be a JSON object.
    pub const SUBTITLE_PROVIDER_CONFIG_JSON_INVALID: &str =
        "SUBTITLES_SUBTITLE_PROVIDER_CONFIG_JSON_INVALID";
    /// File too small for subtitle file hash.
    pub const FILE_TOO_SMALL_SUBTITLE_FILE_HASH: &str =
        "SUBTITLES_FILE_TOO_SMALL_SUBTITLE_FILE_HASH";
    /// Unsupported subtitle artifact format.
    pub const SUBTITLE_ARTIFACT_FORMAT_UNSUPPORTED: &str =
        "SUBTITLES_SUBTITLE_ARTIFACT_FORMAT_UNSUPPORTED";
    /// Subtitle sidecar must not be a symlink.
    pub const SIDECAR_SYMLINK_FORBIDDEN: &str = "SUBTITLES_SIDECAR_SYMLINK_FORBIDDEN";
    /// Subtitle sidecar must be a regular file.
    pub const SUBTITLE_SIDECAR_INVALID: &str = "SUBTITLES_SUBTITLE_SIDECAR_INVALID";
    /// Subtitle sidecar exceeds content probe size limit.
    pub const SUBTITLE_SIDECAR_EXCEEDS_CONTENT_PROBE_SIZE_LIMIT: &str =
        "SUBTITLES_SUBTITLE_SIDECAR_EXCEEDS_CONTENT_PROBE_SIZE_LIMIT";
    /// Subtitle sidecar changed during content probe.
    pub const SUBTITLE_SIDECAR_CHANGED_DURING_CONTENT_PROBE: &str =
        "SUBTITLES_SUBTITLE_SIDECAR_CHANGED_DURING_CONTENT_PROBE";
    /// No enabled subtitle providers are configured.
    pub const NO_ENABLED_SUBTITLE_PROVIDERS_NOT_CONFIGURED: &str =
        "SUBTITLES_NO_ENABLED_SUBTITLE_PROVIDERS_NOT_CONFIGURED";
    /// Subtitle provider  is not configured was not found.
    pub const SUBTITLE_PROVIDER_NOT_NOT_FOUND: &str = "SUBTITLES_SUBTITLE_PROVIDER_NOT_NOT_FOUND";
    /// Media file was not found.
    pub const MEDIA_FILE_NOT_FOUND: &str = "SUBTITLES_MEDIA_FILE_NOT_FOUND";
    /// Title was not found.
    pub const TITLE_NOT_FOUND: &str = "SUBTITLES_TITLE_NOT_FOUND";
    /// Subtitle archive did not contain a supported subtitle file.
    pub const ARCHIVE_SUBTITLE_NOT_FOUND: &str = "SUBTITLES_ARCHIVE_SUBTITLE_NOT_FOUND";
    /// Subtitle artifact exceeds nesting or input limits.
    pub const SUBTITLE_ARTIFACT_EXCEEDS_NESTING_INPUT_LIMITS: &str =
        "SUBTITLES_SUBTITLE_ARTIFACT_EXCEEDS_NESTING_INPUT_LIMITS";
    /// Subtitle artifacts contain too many files.
    pub const SUBTITLE_ARTIFACTS_CONTAIN_TOO_MANY_FILES: &str =
        "SUBTITLES_SUBTITLE_ARTIFACTS_CONTAIN_TOO_MANY_FILES";
    /// Archive plugin does not support this subtitle archive format.
    pub const ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_SUBTITLE_ARCHIVE_FORMAT: &str =
        "SUBTITLES_ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_SUBTITLE_ARCHIVE_FORMAT";
    /// Subtitle archive has multiple equally ranked subtitle files.
    pub const SUBTITLE_ARCHIVE_MULTIPLE_EQUALLY_RANKED_SUBTITLE_FILES: &str =
        "SUBTITLES_SUBTITLE_ARCHIVE_MULTIPLE_EQUALLY_RANKED_SUBTITLE_FILES";
    /// Subtitle archive contains too many files.
    pub const SUBTITLE_ARCHIVE_CONTAINS_TOO_MANY_FILES: &str =
        "SUBTITLES_SUBTITLE_ARCHIVE_CONTAINS_TOO_MANY_FILES";
    /// Archive plugin output escaped output root.
    pub const ARCHIVE_PLUGIN_OUTPUT_ESCAPED_OUTPUT_ROOT: &str =
        "SUBTITLES_ARCHIVE_PLUGIN_OUTPUT_ESCAPED_OUTPUT_ROOT";
    /// Subtitle artifact expands beyond the the selected value byte limit.
    pub const SUBTITLE_ARTIFACT_EXPANDS_BEYOND_BYTE_LIMIT: &str =
        "SUBTITLES_SUBTITLE_ARTIFACT_EXPANDS_BEYOND_BYTE_LIMIT";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SUBTITLE_LANGUAGE_CODE_INVALID,
        SUBTITLE_FORMAT_UNSUPPORTED,
        SUBTITLE_PROVIDER_CONFIG_NAME_REQUIRED,
        SUBTITLE_PROVIDER_TYPE_REQUIRED,
        SUBTITLE_PROVIDER_CONFIG_FIELD_REQUIRED,
        SUBTITLE_PROVIDER_CONFIG_NOT_FOUND,
        SUBTITLE_PLUGIN_PROVIDER_TYPE_NOT_FOUND,
        SUBTITLE_PROVIDER_CONFIG_JSON_INVALID,
        FILE_TOO_SMALL_SUBTITLE_FILE_HASH,
        SUBTITLE_ARTIFACT_FORMAT_UNSUPPORTED,
        SIDECAR_SYMLINK_FORBIDDEN,
        SUBTITLE_SIDECAR_INVALID,
        SUBTITLE_SIDECAR_EXCEEDS_CONTENT_PROBE_SIZE_LIMIT,
        SUBTITLE_SIDECAR_CHANGED_DURING_CONTENT_PROBE,
        NO_ENABLED_SUBTITLE_PROVIDERS_NOT_CONFIGURED,
        SUBTITLE_PROVIDER_NOT_NOT_FOUND,
        MEDIA_FILE_NOT_FOUND,
        TITLE_NOT_FOUND,
        ARCHIVE_SUBTITLE_NOT_FOUND,
        SUBTITLE_ARTIFACT_EXCEEDS_NESTING_INPUT_LIMITS,
        SUBTITLE_ARTIFACTS_CONTAIN_TOO_MANY_FILES,
        ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_SUBTITLE_ARCHIVE_FORMAT,
        SUBTITLE_ARCHIVE_MULTIPLE_EQUALLY_RANKED_SUBTITLE_FILES,
        SUBTITLE_ARCHIVE_CONTAINS_TOO_MANY_FILES,
        ARCHIVE_PLUGIN_OUTPUT_ESCAPED_OUTPUT_ROOT,
        SUBTITLE_ARTIFACT_EXPANDS_BEYOND_BYTE_LIMIT,
    ];
}

pub mod jobs {
    /// Script the selected item is not a scheduled script.
    pub const SCRIPT_NOT_SCHEDULED_SCRIPT: &str = "JOB_SCRIPT_NOT_SCHEDULED_SCRIPT";
    /// Scheduled script the selected item is disabled.
    pub const SCHEDULED_SCRIPT_DISABLED: &str = "JOB_SCHEDULED_SCRIPT_DISABLED";
    /// Script was not found.
    pub const SCRIPT_NOT_FOUND: &str = "JOB_SCRIPT_NOT_FOUND";
    /// The selected item is already running.
    pub const ALREADY_RUNNING: &str = "JOB_ALREADY_RUNNING";
    /// This scheduled script is already running.
    pub const SCHEDULED_SCRIPT_ALREADY_RUNNING: &str = "JOB_SCHEDULED_SCRIPT_ALREADY_RUNNING";
    /// Custom job run does not name a script.
    pub const CUSTOM_JOB_RUN_DOES_NOT_NAME_SCRIPT: &str = "JOB_CUSTOM_JOB_RUN_DOES_NOT_NAME_SCRIPT";
    /// Scheduled script the selected value no longer exists.
    pub const SCHEDULED_SCRIPT_NO_LONGER_EXISTS: &str = "JOB_SCHEDULED_SCRIPT_NO_LONGER_EXISTS";
    /// You do not have permission to perform this action.
    pub const ACTION_PERMISSION_DENIED: &str = "JOB_ACTION_PERMISSION_DENIED";
    /// A custom job run needs the id of the scheduled script to run.
    pub const CUSTOM_JOB_RUN_NEEDS_ID_SCHEDULED_SCRIPT_RUN: &str =
        "JOB_CUSTOM_JOB_RUN_NEEDS_ID_SCHEDULED_SCRIPT_RUN";
    /// Full-hash backfill is shutting down.
    pub const FULL_HASH_BACKFILL_SHUTTING_DOWN: &str = "JOB_FULL_HASH_BACKFILL_SHUTTING_DOWN";
    /// Artwork encoding is shutting down.
    pub const ARTWORK_ENCODING_SHUTTING_DOWN: &str = "JOB_ARTWORK_ENCODING_SHUTTING_DOWN";
    /// The requested value can only run on its configured schedule.
    pub const ONLY_RUN_CONFIGURED_SCHEDULE: &str = "JOB_ONLY_RUN_CONFIGURED_SCHEDULE";
    /// The requested value has no libraries.
    pub const NO_LIBRARIES: &str = "JOB_NO_LIBRARIES";
    /// Background library refresh is temporarily disabled.
    pub const BACKGROUND_LIBRARY_REFRESH_TEMPORARILY_DISABLED: &str =
        "JOB_BACKGROUND_LIBRARY_REFRESH_TEMPORARILY_DISABLED";
    /// Title deletion jobs must be started from the title deletion mutation.
    pub const TITLE_DELETION_JOBS_INVALID: &str = "JOB_TITLE_DELETION_JOBS_INVALID";
    /// Title rename jobs must be started from the rename mutation.
    pub const TITLE_RENAME_JOBS_INVALID: &str = "JOB_TITLE_RENAME_JOBS_INVALID";
    /// Media file deletion jobs must be started from the media file deletion mutation.
    pub const MEDIA_FILE_DELETION_JOBS_INVALID: &str = "JOB_MEDIA_FILE_DELETION_JOBS_INVALID";
    /// Select files, seasons, or titles to refresh scores.
    pub const SELECT_FILES_SEASONS_TITLES_REFRESH_SCORES: &str =
        "JOB_SELECT_FILES_SEASONS_TITLES_REFRESH_SCORES";
    /// Recycle restore jobs must be started from the recycle restore mutation.
    pub const RECYCLE_RESTORE_JOBS_INVALID: &str = "JOB_RECYCLE_RESTORE_JOBS_INVALID";
    /// Recycle purge jobs must be started from the recycle bin mutation.
    pub const RECYCLE_PURGE_JOBS_INVALID: &str = "JOB_RECYCLE_PURGE_JOBS_INVALID";
    /// Acquisition search jobs must be started from the acquisition search mutation.
    pub const ACQUISITION_SEARCH_JOBS_INVALID: &str = "JOB_ACQUISITION_SEARCH_JOBS_INVALID";
    /// Application upgrade jobs must be started from the application upgrade mutation.
    pub const APPLICATION_UPGRADE_JOBS_INVALID: &str = "JOB_APPLICATION_UPGRADE_JOBS_INVALID";
    /// Location operations must be started from the location operation mutation.
    pub const LOCATION_OPERATIONS_INVALID: &str = "JOB_LOCATION_OPERATIONS_INVALID";
    /// Discovery incremental requires an active generation.
    pub const DISCOVERY_INCREMENTAL_REQUIRES_ACTIVE_GENERATION: &str =
        "JOB_DISCOVERY_INCREMENTAL_REQUIRES_ACTIVE_GENERATION";
    /// Discovery incremental requires a previous fingerprint.
    pub const DISCOVERY_INCREMENTAL_REQUIRES_PREVIOUS_FINGERPRINT: &str =
        "JOB_DISCOVERY_INCREMENTAL_REQUIRES_PREVIOUS_FINGERPRINT";
    /// Select at least one recycle-bin item.
    pub const RECYCLE_BIN_ITEM_REQUIRED: &str = "JOB_RECYCLE_BIN_ITEM_REQUIRED";
    /// Recycle-bin batches are limited to the selected value items.
    pub const RECYCLE_BIN_BATCHES_LIMITED_ITEMS: &str = "JOB_RECYCLE_BIN_BATCHES_LIMITED_ITEMS";
    /// Recycle-bin batches cannot include the same item more than once.
    pub const RECYCLE_BIN_BATCHES_CANNOT_INCLUDE_SAME_ITEM_MORE_ONCE: &str =
        "JOB_RECYCLE_BIN_BATCHES_CANNOT_INCLUDE_SAME_ITEM_MORE_ONCE";
    /// Refusing to replace restore destination the selected value because it is a symlink.
    pub const SYMLINK_REFUSED: &str = "JOB_SYMLINK_REFUSED";
    /// Refusing to replace restore destination the selected value because it is not a regular file.
    pub const REGULAR_FILE_REQUIRED: &str = "JOB_REGULAR_FILE_REQUIRED";
    /// Refusing to replace the selected value because the recycle bin is unavailable.
    pub const REPLACE_RECYCLE_BIN_UNAVAILABLE: &str = "JOB_REPLACE_RECYCLE_BIN_UNAVAILABLE";
    /// You do not have permission to view the recycle bin.
    pub const VIEW_RECYCLE_BIN_PERMISSION_DENIED: &str = "JOB_VIEW_RECYCLE_BIN_PERMISSION_DENIED";
    /// A recycle restore is already running for the selected value.
    pub const RECYCLE_RESTORE_ALREADY_RUNNING: &str = "JOB_RECYCLE_RESTORE_ALREADY_RUNNING";
    /// Selected recycle entries cannot share the same restore destination.
    pub const SELECTED_RECYCLE_ENTRIES_CANNOT_SHARE_SAME_RESTORE_DESTINATION: &str =
        "JOB_SELECTED_RECYCLE_ENTRIES_CANNOT_SHARE_SAME_RESTORE_DESTINATION";
    /// Recycle restore preview is stale; review the conflict choice again.
    pub const RECYCLE_RESTORE_PREVIEW_STALE: &str = "JOB_RECYCLE_RESTORE_PREVIEW_STALE";
    /// A recycle restore is already running for one of the selected destinations.
    pub const RECYCLE_RESTORE_ALREADY_RUNNING_ONE_SELECTED_DESTINATIONS: &str =
        "JOB_RECYCLE_RESTORE_ALREADY_RUNNING_ONE_SELECTED_DESTINATIONS";
    /// Recycle entry was not found.
    pub const RECYCLE_ENTRY_NOT_FOUND: &str = "JOB_RECYCLE_ENTRY_NOT_FOUND";
    /// A recycle operation is already running for the selected value.
    pub const RECYCLE_OPERATION_ALREADY_RUNNING: &str = "JOB_RECYCLE_OPERATION_ALREADY_RUNNING";
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "JOB_LIBRARY_ACCESS_DENIED";
    /// Refusing to restore recycle entry the selected value because original path is outside the resolved library roots.
    pub const PATH_OUTSIDE_LIBRARY_ROOT: &str = "JOB_PATH_OUTSIDE_LIBRARY_ROOT";
    /// Restore destination changed after confirmation; preview and retry this item.
    pub const RESTORE_DESTINATION_CHANGED_AFTER_CONFIRMATION: &str =
        "JOB_RESTORE_DESTINATION_CHANGED_AFTER_CONFIRMATION";
    /// Emptying the recycle bin is already running.
    pub const EMPTYING_RECYCLE_BIN_ALREADY_RUNNING: &str =
        "JOB_EMPTYING_RECYCLE_BIN_ALREADY_RUNNING";
    /// The requested background task failed.
    pub const TASK_FAILED: &str = "JOB_TASK_FAILED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        SCRIPT_NOT_SCHEDULED_SCRIPT,
        SCHEDULED_SCRIPT_DISABLED,
        SCRIPT_NOT_FOUND,
        ALREADY_RUNNING,
        SCHEDULED_SCRIPT_ALREADY_RUNNING,
        CUSTOM_JOB_RUN_DOES_NOT_NAME_SCRIPT,
        SCHEDULED_SCRIPT_NO_LONGER_EXISTS,
        ACTION_PERMISSION_DENIED,
        CUSTOM_JOB_RUN_NEEDS_ID_SCHEDULED_SCRIPT_RUN,
        FULL_HASH_BACKFILL_SHUTTING_DOWN,
        ARTWORK_ENCODING_SHUTTING_DOWN,
        ONLY_RUN_CONFIGURED_SCHEDULE,
        NO_LIBRARIES,
        BACKGROUND_LIBRARY_REFRESH_TEMPORARILY_DISABLED,
        TITLE_DELETION_JOBS_INVALID,
        TITLE_RENAME_JOBS_INVALID,
        MEDIA_FILE_DELETION_JOBS_INVALID,
        SELECT_FILES_SEASONS_TITLES_REFRESH_SCORES,
        RECYCLE_RESTORE_JOBS_INVALID,
        RECYCLE_PURGE_JOBS_INVALID,
        ACQUISITION_SEARCH_JOBS_INVALID,
        APPLICATION_UPGRADE_JOBS_INVALID,
        LOCATION_OPERATIONS_INVALID,
        DISCOVERY_INCREMENTAL_REQUIRES_ACTIVE_GENERATION,
        DISCOVERY_INCREMENTAL_REQUIRES_PREVIOUS_FINGERPRINT,
        RECYCLE_BIN_ITEM_REQUIRED,
        RECYCLE_BIN_BATCHES_LIMITED_ITEMS,
        RECYCLE_BIN_BATCHES_CANNOT_INCLUDE_SAME_ITEM_MORE_ONCE,
        SYMLINK_REFUSED,
        REGULAR_FILE_REQUIRED,
        REPLACE_RECYCLE_BIN_UNAVAILABLE,
        VIEW_RECYCLE_BIN_PERMISSION_DENIED,
        RECYCLE_RESTORE_ALREADY_RUNNING,
        SELECTED_RECYCLE_ENTRIES_CANNOT_SHARE_SAME_RESTORE_DESTINATION,
        RECYCLE_RESTORE_PREVIEW_STALE,
        RECYCLE_RESTORE_ALREADY_RUNNING_ONE_SELECTED_DESTINATIONS,
        RECYCLE_ENTRY_NOT_FOUND,
        RECYCLE_OPERATION_ALREADY_RUNNING,
        LIBRARY_ACCESS_DENIED,
        PATH_OUTSIDE_LIBRARY_ROOT,
        RESTORE_DESTINATION_CHANGED_AFTER_CONFIRMATION,
        EMPTYING_RECYCLE_BIN_ALREADY_RUNNING,
        TASK_FAILED,
    ];
}

pub mod events {
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "EVENTS_LIBRARY_ACCESS_DENIED";
    /// No active plugin install progress for was not found.
    pub const ACTIVE_PLUGIN_INSTALL_PROGRESS_NOT_FOUND: &str =
        "EVENTS_ACTIVE_PLUGIN_INSTALL_PROGRESS_NOT_FOUND";
    /// Episode was not found.
    pub const EPISODE_NOT_FOUND: &str = "EVENTS_EPISODE_NOT_FOUND";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        LIBRARY_ACCESS_DENIED,
        ACTIVE_PLUGIN_INSTALL_PROGRESS_NOT_FOUND,
        EPISODE_NOT_FOUND,
    ];
}

pub mod import {
    /// Cannot safely upgrade the selected value because no configured media root could be resolved.
    pub const MEDIA_ROOT_REQUIRED: &str = "IMPORT_MEDIA_ROOT_REQUIRED";
    /// Refusing same-path upgrade guard because old file the selected value is outside media root the selected value.
    pub const PATH_OUTSIDE_LIBRARY_ROOT: &str = "IMPORT_PATH_OUTSIDE_LIBRARY_ROOT";
    /// Refusing to upgrade the selected value because no recycle bin config could be resolved for old file root the selected value.
    pub const RECYCLE_BIN_CONFIG_REQUIRED: &str = "IMPORT_RECYCLE_BIN_CONFIG_REQUIRED";
    /// Refusing to dispose old upgrade file the selected value because it is a symlink.
    pub const SYMLINK_REFUSED: &str = "IMPORT_SYMLINK_REFUSED";
    /// Refusing to dispose old upgrade file the selected value because it is not a regular file.
    pub const REGULAR_FILE_REQUIRED: &str = "IMPORT_REGULAR_FILE_REQUIRED";
    /// Refusing to upgrade because the recycle bin path is unsafe.
    pub const UPGRADE_RECYCLE_BIN_PATH_UNSAFE: &str = "IMPORT_UPGRADE_RECYCLE_BIN_PATH_UNSAFE";
    /// Refusing to restore old upgrade backup the selected value because destination is occupied.
    pub const DESTINATION_OCCUPIED: &str = "IMPORT_DESTINATION_OCCUPIED";
    /// A script's trigger cannot be changed after it is created.
    pub const SCRIPT_S_TRIGGER_CANNOT_CHANGED_AFTER_CREATED: &str =
        "IMPORT_SCRIPT_S_TRIGGER_CANNOT_CHANGED_AFTER_CREATED";
    /// Script was not found.
    pub const SCRIPT_NOT_FOUND: &str = "IMPORT_SCRIPT_NOT_FOUND";
    /// Scheduled scripts require a schedule.
    pub const SCHEDULED_SCRIPTS_REQUIRE_SCHEDULE: &str =
        "IMPORT_SCHEDULED_SCRIPTS_REQUIRE_SCHEDULE";
    /// No arr source warmup result was not found.
    pub const ARR_SOURCE_WARMUP_RESULT_NOT_FOUND: &str =
        "IMPORT_ARR_SOURCE_WARMUP_RESULT_NOT_FOUND";
    /// No prowlarr warmup result was not found.
    pub const PROWLARR_WARMUP_RESULT_NOT_FOUND: &str = "IMPORT_PROWLARR_WARMUP_RESULT_NOT_FOUND";
    /// External import setup secret draft cannot be empty; clear the draft instead.
    pub const EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_CANNOT_EMPTY: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_CANNOT_EMPTY";
    /// External import setup secret draft supports at most the selected value instance API keys.
    pub const EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST_INSTANCE_API_KEYS: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST_INSTANCE_API_KEYS";
    /// External import setup secret draft supports at most the selected value the selected value.
    pub const EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST";
    /// Duplicate external import setup instanceId.
    pub const EXTERNAL_IMPORT_SETUP_INSTANCEID_DUPLICATED: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_INSTANCEID_DUPLICATED";
    /// Duplicate external import setup dedupKey in the selected value.
    pub const EXTERNAL_IMPORT_SETUP_DEDUPKEY_DUPLICATED: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_DEDUPKEY_DUPLICATED";
    /// External import setup secret draft the selected value cannot be empty.
    pub const EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_REQUIRED: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_REQUIRED";
    /// External import setup secret draft the selected value must be at most the selected value bytes.
    pub const EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_OUT_OF_RANGE: &str =
        "IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_OUT_OF_RANGE";
    /// The completed download has not been observed yet; release again once it has.
    pub const COMPLETED_DOWNLOAD_NOT_OBSERVED_YET: &str =
        "IMPORT_COMPLETED_DOWNLOAD_NOT_OBSERVED_YET";
    /// The grab this download came from could not be resolved; release again once it can.
    pub const GRAB_DOWNLOAD_CAME_COULD_NOT_RESOLVED: &str =
        "IMPORT_GRAB_DOWNLOAD_CAME_COULD_NOT_RESOLVED";
    /// Manual import path is not accessible.
    pub const MANUAL_IMPORT_PATH_UNAVAILABLE: &str = "IMPORT_MANUAL_IMPORT_PATH_UNAVAILABLE";
    /// Manual import file must have a file name.
    pub const MANUAL_IMPORT_FILE_INVALID: &str = "IMPORT_MANUAL_IMPORT_FILE_INVALID";
    /// Manual import source root is not accessible.
    pub const MANUAL_IMPORT_SOURCE_ROOT_UNAVAILABLE: &str =
        "IMPORT_MANUAL_IMPORT_SOURCE_ROOT_UNAVAILABLE";
    /// Manual import file is not accessible.
    pub const MANUAL_IMPORT_FILE_UNAVAILABLE: &str = "IMPORT_MANUAL_IMPORT_FILE_UNAVAILABLE";
    /// Manual import file is outside the trusted source root.
    pub const MANUAL_IMPORT_FILE_OUTSIDE_TRUSTED_SOURCE_ROOT: &str =
        "IMPORT_MANUAL_IMPORT_FILE_OUTSIDE_TRUSTED_SOURCE_ROOT";
    /// Manual import source root is not a file or directory.
    pub const MANUAL_IMPORT_SOURCE_ROOT_NOT_FILE_DIRECTORY: &str =
        "IMPORT_MANUAL_IMPORT_SOURCE_ROOT_NOT_FILE_DIRECTORY";
    /// Manual import file path is outside the trusted source root.
    pub const MANUAL_IMPORT_FILE_PATH_OUTSIDE_TRUSTED_SOURCE_ROOT: &str =
        "IMPORT_MANUAL_IMPORT_FILE_PATH_OUTSIDE_TRUSTED_SOURCE_ROOT";
    /// Manual import source is not a file.
    pub const MANUAL_IMPORT_SOURCE_NOT_FILE: &str = "IMPORT_MANUAL_IMPORT_SOURCE_NOT_FILE";
    /// Disc import targets must exactly match its explicit episode mappings.
    pub const DISC_IMPORT_TARGETS_INVALID: &str = "IMPORT_DISC_IMPORT_TARGETS_INVALID";
    /// Import was not found.
    pub const RESOURCE_NOT_FOUND: &str = "IMPORT_RESOURCE_NOT_FOUND";
    /// Import the selected value has status the selected item, only failed or skipped imports can be retried.
    pub const STATUS_ONLY_FAILED_SKIPPED_IMPORTS_RETRIED: &str =
        "IMPORT_STATUS_ONLY_FAILED_SKIPPED_IMPORTS_RETRIED";
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "IMPORT_LIBRARY_ACCESS_DENIED";
    /// This download is already being imported.
    pub const DOWNLOAD_ALREADY_BEING_IMPORTED: &str = "IMPORT_DOWNLOAD_ALREADY_BEING_IMPORTED";
    /// The import changed while retry was requested; refresh its history.
    pub const CHANGED_WHILE_RETRY_REQUESTED: &str = "IMPORT_CHANGED_WHILE_RETRY_REQUESTED";
    /// The manual import source is unavailable.
    pub const SOURCE_UNAVAILABLE: &str = "IMPORT_SOURCE_UNAVAILABLE";
    /// The download is busy or the import changed; refresh and retry.
    pub const DOWNLOAD_BUSY_IMPORT_CHANGED: &str = "IMPORT_DOWNLOAD_BUSY_IMPORT_CHANGED";
    /// An import of this download has no title to authorize its release against.
    pub const DOWNLOAD_NO_TITLE_AUTHORIZE_RELEASE_AGAINST: &str =
        "IMPORT_DOWNLOAD_NO_TITLE_AUTHORIZE_RELEASE_AGAINST";
    /// Releasing these sources also releases an import for a title you may not resolve imports for.
    pub const RELEASING_THESE_SOURCES_ALSO_RELEASES_IMPORT_TITLE_MAY_NOT_RESOLVE_IMPORTS: &str =
        "IMPORT_RELEASING_THESE_SOURCES_ALSO_RELEASES_IMPORT_TITLE_MAY_NOT_RESOLVE_IMPORTS";
    /// This download has no stable identity yet; its sources stay held.
    pub const DOWNLOAD_NO_STABLE_IDENTITY_YET: &str = "IMPORT_DOWNLOAD_NO_STABLE_IDENTITY_YET";
    /// This download is still being imported; release its sources once that finishes.
    pub const DOWNLOAD_STILL_BEING_IMPORTED: &str = "IMPORT_DOWNLOAD_STILL_BEING_IMPORTED";
    /// Import the selected value is not holding its sources.
    pub const NOT_HOLDING_SOURCES: &str = "IMPORT_NOT_HOLDING_SOURCES";
    /// Archive matched title the selected item but extracted workspace the selected value could not be moved to the selected value without copying.
    pub const ARCHIVE_MATCHED_TITLE_BUT_EXTRACTED_WORKSPACE_COULD_NOT_MOVED_WITHOUT_COPYING: &str =
        "IMPORT_ARCHIVE_MATCHED_TITLE_BUT_EXTRACTED_WORKSPACE_COULD_NOT_MOVED_WITHOUT_COPYING";
    /// Archive matched title the selected item but could not be re-extracted under the matched title destination.
    pub const ARCHIVE_REEXTRACTION_FAILED: &str = "IMPORT_ARCHIVE_REEXTRACTION_FAILED";
    /// Manual title the selected value is outside the durable Scryer submission title the selected value.
    pub const MANUAL_TITLE_OUTSIDE_DURABLE_SCRYER_SUBMISSION_TITLE: &str =
        "IMPORT_MANUAL_TITLE_OUTSIDE_DURABLE_SCRYER_SUBMISSION_TITLE";
    /// Scene subtitle file count limit exceeded; workspace preserved.
    pub const SCENE_SUBTITLE_FILE_COUNT_LIMIT_EXCEEDED: &str =
        "IMPORT_SCENE_SUBTITLE_FILE_COUNT_LIMIT_EXCEEDED";
    /// Scene subtitle aggregate size limit exceeded; workspace preserved.
    pub const SCENE_SUBTITLE_AGGREGATE_SIZE_LIMIT_EXCEEDED: &str =
        "IMPORT_SCENE_SUBTITLE_AGGREGATE_SIZE_LIMIT_EXCEEDED";
    /// Scene subtitle delivery context is unavailable.
    pub const SCENE_SUBTITLE_DELIVERY_CONTEXT_UNAVAILABLE: &str =
        "IMPORT_SCENE_SUBTITLE_DELIVERY_CONTEXT_UNAVAILABLE";
    /// Imported video has no filename stem.
    pub const IMPORTED_VIDEO_NO_FILENAME_STEM: &str = "IMPORT_IMPORTED_VIDEO_NO_FILENAME_STEM";
    /// Imported video has no parent directory.
    pub const IMPORTED_VIDEO_NO_PARENT_DIRECTORY: &str =
        "IMPORT_IMPORTED_VIDEO_NO_PARENT_DIRECTORY";
    /// Multiple scene subtitles map to the same destination; sources preserved.
    pub const MULTIPLE_SCENE_SUBTITLES_MAP_SAME_DESTINATION: &str =
        "IMPORT_MULTIPLE_SCENE_SUBTITLES_MAP_SAME_DESTINATION";
    /// Scene subtitle staging workspace is unavailable.
    pub const SCENE_SUBTITLE_STAGING_WORKSPACE_UNAVAILABLE: &str =
        "IMPORT_SCENE_SUBTITLE_STAGING_WORKSPACE_UNAVAILABLE";
    /// Insufficient disk allowance for scene subtitles; workspace preserved.
    pub const INSUFFICIENT_DISK_ALLOWANCE_SCENE_SUBTITLES: &str =
        "IMPORT_INSUFFICIENT_DISK_ALLOWANCE_SCENE_SUBTITLES";
    /// Scene subtitle lies outside its owned workspace.
    pub const SCENE_SUBTITLE_LIES_OUTSIDE_OWNED_WORKSPACE: &str =
        "IMPORT_SCENE_SUBTITLE_LIES_OUTSIDE_OWNED_WORKSPACE";
    /// Scene subtitle workspace identity changed.
    pub const SCENE_SUBTITLE_WORKSPACE_IDENTITY_CHANGED: &str =
        "IMPORT_SCENE_SUBTITLE_WORKSPACE_IDENTITY_CHANGED";
    /// Invalid scene subtitle path component.
    pub const SCENE_SUBTITLE_PATH_COMPONENT_INVALID: &str =
        "IMPORT_SCENE_SUBTITLE_PATH_COMPONENT_INVALID";
    /// Invalid scene subtitle path.
    pub const SCENE_SUBTITLE_PATH_INVALID: &str = "IMPORT_SCENE_SUBTITLE_PATH_INVALID";
    /// Scene subtitle is not a regular file.
    pub const SCENE_SUBTITLE_NOT_REGULAR_FILE: &str = "IMPORT_SCENE_SUBTITLE_NOT_REGULAR_FILE";
    /// Scene subtitle path has no filename.
    pub const SCENE_SUBTITLE_PATH_NO_FILENAME: &str = "IMPORT_SCENE_SUBTITLE_PATH_NO_FILENAME";
    /// Scene subtitle handle lies outside its owned workspace or is a reparse point.
    pub const SCENE_SUBTITLE_HANDLE_LIES_OUTSIDE_OWNED_WORKSPACE_REPARSE_POINT: &str =
        "IMPORT_SCENE_SUBTITLE_HANDLE_LIES_OUTSIDE_OWNED_WORKSPACE_REPARSE_POINT";
    /// Scene subtitle handle path is invalid UTF-16.
    pub const SCENE_SUBTITLE_HANDLE_PATH_INVALID_UTF_16: &str =
        "IMPORT_SCENE_SUBTITLE_HANDLE_PATH_INVALID_UTF_16";
    /// Scene subtitle path contains a symlink.
    pub const SCENE_SUBTITLE_PATH_CONTAINS_SYMLINK: &str =
        "IMPORT_SCENE_SUBTITLE_PATH_CONTAINS_SYMLINK";
    /// Client-reported subtitle is a reparse point.
    pub const CLIENT_REPORTED_SUBTITLE_REPARSE_POINT: &str =
        "IMPORT_CLIENT_REPORTED_SUBTITLE_REPARSE_POINT";
    /// Client-reported subtitle is not a regular file.
    pub const CLIENT_REPORTED_SUBTITLE_NOT_REGULAR_FILE: &str =
        "IMPORT_CLIENT_REPORTED_SUBTITLE_NOT_REGULAR_FILE";
    /// Scene subtitle directory nesting limit exceeded.
    pub const SCENE_SUBTITLE_DIRECTORY_NESTING_LIMIT_EXCEEDED: &str =
        "IMPORT_SCENE_SUBTITLE_DIRECTORY_NESTING_LIMIT_EXCEEDED";
    /// Scene subtitle directory entry limit exceeded.
    pub const SCENE_SUBTITLE_DIRECTORY_ENTRY_LIMIT_EXCEEDED: &str =
        "IMPORT_SCENE_SUBTITLE_DIRECTORY_ENTRY_LIMIT_EXCEEDED";
    /// Scene subtitle staging requires an owned archive workspace.
    pub const SCENE_SUBTITLE_STAGING_REQUIRES_OWNED_ARCHIVE_WORKSPACE: &str =
        "IMPORT_SCENE_SUBTITLE_STAGING_REQUIRES_OWNED_ARCHIVE_WORKSPACE";
    /// Scene subtitle is not a bounded regular file.
    pub const SCENE_SUBTITLE_NOT_BOUNDED_REGULAR_FILE: &str =
        "IMPORT_SCENE_SUBTITLE_NOT_BOUNDED_REGULAR_FILE";
    /// Scene subtitle size limit exceeded.
    pub const SCENE_SUBTITLE_SIZE_LIMIT_EXCEEDED: &str =
        "IMPORT_SCENE_SUBTITLE_SIZE_LIMIT_EXCEEDED";
    /// Scene subtitle destination already exists with different contents; source preserved.
    pub const SCENE_SUBTITLE_DESTINATION_ALREADY_EXISTS_DIFFERENT_CONTENTS: &str =
        "IMPORT_SCENE_SUBTITLE_DESTINATION_ALREADY_EXISTS_DIFFERENT_CONTENTS";
    /// The import record conflicts with its completed source identity; use manual import.
    pub const RECORD_CONFLICTS_COMPLETED_SOURCE_IDENTITY: &str =
        "IMPORT_RECORD_CONFLICTS_COMPLETED_SOURCE_IDENTITY";
    /// The completed source has no trustworthy download identity; use manual import.
    pub const COMPLETED_SOURCE_NO_TRUSTWORTHY_DOWNLOAD_IDENTITY: &str =
        "IMPORT_COMPLETED_SOURCE_NO_TRUSTWORTHY_DOWNLOAD_IDENTITY";
    /// The client job ID now belongs to a different download; use manual import for the retained files.
    pub const CLIENT_JOB_ID_NOW_BELONGS_DIFFERENT_DOWNLOAD: &str =
        "IMPORT_CLIENT_JOB_ID_NOW_BELONGS_DIFFERENT_DOWNLOAD";
    /// This import retry is already running or awaiting reconciliation.
    pub const RETRY_ALREADY_RUNNING_AWAITING_RECONCILIATION: &str =
        "IMPORT_RETRY_ALREADY_RUNNING_AWAITING_RECONCILIATION";
    /// The download failed; retry the download before importing.
    pub const DOWNLOAD_FAILED: &str = "IMPORT_DOWNLOAD_FAILED";
    /// The download is already settled; refresh its history.
    pub const DOWNLOAD_ALREADY_SETTLED: &str = "IMPORT_DOWNLOAD_ALREADY_SETTLED";
    /// The download client configuration no longer matches this source; use manual import.
    pub const DOWNLOAD_CLIENT_CONFIGURATION_NO_LONGER_MATCHES_SOURCE: &str =
        "IMPORT_DOWNLOAD_CLIENT_CONFIGURATION_NO_LONGER_MATCHES_SOURCE";
    /// The client returned a different download identity; refresh and retry.
    pub const CLIENT_RETURNED_DIFFERENT_DOWNLOAD_IDENTITY: &str =
        "IMPORT_CLIENT_RETURNED_DIFFERENT_DOWNLOAD_IDENTITY";
    /// The client does not report a completed download; import retry cannot restart a failed or active download.
    pub const CLIENT_DOES_NOT_REPORT_COMPLETED_DOWNLOAD: &str =
        "IMPORT_CLIENT_DOES_NOT_REPORT_COMPLETED_DOWNLOAD";
    /// The client's current download state could not be established; try again when its history is available.
    pub const DOWNLOAD_STATE_UNAVAILABLE: &str = "IMPORT_DOWNLOAD_STATE_UNAVAILABLE";
    /// No completed import attempt is available to retry; use manual import.
    pub const NO_COMPLETED_IMPORT_ATTEMPT_UNAVAILABLE: &str =
        "IMPORT_NO_COMPLETED_IMPORT_ATTEMPT_UNAVAILABLE";
    /// A newer import attempt owns this download; refresh its history.
    pub const NEWER_IMPORT_ATTEMPT_OWNS_DOWNLOAD: &str =
        "IMPORT_NEWER_IMPORT_ATTEMPT_OWNS_DOWNLOAD";
    /// Retry import record was not found.
    pub const RETRY_IMPORT_RECORD_NOT_FOUND: &str = "IMPORT_RETRY_IMPORT_RECORD_NOT_FOUND";
    /// This download is awaiting import retry reconciliation.
    pub const DOWNLOAD_AWAITING_IMPORT_RETRY_RECONCILIATION: &str =
        "IMPORT_DOWNLOAD_AWAITING_IMPORT_RETRY_RECONCILIATION";
    /// Media file was not found.
    pub const MEDIA_FILE_NOT_FOUND: &str = "IMPORT_MEDIA_FILE_NOT_FOUND";
    /// Saved disc selection changed before import metadata was published.
    pub const SAVED_DISC_SELECTION_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED: &str =
        "IMPORT_SAVED_DISC_SELECTION_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED";
    /// Media file changed before import metadata was published.
    pub const MEDIA_FILE_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED: &str =
        "IMPORT_MEDIA_FILE_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED";
    /// Archive extraction requires a resolved import destination before staging output for the selected value.
    pub const ARCHIVE_EXTRACTION_REQUIRES_RESOLVED_IMPORT_DESTINATION_BEFORE_STAGING_OUTPUT: &str =
        "IMPORT_ARCHIVE_EXTRACTION_REQUIRES_RESOLVED_IMPORT_DESTINATION_BEFORE_STAGING_OUTPUT";
    /// Archive holds archives nested more than the selected value levels deep.
    pub const ARCHIVE_HOLDS_ARCHIVES_NESTED_MORE_LEVELS_DEEP: &str =
        "IMPORT_ARCHIVE_HOLDS_ARCHIVES_NESTED_MORE_LEVELS_DEEP";
    /// Archive holds more than the selected value nested archives.
    pub const ARCHIVE_HOLDS_MORE_NESTED_ARCHIVES: &str =
        "IMPORT_ARCHIVE_HOLDS_MORE_NESTED_ARCHIVES";
    /// Archive output directory the selected value is not inside the staging workspace.
    pub const ARCHIVE_OUTPUT_DIRECTORY_NOT_INSIDE_STAGING_WORKSPACE: &str =
        "IMPORT_ARCHIVE_OUTPUT_DIRECTORY_NOT_INSIDE_STAGING_WORKSPACE";
    /// Archive staging output directory the selected value is not empty.
    pub const ARCHIVE_STAGING_OUTPUT_DIRECTORY_NOT_EMPTY: &str =
        "IMPORT_ARCHIVE_STAGING_OUTPUT_DIRECTORY_NOT_EMPTY";
    /// Archive source the selected value lies inside its own extraction output.
    pub const ARCHIVE_SOURCE_LIES_INSIDE_OWN_EXTRACTION_OUTPUT: &str =
        "IMPORT_ARCHIVE_SOURCE_LIES_INSIDE_OWN_EXTRACTION_OUTPUT";
    /// Archive source must not be a symlink.
    pub const ARCHIVE_SOURCE_SYMLINK_FORBIDDEN: &str = "IMPORT_ARCHIVE_SOURCE_SYMLINK_FORBIDDEN";
    /// Download holds more than the selected value archive sets.
    pub const DOWNLOAD_HOLDS_MORE_ARCHIVE_SETS: &str = "IMPORT_DOWNLOAD_HOLDS_MORE_ARCHIVE_SETS";
    /// Archive source metadata exceeds entry limit.
    pub const ARCHIVE_SOURCE_METADATA_EXCEEDS_ENTRY_LIMIT: &str =
        "IMPORT_ARCHIVE_SOURCE_METADATA_EXCEEDS_ENTRY_LIMIT";
    /// Archive source metadata must identify a regular file.
    pub const ARCHIVE_SOURCE_METADATA_MUST_IDENTIFY_REGULAR_FILE: &str =
        "IMPORT_ARCHIVE_SOURCE_METADATA_MUST_IDENTIFY_REGULAR_FILE";
    /// Archive source metadata escapes its source directory.
    pub const ARCHIVE_SOURCE_METADATA_ESCAPES_SOURCE_DIRECTORY: &str =
        "IMPORT_ARCHIVE_SOURCE_METADATA_ESCAPES_SOURCE_DIRECTORY";
    /// Archive plugin does not support the selected value extraction.
    pub const ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_EXTRACTION: &str =
        "IMPORT_ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_EXTRACTION";
    /// Archive plugin manifest output is not a regular file.
    pub const ARCHIVE_PLUGIN_MANIFEST_OUTPUT_NOT_REGULAR_FILE: &str =
        "IMPORT_ARCHIVE_PLUGIN_MANIFEST_OUTPUT_NOT_REGULAR_FILE";
    /// Archive plugin manifest size mismatch for the selected value.
    pub const ARCHIVE_PLUGIN_MANIFEST_SIZE_MISMATCH: &str =
        "IMPORT_ARCHIVE_PLUGIN_MANIFEST_SIZE_MISMATCH";
    /// Archive plugin output contains too many entries.
    pub const ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_ENTRIES: &str =
        "IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_ENTRIES";
    /// Archive plugin output contains a symlink.
    pub const ARCHIVE_PLUGIN_OUTPUT_CONTAINS_SYMLINK: &str =
        "IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_SYMLINK";
    /// Archive plugin output contains too many directories.
    pub const ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_DIRECTORIES: &str =
        "IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_DIRECTORIES";
    /// Archive plugin output is not a regular file.
    pub const ARCHIVE_PLUGIN_OUTPUT_NOT_REGULAR_FILE: &str =
        "IMPORT_ARCHIVE_PLUGIN_OUTPUT_NOT_REGULAR_FILE";
    /// Archive plugin output contains too many files.
    pub const ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_FILES: &str =
        "IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_FILES";
    /// Archive plugin output is too large.
    pub const ARCHIVE_PLUGIN_OUTPUT_TOO_LARGE: &str = "IMPORT_ARCHIVE_PLUGIN_OUTPUT_TOO_LARGE";
    /// Archive plugin output exceeds the selected value bytes.
    pub const ARCHIVE_PLUGIN_OUTPUT_EXCEEDS_BYTES: &str =
        "IMPORT_ARCHIVE_PLUGIN_OUTPUT_EXCEEDS_BYTES";
    /// Unsafe archive entry path.
    pub const UNSAFE_ARCHIVE_ENTRY_PATH: &str = "IMPORT_UNSAFE_ARCHIVE_ENTRY_PATH";
    /// Archive entry escapes extraction directory.
    pub const ARCHIVE_ENTRY_ESCAPES_EXTRACTION_DIRECTORY: &str =
        "IMPORT_ARCHIVE_ENTRY_ESCAPES_EXTRACTION_DIRECTORY";
    /// Archive source is not a regular directory.
    pub const ARCHIVE_SOURCE_NOT_REGULAR_DIRECTORY: &str =
        "IMPORT_ARCHIVE_SOURCE_NOT_REGULAR_DIRECTORY";
    /// Archive discovery entry limit exceeded.
    pub const ARCHIVE_DISCOVERY_ENTRY_LIMIT_EXCEEDED: &str =
        "IMPORT_ARCHIVE_DISCOVERY_ENTRY_LIMIT_EXCEEDED";
    /// Archive discovery directory or depth limit exceeded.
    pub const ARCHIVE_DISCOVERY_DIRECTORY_DEPTH_LIMIT_EXCEEDED: &str =
        "IMPORT_ARCHIVE_DISCOVERY_DIRECTORY_DEPTH_LIMIT_EXCEEDED";
    /// Manual import source has a conflicting canonical download identity.
    pub const MANUAL_IMPORT_SOURCE_CONFLICTING_CANONICAL_DOWNLOAD_IDENTITY: &str =
        "IMPORT_MANUAL_IMPORT_SOURCE_CONFLICTING_CANONICAL_DOWNLOAD_IDENTITY";
    /// Manual import title does not match the Scryer submission that grabbed this download.
    pub const MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD: &str =
        "IMPORT_MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD";
    /// Extracted archive workspace is not accessible.
    pub const EXTRACTED_ARCHIVE_WORKSPACE_UNAVAILABLE: &str =
        "IMPORT_EXTRACTED_ARCHIVE_WORKSPACE_UNAVAILABLE";
    /// Disc selection cannot include a separate episode or series-movie target.
    pub const DISC_SELECTION_CANNOT_INCLUDE_SEPARATE_EPISODE_SERIES_MOVIE_TARGET: &str =
        "IMPORT_DISC_SELECTION_CANNOT_INCLUDE_SEPARATE_EPISODE_SERIES_MOVIE_TARGET";
    /// Invalid disc selection or mapping limit exceeded.
    pub const DISC_SELECTION_MAPPING_LIMIT_EXCEEDED_INVALID: &str =
        "IMPORT_DISC_SELECTION_MAPPING_LIMIT_EXCEEDED_INVALID";
    /// Disc titles require distinct, explicit single-episode mappings.
    pub const DISC_TITLES_REQUIRE_DISTINCT_EXPLICIT_SINGLE_EPISODE_MAPPINGS: &str =
        "IMPORT_DISC_TITLES_REQUIRE_DISTINCT_EXPLICIT_SINGLE_EPISODE_MAPPINGS";
    /// Episodic discs require playback-title mappings; movie discs cannot map episodes.
    pub const EPISODIC_DISCS_REQUIRE_PLAYBACK_TITLE_MAPPINGS: &str =
        "IMPORT_EPISODIC_DISCS_REQUIRE_PLAYBACK_TITLE_MAPPINGS";
    /// Manual import mapping requires episode_id or series_movie_link_id.
    pub const MANUAL_IMPORT_MAPPING_REQUIRES_EPISODE_ID_SERIES_MOVIE_LINK_ID: &str =
        "IMPORT_MANUAL_IMPORT_MAPPING_REQUIRES_EPISODE_ID_SERIES_MOVIE_LINK_ID";
    /// Manual import mapping cannot include both episode_id and series_movie_link_id.
    pub const MANUAL_IMPORT_MAPPING_CANNOT_INCLUDE_BOTH_EPISODE_ID_SERIES_MOVIE_LINK_ID: &str =
        "IMPORT_MANUAL_IMPORT_MAPPING_CANNOT_INCLUDE_BOTH_EPISODE_ID_SERIES_MOVIE_LINK_ID";
    /// At least one manual import candidate is required.
    pub const MANUAL_IMPORT_CANDIDATE_REQUIRED: &str = "IMPORT_MANUAL_IMPORT_CANDIDATE_REQUIRED";
    /// Manual import candidate id is required.
    pub const MANUAL_IMPORT_CANDIDATE_ID_REQUIRED: &str =
        "IMPORT_MANUAL_IMPORT_CANDIDATE_ID_REQUIRED";
    /// Manual import candidate ids must be unique.
    pub const MANUAL_IMPORT_CANDIDATE_IDS_INVALID: &str =
        "IMPORT_MANUAL_IMPORT_CANDIDATE_IDS_INVALID";
    /// Manual import episode target is unavailable.
    pub const MANUAL_IMPORT_EPISODE_TARGET_UNAVAILABLE: &str =
        "IMPORT_MANUAL_IMPORT_EPISODE_TARGET_UNAVAILABLE";
    /// Manual import episode target the selected value does not belong to title the selected value.
    pub const MANUAL_IMPORT_EPISODE_TARGET_DOES_NOT_BELONG_TITLE: &str =
        "IMPORT_MANUAL_IMPORT_EPISODE_TARGET_DOES_NOT_BELONG_TITLE";
    /// Manual import series movie target is unavailable.
    pub const MANUAL_IMPORT_SERIES_MOVIE_TARGET_UNAVAILABLE: &str =
        "IMPORT_MANUAL_IMPORT_SERIES_MOVIE_TARGET_UNAVAILABLE";
    /// Manual import series movie target the selected value does not belong to title the selected value.
    pub const MANUAL_IMPORT_SERIES_MOVIE_TARGET_DOES_NOT_BELONG_TITLE: &str =
        "IMPORT_MANUAL_IMPORT_SERIES_MOVIE_TARGET_DOES_NOT_BELONG_TITLE";
    /// Manual import source root is required.
    pub const MANUAL_IMPORT_SOURCE_ROOT_REQUIRED: &str =
        "IMPORT_MANUAL_IMPORT_SOURCE_ROOT_REQUIRED";
    /// A mapped disc episode no longer belongs to the import title.
    pub const MAPPED_DISC_EPISODE_NO_LONGER_BELONGS_IMPORT_TITLE: &str =
        "IMPORT_MAPPED_DISC_EPISODE_NO_LONGER_BELONGS_IMPORT_TITLE";
    /// Manual import request is missing an actor.
    pub const MANUAL_IMPORT_REQUEST_MISSING_ACTOR: &str =
        "IMPORT_MANUAL_IMPORT_REQUEST_MISSING_ACTOR";
    /// Manual import request actor no longer exists.
    pub const MANUAL_IMPORT_REQUEST_ACTOR_NO_LONGER_EXISTS: &str =
        "IMPORT_MANUAL_IMPORT_REQUEST_ACTOR_NO_LONGER_EXISTS";
    /// Cleanup locator is reused or identity is ambiguous.
    pub const CLEANUP_LOCATOR_REUSED_IDENTITY_AMBIGUOUS: &str =
        "IMPORT_CLEANUP_LOCATOR_REUSED_IDENTITY_AMBIGUOUS";
    /// Download client completed download the selected value is unavailable for payload cleanup.
    pub const DOWNLOAD_CLIENT_COMPLETED_DOWNLOAD_NOT_FOUND: &str =
        "IMPORT_DOWNLOAD_CLIENT_COMPLETED_DOWNLOAD_NOT_FOUND";
    /// Download payload directory the selected value has too many entries to inventory safely.
    pub const DOWNLOAD_PAYLOAD_DIRECTORY_TOO_MANY_ENTRIES_INVENTORY_SAFELY: &str =
        "IMPORT_DOWNLOAD_PAYLOAD_DIRECTORY_TOO_MANY_ENTRIES_INVENTORY_SAFELY";
    /// Download payload directory the selected value is no longer a real directory.
    pub const DOWNLOAD_PAYLOAD_DIRECTORY_NO_LONGER_REAL_DIRECTORY: &str =
        "IMPORT_DOWNLOAD_PAYLOAD_DIRECTORY_NO_LONGER_REAL_DIRECTORY";
    /// Download payload inventory entry the selected value is not a safe relative path.
    pub const DOWNLOAD_PAYLOAD_INVENTORY_ENTRY_NOT_SAFE_RELATIVE_PATH: &str =
        "IMPORT_DOWNLOAD_PAYLOAD_INVENTORY_ENTRY_NOT_SAFE_RELATIVE_PATH";
    /// NZBGet cleanup identity changed.
    pub const NZBGET_CLEANUP_IDENTITY_CHANGED: &str = "IMPORT_NZBGET_CLEANUP_IDENTITY_CHANGED";
    /// Nzbget terminal cleanup metadata is unavailable was not found.
    pub const NZBGET_TERMINAL_CLEANUP_METADATA_NOT_FOUND: &str =
        "IMPORT_NZBGET_TERMINAL_CLEANUP_METADATA_NOT_FOUND";
    /// NZBGet native deletion path is unverified.
    pub const NZBGET_NATIVE_DELETION_PATH_UNVERIFIED: &str =
        "IMPORT_NZBGET_NATIVE_DELETION_PATH_UNVERIFIED";
    /// NZBGet native deletion parent is unavailable.
    pub const NZBGET_NATIVE_DELETION_PARENT_UNAVAILABLE: &str =
        "IMPORT_NZBGET_NATIVE_DELETION_PARENT_UNAVAILABLE";
    /// NZBGet history removal would delete retained or unverified data.
    pub const NZBGET_HISTORY_REMOVAL_WOULD_DELETE_RETAINED_UNVERIFIED_DATA: &str =
        "IMPORT_NZBGET_HISTORY_REMOVAL_WOULD_DELETE_RETAINED_UNVERIFIED_DATA";
    /// Download client payload cleanup requires a configured client id.
    pub const DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_REQUIRES_CONFIGURED_CLIENT_ID: &str =
        "IMPORT_DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_REQUIRES_CONFIGURED_CLIENT_ID";
    /// Download client was not found.
    pub const DOWNLOAD_CLIENT_NOT_FOUND: &str = "IMPORT_DOWNLOAD_CLIENT_NOT_FOUND";
    /// Download client path mapping has an unusable local root.
    pub const DOWNLOAD_CLIENT_PATH_MAPPING_UNUSABLE_LOCAL_ROOT: &str =
        "IMPORT_DOWNLOAD_CLIENT_PATH_MAPPING_UNUSABLE_LOCAL_ROOT";
    /// Download client payload path must be absolute.
    pub const DOWNLOAD_CLIENT_PAYLOAD_PATH_INVALID: &str =
        "IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_INVALID";
    /// Download client payload path must not be a filesystem root or contain traversal.
    pub const PAYLOAD_PATH_UNSAFE: &str = "IMPORT_PAYLOAD_PATH_UNSAFE";
    /// Refusing to delete download client output root the selected value.
    pub const CLEANUP_TARGET_IS_OUTPUT_ROOT: &str = "IMPORT_CLEANUP_TARGET_IS_OUTPUT_ROOT";
    /// Payload overlaps a recycle root.
    pub const PAYLOAD_OVERLAPS_RECYCLE_ROOT: &str = "IMPORT_PAYLOAD_OVERLAPS_RECYCLE_ROOT";
    /// Refusing to delete download client payload the selected value because it overlaps a configured library root.
    pub const CLEANUP_TARGET_OVERLAPS_LIBRARY_ROOT: &str =
        "IMPORT_CLEANUP_TARGET_OVERLAPS_LIBRARY_ROOT";
    /// File payload cleanup requires durable job ownership.
    pub const FILE_PAYLOAD_CLEANUP_REQUIRES_DURABLE_JOB_OWNERSHIP: &str =
        "IMPORT_FILE_PAYLOAD_CLEANUP_REQUIRES_DURABLE_JOB_OWNERSHIP";
    /// File payload ownership is unverified; retaining source and client entry.
    pub const FILE_PAYLOAD_OWNERSHIP_UNVERIFIED: &str = "IMPORT_FILE_PAYLOAD_OWNERSHIP_UNVERIFIED";
    /// File payload changed since import; retaining source and client entry.
    pub const FILE_PAYLOAD_CHANGED_SINCE_IMPORT: &str = "IMPORT_FILE_PAYLOAD_CHANGED_SINCE_IMPORT";
    /// Payload directory ownership is unverified; refusing shared-directory deletion.
    pub const PAYLOAD_DIRECTORY_OWNERSHIP_UNVERIFIED: &str =
        "IMPORT_PAYLOAD_DIRECTORY_OWNERSHIP_UNVERIFIED";
    /// Download client payload path the selected value is not a file, directory, or symlink.
    pub const DOWNLOAD_CLIENT_PAYLOAD_PATH_NOT_FILE_DIRECTORY_SYMLINK: &str =
        "IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_NOT_FILE_DIRECTORY_SYMLINK";
    /// Download client payload cleanup checkpoint the selected value is not a directory.
    pub const DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_CHECKPOINT_NOT_DIRECTORY: &str =
        "IMPORT_DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_CHECKPOINT_NOT_DIRECTORY";
    /// Download client payload path the selected value is outside output root the selected value.
    pub const DOWNLOAD_CLIENT_PAYLOAD_PATH_OUTSIDE_OUTPUT_ROOT: &str =
        "IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_OUTSIDE_OUTPUT_ROOT";
    /// Download client payload path the selected value traverses symlink the selected value.
    pub const DOWNLOAD_CLIENT_PAYLOAD_PATH_TRAVERSES_SYMLINK: &str =
        "IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_TRAVERSES_SYMLINK";
    /// You do not have access to any libraries.
    pub const DO_NOT_ACCESS_ANY_LIBRARIES: &str = "IMPORT_DO_NOT_ACCESS_ANY_LIBRARIES";
    /// Active import stream was not found.
    pub const ACTIVE_IMPORT_STREAM_NOT_FOUND: &str = "IMPORT_ACTIVE_IMPORT_STREAM_NOT_FOUND";
    /// The import is no longer cancellable.
    pub const UNAVAILABLE: &str = "IMPORT_UNAVAILABLE";
    /// The recorded cleanup state is invalid.
    pub const CLEANUP_STATE_INVALID: &str = "IMPORT_CLEANUP_STATE_INVALID";
    /// The recorded cleanup has no valid routing attribution.
    pub const CLEANUP_ATTRIBUTION_MISSING: &str = "IMPORT_CLEANUP_ATTRIBUTION_MISSING";
    /// The source file was not found.
    pub const FILE_NOT_FOUND: &str = "IMPORT_FILE_NOT_FOUND";
    /// The filesystem denied access to the import.
    pub const PERMISSION_DENIED: &str = "IMPORT_PERMISSION_DENIED";
    /// There is not enough disk space to import the file.
    pub const DISK_FULL: &str = "IMPORT_DISK_FULL";
    /// The filesystem operation failed.
    pub const IO_FAILED: &str = "IMPORT_IO_FAILED";
    /// The episode was not found.
    pub const EPISODE_NOT_FOUND: &str = "IMPORT_EPISODE_NOT_FOUND";
    /// The episode could not be loaded.
    pub const EPISODE_LOOKUP_FAILED: &str = "IMPORT_EPISODE_LOOKUP_FAILED";
    /// The source download failed.
    pub const SOURCE_JOB_FAILED: &str = "IMPORT_SOURCE_JOB_FAILED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        MEDIA_ROOT_REQUIRED,
        PATH_OUTSIDE_LIBRARY_ROOT,
        RECYCLE_BIN_CONFIG_REQUIRED,
        SYMLINK_REFUSED,
        REGULAR_FILE_REQUIRED,
        UPGRADE_RECYCLE_BIN_PATH_UNSAFE,
        DESTINATION_OCCUPIED,
        SCRIPT_S_TRIGGER_CANNOT_CHANGED_AFTER_CREATED,
        SCRIPT_NOT_FOUND,
        SCHEDULED_SCRIPTS_REQUIRE_SCHEDULE,
        ARR_SOURCE_WARMUP_RESULT_NOT_FOUND,
        PROWLARR_WARMUP_RESULT_NOT_FOUND,
        EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_CANNOT_EMPTY,
        EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST_INSTANCE_API_KEYS,
        EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST,
        EXTERNAL_IMPORT_SETUP_INSTANCEID_DUPLICATED,
        EXTERNAL_IMPORT_SETUP_DEDUPKEY_DUPLICATED,
        EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_REQUIRED,
        EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_OUT_OF_RANGE,
        COMPLETED_DOWNLOAD_NOT_OBSERVED_YET,
        GRAB_DOWNLOAD_CAME_COULD_NOT_RESOLVED,
        MANUAL_IMPORT_PATH_UNAVAILABLE,
        MANUAL_IMPORT_FILE_INVALID,
        MANUAL_IMPORT_SOURCE_ROOT_UNAVAILABLE,
        MANUAL_IMPORT_FILE_UNAVAILABLE,
        MANUAL_IMPORT_FILE_OUTSIDE_TRUSTED_SOURCE_ROOT,
        MANUAL_IMPORT_SOURCE_ROOT_NOT_FILE_DIRECTORY,
        MANUAL_IMPORT_FILE_PATH_OUTSIDE_TRUSTED_SOURCE_ROOT,
        MANUAL_IMPORT_SOURCE_NOT_FILE,
        DISC_IMPORT_TARGETS_INVALID,
        RESOURCE_NOT_FOUND,
        STATUS_ONLY_FAILED_SKIPPED_IMPORTS_RETRIED,
        LIBRARY_ACCESS_DENIED,
        DOWNLOAD_ALREADY_BEING_IMPORTED,
        CHANGED_WHILE_RETRY_REQUESTED,
        SOURCE_UNAVAILABLE,
        DOWNLOAD_BUSY_IMPORT_CHANGED,
        DOWNLOAD_NO_TITLE_AUTHORIZE_RELEASE_AGAINST,
        RELEASING_THESE_SOURCES_ALSO_RELEASES_IMPORT_TITLE_MAY_NOT_RESOLVE_IMPORTS,
        DOWNLOAD_NO_STABLE_IDENTITY_YET,
        DOWNLOAD_STILL_BEING_IMPORTED,
        NOT_HOLDING_SOURCES,
        ARCHIVE_MATCHED_TITLE_BUT_EXTRACTED_WORKSPACE_COULD_NOT_MOVED_WITHOUT_COPYING,
        ARCHIVE_REEXTRACTION_FAILED,
        MANUAL_TITLE_OUTSIDE_DURABLE_SCRYER_SUBMISSION_TITLE,
        SCENE_SUBTITLE_FILE_COUNT_LIMIT_EXCEEDED,
        SCENE_SUBTITLE_AGGREGATE_SIZE_LIMIT_EXCEEDED,
        SCENE_SUBTITLE_DELIVERY_CONTEXT_UNAVAILABLE,
        IMPORTED_VIDEO_NO_FILENAME_STEM,
        IMPORTED_VIDEO_NO_PARENT_DIRECTORY,
        MULTIPLE_SCENE_SUBTITLES_MAP_SAME_DESTINATION,
        SCENE_SUBTITLE_STAGING_WORKSPACE_UNAVAILABLE,
        INSUFFICIENT_DISK_ALLOWANCE_SCENE_SUBTITLES,
        SCENE_SUBTITLE_LIES_OUTSIDE_OWNED_WORKSPACE,
        SCENE_SUBTITLE_WORKSPACE_IDENTITY_CHANGED,
        SCENE_SUBTITLE_PATH_COMPONENT_INVALID,
        SCENE_SUBTITLE_PATH_INVALID,
        SCENE_SUBTITLE_NOT_REGULAR_FILE,
        SCENE_SUBTITLE_PATH_NO_FILENAME,
        SCENE_SUBTITLE_HANDLE_LIES_OUTSIDE_OWNED_WORKSPACE_REPARSE_POINT,
        SCENE_SUBTITLE_HANDLE_PATH_INVALID_UTF_16,
        SCENE_SUBTITLE_PATH_CONTAINS_SYMLINK,
        CLIENT_REPORTED_SUBTITLE_REPARSE_POINT,
        CLIENT_REPORTED_SUBTITLE_NOT_REGULAR_FILE,
        SCENE_SUBTITLE_DIRECTORY_NESTING_LIMIT_EXCEEDED,
        SCENE_SUBTITLE_DIRECTORY_ENTRY_LIMIT_EXCEEDED,
        SCENE_SUBTITLE_STAGING_REQUIRES_OWNED_ARCHIVE_WORKSPACE,
        SCENE_SUBTITLE_NOT_BOUNDED_REGULAR_FILE,
        SCENE_SUBTITLE_SIZE_LIMIT_EXCEEDED,
        SCENE_SUBTITLE_DESTINATION_ALREADY_EXISTS_DIFFERENT_CONTENTS,
        RECORD_CONFLICTS_COMPLETED_SOURCE_IDENTITY,
        COMPLETED_SOURCE_NO_TRUSTWORTHY_DOWNLOAD_IDENTITY,
        CLIENT_JOB_ID_NOW_BELONGS_DIFFERENT_DOWNLOAD,
        RETRY_ALREADY_RUNNING_AWAITING_RECONCILIATION,
        DOWNLOAD_FAILED,
        DOWNLOAD_ALREADY_SETTLED,
        DOWNLOAD_CLIENT_CONFIGURATION_NO_LONGER_MATCHES_SOURCE,
        CLIENT_RETURNED_DIFFERENT_DOWNLOAD_IDENTITY,
        CLIENT_DOES_NOT_REPORT_COMPLETED_DOWNLOAD,
        DOWNLOAD_STATE_UNAVAILABLE,
        NO_COMPLETED_IMPORT_ATTEMPT_UNAVAILABLE,
        NEWER_IMPORT_ATTEMPT_OWNS_DOWNLOAD,
        RETRY_IMPORT_RECORD_NOT_FOUND,
        DOWNLOAD_AWAITING_IMPORT_RETRY_RECONCILIATION,
        MEDIA_FILE_NOT_FOUND,
        SAVED_DISC_SELECTION_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED,
        MEDIA_FILE_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED,
        ARCHIVE_EXTRACTION_REQUIRES_RESOLVED_IMPORT_DESTINATION_BEFORE_STAGING_OUTPUT,
        ARCHIVE_HOLDS_ARCHIVES_NESTED_MORE_LEVELS_DEEP,
        ARCHIVE_HOLDS_MORE_NESTED_ARCHIVES,
        ARCHIVE_OUTPUT_DIRECTORY_NOT_INSIDE_STAGING_WORKSPACE,
        ARCHIVE_STAGING_OUTPUT_DIRECTORY_NOT_EMPTY,
        ARCHIVE_SOURCE_LIES_INSIDE_OWN_EXTRACTION_OUTPUT,
        ARCHIVE_SOURCE_SYMLINK_FORBIDDEN,
        DOWNLOAD_HOLDS_MORE_ARCHIVE_SETS,
        ARCHIVE_SOURCE_METADATA_EXCEEDS_ENTRY_LIMIT,
        ARCHIVE_SOURCE_METADATA_MUST_IDENTIFY_REGULAR_FILE,
        ARCHIVE_SOURCE_METADATA_ESCAPES_SOURCE_DIRECTORY,
        ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_EXTRACTION,
        ARCHIVE_PLUGIN_MANIFEST_OUTPUT_NOT_REGULAR_FILE,
        ARCHIVE_PLUGIN_MANIFEST_SIZE_MISMATCH,
        ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_ENTRIES,
        ARCHIVE_PLUGIN_OUTPUT_CONTAINS_SYMLINK,
        ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_DIRECTORIES,
        ARCHIVE_PLUGIN_OUTPUT_NOT_REGULAR_FILE,
        ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_FILES,
        ARCHIVE_PLUGIN_OUTPUT_TOO_LARGE,
        ARCHIVE_PLUGIN_OUTPUT_EXCEEDS_BYTES,
        UNSAFE_ARCHIVE_ENTRY_PATH,
        ARCHIVE_ENTRY_ESCAPES_EXTRACTION_DIRECTORY,
        ARCHIVE_SOURCE_NOT_REGULAR_DIRECTORY,
        ARCHIVE_DISCOVERY_ENTRY_LIMIT_EXCEEDED,
        ARCHIVE_DISCOVERY_DIRECTORY_DEPTH_LIMIT_EXCEEDED,
        MANUAL_IMPORT_SOURCE_CONFLICTING_CANONICAL_DOWNLOAD_IDENTITY,
        MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD,
        EXTRACTED_ARCHIVE_WORKSPACE_UNAVAILABLE,
        DISC_SELECTION_CANNOT_INCLUDE_SEPARATE_EPISODE_SERIES_MOVIE_TARGET,
        DISC_SELECTION_MAPPING_LIMIT_EXCEEDED_INVALID,
        DISC_TITLES_REQUIRE_DISTINCT_EXPLICIT_SINGLE_EPISODE_MAPPINGS,
        EPISODIC_DISCS_REQUIRE_PLAYBACK_TITLE_MAPPINGS,
        MANUAL_IMPORT_MAPPING_REQUIRES_EPISODE_ID_SERIES_MOVIE_LINK_ID,
        MANUAL_IMPORT_MAPPING_CANNOT_INCLUDE_BOTH_EPISODE_ID_SERIES_MOVIE_LINK_ID,
        MANUAL_IMPORT_CANDIDATE_REQUIRED,
        MANUAL_IMPORT_CANDIDATE_ID_REQUIRED,
        MANUAL_IMPORT_CANDIDATE_IDS_INVALID,
        MANUAL_IMPORT_EPISODE_TARGET_UNAVAILABLE,
        MANUAL_IMPORT_EPISODE_TARGET_DOES_NOT_BELONG_TITLE,
        MANUAL_IMPORT_SERIES_MOVIE_TARGET_UNAVAILABLE,
        MANUAL_IMPORT_SERIES_MOVIE_TARGET_DOES_NOT_BELONG_TITLE,
        MANUAL_IMPORT_SOURCE_ROOT_REQUIRED,
        MAPPED_DISC_EPISODE_NO_LONGER_BELONGS_IMPORT_TITLE,
        MANUAL_IMPORT_REQUEST_MISSING_ACTOR,
        MANUAL_IMPORT_REQUEST_ACTOR_NO_LONGER_EXISTS,
        CLEANUP_LOCATOR_REUSED_IDENTITY_AMBIGUOUS,
        DOWNLOAD_CLIENT_COMPLETED_DOWNLOAD_NOT_FOUND,
        DOWNLOAD_PAYLOAD_DIRECTORY_TOO_MANY_ENTRIES_INVENTORY_SAFELY,
        DOWNLOAD_PAYLOAD_DIRECTORY_NO_LONGER_REAL_DIRECTORY,
        DOWNLOAD_PAYLOAD_INVENTORY_ENTRY_NOT_SAFE_RELATIVE_PATH,
        NZBGET_CLEANUP_IDENTITY_CHANGED,
        NZBGET_TERMINAL_CLEANUP_METADATA_NOT_FOUND,
        NZBGET_NATIVE_DELETION_PATH_UNVERIFIED,
        NZBGET_NATIVE_DELETION_PARENT_UNAVAILABLE,
        NZBGET_HISTORY_REMOVAL_WOULD_DELETE_RETAINED_UNVERIFIED_DATA,
        DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_REQUIRES_CONFIGURED_CLIENT_ID,
        DOWNLOAD_CLIENT_NOT_FOUND,
        DOWNLOAD_CLIENT_PATH_MAPPING_UNUSABLE_LOCAL_ROOT,
        DOWNLOAD_CLIENT_PAYLOAD_PATH_INVALID,
        PAYLOAD_PATH_UNSAFE,
        CLEANUP_TARGET_IS_OUTPUT_ROOT,
        PAYLOAD_OVERLAPS_RECYCLE_ROOT,
        CLEANUP_TARGET_OVERLAPS_LIBRARY_ROOT,
        FILE_PAYLOAD_CLEANUP_REQUIRES_DURABLE_JOB_OWNERSHIP,
        FILE_PAYLOAD_OWNERSHIP_UNVERIFIED,
        FILE_PAYLOAD_CHANGED_SINCE_IMPORT,
        PAYLOAD_DIRECTORY_OWNERSHIP_UNVERIFIED,
        DOWNLOAD_CLIENT_PAYLOAD_PATH_NOT_FILE_DIRECTORY_SYMLINK,
        DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_CHECKPOINT_NOT_DIRECTORY,
        DOWNLOAD_CLIENT_PAYLOAD_PATH_OUTSIDE_OUTPUT_ROOT,
        DOWNLOAD_CLIENT_PAYLOAD_PATH_TRAVERSES_SYMLINK,
        DO_NOT_ACCESS_ANY_LIBRARIES,
        ACTIVE_IMPORT_STREAM_NOT_FOUND,
        UNAVAILABLE,
        CLEANUP_STATE_INVALID,
        CLEANUP_ATTRIBUTION_MISSING,
        FILE_NOT_FOUND,
        PERMISSION_DENIED,
        DISK_FULL,
        IO_FAILED,
        EPISODE_NOT_FOUND,
        EPISODE_LOOKUP_FAILED,
        SOURCE_JOB_FAILED,
    ];
}

pub mod notifications {
    /// Unknown notification event type.
    pub const EVENT_TYPE_UNKNOWN: &str = "NOTIFICATION_EVENT_TYPE_UNKNOWN";
    /// Notification event type is not subscribable.
    pub const EVENT_TYPE_NOT_SUBSCRIBABLE: &str = "NOTIFICATION_EVENT_TYPE_NOT_SUBSCRIBABLE";
    /// Channel name must not be empty.
    pub const CHANNEL_NAME_REQUIRED: &str = "NOTIFICATION_CHANNEL_NAME_REQUIRED";
    /// Invalid channel type.
    pub const CHANNEL_TYPE_INVALID: &str = "NOTIFICATION_CHANNEL_TYPE_INVALID";
    /// Media server notification targets must be managed in Media Servers.
    pub const MEDIA_SERVER_NOTIFICATION_TARGETS_INVALID: &str =
        "NOTIFICATION_MEDIA_SERVER_NOTIFICATION_TARGETS_INVALID";
    /// Notification channels cannot reference media server connections.
    pub const CHANNELS_CANNOT_REFERENCE_MEDIA_SERVER_CONNECTIONS: &str =
        "NOTIFICATION_CHANNELS_CANNOT_REFERENCE_MEDIA_SERVER_CONNECTIONS";
    /// Notification channel was not found.
    pub const CHANNEL_NOT_FOUND: &str = "NOTIFICATION_CHANNEL_NOT_FOUND";
    /// Notification provider the selected item does not support test notifications.
    pub const PROVIDER_DOES_NOT_SUPPORT_TEST_NOTIFICATIONS: &str =
        "NOTIFICATION_PROVIDER_DOES_NOT_SUPPORT_TEST_NOTIFICATIONS";
    /// No notification plugin for channel type was not found.
    pub const PLUGIN_CHANNEL_TYPE_NOT_FOUND: &str = "NOTIFICATION_PLUGIN_CHANNEL_TYPE_NOT_FOUND";
    /// Notification subscription was not found.
    pub const SUBSCRIPTION_NOT_FOUND: &str = "NOTIFICATION_SUBSCRIPTION_NOT_FOUND";
    /// Notification channel is disabled.
    pub const CHANNEL_DISABLED: &str = "NOTIFICATION_CHANNEL_DISABLED";
    /// Media server notification providers must be targeted through media server connections.
    pub const MEDIA_SERVER_NOTIFICATION_PROVIDERS_INVALID: &str =
        "NOTIFICATION_MEDIA_SERVER_NOTIFICATION_PROVIDERS_INVALID";
    /// Media server connection was not found.
    pub const MEDIA_SERVER_CONNECTION_NOT_FOUND: &str =
        "NOTIFICATION_MEDIA_SERVER_CONNECTION_NOT_FOUND";
    /// Media server connection is disabled.
    pub const MEDIA_SERVER_CONNECTION_DISABLED: &str =
        "NOTIFICATION_MEDIA_SERVER_CONNECTION_DISABLED";
    /// Notification provider the selected item is not available.
    pub const PROVIDER_UNAVAILABLE: &str = "NOTIFICATION_PROVIDER_UNAVAILABLE";
    /// Notification provider the selected item does not support event the selected item.
    pub const PROVIDER_DOES_NOT_SUPPORT_EVENT: &str =
        "NOTIFICATION_PROVIDER_DOES_NOT_SUPPORT_EVENT";
    /// Invalid media server provider the selected item.
    pub const MEDIA_SERVER_PROVIDER_INVALID: &str = "NOTIFICATION_MEDIA_SERVER_PROVIDER_INVALID";
    /// Notification channel media server connection provider does not match channel type.
    pub const CHANNEL_MEDIA_SERVER_CONNECTION_PROVIDER_DOES_NOT_MATCH_CHANNEL_TYPE: &str =
        "NOTIFICATION_CHANNEL_MEDIA_SERVER_CONNECTION_PROVIDER_DOES_NOT_MATCH_CHANNEL_TYPE";
    /// Invalid Plex media server base URL the selected item.
    pub const PLEX_MEDIA_SERVER_BASE_URL_INVALID: &str =
        "NOTIFICATION_PLEX_MEDIA_SERVER_BASE_URL_INVALID";
    /// Notification configuration must be a JSON object.
    pub const CONFIGURATION_INVALID: &str = "NOTIFICATION_CONFIGURATION_INVALID";
    /// Notification provider the selected item requires the selected item.
    pub const PROVIDER_REQUIRES: &str = "NOTIFICATION_PROVIDER_REQUIRES";
    /// Invalid notification target kind.
    pub const TARGET_KIND_INVALID: &str = "NOTIFICATION_TARGET_KIND_INVALID";
    /// Notification target id is required for target kind the selected item.
    pub const TARGET_ID_REQUIRED_TARGET_KIND: &str = "NOTIFICATION_TARGET_ID_REQUIRED_TARGET_KIND";
    /// Notification target kind is required when target id is provided.
    pub const TARGET_KIND_REQUIRED_WHEN_TARGET_ID_PROVIDED: &str =
        "NOTIFICATION_TARGET_KIND_REQUIRED_WHEN_TARGET_ID_PROVIDED";
    /// Notification subscription target is required.
    pub const SUBSCRIPTION_TARGET_REQUIRED: &str = "NOTIFICATION_SUBSCRIPTION_TARGET_REQUIRED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        EVENT_TYPE_UNKNOWN,
        EVENT_TYPE_NOT_SUBSCRIBABLE,
        CHANNEL_NAME_REQUIRED,
        CHANNEL_TYPE_INVALID,
        MEDIA_SERVER_NOTIFICATION_TARGETS_INVALID,
        CHANNELS_CANNOT_REFERENCE_MEDIA_SERVER_CONNECTIONS,
        CHANNEL_NOT_FOUND,
        PROVIDER_DOES_NOT_SUPPORT_TEST_NOTIFICATIONS,
        PLUGIN_CHANNEL_TYPE_NOT_FOUND,
        SUBSCRIPTION_NOT_FOUND,
        CHANNEL_DISABLED,
        MEDIA_SERVER_NOTIFICATION_PROVIDERS_INVALID,
        MEDIA_SERVER_CONNECTION_NOT_FOUND,
        MEDIA_SERVER_CONNECTION_DISABLED,
        PROVIDER_UNAVAILABLE,
        PROVIDER_DOES_NOT_SUPPORT_EVENT,
        MEDIA_SERVER_PROVIDER_INVALID,
        CHANNEL_MEDIA_SERVER_CONNECTION_PROVIDER_DOES_NOT_MATCH_CHANNEL_TYPE,
        PLEX_MEDIA_SERVER_BASE_URL_INVALID,
        CONFIGURATION_INVALID,
        PROVIDER_REQUIRES,
        TARGET_KIND_INVALID,
        TARGET_ID_REQUIRED_TARGET_KIND,
        TARGET_KIND_REQUIRED_WHEN_TARGET_ID_PROVIDED,
        SUBSCRIPTION_TARGET_REQUIRED,
    ];
}

pub mod services {
    /// Maintenance operation in progress.
    pub const MAINTENANCE_OPERATION_PROGRESS: &str = "SERVICES_MAINTENANCE_OPERATION_PROGRESS";
    /// You do not have permission to perform this action.
    pub const ACTION_PERMISSION_DENIED: &str = "SERVICES_ACTION_PERMISSION_DENIED";
    /// You do not have access to this library.
    pub const LIBRARY_ACCESS_DENIED: &str = "SERVICES_LIBRARY_ACCESS_DENIED";
    /// A title identity is required.
    pub const TITLE_IDENTITY_REQUIRED: &str = "SERVICES_TITLE_IDENTITY_REQUIRED";
    /// Movie metadata response missing title was not found.
    pub const MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND: &str =
        "SERVICES_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND";
    /// Title ids must be positive.
    pub const TITLE_IDS_OUT_OF_RANGE: &str = "SERVICES_TITLE_IDS_OUT_OF_RANGE";
    /// Download client item id is required.
    pub const DOWNLOAD_CLIENT_ITEM_ID_REQUIRED: &str = "SERVICES_DOWNLOAD_CLIENT_ITEM_ID_REQUIRED";
    /// Completed download was not found.
    pub const COMPLETED_DOWNLOAD_NOT_FOUND: &str = "SERVICES_COMPLETED_DOWNLOAD_NOT_FOUND";
    /// Download client  references proxy configuration , which was not found.
    pub const DOWNLOAD_CLIENT_REFERENCES_PROXY_CONFIGURATION_WHICH_NOT_FOUND: &str =
        "SERVICES_DOWNLOAD_CLIENT_REFERENCES_PROXY_CONFIGURATION_WHICH_NOT_FOUND";
    /// Download client the selected value is assigned proxy the selected value, which is disabled.
    pub const DOWNLOAD_CLIENT_ASSIGNED_PROXY_WHICH_DISABLED: &str =
        "SERVICES_DOWNLOAD_CLIENT_ASSIGNED_PROXY_WHICH_DISABLED";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        MAINTENANCE_OPERATION_PROGRESS,
        ACTION_PERMISSION_DENIED,
        LIBRARY_ACCESS_DENIED,
        TITLE_IDENTITY_REQUIRED,
        MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND,
        TITLE_IDS_OUT_OF_RANGE,
        DOWNLOAD_CLIENT_ITEM_ID_REQUIRED,
        COMPLETED_DOWNLOAD_NOT_FOUND,
        DOWNLOAD_CLIENT_REFERENCES_PROXY_CONFIGURATION_WHICH_NOT_FOUND,
        DOWNLOAD_CLIENT_ASSIGNED_PROXY_WHICH_DISABLED,
    ];
}

pub mod media {
    /// Media file was not found.
    pub const FILE_NOT_FOUND: &str = "MEDIA_FILE_NOT_FOUND";
    /// Registered media path changed before inspection.
    pub const REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION: &str =
        "MEDIA_REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION";
    /// Media source changed during inspection.
    pub const SOURCE_CHANGED_DURING_INSPECTION: &str = "MEDIA_SOURCE_CHANGED_DURING_INSPECTION";
    /// Disc episode mapping limit exceeded.
    pub const DISC_EPISODE_MAPPING_LIMIT_EXCEEDED: &str =
        "MEDIA_DISC_EPISODE_MAPPING_LIMIT_EXCEEDED";
    /// Map each complete disc title to one episode; combined episodes require separate authored playback titles.
    pub const MAP_EACH_COMPLETE_DISC_TITLE_ONE_EPISODE: &str =
        "MEDIA_MAP_EACH_COMPLETE_DISC_TITLE_ONE_EPISODE";
    /// Mapped disc title the selected value no longer exists; review the mapping.
    pub const MAPPED_DISC_TITLE_NO_LONGER_EXISTS: &str = "MEDIA_MAPPED_DISC_TITLE_NO_LONGER_EXISTS";
    /// Mapped disc title the selected value has incomplete or unsupported analysis.
    pub const MAPPED_DISC_TITLE_INCOMPLETE_UNSUPPORTED_ANALYSIS: &str =
        "MEDIA_MAPPED_DISC_TITLE_INCOMPLETE_UNSUPPORTED_ANALYSIS";
    /// Each episode and distinct playback title can occur in only one mapping.
    pub const EACH_EPISODE_DISTINCT_PLAYBACK_TITLE_OCCUR_ONLY_ONE_MAPPING: &str =
        "MEDIA_EACH_EPISODE_DISTINCT_PLAYBACK_TITLE_OCCUR_ONLY_ONE_MAPPING";
    /// Mapped episode does not belong to this title.
    pub const MAPPED_EPISODE_DOES_NOT_BELONG_TITLE: &str =
        "MEDIA_MAPPED_EPISODE_DOES_NOT_BELONG_TITLE";
    /// Mapped disc title the selected value has no established runtime.
    pub const MAPPED_DISC_TITLE_NO_ESTABLISHED_RUNTIME: &str =
        "MEDIA_MAPPED_DISC_TITLE_NO_ESTABLISHED_RUNTIME";
    /// Disc title the selected value runtime is implausible for episode the selected value.
    pub const DISC_TITLE_RUNTIME_SECONDS_IMPLAUSIBLE_EPISODE: &str =
        "MEDIA_DISC_TITLE_RUNTIME_SECONDS_IMPLAUSIBLE_EPISODE";
    /// Image has no recognized disc titles.
    pub const IMAGE_NO_RECOGNIZED_DISC_TITLES: &str = "MEDIA_IMAGE_NO_RECOGNIZED_DISC_TITLES";
    /// Episode mappings require a series title.
    pub const EPISODE_MAPPINGS_REQUIRE_SERIES_TITLE: &str =
        "MEDIA_EPISODE_MAPPINGS_REQUIRE_SERIES_TITLE";
    /// Disc episode mappings require an ISO image.
    pub const DISC_EPISODE_MAPPINGS_REQUIRE_ISO_IMAGE: &str =
        "MEDIA_DISC_EPISODE_MAPPINGS_REQUIRE_ISO_IMAGE";
    /// Disc analysis requires review before episodes can be made available.
    pub const DISC_ANALYSIS_REQUIRES_REVIEW_BEFORE_EPISODES_MADE_AVAILABLE: &str =
        "MEDIA_DISC_ANALYSIS_REQUIRES_REVIEW_BEFORE_EPISODES_MADE_AVAILABLE";
    /// Media source or selection changed during mapping; refresh and retry.
    pub const SOURCE_SELECTION_CHANGED_DURING_MAPPING: &str =
        "MEDIA_SOURCE_SELECTION_CHANGED_DURING_MAPPING";
    /// Disc selection requires an ISO image.
    pub const DISC_SELECTION_REQUIRES_ISO_IMAGE: &str = "MEDIA_DISC_SELECTION_REQUIRES_ISO_IMAGE";
    /// Invalid numeric disc title identity.
    pub const NUMERIC_DISC_TITLE_IDENTITY_INVALID: &str =
        "MEDIA_NUMERIC_DISC_TITLE_IDENTITY_INVALID";
    /// The requested disc title could not be established; review its inspection status.
    pub const REQUESTED_DISC_TITLE_COULD_NOT_ESTABLISHED: &str =
        "MEDIA_REQUESTED_DISC_TITLE_COULD_NOT_ESTABLISHED";
    /// The requested disc title does not exist.
    pub const REQUESTED_DISC_TITLE_DOES_NOT_EXIST: &str =
        "MEDIA_REQUESTED_DISC_TITLE_DOES_NOT_EXIST";
    /// Media source or selection changed during inspection; refresh and retry.
    pub const SOURCE_SELECTION_CHANGED_DURING_INSPECTION: &str =
        "MEDIA_SOURCE_SELECTION_CHANGED_DURING_INSPECTION";
    /// Unknown search language the selected value; choose a language offered by the search language picker.
    pub const SEARCH_LANGUAGE_CHOOSE_LANGUAGE_OFFERED_SEARCH_LANGUAGE_PICKER_UNKNOWN: &str =
        "MEDIA_SEARCH_LANGUAGE_CHOOSE_LANGUAGE_OFFERED_SEARCH_LANGUAGE_PICKER_UNKNOWN";
    /// At most the selected value search languages are allowed.
    pub const MOST_SEARCH_LANGUAGES_ALLOWED: &str = "MEDIA_MOST_SEARCH_LANGUAGES_ALLOWED";
    /// Native diagnostics are not compiled into this target.
    pub const NATIVE_DIAGNOSTICS_NOT_COMPILED_INTO_TARGET: &str =
        "MEDIA_NATIVE_DIAGNOSTICS_NOT_COMPILED_INTO_TARGET";
    /// No video stream was found in the selected media.
    pub const VIDEO_NOT_FOUND: &str = "MEDIA_VIDEO_NOT_FOUND";
    /// The media analysis result is invalid.
    pub const ANALYSIS_INVALID: &str = "MEDIA_ANALYSIS_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[
        FILE_NOT_FOUND,
        REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION,
        SOURCE_CHANGED_DURING_INSPECTION,
        DISC_EPISODE_MAPPING_LIMIT_EXCEEDED,
        MAP_EACH_COMPLETE_DISC_TITLE_ONE_EPISODE,
        MAPPED_DISC_TITLE_NO_LONGER_EXISTS,
        MAPPED_DISC_TITLE_INCOMPLETE_UNSUPPORTED_ANALYSIS,
        EACH_EPISODE_DISTINCT_PLAYBACK_TITLE_OCCUR_ONLY_ONE_MAPPING,
        MAPPED_EPISODE_DOES_NOT_BELONG_TITLE,
        MAPPED_DISC_TITLE_NO_ESTABLISHED_RUNTIME,
        DISC_TITLE_RUNTIME_SECONDS_IMPLAUSIBLE_EPISODE,
        IMAGE_NO_RECOGNIZED_DISC_TITLES,
        EPISODE_MAPPINGS_REQUIRE_SERIES_TITLE,
        DISC_EPISODE_MAPPINGS_REQUIRE_ISO_IMAGE,
        DISC_ANALYSIS_REQUIRES_REVIEW_BEFORE_EPISODES_MADE_AVAILABLE,
        SOURCE_SELECTION_CHANGED_DURING_MAPPING,
        DISC_SELECTION_REQUIRES_ISO_IMAGE,
        NUMERIC_DISC_TITLE_IDENTITY_INVALID,
        REQUESTED_DISC_TITLE_COULD_NOT_ESTABLISHED,
        REQUESTED_DISC_TITLE_DOES_NOT_EXIST,
        SOURCE_SELECTION_CHANGED_DURING_INSPECTION,
        SEARCH_LANGUAGE_CHOOSE_LANGUAGE_OFFERED_SEARCH_LANGUAGE_PICKER_UNKNOWN,
        MOST_SEARCH_LANGUAGES_ALLOWED,
        NATIVE_DIAGNOSTICS_NOT_COMPILED_INTO_TARGET,
        VIDEO_NOT_FOUND,
        ANALYSIS_INVALID,
    ];
}

pub mod scripts {
    /// The script schedule is invalid.
    pub const SCHEDULE_INVALID: &str = "SCRIPTS_SCHEDULE_INVALID";
    /// Every reason owned by this policy area.
    pub const ALL: &[&str] = &[SCHEDULE_INVALID];
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_area_reason_is_distinct_and_screaming_snake() {
        let areas: &[(&str, &[&str])] = &[
            ("METADATA_GATEWAY_", metadata_gateway::ALL),
            ("SCRIPTS_", scripts::ALL),
            ("LIBRARY_", library::ALL),
            ("EXTERNAL_IMPORT_", external_import::ALL),
            ("CATALOG_", catalog::ALL),
            ("SECURITY_", security::ALL),
            ("RULE_", rules::ALL),
            ("OAUTH_", oauth::ALL),
            ("MAINTENANCE_", maintenance::ALL),
            ("ACQUISITION_", acquisition::ALL),
            ("LOCATION_", location::ALL),
            ("PLUGIN_", plugins::ALL),
            ("PORTS_", ports::ALL),
            ("FOLDER_OWNERSHIP_", folder_ownership::ALL),
            ("INDEXER_CATEGORY_", indexer_category::ALL),
            ("DOWNLOAD_PATH_MAPPING_", download_client_path_mappings::ALL),
            ("MEDIA_REQUEST_", media_requests::ALL),
            ("API_KEY_", api_keys::ALL),
            ("FS_SAFETY_", fs_safety::ALL),
            ("AUTHORIZATION_", authorization::ALL),
            ("DOWNLOAD_CLIENT_CONFIG_", download_client_config::ALL),
            ("SETTINGS_", settings::ALL),
            ("REQUEST_RULE_", request_rules::ALL),
            ("QUALITY_", quality::ALL),
            ("LIST_", lists::ALL),
            ("MEDIA_SERVER_", media_servers::ALL),
            ("INTEGRATION_", integration::ALL),
            ("DISCOVERY_", discovery::ALL),
            ("APPLICATION_UPGRADE_", application_upgrade::ALL),
            ("SUBTITLES_", subtitles::ALL),
            ("JOB_", jobs::ALL),
            ("EVENTS_", events::ALL),
            ("IMPORT_", import::ALL),
            ("NOTIFICATION_", notifications::ALL),
            ("SERVICES_", services::ALL),
            ("MEDIA_", media::ALL),
        ];
        let mut distinct = std::collections::BTreeSet::new();
        for (prefix, codes) in areas {
            for code in *codes {
                assert!(distinct.insert(*code), "duplicate reason {code}");
                // These resource/auth families predate the area registry.
                assert!(
                    code.starts_with(prefix)
                        || code.starts_with("TITLE_")
                        || code.starts_with("RELEASE_TOKEN_")
                        || code.starts_with("EXTERNAL_ACCOUNT_"),
                    "{code}"
                );
                assert!(
                    code.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                    "{code}"
                );
                assert!(!code.ends_with('_') && !code.contains("__"), "{code}");
            }
        }
    }
}
