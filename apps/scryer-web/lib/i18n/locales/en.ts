import type { LocaleDictionary } from "../types";

const en: LocaleDictionary = {
  "error.reason.METADATA_GATEWAY_CLOCK_SKEW":
    "Scryer cannot connect to the metadata gateway because your system clock is off by more than 5 minutes. Synchronize your system clock and restart Scryer.",
  "initialAuth.title": "Set up access to Scryer",
  "initialAuth.description":
    "Before continuing, choose whether this instance requires sign-in.",
  "initialAuth.choose": "How should people access Scryer?",
  "initialAuth.passwordMode": "Require an administrator password",
  "initialAuth.noAuthMode": "Continue without authentication",
  "initialAuth.environment":
    "Your startup environment restricts the available authentication mode.",
  "initialAuth.admin":
    "Set the password for admin. Use at least {{minLength}} characters.",
  "initialAuth.password": "New password",
  "initialAuth.confirm": "Confirm password",
  "initialAuth.mismatch": "The passwords do not match.",
  "initialAuth.acknowledge":
    "I understand that anyone allowed to reach this unauthenticated instance can act as administrator. Existing network restrictions still apply.",
  "initialAuth.code": "Setup code",
  "initialAuth.codeHelp":
    "Find the one-time code in the server console or container/service output. On desktop, open logs/console.log in Scryer’s data folder. Restarting Scryer replaces the code.",
  "initialAuth.continue": "Save password and continue",
  "initialAuth.continueWithout": "Confirm no authentication and continue",
  "initialAuth.saving": "Saving…",
  "initialAuth.loadFailed":
    "Authentication setup status could not be loaded. Retry to continue.",
  "initialAuth.retry": "Retry",
  "initialAuth.saveFailed": "Authentication setup could not be saved.",
  "initialAuth.required": "Complete authentication setup before continuing.",
  "initialAuth.alreadyCompleted":
    "Authentication setup has already been completed. Refresh to continue.",
  "initialAuth.replaceRequired":
    "Sign in with the temporary administrator password and replace it.",
  "initialAuth.invalidCode":
    "Enter the setup code from the server console or, on desktop, logs/console.log in Scryer’s data folder.",
  "initialAuth.rateLimited":
    "Too many incorrect setup codes. Wait one minute and try again.",
  "initialAuth.modeConflict":
    "This choice conflicts with the startup environment or the supplied password.",
  "initialAuth.acknowledgeRequired":
    "Confirm that you want to continue without authentication.",
  "initialAuth.sessionExpired":
    "This temporary session has expired. Sign in again to continue.",
  "initialAuth.replacementTitle": "Choose a new password",
  "initialAuth.replacementDescription":
    "Sign-in used a temporary administrator password. Replace it with a different password that only you know.",
  "initialAuth.showPasswords": "Show passwords",
  "initialAuth.signOut": "Sign out",
  "error.reason.EXTERNAL_ACCOUNT_LINK_FAILED":
    "The external account could not be linked.",
  "error.reason.CATALOG_TAG_INVALID": "The title tag is invalid.",
  "error.reason.SETTINGS_DELAY_PROFILE_CATALOG_INVALID":
    "The delay profile catalog is invalid.",
  "error.reason.SETTINGS_TRUSTED_PROXY_POLICY_INVALID":
    "The trusted proxy policy is invalid.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_INVALID":
    "The seeding profile is invalid.",
  "error.reason.PLUGIN_DESCRIPTOR_SDK_INCOMPATIBLE":
    "The plugin descriptor is incompatible with this SDK.",
  "error.reason.PLUGIN_DESCRIPTOR_HOST_PERMISSIONS_INVALID":
    "The plugin descriptor requests invalid host permissions.",
  "error.reason.HTTP_WEBSOCKET_API_KEY_UNSUPPORTED":
    "API keys are not supported for WebSocket connections.",
  "error.reason.HTTP_WEBSOCKET_CLIENT_PROOF_REQUIRED":
    "Scryer web client proof is required for unauthenticated WebSocket access.",

  "error.reason.PLUGIN_ARTIFACT_DESTINATION_FORBIDDEN":
    "The artifact URL resolves to a private or otherwise forbidden destination.",
  "error.reason.PLUGIN_ARTIFACT_HOST_UNAVAILABLE":
    "The artifact host could not be resolved.",

  "error.reason.INDEXER_CATEGORY_MISMATCH":
    "The indexer category does not match the release subject.",

  "importHistory.skipReason.SCENE_SUBTITLE_PENDING":
    "Scene subtitles are still being prepared",

  "importHistory.skipReason.ALREADY_IMPORTED": "Already imported",
  "importHistory.skipReason.DUPLICATE_FILE": "Duplicate file",
  "importHistory.skipReason.POST_DOWNLOAD_RULE_BLOCKED":
    "Blocked by a post-download rule",
  "importHistory.skipReason.POLICY_MISMATCH":
    "Does not match the import policy",
  "importHistory.skipReason.UNRESOLVED_IDENTITY":
    "Title identity could not be resolved",
  "importHistory.skipReason.UNPARSEABLE_EPISODE":
    "Episode could not be identified",
  "importHistory.skipReason.NO_VIDEO_FILES": "No video files found",
  "importHistory.skipReason.UNWANTED_EXECUTABLES":
    "Download contains unwanted executables",
  "importHistory.skipReason.DOWNLOAD_IN_PROGRESS":
    "Download is still in progress",
  "importHistory.skipReason.SOURCE_CHANGED": "Source changed after inspection",
  "importHistory.skipReason.SOURCE_UNAVAILABLE":
    "Source is temporarily unavailable",
  "importHistory.skipReason.DISK_FULL": "Insufficient disk space",
  "importHistory.skipReason.PERMISSION_DENIED": "Permission denied",
  "importHistory.skipReason.PASSWORD_REQUIRED": "Archive password required",
  "importHistory.skipReason.ARCHIVE_EXTRACTION_PLUGIN_REQUIRED":
    "Archive extractor required",
  "importHistory.skipReason.ARCHIVE_EXTRACTION_TIMED_OUT":
    "Archive extraction timed out",

  "error.reason.LIST_SOURCE_RATE_LIMITED":
    "The list provider asked for fewer requests.",
  "error.reason.LIST_SOURCE_UNAVAILABLE":
    "The list provider could not be reached.",
  "error.reason.LIST_SOURCE_FAILED":
    "The provider did not return a readable list.",

  "error.fallback.accountSecurityVerificationFailed":
    "Account security verification failed.",
  "error.fallback.connectedAppCouldNotBeRevoked":
    "Connected app could not be revoked.",
  "error.fallback.connectedAppsCouldNotBeLoaded":
    "Connected apps could not be loaded.",
  "error.fallback.connectionFailed": "Connection failed",
  "error.fallback.connectionSucceededButProwlarrDiscoveryCouldNotStart":
    "Connection succeeded, but Prowlarr discovery could not start",
  "error.fallback.connectionTestFailed": "Connection test failed",
  "error.fallback.embyConnectTestFailed": "Emby Connect test failed",
  "error.fallback.failedToCompleteSetup": "Failed to complete setup",
  "error.fallback.failedToCreateLibrary": "Failed to create library",
  "error.fallback.failedToFinalizeImport": "Failed to finalize import",
  "error.fallback.failedToLoadImportActivity":
    "Failed to load import activity.",
  "error.fallback.failedToLoadImportProgress": "Failed to load import progress",
  "error.fallback.failedToLoadPreview": "Failed to load preview",
  "error.fallback.failedToLoadProwlarrDiscoveryStatus":
    "Failed to load Prowlarr discovery status",
  "error.fallback.failedToLoadQueue": "Failed to load queue.",
  "error.fallback.failedToLoadWarmupProgress": "Failed to load warmup progress",
  "error.fallback.failedToRestartProwlarrDiscovery":
    "Failed to restart Prowlarr discovery",
  "error.fallback.failedToSave": "Failed to save",
  "error.fallback.failedToUpdateSponsorPreference":
    "Failed to update Sponsor preference.",
  "error.fallback.importFailed": "Import failed",
  "error.fallback.theCreateResultIsUncertainReloadOAuthApplicationsBeforeTryingAgain":
    "The create result is uncertain. Reload OAuth applications before trying again.",
  "error.fallback.thisOAuthRequestHasAnInvalidClientOrRedirectURI":
    "This OAuth request has an invalid client or redirect URI.",
  "error.fallback.unableToAuthorizeThisIntegration":
    "Unable to authorize this integration.",
  "error.fallback.unableToCancelImport": "Unable to cancel import.",
  "error.fallback.unableToCopyTheAPIKey": "Unable to copy the API key.",
  "error.fallback.unableToCreateAPIKey": "Unable to create API key.",
  "error.fallback.unableToDeleteOAuthApplication":
    "Unable to delete OAuth application.",
  "error.fallback.unableToLoadAPIKeys": "Unable to load API keys.",
  "error.fallback.unableToLoadOAuthApplications":
    "Unable to load OAuth applications.",
  "error.fallback.unableToRevokeAPIKey": "Unable to revoke API key.",
  "error.fallback.unableToSaveOAuthApplication":
    "Unable to save OAuth application.",

  "error.reason.ACQUISITION_ANOTHER_SEARCH_RUNNING":
    "Another search is running. Wait for it to finish or cancel it before starting this search.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_BUDGET_EXHAUSTED":
    "The download artifact redirect loop was exhausted.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_LIMIT_EXCEEDED":
    "The download artifact exceeded the redirect limit.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_LOOP":
    "The download artifact redirect looped.",
  "error.reason.ACQUISITION_ARTIFACT_REDIRECT_SCHEME_UNSUPPORTED":
    "Download artifact redirects must use HTTP(S) or magnet URLs.",
  "error.reason.ACQUISITION_ARTIFACT_RESOLVER_UNAVAILABLE":
    "Artifact download is not supported: no indexer artifact resolver is wired.",
  "error.reason.ACQUISITION_AUTOMATIC_SEARCH_REQUIRES_TITLE":
    "Automatic search requires a title.",
  "error.reason.ACQUISITION_AUTOMATIC_SEARCH_REQUIRES_TITLE_CANNOT_SELECT_WANTED_ITEM":
    "Automatic search requires a title and cannot select a Wanted item.",
  "error.reason.ACQUISITION_BANDWIDTH_LIMIT_NEGATIVE":
    "The bandwidth limit must be zero or greater.",
  "error.reason.ACQUISITION_CANONICAL_DOWNLOAD_SUBMISSION_REQUIRES_DOWNLOAD_ID":
    "Canonical download submission requires a download id.",
  "error.reason.ACQUISITION_CLIENT_ID_REQUIRED": "Client id is required.",
  "error.reason.ACQUISITION_CONNECTION_TEST_UNSUPPORTED":
    "Connection testing is not supported for this download client type.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_REDIRECT_LOCATION_INVALID":
    "Download artifact redirect location was invalid.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_REDIRECT_MISSING_LOCATION":
    "Download artifact redirect was missing a location.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_REQUIRED":
    "Release has no download artifact to fetch.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_TYPE_UNSUPPORTED":
    "Resolved the download URL, but the result was not an NZB, magnet URI, or torrent file.",
  "error.reason.ACQUISITION_DOWNLOAD_ARTIFACT_URL_INVALID":
    "Download artifact URL is invalid.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_BASE_URL_INVALID":
    "Download client has no valid base URL.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_CONFIG_INVALID":
    "Invalid download client config JSON.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND":
    "The download client config could not be found.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_CREDENTIALS_REQUIRED":
    "Download client (sabnzbd) requires an API key or username/password.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_DISABLED":
    "Download client is disabled.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_HOST_REQUIRED":
    "Cannot compute base URL from config - host is required.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_ITEM_NOT_FOUND":
    "The download client item could not be found.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_NOT_FOUND":
    "The download client could not be found.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_TYPE_NOT_FOUND":
    "No download client is configured for this type.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_TYPE_UNSUPPORTED":
    "This download client type is not supported.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_UNAVAILABLE":
    "Download client is missing or disabled.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_UPDATE_EMPTY":
    "At least one download client config field must be provided.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_CREDENTIALS_FORBIDDEN":
    "URL must not include embedded credentials.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_HOST_REQUIRED":
    "URL must include a host.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_INVALID": "Invalid URL.",
  "error.reason.ACQUISITION_DOWNLOAD_CLIENT_URL_SCHEME_UNSUPPORTED":
    "URL must use http or https.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_NOT_ABSOLUTE":
    "The HTTP downloader requires an absolute download directory path.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_NOT_DIRECTORY":
    "The download directory is not a directory.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_NOT_WRITABLE":
    "The download directory is not writable.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_REQUIRED":
    "The HTTP downloader needs a download directory.",
  "error.reason.ACQUISITION_DOWNLOAD_DIRECTORY_UNREACHABLE":
    "The download directory is not reachable.",
  "error.reason.ACQUISITION_DOWNLOAD_NOT_PAUSABLE":
    "A download cannot be paused.",
  "error.reason.ACQUISITION_DOWNLOAD_NOT_RESUMABLE":
    "A download cannot be resumed.",
  "error.reason.ACQUISITION_DOWNLOAD_ORIGIN_MISMATCH":
    "Proxied download URL does not match the assigned indexer origin.",
  "error.reason.ACQUISITION_DOWNLOAD_PROXY_DISABLED":
    "The assigned download client proxy is disabled.",
  "error.reason.ACQUISITION_DOWNLOAD_PROXY_NOT_FOUND":
    "The download proxy could not be found.",
  "error.reason.ACQUISITION_DOWNLOAD_ROUTING_STALE":
    "Selected download client is no longer eligible. Refresh routing and retry.",
  "error.reason.ACQUISITION_DOWNLOAD_SOURCE_NOT_NZB_URL":
    "The download source must be an NZB URL.",
  "error.reason.ACQUISITION_DOWNLOAD_SOURCE_REQUIRED":
    "Source hint is required to queue a download.",
  "error.reason.ACQUISITION_DOWNLOAD_SUBMISSION_NOT_FOUND":
    "Download submission was not found.",
  "error.reason.ACQUISITION_EPISODE_VALUE_INVALID": "Invalid episode value.",
  "error.reason.ACQUISITION_HTTP_ARTIFACT_REDIRECT_SCHEME_UNSUPPORTED":
    "Download artifact redirects must use HTTP(S) URLs.",
  "error.reason.ACQUISITION_HTTP_DOWNLOAD_ALREADY_SUBMITTED":
    "This download was already submitted to the HTTP downloader.",
  "error.reason.ACQUISITION_HTTP_DOWNLOAD_NOT_FOUND":
    "The HTTP download could not be found.",
  "error.reason.ACQUISITION_HTTP_QUEUE_NOT_WRITABLE":
    "The HTTP downloader queue is not writable.",
  "error.reason.ACQUISITION_HTTP_QUEUE_UNREADABLE":
    "The HTTP downloader queue cannot be inspected.",
  "error.reason.ACQUISITION_HTTP_SETTING_OUT_OF_RANGE":
    "An HTTP downloader setting is outside its allowed range.",
  "error.reason.ACQUISITION_ID_SEARCH_FACET_UNSUPPORTED":
    "Unsupported ID search facet.",
  "error.reason.ACQUISITION_INDEXER_ARTIFACT_SOURCE_REQUIRED":
    "An indexer artifact source is required.",
  "error.reason.ACQUISITION_INDEXER_CAPS_BASE_URL_INVALID":
    "Indexer caps base_url is invalid.",
  "error.reason.ACQUISITION_INDEXER_CAPS_BASE_URL_REQUIRED":
    "Indexer caps refresh requires a base_url.",
  "error.reason.ACQUISITION_INDEXER_CAPS_PROXY_REQUIRED":
    "Caps refresh for indexer was not given its assigned proxy; refusing to send it directly.",
  "error.reason.ACQUISITION_INDEXER_CONFIG_JSON_INVALID":
    "Indexer config_json is invalid.",
  "error.reason.ACQUISITION_INDEXER_CONFIG_NOT_FOUND":
    "The indexer config could not be found.",
  "error.reason.ACQUISITION_INDEXER_ERROR_CURSOR_INVALID":
    "Invalid indexer error cursor.",
  "error.reason.ACQUISITION_INDEXER_MAPPING_CHANGED":
    "Indexer mapping changed; refresh routing and retry.",
  "error.reason.ACQUISITION_INDEXER_NOT_FOUND":
    "The indexer could not be found.",
  "error.reason.ACQUISITION_INDEXER_PROVIDER_UNSUPPORTED":
    "This indexer provider is not supported.",
  "error.reason.ACQUISITION_INDEXER_PROXY_DISABLED":
    "Proxy is disabled for this indexer.",
  "error.reason.ACQUISITION_INDEXER_PROXY_NOT_FOUND":
    "The indexer proxy could not be found.",
  "error.reason.ACQUISITION_INDEXER_RESOLVED_DIFFERENT_ITEM":
    "The indexer resolved a different item; refusing to submit.",
  "error.reason.ACQUISITION_INDEXER_RESOLVED_DIFFERENT_PAGE":
    "The indexer resolved a different page; refusing to submit.",
  "error.reason.ACQUISITION_INDEXER_ROUTING_UNAVAILABLE":
    "Indexer routing is unavailable.",
  "error.reason.ACQUISITION_INDEXER_SETTINGS_STALE":
    "Indexer settings changed during validation; reload and try again.",
  "error.reason.ACQUISITION_INDEXER_UPDATE_EMPTY":
    "At least one indexer config field must be provided.",
  "error.reason.ACQUISITION_MAGNET_REQUIRES_TORRENT_PROTOCOL":
    "A magnet requires the torrent protocol.",
  "error.reason.ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_ACCEPTED_RECEIPT_INTENT":
    "Maintenance Search dispatch requires an accepted receipt intent.",
  "error.reason.ACQUISITION_MAINTENANCE_SEARCH_DISPATCH_REQUIRES_PREALLOCATED_JOB_RUN_ID":
    "Maintenance Search dispatch requires a preallocated job run id.",
  "error.reason.ACQUISITION_MATCHED_TITLE_NO_LONGER_EXISTS_NOT_FOUND":
    "Matched title was not found.",
  "error.reason.ACQUISITION_MATCHING_EPISODES_PAUSED_ALREADY_DOWNLOADING":
    "The matching episodes are paused or already downloading. Resume paused acquisition or wait for the download to finish.",
  "error.reason.ACQUISITION_NO_ROUTABLE_DOWNLOAD_CLIENT":
    "No enabled compatible download client is allowed by routing.",
  "error.reason.ACQUISITION_NZBGET_JOB_ID_INVALID": "Invalid nzbget job ID.",
  "error.reason.ACQUISITION_NZBGET_METHOD_UNAVAILABLE":
    "NZBGet does not support this request signature.",
  "error.reason.ACQUISITION_NZBGET_QUEUE_ID_INVALID":
    "Invalid nzbget queue id.",
  "error.reason.ACQUISITION_NZB_ARTIFACT_TOO_LARGE":
    "The resolved NZB artifact exceeds the size limit.",
  "error.reason.ACQUISITION_NZB_COMPRESSION_UNSUPPORTED":
    "NZB download payload is a file; only plain, gzip, zlib or zstd NZBs can be staged.",
  "error.reason.ACQUISITION_NZB_DECOMPRESSION_FAILED":
    "The compressed NZB payload could not be decompressed.",
  "error.reason.ACQUISITION_NZB_PAYLOAD_NOT_XML":
    "NZB download payload did not look like xml.",
  "error.reason.ACQUISITION_NZB_ROOT_INVALID":
    "NZB download payload root element must be <nzb>.",
  "error.reason.ACQUISITION_NZB_TEXT_DECODE_FAILED":
    "NZB XML text decode failed.",
  "error.reason.ACQUISITION_NZB_TRAILING_CONTENT":
    "NZB download payload contains content outside its root element.",
  "error.reason.ACQUISITION_NZB_XML_INVALID":
    "NZB download payload was not valid xml: unexpected end of file.",
  "error.reason.ACQUISITION_OUTPUT_FILENAME_INVALID":
    "The output name is not a plain file name.",
  "error.reason.ACQUISITION_PASSWORD_RETRY_INPUT_REQUIRED":
    "Job ID and replacement password are required.",
  "error.reason.ACQUISITION_PAUSED_CONTENT_COVERED_RELEASE":
    "Acquisition is paused for content covered by this release. Resume it before searching.",
  "error.reason.ACQUISITION_PAUSED_WHILE_SEARCH_WAITING":
    "Acquisition was paused while this search was waiting. Resume it before searching.",
  "error.reason.ACQUISITION_PROWLARR_AUTH_FAILED":
    "Prowlarr rejected the configured credentials.",
  "error.reason.ACQUISITION_PROWLARR_CONFIG_INVALID":
    "The Prowlarr configuration is invalid.",
  "error.reason.ACQUISITION_PROWLARR_PARENT_SEARCH_UNSUPPORTED":
    "Prowlarr parent configs are management-only; search through synced child indexers.",
  "error.reason.ACQUISITION_PROXY_CONFIG_NOT_FOUND":
    "The proxy config could not be found.",
  "error.reason.ACQUISITION_PROXY_REPOSITORY_UNAVAILABLE":
    "Download client is assigned a proxy but the proxy repository is not wired.",
  "error.reason.ACQUISITION_RECOVERY_SEARCH_JOB_NOT_FOUND":
    "Recovery search job was not found.",
  "error.reason.ACQUISITION_RELEASE_DOWNLOAD_URL_INVALID":
    "Release download URL is invalid.",
  "error.reason.ACQUISITION_RELEASE_FLAG_UNSUPPORTED":
    "Unsupported release flag.",
  "error.reason.ACQUISITION_RELEASE_NAME_SOURCE_REQUIRED_INVALID":
    "Release name and source are required and must be bounded.",
  "error.reason.ACQUISITION_RELEASE_PROTOCOL_REQUIRED":
    "A selected client requires a release protocol.",
  "error.reason.ACQUISITION_RELEASE_SIZE_OUT_OF_RANGE":
    "Release size must not be negative.",
  "error.reason.ACQUISITION_RELEASE_URL_FLAG_LIST_EXCEEDS_INPUT_LIMIT":
    "Release URL or flag list exceeds the input limit.",
  "error.reason.ACQUISITION_RELEASE_URL_INVALID":
    "Release URL must use HTTP, HTTPS, or a valid magnet.",
  "error.reason.ACQUISITION_REPLACEMENT_PASSWORD_REQUIRED":
    "Replacement password is required.",
  "error.reason.ACQUISITION_RESOLVED_NZB_INVALID":
    "Resolved invalid NZB bytes.",
  "error.reason.ACQUISITION_ROUTED_DOWNLOAD_CLIENT_NOT_FOUND":
    "The routed download client could not be found.",
  "error.reason.ACQUISITION_SABNZBD_CREDENTIALS_REQUIRED":
    "SABnzbd requires an API key or username/password.",
  "error.reason.ACQUISITION_SCOPE_NOT_FOUND":
    "No acquisition scope for was not found.",
  "error.reason.ACQUISITION_SEARCH_ACTOR_DISABLED": "Search actor is disabled.",
  "error.reason.ACQUISITION_SEARCH_ACTOR_NO_LONGER_EXISTS":
    "Search actor no longer exists.",
  "error.reason.ACQUISITION_SEARCH_ACTOR_UNAVAILABLE":
    "Search actor is unavailable.",
  "error.reason.ACQUISITION_SEARCH_FACET_INVALID":
    "Invalid saved search facet.",
  "error.reason.ACQUISITION_SEARCH_FACET_REQUIRED": "Search facet is required.",
  "error.reason.ACQUISITION_SEARCH_FACET_UNSUPPORTED":
    "Unsupported search facet.",
  "error.reason.ACQUISITION_SEARCH_RUNNING":
    "An acquisition search is already running.",
  "error.reason.ACQUISITION_SEARCH_VERSION_UNSUPPORTED":
    "Unsupported saved search version.",
  "error.reason.ACQUISITION_SEASON_EPISODE_INVALID":
    "Season and episode must include numeric values.",
  "error.reason.ACQUISITION_SEASON_EPISODE_REQUIRED":
    "Season and episode are required.",
  "error.reason.ACQUISITION_SEASON_INVALID": "Season must be a whole number.",
  "error.reason.ACQUISITION_SEASON_PACK_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE_EXTERNAL_IDS":
    "Season pack search subject has no searchable title or external IDs.",
  "error.reason.ACQUISITION_SEASON_VALUE_INVALID": "Invalid season value.",
  "error.reason.ACQUISITION_SEEDING_PROFILE_NAME_CONFLICT":
    "A seeding profile with this name already exists.",
  "error.reason.ACQUISITION_SEEDING_PROFILE_NOT_FOUND":
    "The seeding profile could not be found.",
  "error.reason.ACQUISITION_SELECTED_LIBRARY_UNAVAILABLE_INCOMPATIBLE_ENDPOINT":
    "The selected library is unavailable or incompatible with this endpoint.",
  "error.reason.ACQUISITION_SELECTED_SEASON_NO_LONGER_MATCHES_TITLE":
    "The selected season no longer matches this title. Refresh the title and try again.",
  "error.reason.ACQUISITION_SERIES_MOVIE_SEARCH_SUBJECT_NO_SEARCHABLE_TITLE":
    "Series movie search subject has no searchable title.",
  "error.reason.ACQUISITION_STAGED_PAYLOAD_TOO_LARGE":
    "The payload exceeds the staging size limit.",
  "error.reason.ACQUISITION_TITLE_BEING_MOVED_SEARCH_UNAVAILABLE":
    "This title is being moved. Search is available when the move finishes.",
  "error.reason.ACQUISITION_TITLE_NOT_FOUND": "Title was not found.",
  "error.reason.ACQUISITION_TITLE_NO_LONGER_EXISTS_NOT_FOUND":
    "The selected title was not found.",
  "error.reason.ACQUISITION_TITLE_NO_NAME_EXTERNAL_IDS":
    "Title has no name or external IDs.",
  "error.reason.ACQUISITION_TRANSPORT_PROXY_INVALID":
    "The assigned transport proxy configuration is invalid.",
  "error.reason.ACQUISITION_WANTED_ITEM_NOT_FOUND":
    "Wanted item was not found.",
  "error.reason.ACQUISITION_WANTED_KIND_INVALID": "Invalid saved wanted kind.",
  "error.reason.ACQUISITION_WEAVER_JOB_IDENTITY_INVALID":
    "Invalid Weaver job identity.",
  "error.reason.ACQUISITION_WEAVER_JOB_ID_INVALID": "Invalid Weaver job id.",
  "error.reason.API_KEY_API_KEYS_RESTRICTED_USERS_WHO_MANAGE_SYSTEM_SETTINGS":
    "API keys are restricted to users who manage system settings.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_DECLARATIONS_INVALID":
    "Development API key declarations must have unique lookup IDs.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_DECLARATION_INVALID":
    "Invalid development API key declaration.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_LOOKUP_ID_COLLIDES_USER_MANAGED_KEY":
    "Development API key lookup ID collides with a user-managed key.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_LOOKUP_ID_DIFFERENT_SECRET":
    "Development API key lookup ID has a different secret.",
  "error.reason.API_KEY_DEVELOPMENT_API_KEY_OWNER_DOES_NOT_EXIST":
    "Development API key owner does not exist.",
  "error.reason.API_KEY_ENVIRONMENT_SEEDED_API_KEYS_MANAGED_SCRYER_DEV_API_KEYS":
    "Environment-seeded API keys are managed by SCRYER_DEV_API_KEYS.",
  "error.reason.API_KEY_EXPIRED_REVOKED": "API key is expired or revoked.",
  "error.reason.API_KEY_LABEL_CONTROL_CHARACTER":
    "API key label must not contain control characters.",
  "error.reason.API_KEY_LABEL_REQUIRED": "API key label is required.",
  "error.reason.API_KEY_LABEL_TOO_LONG":
    "API key label must not exceed the selected value characters.",
  "error.reason.API_KEY_OWNER_DISABLED": "API key owner is disabled.",
  "error.reason.API_KEY_OWNER_NO_LONGER_EXISTS":
    "API key owner no longer exists.",
  "error.reason.API_KEY_VALUE_INVALID": "Invalid API key.",
  "error.reason.APPLICATION_UPGRADE_APPLICATION_UPDATE_NOTICE_UNAVAILABLE":
    "The application update notice is not available.",
  "error.reason.APPLICATION_UPGRADE_BACKUP_ALREADY_EXISTS":
    "Refusing to overwrite existing application backup the selected item.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDTAG_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE":
    "ExpectedTag does not match the current application update notice.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDTAG_REQUIRED":
    "ExpectedTag must not be empty.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDVERSION_DOES_NOT_MATCH_CURRENT_APPLICATION_UPDATE_NOTICE":
    "ExpectedVersion does not match the current application update notice.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDVERSION_INVALID":
    "ExpectedVersion must be strictly newer than the running version.",
  "error.reason.APPLICATION_UPGRADE_EXPECTEDVERSION_REQUIRED":
    "ExpectedVersion must not be empty.",
  "error.reason.APPLICATION_UPGRADE_INSTALLATION_UNAVAILABLE":
    "Application upgrade installation is not eligible.",
  "error.reason.APPLICATION_UPGRADE_INSTALLER_DIGEST_INVALID":
    "Invalid installer digest.",
  "error.reason.APPLICATION_UPGRADE_INSUFFICIENT_FREE_SPACE_APPLICATION_UPGRADE":
    "Insufficient free space for application upgrade: need the selected value bytes, have the selected value bytes.",
  "error.reason.APPLICATION_UPGRADE_JOB_ALREADY_RUNNING":
    "An application upgrade job is already running.",
  "error.reason.APPLICATION_UPGRADE_JOURNAL_PHASE_UNSUPPORTED":
    "Unsupported application upgrade journal phase the selected item.",
  "error.reason.APPLICATION_UPGRADE_JOURNAL_SCHEMA_UNSUPPORTED":
    "Unsupported application upgrade journal schema the selected item.",
  "error.reason.APPLICATION_UPGRADE_NO_APPLICATION_UPDATE_NOTICE_UNAVAILABLE":
    "No application update notice is available.",
  "error.reason.APPLICATION_UPGRADE_RUNNING_EXECUTABLE_NO_PARENT_DIRECTORY":
    "Running executable has no parent directory.",
  "error.reason.APPLICATION_UPGRADE_RUN_NOT_FOUND":
    "Application upgrade run was not found.",
  "error.reason.APPLICATION_UPGRADE_UPGRADE_MANIFEST_TAG_DOES_NOT_MATCH_EXPECTEDTAG":
    "Upgrade manifest tag does not match expectedTag.",
  "error.reason.APPLICATION_UPGRADE_UPGRADE_MANIFEST_VERSION_DOES_NOT_MATCH_EXPECTEDVERSION":
    "Upgrade manifest version does not match expectedVersion.",
  "error.reason.AUTHORIZATION_ACTION_PERMISSION_DENIED":
    "You do not have permission to perform this action.",
  "error.reason.AUTHORIZATION_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_CANNOT_REPLACE_EXISTING_MEDIA":
    "Additional-file queueing cannot replace existing media.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_DOES_NOT_SUPPORT_COLLECTION_SCOPES_YET":
    "Additional-file queueing does not support collection scopes yet.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_REQUIRES_TITLE_EPISODE_SCOPE":
    "Additional-file queueing requires a title or episode scope.",
  "error.reason.CATALOG_ADDITIONAL_FILE_QUEUEING_SUPPORTS_ONLY_TITLE_SINGLE_EPISODE_SCOPES":
    "Additional-file queueing supports only title and single-episode scopes.",
  "error.reason.CATALOG_ADDITIONAL_FILE_TITLE_QUEUEING_SUPPORTS_ONLY_MOVIE_TITLES":
    "Additional-file title queueing supports only movie titles.",
  "error.reason.CATALOG_BEST_RELEASE_SEARCH_UNSUPPORTED":
    "Best-release search is not supported for collection scopes.",
  "error.reason.CATALOG_BLOCKLIST_ENTRY_NOT_FOUND":
    "Blocklist entry was not found.",
  "error.reason.CATALOG_CHANGING_TITLE_FACET_UNSUPPORTED":
    "Changing a title facet is not supported because titles cannot move between libraries.",
  "error.reason.CATALOG_COLLECTION_FIELD_REQUIRED":
    "At least one collection field must be provided.",
  "error.reason.CATALOG_COLLECTION_INDEX_REQUIRED":
    "Collection index is required.",
  "error.reason.CATALOG_COLLECTION_NOT_FOUND": "Collection was not found.",
  "error.reason.CATALOG_COLLECTION_TYPE_REQUIRED":
    "Collection type is required.",
  "error.reason.CATALOG_COLLECTION_TYPE_UNKNOWN": "Unknown collection type.",
  "error.reason.CATALOG_CONFIGURED_QUALITY_PROFILE_MISSING_CATALOG":
    "Configured quality profile the selected item from the selected value is missing from the catalog.",
  "error.reason.CATALOG_DELETE_PREVIEW_CONFIRMATION_REQUIRED_BEFORE_DELETING_FILES_DISK":
    "Delete preview confirmation is required before deleting files on disk.",
  "error.reason.CATALOG_DOWNLOAD_ALREADY_QUEUED_TITLE":
    "A download is already queued for this title.",
  "error.reason.CATALOG_DOWNLOAD_CLIENT_DISABLED":
    "Download client the selected value is disabled.",
  "error.reason.CATALOG_DOWNLOAD_CLIENT_DOES_NOT_EXIST":
    "Download client the selected value does not exist.",
  "error.reason.CATALOG_DOWNLOAD_CLIENT_UNAVAILABLE":
    "Selected download client is not eligible.",
  "error.reason.CATALOG_EPISODE_FIELD_REQUIRED":
    "At least one episode field must be provided.",
  "error.reason.CATALOG_EPISODE_MISSING_EPISODE_NUMBER":
    "Episode is missing episode number.",
  "error.reason.CATALOG_EPISODE_MISSING_SEASON_NUMBER":
    "Episode is missing season number.",
  "error.reason.CATALOG_EPISODE_NOT_FOUND": "Episode was not found.",
  "error.reason.CATALOG_EPISODE_SEARCHES_REQUIRE_SEASON":
    "Episode searches require a season.",
  "error.reason.CATALOG_EPISODE_TYPE_REQUIRED": "Episode type is required.",
  "error.reason.CATALOG_EPISODE_TYPE_UNKNOWN": "Unknown episode type.",
  "error.reason.CATALOG_EXISTING_DOWNLOAD_NO_LONGER_SAFE_REPLACE":
    "The existing download is no longer safe to replace.",
  "error.reason.CATALOG_ID_ALREADY_ASSIGNED_TITLE":
    "The requested value id the selected value is already assigned to title the selected value.",
  "error.reason.CATALOG_INDEXER_UNKNOWN": "Unknown indexer the selected value.",
  "error.reason.CATALOG_INTERACTIVE_RELEASE_SEARCH_NOT_FOUND":
    "Interactive release search was not found.",
  "error.reason.CATALOG_LIBRARY_NOT_FOUND": "Library was not found.",
  "error.reason.CATALOG_LISTED_BOTH_ADD_REMOVE":
    "The selected item is listed both to add and to remove; choose one.",
  "error.reason.CATALOG_MAGNET_LINK_NO_FILE_DOWNLOAD":
    "The requested value is a magnet link and has no file to download.",
  "error.reason.CATALOG_MEDIA_FILE_DELETION_ALREADY_RUNNING":
    "A media file deletion is already running for the selected value.",
  "error.reason.CATALOG_MEDIA_FILE_NOT_FOUND": "Media file was not found.",
  "error.reason.CATALOG_METADATA_LANGUAGE_INVALID":
    "Metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn.",
  "error.reason.CATALOG_MINIMUM_AVAILABILITY_INVALID":
    "Minimum availability must be announced, in_cinemas, or released.",
  "error.reason.CATALOG_MINIMUM_AVAILABILITY_ONLY_VALID_MOVIE_TITLES":
    "Minimum availability is only valid for movie titles.",
  "error.reason.CATALOG_MOST_RELEASES_DOWNLOADED_ONCE":
    "At most the selected value releases can be downloaded at once.",
  "error.reason.CATALOG_MOVIE_ENTITY_NOT_FOUND": "Movie entity was not found.",
  "error.reason.CATALOG_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND":
    "Movie metadata response missing title was not found.",
  "error.reason.CATALOG_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE":
    "Movie searches cannot include season or episode.",
  "error.reason.CATALOG_MOVIE_TITLE_DOES_NOT_CANONICAL_FOLDER_PATH":
    "Movie title does not have a canonical folder path.",
  "error.reason.CATALOG_NOT_DEFINED_TAG":
    "The selected item is not a defined tag; an administrator has to add it in Settings before it can be applied.",
  "error.reason.CATALOG_NO_QUALITY_PROFILE_NOT_CONFIGURED":
    "No quality profile is configured; choose a global, category, library, or title profile.",
  "error.reason.CATALOG_NO_USABLE_DOWNLOAD_SOURCE":
    "The requested value has no usable download source.",
  "error.reason.CATALOG_PENDING_RENAME_REFERS_MISSING_TITLE_TAG":
    "Pending rename refers to missing title tag the selected value.",
  "error.reason.CATALOG_PRIMARY_MOVIE_FILE_ONLY_SET_MOVIE_TITLES_SERIES_MOVIE_FILES_EPISODE_FILES":
    "Primary movie file can only be set for movie titles, series movie files, or episode files.",
  "error.reason.CATALOG_PROVIDE_EXACTLY_ONE_TITLE_ID_QUERY":
    "Provide exactly one of title id or query.",
  "error.reason.CATALOG_RELEASE_DOWNLOAD_REQUIRED":
    "Select at least one release to download.",
  "error.reason.CATALOG_RELEASE_NO_LONGER_SEARCH_NOT_FOUND":
    "Release is no longer in this search was not found.",
  "error.reason.CATALOG_RELEASE_NO_PROTOCOL": "Release has no protocol.",
  "error.reason.CATALOG_RELEASE_NO_USABLE_DOWNLOAD_SOURCE":
    "Release has no usable download source.",
  "error.reason.CATALOG_RELEASE_SIZE_DOES_NOT_MATCH_SIGNED_CANDIDATE":
    "Release size does not match the signed candidate.",
  "error.reason.CATALOG_ROOTFOLDERID_MUST_REFERENCE_ROOT_TITLE_LIBRARY":
    "RootFolderId must reference a root on the title library.",
  "error.reason.CATALOG_ROOT_REQUIRED":
    "Title library must have at least one root folder.",
  "error.reason.CATALOG_SEARCH_QUERY_REQUIRED": "Search query is required.",
  "error.reason.CATALOG_SELECTED_FILE_OUTSIDE_TITLE_S_CANONICAL_MOVIE_FOLDER":
    "Selected file is outside the title's canonical movie folder.",
  "error.reason.CATALOG_SELECTED_RELEASE_FILES_EXCEED_64_MIB_DOWNLOAD_BUNDLE_LIMIT":
    "Selected release files exceed the 64 MiB download bundle limit.",
  "error.reason.CATALOG_SERIES_MOVIE_DOES_NOT_BELONG_TITLE":
    "Series movie does not belong to title.",
  "error.reason.CATALOG_SERIES_MOVIE_LINK_NOT_FOUND":
    "Series movie link was not found.",
  "error.reason.CATALOG_SERIES_MOVIE_NOT_FOUND": "Series movie was not found.",
  "error.reason.CATALOG_SERIES_MOVIE_REQUIRED":
    "At least one series movie must be provided.",
  "error.reason.CATALOG_SERIES_MOVIE_SEARCHES_CANNOT_INCLUDE_SEASON_EPISODE":
    "Series movie searches cannot include season or episode.",
  "error.reason.CATALOG_SMG_ID_OUT_OF_RANGE": "Smg id must be positive.",
  "error.reason.CATALOG_SMG_NO_SERIES_NOT_FOUND":
    "The requested value: SMG has no series for title id the selected value.",
  "error.reason.CATALOG_SNAPSHOT_JSON_COLUMN_INVALID": "Invalid snapshot JSON.",
  "error.reason.CATALOG_SNAPSHOT_TITLE_MISSING_AMBIGUOUS":
    "Snapshot title is missing or ambiguous.",
  "error.reason.CATALOG_SNAPSHOT_TITLE_NO_LONGER_EXISTS":
    "Snapshot title no longer exists.",
  "error.reason.CATALOG_TAG_ADD_REMOVE_REQUIRED":
    "At least one tag to add or remove must be provided.",
  "error.reason.CATALOG_TITLES_NO_SERIES_METADATA":
    "The requested value titles have no series metadata.",
  "error.reason.CATALOG_TITLE_CARRY_MOST_TAGS":
    "A title can carry at most the selected value tags; this change would leave it with the selected value.",
  "error.reason.CATALOG_TITLE_FACET_UNSUPPORTED": "Unsupported title facet.",
  "error.reason.CATALOG_TITLE_FIELD_REQUIRED":
    "At least one title field must be provided.",
  "error.reason.CATALOG_TITLE_IDENTITY_REQUIRED":
    "A title identity is required.",
  "error.reason.CATALOG_TITLE_NAME_REQUIRED": "Title name is required.",
  "error.reason.CATALOG_TITLE_REQUIRED": "At least one title must be provided.",
  "error.reason.CATALOG_TITLE_TAG_CHANGED_WHILE_RENAME_PENDING":
    "Title tag the selected value changed while its rename was pending.",
  "error.reason.CATALOG_TOO_MANY_CONCURRENT_INTERACTIVE_SEARCHES":
    "Too many concurrent interactive searches.",
  "error.reason.CATALOG_TVDB_ID_INVALID": "Tvdb id must be numeric.",
  "error.reason.CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED":
    "A candidate transition must name the states it expects.",
  "error.reason.CONFIGURATION_MAINTENANCE_ATTEMPT_RESULT_REQUIRED":
    "A maintenance action step finish must carry an attempt result.",
  "error.reason.CONFIGURATION_MAINTENANCE_CANDIDATE_ALREADY_ACTIVE":
    "Maintenance rule already has an active candidate for title.",
  "error.reason.CONFIGURATION_MAINTENANCE_CANDIDATE_PAGE_TOO_LARGE":
    "Maintenance sequence progress accepts at most 100 candidates per page.",
  "error.reason.CONFIGURATION_MAINTENANCE_COMPLETION_LIMIT_INVALID":
    "Invalid maintenance completion limit.",
  "error.reason.CONFIGURATION_MAINTENANCE_DISPATCH_STATE_INVALID":
    "A generic maintenance job dispatch must begin in dispatching state.",
  "error.reason.CONFIGURATION_MAINTENANCE_FAILED_STEP_ID_REQUIRED":
    "Failed maintenance sequence marker lacks its failed step id.",
  "error.reason.CONFIGURATION_MAINTENANCE_HELD_RELEASE_INVALID":
    "Invalid maintenance held-attempt release.",
  "error.reason.CONFIGURATION_MAINTENANCE_JOB_RUN_ID_REQUIRED":
    "Accepted maintenance job receipts require a real job run id.",
  "error.reason.CONFIGURATION_MAINTENANCE_MEMBERSHIP_TERMINAL":
    "Maintenance sequence membership is terminal until a confirmed no-match.",
  "error.reason.CONFIGURATION_MAINTENANCE_RECEIPT_EXPECTED_STATES_REQUIRED":
    "A maintenance job receipt transition must name expected states.",
  "error.reason.CONFIGURATION_MAINTENANCE_RECEIPT_PAGE_TOO_LARGE":
    "Maintenance sequence receipt projection accepts at most 700 steps.",
  "error.reason.CONFIGURATION_MAINTENANCE_RELEASE_REASON_REQUIRED":
    "Maintenance held-attempt release lacks a reason.",
  "error.reason.CONFIGURATION_MAINTENANCE_RETRY_ATTEMPT_NOT_ADVANCED":
    "A retried maintenance action step must advance its attempt.",
  "error.reason.CONFIGURATION_MAINTENANCE_RULE_HAS_HISTORY":
    "This maintenance rule has live candidates or action history; disable it instead.",
  "error.reason.CONFIGURATION_MAINTENANCE_SEQUENCE_IDENTITY_MISMATCH":
    "Maintenance action step does not match its persisted sequence identity.",
  "error.reason.CONFIGURATION_MAINTENANCE_STEP_ATTEMPT_INVALID":
    "A maintenance action step claim must start a non-negative running attempt.",
  "error.reason.CONFIGURATION_MAINTENANCE_STEP_PREFIX_MISMATCH":
    "Failed maintenance sequence marker does not preserve its step prefix.",
  "error.reason.CONFIGURATION_MAINTENANCE_STEP_TARGET_INVALID":
    "A maintenance action step claim has an invalid target identity.",
  "error.reason.CONFIGURATION_MAINTENANCE_SUCCESS_EVIDENCE_INCOMPLETE":
    "Successful maintenance sequence marker lacks complete step evidence.",
  "error.reason.CONFIGURATION_MAINTENANCE_TERMINAL_EVIDENCE_MISMATCH":
    "Maintenance terminal marker does not match persisted step evidence.",
  "error.reason.CONFIGURATION_MAINTENANCE_TERMINAL_STATE_MISMATCH":
    "Maintenance sequence terminal marker and candidate terminal state disagree.",
  "error.reason.CONFIGURATION_MAINTENANCE_TERMINAL_STEP_IDS_INVALID":
    "Maintenance sequence terminal marker has invalid expected step ids.",
  "error.reason.CONFIGURATION_PLUGIN_NOT_INSTALLED":
    "Plugin is no longer installed.",
  "error.reason.CONFIGURATION_PLUGIN_SETTINGS_PLUGIN_REQUIRED":
    "Plugin settings require an installed plugin.",
  "error.reason.CONFIGURATION_QUALITY_PROFILE_REPLACEMENT_SCOPE_REQUIRED":
    "Scope is required to replace quality profiles.",
  "error.reason.CONFIGURATION_QUALITY_PROFILE_SCOPE_REQUIRED":
    "Scope is required to list quality profiles.",
  "error.reason.CONFIGURATION_RULE_COPY_SOURCE_NOT_USER_RULE":
    "Copied rule sets must be ordinary user rules.",
  "error.reason.CONFIGURATION_RULE_PACK_OWNERSHIP_MISMATCH":
    "Rule set is not owned by the requested pack.",
  "error.reason.CONFIGURATION_SETTING_DELETE_IDENTITY_REQUIRED":
    "Scope and key_name are required to delete a setting override.",
  "error.reason.CONFIGURATION_SETTING_KEY_AMBIGUOUS":
    "Ambiguous setting key . found in categories.",
  "error.reason.CONFIGURATION_SETTING_READ_IDENTITY_REQUIRED":
    "Scope and key_name are required to read a setting.",
  "error.reason.CONFIGURATION_SETTING_UPDATE_IDENTITY_REQUIRED":
    "Scope and key_name are required to update a setting.",
  "error.reason.CONFIGURATION_SETTING_VALUE_EMPTY":
    "Setting value cannot be empty.",
  "error.reason.CONFIGURATION_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND":
    "The subtitle provider config could not be found.",
  "error.reason.CONFIGURATION_SUBTITLE_PROVIDER_UPDATE_EMPTY":
    "At least one subtitle provider config field must be provided.",
  "error.reason.CONFIGURATION_UNKNOWN_SETTING_KEY": "Unknown setting key.",
  "error.reason.DATASTORE_BACKUP_BLOB_BASE64_INVALID":
    "Backup blob payload is invalid base64.",
  "error.reason.DATASTORE_BACKUP_BLOB_BYTES_MISSING":
    "Backup blob payload is missing base64 bytes.",
  "error.reason.DATASTORE_BACKUP_BYTEA_UNSUPPORTED":
    "Backup row contains an unsupported PostgreSQL bytea value.",
  "error.reason.DATASTORE_BACKUP_FOREIGN_KEY_INVALID":
    "Restored database failed foreign key validation.",
  "error.reason.DATASTORE_BACKUP_INTEGER_OUT_OF_RANGE":
    "Backup row contains an integer outside SQLite i64 range.",
  "error.reason.DATASTORE_BACKUP_NUMBER_UNSUPPORTED":
    "Backup row contains an unsupported numeric value.",
  "error.reason.DATASTORE_BACKUP_REQUIRED_COLUMN_MISSING":
    "The backup row is missing a required column.",
  "error.reason.DATASTORE_BACKUP_REQUIRED_COLUMN_NULL":
    "The backup row contains null in a required column.",
  "error.reason.DATASTORE_BACKUP_ROW_IMPORT_FAILED":
    "Failed to import backup row for.",
  "error.reason.DATASTORE_BACKUP_ROW_INVALID": "Invalid backup row for.",
  "error.reason.DATASTORE_BACKUP_ROW_NOT_OBJECT":
    "The backup row must be an object.",
  "error.reason.DATASTORE_BACKUP_ROW_UNREADABLE": "Failed to read backup row.",
  "error.reason.DATASTORE_BACKUP_TABLE_MISSING":
    "Backup table payload missing for.",
  "error.reason.DATASTORE_BACKUP_TABLE_SET_MISMATCH":
    "The backup table set does not match the current restore catalog.",
  "error.reason.DATASTORE_BACKUP_TIMESTAMP_BLANK":
    "The backup row contains a blank required timestamp.",
  "error.reason.DATASTORE_DATABASE_MIGRATION_CHECK_FAILED":
    "Database migration check failed; pending migrations.",
  "error.reason.DATASTORE_DIRECTORY_RESTORE_ENGINE_UNSUPPORTED":
    "Prepared backup directory restore is only supported for PostgreSQL.",
  "error.reason.DATASTORE_PASSWORD_FILE_EMPTY":
    "SCRYER_DB_PASSWORD_FILE did not contain a password.",
  "error.reason.DATASTORE_PASSWORD_FILE_UNREADABLE":
    "Failed to read SCRYER_DB_PASSWORD_FILE.",
  "error.reason.DATASTORE_POSTGRES_DATABASE_REQUIRED":
    "PostgreSQL datastore URL must include a database name.",
  "error.reason.DATASTORE_POSTGRES_HOST_REQUIRED":
    "PostgreSQL datastore URL must include a host.",
  "error.reason.DATASTORE_POSTGRES_PASSWORD_INVALID":
    "Failed to set PostgreSQL password on datastore URL.",
  "error.reason.DATASTORE_POSTGRES_PASSWORD_REQUIRED":
    "PostgreSQL datastore requires SCRYER_DB_PASSWORD, SCRYER_DB_PASSWORD_FILE, or a URL password.",
  "error.reason.DATASTORE_POSTGRES_SSLMODE_REQUIRED":
    "PostgreSQL datastore URL must include an explicit sslmode.",
  "error.reason.DATASTORE_POSTGRES_SSLMODE_UNSUPPORTED":
    "The PostgreSQL SSL mode is not supported.",
  "error.reason.DATASTORE_POSTGRES_USERNAME_INVALID":
    "Failed to set PostgreSQL username on datastore URL.",
  "error.reason.DATASTORE_POSTGRES_USERNAME_REQUIRED":
    "PostgreSQL datastore requires SCRYER_DB_USER or a URL username.",
  "error.reason.DATASTORE_RESTORED_EVENT_INVALID":
    "Failed to encode restored domain event payload.",
  "error.reason.DATASTORE_RESTORED_EXPLANATION_COMPACTION_FAILED":
    "Failed to compact restored release explanation.",
  "error.reason.DATASTORE_RESTORED_EXPLANATION_INVALID":
    "Failed to encode restored release explanation.",
  "error.reason.DATASTORE_RESTORED_EXPLANATION_NULL":
    "Present restored release explanation encoded as null.",
  "error.reason.DATASTORE_RESTORED_JSON_INVALID":
    "The restored value contains invalid JSON.",
  "error.reason.DATASTORE_RESTORED_REQUIRED_VALUE_NULL":
    "A required restored value is null.",
  "error.reason.DATASTORE_RESTORED_TABLE_ROW_COUNT_MISMATCH":
    "The restored table row count does not match the backup.",
  "error.reason.DATASTORE_URL_INVALID": "Invalid SCRYER_DB_URL.",
  "error.reason.DATASTORE_URL_SCHEME_UNSUPPORTED":
    "The datastore URL must use SQLite or PostgreSQL.",
  "error.reason.DISCOVERY_CONTEXT_CHANGE_TYPE_UNSUPPORTED":
    "Unsupported discovery context change type the selected value.",
  "error.reason.DISCOVERY_HOME_HERO_NOT_FOUND":
    "Discovery home hero was not found.",
  "error.reason.DISCOVERY_INCREMENTAL_RELOAD_RESOLVES_CHANGED_SUBJECTS_ABOVE_SMG_LIMIT":
    "Discovery incremental reload resolves to the selected value changed subjects, above SMG limit the selected value.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_INVALID_PREVIOUS_SUBJECT_JSON":
    "Pending discovery change the selected value has invalid previous subject JSON.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_INVALID_RAW_SUBJECT_JSON":
    "Pending discovery change the selected value has invalid raw subject JSON.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_MISSING_RAW_SUBJECT_JSON":
    "Pending discovery change the selected value is missing raw subject JSON.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_CHANGE_MISSING_SUBJECT_KEY":
    "Pending discovery change the selected value is missing subject key.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_REMATCH_MISSING_PREVIOUS_SUBJECT_KEY":
    "Pending discovery rematch the selected value is missing previous subject key.",
  "error.reason.DISCOVERY_PENDING_DISCOVERY_REMOVAL_MISSING_SUBJECT_KEY":
    "Pending discovery removal the selected value is missing subject key.",
  "error.reason.DOWNLOAD_CLIENT_CONFIG_JSON_INVALID":
    "Invalid download client config JSON.",
  "error.reason.DOWNLOAD_PATH_MAPPING_DOWNLOAD_CLIENT_CONFIG_JSON_INVALID":
    "Invalid download client config JSON.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_INVALID":
    "Remote_path_mappings must be a string.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_DUPLICATES_EXISTING_REMOTE_PATH_ROOT":
    "Remote path mappings line the selected value duplicates an existing remote path root.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_INVALID":
    "Remote path mappings line the selected value must use REMOTE => LOCAL format.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_LOCAL_PATH_INVALID":
    "Remote path mappings line the selected value local path must be absolute.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_END_WHITESPACE":
    "Remote path mappings line the selected value remote path cannot end with whitespace.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REMOTE_PATH_CANNOT_START_WHITESPACE":
    "Remote path mappings line the selected value remote path cannot start with whitespace.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REQUIRES_LOCAL_PATH":
    "Remote path mappings line the selected value requires a local path.",
  "error.reason.DOWNLOAD_PATH_MAPPING_REMOTE_PATH_MAPPINGS_LINE_REQUIRES_REMOTE_PATH":
    "Remote path mappings line the selected value requires a remote path.",
  "error.reason.EVENTS_ACTIVE_PLUGIN_INSTALL_PROGRESS_NOT_FOUND":
    "No active plugin install progress for was not found.",
  "error.reason.EVENTS_EPISODE_NOT_FOUND": "Episode was not found.",
  "error.reason.EVENTS_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.EXTERNAL_ACCOUNT_IDENTITY_MISMATCH":
    "External account is already linked to a different provider identity.",
  "error.reason.EXTERNAL_ACCOUNT_LINKED_ELSEWHERE":
    "This account is already linked to another user.",
  "error.reason.EXTERNAL_IMPORT_API_VERSION_EXPECTED_ONE_UNSUPPORTED":
    "Unsupported the selected value API version the selected item; expected one of the selected value.",
  "error.reason.EXTERNAL_IMPORT_COULD_NOT_DETERMINE_VERSION":
    "Could not determine the selected value version from the selected item.",
  "error.reason.EXTERNAL_IMPORT_DID_NOT_REPORT_API_VERSION":
    "The requested value did not report an API version.",
  "error.reason.EXTERNAL_IMPORT_INVALID_HOST":
    "The requested value has an invalid host.",
  "error.reason.EXTERNAL_IMPORT_SESSION_GONE":
    "The import session no longer exists. Connect again.",
  "error.reason.EXTERNAL_IMPORT_SOURCE_APPLICATION_MISMATCH":
    "The source URL belongs to a different application.",
  "error.reason.EXTERNAL_IMPORT_SOURCE_URL_INVALID":
    "The source URL is invalid.",
  "error.reason.EXTERNAL_IMPORT_SOURCE_URL_QUERY_OR_FRAGMENT":
    "The requested value must not include a query string or fragment.",
  "error.reason.EXTERNAL_IMPORT_VERSION_EXPECTED_MAJOR_NEWER_UNSUPPORTED":
    "Unsupported the selected value version the selected item; expected major the selected value or newer.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_CLAIM_LIBRARY_ROOT_TITLE_FOLDER":
    "Cannot claim a library root as a title folder.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE":
    "Cannot compare folder the selected value with folder the selected value owned by title the selected value.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_INSPECT_FOLDER_OWNED_ANOTHER_TITLE":
    "Cannot inspect folder the selected value owned by another title.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_INSPECT_TITLE_FOLDER":
    "Cannot inspect title folder the selected value.",
  "error.reason.FOLDER_OWNERSHIP_CANNOT_RESOLVE_TITLE_FOLDER":
    "Cannot resolve title folder the selected value.",
  "error.reason.FOLDER_OWNERSHIP_FOLDER_ALREADY_OWNED_TITLE":
    "Folder the selected value is already owned by title the selected value.",
  "error.reason.FOLDER_OWNERSHIP_FOLDER_NOT_TITLE_BOUNDARY":
    "The recorded folder for the selected item is a library root, not a title folder; correct the title's folder first.",
  "error.reason.FOLDER_OWNERSHIP_FOLDER_OVERLAPS_FOLDER_OWNED_TITLE":
    "Folder the selected value overlaps the folder owned by title the selected value.",
  "error.reason.FOLDER_OWNERSHIP_PATH_OUTSIDE_LIBRARY_ROOT":
    "Cannot claim a title folder outside its library roots.",
  "error.reason.FOLDER_OWNERSHIP_RECYCLE_RESTORE_DESTINATION_OVERLAPS_ANOTHER_TITLE_S_OWNED_FOLDER":
    "Recycle restore destination overlaps another title's owned folder; recycled files were preserved.",
  "error.reason.FOLDER_OWNERSHIP_RECYCLE_RESTORE_PATH_OUTSIDE_TITLE_S_CURRENT_OWNED_FOLDER":
    "Recycle restore path is outside the title's current owned folder; resolve the folder change before restoring.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_ALREADY_OWNS_ANOTHER_FOLDER":
    "Title the selected value already owns another folder.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_FOLDER_INVALID":
    "Title folder must be an absolute path without parent traversal.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_FOLDER_NO_EXISTING_ANCESTOR":
    "Title folder has no existing ancestor.",
  "error.reason.FOLDER_OWNERSHIP_TITLE_FOLDER_PATH_REQUIRED":
    "Title folder path is required.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_EMPTY":
    "Configured media root the selected value is empty.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_NOT_DIRECTORY":
    "Configured media root the selected value is not a directory.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_SYMLINK":
    "Configured media root the selected value is a symlink.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_UNAVAILABLE":
    "Configured media root the selected value is unavailable.",
  "error.reason.FS_SAFETY_CONFIGURED_MEDIA_ROOT_UNREADABLE":
    "Configured media root the selected value is unreadable.",
  "error.reason.HTTP_API_EXPLORER_DISABLED": "API explorer is disabled.",
  "error.reason.HTTP_API_EXPLORER_MODE_DUPLICATE":
    "Duplicate API explorer access mode.",
  "error.reason.HTTP_API_EXPLORER_MODE_INVALID":
    "Invalid API explorer access mode.",
  "error.reason.HTTP_API_EXPLORER_SESSION_CHANGED":
    "API explorer session changed.",
  "error.reason.HTTP_API_EXPLORER_WEB_SESSION_REQUIRED":
    "API explorer requires a web session.",
  "error.reason.HTTP_AUTHORIZATION_HEADER_INVALID":
    "Invalid authorization header.",
  "error.reason.HTTP_BACKUP_BUNDLE_NOT_FOUND":
    "The backup bundle could not be found.",
  "error.reason.HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_INVALID":
    "Backup download authorization is invalid.",
  "error.reason.HTTP_BACKUP_DOWNLOAD_AUTHORIZATION_REQUIRED":
    "Backup download authorization is required.",
  "error.reason.HTTP_BACKUP_FILENAME_INVALID": "Invalid backup filename.",
  "error.reason.HTTP_BACKUP_METADATA_NOT_FOUND":
    "The backup metadata could not be found.",
  "error.reason.HTTP_BACKUP_TICKET_QUERY_FORBIDDEN":
    "Backup download ticket query is not accepted.",
  "error.reason.HTTP_RELEASE_DOWNLOAD_REQUEST_INVALID":
    "Invalid release download request.",
  "error.reason.IDENTITY_ADMINISTRATOR_BOOTSTRAP_SEED_STALE":
    "Default administrator seed changed during bootstrap.",
  "error.reason.IDENTITY_API_KEY_NOT_ENVIRONMENT_MANAGED":
    "An API key with this lookup ID is not environment-managed.",
  "error.reason.IDENTITY_AUTHENTICATION_SESSION_INVALIDATED":
    "Authentication session was invalidated.",
  "error.reason.IDENTITY_EMBY_ACCOUNT_DISABLED": "Emby account is disabled.",
  "error.reason.IDENTITY_EMBY_API_KEY_LIST_ACCESS_DENIED":
    "Emby administrator cannot list API keys.",
  "error.reason.IDENTITY_EMBY_AVATAR_ID_INVALID":
    "Invalid Emby avatar identifier.",
  "error.reason.IDENTITY_EMBY_BASE_URL_INVALID": "Emby base URL is invalid.",
  "error.reason.IDENTITY_EMBY_BASE_URL_PATH_FORBIDDEN":
    "Emby base URL cannot contain path segments.",
  "error.reason.IDENTITY_EMBY_BASE_URL_UNSAFE":
    "Emby base URL must be an absolute HTTP(S) URL without credentials, query, or fragment.",
  "error.reason.IDENTITY_EMBY_CONNECT_CREDENTIALS_INVALID":
    "Invalid Emby Connect credentials.",
  "error.reason.IDENTITY_EMBY_CONNECT_SERVER_ACCESS_DENIED":
    "Emby Connect account cannot access the configured server.",
  "error.reason.IDENTITY_EMBY_CONNECT_SIGN_IN_FAILED":
    "Emby Connect sign-in failed.",
  "error.reason.IDENTITY_EMBY_CREDENTIALS_INVALID": "Invalid Emby credentials.",
  "error.reason.IDENTITY_EMBY_CREDENTIAL_INVALID":
    "Emby credential is invalid.",
  "error.reason.IDENTITY_EMBY_ENDPOINT_URL_INVALID":
    "Emby endpoint URL is invalid.",
  "error.reason.IDENTITY_EMBY_LOCAL_ADMINISTRATOR_REQUIRED":
    "Emby account must be a local administrator.",
  "error.reason.IDENTITY_EMBY_SERVER_IDENTITY_MISMATCH":
    "Emby server identity does not match the saved connection.",
  "error.reason.IDENTITY_EMBY_SERVER_NOT_RECOGNIZED":
    "The supplied URL is not an Emby server.",
  "error.reason.IDENTITY_EMBY_USER_LIST_ACCESS_DENIED":
    "Emby API key is invalid or cannot list users.",
  "error.reason.IDENTITY_EMBY_USER_VERIFICATION_FAILED":
    "Emby user verification failed.",
  "error.reason.IDENTITY_EXTERNAL_ACCOUNT_IDENTITY_REQUIRED":
    "External account identity must be present when claiming.",
  "error.reason.IDENTITY_EXTERNAL_ACCOUNT_NOT_FOUND":
    "The external account could not be found.",
  "error.reason.IDENTITY_JELLYFIN_ADMINISTRATOR_REQUIRED":
    "Jellyfin account must be an administrator to create a Scryer API key.",
  "error.reason.IDENTITY_JELLYFIN_ADMIN_CREDENTIALS_INVALID":
    "Invalid Jellyfin admin credentials.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_LIST_ACCESS_DENIED":
    "Jellyfin admin token cannot list API keys.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_UNAVAILABLE":
    "Jellyfin API key is unavailable.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_URL_INVALID":
    "Jellyfin API key URL is invalid.",
  "error.reason.IDENTITY_JELLYFIN_API_KEY_USER_LIST_ACCESS_DENIED":
    "Jellyfin API key is invalid or does not have user-list access.",
  "error.reason.IDENTITY_JELLYFIN_AUTHENTICATION_URL_INVALID":
    "Jellyfin authentication URL is invalid.",
  "error.reason.IDENTITY_JELLYFIN_BASE_URL_INVALID":
    "Jellyfin base URL is invalid.",
  "error.reason.IDENTITY_JELLYFIN_BASE_URL_QUERY_FRAGMENT_FORBIDDEN":
    "Jellyfin connection base URL must not include a query or fragment.",
  "error.reason.IDENTITY_JELLYFIN_CREDENTIALS_INVALID":
    "Invalid Jellyfin credentials.",
  "error.reason.IDENTITY_JELLYFIN_SERVER_NOT_RECOGNIZED":
    "The supplied URL did not identify itself as a Jellyfin server.",
  "error.reason.IDENTITY_JELLYFIN_SYSTEM_INFO_URL_INVALID":
    "Jellyfin system info URL is invalid.",
  "error.reason.IDENTITY_JELLYFIN_USERS_URL_INVALID":
    "Jellyfin users URL is invalid.",
  "error.reason.IDENTITY_JELLYFIN_USER_ID_INVALID":
    "Jellyfin user ID is invalid.",
  "error.reason.IDENTITY_JELLYFIN_USER_LIST_ACCESS_DENIED":
    "Jellyfin API key is invalid or cannot list users.",
  "error.reason.IDENTITY_JELLYFIN_USER_URL_INVALID":
    "Jellyfin user URL is invalid.",
  "error.reason.IDENTITY_JELLYFIN_USER_VERIFICATION_FAILED":
    "Jellyfin user verification failed.",
  "error.reason.IDENTITY_LAST_SIGN_IN_METHOD_REQUIRED":
    "Cannot remove the last available sign-in method.",
  "error.reason.IDENTITY_LOGIN_VERIFICATION_CHALLENGE_NOT_FOUND":
    "The login verification challenge could not be found.",
  "error.reason.IDENTITY_MEDIA_SERVER_BASE_URL_INVALID":
    "Media server base URL is invalid.",
  "error.reason.IDENTITY_MEDIA_SERVER_REFRESH_API_KEY_REQUIRED":
    "Media server refresh requires an API key.",
  "error.reason.IDENTITY_MEDIA_SERVER_SCAN_API_KEY_REQUIRED":
    "Media server catalog scan requires an API key.",
  "error.reason.IDENTITY_MEDIA_SERVER_SIGNAL_USER_ID_REQUIRED":
    "A media-server signal read needs a provider user id.",
  "error.reason.IDENTITY_OAUTH_CLIENT_DISABLED_UNAVAILABLE":
    "OAuth client is disabled or unavailable.",
  "error.reason.IDENTITY_OAUTH_GRANT_NOT_FOUND":
    "The OAuth grant could not be found.",
  "error.reason.IDENTITY_PASSKEY_DELETION_SESSION_STALE":
    "Passkey deletion session is no longer current.",
  "error.reason.IDENTITY_PASSKEY_ENROLLMENT_SESSION_STALE":
    "Passkey enrollment session is no longer current.",
  "error.reason.IDENTITY_PASSWORD_CHANGE_NOT_REQUIRED":
    "Password change is no longer required.",
  "error.reason.IDENTITY_PLEX_AUTH_TOKEN_INVALID": "Invalid Plex auth token.",
  "error.reason.IDENTITY_PLEX_AUTH_TOKEN_REQUIRED":
    "Plex auth token is required.",
  "error.reason.IDENTITY_PLEX_REFRESH_SERVER_REQUIRED":
    "Plex refresh requires a selected server.",
  "error.reason.IDENTITY_PLEX_SCAN_SERVER_REQUIRED":
    "Plex catalog scan requires a selected server.",
  "error.reason.IDENTITY_PLEX_SERVER_ACCESS_DENIED":
    "Plex account does not have access to the configured server.",
  "error.reason.IDENTITY_PLEX_USER_LIST_ACCESS_DENIED":
    "Plex token is invalid or cannot list users.",
  "error.reason.IDENTITY_RECOVERY_CODE_SESSION_STALE":
    "Recovery-code replacement session is no longer current.",
  "error.reason.IDENTITY_TOTP_CHALLENGE_NOT_FOUND":
    "The TOTP challenge could not be found.",
  "error.reason.IDENTITY_TOTP_CREDENTIAL_NOT_FOUND":
    "The TOTP credential could not be found.",
  "error.reason.IDENTITY_TOTP_DISABLEMENT_SESSION_STALE":
    "TOTP disablement session is no longer current.",
  "error.reason.IDENTITY_TOTP_ENROLLMENT_SESSION_STALE":
    "TOTP enrollment session is no longer current.",
  "error.reason.IDENTITY_USER_NOT_FOUND": "The user could not be found.",
  "error.reason.IDENTITY_USER_UI_SETTINGS_NOT_FOUND":
    "The user UI settings could not be found.",
  "error.reason.IDENTITY_WEBAUTHN_CHALLENGE_NOT_FOUND":
    "The webauthn challenge could not be found.",
  "error.reason.IDENTITY_WEBAUTHN_CREDENTIAL_NOT_FOUND":
    "The webauthn credential could not be found.",
  "error.reason.IMPORT_ACTIVE_IMPORT_STREAM_NOT_FOUND":
    "Active import stream was not found.",
  "error.reason.IMPORT_ARCHIVE_DISCOVERY_DIRECTORY_DEPTH_LIMIT_EXCEEDED":
    "Archive discovery directory or depth limit exceeded.",
  "error.reason.IMPORT_ARCHIVE_DISCOVERY_ENTRY_LIMIT_EXCEEDED":
    "Archive discovery entry limit exceeded.",
  "error.reason.IMPORT_ARCHIVE_ENTRY_ESCAPES_EXTRACTION_DIRECTORY":
    "Archive entry escapes extraction directory.",
  "error.reason.IMPORT_ARCHIVE_EXTRACTION_REQUIRES_RESOLVED_IMPORT_DESTINATION_BEFORE_STAGING_OUTPUT":
    "Archive extraction requires a resolved import destination before staging output for the selected value.",
  "error.reason.IMPORT_ARCHIVE_HOLDS_ARCHIVES_NESTED_MORE_LEVELS_DEEP":
    "Archive holds archives nested more than the selected value levels deep.",
  "error.reason.IMPORT_ARCHIVE_HOLDS_MORE_NESTED_ARCHIVES":
    "Archive holds more than the selected value nested archives.",
  "error.reason.IMPORT_ARCHIVE_MATCHED_TITLE_BUT_EXTRACTED_WORKSPACE_COULD_NOT_MOVED_WITHOUT_COPYING":
    "Archive matched title the selected item but extracted workspace the selected value could not be moved to the selected value without copying.",
  "error.reason.IMPORT_ARCHIVE_OUTPUT_DIRECTORY_NOT_INSIDE_STAGING_WORKSPACE":
    "Archive output directory the selected value is not inside the staging workspace.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_EXTRACTION":
    "Archive plugin does not support the selected value extraction.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_MANIFEST_OUTPUT_NOT_REGULAR_FILE":
    "Archive plugin manifest output is not a regular file.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_MANIFEST_SIZE_MISMATCH":
    "Archive plugin manifest size mismatch for the selected value.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_SYMLINK":
    "Archive plugin output contains a symlink.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_DIRECTORIES":
    "Archive plugin output contains too many directories.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_ENTRIES":
    "Archive plugin output contains too many entries.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_CONTAINS_TOO_MANY_FILES":
    "Archive plugin output contains too many files.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_EXCEEDS_BYTES":
    "Archive plugin output exceeds the selected value bytes.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_NOT_REGULAR_FILE":
    "Archive plugin output is not a regular file.",
  "error.reason.IMPORT_ARCHIVE_PLUGIN_OUTPUT_TOO_LARGE":
    "Archive plugin output is too large.",
  "error.reason.IMPORT_ARCHIVE_REEXTRACTION_FAILED":
    "Archive matched title the selected item but could not be re-extracted under the matched title destination.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_LIES_INSIDE_OWN_EXTRACTION_OUTPUT":
    "Archive source the selected value lies inside its own extraction output.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_METADATA_ESCAPES_SOURCE_DIRECTORY":
    "Archive source metadata escapes its source directory.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_METADATA_EXCEEDS_ENTRY_LIMIT":
    "Archive source metadata exceeds entry limit.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_METADATA_MUST_IDENTIFY_REGULAR_FILE":
    "Archive source metadata must identify a regular file.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_NOT_REGULAR_DIRECTORY":
    "Archive source is not a regular directory.",
  "error.reason.IMPORT_ARCHIVE_SOURCE_SYMLINK_FORBIDDEN":
    "Archive source must not be a symlink.",
  "error.reason.IMPORT_ARCHIVE_STAGING_OUTPUT_DIRECTORY_NOT_EMPTY":
    "Archive staging output directory the selected value is not empty.",
  "error.reason.IMPORT_ARR_SOURCE_WARMUP_RESULT_NOT_FOUND":
    "No arr source warmup result was not found.",
  "error.reason.IMPORT_CHANGED_WHILE_RETRY_REQUESTED":
    "The import changed while retry was requested; refresh its history.",
  "error.reason.IMPORT_CLEANUP_ATTRIBUTION_MISSING":
    "The recorded cleanup has no valid routing attribution.",
  "error.reason.IMPORT_CLEANUP_LOCATOR_REUSED_IDENTITY_AMBIGUOUS":
    "Cleanup locator is reused or identity is ambiguous.",
  "error.reason.IMPORT_CLEANUP_STATE_INVALID":
    "The recorded cleanup state is invalid.",
  "error.reason.IMPORT_CLEANUP_TARGET_IS_OUTPUT_ROOT":
    "Refusing to delete download client output root the selected value.",
  "error.reason.IMPORT_CLEANUP_TARGET_OVERLAPS_LIBRARY_ROOT":
    "Refusing to delete download client payload the selected value because it overlaps a configured library root.",
  "error.reason.IMPORT_CLIENT_DOES_NOT_REPORT_COMPLETED_DOWNLOAD":
    "The client does not report a completed download; import retry cannot restart a failed or active download.",
  "error.reason.IMPORT_CLIENT_JOB_ID_NOW_BELONGS_DIFFERENT_DOWNLOAD":
    "The client job ID now belongs to a different download; use manual import for the retained files.",
  "error.reason.IMPORT_CLIENT_REPORTED_SUBTITLE_NOT_REGULAR_FILE":
    "Client-reported subtitle is not a regular file.",
  "error.reason.IMPORT_CLIENT_REPORTED_SUBTITLE_REPARSE_POINT":
    "Client-reported subtitle is a reparse point.",
  "error.reason.IMPORT_CLIENT_RETURNED_DIFFERENT_DOWNLOAD_IDENTITY":
    "The client returned a different download identity; refresh and retry.",
  "error.reason.IMPORT_COMPLETED_DOWNLOAD_NOT_OBSERVED_YET":
    "The completed download has not been observed yet; release again once it has.",
  "error.reason.IMPORT_COMPLETED_SOURCE_NO_TRUSTWORTHY_DOWNLOAD_IDENTITY":
    "The completed source has no trustworthy download identity; use manual import.",
  "error.reason.IMPORT_DESTINATION_OCCUPIED":
    "Refusing to restore old upgrade backup the selected value because destination is occupied.",
  "error.reason.IMPORT_DISC_IMPORT_TARGETS_INVALID":
    "Disc import targets must exactly match its explicit episode mappings.",
  "error.reason.IMPORT_DISC_SELECTION_CANNOT_INCLUDE_SEPARATE_EPISODE_SERIES_MOVIE_TARGET":
    "Disc selection cannot include a separate episode or series-movie target.",
  "error.reason.IMPORT_DISC_SELECTION_MAPPING_LIMIT_EXCEEDED_INVALID":
    "Invalid disc selection or mapping limit exceeded.",
  "error.reason.IMPORT_DISC_TITLES_REQUIRE_DISTINCT_EXPLICIT_SINGLE_EPISODE_MAPPINGS":
    "Disc titles require distinct, explicit single-episode mappings.",
  "error.reason.IMPORT_DISK_FULL":
    "There is not enough disk space to import the file.",
  "error.reason.IMPORT_DOWNLOAD_ALREADY_BEING_IMPORTED":
    "This download is already being imported.",
  "error.reason.IMPORT_DOWNLOAD_ALREADY_SETTLED":
    "The download is already settled; refresh its history.",
  "error.reason.IMPORT_DOWNLOAD_AWAITING_IMPORT_RETRY_RECONCILIATION":
    "This download is awaiting import retry reconciliation.",
  "error.reason.IMPORT_DOWNLOAD_BUSY_IMPORT_CHANGED":
    "The download is busy or the import changed; refresh and retry.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_COMPLETED_DOWNLOAD_NOT_FOUND":
    "Download client completed download the selected value is unavailable for payload cleanup.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_CONFIGURATION_NO_LONGER_MATCHES_SOURCE":
    "The download client configuration no longer matches this source; use manual import.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_NOT_FOUND":
    "Download client was not found.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PATH_MAPPING_UNUSABLE_LOCAL_ROOT":
    "Download client path mapping has an unusable local root.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_CHECKPOINT_NOT_DIRECTORY":
    "Download client payload cleanup checkpoint the selected value is not a directory.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_CLEANUP_REQUIRES_CONFIGURED_CLIENT_ID":
    "Download client payload cleanup requires a configured client id.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_INVALID":
    "Download client payload path must be absolute.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_NOT_FILE_DIRECTORY_SYMLINK":
    "Download client payload path the selected value is not a file, directory, or symlink.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_OUTSIDE_OUTPUT_ROOT":
    "Download client payload path the selected value is outside output root the selected value.",
  "error.reason.IMPORT_DOWNLOAD_CLIENT_PAYLOAD_PATH_TRAVERSES_SYMLINK":
    "Download client payload path the selected value traverses symlink the selected value.",
  "error.reason.IMPORT_DOWNLOAD_FAILED":
    "The download failed; retry the download before importing.",
  "error.reason.IMPORT_DOWNLOAD_HOLDS_MORE_ARCHIVE_SETS":
    "Download holds more than the selected value archive sets.",
  "error.reason.IMPORT_DOWNLOAD_NO_STABLE_IDENTITY_YET":
    "This download has no stable identity yet; its sources stay held.",
  "error.reason.IMPORT_DOWNLOAD_NO_TITLE_AUTHORIZE_RELEASE_AGAINST":
    "An import of this download has no title to authorize its release against.",
  "error.reason.IMPORT_DOWNLOAD_PAYLOAD_DIRECTORY_NO_LONGER_REAL_DIRECTORY":
    "Download payload directory the selected value is no longer a real directory.",
  "error.reason.IMPORT_DOWNLOAD_PAYLOAD_DIRECTORY_TOO_MANY_ENTRIES_INVENTORY_SAFELY":
    "Download payload directory the selected value has too many entries to inventory safely.",
  "error.reason.IMPORT_DOWNLOAD_PAYLOAD_INVENTORY_ENTRY_NOT_SAFE_RELATIVE_PATH":
    "Download payload inventory entry the selected value is not a safe relative path.",
  "error.reason.IMPORT_DOWNLOAD_STATE_UNAVAILABLE":
    "The client's current download state could not be established; try again when its history is available.",
  "error.reason.IMPORT_DOWNLOAD_STILL_BEING_IMPORTED":
    "This download is still being imported; release its sources once that finishes.",
  "error.reason.IMPORT_DO_NOT_ACCESS_ANY_LIBRARIES":
    "You do not have access to any libraries.",
  "error.reason.IMPORT_EPISODE_LOOKUP_FAILED":
    "The episode could not be loaded.",
  "error.reason.IMPORT_EPISODE_NOT_FOUND": "The episode was not found.",
  "error.reason.IMPORT_EPISODIC_DISCS_REQUIRE_PLAYBACK_TITLE_MAPPINGS":
    "Episodic discs require playback-title mappings; movie discs cannot map episodes.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_DEDUPKEY_DUPLICATED":
    "Duplicate external import setup dedupKey in the selected value.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_INSTANCEID_DUPLICATED":
    "Duplicate external import setup instanceId.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_CANNOT_EMPTY":
    "External import setup secret draft cannot be empty; clear the draft instead.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_OUT_OF_RANGE":
    "External import setup secret draft the selected value must be at most the selected value bytes.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_REQUIRED":
    "External import setup secret draft the selected value cannot be empty.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST":
    "External import setup secret draft supports at most the selected value the selected value.",
  "error.reason.IMPORT_EXTERNAL_IMPORT_SETUP_SECRET_DRAFT_SUPPORTS_MOST_INSTANCE_API_KEYS":
    "External import setup secret draft supports at most the selected value instance API keys.",
  "error.reason.IMPORT_EXTRACTED_ARCHIVE_WORKSPACE_UNAVAILABLE":
    "Extracted archive workspace is not accessible.",
  "error.reason.IMPORT_FILE_NOT_FOUND": "The source file was not found.",
  "error.reason.IMPORT_FILE_PAYLOAD_CHANGED_SINCE_IMPORT":
    "File payload changed since import; retaining source and client entry.",
  "error.reason.IMPORT_FILE_PAYLOAD_CLEANUP_REQUIRES_DURABLE_JOB_OWNERSHIP":
    "File payload cleanup requires durable job ownership.",
  "error.reason.IMPORT_FILE_PAYLOAD_OWNERSHIP_UNVERIFIED":
    "File payload ownership is unverified; retaining source and client entry.",
  "error.reason.IMPORT_GRAB_DOWNLOAD_CAME_COULD_NOT_RESOLVED":
    "The grab this download came from could not be resolved; release again once it can.",
  "error.reason.IMPORT_IMPORTED_VIDEO_NO_FILENAME_STEM":
    "Imported video has no filename stem.",
  "error.reason.IMPORT_IMPORTED_VIDEO_NO_PARENT_DIRECTORY":
    "Imported video has no parent directory.",
  "error.reason.IMPORT_INSUFFICIENT_DISK_ALLOWANCE_SCENE_SUBTITLES":
    "Insufficient disk allowance for scene subtitles; workspace preserved.",
  "error.reason.IMPORT_IO_FAILED": "The filesystem operation failed.",
  "error.reason.IMPORT_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.IMPORT_MANUAL_IMPORT_CANDIDATE_IDS_INVALID":
    "Manual import candidate ids must be unique.",
  "error.reason.IMPORT_MANUAL_IMPORT_CANDIDATE_ID_REQUIRED":
    "Manual import candidate id is required.",
  "error.reason.IMPORT_MANUAL_IMPORT_CANDIDATE_REQUIRED":
    "At least one manual import candidate is required.",
  "error.reason.IMPORT_MANUAL_IMPORT_EPISODE_TARGET_DOES_NOT_BELONG_TITLE":
    "Manual import episode target the selected value does not belong to title the selected value.",
  "error.reason.IMPORT_MANUAL_IMPORT_EPISODE_TARGET_UNAVAILABLE":
    "Manual import episode target is unavailable.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_INVALID":
    "Manual import file must have a file name.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_OUTSIDE_TRUSTED_SOURCE_ROOT":
    "Manual import file is outside the trusted source root.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_PATH_OUTSIDE_TRUSTED_SOURCE_ROOT":
    "Manual import file path is outside the trusted source root.",
  "error.reason.IMPORT_MANUAL_IMPORT_FILE_UNAVAILABLE":
    "Manual import file is not accessible.",
  "error.reason.IMPORT_MANUAL_IMPORT_MAPPING_CANNOT_INCLUDE_BOTH_EPISODE_ID_SERIES_MOVIE_LINK_ID":
    "Manual import mapping cannot include both episode_id and series_movie_link_id.",
  "error.reason.IMPORT_MANUAL_IMPORT_MAPPING_REQUIRES_EPISODE_ID_SERIES_MOVIE_LINK_ID":
    "Manual import mapping requires episode_id or series_movie_link_id.",
  "error.reason.IMPORT_MANUAL_IMPORT_PATH_UNAVAILABLE":
    "Manual import path is not accessible.",
  "error.reason.IMPORT_MANUAL_IMPORT_REQUEST_ACTOR_NO_LONGER_EXISTS":
    "Manual import request actor no longer exists.",
  "error.reason.IMPORT_MANUAL_IMPORT_REQUEST_MISSING_ACTOR":
    "Manual import request is missing an actor.",
  "error.reason.IMPORT_MANUAL_IMPORT_SERIES_MOVIE_TARGET_DOES_NOT_BELONG_TITLE":
    "Manual import series movie target the selected value does not belong to title the selected value.",
  "error.reason.IMPORT_MANUAL_IMPORT_SERIES_MOVIE_TARGET_UNAVAILABLE":
    "Manual import series movie target is unavailable.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_CONFLICTING_CANONICAL_DOWNLOAD_IDENTITY":
    "Manual import source has a conflicting canonical download identity.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_NOT_FILE":
    "Manual import source is not a file.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_ROOT_NOT_FILE_DIRECTORY":
    "Manual import source root is not a file or directory.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_ROOT_REQUIRED":
    "Manual import source root is required.",
  "error.reason.IMPORT_MANUAL_IMPORT_SOURCE_ROOT_UNAVAILABLE":
    "Manual import source root is not accessible.",
  "error.reason.IMPORT_MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD":
    "Manual import title does not match the Scryer submission that grabbed this download.",
  "error.reason.IMPORT_MANUAL_TITLE_OUTSIDE_DURABLE_SCRYER_SUBMISSION_TITLE":
    "Manual title the selected value is outside the durable Scryer submission title the selected value.",
  "error.reason.IMPORT_MAPPED_DISC_EPISODE_NO_LONGER_BELONGS_IMPORT_TITLE":
    "A mapped disc episode no longer belongs to the import title.",
  "error.reason.IMPORT_MEDIA_FILE_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED":
    "Media file changed before import metadata was published.",
  "error.reason.IMPORT_MEDIA_FILE_NOT_FOUND": "Media file was not found.",
  "error.reason.IMPORT_MEDIA_ROOT_REQUIRED":
    "Cannot safely upgrade the selected value because no configured media root could be resolved.",
  "error.reason.IMPORT_MULTIPLE_SCENE_SUBTITLES_MAP_SAME_DESTINATION":
    "Multiple scene subtitles map to the same destination; sources preserved.",
  "error.reason.IMPORT_NEWER_IMPORT_ATTEMPT_OWNS_DOWNLOAD":
    "A newer import attempt owns this download; refresh its history.",
  "error.reason.IMPORT_NOT_HOLDING_SOURCES":
    "Import the selected value is not holding its sources.",
  "error.reason.IMPORT_NO_COMPLETED_IMPORT_ATTEMPT_UNAVAILABLE":
    "No completed import attempt is available to retry; use manual import.",
  "error.reason.IMPORT_NZBGET_CLEANUP_IDENTITY_CHANGED":
    "NZBGet cleanup identity changed.",
  "error.reason.IMPORT_NZBGET_HISTORY_REMOVAL_WOULD_DELETE_RETAINED_UNVERIFIED_DATA":
    "NZBGet history removal would delete retained or unverified data.",
  "error.reason.IMPORT_NZBGET_NATIVE_DELETION_PARENT_UNAVAILABLE":
    "NZBGet native deletion parent is unavailable.",
  "error.reason.IMPORT_NZBGET_NATIVE_DELETION_PATH_UNVERIFIED":
    "NZBGet native deletion path is unverified.",
  "error.reason.IMPORT_NZBGET_TERMINAL_CLEANUP_METADATA_NOT_FOUND":
    "Nzbget terminal cleanup metadata is unavailable was not found.",
  "error.reason.IMPORT_PATH_OUTSIDE_LIBRARY_ROOT":
    "Refusing same-path upgrade guard because old file the selected value is outside media root the selected value.",
  "error.reason.IMPORT_PAYLOAD_DIRECTORY_OWNERSHIP_UNVERIFIED":
    "Payload directory ownership is unverified; refusing shared-directory deletion.",
  "error.reason.IMPORT_PAYLOAD_OVERLAPS_RECYCLE_ROOT":
    "Payload overlaps a recycle root.",
  "error.reason.IMPORT_PAYLOAD_PATH_UNSAFE":
    "Download client payload path must not be a filesystem root or contain traversal.",
  "error.reason.IMPORT_PERMISSION_DENIED":
    "The filesystem denied access to the import.",
  "error.reason.IMPORT_PROWLARR_WARMUP_RESULT_NOT_FOUND":
    "No prowlarr warmup result was not found.",
  "error.reason.IMPORT_RECORD_CONFLICTS_COMPLETED_SOURCE_IDENTITY":
    "The import record conflicts with its completed source identity; use manual import.",
  "error.reason.IMPORT_RECYCLE_BIN_CONFIG_REQUIRED":
    "Refusing to upgrade the selected value because no recycle bin config could be resolved for old file root the selected value.",
  "error.reason.IMPORT_REGULAR_FILE_REQUIRED":
    "Refusing to dispose old upgrade file the selected value because it is not a regular file.",
  "error.reason.IMPORT_RELEASING_THESE_SOURCES_ALSO_RELEASES_IMPORT_TITLE_MAY_NOT_RESOLVE_IMPORTS":
    "Releasing these sources also releases an import for a title you may not resolve imports for.",
  "error.reason.IMPORT_RESOURCE_NOT_FOUND": "Import was not found.",
  "error.reason.IMPORT_RETRY_ALREADY_RUNNING_AWAITING_RECONCILIATION":
    "This import retry is already running or awaiting reconciliation.",
  "error.reason.IMPORT_RETRY_IMPORT_RECORD_NOT_FOUND":
    "Retry import record was not found.",
  "error.reason.IMPORT_SAVED_DISC_SELECTION_CHANGED_BEFORE_IMPORT_METADATA_PUBLISHED":
    "Saved disc selection changed before import metadata was published.",
  "error.reason.IMPORT_SCENE_SUBTITLE_AGGREGATE_SIZE_LIMIT_EXCEEDED":
    "Scene subtitle aggregate size limit exceeded; workspace preserved.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DELIVERY_CONTEXT_UNAVAILABLE":
    "Scene subtitle delivery context is unavailable.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DESTINATION_ALREADY_EXISTS_DIFFERENT_CONTENTS":
    "Scene subtitle destination already exists with different contents; source preserved.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DIRECTORY_ENTRY_LIMIT_EXCEEDED":
    "Scene subtitle directory entry limit exceeded.",
  "error.reason.IMPORT_SCENE_SUBTITLE_DIRECTORY_NESTING_LIMIT_EXCEEDED":
    "Scene subtitle directory nesting limit exceeded.",
  "error.reason.IMPORT_SCENE_SUBTITLE_FILE_COUNT_LIMIT_EXCEEDED":
    "Scene subtitle file count limit exceeded; workspace preserved.",
  "error.reason.IMPORT_SCENE_SUBTITLE_HANDLE_LIES_OUTSIDE_OWNED_WORKSPACE_REPARSE_POINT":
    "Scene subtitle handle lies outside its owned workspace or is a reparse point.",
  "error.reason.IMPORT_SCENE_SUBTITLE_HANDLE_PATH_INVALID_UTF_16":
    "Scene subtitle handle path is invalid UTF-16.",
  "error.reason.IMPORT_SCENE_SUBTITLE_LIES_OUTSIDE_OWNED_WORKSPACE":
    "Scene subtitle lies outside its owned workspace.",
  "error.reason.IMPORT_SCENE_SUBTITLE_NOT_BOUNDED_REGULAR_FILE":
    "Scene subtitle is not a bounded regular file.",
  "error.reason.IMPORT_SCENE_SUBTITLE_NOT_REGULAR_FILE":
    "Scene subtitle is not a regular file.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_COMPONENT_INVALID":
    "Invalid scene subtitle path component.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_CONTAINS_SYMLINK":
    "Scene subtitle path contains a symlink.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_INVALID":
    "Invalid scene subtitle path.",
  "error.reason.IMPORT_SCENE_SUBTITLE_PATH_NO_FILENAME":
    "Scene subtitle path has no filename.",
  "error.reason.IMPORT_SCENE_SUBTITLE_SIZE_LIMIT_EXCEEDED":
    "Scene subtitle size limit exceeded.",
  "error.reason.IMPORT_SCENE_SUBTITLE_STAGING_REQUIRES_OWNED_ARCHIVE_WORKSPACE":
    "Scene subtitle staging requires an owned archive workspace.",
  "error.reason.IMPORT_SCENE_SUBTITLE_STAGING_WORKSPACE_UNAVAILABLE":
    "Scene subtitle staging workspace is unavailable.",
  "error.reason.IMPORT_SCENE_SUBTITLE_WORKSPACE_IDENTITY_CHANGED":
    "Scene subtitle workspace identity changed.",
  "error.reason.IMPORT_SCHEDULED_SCRIPTS_REQUIRE_SCHEDULE":
    "Scheduled scripts require a schedule.",
  "error.reason.IMPORT_SCRIPT_NOT_FOUND": "Script was not found.",
  "error.reason.IMPORT_SCRIPT_S_TRIGGER_CANNOT_CHANGED_AFTER_CREATED":
    "A script's trigger cannot be changed after it is created.",
  "error.reason.IMPORT_SOURCE_JOB_FAILED": "The source download failed.",
  "error.reason.IMPORT_SOURCE_UNAVAILABLE":
    "The manual import source is unavailable.",
  "error.reason.IMPORT_STATUS_ONLY_FAILED_SKIPPED_IMPORTS_RETRIED":
    "Import the selected value has status the selected item, only failed or skipped imports can be retried.",
  "error.reason.IMPORT_SYMLINK_REFUSED":
    "Refusing to dispose old upgrade file the selected value because it is a symlink.",
  "error.reason.IMPORT_UNAVAILABLE": "The import is no longer cancellable.",
  "error.reason.IMPORT_UNSAFE_ARCHIVE_ENTRY_PATH": "Unsafe archive entry path.",
  "error.reason.IMPORT_UPGRADE_RECYCLE_BIN_PATH_UNSAFE":
    "Refusing to upgrade because the recycle bin path is unsafe.",
  "error.reason.INTEGRATION_ACTION_PERMISSION_DENIED":
    "You do not have permission to perform this action.",
  "error.reason.INTEGRATION_API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY":
    "API key appears to be a masked placeholder — enter the real key.",
  "error.reason.INTEGRATION_ASSIGN_INDEXER_TEST_HTTP_3_CONNECT_THROUGH_DESTINATION":
    "Assign an indexer to test HTTP/3 CONNECT through its destination.",
  "error.reason.INTEGRATION_BASE_URL_CREDENTIALS_FORBIDDEN":
    "Base URL must not include embedded credentials.",
  "error.reason.INTEGRATION_BASE_URL_HOST_REQUIRED":
    "Base URL must include a host.",
  "error.reason.INTEGRATION_BASE_URL_INVALID":
    "Base URL must use http or https.",
  "error.reason.INTEGRATION_CHALLENGE_SOLVER_PROXIES_DO_NOT_ACCEPT_USERNAME_PASSWORD":
    "Challenge-solver proxies do not accept a username or password.",
  "error.reason.INTEGRATION_CLIENT_ID_REQUIRED": "Client id is required.",
  "error.reason.INTEGRATION_CLIENT_NAME_REQUIRED":
    "Client name cannot be empty.",
  "error.reason.INTEGRATION_CLIENT_TYPE_REQUIRED": "Client type is required.",
  "error.reason.INTEGRATION_DOWNLOAD_ALREADY_BEING_PROCESSED":
    "This download is already being processed.",
  "error.reason.INTEGRATION_DOWNLOAD_ALREADY_RESOLVED":
    "Download already resolved as the selected value; nothing to ignore.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_CONFIG_INVALID":
    "The download client configuration is invalid.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND":
    "Download client config was not found.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_ALL_PROTOCOL_FAMILIES_REQUIRED_INDEXER":
    "Download client the selected item does not support all protocol families required by indexer the selected item.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_PASSWORD_RETRIES":
    "This download client does not support password retries.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_FIELD_REQUIRED":
    "At least one download client field must be provided.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_ITEM_ID_REQUIRED":
    "Download client item id is required.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_NAME_REQUIRED":
    "Download client name is required.",
  "error.reason.INTEGRATION_DOWNLOAD_CLIENT_TYPE_UNSUPPORTED":
    "Unsupported download client type the selected item.",
  "error.reason.INTEGRATION_DOWNLOAD_NOT_STATE_IMPORTED_HAND_RIGHT_NOW":
    "This download is not in a state that can be imported by hand right now.",
  "error.reason.INTEGRATION_DOWNLOAD_SUBMISSION_NOT_FOUND":
    "Download submission was not found.",
  "error.reason.INTEGRATION_DOWNLOAD_UNAVAILABLE":
    "Download is no longer available for manual import.",
  "error.reason.INTEGRATION_ENABLE_PROXY_BEFORE_TESTING_OUTBOUND_ROUTE":
    "Enable this proxy before testing its outbound route.",
  "error.reason.INTEGRATION_FIELD_REQUIRED": "The requested value is required.",
  "error.reason.INTEGRATION_HELD_SOURCES_COULD_NOT_SETTLED":
    "Held sources could not be settled.",
  "error.reason.INTEGRATION_HTTP_3_CONNECT_ENDPOINT_INVALID":
    "HTTP/3 CONNECT endpoint must be https://host[:port] without a path, query or fragment.",
  "error.reason.INTEGRATION_HTTP_3_CONNECT_REQUIRES_HTTPS_ENDPOINT":
    "HTTP/3 CONNECT requires an https endpoint.",
  "error.reason.INTEGRATION_HTTP_PROXY_BASE_URL_INVALID":
    "HTTP proxy base URL must use http or https.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_ID_REQUIRED":
    "Indexer config id is required.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_INVALID":
    "The indexer configuration is invalid.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_JSON_INVALID":
    "Indexer config_json must be a JSON object.",
  "error.reason.INTEGRATION_INDEXER_CONFIG_NOT_FOUND":
    "Indexer config was not found.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT":
    "Indexer connection test did not return a valid Newznab caps document.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_TEST_FAILED":
    "Indexer connection test failed.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_UNREACHABLE":
    "The indexer could not be reached.",
  "error.reason.INTEGRATION_INDEXER_CONNECTION_URL_REQUIRED":
    "Indexer connection URL is required.",
  "error.reason.INTEGRATION_INDEXER_DOES_NOT_SUPPORT_TORRENTS_SO_SEEDING_PROFILE_CANNOT_ASSIGNED":
    "Indexer the selected item does not support torrents, so seeding profile the selected item cannot be assigned.",
  "error.reason.INTEGRATION_INDEXER_FIELD_REQUIRED":
    "At least one indexer field must be provided.",
  "error.reason.INTEGRATION_INDEXER_ID_REQUIRED": "Indexer id is required.",
  "error.reason.INTEGRATION_INDEXER_NAME_REQUIRED":
    "Indexer name cannot be empty.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_DECLARES_MULTIPLE_CONNECTION_URL_CONFIG_FIELDS":
    "Indexer provider declares multiple connection_url config fields.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_DOES_NOT_DECLARE_SUPPORTED_PROTOCOL_FAMILY":
    "Indexer provider the selected item does not declare a supported protocol family.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_TYPE_UNSUPPORTED":
    "Unsupported indexer provider type the selected item.",
  "error.reason.INTEGRATION_INDEXER_PROVIDER_UNAVAILABLE":
    "Indexer provider is unavailable.",
  "error.reason.INTEGRATION_INDEXER_REQUEST_REJECTED":
    "The indexer rejected the request.",
  "error.reason.INTEGRATION_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION":
    "Indexer settings changed during validation; reload and test again.",
  "error.reason.INTEGRATION_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXERS_CANNOT_SYNCED_DIRECTLY":
    "Managed child indexers cannot be synced directly.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC":
    "Managed child indexers are controlled by their parent sync.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXERS_CONTROLLED_THEIR_PARENT_SYNC_CANNOT_EDITED_DIRECTLY":
    "Managed child indexers are controlled by their parent sync and cannot be edited directly.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXER_REQUIRES_CHILD_KEY":
    "Managed child indexer requires a child key.",
  "error.reason.INTEGRATION_MANAGED_CHILD_INDEXER_REQUIRES_PARENT":
    "Managed child indexer requires a parent.",
  "error.reason.INTEGRATION_MANAGED_CHILD_PLAN_CONTAINS_DUPLICATE_CHILD_KEY":
    "Managed child plan contains duplicate child_key the selected item.",
  "error.reason.INTEGRATION_MANAGED_CHILD_PLAN_ENTRIES_REQUIRE_CHILD_KEY":
    "Managed child plan entries require child_key.",
  "error.reason.INTEGRATION_MANAGED_CHILD_REQUIRES_NAME":
    "Managed child the selected item requires a name.",
  "error.reason.INTEGRATION_MANAGED_CHILD_REQUIRES_PROVIDER_TYPE":
    "Managed child the selected item requires provider_type.",
  "error.reason.INTEGRATION_MANAGED_CHILD_ROUTING_CONTAINS_DUPLICATE_SCOPE":
    "Managed child routing contains duplicate scope the selected item.",
  "error.reason.INTEGRATION_MANAGED_CHILD_ROUTING_SCOPE_UNSUPPORTED":
    "Managed child routing scope the selected item is not supported.",
  "error.reason.INTEGRATION_MANAGED_INDEXERS_CANNOT_USE_PROXY":
    "Managed indexers cannot use a proxy; the managing application owns challenge solving.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_CANDIDATE_UNAVAILABLE":
    "Manual import candidate is unavailable.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_FILES_UNAVAILABLE":
    "Manual import files are no longer available.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_SELECTION_CHANGED":
    "Manual import selection changed; reopen the import dialog.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_SELECTION_TITLE_UNAVAILABLE":
    "Manual import selection title is unavailable.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_SELECTION_UNAVAILABLE":
    "Manual import selection is unavailable; reopen the import dialog.",
  "error.reason.INTEGRATION_MANUAL_IMPORT_TITLE_DOES_NOT_MATCH_SCRYER_SUBMISSION_GRABBED_DOWNLOAD":
    "Manual import title does not match the Scryer submission that grabbed this download.",
  "error.reason.INTEGRATION_MAPPED_DOWNLOAD_CLIENT_NO_LONGER_EXISTS":
    "Mapped download client the selected item no longer exists.",
  "error.reason.INTEGRATION_MAX_QUERIES_PER_MINUTE_OUT_OF_RANGE":
    "Max queries per minute must be at least 1, or empty for no budget.",
  "error.reason.INTEGRATION_MAX_QUERIES_PER_MINUTE_SET_EACH_MANAGED_INDEXER_NOT_INDEXER_SYNCS_THEM":
    "Max queries per minute is set on each managed indexer, not on the indexer that syncs them.",
  "error.reason.INTEGRATION_MINIMUM_SEEDERS_FLOOR_CANNOT_NEGATIVE":
    "Minimum seeders floor cannot be negative.",
  "error.reason.INTEGRATION_NOT_DNS_SERVER_ADDRESS":
    "`the selected value` is not a DNS server address; the `DNS` line takes IP addresses.",
  "error.reason.INTEGRATION_NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE":
    "No indexer management client available for provider type the selected item.",
  "error.reason.INTEGRATION_NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE":
    "No indexer provider available for provider type the selected item.",
  "error.reason.INTEGRATION_ONLY_SSH_TUNNELS_TAKE_PRIVATE_KEY_PASSPHRASE":
    "Only SSH tunnels take a private key passphrase.",
  "error.reason.INTEGRATION_ONLY_SSH_TUNNEL_PROXIES_PIN_HOST_KEY":
    "Only SSH tunnel proxies pin a host key.",
  "error.reason.INTEGRATION_ONLY_TUNNELS_TAKE_PRIVATE_KEY":
    "Only tunnels take a private key.",
  "error.reason.INTEGRATION_PASSWORD_RETRY_REQUIRES_EXACT_DOWNLOAD_CLIENT":
    "Password retry requires an exact download client.",
  "error.reason.INTEGRATION_PLUGIN_RETURNED_INVALID_CONFIG_FIELDS":
    "Plugin returned invalid config fields.",
  "error.reason.INTEGRATION_PRIVATE_KEY_INVALID":
    "Private key must be a PEM block beginning with -----BEGIN and ending with -----.",
  "error.reason.INTEGRATION_PROVIDER_TYPE_DOES_NOT_SUPPORT_MANAGED_CHILD_SYNC":
    "Provider type the selected item does not support managed child sync.",
  "error.reason.INTEGRATION_PROVIDER_TYPE_REQUIRED":
    "Provider type cannot be empty.",
  "error.reason.INTEGRATION_PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS":
    "Prowlarr indexers cannot use challenge solvers; Prowlarr owns challenge handling.",
  "error.reason.INTEGRATION_PROWLARR_MANAGEMENT_PARENTS_CANNOT_ASSIGNED_SEEDING_PROFILE":
    "Prowlarr management parents cannot be assigned a seeding profile.",
  "error.reason.INTEGRATION_PROWLARR_MANAGEMENT_PARENTS_CANNOT_MAPPED_DOWNLOAD_CLIENT":
    "Prowlarr management parents cannot be mapped to a download client.",
  "error.reason.INTEGRATION_PROXIES_DO_NOT_USE_CHALLENGE_SOLVER_PROTOCOL":
    "The requested value proxies do not use a challenge-solver protocol.",
  "error.reason.INTEGRATION_PROXY_BASE_URL_HOST_REQUIRED":
    "Proxy base URL must include a host.",
  "error.reason.INTEGRATION_PROXY_BASE_URL_INVALID":
    "Proxy base URL must include a port.",
  "error.reason.INTEGRATION_PROXY_BASE_URL_REQUIRED":
    "Proxy base URL is required.",
  "error.reason.INTEGRATION_PROXY_CONFIGURATION_NOT_FOUND":
    "Proxy configuration was not found.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ENABLED_DOWNLOAD_CLIENT_S":
    "Proxy config is assigned to the selected value enabled download client(s).",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ENABLED_INDEXER_S":
    "Proxy config is assigned to the selected value enabled indexer(s).",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ONE_MORE_DOWNLOAD_CLIENTS":
    "Proxy config is assigned to one or more download clients.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ASSIGNED_ONE_MORE_INDEXERS":
    "Proxy config is assigned to one or more indexers.",
  "error.reason.INTEGRATION_PROXY_CONFIG_ID_REQUIRED":
    "Proxy config id is required.",
  "error.reason.INTEGRATION_PROXY_CONFIG_NOT_FOUND":
    "Proxy config was not found.",
  "error.reason.INTEGRATION_PROXY_CREDENTIALS_INVALID":
    "The proxy credentials are invalid.",
  "error.reason.INTEGRATION_PROXY_DISABLED_DOWNLOAD_CLIENT":
    "Proxy is disabled for this download client.",
  "error.reason.INTEGRATION_PROXY_DISABLED_INDEXER":
    "Proxy is disabled for this indexer.",
  "error.reason.INTEGRATION_PROXY_FIELD_REQUIRED":
    "At least one proxy field must be provided.",
  "error.reason.INTEGRATION_PROXY_NAME_REQUIRED": "Proxy name is required.",
  "error.reason.INTEGRATION_PROXY_PASSWORD_REQUIRES_USERNAME":
    "Proxy password requires a username.",
  "error.reason.INTEGRATION_PROXY_TIMEOUT_OUT_OF_RANGE":
    "Proxy timeout must be between 1 and the selected value seconds.",
  "error.reason.INTEGRATION_PROXY_URL_CREDENTIALS_FORBIDDEN":
    "Proxy base URL must not embed credentials; use the username and password fields.",
  "error.reason.INTEGRATION_REMOTE_DNS_APPLIES_ONLY_SOCKS_PROXIES":
    "Remote DNS applies only to SOCKS proxies.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_FIELD_REQUIRED":
    "At least one seeding profile field must be provided.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_ID_REQUIRED":
    "Seeding profile id is required.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_NOT_FOUND":
    "Seeding profile was not found.",
  "error.reason.INTEGRATION_SEEDING_PROFILE_STILL_ASSIGNED":
    "Seeding profile is still assigned to the selected value.",
  "error.reason.INTEGRATION_SOCKS4_PROXIES_DO_NOT_CARRY_CREDENTIALS":
    "SOCKS4 proxies do not carry credentials; use SOCKS5 for an authenticated proxy.",
  "error.reason.INTEGRATION_SOCKS4_PROXY_BASE_URL_INVALID":
    "SOCKS4 proxy base URL must use socks4 or socks4a.",
  "error.reason.INTEGRATION_SOCKS5H_SOCKS4A_RESOLVE_NAMES_PROXY":
    "Socks5h:// and socks4a:// resolve names at the proxy; use socks5:// or socks4:// to resolve them locally.",
  "error.reason.INTEGRATION_SOCKS5_PROXY_BASE_URL_INVALID":
    "SOCKS5 proxy base URL must use socks5 or socks5h.",
  "error.reason.INTEGRATION_SSH_PRIVATE_KEY_INVALID":
    "The SSH private key could not be read.",
  "error.reason.INTEGRATION_SSH_TUNNELS_DO_NOT_ACCEPT_PASSWORDS":
    "SSH tunnels do not accept passwords; use an Ed25519 private key.",
  "error.reason.INTEGRATION_SSH_TUNNELS_REQUIRE_ED25519_PRIVATE_KEY":
    "SSH tunnels require an Ed25519 private key.",
  "error.reason.INTEGRATION_SSH_TUNNELS_REQUIRE_USERNAME":
    "SSH tunnels require a username.",
  "error.reason.INTEGRATION_SSH_TUNNEL_BASE_URL_INVALID":
    "SSH tunnel base URL must use ssh.",
  "error.reason.INTEGRATION_STORED_PROXY_PROTOCOL_DOES_NOT_MATCH_PROVIDER":
    "Stored proxy protocol does not match its provider.",
  "error.reason.INTEGRATION_TEST_URL_INVALID":
    "Test URL must use HTTP or HTTPS and contain no credentials.",
  "error.reason.INTEGRATION_TRACKED_DOWNLOAD_BUSY_PROCESSING":
    "Tracked download the selected value is busy processing.",
  "error.reason.INTEGRATION_TRACKED_DOWNLOAD_NOT_FOUND":
    "Tracked download was not found.",
  "error.reason.INTEGRATION_TUNNEL_MTU_OUT_OF_RANGE":
    "The tunnel MTU must be between the selected value and the selected value; the selected value is outside that range.",
  "error.reason.INTEGRATION_WIREGUARD_ADDRESS_INVALID":
    "The WireGuard interface address is invalid.",
  "error.reason.INTEGRATION_WIREGUARD_ADDRESS_REQUIRED":
    "WireGuard tunnels require at least one interface address; copy the `Address` line from the WireGuard configuration (for example `10.6.0.2/32`).",
  "error.reason.INTEGRATION_WIREGUARD_BASE_URL_INVALID":
    "WireGuard base URL must use wireguard.",
  "error.reason.INTEGRATION_WIREGUARD_FIELDS_WRONG_TUNNEL_KIND":
    "Only WireGuard tunnels take a peer public key, preshared key, tunnel addresses, DNS servers, MTU or keepalive.",
  "error.reason.INTEGRATION_WIREGUARD_KEYS_INVALID":
    "The WireGuard keys are invalid.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_AUTHENTICATE_KEYS_NOT_USERNAME_PASSWORD":
    "WireGuard tunnels authenticate with keys, not a username or password.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_NO_HOST_KEY_RESET":
    "WireGuard tunnels have no host key to reset.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_REQUIRE_PEER_S_PUBLIC_KEY_PEER_SECTION":
    "WireGuard tunnels require the peer's public key from the `[Peer]` section.",
  "error.reason.INTEGRATION_WIREGUARD_TUNNELS_REQUIRE_PRIVATE_KEY":
    "WireGuard tunnels require a private key; the selected value.",
  "error.reason.JOB_TASK_FAILED": "The requested background task failed.",
  "error.reason.JOB_ACQUISITION_SEARCH_JOBS_INVALID":
    "Acquisition search jobs must be started from the acquisition search mutation.",
  "error.reason.JOB_ACTION_PERMISSION_DENIED":
    "You do not have permission to perform this action.",
  "error.reason.JOB_ALREADY_RUNNING": "The selected item is already running.",
  "error.reason.JOB_APPLICATION_UPGRADE_JOBS_INVALID":
    "Application upgrade jobs must be started from the application upgrade mutation.",
  "error.reason.JOB_ARTWORK_ENCODING_SHUTTING_DOWN":
    "Artwork encoding is shutting down.",
  "error.reason.JOB_BACKGROUND_LIBRARY_REFRESH_TEMPORARILY_DISABLED":
    "Background library refresh is temporarily disabled.",
  "error.reason.JOB_CUSTOM_JOB_RUN_DOES_NOT_NAME_SCRIPT":
    "Custom job run does not name a script.",
  "error.reason.JOB_CUSTOM_JOB_RUN_NEEDS_ID_SCHEDULED_SCRIPT_RUN":
    "A custom job run needs the id of the scheduled script to run.",
  "error.reason.JOB_DISCOVERY_INCREMENTAL_REQUIRES_ACTIVE_GENERATION":
    "Discovery incremental requires an active generation.",
  "error.reason.JOB_DISCOVERY_INCREMENTAL_REQUIRES_PREVIOUS_FINGERPRINT":
    "Discovery incremental requires a previous fingerprint.",
  "error.reason.JOB_EMPTYING_RECYCLE_BIN_ALREADY_RUNNING":
    "Emptying the recycle bin is already running.",
  "error.reason.JOB_FULL_HASH_BACKFILL_SHUTTING_DOWN":
    "Full-hash backfill is shutting down.",
  "error.reason.JOB_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.JOB_LOCATION_OPERATIONS_INVALID":
    "Location operations must be started from the location operation mutation.",
  "error.reason.JOB_MEDIA_FILE_DELETION_JOBS_INVALID":
    "Media file deletion jobs must be started from the media file deletion mutation.",
  "error.reason.JOB_NO_LIBRARIES": "The requested value has no libraries.",
  "error.reason.JOB_ONLY_RUN_CONFIGURED_SCHEDULE":
    "The requested value can only run on its configured schedule.",
  "error.reason.JOB_PATH_OUTSIDE_LIBRARY_ROOT":
    "Refusing to restore recycle entry the selected value because original path is outside the resolved library roots.",
  "error.reason.JOB_RECYCLE_BIN_BATCHES_CANNOT_INCLUDE_SAME_ITEM_MORE_ONCE":
    "Recycle-bin batches cannot include the same item more than once.",
  "error.reason.JOB_RECYCLE_BIN_BATCHES_LIMITED_ITEMS":
    "Recycle-bin batches are limited to the selected value items.",
  "error.reason.JOB_RECYCLE_BIN_ITEM_REQUIRED":
    "Select at least one recycle-bin item.",
  "error.reason.JOB_RECYCLE_ENTRY_NOT_FOUND": "Recycle entry was not found.",
  "error.reason.JOB_RECYCLE_OPERATION_ALREADY_RUNNING":
    "A recycle operation is already running for the selected value.",
  "error.reason.JOB_RECYCLE_PURGE_JOBS_INVALID":
    "Recycle purge jobs must be started from the recycle bin mutation.",
  "error.reason.JOB_RECYCLE_RESTORE_ALREADY_RUNNING":
    "A recycle restore is already running for the selected value.",
  "error.reason.JOB_RECYCLE_RESTORE_ALREADY_RUNNING_ONE_SELECTED_DESTINATIONS":
    "A recycle restore is already running for one of the selected destinations.",
  "error.reason.JOB_RECYCLE_RESTORE_JOBS_INVALID":
    "Recycle restore jobs must be started from the recycle restore mutation.",
  "error.reason.JOB_RECYCLE_RESTORE_PREVIEW_STALE":
    "Recycle restore preview is stale; review the conflict choice again.",
  "error.reason.JOB_REGULAR_FILE_REQUIRED":
    "Refusing to replace restore destination the selected value because it is not a regular file.",
  "error.reason.JOB_REPLACE_RECYCLE_BIN_UNAVAILABLE":
    "Refusing to replace the selected value because the recycle bin is unavailable.",
  "error.reason.JOB_RESTORE_DESTINATION_CHANGED_AFTER_CONFIRMATION":
    "Restore destination changed after confirmation; preview and retry this item.",
  "error.reason.JOB_SCHEDULED_SCRIPT_ALREADY_RUNNING":
    "This scheduled script is already running.",
  "error.reason.JOB_SCHEDULED_SCRIPT_DISABLED":
    "Scheduled script the selected item is disabled.",
  "error.reason.JOB_SCHEDULED_SCRIPT_NO_LONGER_EXISTS":
    "Scheduled script the selected value no longer exists.",
  "error.reason.JOB_SCRIPT_NOT_FOUND": "Script was not found.",
  "error.reason.JOB_SCRIPT_NOT_SCHEDULED_SCRIPT":
    "Script the selected item is not a scheduled script.",
  "error.reason.JOB_SELECTED_RECYCLE_ENTRIES_CANNOT_SHARE_SAME_RESTORE_DESTINATION":
    "Selected recycle entries cannot share the same restore destination.",
  "error.reason.JOB_SELECT_FILES_SEASONS_TITLES_REFRESH_SCORES":
    "Select files, seasons, or titles to refresh scores.",
  "error.reason.JOB_SYMLINK_REFUSED":
    "Refusing to replace restore destination the selected value because it is a symlink.",
  "error.reason.JOB_TITLE_DELETION_JOBS_INVALID":
    "Title deletion jobs must be started from the title deletion mutation.",
  "error.reason.JOB_TITLE_RENAME_JOBS_INVALID":
    "Title rename jobs must be started from the rename mutation.",
  "error.reason.JOB_VIEW_RECYCLE_BIN_PERMISSION_DENIED":
    "You do not have permission to view the recycle bin.",
  "error.reason.LIBRARY_AUTOMATIC_TITLE_DISCOVERY_DEPTH_LIMIT_REACHED":
    "Automatic title discovery depth limit reached.",
  "error.reason.LIBRARY_BATCH_SIZE_OUT_OF_RANGE":
    "Batch size must be positive.",
  "error.reason.LIBRARY_COLLECTION_DOES_NOT_BELONG_TITLE":
    "Collection the selected value does not belong to title the selected value.",
  "error.reason.LIBRARY_CONFIGURED_ROOT_FOLDER_INVALID":
    "Configured the selected value root folder the selected value is invalid.",
  "error.reason.LIBRARY_DEFAULT_LIBRARIES_CANNOT_DELETED":
    "Default libraries cannot be deleted.",
  "error.reason.LIBRARY_DEFAULT_LIBRARY_NOT_FOUND":
    "Default  library was not found.",
  "error.reason.LIBRARY_DELETE_TARGET_NOT_FILE":
    "Refusing to delete directory path the selected value from a single-file delete flow.",
  "error.reason.LIBRARY_DELETE_TARGET_OUTSIDE_ROOTS":
    "Refusing to delete the selected value the selected value because it is outside the configured root folders.",
  "error.reason.LIBRARY_DELETE_TARGET_PARENT_REQUIRED":
    "Refusing to delete the selected value the selected value because it has no parent directory.",
  "error.reason.LIBRARY_DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY":
    "Destination folder contains an independent or unresolved title boundary.",
  "error.reason.LIBRARY_DESTINATION_OCCUPIED":
    "Refusing to restore recycled file the selected value because destination is occupied.",
  "error.reason.LIBRARY_EPISODE_FOLDER_OPTIONAL_GROUP_INVALID":
    "Invalid episode folder optional group.",
  "error.reason.LIBRARY_EPISODE_FOLDER_PATTERN_INVALID":
    "Invalid episode folder pattern.",
  "error.reason.LIBRARY_EPISODE_ID_REQUIRED":
    "At least one episode id is required.",
  "error.reason.LIBRARY_EPISODE_SELECTION_REQUIRED":
    "At least one episode must be selected.",
  "error.reason.LIBRARY_EXTERNAL_SUBTITLE_NOT_FOUND":
    "External subtitle was not found.",
  "error.reason.LIBRARY_FAILED_ATTACH_PENDING_IMPORT_FILE_SELECTED_MOVIE":
    "Failed to attach pending import file to selected movie.",
  "error.reason.LIBRARY_FAILED_BIND_PENDING_IMPORT_FILE_SELECTED_EPISODES":
    "Failed to bind pending import file to selected episodes.",
  "error.reason.LIBRARY_FOLDER_NOT_TITLE_BOUNDARY":
    "Destination folder already contains other title folders; it is an organizing folder, not a title folder.",
  "error.reason.LIBRARY_FOLDER_OWNERSHIP_CONFLICT":
    "Folder ownership conflicts cannot be bound or adopted.",
  "error.reason.LIBRARY_LINKED_FOLDER_REQUIRES_EXPLICIT_OWNERSHIP_RESOLUTION_BEFORE_SCANNING":
    "Linked folder requires explicit ownership resolution before scanning.",
  "error.reason.LIBRARY_MAINTENANCE_DELETION_FILE_CHANGED":
    "Maintenance deletion file changed; re-evaluation required.",
  "error.reason.LIBRARY_MAINTENANCE_DELETION_FINGERPRINT_CHANGED":
    "Maintenance deletion fingerprint changed.",
  "error.reason.LIBRARY_MAINTENANCE_DELETION_TARGETS_CHANGED":
    "Maintenance deletion targets changed; re-evaluation required.",
  "error.reason.LIBRARY_MEDIA_FILE_NOT_FOUND": "Media file was not found.",
  "error.reason.LIBRARY_MEDIA_IDENTITY_CONFLICTS_PROPOSED_TITLE_FOLDER":
    "Media identity conflicts with the proposed title folder.",
  "error.reason.LIBRARY_MEDIA_ROOT_REQUIRED":
    "Refusing to delete the selected value because no configured media roots are available.",
  "error.reason.LIBRARY_NAME_REQUIRED": "Library name is required.",
  "error.reason.LIBRARY_NO_ROOT_FOLDERS_UNAVAILABLE":
    "No configured the selected value root folders are available for safe deletion.",
  "error.reason.LIBRARY_ONE_MORE_SELECTED_EPISODES_DO_NOT_BELONG_TARGET_TITLE":
    "One or more selected episodes do not belong to the target title.",
  "error.reason.LIBRARY_ONLY_PROVIDER_BACKED_DOWNLOADED_SUBTITLES_BLOCKLISTED":
    "Only provider-backed downloaded subtitles can be blocklisted.",
  "error.reason.LIBRARY_OWNED_FOLDER_UNAVAILABLE":
    "Owned folder is unavailable.",
  "error.reason.LIBRARY_PATH_CANNOT_ESCAPE_ROOT":
    "Path cannot escape its root.",
  "error.reason.LIBRARY_PATH_COMPONENT_INVALID":
    "The requested value must not contain . or .. components.",
  "error.reason.LIBRARY_PATH_INVALID": "Path must be absolute.",
  "error.reason.LIBRARY_PATH_NOT_DIRECTORY": "Library path is not a directory.",
  "error.reason.LIBRARY_PATH_OUTSIDE_LIBRARY_ROOT":
    "Refusing to delete the selected value because it is outside the configured media roots.",
  "error.reason.LIBRARY_PENDING_IMPORT_ALREADY_BEING_RESOLVED":
    "Pending import the selected value is already being resolved.",
  "error.reason.LIBRARY_PENDING_IMPORT_DIRECTORY_CONTAINS_NO_FILES_ATTACH":
    "Pending import directory contains no files to attach.",
  "error.reason.LIBRARY_PENDING_IMPORT_DOES_NOT_KNOWN_TITLE":
    "Pending import does not have a known title.",
  "error.reason.LIBRARY_PENDING_IMPORT_FILE_UNAVAILABLE":
    "Pending import file is unavailable.",
  "error.reason.LIBRARY_PENDING_IMPORT_ID_REQUIRED":
    "Pending import id is required.",
  "error.reason.LIBRARY_PENDING_IMPORT_NOT_FOUND":
    "Pending import was not found.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_FOLDER_NOT_FILE_SO_CANNOT_BOUND_EPISODES":
    "Pending import path is a folder, not a file, so it cannot be bound to episodes; its stale title link was cleared, so reload pending imports and attach the folder to a title again.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_INVALID":
    "Pending import path is missing or invalid.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_NOT_FILE":
    "Pending import path is not a file.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_NOT_FILE_DIRECTORY":
    "Pending import path is not a file or directory.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_NO_PARENT_FOLDER":
    "Pending import path has no parent folder.",
  "error.reason.LIBRARY_PENDING_IMPORT_PATH_UNAVAILABLE":
    "Pending import path is unavailable.",
  "error.reason.LIBRARY_PENDING_IMPORT_REQUIRES_EXPLICIT_EPISODE_BINDING":
    "Pending import requires explicit episode binding.",
  "error.reason.LIBRARY_RECORDED_MEDIA_PATH_NO_FOLDER":
    "Recorded media path has no folder.",
  "error.reason.LIBRARY_RECORDED_MEDIA_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER":
    "Recorded media path is outside its title's owned folder.",
  "error.reason.LIBRARY_RECORDED_SUBTITLE_PATH_OUTSIDE_TITLE_S_OWNED_FOLDER":
    "Recorded subtitle path is outside its title's owned folder.",
  "error.reason.LIBRARY_RECYCLE_BIN_PATH_UNSAFE":
    "Refusing to recycle the selected value because the recycle bin path is unsafe.",
  "error.reason.LIBRARY_RECYCLE_ENTRY_ID_NOT_VALID_OPAQUE_ID":
    "Recycle entry id is not a valid opaque id.",
  "error.reason.LIBRARY_REGULAR_FILE_REQUIRED":
    "Refusing to restore recycled file the selected value because it is not a regular file.",
  "error.reason.LIBRARY_RENAMER_DISABLED": "Renamer_disabled.",
  "error.reason.LIBRARY_RENAME_ALREADY_RUNNING":
    "A rename is already running for the selected value.",
  "error.reason.LIBRARY_RENAME_FOLDERS_OVERLAP":
    "The selected title folders overlap.",
  "error.reason.LIBRARY_RENAME_PATH_OUTSIDE_ROOT":
    "The proposed rename path lies outside its library root.",
  "error.reason.LIBRARY_RENAME_STALE_PLAN": "Rename_stale_plan.",
  "error.reason.LIBRARY_REQUESTED_FACET_DOES_NOT_MATCH_TITLE_FACET":
    "Requested facet does not match title facet.",
  "error.reason.LIBRARY_RESOURCE_NOT_FOUND": "Library was not found.",
  "error.reason.LIBRARY_ROOTS_NOT_CONFIGURED":
    "The requested value library roots are not configured.",
  "error.reason.LIBRARY_ROOT_ALREADY_ASSIGNED":
    "Library root the selected item is already used by the selected value.",
  "error.reason.LIBRARY_ROOT_CANNOT_USED":
    "Library root the selected item cannot be used.",
  "error.reason.LIBRARY_ROOT_DUPLICATED":
    "Library roots must be unique within a library.",
  "error.reason.LIBRARY_ROOT_REQUIRED":
    "Libraries require at least one root folder.",
  "error.reason.LIBRARY_SCAN_ALREADY_RUNNING":
    "A library scan is already running.",
  "error.reason.LIBRARY_SCAN_CANCEL_UNAVAILABLE":
    "Library scan session is not cancelable.",
  "error.reason.LIBRARY_SCAN_FOLDER_BOUNDARY_INVALID":
    "The folder is not a valid title boundary for scanning.",
  "error.reason.LIBRARY_SCAN_SESSION_NOT_FOUND":
    "Library scan session was not found.",
  "error.reason.LIBRARY_SCAN_UNSUPPORTED":
    "Library scan is not supported for this facet.",
  "error.reason.LIBRARY_SCOPED_POLICY_DELETION_CANNOT_REMOVE_DIRECTORIES":
    "Scoped policy deletion cannot remove directories.",
  "error.reason.LIBRARY_SELECTED_FOLDER_ORGANIZING_FOLDER_MIXED_TITLE_LAYOUT":
    "Selected folder is an organizing folder or mixed title layout; select the title folder containing its media.",
  "error.reason.LIBRARY_SYMLINK_REFUSED":
    "Refusing to restore recycled file the selected value because it is a symlink.",
  "error.reason.LIBRARY_TEMPLATE_BRACES_UNBALANCED":
    "The requested value template contains an unmatched '}}'.",
  "error.reason.LIBRARY_TEMPLATE_IDENTITY_TOKEN_REQUIRED":
    "The naming template must include an identity token.",
  "error.reason.LIBRARY_TEMPLATE_OPTIONAL_GROUP_INVALID":
    "The template optional group is invalid.",
  "error.reason.LIBRARY_TEMPLATE_REQUIRED": "Rename template is required.",
  "error.reason.LIBRARY_TEMPLATE_TOKEN_UNSUPPORTED":
    "The naming template uses an unsupported token.",
  "error.reason.LIBRARY_TITLE_ALREADY_EXISTS_LIBRARY":
    "Title already exists in this library.",
  "error.reason.LIBRARY_TITLE_DIRECTORY_NOT_DIRECTORY":
    "Title directory is not a directory.",
  "error.reason.LIBRARY_TITLE_FOLDER_OVERLAPS_ANOTHER_TITLE":
    "Title folder overlaps another title; refusing media deletion.",
  "error.reason.LIBRARY_TITLE_IDENTITY_REQUIRED":
    "A title identity is required.",
  "error.reason.LIBRARY_TITLE_ID_RENAME_REQUEST_DUPLICATED":
    "Duplicate title id in rename request.",
  "error.reason.LIBRARY_TITLE_LIBRARY_SCAN_ONLY_SUPPORTED_EPISODIC_TITLES":
    "Title library scan is only supported for episodic titles.",
  "error.reason.LIBRARY_TITLE_NO_OWNED_FOLDER": "Title has no owned folder.",
  "error.reason.LIBRARY_TITLE_NO_RECORDED_MEDIA":
    "Title has no recorded media.",
  "error.reason.LIBRARY_TITLE_REQUIRED":
    "At least one title is required for renaming.",
  "error.reason.LIBRARY_UNCLOSED_EPISODE_FOLDER_TOKEN":
    "Unclosed episode folder token.",
  "error.reason.LIST_ACCOUNT_DISABLED": "Account is disabled.",
  "error.reason.LIST_ACCOUNT_LINK_NOT_FOUND":
    "List account link was not found.",
  "error.reason.LIST_ACCOUNT_NOT_FOUND": "List account was not found.",
  "error.reason.LIST_ADD_SEARCH_NEED_TITLE_MANAGEMENT_PERMISSION":
    "Add and search need title management permission.",
  "error.reason.LIST_COULD_NOT_START_ACCOUNT_LINK":
    "Could not start account link.",
  "error.reason.LIST_DEPARTURE_CHANGES_NEED_TITLE_MANAGEMENT_PERMISSION":
    "Departure changes need title management permission.",
  "error.reason.LIST_EXCLUSION_NOT_FOUND": "List exclusion was not found.",
  "error.reason.LIST_OWNER_DISABLED": "List owner is disabled.",
  "error.reason.LIST_OWNER_NOT_FOUND": "List owner was not found.",
  "error.reason.LIST_PERSONAL_LISTS_NOT_ALLOWED_MEMBER":
    "Personal lists are not allowed for this member.",
  "error.reason.LIST_PROVIDER_NOT_FOUND": "List provider was not found.",
  "error.reason.LIST_SUBSCRIPTION_CHANGED_NOT_FOUND":
    "List subscription changed was not found.",
  "error.reason.LIST_SUBSCRIPTION_NOT_FOUND":
    "List subscription was not found.",
  "error.reason.LIST_TITLE_NOT_FOUND": "Title was not found.",
  "error.reason.LIST_USER_NOT_FOUND": "User was not found.",
  "error.reason.LOCATION_CATALOG_CANNOT_MAP_FILE_OUTSIDE_RECORDED_FOLDER":
    "The catalog cannot map a file outside the recorded folder for the selected value. Correct its folder mapping first.",
  "error.reason.LOCATION_CATALOG_PATH_AMBIGUOUS":
    "The catalog path for the selected value is ambiguous. Correct its folder mapping before moving it.",
  "error.reason.LOCATION_CONFIRMED_PLAN_NO_INSTRUCTIONS_TITLE":
    "The confirmed plan has no instructions for title the selected value.",
  "error.reason.LOCATION_COULD_NOT_COMPARE":
    "Could not compare the selected value. Check that it is readable and try again; the source was preserved.",
  "error.reason.LOCATION_DESTINATION_CHANGED_DURING_COMPARISON":
    "The destination changed during comparison. The source was preserved; try again when the files are no longer changing.",
  "error.reason.LOCATION_DESTINATION_COPY_NO_LONGER_HOLDS_CONTENT_MOVE_PROVED_IDENTICAL_SOURCE":
    "The destination copy no longer holds the content this move proved identical to the source. The source was preserved; retry to compare the files again.",
  "error.reason.LOCATION_DESTINATION_OCCUPIED":
    "The destination staging path the selected value is occupied by a file this move did not write. Both files were preserved; move or remove that file and try again.",
  "error.reason.LOCATION_DESTINATION_REQUIRED":
    "Choose a destination library or root.",
  "error.reason.LOCATION_DESTINATION_TITLE_NOT_FOUND":
    "Destination title was not found.",
  "error.reason.LOCATION_EXISTING_PATH_NOT_REGULAR_FILE":
    "An existing path is not a regular file. Resolve the conflicting path and try again; the source was preserved.",
  "error.reason.LOCATION_FILE_CHANGED_BEFORE_SOURCE_CLEANUP":
    "A file changed before source cleanup. The source was preserved; retry to compare it again.",
  "error.reason.LOCATION_FILE_CHANGED_WHILE_BEING_COMPARED":
    "A file changed while it was being compared. The source was preserved; try again when the files are no longer changing.",
  "error.reason.LOCATION_FILE_CHANGED_WHILE_HASHING":
    "File changed while hashing.",
  "error.reason.LOCATION_FOLDER_ALREADY_OWNED_TITLE":
    "Folder the selected value is already owned by title the selected value; choose swap or take over.",
  "error.reason.LOCATION_FOLDER_NOT_INSIDE_ROOT_LIBRARY":
    "Folder the selected value is not inside a root of library the selected value.",
  "error.reason.LOCATION_FOLDER_NOT_OWNED_ANOTHER_TITLE":
    "Folder the selected value is not owned by another title; resolution the selected value does not apply.",
  "error.reason.LOCATION_FOLDER_NOT_TITLE_BOUNDARY":
    "Folder the selected value is a library root, not a title folder.",
  "error.reason.LOCATION_FOLDER_PATH_REQUIRED": "Folder path is required.",
  "error.reason.LOCATION_FULL_HASH_BACKFILL_ALREADY_RUNNING":
    "Full-hash backfill is already running.",
  "error.reason.LOCATION_FULL_HASH_BACKFILL_CANCELLED":
    "Full-hash backfill cancelled.",
  "error.reason.LOCATION_LIBRARY_NOT_FOUND": "Library was not found.",
  "error.reason.LOCATION_LIBRARY_ROOT_NOT_FOUND": "Library root was not found.",
  "error.reason.LOCATION_MERGE_TITLE_INTO_BLOCKED":
    "Merge of title the selected value into the selected value is blocked.",
  "error.reason.LOCATION_MERGE_UNAVAILABLE":
    "The merge has no valid identity mapping.",
  "error.reason.LOCATION_MORE_ONE_CATALOG_RECORD_WOULD_POINT":
    "More than one catalog record would point at the selected value. Resolve that catalog conflict before confirming the manual move.",
  "error.reason.LOCATION_OPERATION_NOT_FOUND":
    "Location operation was not found.",
  "error.reason.LOCATION_OPERATION_SOURCE_LIBRARIES_UNAVAILABLE":
    "Location operation source libraries are unavailable.",
  "error.reason.LOCATION_OPERATION_STILL_RUNNING":
    "Location operation the selected value is still running; cancel it instead of abandoning it.",
  "error.reason.LOCATION_OWNERSHIP_BUSY":
    "A location operation currently owns these items.",
  "error.reason.LOCATION_PATH_COMPONENT_INVALID":
    "Folder the selected value must not contain . or .. components.",
  "error.reason.LOCATION_PATH_NOT_FOUND": "Path was not found.",
  "error.reason.LOCATION_PATH_OUTSIDE_LIBRARY_ROOT":
    "Selected folder resolves outside its library root.",
  "error.reason.LOCATION_RECORDED_FOLDER_LIBRARY_ROOT_NOT_FOLDER_SWAP":
    "The recorded folder for the selected value is a library root, not a folder to swap; take over the folder instead.",
  "error.reason.LOCATION_RECOVERY_ALREADY_RUNNING":
    "Recovery is already running.",
  "error.reason.LOCATION_RECOVERY_COMMIT_RECEIPT_REQUIRED":
    "Recovery already committed; a missing receipt cannot authorize another copy.",
  "error.reason.LOCATION_RECOVERY_CONTENT_DISCOVERY_LIMIT_REACHED":
    "Content discovery limit reached.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_ANCESTOR_NOT_FOUND":
    "Recovery destination has no existing ancestor was not found.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_BOUNDARY_INVALID":
    "Recovery destination is not a valid title folder.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_CHANGED":
    "Destination changed during content discovery.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_CLAIM_FAILED":
    "Destination could not be claimed; original and staged files preserved.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_CONTENT_MISMATCH":
    "Destination content does not match; both files preserved.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_HASH_AMBIGUOUS":
    "Multiple destination files match the missing source hash.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_HASH_NOT_FOUND":
    "No destination matches the missing source's full-content hash was not found.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_INVENTORY_LIMIT_REACHED":
    "Generated destination inventory limit exceeded.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_NOT_FOUND":
    "Recovery destination missing was not found.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_OUTSIDE_LIBRARY":
    "Destination is outside the title library.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_OWNERSHIP_CONFLICT":
    "Another title owns an overlapping destination folder.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_PARENT_NOT_DIRECTORY":
    "Recovery destination parent is not a directory.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_PARENT_REQUIRED":
    "Destination has no parent.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_PATH_INVALID":
    "Recovery file destinations collide or escape the title folder.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_RECORD_CONFLICT":
    "A destination is already associated with another media record.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_ROOT_UNPROVEN":
    "No proven destination root.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_UNPROVEN":
    "No proven destination.",
  "error.reason.LOCATION_RECOVERY_DESTINATION_UNRELATED_CONTENT":
    "Generated destination contains unrelated content; ownership remains unresolved.",
  "error.reason.LOCATION_RECOVERY_DOT_SEGMENT_PATH_FORBIDDEN":
    "Recovery does not follow dot-segment paths.",
  "error.reason.LOCATION_RECOVERY_FILE_IDENTITY_UNAVAILABLE":
    "Filesystem identity is unavailable.",
  "error.reason.LOCATION_RECOVERY_FILE_IDENTITY_UNSUPPORTED":
    "Filesystem identity is unsupported.",
  "error.reason.LOCATION_RECOVERY_FILE_NOT_IN_PLAN":
    "Recovery file is not in the persisted plan.",
  "error.reason.LOCATION_RECOVERY_FILE_OWNERSHIP_CONFLICT":
    "Another title tracks a source or destination file.",
  "error.reason.LOCATION_RECOVERY_FOLDER_DISCOVERY_LIMIT_REACHED":
    "Folder discovery limit reached; no automatic repair.",
  "error.reason.LOCATION_RECOVERY_FOLDER_IDENTITY_AMBIGUOUS":
    "Multiple folders identify this title.",
  "error.reason.LOCATION_RECOVERY_FOLDER_IDENTITY_CHANGED":
    "Exact folder identity changed; ownership remains unresolved.",
  "error.reason.LOCATION_RECOVERY_FOLDER_OWNERSHIP_UNPROVEN":
    "No media or exact-ID folder establishes ownership.",
  "error.reason.LOCATION_RECOVERY_FOLDER_TITLE_IDENTITY_MISMATCH":
    "Tracked folder identity contradicts the title.",
  "error.reason.LOCATION_RECOVERY_FULL_HASH_REQUIRED": "Missing full hash.",
  "error.reason.LOCATION_RECOVERY_GENERATED_DESTINATION_IDENTITY_MISMATCH":
    "Generated destination is not this recovery's recorded folder.",
  "error.reason.LOCATION_RECOVERY_GENERATED_DESTINATION_NOT_DIRECTORY":
    "Generated destination is not a directory.",
  "error.reason.LOCATION_RECOVERY_GENERATED_DESTINATION_UNCLAIMED":
    "Generated destination already exists without ownership evidence.",
  "error.reason.LOCATION_RECOVERY_IDENTITY_EVIDENCE_CONFLICT":
    "Folder identity evidence conflicts.",
  "error.reason.LOCATION_RECOVERY_IDENTITY_SIDECAR_INVALID":
    "Identity sidecar is unsupported or too large.",
  "error.reason.LOCATION_RECOVERY_INDEPENDENT_COPY_REQUIRED":
    "Recovery requires an independent destination copy, not a hardlink.",
  "error.reason.LOCATION_RECOVERY_MEDIA_NOT_LOOSE_AT_ANCESTOR":
    "Media is not loose at the corrupt ancestor.",
  "error.reason.LOCATION_RECOVERY_MEDIA_PARENT_REQUIRED":
    "Media has no parent folder.",
  "error.reason.LOCATION_RECOVERY_NATIVE_ABSOLUTE_PATH_REQUIRED":
    "Recovery requires an absolute native filesystem path.",
  "error.reason.LOCATION_RECOVERY_OWNERSHIP_CHANGED":
    "Title ownership changed; originals are preserved.",
  "error.reason.LOCATION_RECOVERY_PATH_UNAVAILABLE":
    "Recovery path is unavailable.",
  "error.reason.LOCATION_RECOVERY_PLAN_NOT_FOUND":
    "Recovery plan was not found.",
  "error.reason.LOCATION_RECOVERY_PLAN_UNSUPPORTED":
    "Unsupported recovery plan.",
  "error.reason.LOCATION_RECOVERY_RECYCLE_STORAGE_OVERLAP":
    "Recovery destination overlaps configured recycle storage.",
  "error.reason.LOCATION_RECOVERY_REGULAR_FILE_REQUIRED":
    "Recovery requires a regular file.",
  "error.reason.LOCATION_RECOVERY_REPARSE_POINT_FORBIDDEN":
    "Recovery does not follow reparse points.",
  "error.reason.LOCATION_RECOVERY_ROOTS_CHANGED":
    "Library roots changed; recovery needs a new plan.",
  "error.reason.LOCATION_RECOVERY_ROOTS_REQUIRED":
    "Title library has no configured roots.",
  "error.reason.LOCATION_RECOVERY_ROOT_CORRUPTION_REQUIRED":
    "Title ownership is not a root corruption case.",
  "error.reason.LOCATION_RECOVERY_ROOT_NOT_FOUND":
    "Recovery root missing was not found.",
  "error.reason.LOCATION_RECOVERY_SELECTED_ROOT_NOT_FOUND":
    "Selected root was not found.",
  "error.reason.LOCATION_RECOVERY_SELECTED_ROOT_OUTSIDE_ANCESTOR":
    "Selected root is not below the corrupt ancestor.",
  "error.reason.LOCATION_RECOVERY_SOURCE_AND_DESTINATION_NOT_FOUND":
    "Both source and destination are missing was not found.",
  "error.reason.LOCATION_RECOVERY_SOURCE_CHANGED":
    "Source changed while verifying.",
  "error.reason.LOCATION_RECOVERY_SOURCE_CONTENT_PROOF_REQUIRED":
    "Missing source has no full-content proof.",
  "error.reason.LOCATION_RECOVERY_SOURCE_FILENAME_REQUIRED":
    "Source filename is unavailable.",
  "error.reason.LOCATION_RECOVERY_SOURCE_FULL_HASH_REQUIRED":
    "Missing source has no attested full hash.",
  "error.reason.LOCATION_RECOVERY_SOURCE_LOCATION_UNPROVEN":
    "Tracked source is neither in the proven folder nor loose at the corrupt ancestor.",
  "error.reason.LOCATION_RECOVERY_SOURCE_PARENT_UNAVAILABLE":
    "Source parent is unavailable.",
  "error.reason.LOCATION_RECOVERY_SYMLINK_FORBIDDEN":
    "Recovery does not follow symbolic links.",
  "error.reason.LOCATION_RECOVERY_TITLE_CHANGED":
    "Title changed during recovery; original mappings preserved.",
  "error.reason.LOCATION_RECOVERY_TITLE_NOT_FOUND": "Title was not found.",
  "error.reason.LOCATION_RECOVERY_TRACKED_MEDIA_CHANGED":
    "Tracked media changed; originals are preserved.",
  "error.reason.LOCATION_RELATED_MEDIA_FILE_COULD_NOT_TRANSFERRED":
    "The related media file could not be transferred. Its companion files were preserved.",
  "error.reason.LOCATION_RELATED_MEDIA_TRANSFER_INTERRUPTED":
    "The related media transfer was interrupted. The source was preserved.",
  "error.reason.LOCATION_ROOT_MOVE_SPANS_ONE_LIBRARY":
    "A root move spans one library; choose a destination library for a selection that spans several.",
  "error.reason.LOCATION_ROOT_NEW_PATH_REQUIRED":
    "Choose a new path for this root.",
  "error.reason.LOCATION_ROOT_NOT_FOUND": "Root  in library was not found.",
  "error.reason.LOCATION_SELECTED_FOLDER_OVERLAPS_ANOTHER_TITLE_S_FOLDER":
    "Selected folder overlaps another title's folder; no ownership was changed.",
  "error.reason.LOCATION_SELECTION_NOTHING_MOVE":
    "This selection has nothing to move.",
  "error.reason.LOCATION_SOURCE_CHANGED_BEFORE_CLEANUP":
    "Source changed before cleanup; preserved it.",
  "error.reason.LOCATION_SOURCE_CHANGED_DURING_FILE_PLACEMENT":
    "Source changed during file placement.",
  "error.reason.LOCATION_SOURCE_CHANGED_DURING_TRANSFER":
    "The source changed during transfer. It was preserved; try again when the file is no longer changing.",
  "error.reason.LOCATION_SOURCE_DESTINATION_CHANGED_BEFORE_CLEANUP":
    "Source or destination changed before cleanup.",
  "error.reason.LOCATION_SOURCE_DESTINATION_SAME_PATH":
    "Source and destination are the same path.",
  "error.reason.LOCATION_STALE_PLAN": "The plan changed. Preview it again.",
  "error.reason.LOCATION_TITLE_OWNS_NO_FOLDER_SWAP":
    "Title the selected value owns no folder to swap; take over the folder instead.",
  "error.reason.LOCATION_TITLE_SELECTION_REQUIRED":
    "Select at least one title to move.",
  "error.reason.LOCATION_TRANSFERRED_FILE_NEEDS_VERIFIED_AGAIN_BEFORE_CATALOG_ENTRY_UPDATED":
    "The transferred file the selected value needs to be verified again before its catalog entry can be updated.",
  "error.reason.LOCATION_TRANSFERRED_FILE_NOT_FINISHED_VERIFICATION":
    "The transferred file the selected value has not finished verification. Its catalog entry was left unchanged.",
  "error.reason.LOCATION_VERIFICATION_FAILED":
    "Verification of the selected value failedthe selected value.",
  "error.reason.LOCATION_VERIFICATION_MISMATCH":
    "The destination did not pass verification.",
  "error.reason.MAINTENANCE_ACTION_CONFIGURATION_INVALID":
    "The maintenance action configuration is invalid.",
  "error.reason.MAINTENANCE_ACTION_INVALID":
    "Maintenance action is not valid for a title-scoped rule.",
  "error.reason.MAINTENANCE_ACTION_NOT_STORABLE":
    "Maintenance action is not storable.",
  "error.reason.MAINTENANCE_ACTION_UNSUPPORTED":
    "This maintenance action is not supported for titles.",
  "error.reason.MAINTENANCE_ALREADY_RUN_ACTIONS_LIBRARY_DELETING_WOULD_ERASE_RECORD_WHAT_DID":
    "The selected item has already run actions on your library, and deleting it would erase the record of what it did. Set it to Disabled instead: the rule stops immediately and its history is kept.",
  "error.reason.MAINTENANCE_AMBIGUOUS_MAINTENANCE_FILE_COVERAGE":
    "Ambiguous maintenance file coverage.",
  "error.reason.MAINTENANCE_ARM_COUNT_MISMATCH":
    "The candidate count changed. Review the current count before arming.",
  "error.reason.MAINTENANCE_CHILD_DELETION_REQUIRES_EPISODE_SEASON":
    "Child deletion requires an episode or season.",
  "error.reason.MAINTENANCE_CURRENTLY_TRACKING_TITLE_S_DELETING_WOULD_DROP_THEM_WITHOUT_TRACE":
    "The selected item is currently tracking the selected value title(s), and deleting it would drop them without a trace. Set it to Disabled instead, or clear the candidates first.",
  "error.reason.MAINTENANCE_DELETEFILES_INCOMPATIBLE_UNMONITOR_PREREQUISITE":
    "DeleteFiles has an incompatible Unmonitor prerequisite.",
  "error.reason.MAINTENANCE_DELETEFILES_REQUIRES_EARLIER_UNMONITOR_STEP":
    "DeleteFiles requires an earlier Unmonitor step.",
  "error.reason.MAINTENANCE_EXCLUSION_NOT_FOUND":
    "Maintenance exclusion was not found.",
  "error.reason.MAINTENANCE_FIELD_REQUIRED":
    "The requested value must not be empty.",
  "error.reason.MAINTENANCE_FILE_OWNERSHIP_CHANGED":
    "Maintenance file ownership changed.",
  "error.reason.MAINTENANCE_FILE_TARGETS_CHANGED_ANOTHER_EVALUATION_REQUIRED":
    "Maintenance file targets changed; another evaluation is required.",
  "error.reason.MAINTENANCE_GRACE_PERIOD_INVALID":
    "Grace period must be zero or more days.",
  "error.reason.MAINTENANCE_GRACE_PERIOD_OUT_OF_RANGE":
    "Grace period must be at most the selected value days (ten years).",
  "error.reason.MAINTENANCE_MATCHER_READS_STORAGE_FACTS_MUST_SELECT_CONFIGURED_LIBRARY_ROOT":
    "A matcher that reads storage facts must select a configured library root.",
  "error.reason.MAINTENANCE_MOVIE_TITLES_DO_NOT_EPISODE_INVENTORY":
    "Movie titles do not have an episode inventory.",
  "error.reason.MAINTENANCE_PERSON_TARGETING_PERMISSION_REQUIRED":
    "This rule reads facts about specific people, so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
  "error.reason.MAINTENANCE_PREVIEW_ACCEPTS_MOST_TITLES_REQUESTED":
    "Preview accepts at most the selected value titles, the selected value were requested.",
  "error.reason.MAINTENANCE_PREVIEW_REQUIRES_BETWEEN_ONE_FIFTY_SUBJECTS":
    "Preview requires between one and fifty subjects.",
  "error.reason.MAINTENANCE_PREVIEW_SUBJECT_OUTSIDE_SCOPE":
    "Preview subject is missing, outside the rule libraries, or does not belong to the selected titles and scope.",
  "error.reason.MAINTENANCE_QUALITY_PROFILE_ACTION_NO_TARGET_PROFILE":
    "Quality-profile action has no target profile.",
  "error.reason.MAINTENANCE_RULE_CHANGED_WHILE_CONFIRMING":
    "The rule changed while confirming; review it again before arming deletion.",
  "error.reason.MAINTENANCE_RULE_CONFIGURATION_INVALID":
    "The rule configuration must be reviewed again before arming deletion; reopen the confirmation.",
  "error.reason.MAINTENANCE_RULE_EVALUATION_FAILED": "Rule evaluation failed.",
  "error.reason.MAINTENANCE_RULE_FAILED_COMPILE":
    "Maintenance rule the selected value failed to compile.",
  "error.reason.MAINTENANCE_RULE_SET_NOT_FOUND":
    "Maintenance rule set was not found.",
  "error.reason.MAINTENANCE_RULE_SHIPS_SCRYER":
    "This rule ships with Scryer: its matcher and actions cannot be changed. Its grace period, libraries and state can.",
  "error.reason.MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_DELETED":
    "This rule ships with Scryer and cannot be deleted. Set it to Disabled instead.",
  "error.reason.MAINTENANCE_RULE_SHIPS_SCRYER_CANNOT_RENAMED":
    "This rule ships with Scryer and cannot be renamed.",
  "error.reason.MAINTENANCE_RULE_SOURCE_INVALID":
    "The maintenance rule source is invalid.",
  "error.reason.MAINTENANCE_RULE_S_ACTION_NEVER_CHANGES_ANYTHING_SO_ONLY_RUN_DRY_RUN":
    "This rule's action never changes anything, so it can only run as a dry run.",
  "error.reason.MAINTENANCE_RULE_VALIDATION_FAILED": "Rule validation failed.",
  "error.reason.MAINTENANCE_SEQUENCE_ACTIONS_INVALID":
    "Sequence actions must use the sequence executor.",
  "error.reason.MAINTENANCE_SEQUENCE_STEP_PARAMETERS_INVALID":
    "Maintenance sequence step parameters are invalid.",
  "error.reason.MAINTENANCE_STORAGE_ROOT_NOT_CONFIGURED":
    "Storage root the selected item is not configured.",
  "error.reason.MAINTENANCE_SUBJECT_ID_REQUIRED":
    "Maintenance subject ID is required.",
  "error.reason.MAINTENANCE_SUBJECT_KIND_INVALID":
    "Invalid maintenance subject kind.",
  "error.reason.MAINTENANCE_SUBJECT_NO_LONGER_BELONGS_TITLE_NOT_FOUND":
    "Maintenance subject no longer belongs to this title was not found.",
  "error.reason.MAINTENANCE_TAG_ACTION_NO_TAGS_CONFIGURED":
    "Tag action has no tags configured.",
  "error.reason.MAINTENANCE_TARGET_QUALITY_PROFILE_DOES_NOT_EXIST":
    "Target quality profile the selected item does not exist.",
  "error.reason.MAINTENANCE_UNMONITORING_CANDIDATE_IDENTITY_DOES_NOT_MATCH_TITLE":
    "Maintenance unmonitoring candidate identity does not match its title.",
  "error.reason.MEDIA_ANALYSIS_INVALID":
    "The media analysis result is invalid.",
  "error.reason.MEDIA_ARTWORK_WORKER_COUNT_INVALID":
    "Artwork worker count must be two or four.",
  "error.reason.MEDIA_CACHED_AVIF_INVALID":
    "Cached JPEG encoder returned invalid AVIF.",
  "error.reason.MEDIA_CACHED_IMAGE_NOT_JPEG": "Cached image is not a JPEG.",
  "error.reason.MEDIA_CACHED_JPEG_DECODE_FAILED":
    "Failed to decode cached JPEG.",
  "error.reason.MEDIA_CACHED_JPEG_DIMENSIONS_INVALID":
    "Invalid cached JPEG size or target width.",
  "error.reason.MEDIA_CACHED_JPEG_KIND_UNSUPPORTED":
    "Unsupported cached JPEG kind.",
  "error.reason.MEDIA_CACHED_JPEG_TOO_LARGE":
    "Cached JPEG exceeds source size limit.",
  "error.reason.MEDIA_CLAIM_EXTENSION_NOT_FUTURE":
    "Claim extension must be in the future.",
  "error.reason.MEDIA_CLAIM_EXTENSION_NOT_LONGER":
    "Claim extension must lengthen a live finite lease.",
  "error.reason.MEDIA_COLLECTION_NOT_FOUND":
    "The collection could not be found.",
  "error.reason.MEDIA_COLLECTION_UPDATE_EMPTY":
    "At least one collection field must be provided.",
  "error.reason.MEDIA_DESTINATION_LIBRARY_NOT_FOUND":
    "The destination library could not be found.",
  "error.reason.MEDIA_DISC_ANALYSIS_REQUIRES_REVIEW_BEFORE_EPISODES_MADE_AVAILABLE":
    "Disc analysis requires review before episodes can be made available.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPINGS_REQUIRE_ISO_IMAGE":
    "Disc episode mappings require an ISO image.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPING_DUPLICATE":
    "Duplicate disc episode mapping.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPING_INVALID":
    "Invalid or incomplete disc episode mapping.",
  "error.reason.MEDIA_DISC_EPISODE_MAPPING_LIMIT_EXCEEDED":
    "Disc episode mapping limit exceeded.",
  "error.reason.MEDIA_DISC_EPISODE_TITLE_MISMATCH":
    "Disc episode belongs to a different title or no longer exists.",
  "error.reason.MEDIA_DISC_SELECTION_REQUIRES_ISO_IMAGE":
    "Disc selection requires an ISO image.",
  "error.reason.MEDIA_DISC_TITLE_RUNTIME_SECONDS_IMPLAUSIBLE_EPISODE":
    "Disc title the selected value runtime is implausible for episode the selected value.",
  "error.reason.MEDIA_EACH_EPISODE_DISTINCT_PLAYBACK_TITLE_OCCUR_ONLY_ONE_MAPPING":
    "Each episode and distinct playback title can occur in only one mapping.",
  "error.reason.MEDIA_EPISODE_MAPPINGS_REQUIRE_SERIES_TITLE":
    "Episode mappings require a series title.",
  "error.reason.MEDIA_EPISODE_NOT_FOUND": "The episode could not be found.",
  "error.reason.MEDIA_EPISODE_UPDATE_EMPTY":
    "At least one episode field must be provided.",
  "error.reason.MEDIA_EXTERNAL_ID_AMBIGUOUS":
    "External ids already map to multiple titles.",
  "error.reason.MEDIA_FACET_LIBRARY_NOT_FOUND":
    "The facet library could not be found.",
  "error.reason.MEDIA_FILE_NOT_FOUND": "Media file was not found.",
  "error.reason.MEDIA_FILE_OWNERSHIP_CHANGED": "Media file ownership changed.",
  "error.reason.MEDIA_IMAGE_DECODE_FAILED": "Failed to decode image.",
  "error.reason.MEDIA_IMAGE_DIMENSIONS_EMPTY":
    "Image dimensions must be non-zero.",
  "error.reason.MEDIA_IMAGE_FORMAT_DETECTION_FAILED":
    "Failed to detect image format.",
  "error.reason.MEDIA_IMAGE_FORMAT_UNSUPPORTED": "Unsupported image format.",
  "error.reason.MEDIA_IMAGE_NO_RECOGNIZED_DISC_TITLES":
    "Image has no recognized disc titles.",
  "error.reason.MEDIA_IMAGE_PROXY_CONTENT_TYPE_MISMATCH":
    "Image proxy response bytes do not match its image content type.",
  "error.reason.MEDIA_IMAGE_PROXY_CONTENT_TYPE_UNSUPPORTED":
    "Unsupported image proxy content type.",
  "error.reason.MEDIA_IMAGE_PROXY_RESPONSE_TOO_LARGE":
    "Image proxy response is too large.",
  "error.reason.MEDIA_IMAGE_PROXY_SOURCE_UNAPPROVED":
    "Unapproved image proxy source.",
  "error.reason.MEDIA_IMAGE_SOURCE_REJECTED":
    "The image source failed public HTTP URL validation.",
  "error.reason.MEDIA_LIBRARY_NOT_FOUND": "The library could not be found.",
  "error.reason.MEDIA_LIBRARY_PATH_NOT_DIRECTORY":
    "Library path must be a directory.",
  "error.reason.MEDIA_LIBRARY_PATH_UNAVAILABLE": "Library path error.",
  "error.reason.MEDIA_LIBRARY_ROOT_ALREADY_CONFIGURED":
    "This library root is already configured.",
  "error.reason.MEDIA_LIBRARY_ROOT_NOT_FOUND":
    "The library root could not be found.",
  "error.reason.MEDIA_LIBRARY_ROOT_PATH_EMPTY":
    "A library root path cannot be empty.",
  "error.reason.MEDIA_LIBRARY_ROOT_REFERENCED":
    "Library root cannot be removed while titles reference it.",
  "error.reason.MEDIA_LIFECYCLE_CLAIM_NOT_LIVE":
    "Lifecycle claim is no longer live.",
  "error.reason.MEDIA_LIST_ACCOUNT_NOT_FOUND":
    "The list account could not be found.",
  "error.reason.MEDIA_LIST_EXCLUSION_NOT_FOUND":
    "The list exclusion could not be found.",
  "error.reason.MEDIA_LIST_MOVIE_SELECTION_MISSING":
    "List movie selection is missing.",
  "error.reason.MEDIA_LIST_SUBSCRIPTION_NOT_FOUND":
    "The list subscription could not be found.",
  "error.reason.MEDIA_LOCATION_OPERATION_NOT_FOUND":
    "The location operation could not be found.",
  "error.reason.MEDIA_MAPPED_DISC_TITLE_INCOMPLETE_UNSUPPORTED_ANALYSIS":
    "Mapped disc title the selected value has incomplete or unsupported analysis.",
  "error.reason.MEDIA_MAPPED_DISC_TITLE_NO_ESTABLISHED_RUNTIME":
    "Mapped disc title the selected value has no established runtime.",
  "error.reason.MEDIA_MAPPED_DISC_TITLE_NO_LONGER_EXISTS":
    "Mapped disc title the selected value no longer exists; review the mapping.",
  "error.reason.MEDIA_MAPPED_EPISODE_DOES_NOT_BELONG_TITLE":
    "Mapped episode does not belong to this title.",
  "error.reason.MEDIA_MAP_EACH_COMPLETE_DISC_TITLE_ONE_EPISODE":
    "Map each complete disc title to one episode; combined episodes require separate authored playback titles.",
  "error.reason.MEDIA_MOST_SEARCH_LANGUAGES_ALLOWED":
    "At most the selected value search languages are allowed.",
  "error.reason.MEDIA_MOVIE_IDENTITY_UNSUPPORTED":
    "Selected movie has no supported identity.",
  "error.reason.MEDIA_NATIVE_DIAGNOSTICS_NOT_COMPILED_INTO_TARGET":
    "Native diagnostics are not compiled into this target.",
  "error.reason.MEDIA_NUMERIC_DISC_TITLE_IDENTITY_INVALID":
    "Invalid numeric disc title identity.",
  "error.reason.MEDIA_PENDING_IMPORT_UNRESOLVED":
    "Pending import could not be resolved for title.",
  "error.reason.MEDIA_PLAYBACK_CONNECTION_MISMATCH":
    "Playback mapping connection ID does not match persistence target.",
  "error.reason.MEDIA_RECOVERY_CANCELED_BEFORE_COMMIT":
    "Recovery canceled before catalog commit; original mappings preserved.",
  "error.reason.MEDIA_RECOVERY_CATALOG_EVIDENCE_LIMIT_EXCEEDED":
    "Recovery catalog evidence limit exceeded; no changes made.",
  "error.reason.MEDIA_RECOVERY_IDENTITY_CHANGED": "Recovery identity changed.",
  "error.reason.MEDIA_RECOVERY_MEDIA_CHANGED": "Media changed during recovery.",
  "error.reason.MEDIA_RECOVERY_NOT_REPLANNABLE":
    "Recovery cannot be replanned in its current state.",
  "error.reason.MEDIA_RECOVERY_OPERATION_INVALID":
    "Invalid recovery operation.",
  "error.reason.MEDIA_RECOVERY_OPERATION_NOT_FOUND":
    "The recovery operation could not be found.",
  "error.reason.MEDIA_RECOVERY_PLAN_UNRESOLVED":
    "Recovery plan changed or remains unresolved.",
  "error.reason.MEDIA_RECOVERY_ROOT_MISSING": "Missing recovery root.",
  "error.reason.MEDIA_RECOVERY_STAGING_EVIDENCE_LIMIT_EXCEEDED":
    "Recovery staging evidence limit exceeded.",
  "error.reason.MEDIA_RECOVERY_TITLE_NOT_FOUND":
    "The recovery title could not be found.",
  "error.reason.MEDIA_RECOVERY_VERIFICATION_MISMATCH":
    "Recovery requires matching full verification receipts.",
  "error.reason.MEDIA_RECOVERY_VERIFICATION_RECEIPT_REQUIRED":
    "Recovery file has no durable verification receipt.",
  "error.reason.MEDIA_REGISTERED_MEDIA_PATH_CHANGED_BEFORE_INSPECTION":
    "Registered media path changed before inspection.",
  "error.reason.MEDIA_RENAME_TARGET_PATH_REQUIRED":
    "Rename target path is required for move actions.",
  "error.reason.MEDIA_REQUESTED_DISC_TITLE_COULD_NOT_ESTABLISHED":
    "The requested disc title could not be established; review its inspection status.",
  "error.reason.MEDIA_REQUESTED_DISC_TITLE_DOES_NOT_EXIST":
    "The requested disc title does not exist.",
  "error.reason.MEDIA_REQUEST_ADVANCED_MONITORING_ONLY_VALID_SERIES_ANIME_TITLES":
    "Advanced monitoring is only valid for series and anime titles.",
  "error.reason.MEDIA_REQUEST_APPROVAL_TITLE_CHANGED":
    "Request title changed during approval; retry.",
  "error.reason.MEDIA_REQUEST_APPROVED_QUALITY_PROFILE_NAME_REQUIRED":
    "Approved quality profile name is required.",
  "error.reason.MEDIA_REQUEST_APPROVED_QUALITY_PROFILE_REQUIRED":
    "Approved quality profile is required.",
  "error.reason.MEDIA_REQUEST_CLAIM_EXTENSION_MUST_LENGTHEN_FINITE_LEASE_INTO_FUTURE":
    "Claim extension must lengthen a finite lease into the future.",
  "error.reason.MEDIA_REQUEST_DO_NOT_OWN_MEDIA_REQUEST":
    "You do not own this media request.",
  "error.reason.MEDIA_REQUEST_EXTERNAL_IDS_MAP_MULTIPLE_TITLES":
    "External ids map to multiple titles.",
  "error.reason.MEDIA_REQUEST_ID_REQUIRED": "Media request id is required.",
  "error.reason.MEDIA_REQUEST_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET":
    "Library facet does not match requested media facet.",
  "error.reason.MEDIA_REQUEST_LIBRARY_NOT_FOUND": "Library was not found.",
  "error.reason.MEDIA_REQUEST_LIFECYCLE_CLAIM_NOT_FOUND":
    "Lifecycle claim was not found.",
  "error.reason.MEDIA_REQUEST_LIST_MOVIE_APPROVAL_MUST_RETAIN_SELECTED_MOVIE":
    "A list movie approval must retain its selected movie.",
  "error.reason.MEDIA_REQUEST_LIST_MOVIE_REQUEST_MUST_RETAIN_SELECTED_MOVIE":
    "A list movie request must retain its selected movie.",
  "error.reason.MEDIA_REQUEST_MEDIA_REQUESTS_INVALID":
    "Media requests must include a searchable SMG identifier.",
  "error.reason.MEDIA_REQUEST_MONITORING_TARGET_REQUIRED":
    "Advanced monitoring requires at least one season or series movie.",
  "error.reason.MEDIA_REQUEST_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND":
    "Movie metadata response missing title was not found.",
  "error.reason.MEDIA_REQUEST_NOT_DISMISSED":
    "Only a dismissed media request can be reopened.",
  "error.reason.MEDIA_REQUEST_NOT_FOUND": "The request could not be found.",
  "error.reason.MEDIA_REQUEST_NOT_PENDING": "Request is no longer pending.",
  "error.reason.MEDIA_REQUEST_NO_LONGER_PENDING":
    "Media request is no longer pending.",
  "error.reason.MEDIA_REQUEST_ONLY_DISMISSED_MEDIA_REQUEST_REOPENED":
    "Only a dismissed media request can be reopened.",
  "error.reason.MEDIA_REQUEST_QUALITY_PROFILE_ID_REQUIRED":
    "Quality profile id is required.",
  "error.reason.MEDIA_REQUEST_QUALITY_PROFILE_UNKNOWN":
    "Unknown quality profile the selected value.",
  "error.reason.MEDIA_REQUEST_RELEASE_REASON_REQUIRED":
    "A release reason is required.",
  "error.reason.MEDIA_REQUEST_REQUESTED_QUALITY_PROFILE_NOT_ALLOWED_LIBRARY":
    "Requested quality profile is not allowed for this library.",
  "error.reason.MEDIA_REQUEST_REQUEST_MONITOR_TYPE_UNSUPPORTED":
    "Unsupported request monitor type the selected value.",
  "error.reason.MEDIA_REQUEST_REQUEST_TITLE_REQUIRED":
    "Request title is required.",
  "error.reason.MEDIA_REQUEST_RESOURCE_NOT_FOUND":
    "Media request was not found.",
  "error.reason.MEDIA_REQUEST_STATE_NOT_PENDING":
    "Media request is no longer pending.",
  "error.reason.MEDIA_REQUEST_TITLE_ALREADY_EXISTS_TARGET_LIBRARY":
    "Title already exists in the target library.",
  "error.reason.MEDIA_REQUEST_TITLE_BEING_MODIFIED":
    "Title is being modified; retry approval.",
  "error.reason.MEDIA_REQUEST_TITLE_CHANGED_DURING_APPROVAL":
    "Title changed during approval; retry.",
  "error.reason.MEDIA_REQUEST_TITLE_NOT_FOUND": "Title was not found.",
  "error.reason.MEDIA_SCAN_BATCH_SIZE_INVALID":
    "Batch size must be greater than 0.",
  "error.reason.MEDIA_SEARCH_LANGUAGE_CHOOSE_LANGUAGE_OFFERED_SEARCH_LANGUAGE_PICKER_UNKNOWN":
    "Unknown search language the selected value; choose a language offered by the search language picker.",
  "error.reason.MEDIA_SELECTED_ROOT_CHANGED": "Selected root changed.",
  "error.reason.MEDIA_SERIES_MOVIE_LINK_NOT_FOUND":
    "The series movie link could not be found.",
  "error.reason.MEDIA_SERVER_BASE_URL_INVALID":
    "Media server base URL must be an HTTP or HTTPS URL.",
  "error.reason.MEDIA_SERVER_BASE_URL_QUERY_OR_FRAGMENT":
    "Media server base URL must not include a query or fragment.",
  "error.reason.MEDIA_SERVER_BASE_URL_REQUIRED":
    "Media server base URL is required.",
  "error.reason.MEDIA_SERVER_BOTH_EMBY_ADMINISTRATOR_USERNAME_PASSWORD_REQUIRED":
    "Both Emby administrator username and password are required.",
  "error.reason.MEDIA_SERVER_BOTH_JELLYFIN_ADMIN_USERNAME_PASSWORD_REQUIRED":
    "Both Jellyfin admin username and password are required.",
  "error.reason.MEDIA_SERVER_CANNOT_CHANGE_PROVIDER_CONNECTION_LINKED_ACCOUNTS":
    "Cannot change provider for a connection with linked accounts.",
  "error.reason.MEDIA_SERVER_CHANGING_EMBY_SERVER_URL_REQUIRES_STORED_API_KEY":
    "Changing an Emby server URL requires a stored API key.",
  "error.reason.MEDIA_SERVER_CHOOSE_EITHER_EMBY_API_KEY_LOCAL_ADMINISTRATOR_CREDENTIALS_NOT_BOTH":
    "Choose either an Emby API key or local administrator credentials, not both.",
  "error.reason.MEDIA_SERVER_CHOOSE_EITHER_JELLYFIN_API_KEY_JELLYFIN_ADMIN_LOGIN_CREDENTIALS_NOT_BOTH":
    "Choose either a Jellyfin API key or Jellyfin admin login credentials, not both.",
  "error.reason.MEDIA_SERVER_CONNECTION_ID_REQUIRED":
    "Media server connection id is required.",
  "error.reason.MEDIA_SERVER_CONNECTION_NOT_FOUND":
    "The server connection could not be found.",
  "error.reason.MEDIA_SERVER_CONNECTION_REFERENCED_LINKED_ACCOUNTS":
    "Media server connection is referenced by linked accounts; disable it instead.",
  "error.reason.MEDIA_SERVER_CONNECTION_REFERENCED_NOTIFICATION_CHANNELS":
    "Media server connection is referenced by notification channels; disable it instead.",
  "error.reason.MEDIA_SERVER_DISCOVER_SELECT_PLEX_SERVER_BEFORE_ENABLING_LOGIN_LINKING_AUTO_ADD":
    "Discover and select a Plex server before enabling login, linking, or auto-add.",
  "error.reason.MEDIA_SERVER_EMBY_API_KEY_REQUIRED":
    "Emby API key is required.",
  "error.reason.MEDIA_SERVER_EMBY_AVATAR_NOT_FOUND":
    "Emby avatar was not found.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY":
    "Emby connection has no verified server identity.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECTION_REQUIRES_VERIFIED_INTEGRATION_API_KEY":
    "Emby connection requires a verified integration API key.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECTION_TEST_REQUIRES_SAVED_INTEGRATION_API_KEY":
    "Emby connection test requires a saved integration API key.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_LOCAL_CREDENTIALS_FORBIDDEN":
    "Emby Connect setup does not accept a pasted API key or local administrator credentials.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_LOGIN_REQUIRES_VERIFIED_SERVER_IDENTITY":
    "Emby Connect login requires a verified server identity.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_PASSWORD_REQUIRED":
    "Emby Connect password is required.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_SIGN_DISABLED_CONNECTION":
    "Emby Connect sign-in is disabled for this connection.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_TEST_REQUIRES_EMBY_CONNECTION":
    "Emby Connect test requires an Emby connection.",
  "error.reason.MEDIA_SERVER_EMBY_CONNECT_USERNAME_EMAIL_REQUIRED":
    "Emby Connect username or email is required.",
  "error.reason.MEDIA_SERVER_EMBY_SERVER_IDENTITY_DOES_NOT_MATCH_SAVED_CONNECTION":
    "Emby server identity does not match the saved connection.",
  "error.reason.MEDIA_SERVER_EMBY_USER_LISTING_REQUIRES_EMBY_CONNECTION":
    "Emby user listing requires an Emby connection.",
  "error.reason.MEDIA_SERVER_EMBY_USER_LISTING_REQUIRES_SAVED_INTEGRATION_API_KEY":
    "Emby user listing requires a saved integration API key.",
  "error.reason.MEDIA_SERVER_ENABLED_EMBY_CONNECTION_MUST_RETAIN_VERIFIED_API_KEY":
    "An enabled Emby connection must retain a verified API key.",
  "error.reason.MEDIA_SERVER_EPISODE_NOT_FOUND": "Episode was not found.",
  "error.reason.MEDIA_SERVER_EXTERNAL_URL_CREDENTIALS_FORBIDDEN":
    "Media server external URL must not include credentials.",
  "error.reason.MEDIA_SERVER_EXTERNAL_URL_INVALID":
    "Media server external URL must be an HTTP or HTTPS URL.",
  "error.reason.MEDIA_SERVER_EXTERNAL_URL_QUERY_OR_FRAGMENT":
    "Media server external URL must not include a query or fragment.",
  "error.reason.MEDIA_SERVER_JELLYFIN_USER_LISTING_REQUIRES_JELLYFIN_CONNECTION":
    "Jellyfin user listing requires a Jellyfin connection.",
  "error.reason.MEDIA_SERVER_JELLYFIN_USER_LISTING_REQUIRES_SAVED_API_KEY":
    "Jellyfin user listing requires a saved API key; save an API key to load Jellyfin users.",
  "error.reason.MEDIA_SERVER_LOCAL_EMBY_SETUP_DOES_NOT_ACCEPT_EMBY_CONNECT_CREDENTIALS":
    "Local Emby setup does not accept Emby Connect credentials.",
  "error.reason.MEDIA_SERVER_PATH_MAPPINGS_INVALID":
    "Media server path mappings must have unique source paths.",
  "error.reason.MEDIA_SERVER_PLEX_ACCOUNT_DOES_NOT_ACCESS_SELECTED_SERVER":
    "Plex account does not have access to the selected server.",
  "error.reason.MEDIA_SERVER_PLEX_DID_NOT_RETURN_ANY_ACCESSIBLE_SERVERS_ACCOUNT":
    "Plex did not return any accessible servers for that account.",
  "error.reason.MEDIA_SERVER_SELECTED_EMBY_CONNECT_SERVER_IDENTITY_DOES_NOT_MATCH_REACHABLE_SERVER":
    "Selected Emby Connect server identity does not match the reachable server.",
  "error.reason.MEDIA_SERVER_SELECTED_PLEX_SERVER_NOT_RETURNED_PLEX_DISCOVERY":
    "The selected Plex server was not returned by Plex discovery.",
  "error.reason.MEDIA_SERVER_SELECT_EMBY_CONNECT_SERVER":
    "Select an Emby Connect server.",
  "error.reason.MEDIA_SERVER_SELECT_PLEX_SERVER_BEFORE_SAVING_CONNECTION":
    "Select a Plex server before saving this connection.",
  "error.reason.MEDIA_SERVER_SIGN_PLEX_BEFORE_SELECTING_PLEX_SERVER":
    "Sign in with Plex before selecting a Plex server.",
  "error.reason.MEDIA_SERVER_SIGN_PLEX_TEST_CONNECTION":
    "Sign in with Plex to test this connection.",
  "error.reason.MEDIA_SOURCE_CHANGED_DURING_INSPECTION":
    "Media source changed during inspection.",
  "error.reason.MEDIA_SOURCE_SELECTION_CHANGED_DURING_INSPECTION":
    "Media source or selection changed during inspection; refresh and retry.",
  "error.reason.MEDIA_SOURCE_SELECTION_CHANGED_DURING_MAPPING":
    "Media source or selection changed during mapping; refresh and retry.",
  "error.reason.MEDIA_TITLE_FACET_CHANGE_UNSUPPORTED":
    "Changing a title facet is not supported because titles cannot move between libraries.",
  "error.reason.MEDIA_TITLE_IMAGE_CONTENT_TYPE_UNSUPPORTED":
    "Unsupported title image content type.",
  "error.reason.MEDIA_TITLE_IMAGE_HOST_UNAPPROVED":
    "The title image host is not an approved artwork provider.",
  "error.reason.MEDIA_TITLE_IMAGE_REDIRECT_FORBIDDEN":
    "Title image redirects are not allowed.",
  "error.reason.MEDIA_TITLE_IMAGE_TOO_LARGE":
    "Title image exceeds max size of bytes.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_CREDENTIALS_FORBIDDEN":
    "Title image URL must not include credentials.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_EMPTY":
    "Title image URL must not be empty.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_HOST_REQUIRED":
    "Title image URL must include a host.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_INVALID": "Invalid title image URL.",
  "error.reason.MEDIA_TITLE_IMAGE_URL_SCHEME_UNSUPPORTED":
    "Title image URL must use http or https.",
  "error.reason.MEDIA_TITLE_MERGE_BLOCKED":
    "The selected titles cannot be merged.",
  "error.reason.MEDIA_TITLE_NAME_EMPTY": "Title name cannot be empty.",
  "error.reason.MEDIA_TITLE_SLUG_AMBIGUOUS":
    "The title slug maps to multiple titles.",
  "error.reason.MEDIA_TITLE_TAG_LIMIT_EXCEEDED":
    "The title has reached its tag limit.",
  "error.reason.MEDIA_TITLE_TAG_NOT_FOUND": "The title tag could not be found.",
  "error.reason.MEDIA_TITLE_UPDATE_EMPTY":
    "At least one title field must be provided.",
  "error.reason.MEDIA_TRANSFER_DESTINATION_REQUIRED":
    "A transfer needs both a destination library and a destination root.",
  "error.reason.MEDIA_VIDEO_NOT_FOUND":
    "No video stream was found in the selected media.",
  "error.reason.NOTIFICATION_CHANNELS_CANNOT_REFERENCE_MEDIA_SERVER_CONNECTIONS":
    "Notification channels cannot reference media server connections.",
  "error.reason.NOTIFICATION_CHANNEL_DISABLED":
    "Notification channel is disabled.",
  "error.reason.NOTIFICATION_CHANNEL_MEDIA_SERVER_CONNECTION_PROVIDER_DOES_NOT_MATCH_CHANNEL_TYPE":
    "Notification channel media server connection provider does not match channel type.",
  "error.reason.NOTIFICATION_CHANNEL_NAME_REQUIRED":
    "Channel name must not be empty.",
  "error.reason.NOTIFICATION_CHANNEL_NOT_FOUND":
    "The channel could not be found.",
  "error.reason.NOTIFICATION_CHANNEL_TYPE_INVALID": "Invalid channel type.",
  "error.reason.NOTIFICATION_CONFIGURATION_INVALID":
    "Notification configuration must be a JSON object.",
  "error.reason.NOTIFICATION_EVENT_TYPE_NOT_SUBSCRIBABLE":
    "Notification event type is not subscribable.",
  "error.reason.NOTIFICATION_EVENT_TYPE_UNKNOWN":
    "Unknown notification event type.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_CONNECTION_DISABLED":
    "Media server connection is disabled.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_CONNECTION_NOT_FOUND":
    "Media server connection was not found.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_NOTIFICATION_PROVIDERS_INVALID":
    "Media server notification providers must be targeted through media server connections.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_NOTIFICATION_TARGETS_INVALID":
    "Media server notification targets must be managed in Media Servers.",
  "error.reason.NOTIFICATION_MEDIA_SERVER_PROVIDER_INVALID":
    "Invalid media server provider the selected item.",
  "error.reason.NOTIFICATION_PLEX_MEDIA_SERVER_BASE_URL_INVALID":
    "Invalid Plex media server base URL the selected item.",
  "error.reason.NOTIFICATION_PLUGIN_CHANNEL_TYPE_NOT_FOUND":
    "No notification plugin for channel type was not found.",
  "error.reason.NOTIFICATION_PROVIDER_DOES_NOT_SUPPORT_EVENT":
    "Notification provider the selected item does not support event the selected item.",
  "error.reason.NOTIFICATION_PROVIDER_DOES_NOT_SUPPORT_TEST_NOTIFICATIONS":
    "Notification provider the selected item does not support test notifications.",
  "error.reason.NOTIFICATION_PROVIDER_REQUIRES":
    "Notification provider the selected item requires the selected item.",
  "error.reason.NOTIFICATION_PROVIDER_UNAVAILABLE":
    "Notification provider the selected item is not available.",
  "error.reason.NOTIFICATION_SUBSCRIPTION_NOT_FOUND":
    "The subscription could not be found.",
  "error.reason.NOTIFICATION_SUBSCRIPTION_TARGET_REQUIRED":
    "Notification subscription target is required.",
  "error.reason.NOTIFICATION_TARGET_ID_REQUIRED_TARGET_KIND":
    "Notification target id is required for target kind the selected item.",
  "error.reason.NOTIFICATION_TARGET_KIND_INVALID":
    "Invalid notification target kind.",
  "error.reason.NOTIFICATION_TARGET_KIND_REQUIRED_WHEN_TARGET_ID_PROVIDED":
    "Notification target kind is required when target id is provided.",
  "error.reason.OAUTH_AUTHLESS_AUTHORIZATION_CODE_NO_LONGER_ALLOWED":
    "Authless authorization code is no longer allowed.",
  "error.reason.OAUTH_AUTHLESS_REFRESH_GRANT_NO_LONGER_ALLOWED":
    "Authless refresh grant is no longer allowed.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_BINDING_MISMATCH":
    "Authorization code binding mismatch.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_EXPIRED_ALREADY_USED":
    "Authorization code is expired or already used.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_INVALID":
    "Authorization code is invalid.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_SESSION_UNAVAILABLE":
    "Authorization code session is no longer valid.",
  "error.reason.OAUTH_AUTHORIZATION_CODE_UNAVAILABLE":
    "Authorization code is no longer valid.",
  "error.reason.OAUTH_CLIENT_DISABLED_UNAVAILABLE":
    "OAuth client is disabled or unavailable.",
  "error.reason.OAUTH_CLIENT_DISPLAY_NAME_REQUIRED":
    "OAuth client display name is required.",
  "error.reason.OAUTH_CLIENT_DISPLAY_NAME_TOO_LONG":
    "OAuth client display name must not exceed the selected value characters.",
  "error.reason.OAUTH_CLIENT_NOT_FOUND": "Oauth client was not found.",
  "error.reason.OAUTH_CLIENT_UNKNOWN": "Unknown OAuth client.",
  "error.reason.OAUTH_CODE_CHALLENGE_INVALID":
    "Code_challenge must be a valid S256 base64url value.",
  "error.reason.OAUTH_CODE_CHALLENGE_METHOD_INVALID":
    "Code_challenge_method must be S256.",
  "error.reason.OAUTH_CODE_VERIFIER_INVALID":
    "Code_verifier must be 43-128 RFC7636 unreserved characters.",
  "error.reason.OAUTH_CUSTOM_OAUTH_REDIRECT_URIS_INVALID":
    "Custom OAuth redirect URIs must be absolute HTTPS URLs without credentials.",
  "error.reason.OAUTH_GRANT_REVOKED_UNAVAILABLE":
    "OAuth grant is revoked or unavailable.",
  "error.reason.OAUTH_INVALID_SCOPE":
    "The requested authorization scope is not supported.",
  "error.reason.OAUTH_JELLYFIN_CONNECTION_AMBIGUOUS":
    "More than one Jellyfin connection matches. Choose a connection.",
  "error.reason.OAUTH_PKCE_VERIFICATION_FAILED": "PKCE verification failed.",
  "error.reason.OAUTH_REDIRECT_URIS_INVALID":
    "OAuth redirect URIs must be unique.",
  "error.reason.OAUTH_REDIRECT_URI_FRAGMENT_FORBIDDEN":
    "Redirect_uri must not contain a fragment.",
  "error.reason.OAUTH_REDIRECT_URI_INVALID": "Invalid OAuth redirect URI.",
  "error.reason.OAUTH_REDIRECT_URI_NOT_ALLOWED_OAUTH_CLIENT":
    "Redirect_uri is not allowed for this OAuth client.",
  "error.reason.OAUTH_REDIRECT_URI_REQUIRED":
    "At least one OAuth redirect URI is required.",
  "error.reason.OAUTH_REFRESH_TOKEN_CLIENT_MISMATCH":
    "Refresh token client mismatch.",
  "error.reason.OAUTH_REFRESH_TOKEN_INVALID": "Refresh token is invalid.",
  "error.reason.OAUTH_REFRESH_TOKEN_REUSED": "Refresh token was reused.",
  "error.reason.OAUTH_REFRESH_TOKEN_REVOKED": "Refresh token is revoked.",
  "error.reason.OAUTH_REFRESH_TOKEN_SESSION_UNAVAILABLE":
    "Refresh token session is no longer valid.",
  "error.reason.OAUTH_TOKEN_MALFORMED": "OAuth token is malformed.",
  "error.reason.OAUTH_USER_NO_LONGER_EXISTS": "OAuth user no longer exists.",
  "error.reason.PLUGIN_ALREADY_INSTALLED": "This plugin is already installed.",
  "error.reason.PLUGIN_ALREADY_VERSION_SELECTED_RELEASE":
    "Plugin the selected item is already at version the selected value.",
  "error.reason.PLUGIN_ARCHIVE_EXTRACTOR_CONFLICT":
    "Multiple enabled archive extraction plugins support this format. Disable one in Settings → Plugins.",
  "error.reason.PLUGIN_ARCHIVE_PATH_OUTSIDE_ROOT":
    "The archive plugin path is outside the allowed root.",
  "error.reason.PLUGIN_ARCHIVE_PATH_UNSAFE":
    "The archive plugin path is not a safe relative path.",
  "error.reason.PLUGIN_ARTIFACT_DIGEST_REQUIRED":
    "The requested value must include at least one digest.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECTS_NOT_ALLOWED":
    "Plugin artifact redirects are not allowed for the selected value.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_DID_NOT_INCLUDE_LOCATION_HEADER":
    "Plugin artifact redirect for the selected value did not include a Location header.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_HEADER":
    "Plugin artifact redirect for the selected value included an invalid Location header.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_INCLUDED_INVALID_LOCATION_URL":
    "Plugin artifact redirect for the selected value included an invalid Location URL.",
  "error.reason.PLUGIN_ARTIFACT_REDIRECT_LIMIT_EXCEEDED":
    "Plugin artifact redirect limit exceeded for the selected value.",
  "error.reason.PLUGIN_ARTIFACT_UNTRUSTED":
    "The plugin artifact could not be trusted.",
  "error.reason.PLUGIN_BLOCKED_VERSION_SCRYER_SO_CANNOT_TESTED":
    "The the selected item plugin is blocked on this version of Scryer, so it cannot be tested. the selected value.",
  "error.reason.PLUGIN_BROTLI_DECOMPRESSION_NOT_COMPILED_INTO_TARGET":
    "Plugin brotli decompression is not compiled into this target.",
  "error.reason.PLUGIN_BUNDLED_PLUGIN":
    "Plugin the selected item is a bundled plugin; uninstall any downloaded override before uploading a local build.",
  "error.reason.PLUGIN_CANNOT_REVERT_BUILT_PLUGIN_NO_BUNDLED_DEFINITION_UNAVAILABLE":
    "Cannot revert built-in plugin the selected item because no bundled definition is available.",
  "error.reason.PLUGIN_CANNOT_UNINSTALL_BUILT_PLUGINS":
    "Cannot uninstall built-in plugins; disable them instead.",
  "error.reason.PLUGIN_CATALOG_ARTIFACTS_MISSING":
    "Plugin catalog redirect did not contain any artifacts.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_CATALOG_VERSION_OUT_OF_RANGE":
    "Plugin catalog redirect catalog_version must be greater than zero.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_DID_NOT_YIELD_USABLE_CATALOG":
    "Plugin catalog redirect did not yield a usable catalog.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_JSON_INVALID":
    "Invalid plugin catalog redirect JSON.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_OUT_OF_RANGE":
    "Plugin catalog redirect must include at least one artifact.",
  "error.reason.PLUGIN_CATALOG_REDIRECT_SCHEMA_UNSUPPORTED":
    "Unsupported plugin catalog redirect schema the selected item.",
  "error.reason.PLUGIN_CATALOG_SCHEMA_UNSUPPORTED":
    "Unsupported plugin catalog schema the selected item.",
  "error.reason.PLUGIN_CATALOG_V3_JSON_INVALID":
    "Invalid plugin catalog v3 JSON.",
  "error.reason.PLUGIN_CATALOG_VERSION_INVALID":
    "Invalid catalog version the selected item.",
  "error.reason.PLUGIN_CATALOG_VERSION_OUT_OF_RANGE":
    "Plugin catalog version must be greater than zero.",
  "error.reason.PLUGIN_CHANGED_WHILE_RUNTIME_BEING_PREPARED":
    "Plugin changed while its runtime was being prepared.",
  "error.reason.PLUGIN_CLIENT_CONFIG_JSON_INVALID":
    "Invalid client config_json.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_APPROVED_TIER_INVALID":
    "Community catalog source the selected item approved tier must be verified_community.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_MUST_PUBLISH_EXACTLY_ONE_PLUGIN":
    "Community catalog source the selected item must publish exactly one plugin.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_PUBLISHED_PLUGIN":
    "Community catalog source the selected item published plugin the selected item.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_SIGNER_REPO_DOES_NOT_MATCH_APPROVED_REPO":
    "Community catalog source the selected item signer repo the selected item does not match approved repo the selected item.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_SOURCE_REPO_DOES_NOT_MATCH_APPROVED_REPO":
    "Community catalog source the selected item source repo the selected item does not match approved repo the selected item.",
  "error.reason.PLUGIN_COMMUNITY_CATALOG_SOURCE_SUPPORT_TIER_DOES_NOT_MATCH_APPROVED_TIER":
    "Community catalog source the selected item support tier does not match approved tier.",
  "error.reason.PLUGIN_COMMUNITY_RULE_PACK_PUBLISH_FORBIDDEN":
    "Community catalog source the selected item must not publish rule packs.",
  "error.reason.PLUGIN_COMMUNITY_SOURCE_ID_PLUGIN_CATALOG_DUPLICATED":
    "Duplicate community source id the selected item in plugin catalog.",
  "error.reason.PLUGIN_COMMUNITY_SOURCE_NESTING_FORBIDDEN":
    "Community catalog source the selected item must not publish nested community sources.",
  "error.reason.PLUGIN_COMMUNITY_SOURCE_SUPPORT_TIER_INVALID":
    "Community source the selected item support tier must be verified_community.",
  "error.reason.PLUGIN_COMPONENT_DESCRIPTOR_DIFFERS_PERSISTED_METADATA":
    "Component descriptor differs from its persisted metadata.",
  "error.reason.PLUGIN_COMPONENT_DESCRIPTOR_MISMATCH":
    "Initialized component descriptor differs from embedded metadata.",
  "error.reason.PLUGIN_COMPONENT_REPLACEMENT_CHANGED_PLUGIN_IDENTITY":
    "Component replacement changed plugin identity.",
  "error.reason.PLUGIN_COMPONENT_UPGRADE_BLOCKED":
    "The plugin upgrade is blocked by installed components.",
  "error.reason.PLUGIN_DECOMPRESSED_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES":
    "The requested value decompressed payload exceeds maximum size of the selected value bytes.",
  "error.reason.PLUGIN_DECOMPRESSED_SIZE_LIMIT_EXCEEDS_PLATFORM_S_MAXIMUM_BUFFER_SIZE":
    "The requested value decompressed size limit the selected value exceeds this platform's maximum buffer size.",
  "error.reason.PLUGIN_DESCRIPTOR_ID_MISMATCH":
    "Downloaded plugin descriptor id the selected item does not match registry id the selected item.",
  "error.reason.PLUGIN_DIGEST_ALGORITHM_REQUIRED":
    "Digest algorithm is missing.",
  "error.reason.PLUGIN_DIGEST_MISMATCH":
    "The requested value digest mismatch: expected blake3.",
  "error.reason.PLUGIN_DIGEST_STRING_INVALID":
    "Invalid digest string the selected item.",
  "error.reason.PLUGIN_DIGEST_VALUE_INVALID":
    "Digest value the selected item must be hexadecimal.",
  "error.reason.PLUGIN_DIGEST_VALUE_REQUIRED": "Digest value is missing.",
  "error.reason.PLUGIN_DOES_NOT_INCLUDE_BLAKE3_DIGEST":
    "The requested value does not include a blake3 digest.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_PLUGIN_TYPE_BUT_REGISTRY_EXPECTS":
    "Downloaded plugin the selected item has plugin_type the selected item but registry expects the selected item.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_PROVIDER_TYPE_BUT_REGISTRY_EXPECTS":
    "Downloaded plugin the selected item has provider_type the selected item but registry expects the selected item.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_SDK_VERSION_BUT_SELECTED_RELEASE":
    "Downloaded plugin the selected item has sdk_version the selected item but the selected release is the selected item.",
  "error.reason.PLUGIN_DOWNLOADED_PLUGIN_VERSION_BUT_SELECTED_RELEASE":
    "Downloaded plugin the selected item has version the selected item but the selected release is the selected item.",
  "error.reason.PLUGIN_FIELD_REQUIRED": "The requested value is required.",
  "error.reason.PLUGIN_GITHUB_OWNER_REQUIRED": "GitHub owner is missing.",
  "error.reason.PLUGIN_GITHUB_REPOSITORY_INVALID":
    "GitHub repository must include owner and repo.",
  "error.reason.PLUGIN_GITHUB_REPOSITORY_URL_INVALID":
    "Invalid GitHub repository URL.",
  "error.reason.PLUGIN_GITHUB_REPO_NAME_REQUIRED":
    "GitHub repo name is missing.",
  "error.reason.PLUGIN_ID_PLUGIN_CATALOG_DUPLICATED":
    "Duplicate plugin id the selected item in plugin catalog.",
  "error.reason.PLUGIN_INDEXER_HOST_FORBIDDEN":
    "The indexer host is not allowed for this instance.",
  "error.reason.PLUGIN_INSTALLED_PACK_VERSION_INVALID":
    "Invalid installed pack version.",
  "error.reason.PLUGIN_INSTALLED_PLUGIN_SETTINGS_DESCRIPTOR_UNAVAILABLE":
    "Installed plugin settings descriptor is unavailable.",
  "error.reason.PLUGIN_INSTALLED_VERSION_INVALID":
    "Invalid installed version the selected item.",
  "error.reason.PLUGIN_INVALID_PERSISTED_DESCRIPTOR_JSON":
    "Plugin the selected item has invalid persisted descriptor_json.",
  "error.reason.PLUGIN_LIST_LINKED_ACCOUNT_REQUIRED":
    "List provider reads only a member's own lists and needs that member's linked account.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_CATALOG_NOT_UTF_8":
    "Manual plugin catalog is not UTF-8.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_CATALOG_SOURCE_MISSING_GITHUB_REPO":
    "Manual plugin catalog source the selected item is missing github repo.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_MISSING_SOURCE_REPO":
    "Manual plugin the selected item is missing source repo.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPOSITORIES_INVALID":
    "Manual plugin repositories must be hosted on github.com.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPO_MUST_PUBLISH_EXACTLY_ONE_PLUGIN_ENTRY":
    "Manual plugin repo the selected item must publish exactly one plugin entry.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPO_NO_SDK_COMPATIBLE_RELEASE":
    "Manual plugin repo the selected item has no SDK-compatible release.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_REPO_PUBLISHED_PLUGIN_SOURCE_REPO":
    "Manual plugin repo the selected item published plugin the selected item from source repo the selected item.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_FILE_NAME_REQUIRED":
    "Manual plugin upload file name is required.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_INVALID":
    "Manual plugin upload must be a .wasm or .wasm.zst file.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_PAYLOAD_INVALID":
    "Manual plugin upload payload is not valid base64.",
  "error.reason.PLUGIN_MANUAL_PLUGIN_UPLOAD_REQUIRES_EXPLICIT_RISK_ACKNOWLEDGEMENT":
    "Manual plugin upload requires explicit risk acknowledgement.",
  "error.reason.PLUGIN_MISSING_PERSISTED_DESCRIPTOR_JSON":
    "Plugin the selected item is missing persisted descriptor_json.",
  "error.reason.PLUGIN_MISSING_PERSISTED_WASM_DIGEST_ALGORITHM":
    "Plugin the selected item is missing persisted wasm digest algorithm.",
  "error.reason.PLUGIN_MISSING_PERSISTED_WASM_DIGEST_VALUE":
    "Plugin the selected item is missing persisted wasm digest value.",
  "error.reason.PLUGIN_MISSING_PERSISTED_WASM_PAYLOAD":
    "Plugin the selected item is missing persisted WASM payload.",
  "error.reason.PLUGIN_MISSING_USABLE_BLAKE3_DIGEST":
    "The requested value is missing a usable blake3 digest.",
  "error.reason.PLUGIN_MUST_USE_SUPPORTED_ALGORITHM_HEX_DIGEST":
    "The requested value must use a supported <algorithm>:<hex> digest.",
  "error.reason.PLUGIN_NETWORK_ENDPOINT_RESOLUTION_FAILED":
    "The indexer's configured endpoints could not be resolved.",
  "error.reason.PLUGIN_NEWZNAB_PROFILE_INVALID":
    "The selected Newznab provider profile is invalid.",
  "error.reason.PLUGIN_NOT_AVAILABLE_PLUGIN_CATALOG_NOT_FOUND":
    "Plugin  is not available from the plugin catalog was not found.",
  "error.reason.PLUGIN_NO_LONGER_HOST_COMPATIBLE_RELEASE_SCRYER_VERSION":
    "Plugin the selected item no longer has a host-compatible release for this Scryer version.",
  "error.reason.PLUGIN_NO_LONGER_INSTALLED_NOT_FOUND":
    "Plugin is no longer installed was not found.",
  "error.reason.PLUGIN_OFFICIAL_NZBGEEK_PLUGIN_NOT_AVAILABLE_BUILTIN_MIGRATION_NOT_FOUND":
    "Official nzbgeek plugin is not available for builtin migration.",
  "error.reason.PLUGIN_PAYLOAD_EXCEEDS_MAXIMUM_SIZE_BYTES":
    "The requested value payload exceeds maximum size of the selected value bytes.",
  "error.reason.PLUGIN_PAYLOAD_TOO_LARGE_VALIDATE_DECOMPRESSED_SIZE":
    "The requested value payload is too large to validate decompressed size.",
  "error.reason.PLUGIN_PROVIDER_TYPE_RESERVED_FIRST_PARTY_CODE":
    "Provider type the selected item is reserved for first-party code.",
  "error.reason.PLUGIN_PROXY_CONFIGURATION_NOT_FOUND":
    "Proxy configuration was not found.",
  "error.reason.PLUGIN_PROXY_DISABLED_DOWNLOAD_CLIENT":
    "Proxy is disabled for this download client.",
  "error.reason.PLUGIN_RELEASE_ARTIFACT_BYTES_OUT_OF_RANGE":
    "Plugin the selected item release the selected item artifact the selected item bytes must be greater than zero.",
  "error.reason.PLUGIN_RELEASE_ARTIFACT_INVALID":
    "Plugin the selected item release the selected item artifact the selected item must include wasm_digests.",
  "error.reason.PLUGIN_RELEASE_DUPLICATE_ARTIFACT_VARIANT_ENCODING_ROWS":
    "Plugin the selected item release the selected item has duplicate artifact variant/encoding rows.",
  "error.reason.PLUGIN_RELEASE_INVALID_MAX_SCRYER_VERSION":
    "Plugin the selected item release the selected item has invalid max_scryer_version.",
  "error.reason.PLUGIN_RELEASE_INVALID_MIN_SCRYER_VERSION":
    "Plugin the selected item release the selected item has invalid min_scryer_version.",
  "error.reason.PLUGIN_RELEASE_MIN_SCRYER_VERSION_GREATER_MAX_SCRYER_VERSION":
    "Plugin the selected item release the selected item has min_scryer_version greater than max_scryer_version.",
  "error.reason.PLUGIN_RELEASE_OUT_OF_RANGE":
    "The selected item release the selected item must include at least one artifact.",
  "error.reason.PLUGIN_RELEASE_VERSION_INVALID":
    "Invalid plugin release version the selected item.",
  "error.reason.PLUGIN_RELEASE_VERSION_PLUGIN_DUPLICATED":
    "Duplicate release version the selected item for plugin the selected item.",
  "error.reason.PLUGIN_REQUESTS_HOST_PROCESS_CAPABILITY_WHICH_RESERVED_OFFICIAL_PLUGINS":
    "Plugin the selected item requests the host-process capability, which is reserved for official plugins.",
  "error.reason.PLUGIN_REQUIRED_PLUGIN_SETTING_REQUIRED":
    "Required plugin setting is missing.",
  "error.reason.PLUGIN_RESOURCE_NOT_FOUND":
    "Plugin  not installed was not found.",
  "error.reason.PLUGIN_RULE_PACK_CATALOG_CHANGED_DURING_DOWNLOAD":
    "Rule pack catalog changed during download.",
  "error.reason.PLUGIN_RULE_PACK_CONTAINS_DUPLICATE_RULE_ID":
    "Rule pack the selected item contains duplicate rule id the selected item.",
  "error.reason.PLUGIN_RULE_PACK_CONTAINS_NO_RULES":
    "Rule pack the selected item contains no rules.",
  "error.reason.PLUGIN_RULE_PACK_CONTAINS_RULE_REQUIRED_FIELDS_MISSING":
    "Rule pack the selected item contains a rule with required fields missing.",
  "error.reason.PLUGIN_RULE_PACK_DOES_NOT_MATCH_CATALOG_ID_VERSION":
    "Rule pack the selected item does not match the catalog id/version.",
  "error.reason.PLUGIN_RULE_PACK_EMPTY":
    "Rule pack the selected item has an empty the selected value.",
  "error.reason.PLUGIN_RULE_PACK_ID_PLUGIN_CATALOG_DUPLICATED":
    "Duplicate rule pack id the selected item in plugin catalog.",
  "error.reason.PLUGIN_RULE_PACK_INVALID_MIN_SCRYER_VERSION":
    "Rule pack the selected item has invalid min_scryer_version the selected item.",
  "error.reason.PLUGIN_RULE_PACK_NOT_FOUND": "Rule pack was not found.",
  "error.reason.PLUGIN_RULE_PACK_NO_COMPATIBLE_ARTIFACT":
    "Rule pack the selected item has no compatible artifact.",
  "error.reason.PLUGIN_RULE_PACK_RELEASE_INVALID":
    "Rule pack the selected item release the selected item must include rule_pack_digests.",
  "error.reason.PLUGIN_RULE_PACK_RELEASE_VERSION_DUPLICATED":
    "Duplicate rule pack release version the selected item for the selected item.",
  "error.reason.PLUGIN_RULE_PACK_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING":
    "Rule pack the selected item selected artifact the selected item has unsupported encoding.",
  "error.reason.PLUGIN_RULE_PACK_USES_UNSUPPORTED_SCHEMA_VERSION":
    "Rule pack the selected item uses unsupported schema version the selected value.",
  "error.reason.PLUGIN_SDK_CONSTRAINT_PLUGIN_INVALID":
    "Invalid sdk_constraint the selected item for plugin the selected item.",
  "error.reason.PLUGIN_SELECTED_ARTIFACT_BYTES_MISMATCH":
    "Plugin the selected item selected artifact the selected item bytes mismatch: expected the selected value, got the selected value.",
  "error.reason.PLUGIN_SELECTED_ARTIFACT_TOO_LARGE_VALIDATE_BYTES":
    "Plugin the selected item selected artifact the selected item is too large to validate bytes.",
  "error.reason.PLUGIN_SELECTED_ARTIFACT_UNSUPPORTED_ENCODING":
    "Plugin the selected item selected artifact the selected item has unsupported encoding.",
  "error.reason.PLUGIN_SETTING_UNKNOWN": "Unknown plugin setting.",
  "error.reason.PLUGIN_SETTING_VALUE_INVALID": "Invalid plugin setting value.",
  "error.reason.PLUGIN_SIGNATURE_VERIFICATION_NOT_COMPILED_INTO_TARGET":
    "Plugin signature verification is not compiled into this target.",
  "error.reason.PLUGIN_SIGNER_REPO_DOES_NOT_MATCH_SOURCE_REPO":
    "Plugin the selected item signer repo the selected item does not match source repo the selected item.",
  "error.reason.PLUGIN_SUBTITLE_CONFIG_NOT_OBJECT":
    "Subtitle provider config_json must be a JSON object.",
  "error.reason.PLUGIN_SUBTITLE_HOST_BINDING_REQUIRED":
    "The subtitle provider is missing a required host binding.",
  "error.reason.PLUGIN_SUBTITLE_INPUT_FILENAME_REQUIRED":
    "The subtitle input path must name a file.",
  "error.reason.PLUGIN_SUBTITLE_INPUT_TOO_LARGE":
    "The subtitle input exceeds the size limit.",
  "error.reason.PLUGIN_SUBTITLE_INPUT_TOO_LONG":
    "The subtitle input exceeds the duration limit.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_FILENAME_REQUIRED":
    "The subtitle sync path must name a file.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_NOT_FILE":
    "Subtitle sync plugin output is not a regular file.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_OUTSIDE_DIRECTORY":
    "Subtitle sync plugin output escapes the output directory.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_PATH_MISMATCH":
    "The subtitle sync plugin returned an unexpected output path.",
  "error.reason.PLUGIN_SUBTITLE_SYNC_OUTPUT_TOO_LARGE":
    "The subtitle sync output exceeds the size limit.",
  "error.reason.PLUGIN_TEST_CONNECTION_UNSUPPORTED":
    "Test connection is not supported for client type the selected item.",
  "error.reason.PLUGIN_TYPE_BUILT_PREPARATION_UNSUPPORTED":
    "Unsupported plugin_type the selected item for built-in preparation.",
  "error.reason.PLUGIN_TYPE_BUILT_RESTORE_UNSUPPORTED":
    "Unsupported plugin_type the selected item for built-in restore.",
  "error.reason.PLUGIN_TYPE_RUNTIME_REMOVAL_UNSUPPORTED":
    "Unsupported plugin_type the selected item for runtime removal.",
  "error.reason.PLUGIN_TYPE_RUNTIME_UPSERT_UNSUPPORTED":
    "Unsupported plugin_type the selected item for runtime upsert.",
  "error.reason.PLUGIN_URL_INVALID":
    "The requested value URL the selected item must use http or https, got the selected item.",
  "error.reason.PLUGIN_USES_UNSUPPORTED_DIGEST_ALGORITHM":
    "The requested value uses unsupported digest algorithm the selected item.",
  "error.reason.PLUGIN_WASM_ARTIFACT_TOO_LARGE":
    "Plugin the selected item WASM artifact declared size the selected value exceeds maximum size of the selected value bytes.",
  "error.reason.PLUGIN_ZSTD_COMPRESSION_NOT_COMPILED_INTO_TARGET":
    "Plugin zstd compression is not compiled into this target.",
  "error.reason.PLUGIN_ZSTD_DECOMPRESSION_NOT_COMPILED_INTO_TARGET":
    "Plugin zstd decompression is not compiled into this target.",
  "error.reason.PORTS_ARTIFACT_DOWNLOAD_UNSUPPORTED":
    "Artifact download is not supported by this client.",
  "error.reason.PORTS_CACHED_JPEG_ENCODING_UNAVAILABLE":
    "Cached JPEG encoding is unavailable.",
  "error.reason.PORTS_CATEGORY_OUT_OF_RANGE":
    "Category must be at most 255 bytes without control characters.",
  "error.reason.PORTS_CLIENT_ROUTING_DISCOVERY_UNSUPPORTED":
    "Client routing discovery is unsupported.",
  "error.reason.PORTS_DISC_TITLE_SELECTION_UNSUPPORTED_ANALYZER":
    "Disc title selection is unsupported by this analyzer.",
  "error.reason.PORTS_INDEXER_CONFIGURATION_NOT_FOUND":
    "Indexer configuration was not found.",
  "error.reason.PORTS_INDEXER_SETTINGS_CHANGED_DURING_VALIDATION":
    "Indexer settings changed during validation; reload and try again.",
  "error.reason.PORTS_MULTIPLE_TITLES_FOUND_SLUG_LOOKUP":
    "Multiple titles found for slug lookup.",
  "error.reason.PORTS_SELECT_DOWNLOAD_CLIENT": "Select a download client.",
  "error.reason.PORTS_STRUCTURAL_DIAGNOSTICS_UNSUPPORTED_ANALYZER":
    "Structural diagnostics are unsupported by this analyzer.",
  "error.reason.QUALITY_FILE_NOT_FOUND": "Selected file was not found.",
  "error.reason.QUALITY_FILE_SCORE_EVALUATION_FAILED":
    "File score evaluation failed; previous evaluation retained.",
  "error.reason.QUALITY_FILE_SCORING_INPUTS_CHANGED":
    "File scoring inputs changed; refresh again.",
  "error.reason.QUALITY_MEDIA_FILE_NOT_FOUND": "Media file was not found.",
  "error.reason.QUALITY_MEDIA_TITLE_NOT_FOUND": "Media title was not found.",
  "error.reason.QUALITY_SCORE_REFRESH_BELONGS_ANOTHER_USER":
    "Score refresh belongs to another user.",
  "error.reason.QUALITY_SCORE_REFRESH_QUEUE_FULL":
    "Score refresh queue is full.",
  "error.reason.QUALITY_SCORE_REFRESH_USER_NO_LONGER_EXISTS":
    "Score refresh user no longer exists.",
  "error.reason.QUALITY_SEASON_NOT_FOUND": "Selected season was not found.",
  "error.reason.QUALITY_SELECT_BETWEEN_1_1000_FILES_SEASONS_TITLES":
    "Select between 1 and 1000 files, seasons, or titles.",
  "error.reason.QUALITY_TITLE_NOT_FOUND": "Selected title was not found.",
  "error.reason.RELEASE_TOKEN_EXPIRED":
    "The search results expired. Search again.",
  "error.reason.RELEASE_TOKEN_INVALID":
    "The release selection could not be verified. Search again.",
  "error.reason.REQUEST_RULE_FAILED_BUILD_REQUEST_RULES_ENGINE":
    "Failed to build the request rules engine.",
  "error.reason.REQUEST_RULE_FIELD_REQUIRED":
    "The requested value must not be empty.",
  "error.reason.REQUEST_RULE_LEASE_OUT_OF_RANGE":
    "A lease must be at most the selected value days (ten years), or forever.",
  "error.reason.REQUEST_RULE_LIBRARY_FACET_DOES_NOT_MATCH_REQUESTED_MEDIA_FACET":
    "Library facet does not match requested media facet.",
  "error.reason.REQUEST_RULE_LIBRARY_NOT_FOUND": "Library was not found.",
  "error.reason.REQUEST_RULE_MAY_NOT_READ_REQUEST_S_DECISION":
    "You may not read this request's decision.",
  "error.reason.REQUEST_RULE_MEDIA_REQUESTS_INVALID":
    "Media requests must include SMG external identifiers.",
  "error.reason.REQUEST_RULE_MEDIA_REQUEST_ID_REQUIRED":
    "Media request id is required.",
  "error.reason.REQUEST_RULE_MEDIA_REQUEST_NOT_FOUND":
    "Media request was not found.",
  "error.reason.REQUEST_RULE_PERSON_TARGETING_PERMISSION_REQUIRED":
    "This rule reads facts about specific people, so saving it needs permission to manage permissions. Ask an administrator to save it, or rewrite the rule without those facts.",
  "error.reason.REQUEST_RULE_REQUEST_MAY_CARRY_MOST_TAGS_SUPPLIED":
    "A request may carry at most the selected value tags, the selected value were supplied.",
  "error.reason.REQUEST_RULE_RULE_EVALUATION_FAILED": "Rule evaluation failed.",
  "error.reason.REQUEST_RULE_RULE_FAILED_COMPILE": "Rule failed to compile.",
  "error.reason.REQUEST_RULE_RULE_VALIDATION_FAILED": "Rule validation failed.",
  "error.reason.REQUEST_RULE_SAMPLE_LIBRARY_NOT_FOUND":
    "Sample library was not found.",
  "error.reason.REQUEST_RULE_SAMPLE_REQUESTER_NOT_FOUND":
    "Sample requester was not found.",
  "error.reason.REQUEST_RULE_SET_NOT_FOUND": "Request rule set was not found.",
  "error.reason.REQUEST_RULE_TAG_CHARACTERS_MOST_ALLOWED":
    "Tag is the selected value characters, at most the selected value are allowed.",
  "error.reason.REQUEST_RULE_TAG_CONTAINS_UNSUPPORTED_CHARACTER":
    "Tag the selected item contains unsupported character the selected item; use letters, digits, '.', '_', '-', or spaces.",
  "error.reason.REQUEST_RULE_TAG_REQUIRED": "Tag must not be empty.",
  "error.reason.REQUEST_RULE_TAG_USES_RESERVED_PREFIX":
    "Tag the selected item uses the reserved the selected item prefix.",
  "error.reason.RULE_AUTOMATIC_RULE_PACK_UPDATES_REQUIRE_STABLE_PATCH_INSTALLED_MAJOR_MINOR":
    "Automatic rule pack updates require a stable patch on the installed major/minor.",
  "error.reason.RULE_BUNDLED_RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE":
    "Bundled rule pack installation conflicted with another change.",
  "error.reason.RULE_BUNDLED_TRASH_PACK_CONTAINS_NO_TEMPLATES":
    "Bundled TRaSH pack contains no templates.",
  "error.reason.RULE_BUNDLED_TRASH_PACK_INVALID": "Invalid bundled TRaSH pack.",
  "error.reason.RULE_BUNDLED_TRASH_PACK_UNSUPPORTED_IDENTITY_SCHEMA":
    "Bundled TRaSH pack has an unsupported identity or schema.",
  "error.reason.RULE_CANNOT_ADOPT_DUPLICATE_LEGACY_LOCALE_RULES":
    "Cannot adopt duplicate legacy locale rules for the selected value.",
  "error.reason.RULE_COPYING_TRACKED_RULE_REQUIRES_DISABLING_SOURCE":
    "Copying a tracked rule requires disabling its source.",
  "error.reason.RULE_COPY_DISABLES_SOURCE_REQUIRES_COPY_SOURCE_RULE_SET":
    "Copy_disables_source requires a copy source rule set.",
  "error.reason.RULE_ENABLED_TEMPLATES_INVALID":
    "Enabled templates must be unique members of the selected pack.",
  "error.reason.RULE_EPISODE_CANNOT_SELECTED_WHEN_TESTING_MOVIE_RULE":
    "An episode cannot be selected when testing a movie rule.",
  "error.reason.RULE_EPISODE_DOES_NOT_BELONG_TITLE":
    "Episode the selected value does not belong to title the selected value.",
  "error.reason.RULE_EPISODE_NOT_FOUND": "Episode was not found.",
  "error.reason.RULE_EPISODE_REQUIRED_WHEN_TESTING_SERIES_ANIME_RULE":
    "An episode is required when testing a series or anime rule.",
  "error.reason.RULE_FAILED_BUILD_RULES_ENGINE":
    "Failed to build rules engine.",
  "error.reason.RULE_INDEXER_LANGUAGES_TOO_LARGE_SCORING_PREVIEW":
    "Indexer languages are too large for a scoring preview.",
  "error.reason.RULE_INPUT_RETIRED": "The rule uses a retired input field.",
  "error.reason.RULE_MANAGED_CONVENIENCE_SETTING":
    "This rule is managed by a convenience setting. Change the setting instead of editing the rule directly.",
  "error.reason.RULE_MANAGED_LOCALE_FILTER_FORBIDDEN":
    "Managed TRaSH Guides locale packs use their predefined locale policy and cannot be filtered.",
  "error.reason.RULE_MANAGED_TRACKED_RULES_INVALID":
    "Managed or tracked rules must be copied before editing.",
  "error.reason.RULE_MEDIA_FILE_DOES_NOT_COVER_EPISODE":
    "Media file the selected value does not cover episode the selected value.",
  "error.reason.RULE_MEDIA_FILE_NOT_FOUND":
    "Media file  of title was not found.",
  "error.reason.RULE_ONLY_ONE_ENABLED_RULE_MAY_USE_EXCLUSIVE_GROUP":
    "Only one enabled rule may use exclusive group the selected value.",
  "error.reason.RULE_PACK_ALREADY_INSTALLED":
    "Rule pack the selected value is already installed.",
  "error.reason.RULE_PACK_FACET_INVALID":
    "Invalid rule pack facet the selected value.",
  "error.reason.RULE_PACK_INSTALLATION_CONFLICTED_ANOTHER_CHANGE":
    "Rule pack installation conflicted with another change.",
  "error.reason.RULE_PACK_PRIORITY_TEMPLATE_ID_DUPLICATED":
    "Duplicate rule pack priority template ID.",
  "error.reason.RULE_PACK_SETTINGS_REFERENCE_UNKNOWN_REMOVED_TEMPLATE":
    "Rule pack settings reference an unknown or removed template.",
  "error.reason.RULE_PACK_TEMPLATE_ID_DUPLICATED":
    "Duplicate rule pack template ID.",
  "error.reason.RULE_PACK_UPDATE_INVALID":
    "Rule pack update must be a newer version.",
  "error.reason.RULE_PACK_UP_TO_DATE": "The rule pack is already up to date.",
  "error.reason.RULE_PACK_VERSION_DIGEST_DID_NOT_MATCH_VERIFIED_CATALOG_RELEASE":
    "Rule pack version or digest did not match the verified catalog release.",
  "error.reason.RULE_PREVIEW_CANNOT_EDIT_COPY_RULE_SAME_TIME":
    "A preview cannot edit and copy a rule at the same time.",
  "error.reason.RULE_PREVIEW_REQUIRES_EITHER_DRAFT_SAVED_RULE_SET":
    "A preview requires either a draft or a saved rule set.",
  "error.reason.RULE_PUBLISHED_INVALID":
    "Published at must be an RFC 2822 or RFC 3339 timestamp, got the selected value.",
  "error.reason.RULE_RELEASE_NAME_TOO_LARGE_SCORING_PREVIEW":
    "Release name is too large for a scoring preview.",
  "error.reason.RULE_RULES_PACK_CANNOT_COPIED":
    "Rules from this pack cannot be copied.",
  "error.reason.RULE_SET_FIELD_REQUIRED":
    "At least one rule set field must be provided.",
  "error.reason.RULE_SET_NOT_FOUND": "Rule set was not found.",
  "error.reason.RULE_SET_NOT_TRACKED_RULE_PACK":
    "Rule set is not tracked by a rule pack.",
  "error.reason.RULE_SIZE_BYTES_CANNOT_NEGATIVE":
    "Size bytes cannot be negative.",
  "error.reason.RULE_SOURCE_INVALID": "The rule source is invalid.",
  "error.reason.RULE_SOURCE_USES_RETIRED_FIELDS":
    "Rule source uses retired fields.",
  "error.reason.RULE_STORED_MEDIA_FILE_SCORED_OWN_SIZE_LISTING_FACTS":
    "A stored media file is scored from its own size and listing facts; size and listing inputs apply only to a release name.",
  "error.reason.RULE_TAG_FILTER_ONLY_APPLIES_MANAGED_RULE_SETS":
    "A tag filter only applies to managed rule sets.",
  "error.reason.RULE_TARGETS_PERSON": "Request rules cannot target a person.",
  "error.reason.RULE_TESTING_SAVED_RULE_CANNOT_INCLUDE_DRAFT_EDIT_COPY_CONTEXT":
    "Testing a saved rule cannot include a draft, edit, or copy context.",
  "error.reason.RULE_TEST_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_NOT_BOTH":
    "Test either a release name or a stored media file, not both.",
  "error.reason.RULE_TITLE_EITHER_RELEASE_NAME_STORED_MEDIA_FILE_REQUIRED":
    "Title and either a release name or a stored media file are required.",
  "error.reason.RULE_TITLE_REQUIRED": "A title is required.",
  "error.reason.RULE_TRACKED_COMMUNITY_PACK":
    "This rule is tracked by a community pack. Change its settings through the tracked pack so membership revisions remain atomic, or copy it to customize its content.",
  "error.reason.RULE_TRACKED_RULE_PACK_AUTOMATIC_UPDATES_DISABLED":
    "Tracked rule pack automatic updates are disabled.",
  "error.reason.RULE_TRACKED_RULE_PACK_CHANGED":
    "Tracked rule pack changed; reload and retry.",
  "error.reason.RULE_TRACKED_RULE_PACK_NOT_FOUND":
    "Tracked rule pack was not found.",
  "error.reason.RULE_TRACKED_RULE_REQUIRED":
    "Tracked rule the selected value is missing.",
  "error.reason.RULE_VALIDATION_ERROR": "Rule validation error.",
  "error.reason.RULE_VALIDATION_FAILED": "Rule validation failed.",
  "error.reason.RULE_VERSION_INVALID":
    "The requested value version is not valid semantic version.",
  "error.reason.RULE_VOTES_CANNOT_NEGATIVE": "Votes cannot be negative.",
  "error.reason.SCRIPTS_SCHEDULE_INVALID": "The script schedule is invalid.",
  "error.reason.SECURITY_ACCOUNT_NO_PASSWORD_SET":
    "Account has no password set.",
  "error.reason.SECURITY_ACTOR_CAPABILITY_CLAIM_UNKNOWN":
    "Unknown actor capability claim.",
  "error.reason.SECURITY_ADMIN_DISABLED_SCRYER_DISABLE_DEFAULT_ADMIN":
    "Admin is disabled by SCRYER_DISABLE_DEFAULT_ADMIN; remove that setting and restart before using the default administrator.",
  "error.reason.SECURITY_ADMIN_PASSWORD_REQUIRED":
    "Admin password is required.",
  "error.reason.SECURITY_ADMIN_RESET_PASSWORD_REQUIRED":
    "SCRYER_ADMIN_PASSWORD_RESET requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE.",
  "error.reason.SECURITY_ADMIN_USERNAME_REQUIRED":
    "Admin username is required.",
  "error.reason.SECURITY_ALTERNATIVE_LOCAL_ADMIN_REQUIRED":
    "SCRYER_DISABLE_DEFAULT_ADMIN requires another enabled local full administrator with a usable password.",
  "error.reason.SECURITY_ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION":
    "Anonymous is reserved for authless audit attribution.",
  "error.reason.SECURITY_AUTHENTICATION_SESSION_INVALIDATED":
    "Authentication session was invalidated.",
  "error.reason.SECURITY_BACKUP_BUNDLE_ENCRYPTED_REQUIRES_PASSWORD":
    "This backup bundle is encrypted and requires a password.",
  "error.reason.SECURITY_BACKUP_BUNDLE_INVALID":
    "The backup bundle is invalid.",
  "error.reason.SECURITY_BACKUP_BUNDLE_NOT_FOUND":
    "Backup bundle was not found.",
  "error.reason.SECURITY_BACKUP_BUNDLE_STILL_BEING_CREATED":
    "Backup bundle is still being created.",
  "error.reason.SECURITY_BACKUP_BUNDLE_TABLE_SET_DOES_NOT_MATCH_CURRENT_RESTORE_CATALOG":
    "Backup bundle table set does not match the current restore catalog: missing [the selected value], unexpected [the selected value]. the selected value.",
  "error.reason.SECURITY_BACKUP_CHECKSUM_MANIFEST_INCOMPLETE":
    "Backup checksum manifest is incomplete: missing [the selected value], unexpected [the selected value].",
  "error.reason.SECURITY_BACKUP_CHECKSUM_MISMATCH":
    "Backup checksum mismatch for the selected value.",
  "error.reason.SECURITY_BACKUP_CREATION_FAILED":
    "The backup could not be created.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_SUBJECT_UNKNOWN":
    "Unknown backup download subject.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_FILENAME_DOES_NOT_MATCH_REQUEST":
    "Backup download token filename does not match request.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_INVALID":
    "Invalid backup download token.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_KIND_INVALID":
    "Invalid backup download token kind.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR":
    "Backup download token subject does not match actor.",
  "error.reason.SECURITY_BACKUP_DOWNLOAD_TOKEN_SUBJECT_EMPTY":
    "Backup download token subject is empty.",
  "error.reason.SECURITY_BACKUP_ENCRYPTION_METADATA_INVALID":
    "Backup encryption metadata is invalid.",
  "error.reason.SECURITY_BACKUP_EXPORT_REQUIRES_CONFIGURED_ENCRYPTION_MASTER_KEY":
    "Backup export requires a configured encryption master key.",
  "error.reason.SECURITY_BACKUP_EXPORT_REQUIRES_NON_EMPTY_PASSWORD":
    "Backup export requires a non-empty password.",
  "error.reason.SECURITY_BACKUP_FILENAME_INVALID": "Invalid backup filename.",
  "error.reason.SECURITY_BACKUP_FORMAT_VERSION_UNSUPPORTED":
    "Unsupported backup format version the selected value.",
  "error.reason.SECURITY_BACKUP_MANIFEST_INVALID":
    "Backup manifest is invalid.",
  "error.reason.SECURITY_BACKUP_MANIFEST_MISSING": "Backup manifest missing.",
  "error.reason.SECURITY_BACKUP_METADATA_NOT_FOUND":
    "Backup metadata was not found.",
  "error.reason.SECURITY_BACKUP_PART_PATH_ESCAPES_BUNDLE_ROOT":
    "Backup part path escapes the bundle root.",
  "error.reason.SECURITY_BACKUP_PART_PATH_INVALID":
    "Backup part path must be a regular file.",
  "error.reason.SECURITY_BACKUP_PART_PATH_UNREADABLE":
    "Backup part path is unreadable.",
  "error.reason.SECURITY_BACKUP_PASSWORD_REQUIRED_FULL_BACKUPS":
    "Backup password is required for full backups.",
  "error.reason.SECURITY_BACKUP_SECRETS_PAYLOAD_INVALID":
    "Backup secrets payload is invalid.",
  "error.reason.SECURITY_BACKUP_SECRETS_PAYLOAD_MISSING":
    "Backup secrets payload missing.",
  "error.reason.SECURITY_BOOTSTRAP_ADMINISTRATOR_NO_USABLE_CREDENTIALS":
    "Bootstrap administrator has no usable credentials.",
  "error.reason.SECURITY_BOOTSTRAP_ADMIN_PASSWORD_REQUIRED":
    "New bootstrap administrator requires SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE.",
  "error.reason.SECURITY_CANNOT_CHANGE_LOGIN_STATUS_CURRENT_USER":
    "Cannot change login status for the current user.",
  "error.reason.SECURITY_CANNOT_CREATE_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE":
    "Cannot create admin while SCRYER_DISABLE_DEFAULT_ADMIN is true.",
  "error.reason.SECURITY_CANNOT_DELETE_CURRENT_USER":
    "Cannot delete current user.",
  "error.reason.SECURITY_CANNOT_DELETE_DEFAULT_ADMIN":
    "Cannot delete the default admin; disable its login instead.",
  "error.reason.SECURITY_CANNOT_DELETE_LAST_FULL_ADMINISTRATOR":
    "Cannot delete the last full administrator.",
  "error.reason.SECURITY_CANNOT_DEMOTE_LAST_FULL_ADMINISTRATOR":
    "Cannot demote the last full administrator.",
  "error.reason.SECURITY_CANNOT_DISABLE_ADMIN_WITHOUT_ANOTHER_USABLE_ADMINISTRATOR":
    "Cannot disable admin without another usable administrator.",
  "error.reason.SECURITY_CANNOT_DISABLE_LAST_USABLE_FULL_ADMINISTRATOR":
    "Cannot disable the last usable full administrator.",
  "error.reason.SECURITY_CANNOT_PROVISION_SELECT_ADMIN_WHILE_SCRYER_DISABLE_DEFAULT_ADMIN_TRUE":
    "Cannot provision or select admin while SCRYER_DISABLE_DEFAULT_ADMIN is true.",
  "error.reason.SECURITY_CANNOT_REMOVE_OWN_USER_MANAGEMENT_PERMISSION_MANAGEMENT_ACCESS":
    "Cannot remove your own user-management or permission-management access.",
  "error.reason.SECURITY_CANNOT_RESET_OWN_AUTHENTICATION_FACTORS":
    "Cannot reset your own authentication factors.",
  "error.reason.SECURITY_CANNOT_RESOLVE_BACKUP_DOWNLOAD_SIGNING_KEY_ACTOR":
    "Cannot resolve backup download signing key for actor the selected value.",
  "error.reason.SECURITY_CANNOT_RESOLVE_RELEASE_CANDIDATE_SIGNING_KEY_ACTOR":
    "Cannot resolve release candidate signing key for actor the selected value.",
  "error.reason.SECURITY_CONNECTION_DISABLED":
    "The requested value connection is disabled.",
  "error.reason.SECURITY_CONNECTION_NOT_CONFIGURED":
    "The requested value connection is not configured for external auth.",
  "error.reason.SECURITY_COULD_NOT_ALLOCATE_SCRYER_USERNAME_EXTERNAL_ACCOUNT":
    "Could not allocate a Scryer username for the external account.",
  "error.reason.SECURITY_CREDENTIALS_INVALID": "Invalid credentials.",
  "error.reason.SECURITY_CREDENTIALS_UNAVAILABLE": "Credentials unavailable.",
  "error.reason.SECURITY_CURRENT_PASSWORD_INCORRECT":
    "Current password is incorrect.",
  "error.reason.SECURITY_CURRENT_PASSWORD_INVALID": "Invalid current password.",
  "error.reason.SECURITY_CURRENT_PASSWORD_REQUIRED":
    "Current password is required.",
  "error.reason.SECURITY_CURRENT_PASSWORD_REQUIRED_REAUTHENTICATION":
    "Current password is required for reauthentication.",
  "error.reason.SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION":
    "Discoverable passkey credential changed during authentication.",
  "error.reason.SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_RESOLVED_USER":
    "Discoverable passkey credential does not belong to the resolved user.",
  "error.reason.SECURITY_DISCOVERABLE_PASSKEY_CREDENTIAL_NOT_FOUND":
    "Discoverable passkey credential was not found.",
  "error.reason.SECURITY_EMBY_ACCOUNT_ALREADY_LINKED_INVITED":
    "Emby account is already linked or invited.",
  "error.reason.SECURITY_EMBY_CONNECTION_NO_VERIFIED_SERVER_IDENTITY":
    "Emby connection has no verified server identity.",
  "error.reason.SECURITY_EMBY_CONNECT_SIGN_DISABLED_CONNECTION":
    "Emby Connect sign-in is disabled for this connection.",
  "error.reason.SECURITY_EMBY_INVITES_REQUIRE_SAVED_INTEGRATION_API_KEY":
    "Emby invites require a saved integration API key.",
  "error.reason.SECURITY_EMBY_INVITES_REQUIRE_SELECTING_EMBY_USER_PICKER":
    "Emby invites require selecting an Emby user from the picker.",
  "error.reason.SECURITY_EMBY_SIGN_FAILED": "Emby sign-in failed.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_HEADER_TRUNCATED":
    "Encrypted backup header is truncated.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_METADATA_HEADER_TRUNCATED":
    "Encrypted backup metadata header is truncated.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_METADATA_TRUNCATED":
    "Encrypted backup metadata is truncated.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_PAYLOAD_LENGTH_INVALID":
    "Encrypted backup payload length is invalid.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_PAYLOAD_LENGTH_TRUNCATED":
    "Encrypted backup payload length is truncated.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_PAYLOAD_TRUNCATED_INVALID":
    "Encrypted backup payload is truncated or invalid.",
  "error.reason.SECURITY_ENCRYPTED_BACKUP_VERSION_UNSUPPORTED":
    "Unsupported encrypted backup version the selected value.",
  "error.reason.SECURITY_EXTERNALLY_MANAGED_USERS_CANNOT_SET_SCRYER_PASSWORD":
    "Externally managed users cannot set a Scryer password.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_DISABLED":
    "External account is disabled.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_DISABLED_INVALID":
    "External account is disabled and must be repaired by an administrator.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_NOT_FOUND":
    "External account was not found.",
  "error.reason.SECURITY_EXTERNAL_ACCOUNT_NOT_INVITED":
    "External account is not invited.",
  "error.reason.SECURITY_FAILED_COMPUTE_NEXT_BACKUP_DAY":
    "Failed to compute next backup day.",
  "error.reason.SECURITY_FAILED_DECRYPT_BACKUP_BUNDLE":
    "Failed to decrypt backup bundle: wrong password or corrupted data.",
  "error.reason.SECURITY_FAILED_DERIVE_BACKUP_ENCRYPTION_KEY_PASSWORD":
    "Failed to derive backup encryption key from password.",
  "error.reason.SECURITY_FAILED_FINISH_DISCOVERABLE_PASSKEY_AUTHENTICATION":
    "Failed to finish discoverable passkey authentication.",
  "error.reason.SECURITY_FAILED_FINISH_PASSKEY_AUTHENTICATION":
    "Failed to finish passkey authentication.",
  "error.reason.SECURITY_FAILED_FINISH_PASSKEY_REGISTRATION":
    "Failed to finish passkey registration.",
  "error.reason.SECURITY_FAILED_IDENTIFY_DISCOVERABLE_PASSKEY_AUTHENTICATION":
    "Failed to identify discoverable passkey authentication.",
  "error.reason.SECURITY_FAILED_RESOLVE_CONFIGURED_LOCAL_BACKUP_TIME":
    "Failed to resolve the configured local backup time.",
  "error.reason.SECURITY_FAILED_START_DISCOVERABLE_PASSKEY_AUTHENTICATION":
    "Failed to start discoverable passkey authentication.",
  "error.reason.SECURITY_FAILED_START_PASSKEY_AUTHENTICATION":
    "Failed to start passkey authentication.",
  "error.reason.SECURITY_FAILED_START_PASSKEY_REGISTRATION":
    "Failed to start passkey registration.",
  "error.reason.SECURITY_JELLYFIN_ACCOUNT_ALREADY_LINKED_INVITED":
    "Jellyfin account is already linked or invited.",
  "error.reason.SECURITY_JELLYFIN_CONNECTION_AUTHORIZATION_CHANGED":
    "Jellyfin connection authorization changed.",
  "error.reason.SECURITY_JELLYFIN_CONNECTION_UNAVAILABLE":
    "Jellyfin connection is unavailable.",
  "error.reason.SECURITY_JELLYFIN_CREDENTIALS_INVALID":
    "Invalid Jellyfin credentials.",
  "error.reason.SECURITY_JELLYFIN_INVITES_REQUIRE_SELECTING_JELLYFIN_USER_PICKER":
    "Jellyfin invites require selecting a Jellyfin user from the picker.",
  "error.reason.SECURITY_JELLYFIN_INVITE_API_KEY_REQUIRED":
    "Jellyfin invites require a saved Jellyfin API key so Scryer can resolve the immutable user id.",
  "error.reason.SECURITY_JELLYFIN_LINK_AUTHORIZATION_CHANGED":
    "Jellyfin link authorization changed.",
  "error.reason.SECURITY_JELLYFIN_USER_ID_INVALID":
    "Jellyfin user ID is invalid.",
  "error.reason.SECURITY_JELLYFIN_USER_VERIFICATION_FAILED":
    "Jellyfin user verification failed.",
  "error.reason.SECURITY_LAST_EXTERNAL_LOGIN_REMOVAL_FORBIDDEN":
    "Cannot unlink the last external login without a local password or passkey.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_ALREADY_USED":
    "Login verification challenge was already used.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_EXPIRED":
    "Login verification challenge has expired.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_INVALIDATED":
    "Login verification challenge was invalidated.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_CHALLENGE_UNAVAILABLE":
    "Login verification challenge is unavailable.",
  "error.reason.SECURITY_LOGIN_VERIFICATION_USER_UNAVAILABLE":
    "Login verification user is unavailable.",
  "error.reason.SECURITY_MALFORMED_BACKUP_DOWNLOAD_TOKEN":
    "Malformed backup download token.",
  "error.reason.SECURITY_MALFORMED_TOKEN": "Malformed token.",
  "error.reason.SECURITY_MANUAL_BACKUP_ALREADY_RUNNING":
    "A manual backup is already running.",
  "error.reason.SECURITY_NEW_PASSWORD_MUST_DIFFER_TEMPORARY_PASSWORD":
    "New password must differ from the temporary password.",
  "error.reason.SECURITY_NOT_ENABLED":
    "The requested value is not enabled for the selected value.",
  "error.reason.SECURITY_OAUTH_CLIENT_UNAVAILABLE":
    "OAuth client is unavailable.",
  "error.reason.SECURITY_OAUTH_GRANT_UNAVAILABLE":
    "OAuth grant is not eligible for Jellyfin linking.",
  "error.reason.SECURITY_OAUTH_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES":
    "OAuth tokens cannot carry actor capabilities.",
  "error.reason.SECURITY_OAUTH_TOKENS_CANNOT_CARRY_APP_PERMISSIONS":
    "OAuth tokens cannot carry app permissions.",
  "error.reason.SECURITY_OAUTH_TOKEN_CLAIMS_INVALID":
    "Invalid OAuth token claims.",
  "error.reason.SECURITY_PASSKEY_AUTHENTICATION_NOT_CONFIGURED":
    "Passkey authentication is not configured.",
  "error.reason.SECURITY_PASSKEY_AUTHENTICATION_RESPONSE_PAYLOAD_INVALID":
    "Invalid passkey authentication response payload.",
  "error.reason.SECURITY_PASSKEY_AUTHENTICATION_UNAVAILABLE_WHILE_FORM_LOGIN_DISABLED":
    "Passkey authentication is unavailable while form login is disabled.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_DOES_NOT_BELONG_CURRENT_USER":
    "Passkey challenge does not belong to the current user.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_DOES_NOT_BELONG_LOGIN_VERIFICATION":
    "Passkey challenge does not belong to this login verification.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_EXPIRED":
    "Passkey challenge has expired.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_ACTOR_BOUND":
    "Passkey challenge is not actor-bound.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_AUTHENTICATION_CEREMONY":
    "Passkey challenge is not an authentication ceremony.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_FOUND":
    "Passkey challenge was not found.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_NOT_REGISTRATION_CEREMONY":
    "Passkey challenge is not a registration ceremony.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_SESSION_INVALIDATED":
    "Passkey challenge session has been invalidated.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_STORED_AUTHENTICATION_CEREMONY":
    "Passkey challenge stored an authentication ceremony.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_STORED_REGISTRATION_CEREMONY":
    "Passkey challenge stored a registration ceremony.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_UNEXPECTED_AUTHENTICATION_PURPOSE":
    "Passkey challenge has an unexpected authentication purpose.",
  "error.reason.SECURITY_PASSKEY_CHALLENGE_UNEXPECTED_REGISTRATION_PURPOSE":
    "Passkey challenge has an unexpected registration purpose.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_CHANGED_DURING_AUTHENTICATION":
    "Passkey credential changed during authentication.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_CHALLENGE_USER":
    "Passkey credential does not belong to the challenge user.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_DOES_NOT_BELONG_LOGIN_VERIFICATION":
    "Passkey credential does not belong to this login verification.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_ID_ALREADY_REGISTERED":
    "A passkey with this credential id is already registered.",
  "error.reason.SECURITY_PASSKEY_CREDENTIAL_NOT_FOUND":
    "Passkey credential was not found.",
  "error.reason.SECURITY_PASSKEY_FACTOR_UNAVAILABLE":
    "Passkey factor is no longer available.",
  "error.reason.SECURITY_PASSKEY_REAUTHENTICATION_CHALLENGE_BELONGS_ANOTHER_USER":
    "Passkey reauthentication challenge belongs to another user.",
  "error.reason.SECURITY_PASSKEY_REAUTHENTICATION_UNAVAILABLE_ACCOUNT":
    "Passkey reauthentication is unavailable for this account.",
  "error.reason.SECURITY_PASSKEY_REGISTRATION_RESPONSE_PAYLOAD_INVALID":
    "Invalid passkey registration response payload.",
  "error.reason.SECURITY_PASSWORD_CHANGE_ENROLLMENT_TOKEN_CLAIMS_INVALID":
    "Invalid password-change enrollment token claims.",
  "error.reason.SECURITY_PASSWORD_HASH_VERSION_UNSUPPORTED":
    "Unsupported password hash version.",
  "error.reason.SECURITY_PASSWORD_OUT_OF_RANGE":
    "Password must be at least {{minLength}} characters.",
  "error.passwordLengthRequirement":
    "Password does not meet the configured length requirement.",
  "error.reason.SECURITY_PASSWORD_REAUTHENTICATION_UNAVAILABLE_ACCOUNT":
    "Password reauthentication is unavailable for this account.",
  "error.reason.SECURITY_PASSWORD_REQUIRED": "Password is required.",
  "error.reason.SECURITY_PLEX_ACCOUNT_ALREADY_LINKED_INVITED":
    "Plex account is already linked or invited.",
  "error.reason.SECURITY_PLEX_INVITES_REQUIRE_SELECTING_PLEX_USER_PICKER":
    "Plex invites require selecting a Plex user from the picker.",
  "error.reason.SECURITY_PLEX_INVITE_TOKEN_REQUIRED":
    "Plex invites require a saved Plex token so Scryer can resolve the immutable user id.",
  "error.reason.SECURITY_PLEX_SERVER_DISCOVERY_REQUIRED_BEFORE_USING_PLEX_AUTH":
    "Plex server discovery is required before using Plex for auth.",
  "error.reason.SECURITY_PREPARED_BACKUP_DIRECTORY_CONTAINS_UNSUPPORTED_ENTRY":
    "Prepared backup directory contains unsupported entry the selected value.",
  "error.reason.SECURITY_RECOVERY_ADMIN_LOGIN_MANAGED_ENVIRONMENT":
    "Recovery-admin login is managed by the environment.",
  "error.reason.SECURITY_RECOVERY_ADMIN_USER_EXTERNALLY_MANAGED_CANNOT_REPAIRED_LOCAL_LOGIN":
    "Recovery admin user is externally managed and cannot be repaired for local login.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_EXPIRED":
    "Release candidate expired; search again.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_EMPTY_EPISODE_SET_SCOPE":
    "Release candidate token has empty episode-set scope.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_KIND_INVALID":
    "Invalid release candidate token kind.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_COLLECTION_SCOPE_ID":
    "Release candidate token missing collection scope id.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SCOPE_ID":
    "Release candidate token missing episode scope id.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_EPISODE_SET_SCOPE_ID":
    "Release candidate token missing episode-set scope id.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_MISSING_SERIES_MOVIE_SCOPE_ID":
    "Release candidate token missing series movie scope id.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_HINT":
    "Release candidate token requires a source hint.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_REQUIRES_SOURCE_TITLE":
    "Release candidate token requires a source title.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_SCOPE_DOES_NOT_MATCH_REQUEST":
    "Release candidate token scope does not match request.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_SUBJECT_DOES_NOT_MATCH_ACTOR":
    "Release candidate token subject does not match actor.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_TITLE_DOES_NOT_MATCH_REQUEST":
    "Release candidate token title does not match request.",
  "error.reason.SECURITY_RELEASE_CANDIDATE_TOKEN_UNKNOWN_SCOPE":
    "Release candidate token has unknown scope.",
  "error.reason.SECURITY_RELEASE_REPORTS_SEEDERS_BELOW_MINIMUM_INDEXER":
    "Release reports the selected value seeders, below the minimum of the selected value for this indexer.",
  "error.reason.SECURITY_REQUESTED_LOGIN_VERIFICATION_FACTOR_UNAVAILABLE":
    "Requested login verification factor is unavailable.",
  "error.reason.SECURITY_RESERVED_INSTANCE_RECOVERY":
    "The requested value is reserved for instance recovery.",
  "error.reason.SECURITY_RESTRICTED_AUTHENTICATION_TOKENS_CANNOT_CARRY_ACTOR_CAPABILITIES":
    "Restricted authentication tokens cannot carry actor capabilities.",
  "error.reason.SECURITY_SCRYER_ADMIN_USERNAME_CONFLICTS_EXISTING_USERNAME":
    "SCRYER_ADMIN_USERNAME conflicts with an existing username; use its exact stored spelling.",
  "error.reason.SECURITY_SCRYER_ADMIN_USERNAME_MUST_NAME_NON_RESERVED_LOCAL_ADMINISTRATOR":
    "SCRYER_ADMIN_USERNAME must name a non-reserved local administrator.",
  "error.reason.SECURITY_SELECTED_BOOTSTRAP_ACCOUNT_MUST_ALREADY_ENABLED_LOCAL_FULL_ADMINISTRATOR":
    "Selected bootstrap account must already be an enabled local full administrator; use recovery for lockouts.",
  "error.reason.SECURITY_SELECTED_EMBY_USER_NOT_FOUND_CONFIGURED_SERVER":
    "Selected Emby user was not found on the configured server.",
  "error.reason.SECURITY_SELECTED_JELLYFIN_USER_NOT_FOUND_CONFIGURED_SERVER":
    "Selected Jellyfin user was not found on the configured server.",
  "error.reason.SECURITY_SELECTED_PLEX_USER_NOT_FOUND_CONFIGURED_ACCOUNT":
    "Selected Plex user was not found on the configured account.",
  "error.reason.SECURITY_SESSION_DURATION_OUT_OF_RANGE":
    "Session duration must be between 1 and 365 days.",
  "error.reason.SECURITY_TOKEN_INVALID": "Invalid token.",
  "error.reason.SECURITY_TOKEN_SUBJECT_NO_LONGER_EXISTS":
    "Token subject no longer exists.",
  "error.reason.SECURITY_TOKEN_SUBJECT_UNKNOWN": "Unknown token subject.",
  "error.reason.SECURITY_TOTP_ALREADY_ENABLED": "TOTP is already enabled.",
  "error.reason.SECURITY_TOTP_CHALLENGE_NOT_FOUND":
    "Totp challenge was not found.",
  "error.reason.SECURITY_TOTP_ENROLLMENT_CHALLENGE_EXPIRED":
    "TOTP enrollment challenge has expired.",
  "error.reason.SECURITY_USERNAME_REQUIRED": "Username is required.",
  "error.reason.SECURITY_USER_ALREADY_EXISTS":
    "User the selected value already exists.",
  "error.reason.SECURITY_USER_NOT_FOUND": "User was not found.",
  "error.reason.SECURITY_USE_CHANGE_OWN_PASSWORD_UPDATE_OWN_PASSWORD":
    "Use change_own_password to update your own password.",
  "error.reason.SECURITY_V2_PASSWORD_HASH_INVALID": "Invalid v2 password hash.",
  "error.reason.SECURITY_VERIFIED_EXTERNAL_IDENTITY_CONNECTION_DID_NOT_MATCH_REQUESTED_CONNECTION":
    "Verified external identity connection did not match the requested connection.",
  "error.reason.SECURITY_VERIFIED_EXTERNAL_IDENTITY_PROVIDER_DID_NOT_MATCH_REQUESTED_PROVIDER":
    "Verified external identity provider did not match the requested provider.",
  "error.reason.SERVICES_ACTION_PERMISSION_DENIED":
    "You do not have permission to perform this action.",
  "error.reason.SERVICES_COMPLETED_DOWNLOAD_NOT_FOUND":
    "Completed download was not found.",
  "error.reason.SERVICES_DOWNLOAD_CLIENT_ASSIGNED_PROXY_WHICH_DISABLED":
    "Download client the selected value is assigned proxy the selected value, which is disabled.",
  "error.reason.SERVICES_DOWNLOAD_CLIENT_ITEM_ID_REQUIRED":
    "Download client item id is required.",
  "error.reason.SERVICES_DOWNLOAD_CLIENT_REFERENCES_PROXY_CONFIGURATION_WHICH_NOT_FOUND":
    "Download client  references proxy configuration , which was not found.",
  "error.reason.SERVICES_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.SERVICES_MAINTENANCE_OPERATION_PROGRESS":
    "Maintenance operation in progress.",
  "error.reason.SERVICES_MOVIE_METADATA_RESPONSE_MISSING_TITLE_NOT_FOUND":
    "Movie metadata response missing title was not found.",
  "error.reason.SERVICES_TITLE_IDENTITY_REQUIRED":
    "A title identity is required.",
  "error.reason.SERVICES_TITLE_IDS_OUT_OF_RANGE": "Title ids must be positive.",
  "error.reason.SETTINGS_ACQUISITION_POLL_INTERVAL_OUT_OF_RANGE":
    "Acquisition poll interval must be at least 1 second.",
  "error.reason.SETTINGS_ACQUISITION_THRESHOLDS_CANNOT_NEGATIVE":
    "Acquisition thresholds cannot be negative.",
  "error.reason.SETTINGS_ACQUISITION_WALK_INTERVAL_OUT_OF_RANGE":
    "Acquisition walk interval must be at least 1 second.",
  "error.reason.SETTINGS_ANIME_SPECIFIC_SETTINGS_REQUIRE_ANIME_LIBRARY":
    "Anime-specific settings require an anime library.",
  "error.reason.SETTINGS_ANIME_SPECIFIC_SETTINGS_REQUIRE_SCOPE_ANIME":
    "Anime-specific settings require scope anime.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUPS_REQUIRE_BACKUP_KEY":
    "Automatic backups require a backup key.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUP_KEY_CANNOT_CLEARED_WHILE_AUTOMATIC_BACKUPS_DISABLED":
    "Automatic backup key cannot be cleared while automatic backups are enabled.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUP_KEY_CANNOT_REPLACED_CLEARED_SAME_REQUEST":
    "Automatic backup key cannot be replaced and cleared in the same request.",
  "error.reason.SETTINGS_AUTOMATIC_BACKUP_KEY_OUT_OF_RANGE":
    "Automatic backup key must be at least the selected value non-whitespace characters.",
  "error.reason.SETTINGS_BACKUP_PATH_INVALID":
    "Backup path must be a directory.",
  "error.reason.SETTINGS_BACKUP_PATH_NOT_WRITABLE":
    "Backup path is not writable.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_GLOBAL_DEFAULT":
    "Cannot remove quality profile the selected item because it is the global default.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_ALLOWS_REQUESTS":
    "Cannot remove quality profile the selected item because library the selected item allows it for requests.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_LIBRARY_REFERENCES":
    "Cannot remove quality profile the selected item because library the selected item references it.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_NOT_CONFIGURED":
    "Cannot remove quality profile the selected item because it is configured for the selected value.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_PENDING_MEDIA_REQUEST_S_REFERENCE":
    "Cannot remove quality profile the selected item because the selected value pending media request(s) reference it.",
  "error.reason.SETTINGS_CANNOT_REMOVE_QUALITY_PROFILE_TITLE_S_REFERENCE":
    "Cannot remove quality profile the selected item because the selected value title(s) reference it.",
  "error.reason.SETTINGS_CANNOT_SAVE_RESET_TRUSTED_PROXIES_TOGETHER":
    "Cannot save and reset trusted proxies together.",
  "error.reason.SETTINGS_CATALOG_VIEW_COLUMNS_INVALID":
    "Catalog view columns must match the saved device class and facet.",
  "error.reason.SETTINGS_CERTIFICATE_BUNDLE_EMPTY":
    "Trusted certificate bundle did not contain a readable X.509 certificate.",
  "error.reason.SETTINGS_CHANGE_DEFAULT_ADMIN_PASSWORD_BEFORE_ENABLING_FORM_LOGIN":
    "Change the default admin password before enabling form login.",
  "error.reason.SETTINGS_CONFIGURE_ENABLED_FULL_ADMINISTRATOR_LOGIN_BEFORE_ENABLING_FORM_LOGIN":
    "Configure an enabled full administrator login before enabling form login.",
  "error.reason.SETTINGS_CONVERGENCE_PER_CYCLE_SCOPE_CEILING_OUT_OF_RANGE":
    "Convergence per-cycle scope ceiling must be at least 1.",
  "error.reason.SETTINGS_DEFAULT_LIBRARY_NOT_FOUND":
    "Default  library was not found.",
  "error.reason.SETTINGS_DEFAULT_MOVIE_AVAILABILITY_INVALID":
    "Default movie availability must be announced, in_cinemas, or released.",
  "error.reason.SETTINGS_DELAY_PROFILE_ID_REQUIRED":
    "Delay profile id is required.",
  "error.reason.SETTINGS_DELAY_PROFILE_NAMES_UNKNOWN_LIBRARY":
    "Delay profile the selected item names an unknown library the selected item.",
  "error.reason.SETTINGS_DELAY_PROFILE_NOT_FOUND":
    "Delay profile was not found.",
  "error.reason.SETTINGS_DOWNLOAD_CLIENT_ROUTING_ENTRY_REQUIRES_CLIENT_ID":
    "Download client routing entry requires client_id.",
  "error.reason.SETTINGS_EACH_TRUSTED_CERTIFICATE_ENTRY_INVALID":
    "Each trusted certificate entry must contain exactly one X.509 certificate.",
  "error.reason.SETTINGS_FAILED_CREATE_BACKUP_DIRECTORY":
    "Failed to create backup directory.",
  "error.reason.SETTINGS_FAILED_PARSE_TRAILING_PEM_CONTENT_TRUSTED_CERTIFICATE":
    "Failed to parse trailing PEM content for trusted certificate.",
  "error.reason.SETTINGS_FAILED_PARSE_TRUSTED_CERTIFICATE_PEM_BLOCK":
    "Failed to parse trusted certificate PEM block.",
  "error.reason.SETTINGS_FAILED_REMOVE_BACKUP_PATH_PROBE":
    "Failed to remove backup path probe.",
  "error.reason.SETTINGS_FORM_LOGIN_REQUIRED_DEFAULT_ADMIN_DISABLED":
    "Form login cannot be turned off while SCRYER_DISABLE_DEFAULT_ADMIN is true, because running without login uses the default admin account; remove that setting and restart first.",
  "error.reason.SETTINGS_HISTORY_RETENTION_DAYS_OUT_OF_RANGE":
    "History retention days must be at least 1.",
  "error.reason.SETTINGS_IMAGE_CACHE_MAXIMUM_SIZE_OUT_OF_RANGE":
    "Image cache maximum size must be at least 1 MiB.",
  "error.reason.SETTINGS_INDEXER_ROUTING_ENTRY_REQUIRES_INDEXER_ID":
    "Indexer routing entry requires indexer_id.",
  "error.reason.SETTINGS_LANGUAGE_OUT_OF_RANGE":
    "Language must be a language code of at most the selected value letters, digits or hyphens.",
  "error.reason.SETTINGS_LIBRARY_ACCESS_DENIED":
    "You do not have access to this library.",
  "error.reason.SETTINGS_LIBRARY_NOT_FOUND": "Library was not found.",
  "error.reason.SETTINGS_LIMIT_REQUIRED_OUT_OF_RANGE":
    "Limit is required and must be greater than zero.",
  "error.reason.SETTINGS_MEDIA_SETTING_CHANGE_REQUIRED":
    "At least one media setting change is required.",
  "error.reason.SETTINGS_METADATA_LANGUAGE_INVALID":
    "Metadata language must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn.",
  "error.reason.SETTINGS_MONITOR_SPECIALS_ONLY_VALID_ANIME_LIBRARIES":
    "Monitor_specials is only valid for anime libraries.",
  "error.reason.SETTINGS_MOVIES_TABLE_SETTINGS_CANNOT_INCLUDE_EPISODES_COLUMN":
    "Movies table settings cannot include the episodes column.",
  "error.reason.SETTINGS_MOVIE_AVAILABILITY_SETTINGS_REQUIRE_MOVIE_LIBRARY":
    "Movie availability settings require a movie library.",
  "error.reason.SETTINGS_MOVIE_AVAILABILITY_SETTINGS_REQUIRE_SCOPE_MOVIE":
    "Movie availability settings require scope movie.",
  "error.reason.SETTINGS_MOVIE_RELEASE_MARKET_INVALID":
    "Movie release market must be a two-letter ISO-3166 country code.",
  "error.reason.SETTINGS_MUST_ABSOLUTE_PATH_COMMAND_NAME_WITHOUT_PATH_SEPARATORS":
    "The requested value must be an absolute path or a command name without path separators.",
  "error.reason.SETTINGS_MUST_RRGGBB_HEX_COLOR":
    "The requested value must be a #RRGGBB hex color.",
  "error.reason.SETTINGS_PASSWORD_MINIMUM_LENGTH_OUT_OF_RANGE":
    "Password minimum length must be at least the selected value.",
  "error.reason.SETTINGS_PERMISSIONS_CHOWN_GROUP_SETTING_VALUE_INVALID":
    "Invalid permissions.chown_group setting value the selected item: numeric gid is outside range.",
  "error.reason.SETTINGS_PERSONA_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE":
    "Persona is required when inherit_global is false.",
  "error.reason.SETTINGS_PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME":
    "Plexmatch_write_on_import is only valid for series and anime.",
  "error.reason.SETTINGS_PLEXMATCH_WRITE_IMPORT_ONLY_VALID_SERIES_ANIME_LIBRARIES":
    "Plexmatch_write_on_import is only valid for series and anime libraries.",
  "error.reason.SETTINGS_PROFILE_ID_REQUIRED": "Profile_id is required.",
  "error.reason.SETTINGS_PROFILE_ID_REQUIRED_WHEN_INHERIT_GLOBAL_FALSE":
    "Profile_id is required when inherit_global is false.",
  "error.reason.SETTINGS_QUALITY_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE":
    "Quality profile ids the selected item and the selected item differ only by ASCII case.",
  "error.reason.SETTINGS_QUALITY_PROFILE_ID_AMBIGUOUS_PROFILE_IDS_DIFFER_ONLY_ASCII_CASE":
    "Quality profile id the selected item is ambiguous because profile ids differ only by ASCII case.",
  "error.reason.SETTINGS_QUALITY_PROFILE_ID_REQUIRED":
    "Quality profile id is required.",
  "error.reason.SETTINGS_QUALITY_PROFILE_NOT_FOUND":
    "Quality profile was not found.",
  "error.reason.SETTINGS_QUALITY_PROFILE_UNKNOWN":
    "Unknown quality profile the selected item.",
  "error.reason.SETTINGS_RECYCLE_BIN_PATH_UNSAFE":
    "The recycle bin path is unsafe or overlaps managed media.",
  "error.reason.SETTINGS_RECYCLE_BIN_RETENTION_OUT_OF_RANGE":
    "Recycle bin retention must be between 1 and the selected value days.",
  "error.reason.SETTINGS_REQUEST_QUALITY_PROFILE_UNKNOWN":
    "Unknown request quality profile the selected value.",
  "error.reason.SETTINGS_RE_CONVERGE_BACKSTOP_CANNOT_NEGATIVE":
    "Re-converge backstop cannot be negative.",
  "error.reason.SETTINGS_ROOT_FOLDER_PATH_REQUIRED":
    "Root folder path is required.",
  "error.reason.SETTINGS_ROOT_REQUIRED":
    "At least one root folder is required.",
  "error.reason.SETTINGS_SEASON_FOLDERS_ONLY_VALID_SERIES_ANIME_LIBRARIES":
    "Season folders are only valid for series and anime libraries.",
  "error.reason.SETTINGS_SEASON_FOLDER_TEMPLATES_ONLY_SUPPORTED_SERIES_ANIME":
    "Season folder templates are only supported for series and anime.",
  "error.reason.SETTINGS_SESSION_DURATION_OUT_OF_RANGE":
    "Session duration must be between 1 and 365 days.",
  "error.reason.SETTINGS_SETTING_VALUE_INVALID":
    "Invalid the selected value setting value the selected item: expected a 3 or 4 digit octal mask.",
  "error.reason.SETTINGS_SOURCE_REQUIRED": "Source is required.",
  "error.reason.SETTINGS_SUBTITLE_MINIMUM_SCORE_PERCENTAGES_OUT_OF_RANGE":
    "Subtitle minimum score percentages must be between 0 and 100.",
  "error.reason.SETTINGS_SUBTITLE_SEARCH_INTERVAL_OUT_OF_RANGE":
    "Subtitle search interval must be at least 1 hour.",
  "error.reason.SETTINGS_SUBTITLE_SYNC_MAX_OFFSET_CANNOT_NEGATIVE":
    "Subtitle sync max offset cannot be negative.",
  "error.reason.SETTINGS_SUBTITLE_SYNC_THRESHOLD_PERCENTAGES_OUT_OF_RANGE":
    "Subtitle sync threshold percentages must be between 0 and 100.",
  "error.reason.SETTINGS_TABLE_COLUMN_ORDER_OUT_OF_RANGE":
    "Table column order must be greater than or equal to 0.",
  "error.reason.SETTINGS_TABLE_COLUMN_UNSUPPORTED":
    "Unsupported the selected value table column the selected value.",
  "error.reason.SETTINGS_TITLE_IMAGE_CACHE_REFRESH_ALREADY_RUNNING":
    "Title image cache refresh is already running.",
  "error.reason.SETTINGS_TRUSTED_CERTIFICATE_BUNDLE_INVALID":
    "Trusted certificate bundle must contain PEM-encoded X.509 certificates.",
  "error.reason.SETTINGS_TRUSTED_CERTIFICATE_BUNDLE_MAY_ONLY_CONTAIN_X_509_CERTIFICATE_PEM_BLOCKS":
    "Trusted certificate bundle may only contain X.509 certificate PEM blocks.",
  "error.reason.SETTINGS_UI_TABLE_COLUMN_SETTING_COLUMN_DUPLICATED":
    "Duplicate UI table column setting for the selected value the selected value the selected value column the selected value.",
  "error.reason.SETTINGS_VIEW_QUALITY_PROFILES_PERMISSION_DENIED":
    "You do not have permission to view quality profiles.",
  "error.reason.SETTINGS_VIEW_RECYCLE_BIN_SETTINGS_PERMISSION_DENIED":
    "You do not have permission to view recycle bin settings.",
  "error.reason.SETTINGS_VIEW_VERIFICATION_SETTINGS_PERMISSION_DENIED":
    "You do not have permission to view verification settings.",
  "error.reason.SUBTITLES_ARCHIVE_PLUGIN_DOES_NOT_SUPPORT_SUBTITLE_ARCHIVE_FORMAT":
    "Archive plugin does not support this subtitle archive format.",
  "error.reason.SUBTITLES_ARCHIVE_PLUGIN_OUTPUT_ESCAPED_OUTPUT_ROOT":
    "Archive plugin output escaped output root.",
  "error.reason.SUBTITLES_ARCHIVE_SUBTITLE_NOT_FOUND":
    "Subtitle archive did not contain a supported subtitle file.",
  "error.reason.SUBTITLES_FILE_TOO_SMALL_SUBTITLE_FILE_HASH":
    "File too small for subtitle file hash.",
  "error.reason.SUBTITLES_MEDIA_FILE_NOT_FOUND": "Media file was not found.",
  "error.reason.SUBTITLES_NO_ENABLED_SUBTITLE_PROVIDERS_NOT_CONFIGURED":
    "No enabled subtitle providers are configured.",
  "error.reason.SUBTITLES_SIDECAR_SYMLINK_FORBIDDEN":
    "Subtitle sidecar must not be a symlink.",
  "error.reason.SUBTITLES_SUBTITLE_ARCHIVE_CONTAINS_TOO_MANY_FILES":
    "Subtitle archive contains too many files.",
  "error.reason.SUBTITLES_SUBTITLE_ARCHIVE_MULTIPLE_EQUALLY_RANKED_SUBTITLE_FILES":
    "Subtitle archive has multiple equally ranked subtitle files.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACTS_CONTAIN_TOO_MANY_FILES":
    "Subtitle artifacts contain too many files.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACT_EXCEEDS_NESTING_INPUT_LIMITS":
    "Subtitle artifact exceeds nesting or input limits.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACT_EXPANDS_BEYOND_BYTE_LIMIT":
    "Subtitle artifact expands beyond the the selected value byte limit.",
  "error.reason.SUBTITLES_SUBTITLE_ARTIFACT_FORMAT_UNSUPPORTED":
    "Unsupported subtitle artifact format.",
  "error.reason.SUBTITLES_SUBTITLE_FORMAT_UNSUPPORTED":
    "Unsupported subtitle format.",
  "error.reason.SUBTITLES_SUBTITLE_LANGUAGE_CODE_INVALID":
    "Invalid subtitle language code.",
  "error.reason.SUBTITLES_SUBTITLE_PLUGIN_PROVIDER_TYPE_NOT_FOUND":
    "No subtitle plugin for provider type was not found.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_FIELD_REQUIRED":
    "At least one subtitle provider config field must be provided.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_JSON_INVALID":
    "Subtitle provider config_json must be a JSON object.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_NAME_REQUIRED":
    "Subtitle provider config name must not be empty.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_CONFIG_NOT_FOUND":
    "Subtitle provider config was not found.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_NOT_NOT_FOUND":
    "Subtitle provider  is not configured was not found.",
  "error.reason.SUBTITLES_SUBTITLE_PROVIDER_TYPE_REQUIRED":
    "Subtitle provider type must not be empty.",
  "error.reason.SUBTITLES_SUBTITLE_SIDECAR_CHANGED_DURING_CONTENT_PROBE":
    "Subtitle sidecar changed during content probe.",
  "error.reason.SUBTITLES_SUBTITLE_SIDECAR_EXCEEDS_CONTENT_PROBE_SIZE_LIMIT":
    "Subtitle sidecar exceeds content probe size limit.",
  "error.reason.SUBTITLES_SUBTITLE_SIDECAR_INVALID":
    "Subtitle sidecar must be a regular file.",
  "error.reason.SUBTITLES_TITLE_NOT_FOUND": "Title was not found.",
  "error.reason.TITLE_NOT_FOUND": "The title could not be found.",
  "error.reason.WORKFLOW_ACQUISITION_RECEIPT_REQUIRED":
    "Maintenance Search requires an accepted receipt for its acquisition job run.",
  "error.reason.WORKFLOW_CLEANUP_IDENTITY_REQUIRED":
    "Manual cleanup requires title and facet.",
  "error.reason.WORKFLOW_DOWNLOAD_AWAITING_RETRY_RECONCILIATION":
    "Download is awaiting retry reconciliation.",
  "error.reason.WORKFLOW_DOWNLOAD_BINDING_CHANGED_DURING_RETRY":
    "Download binding changed during retry; reconciliation required.",
  "error.reason.WORKFLOW_DOWNLOAD_IDENTITY_UNRESOLVED":
    "This download has no stable identity yet; its sources stay held.",
  "error.reason.WORKFLOW_DOWNLOAD_RETRY_OWNERSHIP_LOST":
    "Download retry no longer owns this job.",
  "error.reason.WORKFLOW_DOWNLOAD_SUBMISSION_NOT_FOUND":
    "The download submission could not be found.",
  "error.reason.WORKFLOW_IMPORT_NOT_FOUND": "The import could not be found.",
  "error.reason.WORKFLOW_IMPORT_RELEASE_STALE":
    "The import changed while its release was requested; refresh the queue.",
  "error.reason.WORKFLOW_IMPORT_RETRY_DOWNLOAD_NOT_FOUND":
    "The import retry download could not be found.",
  "error.reason.WORKFLOW_IMPORT_RUNNING":
    "This download is still being imported; release its sources once that finishes.",
  "error.reason.WORKFLOW_IMPORT_SOURCES_HELD":
    "Another import of this download holds its sources; refresh the queue.",
  "error.reason.WORKFLOW_OPERATION_NOT_FOUND":
    "The operation could not be found.",
  "error.reason.WORKFLOW_PASSWORD_RETRY_AWAITING_RECONCILIATION":
    "Password retry is awaiting reconciliation; binding preserved.",
  "error.reason.WORKFLOW_RETRY_DOWNLOAD_MISMATCH":
    "Retry response belongs to another download; reconciliation required.",
  "error.reason.WORKFLOW_RETRY_PROGRESS_CONFLICT":
    "Retry response conflicts with observed progress; reconciliation required.",
  "error.reason.WORKFLOW_SOURCE_CLEANUP_ALREADY_CLAIMED":
    "Source cleanup already owns this download.",

  "error.ruleUnknownInputPath":
    "Unknown rule input path: {{path}}. Choose a documented field.",
  "error.ruleDynamicInputPath":
    "This rule input path cannot be resolved statically: {{path}}. Name the field directly.",
  "error.rulePackageRequired": "The rule must declare its assigned package.",
  "error.ruleInputImportUnsupported":
    "Reference rule input directly instead of importing it under another name.",
  "error.ruleMatchRequired":
    "Define a boolean match rule for this maintenance matcher.",
  "error.ruleVoteRequired":
    "Define approve, deny, manual, or tags for this request matcher.",

  "error.reason.INTERFACE_ACTION_DEFINITION_CONFLICT":
    "Choose an action or an action sequence, not both.",
  "error.reason.INTERFACE_ACTION_DEFINITION_REQUIRED":
    "Choose an action or an action sequence.",
  "error.reason.INTERFACE_ACTION_SEQUENCE_INPUT_REQUIRED":
    "Use an action sequence input to create a sequence.",
  "error.reason.INTERFACE_ACTION_SCHEMA_VERSION_INVALID":
    "The action sequence schema version cannot be negative.",
  "error.reason.INTERFACE_ACTION_UNMONITOR_PARAMETERS_INVALID":
    "An unmonitor step only accepts the descendant option.",
  "error.reason.INTERFACE_ACTION_UNMONITOR_DESCENDANTS_REQUIRED":
    "Choose whether the unmonitor step includes descendants.",
  "error.reason.INTERFACE_ACTION_QUALITY_PARAMETERS_INVALID":
    "A quality profile step only accepts a target profile.",
  "error.reason.INTERFACE_ACTION_QUALITY_PROFILE_REQUIRED":
    "Choose a target quality profile for the step.",
  "error.reason.INTERFACE_ACTION_SEARCH_PARAMETERS_INVALID":
    "A search step only accepts a search condition.",
  "error.reason.INTERFACE_ACTION_SEARCH_CONDITION_REQUIRED":
    "Choose a search condition for the step.",
  "error.reason.INTERFACE_ACTION_TAG_PARAMETERS_INVALID":
    "A tag step only accepts tags.",
  "error.reason.INTERFACE_ACTION_TAGS_REQUIRED": "Choose tags for the step.",
  "error.reason.INTERFACE_ACTION_PARAMETERS_UNEXPECTED":
    "This action step does not accept parameters.",
  "error.reason.INTERFACE_REQUEST_SAMPLE_LEASE_CONFLICT":
    "Choose a preview lease duration or an unlimited lease, not both.",

  "error.reason.INTERFACE_MINIMUM_AVAILABILITY_INVALID":
    "Choose an announced, cinema, or released availability.",
  "error.reason.INTERFACE_MINIMUM_AVAILABILITY_FACET_UNSUPPORTED":
    "Minimum availability can only be set on a movie.",
  "error.reason.INTERFACE_SEASON_FOLDERS_FACET_UNSUPPORTED":
    "Season folders can only be set for series or anime.",
  "error.reason.INTERFACE_RELEASE_NUMBERING_FACET_UNSUPPORTED":
    "Release numbering can only be set for series or anime.",
  "error.reason.INTERFACE_MONITOR_SELECTION_FACET_UNSUPPORTED":
    "Advanced monitoring can only be set for series or anime.",
  "error.reason.INTERFACE_MONITOR_SELECTION_REQUIRED":
    "Choose at least one season or series movie for advanced monitoring.",
  "error.reason.INTERFACE_MONITOR_SELECTION_MODE_INVALID":
    "Use advanced monitoring to choose seasons or series movies.",
  "error.reason.INTERFACE_METADATA_LANGUAGE_UNSUPPORTED":
    "Choose a supported metadata language.",
  "error.reason.INTERFACE_SEARCH_OPTIONS_LIMIT_EXCEEDED":
    "Choose at most 32 search options.",
  "error.reason.INTERFACE_SEARCH_ALIAS_INVALID":
    "Search aliases must contain between 1 and 200 characters without control characters.",
  "error.reason.INTERFACE_ROOT_FOLDER_NOT_FOUND":
    "Choose a configured library root.",
  "error.reason.INTERFACE_ROOT_FOLDER_LIBRARY_MISMATCH":
    "Choose a root on the title library.",
  "error.reason.INTERFACE_TITLE_LIBRARY_UNAVAILABLE":
    "The title library is unavailable.",
  "error.reason.INTERFACE_SEARCH_LANGUAGE_UNSUPPORTED":
    "Choose a supported search language.",
  "error.reason.INTERFACE_CONFIG_JSON_INVALID":
    "The configuration must be valid JSON.",
  "error.reason.INTERFACE_NOTIFICATION_JSON_INVALID":
    "The notification configuration must be valid JSON.",
  "error.reason.INTERFACE_LOGIN_FORM_REQUIRED":
    "Enable form login before enabling media server login.",
  "error.reason.INTERFACE_OAUTH_SESSION_REQUIRED":
    "Use an OAuth session for this operation.",
  "error.reason.INTERFACE_BROWSE_DIRECTORY_MISSING":
    "The directory does not exist.",
  "error.reason.INTERFACE_BROWSE_DIRECTORY_UNREADABLE":
    "The directory cannot be read.",
  "error.reason.INTERFACE_BROWSE_DIRECTORY_UNAVAILABLE":
    "The directory cannot be opened.",
  "error.reason.INTERFACE_RULE_ACTION_INVALID":
    "The maintenance action definition is invalid.",
  "error.reason.INTERFACE_REQUEST_RULE_SAMPLE_INVALID":
    "The request rule sample is invalid.",

  "error.reason.INTERFACE_ACQUISITION_SCOPE_INVALID":
    "Automatic search requires a title and optional season without Wanted filters.",
  "error.reason.INTERFACE_BROWSE_PATH_NOT_ABSOLUTE":
    "Choose an absolute directory path.",
  "error.reason.INTERFACE_BROWSE_PATH_NOT_DIRECTORY": "Choose a directory.",
  "error.reason.FOLDER_CREATE_NAME_REQUIRED": "Enter a folder name.",
  "error.reason.FOLDER_CREATE_NAME_RESERVED":
    "That name is reserved and can't be used for a folder.",
  "error.reason.FOLDER_CREATE_NAME_INVALID_CHARACTERS":
    "Folder names can't contain / \\ : * ? \" < > | or control characters.",
  "error.reason.FOLDER_CREATE_NAME_TRAILING_DOT":
    "Folder names can't end with a dot.",
  "error.reason.FOLDER_CREATE_NAME_HIDDEN":
    "Folder names can't start with a dot, because the folder would be hidden.",
  "error.reason.FOLDER_CREATE_NAME_TOO_LONG":
    "That name is too long for this drive.",
  "error.reason.FOLDER_CREATE_NAME_NOT_ALLOWED":
    "This drive doesn't accept that name. Try a simpler one.",
  "error.reason.FOLDER_CREATE_PARENT_NOT_ABSOLUTE":
    "Choose an absolute folder path.",
  "error.reason.FOLDER_CREATE_PARENT_MISSING":
    "The folder you're in no longer exists or can't be reached.",
  "error.reason.FOLDER_CREATE_PARENT_NOT_DIRECTORY":
    "The path you're in is a file, not a folder.",
  "error.reason.FOLDER_CREATE_ALREADY_EXISTS":
    "A file or folder with that name is already here. Some drives treat upper- and lower-case names as the same.",
  "error.reason.FOLDER_CREATE_PERMISSION_DENIED":
    "Scryer isn't allowed to create folders here. Check the folder's permissions for the user Scryer runs as (PUID and PGID in Docker).",
  "error.reason.FOLDER_CREATE_READ_ONLY":
    "This drive is read-only. If Scryer runs in Docker, check the volume isn't mounted read-only (:ro).",
  "error.reason.FOLDER_CREATE_DISK_FULL":
    "The drive is full, or its storage quota is used up.",
  "error.reason.FOLDER_CREATE_FOLDER_FULL":
    "This folder already holds as many subfolders as its drive allows.",
  "error.reason.FOLDER_CREATE_NOT_SUPPORTED":
    "This location doesn't support creating folders.",
  "error.reason.FOLDER_CREATE_DRIVE_UNAVAILABLE":
    "The drive or network share isn't responding. Check that it's connected and mounted.",
  "error.reason.FOLDER_CREATE_FAILED":
    "Scryer couldn't create the folder. The server log has the details.",
  "error.reason.INTERFACE_CATALOG_FILTER_BLANK":
    "Catalog filter entries cannot be blank.",
  "error.reason.INTERFACE_CATALOG_RATING_INVALID":
    "Choose a minimum rating between 0 and 10.",
  "error.reason.INTERFACE_CATALOG_TAG_FILTER_INVALID":
    "Choose valid catalog tags.",
  "error.reason.INTERFACE_CATALOG_YEARS_INVALID":
    "The minimum year cannot exceed the maximum year.",
  "error.reason.INTERFACE_CONFIG_KEY_REQUIRED": "Enter a configuration key.",
  "error.reason.INTERFACE_CONFIG_NUMBER_INVALID":
    "Enter a valid finite number.",
  "error.reason.INTERFACE_CONFIG_VALUE_AMBIGUOUS":
    "Provide exactly one configuration value.",
  "error.reason.INTERFACE_CONFIG_VALUE_OUT_OF_RANGE":
    "The configuration value is outside its allowed range.",
  "error.reason.INTERFACE_CUSTOM_JOB_ID_REQUIRED":
    "Choose the custom job to run.",
  "error.reason.INTERFACE_CUSTOM_JOB_ID_UNEXPECTED":
    "A custom job identifier can only be used with a custom job.",
  "error.reason.INTERFACE_DISCOVERY_FACET_UNSUPPORTED":
    "Choose a supported media type.",
  "error.reason.INTERFACE_DISCOVERY_FILTER_BLANK":
    "Discovery filter entries cannot be blank.",
  "error.reason.INTERFACE_DISCOVERY_LIMIT_INVALID":
    "Choose a valid number of results per section.",
  "error.reason.INTERFACE_DISCOVERY_RATING_INVALID":
    "Choose a minimum rating between 0 and 10.",
  "error.reason.INTERFACE_DISCOVERY_SURFACE_UNSUPPORTED":
    "Choose a supported discovery surface.",
  "error.reason.INTERFACE_DISCOVERY_YEARS_INVALID":
    "The minimum year cannot exceed the maximum year.",
  "error.reason.INTERFACE_DOWNLOAD_CLIENT_NOT_FOUND":
    "The download client could not be found.",
  "error.reason.INTERFACE_DOWNLOAD_CLIENT_PROVIDER_MISMATCH":
    "The selected download client uses a different provider type.",
  "error.reason.INTERFACE_DOWNLOAD_ID_INVALID":
    "The download identifier is invalid.",
  "error.reason.INTERFACE_HISTORY_EVENT_TYPE_INVALID":
    "Choose a supported title history event type.",
  "error.reason.INTERFACE_IDEMPOTENCY_KEY_DUPLICATE":
    "Each request must have a unique idempotency key.",
  "error.reason.INTERFACE_IDEMPOTENCY_KEY_REQUIRED":
    "An idempotency key is required.",
  "error.reason.INTERFACE_IMPORT_LIBRARY_FACET_MISMATCH":
    "The selected library has a different media type.",
  "error.reason.INTERFACE_IMPORT_MAPPING_DUPLICATE":
    "Each source root can only be mapped once.",
  "error.reason.INTERFACE_IMPORT_MAPPING_FACET_MISMATCH":
    "The mapping has an incompatible media type.",
  "error.reason.INTERFACE_IMPORT_MAPPING_INCOMPLETE":
    "Map every source root before continuing.",
  "error.reason.INTERFACE_IMPORT_MAPPING_REQUIRED":
    "Connect a source or add a mapping.",
  "error.reason.INTERFACE_IMPORT_MAPPING_ROOT_UNKNOWN":
    "Reconnect to discover the source root before mapping it.",
  "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_INCOMPLETE":
    "The source mapping requires a source key and type.",
  "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_MISMATCH":
    "The mapping does not match the connected source.",
  "error.reason.INTERFACE_IMPORT_MAPPING_SOURCE_UNSELECTED":
    "The mapping refers to a source that was not selected.",
  "error.reason.INTERFACE_IMPORT_PROWLARR_SOURCE_CONFLICT":
    "Choose one Prowlarr source.",
  "error.reason.INTERFACE_IMPORT_ROOT_LIBRARY_MISMATCH":
    "Choose a root belonging to the selected library.",
  "error.reason.INTERFACE_IMPORT_SOURCE_NOT_READY":
    "Wait for source discovery to finish before continuing.",
  "error.reason.INTERFACE_IMPORT_SOURCE_REQUIRED":
    "Connect at least one Arr or Prowlarr source.",
  "error.reason.INTERFACE_IMPORT_URL_REQUIRED":
    "Enter the Arr instance base URL.",
  "error.reason.INTERFACE_INDEXER_ERROR_LIMIT_INVALID":
    "Choose between 1 and 100 indexer errors.",
  "error.reason.INTERFACE_INTERACTIVE_SESSION_REQUIRED":
    "Sign in with an interactive session to continue.",
  "error.reason.INTERFACE_LIST_EPISODE_POLICY_INVALID":
    "Choose one episode policy for a series or anime list.",
  "error.reason.INTERFACE_LIST_EXCLUSIONS_INVALID":
    "Choose valid canonical exclusions.",
  "error.reason.INTERFACE_LIST_EXCLUSION_TARGET_REQUIRED":
    "Choose the list to exclude.",
  "error.reason.INTERFACE_LIST_FACET_REQUIRED":
    "Choose a media type for this list filter.",
  "error.reason.INTERFACE_LIST_LANGUAGE_UNSUPPORTED":
    "Choose a supported original language.",
  "error.reason.INTERFACE_LIST_NOT_PUBLIC":
    "Only public lists can be followed here.",
  "error.reason.INTERFACE_LIST_RATING_LIMIT_EXCEEDED":
    "Reduce the number of minimum rating filters.",
  "error.reason.INTERFACE_LIST_RATING_OUT_OF_RANGE":
    "Choose a minimum rating within the source scale.",
  "error.reason.INTERFACE_LIST_RATING_SCALE_REQUIRED":
    "Choose a scale for the rating filter.",
  "error.reason.INTERFACE_LIST_RATING_SOURCE_UNSUPPORTED":
    "Choose a supported rating source.",
  "error.reason.INTERFACE_LIST_RATING_VALUE_REQUIRED":
    "Enter a value for the rating filter.",
  "error.reason.INTERFACE_LOCATION_DESTINATION_INVALID":
    "Choose a destination path or root, not both.",
  "error.reason.INTERFACE_LOCATION_TARGET_INVALID":
    "Choose either titles or a library root.",
  "error.reason.INTERFACE_LOGIN_MFA_CONFLICT":
    "Choose consistent MFA requirements for this login method.",
  "error.reason.INTERFACE_LOGIN_MFA_REQUIRED":
    "Choose an MFA requirement for Jellyfin login.",
  "error.reason.INTERFACE_MAINTENANCE_RULE_NOT_FOUND":
    "The maintenance rule could not be found.",
  "error.reason.INTERFACE_METADATA_DATE_INVALID":
    "Enter a valid metadata date.",
  "error.reason.INTERFACE_METADATA_ID_INVALID":
    "Enter a valid metadata identifier.",
  "error.reason.INTERFACE_NOTIFICATION_CHANNEL_NOT_FOUND":
    "The notification channel could not be found.",
  "error.reason.INTERFACE_PROXY_PROTOCOL_UNSUPPORTED":
    "Choose a supported proxy protocol.",
  "error.reason.INTERFACE_PROXY_PROVIDER_UNSUPPORTED":
    "Choose a supported proxy provider.",
  "error.reason.INTERFACE_PROXY_TIMEOUT_INVALID":
    "The request timeout must be positive.",
  "error.reason.INTERFACE_QUALITY_PROFILE_ID_REQUIRED":
    "Enter a quality profile identifier.",
  "error.reason.INTERFACE_QUALITY_PROFILE_NAME_REQUIRED":
    "Enter a quality profile name.",
  "error.reason.INTERFACE_QUALITY_PROFILE_TIERS_REQUIRED":
    "Add at least one quality tier.",
  "error.reason.INTERFACE_QUALITY_PROFILE_VALUE_INVALID":
    "Enter a valid quality profile value.",
  "error.reason.INTERFACE_RELEASE_EXTRA_INVALID":
    "Release extra data must be a JSON object.",
  "error.reason.INTERFACE_RELEASE_SEARCH_EPISODE_INCOMPLETE":
    "Choose both a season and episode.",
  "error.reason.INTERFACE_RELEASE_SEARCH_MOVIE_SCOPE_INVALID":
    "A series movie search cannot include a season or episode.",
  "error.reason.INTERFACE_RELEASE_SEARCH_TITLE_REQUIRED":
    "Choose a title to search for releases.",
  "error.reason.INTERFACE_REQUEST_LEASE_CONFLICT":
    "Choose a lease duration or an unlimited lease, not both.",
  "error.reason.INTERFACE_RESERVED_TAG":
    "Use title options to change reserved tags.",
  "error.reason.INTERFACE_RESTORE_SETUP_REQUIRED":
    "Restore is only available before setup is complete.",
  "error.reason.INTERFACE_RESTORE_UNAVAILABLE":
    "Restore is not available for this installation.",
  "error.reason.INTERFACE_RESTORE_UPLOAD_ID_INVALID":
    "Inspect the restore bundle again before continuing.",
  "error.reason.INTERFACE_RESTORE_UPLOAD_INVALID":
    "The uploaded restore bundle is invalid.",
  "error.reason.INTERFACE_RESTORE_UPLOAD_NOT_FOUND":
    "The restore upload could not be found. Inspect the bundle again.",
  "error.reason.INTERFACE_ROOT_FOLDER_REQUIRED": "Choose a root folder.",
  "error.reason.INTERFACE_RULE_EXCLUSION_SUBJECT_REQUIRED":
    "Choose a subject for episode or season exclusions.",
  "error.reason.INTERFACE_RULE_PREVIEW_TARGET_CONFLICT":
    "Choose a saved rule or an unsaved matcher, not both.",
  "error.reason.INTERFACE_RULE_PREVIEW_TARGET_REQUIRED":
    "Choose a saved rule or provide a draft with its action.",
  "error.reason.INTERFACE_RULE_SCOPE_CONFLICT":
    "Choose titles or a library, not both.",
  "error.reason.INTERFACE_RULE_SCOPE_REQUIRED":
    "Choose titles or a library to preview.",
  "error.reason.INTERFACE_RULE_SOURCE_REQUIRED":
    "Enter rule source to preview this matcher.",
  "error.reason.INTERFACE_RULE_SUBJECT_SELECTION_INVALID":
    "Select titles when selecting subjects, without a library selection.",
  "error.reason.INTERFACE_SCRIPT_ACKNOWLEDGEMENT_REQUIRED":
    "Acknowledge running the inline shell script.",
  "error.reason.INTERFACE_SCRIPT_NOT_FOUND": "The script could not be found.",
  "error.reason.INTERFACE_SCRIPT_PATH_NOT_ABSOLUTE":
    "The script path must be absolute.",
  "error.reason.INTERFACE_SCRIPT_PATH_REQUIRED": "Enter a script file path.",
  "error.reason.INTERFACE_SCRIPT_SCHEDULE_INCOMPLETE":
    "Complete the fields required for this schedule.",
  "error.reason.INTERFACE_SCRIPT_TRIGGER_IMMUTABLE":
    "Create a new script to use a different trigger.",
  "error.reason.INTERFACE_SCRIPT_TYPE_INVALID":
    "Choose a supported post-processing script type.",
  "error.reason.INTERFACE_SUBTITLE_DELETE_CONFIRMATION_REQUIRED":
    "Preview and confirm the subtitle files before deleting them.",
  "error.reason.INTERFACE_TAG_NOT_FOUND": "The title tag could not be found.",
  "error.reason.INTERFACE_TIMESTAMP_INVALID": "Enter a valid timestamp.",
  "error.reason.INTERFACE_UPGRADE_INELIGIBLE":
    "This installation is not eligible for the requested upgrade.",

  "error.libraryScanAlreadyRunning": "A library scan is already running.",
  "error.externalImportSessionGone":
    "This import session has expired. Reconnect to continue.",
  "error.tagAlreadyExists": "A tag with this name already exists.",
  "error.ruleTargetsPerson":
    "Managing permissions is required to use requester fields in a rule.",
  "error.rulePackUpToDate": "This rule pack is already up to date.",
  "error.maintenanceArmCountMismatch":
    "The candidate count changed. Review and acknowledge the current count again.",
  "error.mfaStepUpRequired":
    "Settings verification expired. Enter an authenticator code to continue.",
  "error.reauthenticationRequired": "Verify your identity to continue.",
  "error.rateLimited": "Too many requests. Try again shortly.",
  "error.authenticationRequired": "Sign in to continue.",
  "error.internal": "Internal server error",
  "error.internalReference": "Internal server error. Reference ID: {{id}}",
  "error.disconnected": "Disconnected",

  "httpRelease.refusal.detailsUnavailable":
    "This release's details are no longer available. Search again to grab it.",
  "httpRelease.refusal.live":
    "This release is a live stream, which cannot be downloaded.",
  "httpRelease.refusal.upcoming": "This release has not been published yet.",
  "httpRelease.refusal.removed":
    "This release has been removed by its provider.",
  "httpRelease.refusal.unsupportedTransport":
    "This release is not offered in a form the HTTP downloader can fetch.",
  "httpRelease.refusal.separateTracks":
    "This release is offered only as separate audio and video tracks, which cannot be combined.",
  "httpRelease.refusal.noMediaResource":
    "This release has no media file to download.",
  "httpRelease.refusal.noSubtitleResource":
    "No subtitles in a wanted language are offered for this release.",
  "httpRelease.refusal.noAudioResource":
    "No media resource satisfies the required audio languages.",
  "httpRelease.refusal.unsupportedAuth":
    "This resource needs provider sign-in that the HTTP downloader cannot supply.",
  "httpRelease.refusal.unknownContainer":
    "The downloaded payload could not be identified as a supported video container. The partial file was preserved.",
  "httpRelease.refusal.destinationExists":
    "The destination already exists. Existing files were left untouched.",
  "httpRelease.refusal.publicationFailed":
    "The download could not be published safely on this filesystem. The partial file was preserved.",
  "httpRelease.refusal.recoveryUnverified":
    "The existing output could not be verified against this download. It was preserved for inspection.",
  "palette.title": "Color palette",
  "palette.description":
    "Choose an appearance for your account on every device.",
  "palette.classic": "Classic",
  "palette.scryerDefault": "Scryer Default",
  "palette.slate": "Slate",
  "palette.ocean": "Ocean",
  "palette.forest": "Forest",
  "palette.plum": "Plum",
  "palette.ember": "Ember",
  "palette.graphite": "Graphite",
  "palette.papyrus": "Papyrus",
  "palette.classicMode": "Classic appearance",
  "palette.choose": "Choose color palette",
  "palette.useAccent": "Use palette accent",
  "palette.saveFailed":
    "Could not save your appearance. Your previous selection has been restored.",
  "titleDelete.refusal.selection": "Select titles without duplicates.",
  "titleDelete.refusal.busy": "A deletion is already running.",
  "titleDelete.refusal.preview": "Refresh the deletion preview and confirm it.",
  "titleDelete.refusal.confirmation": "Type DELETE to confirm deletion.",
  "titleDelete.refusal.boundary":
    "The title folder must be inside a library root, not the root itself.",
  "titleDelete.refusal.ownership":
    "Folder ownership could not be verified. Files were preserved.",
  "titleDelete.refusal.layout":
    "The folder or its contents are not safe to delete as one title.",
  "titleDelete.refusal.unavailable":
    "A library root is unavailable. Restore access and preview again.",
  "titleDelete.refusal.outsideRoots":
    "The file path is outside every configured library root. Fix the library roots or the title folder and preview again.",
  "queue.attention.downloadPasswordRetry":
    "A download password retry is awaiting reconciliation. Remove this download if the client no longer has the job.",
  "titleDelete.refusal.recycle":
    "The title was deleted, but some recycle entries could not be purged.",
  "titleDelete.refusal.retry":
    "Catalog cleanup failed. Get a fresh preview before retrying.",
  "settings.downloadClientHttpDownloadDir": "Download folder",
  "settings.downloadClientHttpDownloadDirHelp":
    "Where finished files and their partials are written, as seen by Scryer. Scryer keeps this downloader's queue in a .scryer-http folder inside it.",
  "settings.downloadClientHttpMaxConcurrentDownloads": "Concurrent downloads",
  "settings.downloadClientHttpMaxConcurrentDownloadsHelp":
    "How many releases download at once (1 to 8).",
  "settings.downloadClientHttpMaxConnectionsPerHost": "Connections per host",
  "settings.downloadClientHttpMaxConnectionsPerHostHelp":
    "How many downloads may fetch from the same server at once (1 to 8).",
  "settings.downloadClientHttpBandwidthLimit": "Bandwidth limit (KB/s)",
  "settings.downloadClientHttpBandwidthLimitHelp":
    "Shared by all downloads of this client. Empty or 0 means unlimited.",
  "settings.publicUrlTitle": "Public URL",
  "settings.publicUrlHelp":
    "The address people and services use to reach Scryer, such as https://scryer.example.com. Account linking and OAuth use a change immediately; passkeys use it after Scryer restarts.",
  "settings.publicUrlLabel": "Public URL",
  "settings.publicUrlEnvironmentLocked":
    "Set by SCRYER_PUBLIC_URL. Change the environment variable to edit it.",
  "settings.publicUrlInvalid":
    "The configured public URL is invalid and is not used: {{error}}",
  "settings.publicUrlError.invalid_url":
    "Enter an absolute http or https URL, such as https://scryer.example.com.",
  "settings.publicUrlError.wildcard_host":
    "The public URL must name a single host, not a wildcard.",
  "settings.publicUrlError.credentials":
    "The public URL must not include a username or password.",
  "settings.publicUrlError.query_or_fragment":
    "The public URL must not include a query string or fragment.",
  "settings.publicUrlError.path_not_allowed":
    "Scryer is served at the root, so the public URL must not include a path.",
  "settings.publicUrlError.path_mismatch":
    "The public URL path must be empty or match the base path Scryer is served under.",
  "settings.publicUrlError.environment_locked":
    "The public URL is set by SCRYER_PUBLIC_URL and cannot be changed here.",
  "settings.publicUrlError.save_and_reset":
    "Save a public URL or clear it, not both.",
  "settings.publicUrlError.passkey_acknowledgement_required":
    "This change stops registered passkeys from working. Confirm it to save.",
  "settings.publicUrlPasskeyRestart":
    "Passkeys pick up a public URL change after Scryer restarts.",
  "settings.publicUrlClear": "Clear public URL",
  "settings.publicUrlSaved": "Public URL saved.",
  "settings.publicUrlLoadError": "Unable to load the public URL.",
  "settings.publicUrlSaveError": "Unable to save the public URL.",
  "settings.publicUrlEffectiveValues": "Effective values",
  "settings.publicUrlColumnSetting": "Setting",
  "settings.publicUrlColumnValue": "Value",
  "settings.publicUrlColumnSource": "Source",
  "settings.publicUrlEffective": "Public URL",
  "settings.publicUrlBasePath": "Base path",
  "settings.publicUrlBindAddress": "Bind address",
  "settings.publicUrlTrustedProxies": "Trusted proxies",
  "settings.publicUrlPasskeyRpId": "Passkey relying party ID",
  "settings.publicUrlPasskeyRpOrigin": "Passkey origin",
  "settings.publicUrlNotSet": "Not set",
  "settings.publicUrlSourceEnvironment": "Environment",
  "settings.publicUrlSourceSettings": "Saved settings",
  "settings.publicUrlSourceDefault": "Default",
  "settings.publicUrlSourcePublicUrl": "Public URL",
  "settings.publicUrlPasskeyWarningTitle":
    "Existing passkeys will stop working",
  "settings.publicUrlPasskeyWarningChanged":
    "Passkeys are registered for {{rpId}}. After the next restart, passkeys move to {{nextRpId}} and every existing passkey stops signing in.",
  "settings.publicUrlPasskeyWarningDisabled":
    "Passkeys are registered for {{rpId}}. This public URL cannot carry passkeys (they need an https domain name, or localhost), so after the next restart passkeys turn off and every existing passkey stops signing in.",
  "settings.publicUrlPasskeyWarningCounts":
    "Users with passkeys: {{users}}. Without an authenticator app as a second factor: {{passkeyOnly}}. Those users will be locked out: they cannot sign in at all until their factors are reset.",
  "settings.publicUrlPasskeyWarningCountsUnknown":
    "Scryer could not count registered passkeys; assume some users will be locked out.",
  "settings.publicUrlPasskeyWarningRecovery":
    "To restore access, an administrator who can still sign in can open {{users}} and use {{reset}} for each locked-out user; or revert the public URL and restart Scryer; or start Scryer with SCRYER_RECOVERY_ADMIN_PASSWORD to sign in as the recovery administrator.",
  "settings.publicUrlPasskeyWarningConfirm": "Save anyway",
  "settings.pluginSettings": "Plugin settings",
  "settings.pluginSettingsLoadFailed": "Could not load plugin settings.",
  "settings.pluginSettingsLoading": "Loading plugin settings…",
  "settings.additionalArchivePasswords": "Additional passwords",
  "settings.additionalArchivePasswordsHelp":
    "One password per line. Tried only when an archive requires a password.",
  "settings.pluginSecretSaved":
    "A value is saved. Enter a replacement or clear it.",
  "settings.showEnteredPasswords": "Show entered passwords",
  "settings.hideEnteredPasswords": "Hide entered passwords",
  "settings.pluginSettingsSaveFailed": "Could not save plugin settings.",
  "settings.regoValidationUnavailable":
    "Validation unavailable. Check your connection and try Validate again.",
  "settings.regoValidationFailed": "Rule validation failed. Show errors.",
  "settings.regoFileAvailability":
    "Available only when scoring an imported file with media information. During release searches input.file is null; guard it before reading fields.",
  "settings.regoExtraAvailability":
    "Indexer-provided attributes. Keys vary by indexer and are not suggested.",
  "settings.regoSnippetLabel": "Snippet",
  "settings.regoHelper.blockScore":
    "Return the standard blocking penalty (-10000). Other scoring entries can still offset this penalty.",
  "settings.regoHelper.sizeGib": "Convert a byte count to gibibytes (GiB).",
  "settings.regoHelper.langMatches":
    "Compare a language code with another code or a supported language alias.",
  "settings.regoHelper.normalizeSource":
    "Normalize a source label, such as webdl to WEB-DL.",
  "settings.regoHelper.normalizeCodec":
    "Normalize a codec label, such as x265 to H.265.",
  "settings.regoHelper.lower": "Convert a string to lowercase.",
  "settings.regoHelper.upper": "Convert a string to uppercase.",
  "settings.regoHelper.count":
    "Count items in an array, set or object, or characters in a string.",
  "settings.regoHelper.objectGet":
    "Read an object key, returning the default when absent. Use a named input fact in request and maintenance rules.",
  "settings.regoHelper.startsWith":
    "Return whether a string begins with a prefix.",
  "settings.regoSnippet.scoreEntry":
    "Add a named scoring adjustment when a release condition matches.",
  "settings.regoSnippet.tagScore":
    "Add a scoring adjustment for a title tag. Replace the tag and score placeholders.",
  "settings.regoSnippet.nullableField":
    "Check for a release group before comparing its lowercase name.",
  "settings.regoSnippet.denyRequest":
    "Deny a request when its adult-content fact is true.",
  "settings.regoSnippet.manualRequest":
    "Require manual review when the recent approval count reaches a threshold.",
  "settings.regoSnippet.matchTag":
    "Match a maintenance subject by tag. This matcher does not define an action.",

  "settings.trustedProxiesTitle": "Trusted proxies for rate limiting",
  "settings.trustedProxiesHelp":
    "Trust X-Forwarded-For only from these proxy addresses when identifying clients for rate limiting. This does not change login rules, local access, or rate-limit bypasses. Changes apply immediately.",
  "settings.trustedProxiesAddresses":
    "IPv4, IPv6, or CIDR addresses (one per line or comma-separated)",
  "settings.trustedProxiesEmptyHelp":
    "Saving an empty list trusts no proxies. Saved settings override SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS.",
  "settings.trustedProxiesSavedSource": "Source: saved settings",
  "settings.trustedProxiesEnvironmentSource":
    "Source: environment (defaults to no trusted proxies)",
  "settings.trustedProxiesReset": "Use environment/default",
  "settings.trustedProxiesSaved": "Trusted proxy policy applied.",
  "settings.trustedProxiesLoadError": "Unable to load trusted proxy settings.",
  "settings.trustedProxiesSaveError": "Unable to save trusted proxy settings.",
  "mediaInfo.discIntactImport":
    "The disc image is imported as one file. Map each authored title explicitly for episodes.",
  "mediaInfo.discReviewRequired": "Disc analysis needs review",
  "mediaInfo.discImportMovie": "Import movie",
  "mediaInfo.discSkip": "Skip (unassigned)",
  "mediaInfo.discAutomaticChoice": "Automatic longest title",
  "mediaInfo.discNoTitles":
    "No supported disc titles were found. The source will remain in place.",
  "scoring.mandatoryRejection": "Mandatory requirement failed",
  "scoring.finalScoreRejection": "Final score rejected",

  "label.ready": "Ready",
  "label.moreInfo": "More information",
  "label.search": "Search",
  "label.autoSearch": "Auto Search",
  "label.interactiveSearch": "Interactive Search",
  "label.interactive": "Interactive",
  "label.searching": "Searching",
  "help.autoSearchTooltip":
    "Auto search and queue the latest matching release for this item.",
  "help.interactiveSearchTooltip":
    "Search now for this item and open the release picker to choose what to queue.",
  "series.searchSeason": "Search for season pack",
  "help.seasonSearchTooltip":
    "Search indexers for a season pack release for this season.",
  "series.interactiveSearchSeason": "Search season",
  "help.seasonInteractiveSearchTooltip":
    "Search now for this whole season and choose which release to queue.",
  "label.refresh": "Refresh",
  "label.refreshing": "Refreshing…",
  "label.retry": "Retry",
  "label.save": "Save",
  "label.saving": "Saving…",
  "label.loading": "Loading…",
  "label.moveUp": "Move up",
  "label.moveDown": "Move down",
  "label.update": "Update",
  "label.name": "Name",
  "label.load": "Load",
  "label.edit": "Edit",
  "label.delete": "Delete",
  "label.account": "Account",
  "label.actions": "Actions",
  "label.advanced": "Advanced",
  "label.custom": "Custom",
  "label.create": "Create",
  "label.cancel": "Cancel",
  "folderBrowser.selectPath": "Select path",
  "folderBrowser.go": "Go",
  "folderBrowser.descriptionFileOrFolder":
    "Browse paths on the Scryer host and select a file or folder.",
  "folderBrowser.descriptionFile":
    "Browse paths on the Scryer host and select a file.",
  "folderBrowser.descriptionFolder":
    "Browse paths on the Scryer host and select a folder.",
  "folderBrowser.loading": "Loading folders",
  "folderBrowser.emptyFilesAndFolders": "No files or subdirectories",
  "folderBrowser.emptyFolders": "No subdirectories",
  "folderBrowser.error": "Unable to browse path.",
  "folderBrowser.browseRoot": "Browse /",
  "folderBrowser.selectFolder": "Select folder",
  "folderBrowser.newFolder": "New folder",
  "folderBrowser.newFolderName": "Folder name",
  "folderBrowser.create": "Create",
  "folderBrowser.newFolderFailed": "Couldn't create the folder.",
  "label.discard": "Discard changes",
  "label.clear": "Clear",
  "label.add": "Add",
  "label.remove": "Remove",
  "label.yes": "Yes",
  "label.no": "No",
  "label.filter": "Filter",
  "label.filterOptions": "Filter options",
  "label.noMatchingOptions": "No matching options",
  "label.filters": "Filters",
  "label.enabled": "Enabled",
  "label.disabled": "Disabled",
  "unit.seconds": "seconds",
  "label.enable": "Enable",
  "label.disable": "Disable",
  "label.version": "Version",
  "label.type": "Type",
  "label.title": "Title",
  "label.status": "Status",
  "label.duration": "Duration",
  "label.output": "Output",
  "label.testConnection": "Test Connection",
  "external.openOn": "Open on {{site}}",
  "label.unknown": "Unknown",
  "label.multiEpisodePack": "Multi-episode pack",
  "label.multiEpisodePackHint":
    "This release covers more than the episode you searched for.",
  "label.unchanged": "Unchanged",
  "label.deleting": "Deleting…",
  "label.default": "default",
  "label.language": "Language",
  "label.movies": "Movies",
  "label.series": "Series",
  "label.anime": "Anime",
  "label.unknownType": "unknown type",
  "label.yearUnknown": "year unknown",
  "label.noArt": "no art",
  "label.none": "None",

  "nav.dashboard": "Dashboard",
  "nav.movies": "Movies",
  "nav.series": "Series",
  "nav.anime": "Anime",
  "nav.discovery": "Discovery",
  "nav.activity": "Activity",
  "nav.calendar": "Calendar",
  "nav.settings": "Settings",
  "nav.library": "Library",
  "nav.catalog": "Catalog",
  "nav.import": "Import",
  "nav.logs": "Logs",
  "nav.serviceLogs": "Service Logs",
  "nav.auditLogs": "Audit Logs",
  "nav.usersAccess": "Users & Access",
  "nav.mobileTitle": "Scryer navigation",
  "nav.mobileDescription": "Primary Scryer navigation",
  "nav.mobileTrigger": "Open navigation",
  "nav.group.overview": "Overview",
  "nav.group.catalogs": "Catalogs",
  "nav.group.discover": "Discover",
  "nav.group.requests": "Requests",
  "nav.group.automation": "Automation",
  "nav.group.integrations": "Integrations",
  "nav.group.system": "System",
  "nav.group.more": "More",

  "dashboard.commandDescription": "Operations overview",
  "dashboard.greetingMorning": "Good morning, {{name}}",
  "dashboard.greetingAfternoon": "Good afternoon, {{name}}",
  "dashboard.greetingEvening": "Good evening, {{name}}",
  "dashboard.attentionCount": "{{count}} items need attention",
  "dashboard.attentionCountOne": "1 item needs attention",
  "dashboard.allClear": "Everything is running clean.",
  "dashboard.sourceRequests": "requests",
  "dashboard.sourceImports": "imports",
  "dashboard.sourcePlugins": "plugins",
  "dashboard.sourceIndexers": "indexers",
  "dashboard.pluginUpdateCount": "{{count}} plugin updates",
  "dashboard.pluginUpdateCountOne": "1 plugin update",
  "dashboard.pluginBreaking": "Breaking",
  "dashboard.pluginUpdateAll": "Update all",
  "dashboard.pluginManage": "Manage",
  "dashboard.statLibrary": "Library",
  "dashboard.statWindow": "24h",
  "dashboard.metricMovies": "movies",
  "dashboard.metricSeries": "series",
  "dashboard.metricAnime": "anime",
  "dashboard.metricQueued": "queued",
  "dashboard.metricUpgrades": "upgrades",
  "dashboard.metricImported": "imported",
  "dashboard.metricFailed": "failed",
  "dashboard.deltaHint": "Change from the previous 24 hours",
  "dashboard.manualImports": "Manual Imports",
  "dashboard.recentlyImported": "Recently Imported",
  "dashboard.storage": "Storage",
  "dashboard.activeQueue": "Active Queue",
  "dashboard.viewAll": "All",
  "dashboard.approveRequest": "Approve {{name}}",
  "dashboard.dismissRequest": "Dismiss {{name}}",
  "dashboard.upgradeBadge": "Upgrade",
  "dashboard.newImportBadge": "New import",
  "dashboard.emptyRequests": "No requests are waiting.",
  "dashboard.emptyImports": "No imports need attention.",
  "dashboard.emptyImported": "Nothing has been imported yet.",
  "dashboard.emptyIndexers": "No indexers are configured.",
  "dashboard.emptyClients": "No download clients are configured.",
  "dashboard.emptyStorage": "No root folders are configured.",
  "dashboard.emptyQueue": "Nothing is downloading.",
  "dashboard.columnIndexer": "Indexer",
  "dashboard.columnSearch": "Search",
  "dashboard.columnGrab": "Grab",
  "dashboard.columnFail": "Fail",
  "dashboard.columnQuota": "Quota",
  "dashboard.columnStatus": "Status",
  "dashboard.columnClient": "Client",
  "dashboard.columnActive": "Active",
  "dashboard.columnQueue": "Queue",
  "dashboard.healthCount": "{{healthy}}/{{total}} healthy",
  "dashboard.erroringCount": "{{count}} erroring",
  "dashboard.erroringCountOne": "1 erroring",
  "dashboard.downCount": "{{count}} down",
  "dashboard.downCountOne": "1 down",
  "dashboard.unmetered": "Unmetered",
  "dashboard.statusOk": "OK",
  "dashboard.statusError": "Error",
  "dashboard.rootCount": "{{count}} roots",
  "dashboard.rootCountOne": "1 root",
  "dashboard.storageFree": "{{size}} free",
  "dashboard.storageUnavailable": "Usage unavailable",
  "dashboard.storageUsage": "{{used}}/{{total}} TB",
  "dashboard.storageLegend": "OK · 65%+ · 80%+ · 90%+",
  "dashboard.tagLow": "LOW",
  "dashboard.tagCrit": "CRIT",
  "dashboard.queueCount": "{{count}} active",
  "dashboard.queueCountOne": "1 active",
  "dashboard.columnType": "Type",
  "dashboard.requestedBy": "Requested by {{names}}",
  "dashboard.panel.stats": "At a glance",
  "dashboard.panel.statsDescription":
    "Library size and the last 24 hours of grabs and imports.",
  "dashboard.panel.requestsDescription":
    "Requests waiting for approval, oldest first.",
  "dashboard.panel.manualImportsDescription":
    "Downloads that could not import on their own.",
  "dashboard.panel.recentlyImportedDescription":
    "The latest titles and episodes added to the library.",
  "dashboard.panel.indexersDescription":
    "Indexer health, searches, grabs and API quota.",
  "dashboard.panel.downloadClientsDescription":
    "Download client health and live activity.",
  "dashboard.panel.storageDescription": "Space used on each root folder.",
  "dashboard.panel.activeQueueDescription":
    "What is downloading now and how long is left.",
  "dashboard.layout.edit": "Edit dashboard",
  "dashboard.layout.editing": "Editing dashboard",
  "dashboard.layout.editHint":
    "Drag panels to move them and drag an edge or corner to resize.",
  "dashboard.layout.editHintStacked": "Drag panels to reorder them.",
  "dashboard.layout.panels": "Panels",
  "dashboard.layout.reset": "Reset to default",
  "dashboard.layout.resetConfirmTitle": "Reset the dashboard?",
  "dashboard.layout.resetConfirmDescription":
    "Every screen size goes back to the default layout. Your saved arrangement is removed.",
  "dashboard.layout.unsavedTitle": "Unsaved dashboard changes",
  "dashboard.layout.unsavedDescription":
    "You have rearranged the dashboard without saving. Leave and discard those changes?",
  "dashboard.layout.trayTitle": "Dashboard panels",
  "dashboard.layout.trayDescription":
    "Drag a panel onto the dashboard, or add it to the end.",
  "dashboard.layout.placed": "On dashboard",
  "dashboard.layout.addPanel": "Add {{name}}",
  "dashboard.layout.dragPanel": "Drag {{name}} onto the dashboard",
  "dashboard.layout.movePanel": "Move {{name}}",
  "dashboard.layout.removePanel": "Remove {{name}}",
  "dashboard.layout.empty":
    "No panels are on the dashboard. Open Panels to add some.",
  "dashboard.layout.saved": "Dashboard layout saved.",
  "dashboard.layout.saveFailed": "The dashboard layout could not be saved.",
  "dashboard.layout.resetDone": "Dashboard reset to the default layout.",
  "dashboard.layout.resetFailed": "The dashboard layout could not be reset.",
  "dashboard.layout.refusal.columnsOutOfRange":
    "This layout uses a column count the server does not accept.",
  "dashboard.layout.refusal.tooManyPanels":
    "This layout has more panels than the server allows.",
  "dashboard.layout.refusal.panelIdInvalid":
    "This layout names a panel the server does not recognize.",
  "dashboard.layout.refusal.panelIdDuplicate":
    "This layout places the same panel twice.",
  "dashboard.layout.refusal.outsideGrid":
    "A panel in this layout sits outside the grid.",
  "dashboard.layout.refusal.sizeInvalid":
    "A panel in this layout has no width or height.",

  "discovery.failedToLoad": "Failed to load discovery",
  "discovery.featured": "Featured",
  "discovery.add": "Add",
  "discovery.request": "Request",
  "discovery.requested": "Requested",
  "discovery.openFilters": "Open filters",
  "discovery.closeFilters": "Close filters",
  "discovery.filters": "Filters",
  "discovery.contentType": "Content Type",
  "discovery.type.movies": "Movies",
  "discovery.type.series": "Series",
  "discovery.type.anime": "Anime",
  "discovery.genres": "Genres",
  "discovery.selectGenres": "Select genres...",
  "discovery.tags": "Tags",
  "discovery.selectTags": "Select tags...",
  "discovery.emptyTitle": "No discovery items yet",
  "discovery.emptyDescription":
    "Discovery feeds will appear here once the sync has data.",
  "discovery.clearAll": "Clear all",
  "discovery.releaseYear": "Release Year",
  "discovery.minimumRating": "Minimum Rating",
  "discovery.studio": "Studio",
  "discovery.section.newOnStreaming": "New to Streaming",
  "discovery.section.newOnPhysical": "New on Blu-ray",
  "discovery.section.allTimeFavorites": "All-Time Favorites",
  "discovery.section.rightNow": "Right Now",
  "discovery.section.fromYourTaste": "From Your Taste",
  "discovery.section.almostComplete": "Almost Complete",
  "discovery.section.nextAnimeSeason": "Next Anime Season",
  "discovery.section.popularInAnime": "Popular in Anime",
  "discovery.section.thisSeasonInAnime": "This Season in Anime",
  "discovery.section.comingSoonMovies": "Coming Soon: Movies",
  "discovery.section.comingSoonSeries": "Coming Soon: Series",
  "discovery.section.topRatedForYou": "Top Rated for You",
  "discovery.section.topRated": "Top Rated",
  "discovery.section.recommended": "Recommended",
  "discovery.notInterested": "Not interested",
  "discovery.hiddenTitles": "Hidden titles",
  "discovery.showHidden": "Show hidden",
  "discovery.updatePending": "Update pending",
  "discovery.updatePendingHint":
    "New library changes will appear after the next sync.",

  "settings.profile": "Profile",
  "settings.general": "General",
  "settings.backups": "Backups",
  "settings.security": "Security",
  "settings.enableFormLogin": "Enable login",
  "settings.securityLoginSettings": "Login settings",
  "settings.securityEnableFormLoginHelp":
    "When enabled, Scryer requires an authenticated user session. When disabled, requests resolve as the default admin user.",
  "settings.securityPasswordMinLength": "Minimum password length",
  "settings.securitySessionDuration": "Login valid for",
  "settings.securitySessionDurationHelp":
    "Applies to new sign-ins for everyone on this instance. With Keep me signed in enabled, sessions survive browser restarts until they expire. Existing sessions keep their original expiry.",
  "settings.securitySessionDurationInvalid":
    "Session duration must be a whole number from 1 to 365 days.",
  "settings.securityPasswordMinLengthHelp":
    "Applies to new or changed Scryer passwords only. Existing shorter passwords keep working. Minimum: {{min}}.",
  "settings.securityPasswordMinLengthInvalid":
    "Minimum password length must be at least {{min}}.",
  "settings.openMediaServers": "Open Media Servers",
  "settings.securitySkipLocalIps": "Skip for local IPs",
  "settings.securitySkipLocalIpsHelp":
    "When enabled, callers from 10.0.0.0/8, 172.16.0.0/12, or 192.168.0.0/16 can access Scryer without signing in.",
  "settings.securityMfaConfigStepUp":
    "Require MFA step-up for configuration changes",
  "settings.securityMfaConfigStepUpHelp":
    "When enabled, system and facet configuration changes require a recent MFA verification.",
  "settings.securityMfaPasswordLogin": "Require MFA for password login",
  "settings.securityMfaPasswordLoginHelp":
    "When enabled, Scryer username and password sign-in requires a one-time code. Passkeys are unchanged.",
  "settings.securityTotpJellyfinLogin": "Require MFA for Jellyfin login",
  "settings.securityTotpJellyfinLoginHelp":
    "When enabled, Jellyfin password passthrough sign-in requires the user's enrolled Scryer passkey or authenticator factor.",
  "settings.securitySavedPreference": "Saved preference",
  "settings.securityEffectiveMode": "Effective runtime mode",
  "settings.securityOverrideTitle": "Deployment override active",
  "settings.securityOverrideDescription":
    "Environment configuration is currently overriding the saved preference. Change the deployment config and restart Scryer for a different runtime mode.",
  "settings.securityOverrideReason": "Override source: {{override}}",
  "settings.securityModeEnabled": "Form login enabled",
  "settings.securityModeDisabled": "Open admin mode",
  "settings.securityConfirmTitle": "Enable form login?",
  "settings.securityConfirmDescription":
    "Enter your password before Scryer starts requiring login.",
  "settings.securityConfirmPassword": "Password",
  "settings.securityConfirmAction": "Enable",
  "settings.securitySetAdminPasswordTitle": "Set the admin password",
  "settings.securitySetAdminPasswordDescription":
    "Your account has no password yet. Choose one now and Scryer will enable login with it.",
  "settings.securitySetAdminPasswordAction": "Set password and enable",
  "settings.securitySetAdminPasswordMinLength":
    "At least {{count}} characters.",
  "settings.securityPasswordTooShort":
    "Password must be at least {{count}} characters.",
  "settings.securityDisableConfirmTitle": "Disable form login?",
  "settings.securityDisableConfirmDescription":
    "Scryer will return to open admin mode after reload.",
  "settings.securityDisableConfirmAction": "Disable",
  "settings.securityCredentialsInvalid": "Invalid password.",
  "settings.securityCredentialsInsufficient":
    "That account cannot manage settings.",
  "settings.mfaStepUpTitle": "Verify to open settings",
  "settings.securityAccessControl": "Access control",
  "profile.apiKeyMfaTitle": "Verify to create an API key",
  "profile.apiKeyMfaDescription":
    "Enter an authenticator or recovery code to continue creating your API key.",
  "profile.apiKeyMfaCode": "Authenticator or recovery code",
  "settings.mfaStepUpDescription":
    "Enter an authenticator code before viewing or changing configuration settings.",
  "settings.mfaStepUpSubmit": "Continue",
  "settings.mfaStepUpVerifying": "Verifying...",
  "settings.mfaStepUpVerified": "Settings access verified.",
  "settings.mfaStepUpFailed": "Could not verify that code. Try again.",
  "settings.mfaStepUpRequiredAgain":
    "Your settings verification expired. Enter a new authenticator code to continue.",
  "settings.mfaStepUpPolicyLoadFailed":
    "Settings verification status could not be loaded",
  "settings.mfaStepUpPolicyLoadFailedDescription":
    "Retry before viewing or changing configuration settings.",
  "settings.mfaStepUpPolicyRetry": "Retry",
  "settings.securityEnabledSuccess": "Form login enabled.",
  "settings.securityDisabledSuccess":
    "Form login disabled. Reloading into open admin mode…",
  "settings.securityPreferenceSaved": "Security preference saved.",
  "settings.securityLoadFailed": "Failed to load security settings.",
  "settings.securitySaveFailed": "Failed to save security settings.",
  "settings.externalAuthLoginEnabled": "Login enabled",
  "settings.mediaServerLoginRequiresFormLogin":
    "Enable form login before enabling media-server login.",
  "settings.openSecuritySettings": "Open security settings",
  "settings.externalAuthLinkingEnabled": "Linking enabled",
  "settings.externalAccountInvites": "External account invites",
  "settings.externalAccountInviteRequired":
    "Choose a user and media server account.",
  "settings.externalAccountInviteCreated": "External account invite created.",
  "settings.externalAccountInviteFailed":
    "Failed to create external account invite.",
  "settings.mediaServerUser": "Media server user",
  "settings.mediaServerUserSearchPlaceholder": "Search media server users",
  "settings.mediaServerUserSearchFailed": "Media server user search failed.",
  "settings.mediaServerUserPickerEmpty": "No media server users found.",
  "settings.mediaServerUserPickerServerEmpty": "No users found on this server.",
  "settings.previousExternalAccountInvites": "Previous invites",
  "settings.noExternalAccountInvites": "No external account invites yet.",
  "settings.externalAccountInviteStatusDisabled": "Disabled",
  "settings.externalAccountInviteStatusLoggedIn": "Logged in",
  "settings.externalAccountInviteStatusPending": "Pending",
  "settings.externalAccountInviteStatusActive": "Active",
  "settings.externalAccountInviteProviderUser": "Provider user",
  "settings.externalAccountInviteCreatedAt": "Created",
  "settings.externalAccountInviteLastLogin": "Last login",
  "settings.provider": "Provider",
  "settings.createInvite": "Create invite",
  "settings.users": "Users",
  "settings.indexers": "Indexers",
  "settings.indexerSearch": "Indexer Search",
  "settings.indexerClients": "Client management",
  "indexerClients.endpoints": "Endpoints",
  "indexerClients.endpointCopyFailed": "Could not copy the endpoint URL.",
  "indexerClients.existing": "Indexer clients",
  "indexerClients.none": "No indexer clients yet.",
  "indexerClients.key": "Key",
  "indexerClients.protocol.newznab": "Newznab",
  "indexerClients.protocol.torznab": "Torznab",
  "indexerClients.rotateConfirmTitle": "Rotate the key for {{name}}?",
  "indexerClients.revokeConfirmTitle": "Revoke the key for {{name}}?",
  "indexerClients.mfaTitle": "Verify to change indexer clients",
  "indexerClients.mfaDescription":
    "Enter an authenticator or recovery code to continue changing indexer clients.",
  "indexerClients.secretOnce": "Save this key now. It will not be shown again.",
  "indexerClients.apiKey": "API key",
  "indexerClients.keyActive": "Key active",
  "indexerClients.keyRevoked": "Key revoked",
  "indexerClients.sourceCount": "{{count}} granted indexers",
  "indexerClients.lastUsed": "Last used",
  "indexerClients.never": "Never",
  "indexerClients.outcome.caps": "Capabilities response prepared",
  "indexerClients.outcome.search": "Search response prepared",
  "indexerClients.outcome.partial": "Partial search response prepared",
  "indexerClients.outcome.artifact": "Download fetched",
  "indexerClients.outcome.refused": "Request refused",
  "indexerClients.outcome.unavailable": "Request temporarily unavailable",
  "indexerClients.outcome.failed": "Request failed",
  "indexerClients.outcome.unknown": "Outcome unavailable",
  "indexerClients.edit": "Edit client",
  "indexerClients.rotate": "Rotate key",
  "indexerClients.revoke": "Revoke key",
  "indexerClients.create": "Create client",
  "indexerClients.name": "Client name",
  "indexerClients.protocols": "Allowed protocols",
  "indexerClients.grants": "Granted indexers",
  "indexerClients.allCategories": "Allow all categories on this indexer",
  "indexerClients.grantAll": "Grant every indexer",
  "indexerClients.grantIndexer": "Indexer",
  "indexerClients.granted": "Granted",
  "indexerClients.grantAllCategories": "All categories",
  "indexerClients.enableRequiresKey":
    "Rotate the key before enabling this client.",
  "indexerClients.error.enableRequiresKey":
    "This client's key was revoked. Rotate the key before enabling it.",
  "indexerClients.categories": "Allowed categories",
  "indexerClients.noCategories":
    "No categories selected: this indexer will return no results to the client.",
  "indexerClients.save": "Save client",
  "indexerClients.keyChangeWarning":
    "The current key and its retrieval links will stop working immediately. Update the tool with a new key to restore access.",
  "indexerClients.error.session":
    "Sign in with your account to manage indexer clients. Skipping login for local IPs does not cover client keys.",
  "indexerClients.error.permissions":
    "Every application permission is required to manage indexer clients.",
  "indexerClients.error.key":
    "The client key is invalid, revoked, or disabled.",
  "indexerClients.error.unavailable":
    "Indexer client management is unavailable.",
  "indexerClients.error.missing":
    "This client no longer exists. Refresh the client list.",
  "indexerClients.error.changed":
    "This client changed. Refresh the client list and reopen it before saving.",
  "indexerClients.error.name":
    "Enter a client name of 1 to 120 bytes without control characters.",
  "indexerClients.error.grant":
    "Select existing individual indexers once each, with valid category restrictions.",
  "indexerClients.error.revision":
    "This client's revision capacity is exhausted.",
  "settings.proxies": "Proxies",
  "settings.proxyUsername": "Username",
  "settings.proxyPassword": "Password",
  "settings.proxyCredentialsStored": "Credentials set",
  "settings.proxyCredentialUnchanged": "Unchanged",
  "settings.proxyClearCredentials": "Clear stored credentials",
  "settings.proxyCredentialsHelp":
    "Credentials are stored encrypted and never shown again. Leave both empty if the proxy needs none.",
  "settings.proxyCredentialsStoredHelp":
    "Credentials are stored for this proxy. Leave the fields empty to keep them, or type new values to replace them.",
  "settings.proxyRemoteDns": "Resolve DNS at proxy",
  "settings.proxyRemoteDnsHelp":
    "Resolving DNS at the proxy is the socks5h behaviour: the proxy looks up destination hostnames instead of this machine.",
  "settings.proxiesHelp":
    "Proxies carry traffic for the indexers and download clients you assign them to. A solver answers browser challenges, a standard proxy is an HTTP or SOCKS hop, and a tunnel dials its own transport.",
  "settings.proxyFamilySolver": "Solver",
  "settings.proxyFamilyStandard": "Standard",
  "settings.proxyFamilyTunnel": "Tunnel",
  "settings.proxyFamilyOther": "Other",
  "settings.proxyHealth": "Health",
  "settings.proxyLastError": "Last error",
  "settings.proxyHealthHealthy": "Healthy",
  "settings.proxyHealthUnhealthy": "Unhealthy",
  "settings.proxyEmpty": "No proxies configured.",
  "settings.proxyTest": "Test",
  "settings.proxyTestTitle": "Test {{name}}",
  "settings.proxyTestUrl": "Test URL",
  "settings.proxyTestDescription":
    "Fetch this URL through the saved proxy and show the public IP reported by the endpoint.",
  "settings.proxyTestSolverDescription":
    "Ask the solver to fetch this URL and show its outbound IP. This checks fetching, but does not verify challenge solving or the route used by other Scryer traffic.",
  "settings.proxyTestRunning": "Testing the outbound connection…",
  "settings.proxyTestExitIp": "Proxy exit IP",
  "settings.proxyTestSolverIp": "Solver exit IP",
  "settings.proxyTestNoIp": "Response did not contain a recognizable IP.",
  "settings.proxyTimeout": "Timeout",
  "settings.proxyCreateNew": "Connect proxy",
  "settings.proxyCreate": "Create proxy",
  "settings.proxyUpdate": "Update proxy",
  "settings.proxyDeleteConfirmDescription": "Delete proxy {{name}}?",
  "settings.proxyValidation": "A name and a base URL are required.",
  "settings.proxyEndpoint": "Endpoint",
  "settings.proxyEndpointHelp":
    "ssh://host:port. Port 22 is used when you leave it out.",
  "settings.proxyAssignment": "Proxy",
  "settings.proxyDirect": "Direct (no proxy)",
  "settings.proxyMissing": "Missing proxy",
  "settings.proxyMissingHelp": "The assigned proxy was not found.",
  "settings.proxyDisabledHelp": "The assigned proxy is disabled.",
  "settings.proxyDisabledSuffix": "(disabled)",
  "settings.proxyClearPassword": "Clear stored password",
  "settings.proxyTunnelAuthHelp":
    "SSH requires a username and an Ed25519 private key. Password authentication is not supported.",
  "settings.proxyPrivateKey": "Private key",
  "settings.proxyPrivateKeyHelp":
    "only Ed25519 private keys are supported; generate one with `ssh-keygen -t ed25519` and paste the OpenSSH private key",
  "settings.proxyPrivateKeyStored": "Private key set",
  "settings.proxyPrivateKeyUnchanged":
    "A private key is stored. Paste a new one to replace it.",
  "settings.proxyClearPrivateKey": "Clear stored private key",
  "settings.proxyPrivateKeyPassphrase": "Key passphrase",
  "settings.proxyPrivateKeyPassphraseHelp":
    "Only needed when the private key is passphrase protected.",
  "settings.proxyHostKey": "Host key",
  "settings.proxyHostKeyPinnedAt": "Pinned {{time}}.",
  "settings.proxyHostKeyUnpinned":
    "Not pinned yet. The first successful connection pins the server's host key.",
  "settings.proxyHostKeyReset": "Reset host key",
  "settings.proxyHostKeyResetDescription":
    "Forget the pinned host key. The next connection trusts and pins whatever the server offers, so do this only when you know the server's key really changed.",
  "settings.proxyValidationTunnelUsername": "A tunnel needs a username.",
  "settings.proxyValidationTunnelAuth":
    "An SSH tunnel needs an Ed25519 private key.",
  "settings.downloadClientProxyHelp":
    "Any proxy kind may be assigned. A challenge solver has no effect on a native client, whose requests are not made by a plugin guest.",
  "status.proxyCreated": "Proxy created.",
  "status.proxyUpdated": "Proxy updated.",
  "status.proxyDeleted": "Deleted proxy {{name}}.",
  "status.proxyTestPassed": "Proxy test passed.",
  "status.proxyTestFailed": "Proxy test failed.",
  "status.editingProxy": "Editing proxy {{name}}.",
  "status.proxyHostKeyReset":
    "Host key reset for {{name}}. The next connection pins the server again.",
  "settings.proxyEndpointHelpHttp3":
    "https://host:port (443 by default). Uses HTTP/3 CONNECT over QUIC with verified TLS. Assign an indexer to test the route.",
  "settings.proxyEndpointHelpWireguard":
    "wireguard://host:port. Port 51820 is used when you leave it out.",
  "settings.proxyPrivateKeyHelpWireguard":
    "WireGuard keys are 32 bytes of base64, exactly as `wg genkey` prints them (44 characters ending in `=`)",
  "settings.proxyPeerPublicKey": "Peer public key",
  "settings.proxyPeerPublicKeyHelp":
    "The `PublicKey` line from the server's `[Peer]` section. A public key is public, so it is shown in full.",
  "settings.proxyPresharedKey": "Preshared key",
  "settings.proxyPresharedKeyHelp":
    "Optional. The `PresharedKey` line, when the server uses one.",
  "settings.proxyPresharedKeyStored": "Preshared key set",
  "settings.proxyPresharedKeyUnchanged":
    "A preshared key is stored. Paste a new one to replace it.",
  "settings.proxyClearPresharedKey": "Clear stored preshared key",
  "settings.proxyTunnelAddresses": "Addresses",
  "settings.proxyTunnelAddressesHelp":
    "The `Address` line from the `[Interface]` section, for example 10.6.0.2/32. One per line, or comma separated.",
  "settings.proxyTunnelDnsServers": "DNS servers",
  "settings.proxyTunnelDnsServersHelp":
    "Optional resolvers reached through the tunnel, one per line or comma separated. Without one, destinations must be addressed by IP.",
  "settings.proxyTunnelMtu": "MTU",
  "settings.proxyTunnelMtuHelp":
    "Optional, between {{min}} and {{max}}. Leave it empty for the default of {{default}}.",
  "settings.proxyTunnelKeepalive": "Keepalive",
  "settings.proxyTunnelKeepaliveHelp":
    "Optional, in seconds. Leave it empty for the default of {{default}}, or set 0 to switch keepalive off.",
  "settings.proxyTunnelPublicKey": "This tunnel's public key",
  "settings.proxyTunnelPublicKeyHelp":
    "Paste this into the server's `[Peer]` section as its `PublicKey` line.",
  "settings.proxyTunnelPublicKeyPending":
    "Saved after the private key is stored.",
  "settings.proxyTunnelPublicKeyCopy": "Copy",
  "settings.proxyImportConfig": "Import a WireGuard configuration",
  "settings.proxyImportConfigHelp":
    "Paste the whole configuration your VPN provider gave you, or choose the .conf file, and the fields below are filled from it. Every field also takes a single line pasted on its own.",
  "settings.proxyImportConfigApply": "Fill the form",
  "settings.proxyImportConfigFile": "Choose a file",
  "status.proxyImportConfigFilled":
    "Filled the form from the WireGuard configuration.",
  "status.proxyImportConfigUnreadable":
    "That does not look like a WireGuard configuration.",
  "status.proxyImportConfigFirstPeer":
    "It has {{count}} peers; only the first was imported.",
  "status.proxyImportConfigIgnored":
    "Ignored {{keys}}, which a tunnel proxy does not use.",
  "settings.proxyValidationWireguardPrivateKey":
    "WireGuard needs a private key.",
  "settings.proxyValidationWireguardPeerPublicKey":
    "WireGuard needs the peer's public key.",
  "settings.proxyValidationWireguardAddresses":
    "WireGuard needs at least one interface address, for example 10.6.0.2/32.",
  "settings.proxyValidationWireguardKeyShape":
    "{{field}} does not look like a WireGuard key: they are 32 bytes of base64, exactly as `wg genkey` prints them (44 characters ending in `=`).",
  "settings.proxyValidationWireguardMtu":
    "The tunnel MTU must be between {{min}} and {{max}}.",
  "settings.proxyValidationWireguardKeepalive":
    "Keepalive is a whole number of seconds, and 0 switches it off.",
  "status.proxyTunnelPublicKeyCopied": "Public key copied.",
  "status.proxyTunnelPublicKeyCopyFailed": "Unable to copy the public key.",
  "settings.qualityProfiles": "Quality Profiles",
  "settings.downloadClients": "Download Clients",
  "settings.mediaServers": "Media Servers",
  "settings.downloadClientRouting": "Download Client Routing",
  "settings.moviesSettings": "Movie Settings",
  "settings.seriesSettings": "Series Settings",
  "settings.animeSettings": "Anime settings",
  "facetSettings.general": "General",
  "facetSettings.quality": "Quality",
  "facetSettings.renaming": "Renaming",
  "facetSettings.routing": "Routing",
  "facetSettings.scoringPersona": "Scoring persona",
  "facetSettings.scoringPersonaOverrideLabel": "Persona override",
  "facetSettings.scoringPersonaUseDefault": "Use profile default",
  "facetSettings.scoringBehavior": "Scoring behavior",
  "facetSettings.scoringBehaviorHint":
    "How releases are scored at search time. Determined by the persona. Edit the quality profile for fine-grained control.",
  "persona.trait.balanced.source":
    "Source: standard preferences (BluRay > WEB-DL > WeRip)",
  "persona.trait.balanced.audio": "Audio: balanced codec weighting",
  "persona.trait.balanced.x265": "x265: penalized below 4K",
  "persona.trait.balanced.size": "File size: prefers larger files",
  "persona.trait.balanced.remux": "Remux: not prioritized by default",
  "persona.trait.balanced.hdr": "HDR: Dolby Vision and HDR10 rewarded",
  "persona.trait.audiophile.source": "Source: BluRay strongly preferred",
  "persona.trait.audiophile.audio":
    "Audio: lossless codecs heavily weighted (TrueHD, DTS-X, FLAC)",
  "persona.trait.audiophile.x265": "x265: strongly penalized below 4K",
  "persona.trait.audiophile.size": "File size: larger files preferred",
  "persona.trait.audiophile.remux": "Remux: very high bonus",
  "persona.trait.audiophile.hdr": "HDR: Dolby Vision highly rewarded",
  "persona.trait.efficient.source": "Source: WEB-DL and WEB-Rip preferred",
  "persona.trait.efficient.audio":
    "Audio: lossy codecs acceptable (DDP preferred)",
  "persona.trait.efficient.x265": "x265: rewarded at all resolutions",
  "persona.trait.efficient.size": "File size: compact encodes preferred",
  "persona.trait.efficient.remux": "Remux: not prioritized",
  "persona.trait.efficient.hdr": "HDR: standard",
  "persona.trait.compatible.source": "Source: WEB-DL preferred",
  "persona.trait.compatible.audio":
    "Audio: DDP and AAC prioritized for broad device support",
  "persona.trait.compatible.x265": "x265: H.264 preferred over H.265",
  "persona.trait.compatible.size": "File size: standard preferences",
  "persona.trait.compatible.remux": "Remux: not prioritized",
  "persona.trait.compatible.hdr":
    "HDR: Dolby Vision penalized (compatibility risk)",
  "settings.moviesLibrarySettings": "Movie media settings",
  "settings.seriesLibrarySettings": "Series media settings",
  "libraryFilter.all": "All Libraries",
  "libraryFilter.selectedCount": "{{count}} libraries",
  "settings.librariesLabel": "Libraries",
  "settings.libraryNameLabel": "Library name",
  "settings.libraryNamePlaceholder": "Kids",
  "settings.libraryNewButton": "New library",
  "settings.librarySaveAndScanButton": "Save & Scan",
  "settings.librarySaveOnlyButton": "Save only",
  "settings.libraryDeleteButton": "Delete library",
  "settings.libraryDeleteConfirm":
    "Delete {{name}}? Media files on disk will not be removed.",
  "settings.libraryEmpty": "No libraries available.",
  "settings.libraryCreated": "Library created.",
  "settings.librarySaved": "Library saved.",
  "settings.libraryDeleted": "Library deleted.",
  "settings.librarySaveFailed": "Failed to save library.",
  "settings.libraryDeleteFailed": "Failed to delete library.",
  "settings.librarySettingsLoadFailed": "Failed to load library settings.",
  "settings.libraryRequiredAudioLabel": "Required audio",
  "settings.libraryEffectiveAudio": "Effective: {{value}}",
  "settings.libraryEffectiveValue": "Effective: {{value}}",
  "settings.searchLanguages": "Release search languages",
  "settings.searchLanguagesHelp":
    "Select languages in search order. Automatic searches try these names when earlier queries find no acceptable release; interactive searches try all selected names. Metadata and audio languages are unchanged.",
  "settings.searchAliases": "Custom release names",
  "settings.searchAliasesHelp": "Separate names with semicolons",
  "settings.libraryMetadataLanguageLabel": "Metadata language",
  "settings.libraryEffectiveMetadataLanguage": "Effective: {{value}}",
  "settings.librarySeasonFoldersLabel": "Season folders",
  "settings.libraryEffectiveSeasonFolders": "Effective: {{value}}",
  "settings.libraryInheritGlobal": "Inherit global default",
  "settings.useSeasonFoldersLabel": "Use season folders",
  "settings.useSeasonFoldersHelp":
    "Place episodic files in their season or specials folder.",
  "settings.libraryQualityProfileLabel": "Quality profile",
  "settings.libraryRequestQualityProfilesLabel": "Request profiles",
  "settings.libraryRequestQualityProfilesHelp":
    "Leave empty to let requests use the library quality profile.",
  "settings.libraryInheritFacet": "Inherit catalog default",
  "settings.libraryCustomRouting": "Custom library routing",
  "settings.libraryEffectiveProfile": "Effective: {{value}}",
  "settings.libraryScoringPersonaLabel": "Scoring persona",
  "settings.libraryEffectivePersona": "Effective: {{value}}",
  "settings.rootFoldersLabel": "Root folders",
  "settings.rootFoldersEmpty":
    "No root folders configured. Add at least one path.",
  "settings.rootFoldersHelp":
    "Root folders are the top-level directories where media is stored. The default folder is used for new titles.",
  "settings.rootFolderAdd": "Add root folder",
  "settings.rootFolderEdit": "Edit root folder",
  "settings.rootFolderConflict":
    "This root folder is already used by {{libraries}}.",
  "settings.rootFolderSetDefault": "Set as default",
  "settings.rootFolderInvalid": "Invalid",
  "settings.rootFolderInvalidTooltip":
    "Scryer could not browse this root folder from the host.",
  "settings.rootFolderValidationUnavailable":
    "Root folder validation is temporarily unavailable. Saved paths have not been marked invalid.",
  "settings.unsavedLibraryChangesTitle": "Unsaved library changes",
  "settings.unsavedLibraryChangesConfirm":
    "Discard your unsaved library changes and leave this page?",
  "settings.libraryScanTitle": "Library scan",
  "settings.libraryScanButton": "Scan library",
  "settings.libraryScanRunning": "Scanning library…",
  "settings.libraryScanSummary":
    "{{imported}} imported, {{skipped}} skipped, {{unmatched}} unmatched.",
  "settings.libraryScanFailed": "Library scan failed.",
  "settings.libraryScanCancel": "Cancel",
  "settings.libraryScanRunInBackground": "Run in background",
  "settings.libraryScanCancelFailed": "Failed to cancel library scan.",
  "settings.libraryScanCanceledSummary":
    "Library scan canceled. {{imported}} imported, {{skipped}} skipped, {{unmatched}} unmatched.",
  "settings.libraryScanToastTitle": "{{facet}} library scan",
  "settings.libraryScanDiscovering": "Discovering titles…",
  "settings.libraryScanProgressPending": "Waiting for discovery…",
  "settings.libraryScanProgressCalculatingTotal": "Calculating total…",
  "settings.libraryScanProgressScannedCount": "{{count}} scanned",
  "settings.libraryScanTitleMatch": "Title Match",
  "settings.libraryScanFilesScanned": "Media Analysis",
  "settings.libraryScanNoTitleMatchNeeded": "No title matching needed",
  "settings.libraryScanNoFilesToScan": "No media analysis needed",
  "settings.libraryScanAlreadyRunning":
    "{{facet}} library scan is already running.",
  "settings.libraryScanProgressCount": "{{current}} / {{total}}",
  "settings.libraryScanScanningSubtitle":
    "Found {{count}} titles · analyzing media…",
  "settings.libraryScanDoneSubtitle": "Scan complete · {{count}} titles",
  "settings.libraryScanIssuesSubtitle":
    "Completed with issues · {{count}} titles",
  "settings.libraryScanFailedSubtitle": "Scan failed",
  "settings.libraryScanCanceledSubtitle": "Scan canceled",
  "settings.libraryScanBadgeDone": "Done",
  "settings.libraryScanBadgeReview": "Review",
  "settings.libraryScanBadgeFailed": "Failed",
  "settings.libraryScanBadgeCanceled": "Canceled",
  "settings.libraryScanViewTitles": "View {{count}} titles",
  "settings.libraryScanReviewUnmatched": "Review {{count}} unmatched",
  "jobs.title": "Jobs",
  "jobs.schedule": "Schedule",
  "jobs.never": "Never",
  "jobs.runSummaryCompleted": "Completed",
  "jobs.runSummaryRunning": "Running",
  "jobs.recentRuns": "Recent runs",
  "jobs.loadingRecentRuns": "Loading recent runs…",
  "jobs.noRunsYet": "No runs recorded yet.",
  "jobs.healthCheckIssues": "Health check issues",
  "jobs.fullHashBackfillFailures": "Files that could not be hashed",
  "jobs.fullHashBackfillFailuresMore": "…and {{count}} more not listed",
  "jobs.nextRunPrefix": "Next run: {{value}}",
  "jobs.startedAt": "Started {{value}}",
  "jobs.completedAt": "Completed {{value}}",
  "jobs.failedToTrigger": "Failed to trigger job",
  "jobs.column.name": "Name",
  "jobs.column.schedule": "Schedule",
  "jobs.column.nextRun": "Next run",
  "jobs.column.lastRun": "Last run",
  "jobs.column.status": "Status",
  "jobs.action.run": "Run",
  "jobs.action.runNow": "Run now",
  "jobs.cancelSearchRequested":
    "Cancellation requested. The current search step will finish before the remaining steps stop.",
  "jobs.cancelSearchNotRunning": "This search is no longer running.",
  "jobs.cancelSearchFailed": "Could not cancel the search.",
  "jobs.action.runArtworkNow": "Run now (override schedule)",
  "jobs.status.idle": "Idle",
  "jobs.status.queued": "Queued",
  "jobs.status.discovering": "Discovering",
  "jobs.status.running": "Running",
  "jobs.status.completed": "Completed",
  "jobs.status.warning": "Warning",
  "jobs.status.failed": "Failed",
  "jobs.triggerSource.manual": "Manual",
  "jobs.triggerSource.scheduledStartup": "Scheduled startup",
  "jobs.triggerSource.scheduledInterval": "Scheduled interval",
  "jobs.triggerSource.scheduledDaily": "Scheduled daily",
  "jobs.triggerSource.systemInternal": "System internal",
  "jobs.builtin.title": "Job schedule",
  "jobs.custom.title": "Your jobs",
  "jobs.custom.description": "Scripts you run on a schedule or on demand.",
  "jobs.custom.add": "Add job",
  "jobs.custom.addTitle": "Add job",
  "jobs.custom.editTitle": "Edit job",
  "jobs.custom.editorDescription":
    "Write a script or point to one on disk, then choose when it runs.",
  "jobs.custom.empty": "No jobs yet. Add a script to run it on a schedule.",
  "jobs.custom.edit": "Edit job",
  "jobs.custom.enable": "Enable job",
  "jobs.custom.disable": "Disable job",
  "jobs.custom.disabled": "Disabled",
  "jobs.custom.delete": "Delete job",
  "jobs.custom.capturedOutput": "Captured output",
  "jobs.custom.noOutput": "No runs with captured output yet.",
  "jobs.custom.started": "Started",
  "jobs.custom.duration": "Duration",
  "jobs.custom.exitCode": "Exit code",
  "deletePreview.counting": "Counting files to delete...",
  "deletePreview.error": "Unable to prepare delete preview.",
  "deletePreview.folderUnknown":
    "Scryer can't tell which folder holds this title's files, so it won't delete anything from disk.",
  "deletePreview.removeFromCatalogOnly": "Remove from catalog only",
  "deletePreview.files": "Files",
  "deletePreview.media": "Media",
  "deletePreview.subtitles": "Subtitles",
  "deletePreview.images": "Images",
  "deletePreview.other": "Other",
  "deletePreview.folders": "Folders",
  "deletePreview.samplePaths": "Sample paths",
  "deletePreview.confirmPrompt": "Type DELETE to confirm this delete.",
  "settings.movieSettingsSaved": "Movie settings saved.",
  "settings.seriesSettingsSaved": "Series settings saved.",
  "settings.qualityProfileSection": "Quality profile",
  "settings.qualityProfileGlobalLabel": "Default quality profile (global)",
  "settings.qualityProfileNameRequired": "Quality profile name is required.",
  "settings.qualityProfileNameDuplicate":
    "Quality profile names must be unique.",
  "settings.qualityProfileGlobalHelp":
    "Used when a category does not override it.",
  "settings.qualityProfileOverrideLabel": "{{category}} quality override",
  "settings.qualityProfileOverridesLabel": "Category quality profile overrides",
  "settings.qualityProfileOverrideHelp":
    "Use inherit to fall back to the global default for this content category.",
  "settings.qualityProfileInheritLabel": "Inherit from global",
  "settings.qualityProfileCatalogInvalid":
    "Quality profile catalog is not valid JSON.",
  "settings.qualityProfileUnknown": 'Unknown quality profile id "{{id}}".',
  "settings.qualitySettingsSaved": "Quality settings saved.",
  "settings.downloadClientRoutingSaved": "Download client routing saved.",
  "settings.downloadClientRoutingSavedFor":
    'Download client routing saved for "{{name}}".',
  "settings.delayProfiles": "Delay Profiles",
  "settings.acquisition": "Acquisition",
  "settings.acquisitionSaved": "Acquisition settings saved.",
  "settings.acquisitionIntro":
    "RSS is the steady-state acquisition path: each wanted item is actively searched once per indexer, then watched via RSS. Active backfill is paced and finite.",
  "settings.acquisitionEnabled": "Enable automatic acquisition",
  "settings.acquisitionEnabledHelp":
    "Runs the background acquisition loop: convergence search sweeps and RSS matching.",
  "settings.acquisitionThresholds": "Upgrade thresholds",
  "settings.acquisitionSameTierMinDelta": "Same-tier minimum score delta",
  "settings.acquisitionSameTierMinDeltaHelp":
    "A release in the same quality tier must beat the current file's score by at least this much.",
  "settings.acquisitionConvergence": "Convergence & pacing",
  "settings.acquisitionPollIntervalSeconds": "Poll interval (seconds)",
  "settings.acquisitionPollIntervalSecondsHelp":
    "How often download clients are checked for failed downloads.",
  "settings.acquisitionWalkIntervalSeconds": "Search walk interval (seconds)",
  "settings.acquisitionWalkIntervalSecondsHelp":
    "How often the library is checked for missing and upgradable media and a batch of titles is searched. Adding a title, editing a quality profile and similar changes start a walk right away. A walk never runs more often than the poll interval.",
  "settings.acquisitionMaxScopesPerCycle": "Max items evaluated per cycle",
  "settings.acquisitionMaxScopesPerCycleHelp":
    "Evaluation ceiling for one backfill cycle. Indexer rate limits — not this count — set the actual request rate.",
  "settings.acquisitionReconvergeDays": "Re-converge after (days)",
  "settings.acquisitionReconvergeDaysHelp":
    "0 disables it (recommended): converged items are watched via RSS. Enable only as a break-glass re-check when a feed proves incomplete.",
  "settings.movieAvailability": "Movie availability",
  "settings.libraryMovieAvailabilityDescription":
    "Override the Movie facet's availability settings for this library. Each movie can still set its own minimum availability.",
  "settings.movieAvailabilityFacetDescription":
    "When automatic search starts looking for a movie. Each movie library can override these settings, and each movie can set its own minimum availability.",
  "settings.movieDefaultAvailability": "Default minimum availability",
  "settings.movieDefaultAvailabilityHelp":
    "Applies to every movie that does not set its own minimum availability.",
  "settings.movieReleaseMarket": "Release market",
  "settings.movieReleaseMarketHelp":
    "Two-letter ISO-3166 country code used to select theatrical, digital, and physical dates.",
  "settings.movieAvailabilityDelay": "Availability offset (days)",
  "settings.movieAvailabilityDelayHelp":
    "Negative values allow early acquisition; positive values delay it. Zero starts on the release date.",
  "settings.movieAvailabilityStatus.available": "Available",
  "settings.movieAvailabilityStatus.waiting": "Waiting",
  "settings.movieAvailabilityStatus.unknown": "Unknown",
  "settings.movieAvailability.known": "Known date",
  "settings.movieAvailability.estimated": "Estimated date",
  "settings.movieAvailability.reason.available":
    "The selected threshold has been met.",
  "settings.movieAvailability.reason.release_dates_unknown":
    "Release dates are not known yet.",
  "settings.movieAvailability.reason.release_market_refresh_pending":
    "Refreshing dates for selected market.",
  "settings.movieAvailability.reason.theatrical_release_date_unknown":
    "Theatrical release date is unknown.",
  "settings.movieAvailability.reason.home_release_date_unknown":
    "Digital or physical release date is unknown.",
  "settings.movieAvailability.reason.home_release_date_unusable":
    "Known home-release date could not be read.",
  "settings.movieAvailability.reason.home_release_date_estimated_worldwide":
    "Estimated from the earliest digital or physical release in any market.",
  "settings.movieAvailability.reason.home_release_date_estimated_from_worldwide_theatrical":
    "Estimated 90 days after the earliest theatrical release in any market.",
  "settings.movieAvailability.reason.theatrical_release_date_estimated_worldwide":
    "Estimated from the earliest theatrical release in any market.",
  "settings.movieAvailability.reason.theatrical_release_date_estimated_from_worldwide_home":
    "Estimated from the earliest home release in any market.",
  "settings.movieAvailability.reason.waiting_for_release_date":
    "Waiting for release date.",
  "settings.movieAvailability.reason.effective_release_date_out_of_range":
    "Effective release date is out of range.",
  "settings.delayProfileExisting": "Existing delay profiles",
  "settings.delayProfileNone":
    "No delay profiles configured. Releases will be grabbed immediately.",
  "settings.delayProfileCreate": "Create Profile",
  "settings.delayProfileCreateNew": "Create new delay profile",
  "settings.delayProfileEdit": "Edit Profile",
  "settings.delayProfileConfirmDiscardTitle":
    "Discard current delay profile edits?",
  "settings.delayProfileConfirmDiscardDescription":
    "Your current delay profile edits will be lost.",
  "settings.delayProfileDeleteConfirm": "Delete this delay profile?",
  "settings.delayProfileNameLabel": "Name",
  "settings.delayProfileNamePlaceholder": "e.g. Wait for better quality",
  "settings.delayProfileNameRequired": "Delay profile name is required.",
  "settings.delayProfileUsenetDelay": "Usenet delay (minutes)",
  "settings.delayProfileUsenetDelayHelp":
    "How long to hold usenet releases before grabbing. 0 = grab immediately.",
  "settings.delayProfileTorrentDelay": "Torrent delay (minutes)",
  "settings.delayProfileTorrentDelayHelp":
    "How long to hold torrent releases before grabbing. 0 = grab immediately.",
  "settings.delayProfilePreferred": "Preferred protocol",
  "settings.delayProfilePreferredHelp":
    "Score-based bypass only applies to releases on the preferred protocol.",
  "settings.delayProfileProtocolModeLabel": "Protocol mode",
  "settings.delayProfileProtocolModeHelp":
    "Delays are measured from the time your indexer published the release. Preferred protocol selects between otherwise equivalent releases and controls bypass eligibility.",
  "settings.delayProfileProtocolMode.preferUsenet": "Prefer Usenet",
  "settings.delayProfileProtocolMode.preferTorrent": "Prefer Torrent",
  "settings.delayProfileProtocolMode.onlyUsenet": "Only Usenet",
  "settings.delayProfileProtocolMode.onlyTorrent": "Only Torrent",
  "settings.delayProfileMinAge": "Minimum age (minutes)",
  "settings.delayProfileMinAgeHelp":
    "Usenet only. Hard gate — releases younger than this are held regardless of score. Prevents grabbing incomplete NZBs. 0 = disabled.",
  "settings.delayProfileBypassLabel": "Bypass score threshold",
  "settings.delayProfileBypassPlaceholder": "Leave empty to never bypass",
  "settings.delayProfileBypassHelp":
    "If a release on the preferred protocol scores at or above this threshold, grab it immediately regardless of the delay.",
  "settings.delayProfileBypassHighestQualityLabel":
    "Bypass delay for highest quality",
  "settings.delayProfileBypassHighestQualityHelp":
    "Grab the highest-quality eligible release immediately when it is on the preferred protocol.",
  "settings.delayProfileFacetsLabel": "Applies to",
  "settings.delayProfileFacetsHelp":
    "Which content types this profile applies to. Leave all unchecked for all types.",
  "settings.delayProfileLibrariesLabel": "Libraries",
  "settings.delayProfileLibrariesHelp":
    "Libraries this profile applies to. Leave all selected to apply to every library. Deleting a profile's last library disables the profile.",
  "settings.delayProfileUnknownLibrary": "Unknown library",
  "settings.delayProfileAllFacets": "All",
  "settings.delayProfileTagsLabel": "Tags",
  "settings.delayProfileTagsPlaceholder":
    "Comma-separated tags (empty = default catch-all)",
  "settings.delayProfileTagsHelp":
    "Match titles by tag. Empty means this profile is the default catch-all.",
  "settings.delayProfilePriorityLabel": "Priority",
  "settings.delayProfilePriorityHelp":
    "Lower number = higher priority. First matching profile wins.",
  "settings.delayProfileEnabledLabel": "Enabled",
  "settings.delayProfilesSaved": "Delay profiles saved.",
  "settings.delayProfileSaveError": "Failed to save delay profiles.",
  "settings.seedingProfiles": "Seeding Profiles",
  "settings.seedingProfileExisting": "Existing seeding profiles",
  "settings.seedingProfileNone":
    "No seeding profiles configured. Torrents are grabbed without Scryer seed goals and follow your download client's own limits.",
  "settings.seedingProfileCreate": "Create Profile",
  "settings.seedingProfileCreateNew": "Create new seeding profile",
  "settings.seedingProfileEdit": "Edit Profile",
  "settings.seedingProfileConfirmDiscardTitle":
    "Discard current seeding profile edits?",
  "settings.seedingProfileConfirmDiscardDescription":
    "Your current seeding profile edits will be lost.",
  "settings.seedingProfileDeleteConfirm": "Delete this seeding profile?",
  "settings.seedingProfileNameLabel": "Name",
  "settings.seedingProfileNamePlaceholder": "e.g. Private tracker — 1.0 ratio",
  "settings.seedingProfileGoalPlaceholder":
    "Leave empty to use the client's own limit",
  "settings.seedingProfileRatioLabel": "Ratio",
  "settings.seedingProfileRatioHelp":
    "Stop seeding once this share ratio is reached. Empty defers to the download client's own limit.",
  "settings.seedingProfileSeedTimeFormatHint":
    "Duration such as 90m, 36h, 1d 12h or 2w. A plain number is minutes.",
  "settings.seedingProfileSeedTimeLabel": "Seed time",
  "settings.seedingProfileMinimumSeedersLabel": "Minimum seeders",
  "settings.seedingProfileMinimumSeedersHelp":
    "Releases from an indexer using this profile need at least this many seeders to be grabbed. Leave empty to use the system default; enter 0 to turn the check off.",
  "settings.seedingProfileMinimumSeedersUnknownHelp":
    "Indexers that do not report a seeder count are always eligible.",
  "settings.seedingProfileMinimumSeedersRoutingHelp":
    "A profile that is only assigned through a download client routing entry does not apply its minimum: the check runs before a client is picked, so it looks at the indexer's profile, then a profile imported from Prowlarr, then the default profile.",
  "settings.seedingProfileMinimumSeedersInheritHelp":
    "An empty field inherits the Default minimum seeders setting, not a value imported from Prowlarr — assigning this profile to a Prowlarr-managed indexer replaces that imported minimum.",
  "settings.seedingProfileMinimumSeedersPlaceholder":
    "Leave empty to use the system default",
  "settings.seedingProfileMinimumSeedersOffBadge": "Seeders check off",
  "settings.seedingProfileMinimumSeedersBadge": "≥ {{value}} seeders",
  "settings.seedingMinimumSeedersFloorLabel": "Default minimum seeders",
  "settings.seedingMinimumSeedersFloorHelp":
    "Applied when no seeding profile covers the indexer a release came from. Enter 0 to allow releases with no seeders anywhere.",
  "settings.seedingMinimumSeedersFloorSaved": "Default minimum seeders saved.",
  "settings.seedingProfileSeedTimeHelp":
    "Stop seeding once the torrent has seeded this long. Empty defers to the download client's own limit.",
  "settings.seedingProfileSeedTimeTransmissionHelp":
    "Transmission has no total seed-time limit, so this maps to its idle limit there.",
  "settings.seedingProfileBehaviorColumn": "Behavior",
  "settings.seedingProfileSeasonPackSummary": "Season packs: {{value}}",
  "settings.seedingProfileIgnoresMinimums": "Ignores tracker minimums",
  "settings.seedingProfileNoneAvailable":
    "No profiles yet — create one under Settings → Seeding Profiles",
  "settings.seedingProfileSeasonPackAdvanced":
    "Season pack overrides (Advanced)",
  "settings.seedingProfileSeasonPackModeLabel": "Season pack goals",
  "settings.seedingProfileSeasonPackModeHelp":
    "Inherit uses the goals above for season packs too. Override applies the season-pack goals instead.",
  "settings.seedingProfileSeasonPackInherit": "Inherit",
  "settings.seedingProfileSeasonPackOverride": "Override",
  "settings.seedingProfileSeasonPackRatioLabel": "Season pack ratio",
  "settings.seedingProfileSeasonPackSeedTimeLabel": "Season pack seed time",
  "settings.seedingProfileSeasonPackGoalsHelp":
    "Only used in override mode. An empty field defers that goal to the download client.",
  "settings.seedingProfileHonorTrackerMinimumsLabel": "Honor tracker minimums",
  "settings.seedingProfileHonorTrackerMinimumsHelp":
    "Raise the resolved goals to the minimum ratio and seed time a release declares, so private trackers do not record a hit and run.",
  "settings.seedingProfilePostImportTrackingLabel": "After import",
  "settings.seedingProfilePostImportTrackingHelp":
    "Whether Scryer keeps managing the torrent once the files have been imported.",
  "settings.seedingProfilePostImportTrackingPark":
    "Keep managing until goals are met",
  "settings.seedingProfilePostImportTrackingHandOff":
    "Hand off to the download client",
  "settings.seedingProfilePostImportTrackingHandOffBadge": "Handed off",
  "settings.seedingProfilePostImportTrackingHandOffHelp":
    "After import, Scryer stops managing this torrent: the entry stays in the client, is never auto-removed, and leaves the queue. Seed goals and the goal-met action no longer apply — your client's own limits take over. Files are still imported by hardlink or copy while the torrent is seeding.",
  "settings.seedingProfileGoalMetActionLabel": "When the goal is met",
  "settings.seedingProfileGoalMetActionHelp":
    "What happens to the torrent once its seed goal is reached.",
  "settings.seedingProfileGoalMetRemoveEntry": "Remove entry",
  "settings.seedingProfileGoalMetStopSeeding": "Stop seeding",
  "settings.seedingProfileGoalMetKeep": "Keep seeding",
  "settings.seedingProfileNeverRemoveLabel": "Never remove",
  "settings.seedingProfileNeverRemoveHelp":
    "Blocks all automatic removal for torrents grabbed under this profile, even after the goal is met. You have to remove them yourself.",
  "settings.seedingProfileNeverRemoveBadge": "Never removed",
  "settings.seedingProfileDefaultTitle": "Default seeding profile",
  "settings.seedingProfileDefaultLabel":
    "Applies when nothing more specific matches",
  "settings.seedingProfileDefaultHelp":
    "Used for torrent grabs when neither the indexer nor the download-client routing entry names a profile.",
  "settings.seedingProfileDefaultNone":
    "No default — use the client's own limits",
  "settings.seedingProfileDefaultBadge": "Default",
  "settings.seedingProfileMissing": 'Missing profile "{{id}}"',
  "settings.seedingProfileSaved": "Seeding profile saved.",
  "settings.seedingProfileSaveError": "Failed to save the seeding profile.",
  "settings.seedingProfileDeleted": "Seeding profile deleted.",
  "settings.seedingProfileDeleteError": "Failed to delete the seeding profile.",
  "settings.seedingProfileDefaultSaved": "Default seeding profile saved.",
  "settings.seedingProfileProwlarrManaged": "Managed by Prowlarr",
  "settings.seedingProfileProwlarrManagedMinimum":
    "Managed by Prowlarr · minimum {{count}} seeders",
  "settings.seedingProfileProwlarrManagedMinimumOne":
    "Managed by Prowlarr · minimum 1 seeder",
  "settings.seedingProfileProwlarrManagedMinimumOff":
    "Managed by Prowlarr · seeder check off",
  "settings.seedingProfileInherit": "Inherit default",
  "settings.seedingProfileInheritProwlarrMinimum":
    "Inherit default · Prowlarr minimum {{count}} seeders",
  "settings.seedingProfileInheritProwlarrMinimumOne":
    "Inherit default · Prowlarr minimum 1 seeder",
  "settings.seedingProfileInheritProwlarrMinimumOff":
    "Inherit default · Prowlarr seeder check off",
  "settings.seedingProfileRoutingInherit": "Inherit global default",
  "settings.seedingProfileColumn": "Seeding profile",
  "settings.seedingProfileIndexerLabel": "Seeding profile for {{name}}",
  "settings.seedingProfileRoutingLabel": "Seeding profile for {{name}}",
  "settings.seedingProfileNotApplicable": "Torrent indexers only",
  "status.indexerSeedingProfileSaving": "Saving seeding profile…",
  "status.indexerSeedingProfileSaved": 'Seeding profile saved for "{{name}}".',
  "settings.mediaServersSection": "Media server connections",
  "settings.mediaServerCreate": "Save Media Server",
  "settings.mediaServerCreateNew": "Connect a new media server",
  "settings.mediaServerUpdate": "Update Media Server",
  "settings.mediaServerConfirmDiscardTitle":
    "Discard current media server edits?",
  "settings.mediaServerConfirmDiscardDescription":
    "Your current media server edits will be lost.",
  "settings.mediaServerEnabled": "Enable media server connection",
  "settings.mediaServerAuthCapabilities": "Permissions",
  "settings.mediaServerAutoAddEnabled": "Auto-add new verified users",
  "settings.mediaServerPathMappings": "Path Mappings",
  "settings.mediaServerPathMappingsHelp":
    "Each rule is saved as server path => Scryer path.",
  "settings.mediaServerValidation": "Name and base URL are required.",
  "settings.noMediaServersFound": "No media server connections found.",
  "settings.capabilities": "Capabilities",
  "settings.credentials": "Credentials",
  "settings.apiKeyConfigured": "API key saved",
  "settings.apiKeyMissing": "API key missing",
  "settings.plexServerSelected": "Plex server selected",
  "settings.plexServerMissing": "Plex server not selected",
  "settings.discoverPlexServers": "Sign in with Plex to find servers",
  "settings.plexServer": "Plex server",
  "settings.plexServerDiscoveryEmpty":
    "Plex did not return any accessible servers.",
  "settings.plexServerDiscoveryRequired":
    "Sign in with Plex and select a server before enabling Plex login, linking, or auto-add.",
  "settings.setupViaApiKey": "Setup via API key",
  "settings.loginAsAdmin": "Login as an admin",
  "settings.clearSavedApiKey": "Clear saved API key",
  "settings.adminUsername": "Admin username",
  "settings.adminPassword": "Admin password",
  "settings.downloadClientCreate": "Connect a download client",
  "settings.downloadClientCreateNew": "Connect a download client",
  "settings.downloadClientUpdate": "Update Download Client",
  "settings.downloadClientConfirmDiscardTitle":
    "Discard current download client edits?",
  "settings.downloadClientConfirmDiscardDescription":
    "Your current download client edits will be lost.",
  "settings.downloadClientUseSsl": "Use SSL",
  "settings.downloadClientUrlBase": "URL Base",
  "settings.downloadClientUrlPreview": "Computed base URL",
  "settings.downloadClientCategory": "Category",
  "settings.downloadClientRecentPriority": "Recent Queue Priority",
  "settings.downloadClientOlderPriority": "Older Queue Priority",
  "settings.downloadClientPriorityForce": "Force",
  "settings.downloadClientPriorityVeryHigh": "Very high",
  "settings.downloadClientPriorityHigh": "High",
  "settings.downloadClientPriorityNormal": "Normal",
  "settings.downloadClientPriorityLow": "Low",
  "settings.downloadClientPriorityVeryLow": "Very low",
  "settings.downloadClientPriority": "Priority",
  "settings.downloadClientRemoveCompleted": "Remove Completed",
  "settings.downloadClientRemoveFailed": "Remove Failed",
  "settings.downloadClientUseSslInfo":
    "Use SSL/TLS when connecting to this download client.",
  "settings.downloadClientEnabledInfo":
    "When disabled, this client is skipped during download routing.",
  "settings.downloadClientEnabledLabel": "Enable download client",
  "settings.downloadClientFilesystemPathMapping":
    "Filesystem Path Mapping (Advanced)",
  "settings.downloadClientFilesystemPathMappingHelp":
    "Remote path mappings are rarely needed. Use them only when Scryer and this download client see the same download folder at different filesystem paths.",
  "settings.downloadClientRemotePathMappings": "Remote Path Mappings",
  "settings.downloadClientRemotePathMappingsHelp":
    "Each rule is saved as REMOTE => LOCAL.",
  "settings.downloadClientRemotePathMappingsRemoteLabel": "Remote Path",
  "settings.downloadClientRemotePathMappingsRemoteHelp":
    "Path reported by the download client.",
  "settings.downloadClientRemotePathMappingsLocalLabel": "Local Path",
  "settings.downloadClientRemotePathMappingsLocalHelp":
    "Path Scryer should use on this machine.",
  "settings.downloadClientRemotePathMappingsAdd": "Add Mapping",
  "settings.downloadClientRemotePathMappingsEmptyState":
    "No path mappings configured yet. Add one only if Scryer cannot read the path reported by the download client.",
  "settings.downloadClientRemotePathMappingsExampleLabel": "Example",
  "settings.downloadClientRemotePathMappingsRemoteRequired":
    "Remote Path is required when a mapping row is present.",
  "settings.downloadClientRemotePathMappingsLocalRequired":
    "Local Path is required when a mapping row is present.",
  "settings.downloadClientRemotePathMappingsLocalAbsolute":
    "Local Path must be an absolute path.",
  "settings.downloadClientRemotePathMappingsRemoteDuplicate":
    "Remote Path duplicates another mapping in this client.",
  "settings.downloadClientSabnzbdAuthHelp":
    "SABnzbd supports either an API key or username/password. For Decypharr, this is typically host :8282, URL base /sabnzbd, username = your Arr URL, password = your Arr token.",
  "settings.downloadClientSabnzbdNzbdavHelp":
    "For NzbDav, point SAB-compatible completed downloads at the mounted completed-symlinks path, make sure Scryer can read that mount locally, and add remote path mappings if NzbDav reports a different visible path.",
  "settings.downloadClientQbittorrentDecypharrHelp":
    "For Decypharr-backed qBittorrent, use your Arr URL as the username and your Arr token as the password. Set the matching Arr name, like radarr or sonarr, in Download Client Routing for that facet.",
  "settings.downloadClientDecypharrFilesystemHelp":
    "Completed-download import only works from paths Scryer can read locally. For Decypharr and NzbDav, Scryer imports from the completed symlink folder reported by the client, so mount that path into Scryer and add remote path mappings when the reported paths differ.",
  "settings.downloadClientNamePlaceholder": "My Download Client",
  "settings.downloadClientHostPlaceholder": "download.example.com",
  "settings.downloadClientUrlBasePlaceholder": "e.g. api",
  "settings.downloadClientPortPlaceholder": "8080",
  "settings.downloadClientCategoryPlaceholder": "series",
  "settings.downloadClientStatus": "Status",
  "settings.downloadClientLastError": "Error",
  "settings.downloadClientLastSeen": "Seen {{time}}",
  "settings.downloadClientNoActivity": "No activity",
  "settings.downloadClientRoutingScope":
    "Download client metadata for {{scope}}",
  "settings.downloadClientRoutingGloballyEnabled": "Globally Enabled",
  "settings.downloadClientRoutingEnabled": "Enabled",
  "settings.noDownloadClientsFound": "No download clients found.",
  "settings.indexerRoutingScope": "Indexer metadata for {{scope}}",
  "settings.indexerRoutingPriority": "Priority",
  "settings.indexerRoutingCategories": "Categories",
  "settings.indexerRoutingGloballyEnabled": "Globally Enabled",
  "settings.indexerRoutingEnabled": "Enabled",
  "settings.indexerRoutingCategoriesPlaceholder": "Select categories",
  "settings.indexerRoutingNoIndexers": "No indexers found.",
  "settings.indexerRoutingCustomCategories": "Custom codes",
  "settings.indexerRoutingCustomCategoryPlaceholder":
    "Numeric id from the indexer caps",
  "settings.indexerRoutingCustomCategoryAdd": "Add",
  "settings.indexerRoutingIndexerCategories": "Also on this indexer",
  "settings.indexerCategorySeries": "Series",
  "settings.indexerCategoryMovies": "Movies",
  "settings.indexerCategoryOther": "Other",
  "settings.indexerCategoryForeign": "Foreign",
  "settings.indexerCategorySd": "SD",
  "settings.indexerCategoryHd": "HD",
  "settings.indexerCategoryUhd": "UHD",
  "settings.indexerCategorySport": "Sport",
  "settings.indexerCategoryAnime": "Anime",
  "settings.indexerCategoryDocumentary": "Documentary",
  "settings.indexerCategoryBluRay": "Blu-ray",
  "settings.indexerCategoryThreeD": "3D",
  "settings.indexerCategoryMisc": "Misc",
  "indexerCategory.labelCategory": "Category",
  "settings.downloadClientValidation": "Name and host are required.",
  "settings.downloadClientBaseUrlRequired":
    "A valid host and port are required.",
  "settings.folderTemplateLabel": "Folder template",
  "settings.folderTemplatePlaceholder": "Placeholder",
  "settings.folderTemplateRequired": "Folder template is required.",
  "settings.seasonFolderTemplateLabel": "Season folder template",
  "settings.seasonFolderTemplatePlaceholder": "Season {season}",
  "settings.seasonFolderTemplateRequired":
    "Season folder template is required.",
  "settings.specialsFolderTemplateLabel": "Specials folder template",
  "settings.specialsFolderTemplatePlaceholder": "Specials",
  "settings.specialsFolderTemplateRequired":
    "Specials folder template is required.",
  "settings.seasonFolderTemplateMustContainSeason":
    "Season folder template must include {season}.",
  "settings.seasonFolderValidationUnknownToken":
    "Season folder templates support {season}, title, year, and external ID tokens. Invalid token: {{token}}",
  "settings.folderRenameSectionTitle": "Folder Renaming",
  "settings.folderValidationUnknownToken":
    "Folder templates support {title}, {year}, and external ID tokens. Invalid token: {{token}}",
  "settings.folderValidationEmpty": "Folder template cannot be empty",
  "settings.folderValidationLeafTitleToken":
    "The last folder in the template must include a title or external ID token",
  "settings.folderValidationIllegalCharacter":
    "Folder template contains an illegal filesystem character: {{character}}",
  "settings.renameTemplateLabel": "Rename template",
  "settings.renameTemplatePlaceholder": "Placeholder",
  "settings.renameTemplateRequired": "Rename template is required.",
  "settings.renameCollisionPolicyLabel": "Collision policy",
  "settings.renameCollisionPolicySkip": "Skip",
  "settings.renameCollisionPolicyError": "Error",
  "settings.renameCollisionPolicyReplaceIfBetter": "Replace if better",
  "settings.renameMissingMetadataPolicyLabel": "Missing metadata policy",
  "settings.renameMissingMetadataPolicyFallbackTitle": "Fallback to title",
  "settings.renameMissingMetadataPolicySkip": "Skip",
  "settings.renameSectionTitle": "File Renaming",
  "settings.renameReferenceTitle": "Reference",
  "settings.renameReferenceTokens": "Tokens",
  "settings.renameReferenceFunctions": "Functions",
  "settings.renameReferenceFiles": "Files",
  "settings.renameReferenceFolders": "Folders",
  "settings.renameLiteralBracesLabel": "Literal braces",
  "settings.renameFunctionTruncateTitle": "Length",
  "settings.renameFunctionOptionalGroupTitle": "Optional group",
  "settings.renameOptionalGroupDescription":
    "Render the group only when its guard token has a value. Optional groups cannot be nested.",
  "settings.renameFunctionWhitespaceTitle": "Whitespace",
  "settings.renameFunctionWhitespaceDescription":
    "Use one space filter to choose the separator for token text.",
  "settings.renameFunctionPaddingTitle": "Number padding",
  "settings.renameFunctionChainTitle": "Chaining",
  "settings.renameFunctionBracesTitle": "Braces",
  "settings.renameTruncateFilterLabel":
    "Limit a token to the first N characters.",
  "settings.renameNumberPaddingLabel": "Pad numeric tokens to N digits.",
  "settings.renameSpaceUnderscoreFilterLabel":
    "Replace whitespace with underscores.",
  "settings.renameSpaceDotFilterLabel": "Replace whitespace with dots.",
  "settings.renameSpaceDashFilterLabel": "Replace whitespace with dashes.",
  "settings.renameSpaceRemoveFilterLabel":
    "Remove whitespace from the token value.",
  "settings.renameFilterChainLabel": "Run filters from left to right.",
  "settings.renameTokenTitle": "Title name",
  "settings.renameTokenYear": "Release year",
  "settings.renameTokenTitleWithYear":
    "Title name followed by the year, added only when the name does not already end with it",
  "settings.renameTokenTitleWithoutYear":
    "Title name with any year at the end removed",
  "settings.renameTokenTitleWithArticle":
    "Title with a leading article moved to the end using its metadata language",
  "settings.renameTokenTitleFirstCharacter":
    "First letter or digit after language-aware article sorting; preserves non-Latin characters and falls back to _",
  "rename.namingLanguage": "Naming language: {{language}}",
  "rename.organizingFolderChanges":
    "Titles changing organizing folder: {{count}}",
  "settings.renameTokenQuality": "Quality (e.g. 1080p)",
  "settings.renameTokenEdition": "Edition (e.g. Director's Cut)",
  "settings.renameTokenImdbId": "IMDb ID",
  "settings.renameTokenTmdbId": "TMDb ID",
  "settings.renameTokenTvdbId": "TVDb ID",
  "settings.renameTokenAnidbId": "AniDB ID",
  "settings.renameTokenMalId": "MAL ID",
  "settings.renameTokenAnilistId": "AniList ID",
  "settings.renameTokenSource": "Source (e.g. BluRay, WEB-DL)",
  "settings.renameTokenDynamicRange": "Dynamic range (e.g. HDR10, DV, HDR10+)",
  "settings.renameTokenRevision": "Release revision (Proper/REPACK)",
  "settings.renameTokenIs3d": "3D marker",
  "settings.renameTokenVideoCodec": "Video codec (e.g. x265)",
  "settings.renameTokenAudioCodec": "Audio codec (e.g. DTS)",
  "settings.renameTokenAudioChannels": "Audio channels (e.g. 5.1)",
  "settings.renameTokenSeason": "Season number",
  "settings.renameTokenSeasonOrder": "Season order (narrative position)",
  "settings.renameTokenEpisode": "Episode number",
  "settings.renameTokenAbsoluteEpisode": "Absolute episode number",
  "settings.renameTokenEpisodeTitle": "Episode title",
  "settings.renameTokenGroup": "Release group",
  "settings.renameTokenExt": "File extension",
  "settings.renameValidationUnmatchedOpen": "Unmatched opening bracket '{'",
  "settings.renameValidationUnmatchedClose": "Unmatched closing bracket '}'",
  "settings.renameValidationUnknownToken": "Unknown token: {{token}}",
  "settings.renameValidationInvalidPadding":
    "Numeric padding must be an ASCII integer from 0 to 240: {{padding}}",
  "settings.renameValidationInvalidFilter": "Invalid token filter: {{filter}}",
  "settings.renameValidationEmpty": "Rename template cannot be empty",
  "settings.renameValidationInvalidOptionalGroup": "Invalid optional group",
  "settings.renameValidationNestedOptionalGroup":
    "Optional groups cannot be nested",
  "settings.renameValidationUnsupportedOptionalFallback":
    "Optional groups do not support fallback branches",
  "settings.mediaSettingsSaved": "Media settings saved.",
  "rename.previewButton": "Preview Rename",
  "rename.previewing": "Previewing…",
  "rename.applyButton": "Apply Rename",
  "rename.applying": "Applying…",
  "rename.currentPath": "Current",
  "rename.proposedPath": "Proposed",
  "rename.planSummary":
    "{{total}} item(s), {{renamable}} renamable, {{noop}} noop, {{conflicts}} conflicts, {{errors}} errors",
  "rename.noRenamableFiles": "No files need renaming.",
  "rename.blockedFiles":
    "{{count}} file(s) cannot be renamed and will stop this title from being renamed.",
  "rename.sampleNote":
    "Showing the first {{shown}} of {{renamable}} files to rename.",
  "settings.sectionTitle": "Settings • {{section}}",
  "settings.generalText": "General application settings.",
  "settings.generalPlaceholder":
    "Configure language and other global preferences.",
  "settings.dateTimeFormatLabel": "Date and time format",
  "settings.dateTimeFormatLocale": "Regional default",
  "settings.dateTimeFormatIso24h": "YYYY-MM-DD, 24-hour time",
  "settings.dateTimeFormatHelp":
    "Controls how dates and times are shown in the interface.",
  "settings.historyRetentionTitle": "History retention",
  "settings.historyRetentionHelp":
    "This preserves Scryer’s user-facing history and audit trail. Changes take effect on the next housekeeping run.",
  "settings.historyRetentionExternalHelp":
    "Operational cleanup for acquisition scratch data still runs, and this does not control history retained by SABnzbd, NZBGet, or Weaver.",
  "settings.featuresHeader": "Features",
  "settings.apiExplorerLabel": "Enable API explorer",
  "settings.apiExplorerHelp":
    "Adds GraphiQL to the API navigation item for users with Manage System Settings. Requests run with API-key or OAuth library permissions.",
  "apiExplorer.accessMode": "Access as",
  "apiExplorer.renameTab": "Rename tab",
  "apiExplorer.tabName": "Tab name",
  "apiExplorer.apiKey": "API Key",
  "apiExplorer.oauth": "OAuth App",
  "apiExplorer.apiKeyHelp":
    "Your app and library permissions, without interactive account access. Subscriptions are unavailable.",
  "apiExplorer.oauthHelp":
    "Your library permissions only. App settings and account operations are unavailable.",
  "apiExplorer.sessionHelp":
    "Uses your current session. No credentials or client setup required.",
  "settings.experimentalFeaturesLabel": "Enable experimental features",
  "settings.experimentalFeaturesHelp":
    "Experimental features can change at any time without notice. We make a best effort to preserve compatibility with previous changes, but cannot guarantee it.",
  "settings.personalizedDiscoveryLabel": "Personalized discovery",
  "settings.personalizedDiscoveryHelp":
    "Sends a summary of your library to the metadata gateway so Discovery can recommend titles based on what you own. Turn off to keep Discovery to public feeds only.",
  "settings.srrdbFilenameRecoveryLabel":
    "Recover obfuscated filenames from srrdb.com",
  "settings.srrdbFilenameRecoveryHelp":
    "When an automatic import from SABnzbd or NZBGet contains video files with obfuscated names, Scryer sends each file's CRC-32 checksum and size to srrdb.com, a third-party service, to look up the original filename. Nothing else about your library or the file is sent. Files on disk are never renamed.",
  "discovery.personalizedDisabled":
    "Personalized discovery is turned off for this instance.",
  "settings.keepHistoryForever": "Keep history forever",
  "settings.historyRetentionDaysHeader": "Retain history for",
  "settings.historyRetentionDaysSuffix": "days",
  "settings.securitySessionDurationSuffix": "days",
  "settings.generalSaved": "General settings saved.",
  "settings.verificationDepthTitle": "Import copy verification",
  "settings.verificationDepthHelp":
    "How thoroughly Scryer proves a copied file before it touches the source.",
  "settings.verificationDepthScope":
    "This applies to download-client import copies only. Library and root moves always verify in full, whatever is chosen here.",
  "settings.verificationDepthLabel": "Verification depth",
  "settings.verificationDepthFull": "Full read-back",
  "settings.verificationDepthFullHelp":
    "Reads the copy back in full and compares it against the checksum streamed during the copy.",
  "settings.verificationDepthQuick": "Quick check (default)",
  "settings.verificationDepthQuickHelp":
    "Compares the file size and the sampled head and tail content. Faster, and a weaker guarantee.",
  "settings.verificationDepthSaved": "Verification depth saved.",
  "settings.uiSaved": "Display settings saved.",
  "settings.historyRetentionValidation":
    "History retention days must be at least 1 unless history is kept forever.",
  "settings.imageCacheTitle": "Image cache",
  "settings.imageCacheMaxSizeLabel": "Image cache size",
  "settings.imageCacheMbSuffix": "MiB",
  "settings.imageCacheEnvOverride":
    "SCRYER_IMAGE_CACHE_MAX_BYTES is active. Its value is effective; the saved limit applies after the environment override is removed.",
  "settings.imageCacheMaxSizeValidation":
    "Image cache maximum size must be at least 1 MiB.",
  "settings.clearImageCache": "Clear image cache",
  "settings.clearingImageCache": "Clearing image cache…",
  "settings.imageCacheClearQueued": "Image cache reset queued.",
  "settings.pluginHttpTrustTitle": "Plugin HTTPS trusted certificates",
  "settings.pluginHttpTrustAdvancedLabel": "Advanced feature",
  "settings.pluginHttpTrustStoredCount": "{{count}} stored",
  "settings.pluginHttpTrustDescription":
    "Augment outbound HTTPS trust for WASM plugins with additional private or self-managed certificate authorities.",
  "settings.pluginHttpTrustWarningTitle":
    "Use this only when you trust the certificate authority",
  "settings.pluginHttpTrustWarningBody":
    "These certificates extend HTTPS trust for plugin outbound requests. Only upload certificate authority files that you control or explicitly trust.",
  "settings.pluginHttpTrustUploadButton": "Upload trusted certificates",
  "settings.pluginHttpTrustClearButton": "Clear all",
  "settings.pluginHttpTrustEmpty": "No extra trusted certificates uploaded.",
  "settings.pluginHttpTrustEntryLabel": "Trusted certificate",
  "settings.pluginHttpTrustUploadMissingCertificate":
    "The uploaded file did not contain a PEM certificate block or DER certificate.",
  "settings.pluginHttpTrustUploadTrailingText":
    "The uploaded PEM file contained unexpected non-certificate text.",
  "settings.pluginHttpTrustUploadInvalidCertificate":
    "The uploaded certificate could not be parsed.",
  "settings.pluginHttpTrustUploadReadError":
    "The uploaded certificate file could not be processed.",
  "settings.languageChangeTitle": "Change Language",
  "settings.languageChangeWarning":
    "Changing to {{language}} will re-download metadata for all titles in your library (names, descriptions, sort titles) in the new language. The interface will change immediately, but it may take up to 24h for all language content to change.",
  "settings.languageChangeConfirm": "Change Language",
  "settings.metadataRehydrationStarted":
    "Metadata rehydration started. Titles will update in the background.",

  "theme.label": "Theme",
  "theme.light": "Light",
  "theme.dark": "Dark",
  "theme.system": "System",
  "theme.switchLabel": "Switch theme (current: {{theme}})",

  "search.globalPlaceholder":
    "Search movies, series, anime, actions, and settings…",
  "search.globalPlaceholderNoLibrary":
    "Search movies, series, anime, actions, and settings…",
  "search.overlayPlaceholder":
    "Search your libraries, actions, settings, and everywhere else…",
  "search.overlayPlaceholderNoLibrary":
    "Search movies, series, anime, actions, and settings…",
  "search.shortcutHint": "⌘K",
  "search.shortcutHintControl": "Ctrl+K",
  "search.title": "Global Search",
  "search.subtitle":
    "Search movies, series, anime, your library, actions, and settings.",
  "search.subtitleNoLibrary":
    "Search movies, series, anime, actions, and settings.",
  "search.subtitleWithQuery": "Results for “{{query}}”",
  "search.statusLoading": "Searching for “{{query}}”…",
  "search.statusResultOne": "1 result for “{{query}}”",
  "search.statusResultOther": "{{count}} results for “{{query}}”",
  "search.statusNoResults": "No matches for “{{query}}”",
  "search.tabAll": "All",
  "search.tabLibrary": "In Library",
  "search.inLibrary": "In Library",
  "search.actionsAndSettings": "Actions & Settings",
  "search.alreadyInCollection": "already in your collection",
  "search.resultCountOne": "1 result",
  "search.resultCountOther": "{{count}} results",
  "search.monitored": "Monitored",
  "search.unmonitored": "Unmonitored",
  "search.view": "View",
  "search.viewAll": "View all",
  "search.add": "Add",
  "search.addToAnotherLibrary": "Add to another library",
  "search.requestForAnotherLibrary": "Request for another library",
  "search.chooseLibraryToView": "Choose a library to view {{name}} in",
  "search.footerTip": "Can't find it? Try another term or check your indexers.",
  "search.searchTips": "Search tips",
  "search.tipTitles":
    "Search by movie, series, or anime title, catalog name, action, or setting.",
  "search.tipTitlesNoLibrary":
    "Search by movie, series, or anime title, action, or setting.",
  "search.tipTabs":
    "Use tabs to narrow results to your library or one catalog type.",
  "search.tipTabsNoLibrary": "Use tabs to narrow results to one catalog type.",
  "search.tipIndexers":
    "Use a broader term before checking indexers for titles not in metadata results.",
  "search.noMatchesFor": "No matches for “{{query}}”",
  "search.emptyHint":
    "Try another movie, series, anime title, catalog name, action, or setting.",
  "search.emptyHintNoLibrary":
    "Try another movie, series, anime title, action, or setting.",
  "search.minimumQueryTitle": "Keep typing",
  "search.minimumQueryHint":
    "Type at least 2 characters to search your library and metadata.",
  "search.minimumQueryHintNoLibrary":
    "Type at least 2 characters to search metadata.",
  "search.noCatalogMatches": "No managed catalog matches yet.",
  "search.noMetadataMatches": "No metadata matches in this section yet.",
  "search.alreadyCataloged": "Already in catalog",
  "search.cataloged": "Cataloged",
  "search.unavailable": "Unavailable",
  "search.configureAdd": "Configure add options",
  "search.addConfigLibrary": "Library",
  "search.addConfigQualityProfile": "Quality profile",
  "search.addConfigInheritLibrary": "Inherit library",
  "search.addConfigRootFolder": "Root folder",
  "search.addConfigSeasonFolder": "Season folder",
  "search.addConfigMinAvailabilityNoReleaseDate":
    "No release date is known yet. Automatic searches wait until one is.",
  "search.addConfigMonitorType": "Monitor type",
  "search.addConfigNoQualityProfiles": "No quality profiles configured.",
  "search.seasonFolder.enabled": "Enabled",
  "search.seasonFolder.disabled": "Disabled",
  "search.monitorType.monitored": "Monitored",
  "search.monitorType.unmonitored": "Unmonitored",
  "search.monitorType.futureEpisodes": "Future episodes",
  "search.monitorType.missingAndFutureEpisodes": "Missing + future episodes",
  "search.monitorType.allEpisodes": "All episodes",
  "search.monitorType.none": "None",
  "search.monitorType.advanced": "Advanced",

  "monitorSelection.seasons": "Seasons",
  "monitorSelection.seriesMovies": "Series movies",
  "monitorSelection.specials": "Specials",
  "monitorSelection.season": "Season {{number}}",
  "monitorSelection.seasonNamed": "Season {{number}}: {{name}}",
  "monitorSelection.selectAll": "Select all",
  "monitorSelection.clear": "Clear",
  "monitorSelection.retry": "Retry",
  "monitorSelection.loading": "Loading seasons…",
  "monitorSelection.loadFailed": "Could not load seasons for this title.",
  "monitorSelection.noSeasons": "No seasons were returned for this title.",
  "monitorSelection.unavailable":
    "Advanced monitoring needs a TVDB match, which this result does not have.",
  "monitorSelection.emptyHint": "Pick at least one season or series movie.",
  "monitorSelection.summaryPrefix": "Requested: {{selection}}",
  "monitorSelection.summaryMovies": "Movies: {{movies}}",
  "search.adding": "Adding…",
  "search.request": "Request",
  "search.requesting": "Requesting…",
  "search.facetMovie": "Movie",
  "search.facetSeries": "Series",
  "search.facetAnime": "Anime",
  "search.metadataSearch": "Metadata Search",

  "toast.catalogAdded": "Added to catalog",
  "toast.catalogAddedAutoSearch": "Automatic search queued.",
  "toast.releaseGrabbed": "Release grabbed",
  "toast.titleImported": "Title imported",
  "toast.viewInCatalog": "View in catalog",

  "label.back": "Back",
  "label.close": "Close",
  "label.dismiss": "Dismiss",
  "label.copy": "Copy",
  "label.copied": "Copied",

  "pwa.offline": "You're offline. Some features may be unavailable.",
  "pwa.installApp": "Install Scryer",
  "pwa.iosInstallHint":
    'Tap the share button and select "Add to Home Screen" to install.',
  "smgUpgrade.deprecatedTitle": "Scryer update recommended",
  "smgUpgrade.deprecatedBody":
    "This Scryer version is nearing end of support. Upgrade soon to continue using Scryer.",
  "smgUpgrade.blockedTitle": "Scryer update required",
  "smgUpgrade.blockedBody":
    "This Scryer version is no longer supported. Upgrade now to continue using Scryer.",
  "smgUpgrade.minimumVersion": "Minimum supported version: {{version}}",
  "smgUpgrade.deadline": "Support cutoff: {{date}}",
  "smgUpdate.title": "Scryer update available",
  "smgUpdate.body": "You are running {{current}}. Latest is {{latest}}.",
  "smgUpdate.releaseNotes": "Release notes",

  "title.addAndQueue": "Add Title and Queue",
  "title.addToCatalog": "Add to Catalog",
  "title.name": "Title Name",
  "title.namePlaceholder": "Search TVDB by title",
  "title.facet": "Facet",
  "title.manageMovies": "Manage Movies",
  "title.manageSeries": "Manage Series",
  "title.manageAnime": "Manage Anime",
  "title.monitored": "Monitored",
  "title.unmonitored": "Unmonitored",
  "title.ended": "Ended",
  "title.continuing": "Continuing",
  "title.upcoming": "Upcoming",
  "title.backToFacet": "Back to {{facet}}",
  "title.notFound": "Title not found.",
  "title.noPoster": "No poster",
  "title.addedAt": "Added {{date}}",
  "title.monitorAction": "Monitor",
  "title.unmonitorAction": "Unmonitor",
  "title.renameAction": "Rename",
  "title.filterPlaceholder": "Filter managed titles",
  "title.loading": "No data loaded",
  "title.noManaged": "No titles found for this section.",
  "title.statusTemplate": "Loaded {{count}} title(s).",
  "title.contextPanelTitle": "Title details",
  "title.contextTitleList": "Title list",
  "title.contextListDisclosure": "List",
  "title.showTitleList": "Show list",
  "title.hideTitleList": "Hide list",
  "title.contextForYouTitle": "Discover",
  "title.contextForYouTop": "Top",
  "title.contextForYouTopMoviesThisWeek": "Top Movies This Week",
  "title.contextForYouTopSeriesThisWeek": "Top Series This Week",
  "title.contextForYouTopAnimeThisWeek": "Top Anime This Week",
  "title.contextForYouGenre": "Because you like {{genre}}",
  "title.contextForYouTag": "Because you like {{tag}}",
  "title.contextForYouAcclaimed": "Acclaimed - Not in Your Library",
  "title.contextForYouCompleteCollection": "Complete the Collection",
  "title.contextForYouReasonTop": "High-signal library match",
  "title.contextForYouReasonWeekly": "Weekly library signal",
  "title.contextForYouReasonGenre": "Matches your {{genre}} shelf",
  "title.contextForYouReasonTag": "Matches your {{tag}} shelf",
  "title.contextForYouReasonAcclaimed": "Highly rated and not in your library",
  "title.contextForYouReasonCollection": "Part of a collection you have",
  "title.contextForYouEmptyTitle": "No recommendations yet",
  "title.contextForYouEmptyBody":
    "Add or scan titles to build this library view.",
  "title.topBilledCast": "Top billed cast",
  "title.dubCast": "Dub cast",
  "title.dubCastLanguage": "Language",
  "title.contextMoreLikeThis": "More like this",
  "title.contextBlockedReleases": "Blocked Releases",
  "title.clearBlockedRelease": "Remove {{releaseName}} from the blocklist",
  "title.clearBlockedReleaseHint":
    "Remove this release from the blocklist so it can be grabbed again",
  "title.contextAdded": "Added",
  "title.columns": "Columns",
  "title.toggleColumns": "Toggle columns",
  "title.table.groupCore": "Core",
  "title.table.groupRatings": "Ratings",
  "title.table.groupMedia": "Media",
  "title.table.groupOperational": "Operational",
  "title.table.facet": "Facet",
  "title.table.library": "Library",
  "title.table.monitored": "Monitored",
  "title.table.qualityTier": "Quality",
  "title.table.profile": "Profile",
  "title.table.episodes": "Episodes",
  "title.table.year": "Year",
  "title.table.runtime": "Runtime",
  "title.table.avgRuntime": "Avg runtime",
  "title.table.episodeProgressTooltip": "{{owned}} / {{total}} episodes",
  "title.table.episodeProgressTooltipWithTotal":
    "{{owned}} / {{total}} episodes ({{monitored}} monitored)",
  "title.table.status": "Status",
  "title.table.root": "Root",
  "title.table.popularity": "Popularity",
  "title.table.resolution": "Resolution",
  "title.table.hdr": "HDR",
  "title.table.audioCodec": "Audio codec",
  "title.table.size": "Size",
  "title.viewModeToggle": "View mode",
  "title.viewModeCompact": "Compact",
  "title.viewModePosterTable": "List",
  "title.viewModePoster": "Poster view",
  "title.bulkSelectionCount": "{{count}} selected",
  "title.bulkSelectionPanelTitle": "Bulk selection",
  "title.bulkSelectionPanelHint":
    "Choose an action for the selected titles, or clear the selection.",
  "title.bulkEditTitle": "Edit Selected Titles",
  "title.bulkEditDescription": "Apply changes to {{count}} selected titles.",
  "title.editOptionsTitle": "Edit Title Options",
  "title.editOptionsDescription": "Change options for {{name}}.",
  "title.bulkDeleteTitle": "Delete Selected Titles",
  "title.bulkDeleteDescription":
    "Remove {{count}} selected titles from the catalog.",
  "title.bulkRenameTitle": "Rename Selected Titles",
  "title.bulkRenameDescription":
    "Preview and apply renames for {{count}} selected titles.",
  "title.selectTitle": "Select {{name}}",
  "title.selectAllTitles": "Select all visible titles",
  "title.seasonMonitored": "Season monitored",
  "title.episodeMonitored": "Episode monitored",
  "title.seriesMovieMonitored": "Series movie monitored",
  "title.deleteFilesOnDisk": "Permanently delete files from disk",
  "title.seasonsAndEpisodes": "Seasons and Episodes",
  "title.noTrackedSeasons": "No seasons are tracked for this show yet.",
  "title.fetchingData": "Fetching data",
  "title.filesOnDisk": "Files on Disk",
  "title.files": "Files",
  "title.noFilesTracked": "No files tracked.",
  "title.noFilesTrackedHint": "Run a library scan to detect files on disk.",
  "title.configureRootFoldersHint":
    "Configure your root data folders in facet settings before scanning this library.",
  "title.invalidRootFoldersTitle": "One or more root folders are invalid.",
  "title.invalidRootFoldersHint":
    "Fix your configured root data folders in facet settings before running a library scan.",
  "title.configureRootFoldersButton": "Configure root data folders",
  "title.descriptionUnavailable": "No description available.",
  "title.canon": "Canon",
  "title.mixed": "Mixed",
  "title.specials": "Specials",
  "title.seasonNumber": "Season {{number}}",
  "title.episodeRange": "Episodes {{start}} - {{end}}",
  "title.episodeCountOne": "{{count}} episode",
  "title.episodeCountOther": "{{count}} episodes",
  "title.contextReleaseSearchSummary":
    "{{releaseCount}} releases · {{indexerCount}} indexers searched",
  "title.contextReleaseSearchProgress":
    "{{releaseCount}} releases · {{done}}/{{total}} indexers",
  "title.contextReleaseSearchSummaryDetailed":
    "{{releaseCount}} releases · {{searched}}/{{total}} indexers searched",
  "title.contextReleaseSearchIndexerFailures":
    "{{count}} indexer(s) failed: {{names}}",
  "title.contextReleaseSearchIndexerSkipped":
    "{{count}} indexer(s) skipped: {{names}}",
  "title.noReleasesFound": "No releases found for {{name}}.",
  "title.searchNeedsDownloadClientTitle": "Search requires a download client",
  "title.searchNeedsDownloadClientDescription":
    "Configure at least one download client before using search or interactive search from this title page.",
  "title.searchNeedsDownloadClientAction": "Open Download Clients",
  "title.blockedReleases": "Blocked Releases",
  "title.noBlockedReleases": "No blocked releases recorded for this movie.",
  "title.qualityProfile": "Quality Profile",
  "title.rootFolder": "Root Folder",
  "title.catalogFilters.allRootFolders": "All root folders",
  "title.catalogFilters.selectedCount": "{{count}} selected",
  "title.catalogFilters.minimumYear": "Minimum release year",
  "title.catalogFilters.maximumYear": "Maximum release year",
  "title.catalogFilters.loadError": "Filter options could not be refreshed.",
  "title.catalogFilters.retry": "Retry filter options",
  "title.defaultRootFolder": "Default ({{path}})",
  "title.inheritDefault": "Inherit Default",
  "title.monitoredSection": "Monitored {{facet}}",
  "title.searchReleasesAction": "Search Releases",
  "title.queueLatest": "Queue Latest",
  "title.requiredAudioLanguages": "Required Audio Languages",
  "title.originalAudioLanguagePerTitle": "Original language (per title)",
  "title.requiredAudioLanguagesFacetInfo":
    "These languages are enforced during release picking and import validation for this facet.",
  "title.requiredAudioResetInherit": "Reset to inherit",
  "title.fixMatchHeading": "Fix metadata match",
  "title.fixMatchAction": "Fix Match",
  "title.fixMatchDescriptionMovie":
    "Repair the TVDB match for this movie. Local title and folder naming stay the same.",
  "title.fixMatchDescriptionSeries":
    "Repair the TVDB match for this title. Local title and folder naming stay the same.",
  "title.fixMatchDialogTitle": "Fix Match",
  "title.fixMatchDialogDescription":
    "Repair the TVDB match for {{name}}. The local title and folder name stay the same. Series and anime will rebuild seasons and episodes internally.",
  "title.fixMatchUnnamed": "this title",
  "title.fixMatchSearchPlaceholder": "Search TVDB titles",
  "title.fixMatchCurrentTvdbId": "Current TVDB ID",
  "title.fixMatchCurrentTvdbNone": "none",
  "title.fixMatchSearching": "Searching metadata…",
  "title.fixMatchNoResults": "No metadata matches found.",
  "title.fixMatchSelected": "Selected",
  "title.fixMatchChoose": "Choose",
  "title.fixMatchApplying": "Fixing Match…",
  "title.fixMatchApply": "Apply Match",
  "title.fixMatchSearchFailed": "Failed to search metadata.",
  "title.fixMatchApplyFailed": "Failed to fix title match.",

  "title.changeFolderHeading": "Change folder",
  "title.changeFolderDescription":
    "Correct which existing folder this title owns. No files are moved.",
  "title.changeFolderAction": "Change Folder",
  "title.changeFolderDialogTitle": "Change folder",
  "title.changeFolderDialogDescription":
    "Pick the folder that belongs to {{name}}. Scryer reassigns ownership and rescans the folder; no files are moved.",
  "title.changeFolderLibrary": "Library",
  "title.changeFolderCurrentFolder": "Current folder",
  "title.changeFolderCurrentFolderUnknown": "Shown once you select a folder",
  "title.changeFolderCurrentRoot": "Current root",
  "title.changeFolderCurrentRootUnknown": "Unknown",
  "title.changeFolderRootLabel": "Root",
  "title.changeFolderRootScopeHelp":
    "Only folders inside this title's library roots can be selected.",
  "title.changeFolderNoRoots": "This title's library has no roots to browse.",
  "title.changeFolderBrowseLoading": "Loading folders…",
  "title.changeFolderBrowseEmpty": "No subfolders here.",
  "title.changeFolderBrowseFailed": "Failed to list folders.",
  "title.changeFolderUp": "Up one level",
  "title.changeFolderOpen": "Open",
  "title.changeFolderSelectPrompt": "Select a folder to preview the change.",
  "title.changeFolderPreviewLoading": "Checking the selected folder…",
  "title.changeFolderPreviewFailed": "Failed to preview the folder change.",
  "title.changeFolderSelectedFolder": "Selected folder",
  "title.changeFolderOwnershipUnowned": "No title owns this folder.",
  "title.changeFolderOwnershipThisTitle":
    "This title already owns this folder.",
  "title.changeFolderOwnershipAnother": "{{name}} owns this folder.",
  "title.changeFolderNoOpHelp":
    "Nothing will be submitted. Pick a different folder or close this dialog.",
  "title.changeFolderTrackedMediaCurrent":
    "Tracked media in the current folder: {{count}}",
  "title.changeFolderTrackedMediaSelected":
    "Tracked media in the selected folder: {{count}}",
  "title.changeFolderNoFilesMoved":
    "No files will be moved. Only the folder match and the media associations derived from it change.",
  "title.changeFolderConflictHeading":
    "This folder already belongs to another title",
  "title.changeFolderChooseResolution":
    "Choose Swap folders or Take over folder to continue.",
  "title.changeFolderSwapAction": "Swap folders",
  "title.changeFolderSwapExplanation":
    "{{owner}} takes this title's current folder and this title takes {{folder}}. Both titles are rescanned.",
  "title.changeFolderTakeOverAction": "Take over folder",
  "title.changeFolderTakeOverExplanation":
    "This title takes {{folder}}. {{owner}} is left without a folder, becomes unmatched, and needs repair in Pending Imports.",
  "title.changeFolderConfirm": "Change folder",
  "title.changeFolderApplying": "Changing folder…",
  "title.changeFolderApplyFailed": "Failed to change the folder match.",
  "title.changeFolderOutcomeAssigned": "{{name}} now owns {{folder}}.",
  "title.changeFolderOutcomeSwapped": "{{name}} and {{other}} traded folders.",
  "title.changeFolderOutcomeAlreadyOwned":
    "{{name}} already owned that folder; nothing changed.",
  "title.changeFolderDetachedMedia":
    "{{count}} media associations were detached.",
  "title.changeFolderScanSummary":
    "Scan: {{scanned}} scanned • {{matched}} matched • {{imported}} imported • {{unmatched}} unmatched",
  "title.changeFolderDisplacedNote":
    '{{name}} lost {{folder}} and is now unmatched. Repair it in Pending Imports, where it appears with the reason "Folder ownership changed by user".',

  "pendingImports.title": "Pending Imports",
  "pendingImports.description":
    "Resolve unmatched on-disk items for {{facet}} by selecting the correct metadata match.",
  "pendingImports.unresolvedCountOne": "1 unresolved",
  "pendingImports.unresolvedCountOther": "{{count}} unresolved",
  "pendingImports.loading": "Loading pending imports…",
  "pendingImports.loadFailed": "Failed to load pending imports.",
  "pendingImports.empty": "No pending imports for this facet.",
  "pendingImports.path": "Path",
  "pendingImports.searchPlaceholder": "Search metadata titles",
  "pendingImports.searchYearLabel": "Year",
  "pendingImports.searchYearPlaceholder": "Year",
  "pendingImports.searchYearClear": "Clear year",
  "pendingImports.searchYearHint":
    "The year comes from the scan. Clear it to widen the search when the filename year is wrong.",
  "pendingImports.searching": "Searching metadata…",
  "pendingImports.searchFailed": "Failed to search metadata.",
  "pendingImports.searchRequestFailed": "Metadata search failed: {{error}}",
  "pendingImports.noSearchResults": "No metadata matches found.",
  "pendingImports.searchAction": "Search & Match",
  "pendingImports.match": "Match",
  "pendingImports.alreadyInLibrary": "Already in this library",
  "pendingImports.attachToExistingTitle": "Attach to existing title",
  "pendingImports.attachSuccess": "Attached pending import to {{name}}.",
  "pendingImports.ignoredSection": "Ignored Imports",
  "pendingImports.ignore": "Ignore",
  "pendingImports.ignoreConfirmTitle": "Ignore import?",
  "pendingImports.ignoreSuccess": "Ignored import for {{name}}.",
  "pendingImports.ignoreFailed": "Failed to ignore pending import.",
  "pendingImports.resolveSuccess": "Resolved pending import for {{name}}.",
  "pendingImports.resolveFailed": "Failed to resolve pending import.",
  "pendingImports.prev": "Previous",
  "pendingImports.next": "Next",
  "pendingImports.pageRange": "{{start}}–{{end}} / {{total}}",
  "pendingImports.library": "Library:",
  "pendingImports.knownTitle": "Known title:",
  "pendingImports.reason.invalid_title_folder_ownership":
    "Title folder needs recovery",
  "pendingImports.invalidOwnershipHelp":
    "This title incorrectly claims a library root or its parent. Check title folder recovery in Activity. Original files are preserved; do not move files into the recorded root.",
  "pendingImports.reason.title_already_owns_another_folder":
    "Title already owns another folder",
  "pendingImports.reason.metadata_id_lookup_unresolved":
    "Metadata lookup could not resolve this folder's external id; the next scan retries it",
  "pendingImports.reason.folder_ownership_changed_by_user":
    "Folder ownership changed by user",
  "pendingImports.folderOwnershipChangedHelp":
    "This folder was given to another title, so this title no longer has one. Open the title and use Change folder to point it at the right folder, or use Ignore to hide this item.",
  "pendingImports.ownershipConflictHelp":
    "This title is assigned to {{folder}}. Move this folder's files there, or delete the duplicate folder, then rescan.",
  "pendingImports.bindEpisodes": "Bind Episodes",
  "pendingImports.bindPreviewLoadFailed": "Failed to load binding preview",
  "pendingImports.bindFailed": "Failed to bind pending import.",
  "pendingImports.selectAtLeastOneEpisode": "Select at least one episode.",
  "pendingImports.loadingEpisodeBindings": "Loading episode bindings...",
  "pendingImports.parsedHints": "Parsed hints:",
  "pendingImports.parsedHintSeason": " season {{season}}",
  "pendingImports.parsedHintEpisodes": " • episodes {{episodes}}",
  "pendingImports.parsedHintAbsolute": " • absolute {{absolute}}",
  "pendingImports.useSuggested": "Use Suggested",
  "pendingImports.specials": "Specials",
  "pendingImports.seasonNumber": "Season {{number}}",
  "pendingImports.selectSeason": "Select Season",
  "pendingImports.bindSelectedEpisodes": "Bind Selected Episodes",
  "pendingImports.episodeNumberLabel": "Episode {{number}}",

  "tvdb.searchResults": "Metadata Search Results",
  "tvdb.searchPrompt": "Search metadata by entering a title above.",
  "tvdb.searchByTvdb": "Search metadata",

  "nzb.searchResultsFor": "NZB results for {{name}}",
  "nzb.noResultsYet": "No release matches yet.",
  "nzb.currentFile": "Current file",
  "nzb.searchFailed": "Release search failed.",
  "nzb.queue": "Queue",
  "nzb.movieAvailabilityWarning": "Automatic search is waiting: {{reason}}",
  "nzb.queueAdditionalFile": "Queue as additional file",
  "nzb.noConsideredResults": "No releases passed the quality profile.",
  "nzb.showScoringLog": "Show scoring log",
  "nzb.hideScoringLog": "Hide scoring log",
  "nzb.score": "Score",
  "nzb.recommended": "Recommended",
  "nzb.size": "Size",
  "nzb.scoringLog": "Scoring log",
  "nzb.total": "Total",

  "media.posterAlt": "{{name}} poster",

  "activity.filterBarLabel": "Download activity filter",
  "activity.import": "Import",
  "activity.activity": "Activity",
  "activity.history": "History",
  "activity.importFilter.importing": "Importing",
  "activity.importFilter.pending": "Pending",
  "activity.importFilter.blocked": "Blocked",
  "activity.importFilter.failed": "Failed",
  "activity.activityFilter.downloading": "Downloading",
  "activity.activityFilter.queued": "Queued",
  "activity.activityFilter.paused": "Paused",
  "activity.activityFilter.postProcessing": "Post-Processing",
  "activity.activityFilter.seeding": "Seeding",
  "activity.activityFilter.warning": "Warning",
  "activity.historyFilter.all": "All",
  "activity.historyFilter.success": "Success",
  "activity.historyFilter.failed": "Failed",
  "activity.scryerSubmitted": "Scryer submitted",
  "activity.selectAllImportItems": "Select all import items",
  "activity.selectImportItem": "Select import item",
  "activity.selectedImportCount": "{{count}} selected",
  "activity.importEmpty": "No import items.",
  "activity.activityEmpty": "No active downloads.",
  "activity.historyEmpty": "No history items.",
  "activity.queueStale":
    "Queue data is stale; showing the last successful refresh.",
  "queue.title": "Title",
  "queue.client": "Client",
  "queue.status": "Status",
  "queue.source": "Source",
  "queue.progress": "Progress",
  "queue.size": "Size",
  "queue.state.queued": "Queued",
  "queue.state.downloading": "Downloading",
  "queue.state.verifying": "Verifying",
  "queue.state.repairing": "Repairing",
  "queue.state.extracting": "Extracting",
  "queue.state.postProcessing": "Post-Processing",
  "queue.state.paused": "Paused",
  "queue.state.completed": "Completed",
  "queue.state.importedSeeding": "Imported · Seeding",
  "queue.state.importing": "Importing",
  "queue.state.importPending": "Import Pending",
  "queue.state.waitingForDiskSpace": "Waiting for disk space",
  "activity.refreshTimedOut": "Refresh timed out",
  "activity.refreshBehind":
    "Queue refresh did not reach the latest update. Retry.",
  "activity.retry": "Retry",
  "activity.loadMore": "Load more",
  "activity.staleData": "Showing the last successful result. Refresh failed.",
  "activity.countUnknown": "Import attention count is not available yet",
  "activity.countStale": "Import attention count may be out of date",
  "queue.state.importBlocked": "Import Blocked",
  "queue.state.warning": "Warning",
  "queue.state.failed": "Failed",
  "queue.state.ignored": "Ignored",
  "queue.state.unknown": "Unknown",
  "queue.manualImport": "Manual Import",
  "queue.manualImporting": "Queueing…",
  "queue.transfer.copying": "Copying",
  "queue.transfer.extracting": "Extracting",
  "queue.transfer.finalizing": "Finalizing",
  "queue.manualImportQueued": "Manual import queued.",
  "queue.manualImportFailed": "Manual import failed.",
  "queue.manualImportMultipleEpisodes": "Several episodes in this file",
  "queue.manualImportMultipleEpisodesLabel":
    "Assign several episodes to this file",
  "queue.manualImportApplySuggestions": "Apply suggestions to all",
  "queue.removeFailed": "Remove Failed",
  "queue.manualImportTooltip":
    "Manual import is available for attention-required queue items.",
  "queue.releaseTitle": "Release",
  "queue.blockReason": "Block Reason",
  "queue.blockReasonFallbackUnassigned":
    "Automatic import could not identify a library title. Assign a title to continue.",
  "queue.blockReasonFallbackEpisodic":
    "Automatic import could not determine a unique season and episode mapping. Open Manual Import and assign the correct season and episode.",
  "queue.blockReasonFallbackReview":
    "Automatic import needs operator review. Open Manual Import and confirm the file mapping to continue.",
  "queue.seeding.stateSeeding": "Seeding",
  "queue.seeding.stateGoalMet": "Seed goal met",
  "queue.seeding.stateHeldPrivate": "Seeding · private",
  "queue.seeding.stateNeverRemove": "Seeding forever",
  "queue.seeding.ratio": "Ratio {{value}}",
  "queue.seeding.seedTime": "Seeded {{value}}",
  "queue.seeding.private": "Private tracker",
  "queue.seeding.privateTooltip":
    "Private tracker. Scryer never removes this torrent from the client on its own unless a seeding profile goal is met.",
  "queue.errorCode": "Error Code",
  "queue.showDetails": "Show details",
  "queue.hideDetails": "Hide details",
  "queue.assignTitle": "Assign Title",
  "queue.reassignTitle": "Change Title",
  "queue.assignTitleTitle": "Assign Download Title",
  "queue.assignTitleDescription":
    "Choose the library title this blocked download belongs to. Scryer will re-check the import after assignment.",
  "queue.assignTitlePlaceholder": "Search titles...",
  "queue.assignTitleResults": "Matching titles",
  "queue.assignTitleEmpty": "No matching titles found.",
  "queue.assignTitleSearchHint": "Type a title name to search…",
  "queue.assignTitleQueued": "Title assigned. Import will be re-evaluated.",
  "queue.assignTitleFailed": "Failed to assign title.",
  "queue.assignTitleBeforeImport":
    "Assign a title before starting manual import.",
  "queue.ignore": "Ignore",
  "queue.ignoreSuccess": "Download ignored.",
  "queue.ignoreFailed": "Failed to ignore download.",
  "queue.releaseHeldSources": "Release held sources",
  "queue.releaseHeldSourcesConfirmTitle": "Release held sources?",
  "queue.releaseHeldSourcesConfirmDescription":
    "{{download}} in {{client}} finished importing, but its sources are still being held. Releasing clears the hold on every import of this download. Temporary extraction folders are removed only when every video in them was imported and nothing else is using them.",
  "queue.releaseHeldSourcesClientRemoves":
    "{{client}} is set to remove completed downloads. Once Scryer confirms the import, this download and its files will be removed from the download client.",
  "queue.releaseHeldSourcesClientKeeps":
    "{{client}} is set to keep completed downloads, so this download stays in the download client.",
  "queue.releaseHeldSourcesSuccess": "Held sources released.",
  "queue.releaseHeldSourcesFailed": "Failed to release held sources.",
  "queue.releaseHeldSourcesReasonSubtitlesPending":
    "The sources are being held for subtitles that still need attention. Releasing stops waiting for them and clears the warning.",
  "queue.releaseHeldSourcesReasonSourceCleanupIncomplete":
    "The sources are being held because removing the files this import replaced did not finish.",
  "queue.releaseHeldSourcesReasonArchiveExtractionFailed":
    "Archive extraction failed for this download, and its unextracted archives were never imported. Releasing marks the download imported, which lets the download client remove it, only if every file it should contain is proven imported; anything never imported is removed with it.",
  "queue.releaseHeldSourcesReasonUnknown":
    "Scryer has no record of why these sources are being held. Releasing marks the download imported, which lets the download client remove it, only if every file it should contain is proven imported; anything in it that was never imported is removed with it.",
  "queue.releaseHeldSourcesTitles": "This release covers: {{titles}}",
  "queue.releaseHeldSourcesClientRemovesAfterSeeding":
    "{{client}} removes completed downloads once their seeding requirements are met. Once Scryer confirms the import, this download and its files will be removed when seeding finishes.",
  "queue.releaseHeldSourcesClientPolicyUnknown":
    "Scryer could not determine what {{client}} will do with this download after release.",
  "queue.releaseHeldSourcesAwaitingImport":
    "Held sources released. The download is not fully imported, so it went back to import and nothing was removed.",
  "queue.releaseHeldSourcesNotSettled":
    "Held sources released, but the download could not be updated. Release again to finish.",
  "queue.releaseHeldSourcesUnproven":
    "Held sources released, but nothing proves every file of the download was imported, so it was not marked imported and nothing was cleaned up. Import the rest or remove it yourself.",
  "queue.releaseHeldSourcesNotSettledNothingCleaned":
    "Held sources released, but the download was not settled and nothing was cleaned up.",
  "queue.releaseHeldSourcesWorkspacesPreserved":
    "Some temporary extraction folders were kept: {{reasons}}.",
  "queue.releaseHeldSourcesPreservedNotOwned":
    "ownership could not be confirmed",
  "queue.releaseHeldSourcesPreservedUnsafe":
    "unexpected or unreadable contents",
  "queue.releaseHeldSourcesPreservedHoldsUnimportedVideo":
    "contains a video that was never imported",
  "queue.releaseHeldSourcesPreservedInUse": "still needed by another import",
  "queue.releaseHeldSourcesPreservedUnverified":
    "imported files could not be confirmed",
  "queue.releaseHeldSourcesPreservedRemovalFailed": "removal failed",
  "queue.releaseHeldSourcesPreservedDownloadNotImported":
    "the download was not proven imported",
  "queue.releaseHeldSourcesPreservedLookupIncomplete":
    "not every folder could be checked",
  "queue.markFailedSearchAgain": "Blocklist, Delete Data and Search Again",
  "queue.markFailedOnly": "Blocklist and Delete Data",
  "queue.markFailedSearchSuccess":
    "Blocklist and data removal requested; reacquisition enabled.",
  "queue.markFailedOnlySuccess":
    "Blocklist and data removal requested without reacquisition.",
  "queue.markFailedFailed": "Failed to mark download failed.",
  "queue.bulkIgnoreSuccess": "Ignored {{count}} downloads.",
  "queue.bulkIgnorePartial": "Ignored {{count}} downloads; {{failed}} failed.",
  "queue.bulkIgnoreFailed": "Failed to ignore selected downloads.",
  "queue.pause": "Pause",
  "queue.pauseSuccess": "Download paused.",
  "queue.pauseFailed": "Failed to pause download.",
  "queue.resume": "Resume",
  "queue.resumeSuccess": "Download resumed.",
  "queue.passwordRetryAccepted": "Password updated and retry accepted.",
  "queue.passwordRetryRefused":
    "The download client refused the retry. The download has been retained.",
  "queue.passwordRetryUncertain":
    "The retry outcome is uncertain. The download is retained while awaiting reconciliation; another retry has not been sent.",
  "queue.passwordRetryFailed":
    "The password retry could not be started. Refresh the job to check its current state.",
  "queue.resumeFailed": "Failed to resume download.",
  "queue.deleting": "Deleting…",
  "queue.deleteQueued": "Removal from downloader queued.",
  "queue.deleteFailed": "Failed to remove download from downloader.",
  "queue.removeFromDownloader": "Remove from downloader",
  "queue.deleteConfirmTitle": "Remove from downloader",
  "queue.deleteConfirmDescription":
    "Remove this download from the downloader? It will be ignored after the downloader confirms removal.",
  "queue.bulkDeleteQueued":
    "Removal from downloader queued for {{count}} downloads.",
  "queue.bulkDeletePartial":
    "Removal queued for {{count}} downloads; {{failed}} failed.",
  "queue.bulkDeleteFailed": "Failed to delete selected downloads.",
  "queue.bulkDeleteConfirmTitle": "Remove selected from downloader",
  "queue.bulkDeleteConfirmDescription":
    "Remove {{count}} selected downloads from the downloader? They will be ignored after removal succeeds.",
  "queue.manualUnavailableForResult":
    "Manual queue unavailable for this result.",
  "queue.belowMinimumSeeders": "Too few seeders for this indexer's minimum.",

  "importHistory.retry": "Retry import",
  "importHistory.retryHelp":
    "Reevaluate existing downloaded files using the current title and quality profile.",
  "importHistory.retryWithPassword": "Retry with password",
  "importHistory.passwordRequired": "Password Required",
  "importHistory.passwordPlaceholder": "Archive password",
  "importHistory.showPassword": "Show password",
  "importHistory.hidePassword": "Hide password",
  "importHistory.retrySuccess": "Import retry started",

  "system.title": "System Health",
  "system.jobsTitle": "Jobs",
  "system.auditTitle": "Audit",
  "system.auditLoaded": "Audit log loaded.",
  "system.auditEmpty": "No audit events found.",
  "system.auditTarget": "Target",
  "system.auditStream": "Stream",
  "system.notLoaded": "System health not loaded.",
  "system.refreshing": "Refreshing",
  "system.loaded": "System state loaded.",
  "system.notReady": "Service not ready.",
  "system.serviceReady": "Service ready",
  "system.monitoredTitlesLabel": "Monitored titles",
  "system.dbPathLabel": "DB path",
  "system.totalTitlesLabel": "Total titles",
  "system.usersLabel": "Users",
  "system.facetLabel": "Facets",
  "system.recentEventSample": "Recent event sample",
  "system.sourcesTitle": "Data sources",
  "system.sourcesSupport":
    "If these services help power your library, please consider supporting them with a donation if you can.",
  "system.sourceTvdbName": "TheTVDB",
  "system.sourceTmdbName": "The Movie Database (TMDB)",
  "system.sourceMalName": "MyAnimeList",
  "system.sourceAniBridgeName": "AniBridge",

  "settings.knownUsers": "Known users",
  "settings.user": "User",
  "settings.createUser": "Create user",
  "settings.username": "Username",
  "settings.password": "Password",
  "settings.passwordMinLengthError":
    "Password must be at least {{min}} characters.",
  "settings.permissions": "Permissions",
  "settings.noUsers": "No users loaded.",
  "settings.updatePassword": "Update",
  "settings.temporaryPassword": "Temporary password",
  "settings.resetPassword": "Reset password",
  "settings.setTemporaryPassword": "Set temporary password",
  "settings.temporaryPasswordHelp":
    "The user must replace this password after local sign-in. Send it through a safe out-of-band channel.",
  "settings.passwordChangeRequired": "Password change required",
  "settings.temporaryPasswordResetTitle": "Set a temporary password?",
  "settings.temporaryPasswordResetDescription":
    "This revokes {{name}}’s active sessions and requires them to choose a new password after local sign-in.",
  "settings.apiKey": "API Key",
  "settings.existingIndexers": "Existing indexers",
  "settings.indexerProvider": "Provider",
  "settings.indexerInteractiveSearch": "Interactive",
  "settings.indexerAutoSearch": "Auto",
  "settings.indexerStatus": "Status",
  "settings.indexerDisabledUntil": "Disabled until {{time}}",
  "settings.indexerCoolingDownUntil": "Cooling down · resumes {{time}}",
  "settings.indexerCoolingDownHelp":
    "The indexer asked Scryer to slow down. Searching resumes automatically.",
  "settings.indexerLastError": "Error {{time}}",
  "settings.indexerLastSearched": "Searched {{time}}",
  "settings.indexerNoActivity": "No activity",
  "settings.indexerSyncNow": "Sync now",
  "settings.indexerUpdate": "Update Indexer",
  "settings.indexerCreate": "Connect an indexer",
  "settings.indexerCreateNew": "Connect an indexer",
  "settings.indexerManagedParentHint":
    "Prowlarr parent configs are management-only. Search modes come from synced child indexers.",
  "settings.indexerConfirmDiscardTitle": "Discard current indexer edits?",
  "settings.indexerConfirmDiscardDescription":
    "Your current indexer edits will be lost.",
  "settings.indexerFilterPlaceholder": "Filter by provider type",
  "settings.noIndexersFound": "No indexers found.",
  "settings.indexerConfig": "Provider Configuration",
  "settings.indexerDynamicFieldsLoading":
    "Loading the fields for this selection…",
  "settings.indexerDynamicFieldsError":
    "Couldn't load the fields for this selection: {{error}}",
  "settings.indexerLoginChallengeLoading": "Loading the sign-in image…",
  "settings.indexerLoginChallengeImageAlt": "Sign-in image",
  "settings.indexerLoginChallengeError":
    "Couldn't load the sign-in image: {{error}}",
  "settings.indexerLoginChallengeRefresh": "New image",
  "settings.indexerLoginChallengeRefusal.malformed":
    "The plugin's sign-in image could not be read.",
  "settings.indexerLoginChallengeRefusal.kindUnsupported":
    "This indexer asks for a kind of sign-in check Scryer can't show.",
  "settings.indexerLoginChallengeRefusal.imageTypeUnsupported":
    "The sign-in image is in a format Scryer doesn't show.",
  "settings.indexerLoginChallengeRefusal.imageTooLarge":
    "The sign-in image is too large to show.",
  "settings.advancedConfig": "Advanced",
  "settings.managedIndexerBadge": "Managed",
  "settings.managedByIndexer": "Managed by {{name}}",
  "settings.managedByParent": "Managed by parent",
  "settings.managesIndexerCount": "Manages {{count}} indexers",
  "settings.managedIndexerReadOnly":
    "Managed indexers are controlled by their parent sync and cannot be edited directly.",
  "settings.managedIndexerReadOnlyShort": "Read-only",
  "settings.indexerRateLimits": "Rate limits",
  "settings.indexerDownloadClient": "Download client",
  "settings.indexerDownloadClientLabel": "Download client for {{name}}",
  "settings.indexerDownloadClientAutomatic": "Automatic",
  "settings.indexerDownloadClientLoading": "Loading download clients…",
  "settings.indexerDownloadClientLoadRetry": "Retry loading download clients",
  "settings.indexerDownloadClientCatalogStale":
    "Download-client options may be out of date.",
  "settings.indexerDownloadClientSelectionInvalid":
    "Choose Automatic or a download client compatible with this indexer.",
  "settings.indexerDownloadClientNotApplicable": "Not applicable",
  "settings.indexerDownloadClientDisabled": "Disabled",
  "settings.indexerDownloadClientDisabledWarning":
    "{{name}} is disabled. Grabs will fail until it is enabled.",
  "settings.indexerDownloadClientChooseAutomatic": "Choose Automatic",
  "settings.indexerDownloadClientInvalidOption": "Invalid: {{name}}",
  "settings.indexerDownloadClientInvalidMissing":
    "The mapped download client is missing. Choose a valid client or Automatic.",
  "settings.indexerDownloadClientInvalidIncompatible":
    "The mapped download client is incompatible with this indexer.",
  "settings.indexerDownloadClientInvalidUnavailable":
    "The mapped download client is unavailable. Choose a valid client or Automatic.",
  "settings.downloadClientDeleteConfirmDescription":
    "Delete {{name}}? Any indexers mapped to it will revert to Automatic.",
  "settings.newPassword": "New Password",
  "settings.passwordManagedExternally": "Managed externally",
  "settings.mfa": "MFA",
  "settings.passkey": "Passkey",
  "settings.setUp": "Set up",
  "settings.notSetUp": "Not set up",
  "settings.resetMfa": "Reset authentication factors",
  "settings.resetMfaConfirm":
    "Reset authentication factors for {{name}}? All active sessions and registered authentication factors will be revoked. They will need their primary credentials again. If the login policy requires MFA, they will need to enroll a new passkey or authenticator; otherwise their next password login will succeed without enrollment.",
  "settings.loginEnabled": "Login enabled",
  "settings.loginDisabled": "Login disabled",
  "settings.enableLogin": "Enable login",
  "settings.disableLogin": "Disable login",
  "settings.disableLoginConfirm":
    "Disable login for {{name}}? Their credentials and factors will be retained, but existing sessions will be revoked.",

  "settings.rules": "Custom Rules",
  "settings.noRulesFound": "No custom rules configured.",
  "settings.trashPacksTitle": "TRaSH Guides Locale Packs",
  "settings.trashPacksHelp":
    "Score-only packs derived from TRaSH Guides. Select a pack to apply its predefined locale policy.",
  "settings.trashPackFrenchConflict":
    "“{{name}}” is already enabled. Disable it before enabling another French pack.",
  "settings.ruleCreate": "Create Rule",
  "settings.ruleCreateNew": "Create new rule",
  "settings.ruleCopyAsCustom": "Copy as custom rule",
  "settings.ruleUpdate": "Update Rule",
  "settings.ruleApplyTemplate": "Apply template",
  "settings.ruleConfirmDiscardTitle": "Discard current rule edits?",
  "settings.ruleConfirmDiscardDescription":
    "Your current rule edits will be lost.",
  "settings.ruleDescription": "Description",
  "settings.ruleRegoSource": "Rego Source",
  "settings.rulePriority": "Priority",
  "settings.ruleAppliedFacets": "Applied Facets",
  "settings.ruleAppliedFacetsHelp": "Leave empty to apply to all media types.",
  "settings.ruleValidate": "Validate",
  "settings.ruleValidating": "Validating…",
  "settings.ruleValid": "Rule is valid.",
  "settings.ruleValidationRequired": "Name and Rego source are required.",
  "settings.ruleLibrary": "Rule Library",
  "settings.ruleLibraryDescription":
    "Pre-built rules you can add with one click. Select a template to populate the editor.",
  "settings.ruleLibraryAll": "All",
  "settings.ruleLibraryCommunityEmpty":
    "No community rule packs available. Refresh the plugin registry to check for new packs.",
  "settings.arrImportAction": "Import Arr custom format",
  "settings.ruleAiPrompt": "AI prompt",
  "settings.ruleAiPromptTitle": "Create a rule with AI",
  "settings.ruleAiPromptDescription":
    "Describe the result you want, then copy the prompt into an AI assistant.",
  "settings.ruleAiPromptOutcome": "Desired outcome",
  "settings.ruleAiPromptOutcomePlaceholder":
    "For example: Give 100 points to releases with Japanese audio.",
  "settings.ruleAiPromptCopy": "Copy prompt",
  "settings.ruleAiPromptCopied": "Copied",
  "settings.arrImportTitle": "Import Arr custom formats",
  "settings.arrImportDescription":
    "Paste a Sonarr or Radarr custom-format export to create a new Rego rule draft.",
  "settings.arrImportSource": "Source application",
  "settings.arrImportJson": "Custom-format JSON",
  "settings.arrImportJsonPlaceholder":
    "Paste one custom format or an array of custom formats.",
  "settings.arrImportReviewAction": "Review formats",
  "settings.arrImportReview": "{{count}} custom format(s) ready to translate",
  "settings.arrImportNoFormats":
    "No valid custom formats were found in this JSON.",
  "settings.arrImportInvalidJson":
    "Paste valid JSON exported by Sonarr or Radarr.",
  "settings.arrImportReviewFailed": "Unable to review these custom formats.",
  "settings.arrImportScore": "Score",
  "settings.arrImportScoreRequired":
    "Enter an integer score for {{name}} before translating.",
  "settings.arrImportSuggestedScore": "Suggested score: {{score}}.",
  "settings.arrImportUseSuggestion": "Use suggestion",
  "settings.arrImportScoreReasonExported":
    "Uses the score included in the export.",
  "settings.arrImportScoreReasonDefault":
    "Uses the export's default score; other profile choices are available below.",
  "settings.arrImportScoreReasonPreference":
    "The format name expresses a preference.",
  "settings.arrImportScoreReasonPenalty":
    "The format name identifies something undesirable.",
  "settings.arrImportScoreReasonExtras":
    "The format appears to identify extras or samples.",
  "settings.arrImportScoreSuggestionHelp":
    "Suggestions are editable starting points. Matching formats add their scores together; penalties can be offset by other scores.",
  "settings.arrImportAmbiguousScore":
    "This format has more than one suggested score. Choose the score you want to apply.",
  "settings.arrImportBlockScoreHelp":
    "The complete score must exceed -9000 and meet the profile minimum. Mandatory requirements still apply.",
  "settings.arrImportTranslate": "Translate into Rego",
  "settings.arrImportTranslationFailed":
    "Translation failed. Your pasted JSON is still available to review or retry.",
  "settings.arrImportResultSummary":
    "{{translated}} translated; {{disabled}} disabled for manual repair.",
  "settings.arrImportDraftName": "Imported Arr custom formats",
  "settings.arrImportDraftDescription": "Imported from Arr custom-format JSON.",
  "settings.arrImportDiagnostics": "Translation notes",
  "settings.arrImportApply": "Use imported draft",

  "settings.refTitle": "Input Context Reference",
  "settings.refSubtitle":
    "Click to expand the full reference for all fields available to your Rego rules.",
  "settings.refIntro":
    "Every rule receives an input object with five top-level documents. Your Rego code accesses these via input.release, input.profile, input.context, input.builtin_score, and input.file. input.file is null during pre-download scoring and populated only during post-download import evaluation.",
  "settings.refColField": "Field",
  "settings.refColType": "Type",
  "settings.refColDescription": "Description",
  "settings.refColFunction": "Function",
  "settings.refColReturns": "Returns",

  "settings.refSectionRelease": "— Parsed release metadata",
  "settings.refReleaseRawTitle":
    "Original unparsed release title from the indexer",
  "settings.refReleaseQuality":
    "Detected quality tier (e.g. 2160P, 1080P, 720P)",
  "settings.refReleaseSource": "Media source (e.g. WEB-DL, BLURAY, HDTV)",
  "settings.refReleaseVideoCodec": "Video codec (e.g. H.265, H.264, AV1)",
  "settings.refReleaseAudio": "Primary audio codec (e.g. DDP, AAC, TrueHD)",
  "settings.refReleaseAudioCodecs": "All detected audio codecs as an array",
  "settings.refReleaseGuideFacts":
    "Guide facts detected from the release metadata",
  "settings.refReleaseNormalizedTokens":
    "Ordered, normalized tokens produced by the release parser",
  "settings.refReleaseAudioChannels":
    "Audio channel configuration (e.g. 5.1, 7.1, 2.0)",
  "settings.refReleaseLangsAudio":
    "ISO 639-3 audio language codes (e.g. eng, jpn). Includes the languages the indexer listing reported, frozen when the release was grabbed; files that were scanned in, adopted, or imported before listing snapshots were kept contribute only what the name and file show",
  "settings.refReleaseIndexerLanguages":
    "ISO 639-3 audio language codes the indexer listing itself reported, frozen when the release was grabbed. Never inferred from the name or title; empty when the indexer said nothing",
  "settings.refReleaseLangsSub": "ISO 639-3 subtitle language codes",
  "settings.refReleaseIsDualAudio": "True when multiple audio tracks detected",
  "settings.refReleaseIsAtmos": "True when Dolby Atmos audio is present",
  "settings.refReleaseIsDV": "True when Dolby Vision metadata is detected",
  "settings.refReleaseHasHdrFallback":
    "True when the release parser detects an HDR fallback layer",
  "settings.refReleaseDetectedHdr": "True when HDR is detected (non-DV)",
  "settings.refReleaseIsRemux": "True for full remux (no transcoding)",
  "settings.refReleaseIsBdDisk": "True for Blu-ray disc source",
  "settings.refReleaseStereoscopy":
    "Presentation claimed by the release name; null means unknown",
  "settings.refSectionStereoscopy": "— Stereoscopic presentation",
  "settings.refStereoPresentation":
    "two_d, three_d, or mixed_2d_3d; these are name claims, not verified stream properties",
  "settings.refStereoLayout":
    "side_by_side, top_bottom, frame_sequential, row_interleaved, column_interleaved, checkerboard, or anaglyph; null when unspecified, conflicting, or mixed",
  "settings.refStereoSampling":
    "half or full for side-by-side or top/bottom; null when unknown",
  "settings.refStereoEncoding":
    "mvc when explicitly corroborated by a 3D marker; null when unknown",
  "settings.refReleaseStereoscopyTokens":
    "Canonical stereo tokens: stereo:two_d, stereo:three_d, stereo:mixed_2d_3d, stereo:has_3d, stereo:layout:<layout>, stereo:sampling:half/full, stereo:encoding:mvc, and stereo:<field>_conflict. Empty means unknown. Raw normalized_tokens retain their original meaning.",
  "settings.refReleaseRevision":
    "Numeric release revision: 0 for original, 1 for PROPER/REPACK/RERIP, 2 for REPACK2, and so on",
  "settings.refReleaseIsProper": "True when the release is tagged as Proper",
  "settings.refReleaseIsRepack": "True when the release is tagged as Repack",
  "settings.refReleaseIsAiEnhanced":
    "True when the parser detected an AI-enhanced encode marker",
  "settings.refReleaseIsHardcodedSubs":
    "True when hardcoded subtitles are detected in the release name",
  "settings.refReleaseIsPasswordProtected":
    "True when the indexer listing flagged the release as password protected. An indexer listing fact frozen when the release was grabbed; null when the indexer did not say, and null for files that were scanned in, adopted, or imported before listing snapshots were kept",
  "settings.refReleaseIsHdr10Plus":
    "True when HDR10+ is detected from the release metadata",
  "settings.refReleaseIsHlg":
    "True when HLG is detected from the release metadata",
  "settings.refReleaseIs10Bit":
    "True when 10-bit video is detected from the release metadata",
  "settings.refReleaseIsUncensored":
    "True when the release is tagged as uncensored",
  "settings.refReleaseIsDubsOnly":
    "True when the release is tagged as dubs-only",
  "settings.refReleaseHasReleaseGroup":
    "True when a non-empty release group was parsed",
  "settings.refReleaseIsObfuscated":
    "True when the release name appears obfuscated",
  "settings.refReleaseIsRetagged":
    "True when the release name appears retagged or reposted",
  "settings.refReleaseStreamingService":
    "Detected streaming service tag, if present",
  "settings.refReleaseEdition":
    "Edition tag such as extended, IMAX, or criterion",
  "settings.refReleaseAnimeVersion":
    "Anime version number such as v2 or v3, when present",
  "settings.refReleaseEpisodeReleaseType":
    "Episode packaging kind such as single_episode, multi_episode, season_pack, or unknown",
  "settings.refReleaseIsSeasonPack":
    "True when the release appears to contain a full or partial season pack",
  "settings.refReleaseIsMultiEpisode":
    "True when the release appears to contain multiple episodes",
  "settings.refReleaseGroup": "Release group or uploader name",
  "settings.refReleaseYear": "Release year",
  "settings.refReleaseParseConf": "Confidence score for the parse (0.0 to 1.0)",
  "settings.refReleaseSizeBytes":
    "File size in bytes (use scryer.size_gib() to convert)",
  "settings.refReleaseAgeDays":
    "Whole days from the indexer's publish time to when the release was grabbed. An indexer listing fact frozen at the grab: the age at grab, which does not move afterwards. A release not yet grabbed is aged now. Null when the publish time is unknown, and null for files that were scanned in, adopted, or imported before listing snapshots were kept",
  "settings.refReleaseThumbsUp":
    "Indexer thumbs-up votes. An indexer listing fact frozen when the release was grabbed; null when the indexer did not report votes, and null for files that were scanned in, adopted, or imported before listing snapshots were kept",
  "settings.refReleaseThumbsDown":
    "Indexer thumbs-down votes. An indexer listing fact frozen when the release was grabbed; null when the indexer did not report votes, and null for files that were scanned in, adopted, or imported before listing snapshots were kept",
  "settings.refReleaseExtra":
    "Indexer-specific listing attributes (flat values and lists of values), readable as input.release.extra.<key>. Indexer listing facts frozen when the release was grabbed; an empty object for files that were scanned in, adopted, or imported before listing snapshots were kept, so every input.release.extra.<key> lookup is undefined (not null) and a comparison such as == null does not match",
  "settings.refReleaseProtocol":
    "Download protocol: usenet, torrent, or http; null when unknown. Frozen from the listing for import and later rescoring. Historical files remain unknown unless their listing retained protocol metadata.",
  "settings.ruleTestListingProtocol": "Download protocol",

  "settings.ruleTestModeLabel": "What to test",
  "settings.ruleTestModeRelease": "Release name",
  "settings.ruleTestModeStoredFile": "Test a stored file",
  "settings.ruleTestStoredFile": "Stored file",
  "settings.ruleTestStoredFileHelp":
    "Scored with the file's own size and the indexer listing facts frozen when it was grabbed.",
  "settings.ruleTestStoredFileSelectTitle": "Select a library title first",
  "settings.ruleTestStoredFileLoading": "Loading files…",
  "settings.ruleTestStoredFilePlaceholder": "Select a file",
  "settings.ruleTestStoredFileNone": "No files for this title or episode",
  "settings.ruleTestStoredFileFilter": "Filter files",
  "settings.ruleTestStoredFileLoadFailed": "Unable to load this title's files.",
  "settings.ruleTestListingFacts": "Listing facts",
  "settings.ruleTestListingFactsHelp":
    "Optional indexer listing facts. Anything left empty is unknown (null) to the rules. The release is aged from its publish time to now.",
  "settings.ruleTestListingPublishedAt": "Published at",
  "settings.ruleTestListingPublishedAtPlaceholder":
    "2024-01-31, 2024-01-31T12:00:00Z, or Wed, 31 Jan 2024 12:00:00 +0000",
  "settings.ruleTestListingThumbsUp": "Thumbs up",
  "settings.ruleTestListingThumbsDown": "Thumbs down",
  "settings.ruleTestListingPasswordProtected": "Password protected",
  "settings.ruleTestListingUnknown": "Unknown",
  "settings.ruleTestListingYes": "Yes",
  "settings.ruleTestListingNo": "No",
  "settings.ruleTestListingIndexerLanguages":
    "Indexer languages (comma-separated)",
  "settings.ruleTestListingExtra": "Extra attributes (JSON object)",
  "settings.ruleTestListingPublishedAtInvalid":
    "Published at must be a date (2024-01-31), an RFC 3339 time with seconds and a zone (2024-01-31T12:00:00Z), or an RFC 2822 date (Wed, 31 Jan 2024 12:00:00 +0000).",
  "settings.ruleTestListingVotesInvalid":
    "Votes must be whole numbers of zero or more.",
  "settings.ruleTestListingExtraInvalid":
    "Extra attributes must be a JSON object.",
  "settings.ruleTestResultRelease": "Scored release",
  "settings.ruleTestResultListingUnknown":
    "Listing facts unknown: this file was scanned in, adopted, or imported before listing snapshots were kept, so every listing fact is null to the rules.",
  "settings.ruleTestResultAgeDays": "Age (days)",
  "settings.ruleTestResultAgeAtGrab": "{{days}} at grab",
  "settings.ruleTestResultCapturedAt": "Captured",

  "settings.refSectionProfile": "— Active quality profile config",
  "settings.refProfileId": "Profile unique identifier",
  "settings.refProfileName": "Profile display name",
  "settings.refProfileQualityTiers": "Ordered list of acceptable quality tiers",
  "settings.refProfileArchivalQuality":
    "Highest quality tier considered archival",
  "settings.refProfileAllowUnknown":
    "Whether unrecognized qualities are accepted",
  "settings.refProfileSourceAllow": "Allowed sources (empty means all allowed)",
  "settings.refProfileSourceBlock": "Blocked sources",
  "settings.refProfileVCodecAllow":
    "Allowed video codecs (empty means all allowed)",
  "settings.refProfileVCodecBlock": "Blocked video codecs",
  "settings.refProfileACodecAllow":
    "Allowed audio codecs (empty means all allowed)",
  "settings.refProfileACodecBlock": "Blocked audio codecs",
  "settings.refProfileAtmosPreferred":
    "Profile prefers Dolby Atmos when available",
  "settings.refProfileDVAllowed": "Dolby Vision releases are accepted",
  "settings.refProfileHdrAllowed": "HDR releases are accepted",
  "settings.refProfilePreferRemux": "Profile prefers full remux releases",
  "settings.refProfileAllowBdDisk": "Blu-ray disc sources are accepted",
  "settings.refProfileAllowUpgrades": "Upgrading existing files is allowed",
  "settings.refProfilePreferDualAudio": "Profile prefers dual-audio releases",
  "settings.refProfileRequiredLangs":
    "Mandatory audio languages (ISO 639-3 codes)",
  "settings.refProfileScoringPersona":
    "Resolved persona: balanced, audiophile, efficient, or compatible",
  "settings.refProfileScoringOverrides":
    "Explicit profile choices; null leaves the persona default unchanged",
  "settings.refSectionScoringOverrides": "— Scoring overrides",
  "settings.refOverrideX265":
    "Allow non-4K HEVC; null inherits the persona default",
  "settings.refOverrideDolbyVision":
    "Reject Dolby Vision without fallback; null inherits the persona default",
  "settings.refOverrideCompact":
    "Prefer compact encodes; null inherits the persona default",
  "settings.refOverrideLossless":
    "Prefer lossless audio; null inherits the persona default",
  "settings.refOverrideUpscaled":
    "Penalize upscaled releases; null inherits the persona default",
  "settings.refCtxCoverageTotalRuntime":
    "Combined runtime in minutes for the release's coverage, when known",
  "settings.refCtxCoverageMemberRuntime":
    "One covered member's runtime in minutes, when known",
  "settings.refCtxCoverageMemberCount": "Number of covered members, when known",

  "settings.refSectionContext": "— Title and search context",
  "settings.refCtxTitleId":
    "Internal Scryer title id when a concrete title is already known",
  "settings.refCtxLibraryName":
    "Resolved library name when evaluation is scoped to a specific library",
  "settings.refCtxMediaType":
    "Media facet being evaluated, typically movie, series, or anime",
  "settings.refCtxCategory":
    "Category hint used for scoring and rule evaluation",
  "settings.refCtxOriginalLanguage":
    "Normalized original title language, when metadata provides one",
  "settings.refCtxOriginalCountry":
    "Normalized original title country, when metadata provides one",
  "settings.refCtxInferredOriginalAudioLanguage":
    "Normalized original audio language inferred for unlabeled releases",
  "settings.refCtxTags":
    "User-defined tags on the title, lowercase, from the tag registry an administrator maintains in Settings; reserved scryer: settings entries are excluded",
  "settings.refCtxHasExisting":
    "True when a file already exists for this title",
  "settings.refCtxExistingScore":
    "Built-in score of the existing file (if any)",
  "settings.refCtxSearchMode":
    "Evaluation mode. Current values are auto and post_download",
  "settings.refCtxRuntimeMin": "Content runtime in minutes",
  "settings.refCtxIsAnime": "True when the title is categorized as anime",
  "settings.refCtxIsFiller": "True when the episode is marked as filler",

  "settings.refSectionBuiltinScore": "— Built-in scoring engine results",
  "settings.refBuiltinTotal": "Total score from the built-in scoring engine",
  "settings.refBuiltinBlocked": "True when built-in rules blocked this release",
  "settings.refBuiltinCodes":
    "List of scoring codes that were applied (e.g. quality_tier_0)",

  "settings.refSectionFile":
    "— Actual analyzed file metadata (null pre-download)",
  "settings.refFileVideoCodec": "Verified video codec from media analysis",
  "settings.refFileVideoWidth": "Verified video width in pixels",
  "settings.refFileVideoHeight": "Verified video height in pixels",
  "settings.refFileVideoBitrateKbps": "Verified video bitrate in kbps",
  "settings.refFileVideoBitDepth": "Verified video bit depth",
  "settings.refFileVideoHdrFormat":
    "Verified HDR format such as Dolby Vision, HDR10, HDR10+, or HLG",
  "settings.refFileDoviProfile": "Dolby Vision profile number when present",
  "settings.refFileDoviBlCompatId":
    "Dolby Vision base-layer compatibility id when present",
  "settings.refFileVideoFrameRate":
    "Verified frame rate string from media analysis",
  "settings.refFileVideoProfile":
    "Verified codec profile such as Main 10 or High",
  "settings.refFileAudioCodec": "Primary verified audio codec",
  "settings.refFileAudioProfile":
    "Primary verified audio profile or extension label, such as LC, Dolby TrueHD + Dolby Atmos, or DTS-HD MA + DTS:X IMAX",
  "settings.refFileAudioChannels": "Primary verified audio channel count",
  "settings.refFileAudioBitrateKbps": "Primary verified audio bitrate in kbps",
  "settings.refFileAudioLanguages":
    "All detected audio languages from the actual file",
  "settings.refFileAudioStreams":
    "Per-audio-stream details; see input.file.audio_streams[] below",
  "settings.refFileSubtitleLanguages":
    "All detected subtitle languages from the actual file",
  "settings.refFileSubtitleCodecs":
    "All detected subtitle codecs from the actual file",
  "settings.refFileSubtitleStreams":
    "Per-subtitle-stream details; see input.file.subtitle_streams[] below",
  "settings.refFileHasMultiAudio":
    "True when multiple audio streams are present in the actual file",
  "settings.refFileDurationSeconds": "Verified runtime in seconds",
  "settings.refFileNumChapters":
    "Verified chapter count when the container exposes chapters",
  "settings.refFileContainerFormat":
    "Verified container format such as matroska or mp4",

  "settings.refSectionAudioStreams": "— Nested audio stream entries",
  "settings.refAudioStreamCodec": "Per-stream audio codec",
  "settings.refAudioStreamProfile":
    "Per-stream verified audio profile or extension label",
  "settings.refAudioStreamChannels": "Per-stream channel count",
  "settings.refAudioStreamLanguage": "Per-stream language code",
  "settings.refAudioStreamBitrateKbps": "Per-stream bitrate in kbps",

  "settings.refSectionSubtitleStreams": "— Nested subtitle stream entries",
  "settings.refSubtitleStreamCodec": "Per-stream subtitle codec",
  "settings.refSubtitleStreamLanguage": "Per-stream language code",
  "settings.refSubtitleStreamName":
    "Per-stream subtitle track title, if available",
  "settings.refSubtitleStreamForced":
    "True when the subtitle stream is marked forced",
  "settings.refSubtitleStreamDefault":
    "True when the subtitle stream is marked default",

  "settings.refSectionBuiltins": "Custom Scryer Functions",
  "settings.refBuiltinsIntro":
    "These helper functions are available in your Rego rules in addition to all standard OPA builtins.",
  "settings.refFnBlockScore":
    "Returns -10000 as a penalty that other scores can offset",
  "settings.refFnSizeGib":
    "Converts bytes to GiB (e.g. scryer.size_gib(input.release.size_bytes) > 50)",
  "settings.refFnLangMatches":
    'Matches ISO 639-3 codes with aliases (e.g. scryer.lang_matches("jpn", "ja") is true)',
  "settings.refFnNormalizeSource":
    'Normalizes source names (e.g. "webdl" becomes "WEB-DL")',
  "settings.refFnNormalizeCodec":
    'Normalizes codec names (e.g. "h264" becomes "H.264")',

  "settings.refSectionSandbox": "Sandbox & Restrictions",
  "settings.refSandboxIntro":
    "All user rules run inside a sandboxed Rego evaluator with the following restrictions:",
  "settings.refSandboxNoIO":
    "No network or filesystem access — rules cannot make HTTP requests, read files, or perform any I/O. Standard OPA builtins like http.send are not available.",
  "settings.refSandboxPkgIsolation":
    "Package isolation — each rule is automatically namespaced to its own package. Rules cannot read or modify data from other rules.",
  "settings.refSandboxReadOnly":
    "Read-only input — rules can only read the input document. They cannot modify application state or persist data between evaluations.",
  "settings.refSandboxOutputRestricted":
    "Output restricted to score_entry — only the score_entry map is extracted from each rule. Any other definitions or assignments are ignored.",
  "settings.refSandboxIntegerOnly":
    "Integer-only score deltas — values in score_entry must be integers within i32 range. Floats are rejected during validation (use round() or ceil() to convert).",
  "settings.refSandboxValidation":
    "Pre-save validation — rules are compiled and dry-run tested against synthetic input before they can be saved. Invalid rules are rejected.",
  "settings.refSandboxErrorIsolation":
    "Error isolation — if one rule fails at runtime, all other rules continue to evaluate normally. Failures are logged but never abort scoring.",

  "settings.refSectionOutput": "Rule Output Format",
  "settings.refOutputIntro":
    "Rules must define a score_entry rule that returns a map of string keys (score codes) to integer values (point deltas). Positive values boost the release, negative values penalize it, and scryer.block_score() contributes a recoverable -10000 penalty. Eligibility is decided after all contributions; mandatory profile requirements remain blocking. File-based rules naturally no-op pre-download because input.file is null until post-download evaluation.",

  "settings.subtitles": "Subtitles",
  "settings.subtitlesDescription":
    "Configure subtitle languages, matching thresholds, sync behavior, and provider connections.",
  "settings.existingSubtitleProviders": "Configured subtitle providers",
  "settings.subtitleProviderType": "Provider",
  "settings.subtitleProviderStatus": "Status",
  "settings.subtitleProviderFacets": "Content types",
  "settings.subtitleProviderFacetsHelp":
    "Scryer only searches this provider for the selected content types. Plugin recommendations seed these defaults, but do not restrict your choices.",
  "settings.subtitleProviderEmpty": "No subtitle providers configured yet.",
  "settings.subtitleProviderCreate": "Add subtitle provider",
  "settings.subtitleProviderCreateNew": "Create new subtitle provider",
  "settings.subtitleProviderEdit": "Edit subtitle provider",
  "settings.subtitleProviderConfirmDiscardTitle":
    "Discard current subtitle provider edits?",
  "settings.subtitleProviderConfirmDiscardDescription":
    "Your current subtitle provider edits will be lost.",
  "settings.subtitleProviderNamePlaceholder": "OpenSubtitles",
  "settings.subtitleProviderCreated": "Subtitle provider created.",
  "settings.subtitleProviderUpdated": "Subtitle provider updated.",
  "settings.subtitleProviderDeleted": "{{name}} deleted.",
  "settings.subtitleProviderNoActivity": "No activity yet",
  "settings.subtitleProviderDisabledUntil": "Disabled until {{time}}",
  "settings.subtitleProviderLastError": "Last error {{time}}",
  "settings.subtitleProviderUnknownType":
    "This provider type is installed but its field schema is not available yet.",
  "settings.subtitleProviderSecretStored": "Stored value kept unless replaced",
  "settings.sub.languages": "Subtitle Languages",
  "settings.sub.languagePickerAriaLabel": "Select subtitle languages",
  "settings.sub.languagePickerSelect": "Select languages...",
  "settings.sub.languagePickerSearch": "Search languages...",
  "settings.sub.languagePickerEmpty": "No languages found",
  "settings.sub.languagePickerRemove": "Remove {{language}}",
  "settings.sub.hiPreference": "Hearing Impaired",
  "settings.sub.forcedOnly": "Forced subtitles only",
  "settings.sub.minScoreSeries": "Minimum match percent for series",
  "settings.sub.minScoreMovie": "Minimum match percent for movies",
  "settings.sub.minScoreHelp":
    "Subtitles below this match percentage are skipped. Series needs a higher threshold because episode-level matching must be exact.",
  "settings.sub.searchInterval": "Search interval (hours)",
  "settings.sub.autoDownload": "Auto-download on import",
  "settings.sub.excludeAi": "Exclude AI-translated subtitles",
  "settings.sub.excludeMachine": "Exclude machine-translated subtitles",
  "settings.sub.syncEnabled": "Auto-correct subtitle timing",
  "settings.sub.syncEnabledHelp":
    "Uses audio analysis to fix timing drift in downloaded subtitles.",
  "settings.sub.syncThresholdSeries":
    "Auto-correct at or below this percent (series)",
  "settings.sub.syncThresholdMovie":
    "Auto-correct at or below this percent (movies)",
  "settings.sub.syncThresholdHelp":
    "Low-scoring subtitles are more likely to have timing issues. Subtitles above this match percentage are assumed to be well-timed.",
  "settings.sub.syncMaxOffset": "Maximum correction (seconds)",
  "settings.sub.syncMaxOffsetHelp":
    "Skip timing changes larger than this value to avoid damaging a subtitle that is probably already close enough.",
  "settings.sub.syncPluginRequiredTitle": "Enhanced subtitle sync required",
  "settings.sub.syncPluginRequiredDescription":
    "Install {{plugin}} to unlock automatic timing correction settings.",
  "settings.sub.syncPluginLoadingCatalog": "Checking plugin catalog...",
  "settings.sub.syncPluginInstall": "Install plugin",
  "settings.sub.syncPluginInstalling": "Installing...",
  "settings.sub.syncPluginUnavailable":
    "Enhanced Subtitle Sync is not available from the plugin catalog.",
  "settings.sub.syncPluginBlocked":
    "Enhanced Subtitle Sync cannot be installed: {{reason}}",
  "settings.sub.syncPluginDisabled":
    "Enhanced Subtitle Sync is installed but disabled. Enable it in Plugins to use timing correction.",
  "settings.sub.syncPluginInstalled": "{{plugin}} installed",
  "settings.sub.syncPluginLoadFailed": "Could not load the plugin catalog.",
  "settings.sub.syncPluginInstallFailed":
    "Could not install {{plugin}}: {{error}}",
  "settings.postProcessing": "Post-Processing",
  "settings.pp.title": "Post-Processing Scripts",
  "settings.pp.description":
    "Scripts run automatically after media is imported. Blocking scripts execute in priority order. Fire-and-forget scripts run in parallel without delaying the import pipeline.",
  "settings.pp.noScripts": "No post-processing scripts configured.",
  "settings.pp.name": "Script Name",
  "settings.pp.namePlaceholder": "e.g. Notify Plex",
  "settings.pp.descriptionLabel": "Description",
  "settings.pp.descriptionPlaceholder": "What does this script do?",
  "settings.pp.scriptType": "Script Type",
  "settings.pp.inline": "Inline shell",
  "settings.pp.filePath": "File Path",
  "settings.pp.inlineHelp":
    "Enter a shell command to run through the server shell.",
  "settings.pp.filePathHelp":
    "Absolute path to an executable script on the server. Arguments, redirection, and shell expansion are not applied.",
  "settings.pp.blocking": "Blocking",
  "settings.pp.fireAndForget": "Fire & Forget",
  "settings.pp.blockingHelp":
    "Runs in priority order. Next script waits for this one to finish.",
  "settings.pp.fireAndForgetHelp":
    "Runs in the background without blocking import. Output can still be captured if enabled.",
  "settings.pp.executionMode": "Execution Mode",
  "settings.pp.timeout": "Timeout (seconds)",
  "settings.pp.timeoutLabel": "Timeout",
  "settings.pp.priority": "Priority",
  "settings.pp.priorityHelp": "Lower number runs first.",
  "settings.pp.facets": "Applies To",
  "settings.pp.debug": "Capture script output",
  "settings.pp.debugHelp":
    "When enabled, stdout and stderr are saved for each run, including background runs. Useful for troubleshooting.",
  "settings.pp.runHistory": "Run History",
  "settings.pp.noRuns": "No executions yet.",
  "settings.pp.outputNotCaptured":
    "Output not captured. Enable debug mode on this script to see output.",
  "settings.pp.envHeading": "Environment Variables",
  "settings.pp.envDescription":
    "Scripts receive a JSON payload in SCRYER_METADATA, plus the convenience variables shown below. Values may be null when Scryer does not have them.",
  "settings.pp.created": "Script created.",
  "settings.pp.updated": "Script updated.",
  "settings.pp.deleted": "Script deleted.",
  "settings.pp.toggled": "Script {{state}}.",
  "settings.pp.createNewScript": "Create new script",
  "settings.pp.confirmDiscardTitle":
    "Discard current post-processing script edits?",
  "settings.pp.confirmDiscardDescription":
    "Your current post-processing script edits will be lost.",
  "settings.pp.inlineShellConfirmTitle": "Allow inline shell script?",
  "settings.pp.inlineShellConfirmDescription":
    "Inline post-processing scripts run through the server shell after imports. Only save scripts you have reviewed and trust.",
  "settings.pp.inlineShellConfirm": "Allow inline shell",

  "settings.plugins": "Plugins",
  "settings.scripts": "Scripts",
  "settings.pluginRuntimeMemoryEstimate":
    "Estimated plugin runtime memory usage",
  "settings.pluginsRefresh": "Refresh Catalog",
  "settings.pluginAutoUpdateEnabled": "Auto update plugin patches",
  "settings.pluginAutoUpdateEnabledHelp":
    "When enabled, the scheduled Plugin Catalog Refresh (or running that job manually) updates official plugins to their latest patch version, e.g. 1.0.1 → 1.0.2, and installs optimized builds of the installed version when one is available. Minor and major versions stay operator-owned.",
  "settings.pluginsUpdateAll": "Update All",
  "settings.pluginsUpdatingAll": "Updating All…",
  "settings.host": "Host",
  "settings.port": "Port",
  "settings.baseUrl": "Base URL",
  "settings.externalUrl": "External URL",
  "settings.externalUrlHint":
    "Browser-facing URL used for Jellyfin or Emby playback links.",
  "label.watchIn": "Watch in",
  "settings.pluginsInstalled": "Installed",
  "settings.pluginsAvailable": "Available",
  "settings.pluginsNoInstalled": "No plugins installed.",
  "settings.pluginsNoAvailable": "No plugins match the current filter.",
  "settings.pluginsNoPlugins":
    "No plugins found. Refresh the catalog to check for available plugins.",
  "settings.pluginAllCategories": "All Categories",
  "settings.pluginCategoryIndexer": "Indexer",
  "settings.pluginCategoryUsenetIndexer": "Usenet Indexer",
  "settings.pluginCategoryTorrentIndexer": "Torrent Indexer",
  "settings.pluginCategoryDownloadClient": "Download Client",
  "settings.pluginCategoryArchiveExtractor": "Archive Extraction",
  "settings.pluginCategoryNotification": "Notification",
  "settings.pluginCategorySubtitleProvider": "Subtitle Provider",
  "settings.pluginOfficialOnly": "Official only",
  "settings.pluginInstall": "Install",
  "settings.pluginInstalling": "Installing…",
  "settings.pluginInstallDownloading": "Downloading",
  "settings.pluginInstallVerifying": "Verifying",
  "settings.pluginInstallInstalling": "Installing",
  "settings.pluginUninstall": "Uninstall",
  "settings.pluginRevertToBundled": "Revert to bundled",
  "settings.pluginRevert": "Revert",
  "settings.pluginUninstallWarning":
    "deleting this plugin will delete all configurations related to it",
  "settings.pluginRevertToBundledWarning":
    "This removes the downloaded override for '{{name}}' and reverts to the bundled built-in plugin.",
  "settings.pluginBuiltin": "Built-in",
  "settings.pluginInstalled": "Installed",
  "settings.pluginOfficial": "Official",
  "settings.pluginVerifiedCommunity": "Verified Community",
  "settings.pluginUnverified": "Unverified",
  "settings.pluginBeta": "Beta",
  "settings.pluginDeprecated": "Deprecated",
  "settings.pluginBlocked": "Blocked",
  "settings.pluginBlockedNotRunning": "Not running",
  "settings.pluginSource": "Source",
  "settings.pluginDocs": "Docs",
  "settings.pluginInstallManually": "Install Manually",
  "settings.pluginManualUploadTitle": "Local file upload",
  "settings.pluginManualUploadHelp":
    "Upload a local plugin build for fast iteration. Scryer accepts raw .wasm files and .wasm.zst bundles.",
  "settings.pluginManualUploadSelect": "Choose plugin file",
  "settings.pluginManualUploadInstall": "Review and install file",
  "settings.pluginManualUploadNoFile": "No plugin file selected.",
  "settings.pluginManualUploadFormats": "Accepted formats: .wasm and .wasm.zst",
  "settings.pluginManualFileUnsupported":
    "Manual plugin upload must be a .wasm or .wasm.zst file.",
  "settings.pluginManualUploadConfirmTitle": "Install local plugin file?",
  "settings.pluginManualUploadConfirmDescription":
    "You are about to run unverified local plugin code inside Scryer. Only install files you built yourself or explicitly trust.",
  "settings.pluginManualUploadConfirmCheckbox":
    "I understand this plugin will run custom code with the permissions declared by its manifest.",
  "settings.pluginManualUploadConfirmAction": "Install local plugin",
  "settings.pluginManualRepoTitle": "GitHub repo install",
  "settings.pluginManualRepoHelp":
    "Install an unverified plugin directly from a GitHub plugin repository that publishes the Scryer plugin catalog format.",
  "settings.pluginManualRepoUrl": "GitHub plugin repo",
  "settings.pluginInspectManual": "Inspect Repo",
  "settings.pluginOverride": "Override",
  "settings.pluginVersion": "v{{version}}",
  "settings.pluginUpdateAvailable": "Update: v{{version}}",
  "settings.pluginUpgrade": "Upgrade to v{{version}}",
  "settings.pluginOptimizedBuildAvailable": "Optimized build available",
  "settings.pluginInstallOptimizedBuild": "Install optimized build",

  "status.pluginInstalled": "Plugin '{{name}}' installed.",
  "status.pluginInstallAlreadyInProgress":
    "Plugin '{{name}}' is already installing or upgrading.",
  "status.pluginInstallFailedChecksumMismatch":
    "Failed to install plugin '{{name}}': the published artifact no longer matches the registry checksum. Refresh the registry or try again later.",
  "status.pluginInstallFailedSdkMetadataMismatch":
    "Failed to install plugin '{{name}}': the published artifact does not match the registry SDK compatibility metadata. Refresh the registry or try again later.",
  "status.pluginInstallFailedWithReason":
    "Failed to install plugin '{{name}}': {{reason}}",
  "status.pluginRevertedToBundled":
    "Plugin '{{name}}' reverted to the bundled built-in version.",
  "status.pluginUninstalled": "Plugin '{{name}}' uninstalled.",
  "status.pluginUpgraded": "Plugin '{{name}}' upgraded to v{{version}}.",
  "status.pluginsUpgradeQueued": "Queued upgrades for {{count}} plugins.",
  "status.pluginAutoUpdateSettingsSaved":
    "Automatic plugin update setting saved.",
  "status.pluginToggled": "Plugin '{{name}}' {{state}}.",
  "status.catalogRefreshed": "Plugin catalog refreshed.",

  "form.indexerNamePlaceholder": "My Newznab Indexer",
  "form.indexerRateLimitSeconds": "Minimum seconds between queries",
  "form.indexerRateLimitSecondsPlaceholder": "No delay",
  "form.indexerRateLimitSecondsInvalid":
    "Minimum seconds between queries must be a whole number, or empty for no delay.",
  "form.indexerMaxQueriesPerMinute": "Max queries per minute",
  "form.indexerMaxQueriesPerMinutePlaceholder": "No limit",
  "form.indexerMaxQueriesPerMinuteInvalid":
    "Max queries per minute must be a whole number of at least 1, or empty for no limit.",
  "form.providerTypePlaceholder": "Provider Type",
  "form.subtitleProviderValidation": "Name and provider type are required.",
  "form.subtitleProviderFacetsRequired":
    "Select at least one content type. Scryer never searches a provider that has none.",
  "form.apiKeyInputPlaceholder": "api key",
  "form.apiKeyStoredPlaceholder": "stored (leave blank to keep)",
  "form.usernamePlaceholder": "username",
  "form.passwordPlaceholder": "password",
  "form.newPasswordPlaceholder": "new password",
  "form.indexerValidation": "Name, provider type, and base URL are required.",

  "qualityProfile.noProfilesFound": "No profiles found.",
  "qualityProfile.createNewProfile": "Create new quality profile",
  "qualityProfile.editProfile": "Edit quality profile",
  "qualityProfile.confirmDiscardTitle":
    "Discard current quality profile edits?",
  "qualityProfile.confirmDiscardDescription":
    "Your current quality profile edits will be lost.",
  "qualityProfile.profileNameLabel": "Profile name",
  "qualityProfile.qualityTiersAndArchival":
    "Quality tiers and archival quality",
  "qualityProfile.qualityPreference": "Quality preference",
  "qualityProfile.qualityPreferenceHelp":
    "Highest preference first. Tier preference takes priority over score. New tiers are added at the lowest preference.",
  "qualityProfile.moveQualityTierUp": "Move {{value}} up",
  "qualityProfile.moveQualityTierDown": "Move {{value}} down",
  "qualityProfile.dragQualityTier": "Drag to reorder {{value}}",
  "qualityProfile.allowedQualityTiers": "Allowed quality tiers (selected)",
  "qualityProfile.noQualityTiersSelected": "No quality tiers selected.",
  "qualityProfile.removeQualityTier": "Remove {{value}} quality tier",
  "qualityProfile.addQualityTier": "Add {{value}} to selection",
  "qualityProfile.availableQualityTiers": "Available quality tiers",
  "qualityProfile.allQualityTiersSelected": "All known quality tiers selected.",
  "qualityProfile.archivalQuality": "Archival quality",
  "qualityProfile.archivalQualityInfo":
    "The quality level to keep for long-term retention. Scryer may download a higher quality first if no archival quality exists yet, then replace it later when the archival quality release is found.",
  "qualityProfile.qualityTiers": "Quality tiers",
  "qualityProfile.allowHdr": "HDR",
  "qualityProfile.detectedHdrAllowed": "HDR allowed",
  "qualityProfile.detectedHdrAllowedInfo":
    "Allow releases detected as HDR (HDR10, HLG, Dolby Vision, etc.). Note: many release names do not explicitly include HDR, so this does not guarantee HDR releases are filtered out.",
  "qualityProfile.allowUnknownQuality": "Unknown quality allowed",
  "qualityProfile.allowUnknownQualityInfo":
    "Allow releases where quality cannot be identified by parser rules. This can help with unusual encodes but may allow lower-confidence matches.",
  "qualityProfile.dolbyVisionAllowed": "Dolby Vision",
  "qualityProfile.dolbyVisionInfo":
    "Allow Dolby Vision releases. Some displays cannot render Dolby Vision content correctly, so this may not be desired for everyone.",
  "qualityProfile.preferRemux": "Remux preferred",
  "qualityProfile.preferRemuxInfo":
    "Remux are full-quality copies of physical media. This is mainly useful with the Audiophile persona.",
  "qualityProfile.allowBdDisk": "BD disk",
  "qualityProfile.allowBdDiskInfo":
    "Allow BD/Disc image releases (ISO/Blu-ray disk style releases) to pass the profile.",
  "qualityProfile.allowDv": "Dolby Vision",
  "qualityProfile.ignoreProperRepack": "Ignore PROPER/REPACK",
  "qualityProfile.ignoreProperRepackInfo":
    "Let quality and scores decide without automatic revision preference or protection. Scoring rules can still reward PROPER/REPACK releases.",
  "qualityProfile.allowUpgrades": "Upgrades allowed",
  "qualityProfile.allowUpgradesInfo":
    "Allow managed copies to be replaced later by a better match, but never above the selected archival quality.",
  "qualityProfile.sourceAllowlist": "Sources",
  "qualityProfile.videoCodecAllowlist": "Video Codecs",
  "qualityProfile.audioCodecAllowlist": "Audio Codecs",
  "qualityProfile.sourceAllowlistInfo":
    "Only allow releases that match one of the selected source values (for example, WEB-DL, HDTV).",
  "qualityProfile.sourceBlocklistDefault":
    "No source blocks applied by default.",
  "qualityProfile.videoCodecAllowlistInfo":
    "Only allow releases encoded with one of these video codecs.",
  "qualityProfile.videoCodecBlocklistDefault":
    "No blocked video codecs by default.",
  "qualityProfile.audioCodecAllowlistInfo":
    "Only allow releases with matching audio codecs.",
  "qualityProfile.audioCodecBlocklistDefault":
    "No blocked audio codecs by default.",
  "qualityProfile.noSelectedItems": "None selected.",
  "qualityProfile.aboutSection": "About {{title}}",
  "qualityProfile.useDefaultQualityFallback": "Use default quality fallback",
  "qualityProfile.defaultCategoryProfiles": "Default category profiles",
  "qualityProfile.scoringAndPreferences": "Scoring & preferences",
  "qualityProfile.scoringAndPreferencesInfo":
    "Control how releases are ranked and filtered. Set a scoring persona, preferences, and upgrade behavior.",
  "qualityProfile.facetPersonaUseDefault": "Use default",
  "qualityProfile.scoringPersona": "Scoring persona",
  "qualityProfile.scoringPersonaInfo":
    "Preset scoring weights that prioritize different release qualities. Balanced is recommended for most users.",
  "qualityProfile.personaBalanced": "Balanced",
  "qualityProfile.personaAudiophile": "Audiophile",
  "qualityProfile.personaEfficient": "Efficient",
  "qualityProfile.personaCompatible": "Compatible",
  "qualityProfile.scoringOverrides": "Scoring overrides",
  "qualityProfile.scoringOverridesInfo":
    "Override specific scoring behaviors regardless of the selected persona. Leave as 'Default' to use the persona's setting.",
  "qualityProfile.overrideAllowX265Non4k": "Allow x265 at non-4K",
  "qualityProfile.overrideAllowX265Non4kInfo":
    "Allow x265/HEVC encoded releases at resolutions below 4K. Some devices struggle with x265 at lower resolutions.",
  "qualityProfile.overrideBlockDvNoFallback": "Block DV without fallback",
  "qualityProfile.overrideBlockDvNoFallbackInfo":
    "Block Dolby Vision releases that lack an HDR10 fallback layer, which can appear green/purple on non-DV displays.",
  "qualityProfile.overridePreferCompact": "Prefer compact encodes",
  "qualityProfile.overridePreferCompactInfo":
    "Reward smaller, bandwidth-efficient encodes. Useful for limited storage or streaming over slower connections.",
  "qualityProfile.overridePreferLossless": "Prefer lossless audio",
  "qualityProfile.overridePreferLosslessInfo":
    "Give bonus points to releases with lossless audio tracks (TrueHD, FLAC, DTS-HD MA).",
  "qualityProfile.overrideBlockUpscaled": "Block AI upscaled",
  "qualityProfile.overrideBlockUpscaledInfo":
    "Block releases that have been AI upscaled. These releases often have artifacts and are not true native resolution.",
  "qualityProfile.cutoffTier": "Cutoff tier",
  "qualityProfile.cutoffTierInfo":
    "Stop upgrading once this quality tier (or better) has been grabbed. Prevents unnecessary re-downloads.",
  "qualityProfile.cutoffNone": "No cutoff",
  "qualityProfile.minScoreToGrab": "Minimum score to grab",
  "qualityProfile.minScoreToGrabInfo":
    "Only grab releases that score at or above this threshold. Leave empty for no minimum.",
  "qualityProfile.minScorePlaceholder": "No minimum",
  "qualityProfile.confirmDeleteTitle": "Delete Quality Profile",
  "qualityProfile.confirmDeleteDescription":
    'Are you sure you want to delete the quality profile "{{name}}"? This cannot be undone.',
  "qualityProfile.deleteDisabledInUse":
    "This profile is in use. Remove it from all titles and category defaults before deleting.",

  "episode.details": "Details",
  "episode.search": "Search",
  "episode.blocklist": "Blocklist",
  "episode.overview": "Overview",
  "episode.fileOnDisk": "File on Disk",
  "episode.airDate": "Air Date",
  "episode.numberLabel": "Episode",
  "episode.quality": "Quality",
  "episode.missing": "Missing",
  "episode.noBlockedReleases": "No blocked releases recorded for this episode.",
  "episode.untitledRelease": "Untitled release",
  "episode.filler": "Filler",
  "episode.recap": "Recap",
  "episode.titleTba": "TBA",
  "episode.special": "Special",
  "episode.ova": "OVA",
  "episode.ona": "ONA",
  "episode.alternate": "Alternate",
  "seasonSection.seasonSearchResults": "{{season}} search results",
  "seasonSection.noEpisodeRecords": "No episode records for this season.",
  "seriesOverview.selectColumn": "Select",
  "seriesOverview.selectEpisodeForDelete": "Select {{name}}",
  "seriesOverview.selectSeasonForDelete": "Select all episodes in {{name}}",
  "seriesOverview.clearEpisodeSelection": "Clear selection",
  "seriesOverview.deleteSelectedEpisodeFiles": "Delete {{count}} selected",
  "seriesOverview.deleteEpisodeFilesTitle": "Delete selected episode files",
  "seriesOverview.deleteEpisodeFilesDescription":
    "This permanently deletes the files on disk for the selected episodes. Episode records are kept.",
  "seriesOverview.deleteEpisodeFilesSummary":
    "{{files}} files across {{episodes}} episodes",
  "seriesOverview.deleteEpisodeFilesPreviewFailures":
    "{{count}} files could not be previewed and may fail to delete.",
  "mediaFile.pendingScan": "Pending Scan",
  "mediaFile.scanFailed": "Scan Failed",
  "mediaFile.reviewRequired": "Review required",
  "mediaFile.discReviewInventory":
    "Titles from the latest inspection. Saved selection and episode mappings remain in place until you save a validated choice.",
  "mediaFile.score": "Score {{score}}",
  "scoring.current": "Current score",
  "scoring.score": "Score",
  "scoring.previous": "Previous score",
  "scoring.change": "Change",
  "scoring.decision": "Decision",
  "scoring.manualOverride": "Manual selection",
  "scoring.needsRefresh": "Needs refresh",
  "scoring.timeSensitive": "Time-sensitive",
  "scoring.breakdown": "Score breakdown",
  "scoring.customRule": "Custom Rule",
  "scoring.systemRule": "System Rule",
  "scoring.rejected": "Rejected",
  "scoring.refreshOne": "Refresh score",
  "scoring.refreshSelected": "Refresh scores",
  "scoring.refreshFailed":
    "Could not refresh scores. Check your selection and permissions.",
  "mediaFile.primary": "Primary",
  "mediaFile.additional": "Additional",
  "mediaFile.makePrimary": "Make primary",
  "mediaFile.audioCount": "{{count}} Audio",
  "mediaFile.commentary": "Commentary",
  "mediaFile.audioDescription": "Audio description",
  "mediaFile.hearingImpaired": "Hearing impaired",
  "mediaFile.program": "Program {{id}}",
  "mediaFile.discTitle": "Disc playback title",
  "mediaFile.discReviewAction": "Disc needs attention",
  "mediaFile.discOutcomeAutomatic":
    "Playing title {{id}} ({{duration}}), chosen automatically as the longest complete title.",
  "mediaFile.discOutcomeManual":
    "Playing title {{id}} ({{duration}}), chosen by you. Automatic selection is off for this disc.",
  "mediaFile.discOutcomeUnselected":
    "No playable title could be chosen. Pick one below.",
  "mediaFile.discOutcomeIncomplete":
    "The chosen title's inspection did not complete ({{status}}). Pick a complete title or re-check the file.",
  "mediaFile.discChooseTitle": "Which title should play",
  "mediaFile.discAutomaticCurrent": "Currently title {{id}}",
  "mediaFile.discTitleChapterCount": "{{count}} chapter(s)",
  "mediaFile.discTitleAudioCount": "{{count}} audio track(s)",
  "mediaFile.discTitleNotSelectable": "Inspection {{status}}",
  "mediaFile.discTitleMissing": "No longer on the disc",
  "mediaFile.discPlaying": "Playing",
  "mediaFile.discUseTitle": "Use this title",
  "mediaFile.discStreamsAndChapters": "Streams and chapters",
  "mediaFile.discTechnicalDetails": "Technical details",
  "mediaFile.discTitleIdentity": "Title {{id}}",
  "mediaFile.discInspectTitles": "Inspect title streams and chapters",
  "mediaFile.discAngleInventory":
    "{{count}} angle(s). Media inspection covers the first angle when available.",
  "mediaFile.discTitleAliases": "Equivalent title IDs: {{ids}}",
  "mediaFile.discTitleBudgetExhausted":
    "Inspection of this title reached its probe budget.",
  "mediaFile.discTitleStreams": "Title streams",
  "mediaFile.discTitleStreamsUnknown":
    "No stream metadata was recovered for this title.",
  "mediaFile.discTitleChapters": "Title chapters",
  "mediaFile.discTitleChaptersUnknown":
    "No chapter metadata was recovered for this title.",
  "mediaFile.discChapterIdentity": "Chapter {{id}}",
  "mediaFile.discEpisodeMapping": "Map disc titles to episodes",
  "mediaFile.discMappingScope":
    "Choose one episode for each authored playback title. Unmapped titles do not make episodes available. The ISO stays one file.",
  "mediaFile.discMappingSave": "Save episode mappings",
  "mediaFile.discMappingFailed": "Could not save the episode mappings",
  "mediaFile.discMappingLoading": "Loading episodes…",
  "mediaFile.discMappingNone": "No episode",
  "mediaFile.discAutomatic": "Automatic longest valid title",
  "mediaFile.discSaving": "Inspecting title…",
  "mediaFile.discSelectionFailed": "Could not save the disc title selection",
  "mediaFile.analysisHeading": "Media inspection {{status}}",
  "mediaFile.analysisLatestInspection":
    "Latest inspection {{status}} at {{time}}",
  "mediaFile.analysisPreviousRetained":
    "The previous saved inspection is retained.",
  "mediaFile.analysisNoneSaved": "No inspection has been saved for this file.",
  "mediaFile.analysisBudgetExhausted": "The inspection budget was exhausted.",
  "mediaFile.analysisSummary":
    "Duration {{duration}} · overall {{overall}} · video {{video}}",
  "mediaFile.analysisBudgetStopped": "Inspection stopped at its probe budget.",
  "mediaFile.analysisProgram": "Program {{id}}",
  "mediaFile.analysisProgramStreams": "streams {{ids}}",
  "mediaFile.analysisSelected": "selected",
  "mediaFile.analysisChapters": "Chapters",
  "mediaFile.analysisAttachments": "Attachments",
  "mediaFile.analysisUnknownType": "unknown type",
  "mediaFile.analysisBytes": "{{count}} bytes",
  "mediaFile.analysisCaptionService":
    "{{standard}} caption service {{number}} · {{language}}",
  "mediaFile.analysisUnknownLanguage": "unknown language",
  "mediaFile.streamUnknownCodec": "unknown codec",
  "mediaFile.streamLanguage": "Language",
  "mediaFile.streamProfileLevel": "Profile / level",
  "mediaFile.streamBitrate": "Bitrate",
  "mediaFile.streamEstimatedBitrate": "Estimated bitrate",
  "mediaFile.streamDimensions": "Dimensions",
  "mediaFile.streamPixelFormat": "Pixel format / depth",
  "mediaFile.streamFieldOrder": "Field order",
  "mediaFile.streamAspectRotation": "Aspect / rotation",
  "mediaFile.streamDeclaredFrameRate": "Declared frame rate",
  "mediaFile.streamObservedFrameRate": "Observed frame rate",
  "mediaFile.streamVfrUnknown": "variable frame rate unknown",
  "mediaFile.streamVfrObserved": "variable frame rate observed",
  "mediaFile.streamVfrNotObserved": "constant frame rate",
  "mediaFile.streamHdr": "HDR signals",
  "mediaFile.streamNoHdr": "none",
  "mediaFile.streamColor": "Color primaries / transfer / matrix",
  "mediaFile.streamContentLight": "MaxCLL / MaxFALL",
  "mediaFile.streamMasteringLuminance": "Mastering luminance",
  "mediaFile.streamDolbyVision": "Dolby Vision profile / compatibility",
  "mediaFile.streamChannels": "Channels / layout",
  "mediaFile.streamSampleRate": "Sample rate / format",
  "mediaFile.streamSampleDepth": "Sample depth",
  "mediaFile.streamUnknownLayout": "unknown layout",
  "mediaFile.diagnose": "Inspect structural headers",
  "mediaFile.diagnosing": "Inspecting headers…",
  "mediaFile.diagnosticsFailed": "Could not inspect this file",
  "mediaFile.diagnosticsScope":
    "Bounded header sampling does not validate every packet or decoded picture and audio content.",
  "mediaFile.diagnosticsRead": "{{bytes}} bytes read in {{milliseconds}} ms",
  "mediaFile.diagnosticsSamples":
    "{{packets}} transport packets and {{frames}} frame headers sampled",
  "mediaFile.diagnosticsCoverage": "{{percent}}% of source bytes inspected",
  "mediaFile.diagnosticsRanges": "Inspected byte ranges",
  "mediaFile.diagnosticsTruncated":
    "Some ranges or warnings were omitted because the report limit was reached.",
  "mediaFile.subtitleCount": "{{count}} Subs",
  "mediaFile.delete": "Delete file",
  "subtitle.search": "Search Subtitles",
  "subtitle.searching": "Searching...",
  "subtitle.download": "Download",
  "subtitle.downloading": "Downloading...",
  "subtitle.noResults": "No subtitles found",
  "subtitle.score": "Score",
  "subtitle.scoreWithValue": "Score: {{score}}",
  "subtitle.hearingImpaired": "HI",
  "subtitle.forced": "Forced",
  "subtitle.aiTranslated": "AI",
  "subtitle.machineTranslated": "Machine",
  "subtitle.external": "External",
  "subtitle.manualSearch": "Manual Search",
  "subtitle.providersRequiredTitle": "Subtitle provider required",
  "subtitle.providersRequiredBody":
    "Add and enable at least one subtitle provider before searching for subtitles.",
  "subtitle.providersRequiredAction": "Open subtitle providers",
  "subtitle.providerUnavailableTitle": "Subtitle service unavailable",
  "subtitle.providerUnavailableBody":
    "Subtitle service is unavailable for this file right now.",
  "subtitle.subtitlesDisabledTitle": "Subtitle downloads are off",
  "subtitle.subtitlesDisabledBody":
    "Turn subtitle downloads on in the subtitle settings before searching for subtitles.",
  "subtitle.blocklist": "Blocklist",
  "subtitle.blocklisted": "Subtitle added to blocklist",
  "subtitle.deleted": "Subtitle deleted",
  "subtitle.onDisk": "On Disk",
  "subtitle.releaseInfo": "Release",
  "subtitle.provider": "Provider",
  "subtitle.flags": "Flags",
  "subtitle.synced": "Synced",

  "settings.fillerPolicyLabel": "Filler episode policy",
  "settings.fillerPolicyDownloadAll": "Download all episodes",
  "settings.fillerPolicySkipFiller": "Skip filler episodes",
  "settings.recapPolicyLabel": "Recap episode policy",
  "settings.recapPolicyDownloadAll": "Download all episodes",
  "settings.recapPolicySkipRecap": "Skip recap episodes",
  "settings.releaseNumberingLabel": "Release numbering",
  "settings.releaseNumberingAuto": "Auto",
  "settings.releaseNumberingOfficial": "Official order",
  "settings.releaseNumberingAlternate": "Alternate order",
  "settings.releaseNumberingDvd": "DVD order",
  "settings.monitorSpecialsLabel": "Monitor Specials & OVAs",
  "settings.monitorSpecialsDescription":
    "Automatically monitor special episodes and OVAs when adding anime",
  "settings.interSeasonMoviesLabel": "Inter-Season Movies",
  "settings.interSeasonMoviesDescription":
    "Create narrative entries for canonical movies between seasons",
  "settings.monitorFillerMoviesLabel": "Monitor Filler Movies",
  "settings.monitorFillerMoviesDescription":
    "Include movies classified as filler in the download queue",
  "settings.importModeLabel": "Import mode",
  "settings.importModeDescription":
    "Choose whether Scryer preserves the download-side file or removes it after a verified library import.",
  "settings.importModeHardlinkCopy": "Hardlink / Copy",
  "settings.importModeMove": "Move",
  "settings.setPermissionsLinuxLabel": "Set Unix permissions",
  "settings.setPermissionsLinuxDescription":
    "Apply configured file mode and group on Unix imports. Numeric groups are treated as GIDs.",
  "settings.fileChmodLabel": "File chmod",
  "settings.folderChmodLabel": "Folder chmod",
  "settings.chownGroupLabel": "Group owner",
  "settings.fileChmodDeriveFromFolder": "Derive from folder chmod",
  "settings.chmodPresetOwnerWriteEveryoneRead":
    "Owner write, everyone else read",
  "settings.chmodPresetOwnerGroupWriteOtherRead":
    "Owner & Group write, Other read",
  "settings.chmodPresetOwnerGroupWrite": "Owner & Group write",
  "settings.chmodPresetOwnerWriteGroupRead": "Owner write, Group read",
  "settings.chmodPresetEveryoneWrite": "Everyone write",
  "settings.chmodPresetCustom": "Custom",
  "settings.sidecarFilesTitle": "Sidecar Files",
  "settings.identityStorageTitle": "Identity & storage",
  "settings.mediaProfilesTitle": "Media profiles",
  "settings.mediaProfilesHelp":
    "Leave on Inherit catalog default to follow the catalog, or override per library. The resolved value is shown beneath each control.",
  "settings.importBehaviorTitle": "Import behavior",
  "settings.libraryUnsavedChanges": "Unsaved changes",
  "settings.nfoWriteOnImportLabel": "Write NFO on Import",
  "settings.nfoWriteOnImportDescription":
    "Write Kodi/Jellyfin/Emby-compatible .nfo metadata files alongside imported media; an existing .nfo file is never overwritten",
  "settings.plexmatchWriteOnImportLabel": "Write .plexmatch on Import",
  "settings.plexmatchWriteOnImportDescription":
    "Write Plex hint files for accurate series matching without name guessing",
  "settings.minAvailabilityLabel": "Minimum Availability",
  "settings.minAvailability.default": "Use library default",
  "settings.minAvailability.announced": "Announced",
  "settings.minAvailability.in_cinemas": "In Cinemas",
  "settings.minAvailability.released": "Released",

  "status.titleRequired": "Title is required.",
  "status.facetRequired": "Facet is required.",
  "status.catalogAddSuccess": "Added {{name}} to catalog.",
  "status.catalogAddSuccessAutoSearch":
    "Added {{name}} to catalog. Automatic search queued.",
  "status.requestSubmitted": "Request submitted for {{name}}.",
  "status.queueSuccess": "Queued {{name}}",
  "status.primaryMovieFileUpdated": "Primary movie file updated",
  "status.blocklistEntryCleared": "Blocked release removed",
  "status.episodeFilesDeleteQueued":
    "Queued deletion of {{count}} episode files",
  "status.episodeFilesDeleted": "Deleted {{count}} episode files",
  "status.mediaFileDeleteQueued": "Queued deletion of the media file",
  "status.mediaFileDeleted": "Deleted the media file",
  "status.queuedLatest": "Queued latest release for {{name}}",
  "status.nothingFound": "No results found.",
  "status.searchingTvdb": 'Searching TVDB metadata for "{{query}}".',
  "status.foundTvdb": "Found {{count}} TVDB result(s).",
  "status.foundNzb": "Found {{count}} NZBGeek result(s).",
  "status.renamePreviewGenerated":
    "Rename preview ready: {{renamable}} of {{total}} file(s) can be renamed.",
  "status.renameQueued":
    "Renaming in the background; progress appears in Activity.",
  "status.releaseMissingCandidateToken":
    "Selected release is missing a verified candidate token.",
  "status.searchResultsExpired": "Search results expired, please search again",
  "status.deleteCatalogConfirm":
    "Remove {{name}} from catalog? Use the option below to also remove local files.",
  "status.titleDeleted": "{{name}} removed from catalog.",
  "status.bulkMonitorSuccess": "Monitored {{count}} titles.",
  "status.bulkMonitorPartial": "Monitored {{count}} titles; {{failed}} failed.",
  "status.bulkMonitorFailed": "Failed to monitor selected titles.",
  "status.bulkUnmonitorSuccess": "Unmonitored {{count}} titles.",
  "status.bulkUnmonitorPartial":
    "Unmonitored {{count}} titles; {{failed}} failed.",
  "status.bulkUnmonitorFailed": "Failed to unmonitor selected titles.",
  "status.bulkTitleUpdateSuccess": "Updated {{count}} titles.",
  "status.bulkTitleUpdatePartial":
    "Updated {{count}} titles; {{failed}} failed.",
  "status.bulkTitleUpdateFailed": "Failed to update selected titles.",
  "status.bulkDeletePreviewFailed":
    "Failed to prepare delete preview for {{failed}} selected titles.",
  "status.bulkTitleDeleteFailed": "Failed to delete selected titles.",
  "status.bulkRenamePreviewFailed":
    "Failed to preview renames for {{failed}} titles.",
  "status.bulkRenameFailed": "Failed to rename selected titles.",
  "status.bulkRenameQueued":
    "Renaming {{count}} titles in the background; progress appears in Activity.",
  "status.bulkRenameForbidden":
    "You do not have permission to rename titles in every selected library.",
  "status.titleMatchUpdated": "Updated match for {{name}}.",
  "status.languageChanged": "Interface language set to {{language}}.",
  "status.languageSaveFailed":
    "Could not save the language to your profile; other browsers keep their current language.",
  "status.catalogViewSaveFailed":
    "Could not save this layout to your profile; it applies in this browser only.",

  "status.editingIndexer": "Editing indexer {{name}}",
  "status.editingDownloadClient": "Editing download client {{name}}",
  "status.editingSubtitleProvider": "Editing subtitle provider {{name}}",
  "status.deletingUser": "Delete user {{name}}?",
  "status.deletingUserListsWarning":
    "The lists this user follows and their sync history are deleted too.",
  "status.userDeleted": "Deleted user {{name}}.",
  "status.userLoginEnabled": "Enabled login for {{name}}.",
  "status.userLoginDisabled": "Disabled login for {{name}}.",
  "status.deletingIndexer": "Delete indexer {{name}}?",
  "status.deletingSubtitleProvider": "Delete subtitle provider {{name}}?",
  "status.userRequired": "Username and password are required.",
  "status.passwordRequired": "Password is required.",
  "status.failedToCreate": "Failed to create item.",
  "status.failedToDelete": "Failed to delete item.",
  "status.failedToLoad": "Failed to load data.",
  "status.failedToUpdate": "Failed to update item.",
  "status.managePermissionsRequired":
    "Manage Permissions is required to edit permissions.",
  "status.indexerCreated": "Indexer saved.",
  "status.indexerUpdated": "Indexer updated.",
  "status.indexerSynced": "Synced indexers managed by {{name}}.",
  "status.mismatchRecoveryQueued":
    "Queued {{count}} mismatch recovery item(s).",
  "status.titleScanSuccess":
    "Scan complete. {{imported}} imported, {{skipped}} skipped, {{unmatched}} unmatched.",
  "status.titleMonitoringEnabled": "Title monitoring enabled.",
  "status.titleMonitoringDisabled": "Title monitoring disabled.",
  "status.indexerDeleted": "Deleted indexer {{name}}.",
  "plugins.failure.invalidConfig":
    "The plugin could not use these settings. Check them and try again.",
  "plugins.failure.authFailed":
    "The service rejected the sign-in details. Check the API key, username, or password.",
  "plugins.failure.rateLimited":
    "The service asked Scryer to slow down. Try again in a little while.",
  "plugins.failure.upstreamUnavailable":
    "The service could not be reached or did not answer. Check the address and try again.",
  "plugins.failure.unsupported": "The plugin does not support this.",
  "plugins.failure.temporary":
    "The plugin ran into a temporary problem. Try again in a little while.",
  "plugins.failure.permanent":
    "The plugin could not complete this request. Trying again will not help until something changes.",
  "plugins.failure.hostFailure":
    "The plugin stopped unexpectedly. The Scryer log has the details.",
  "plugins.failure.timedOut": "The plugin took too long to respond.",
  "plugins.failure.resourceLimit": "The plugin ran out of memory.",
  "plugins.failure.withDetail": "{{reason}} Details: {{detail}}",
  "plugins.failure.invalidUsernamePassword":
    "The service rejected the username or password. Check both and try again.",
  "plugins.failure.invalidApiKey":
    "The service rejected the API key. Check the key and try again.",
  "plugins.failure.invalidPasskey":
    "The service rejected the passkey. Check the passkey and try again.",
  "plugins.failure.invalidCookie":
    "The service rejected the session cookie. Copy a fresh cookie from your browser and try again.",
  "plugins.failure.captchaRequired":
    "The service is asking for a CAPTCHA. Sign in once on its website, then try again.",
  "plugins.failure.twoFactorRequired":
    "The service requires a two-factor code for this sign-in. Use an API key or app password instead.",
  "plugins.failure.accountBanned":
    "The service has banned this account. Contact the service to resolve it.",
  "plugins.failure.accountDisabled":
    "The service has disabled this account. Check the account on the service's website.",
  "plugins.failure.endpointNotFound":
    "The service could not be found at this address. Check the URL and base path.",
  "plugins.failure.tlsError":
    "A secure connection to the service failed. Check the address and its certificate.",
  "plugins.failure.invalidResponse":
    "The service sent a response the plugin could not read. Check that the address points to the right service.",
  "plugins.failure.quotaExceeded":
    "The account has used up its quota with the service. Wait for it to reset or raise the limit.",
  "plugins.failure.itemNotFound": "The service no longer has this item.",
  "plugins.failure.downloadRejected":
    "The download client refused this download.",
  "plugins.failure.pathNotWritable":
    "The plugin could not write to the configured folder. Check that it exists and is writable.",
  "plugins.failure.unknown": "The plugin reported a problem: {{code}}.",
  "status.testingIndexerConnection": "Testing connection…",
  "status.indexerConnectionTestPassed": "Indexer connection test passed.",
  "status.indexerConnectionTestFailed": "Indexer connection test failed.",
  "status.testingSubtitleProviderConnection":
    "Testing subtitle provider connection…",
  "status.subtitleProviderConnectionTestPassed":
    "Subtitle provider connection test passed.",
  "status.subtitleProviderConnectionTestFailed":
    "Subtitle provider connection test failed.",
  "status.ruleCreated": "Rule saved.",
  "status.ruleUpdated": "Rule updated.",
  "status.ruleDeleted": "Deleted rule {{name}}.",
  "status.ruleToggled": "Rule {{name}} {{state}}.",
  "status.editingRule": "Editing rule {{name}}",
  "status.deletingRule": "Delete rule {{name}}?",
  "status.mediaServerCreated": "Media server connection saved.",
  "status.mediaServerUpdated": "Media server connection updated.",
  "status.mediaServerDeleted": "Deleted media server {{name}}.",
  "status.plexServersDiscovered": "Plex servers loaded.",
  "status.editingMediaServer": "Editing media server {{name}}",
  "status.deletingMediaServer": "Delete media server {{name}}?",
  "status.testingMediaServerConnection": "Testing {{server}} connection…",
  "status.mediaServerConnectionTestPassed":
    "{{server}} connection test passed.",
  "status.mediaServerConnectionTestFailed":
    "{{server}} connection test failed.",
  "status.downloadClientCreated": "Download client saved.",
  "status.downloadClientUpdated": "Download client updated.",
  "status.downloadClientDeletedWithMappings":
    "Deleted download client {{name}}. {{count}} indexer(s) reverted to Automatic.",
  "status.indexerDownloadClientMappingSaving":
    "Saving download client routing…",
  "status.indexerDownloadClientMappingSaved":
    "Download client routing saved for {{name}}.",
  "status.testingDownloadClient": "Testing {{client}} connection…",
  "status.downloadClientConnectionTestPassed":
    "{{client}} connection test passed.",
  "status.downloadClientConnectionTestFailed":
    "{{client}} connection test failed.",
  "status.autoSearchNoCandidates":
    "No release found for {{name}}: no indexer returned a candidate. Check indexer routing categories and indexer health.",
  "status.autoSearchAllRejected":
    "No release found for {{name}}: automatic search rejected all {{count}} candidates. {{reasons}}",
  "status.queueFailed": "Queue operation failed.",
  "status.apiError": "Request failed",
  "status.permissionDenied": "You do not have permission for this action.",

  "user.created": "Created user {{name}}.",
  "user.permissionsUpdated": "Updated permissions for {{name}}.",
  "user.passwordUpdated": "Updated password for {{name}}.",
  "user.mfaReset": "Reset authentication factors for {{name}}.",

  "profile.accountInfo": "Account",
  "profile.appearance": "Appearance",
  "profile.appearanceDescription":
    "Personalize how Scryer looks for your account.",
  "profile.highlightColor": "Highlight color",
  "profile.highlightColorHelp":
    "Used across selection, active navigation, badges, and primary actions.",
  "profile.highlightColorCustom": "Custom color",
  "profile.highlightColorModel": "Color model",
  "profile.highlightColorApply": "Apply color",
  "profile.highlightColorCancel": "Cancel",
  "profile.highlightColorSaved": "Highlight color updated.",
  "profile.highlightColorSaveFailed": "Failed to update highlight color.",
  "color.indigo": "Indigo",
  "color.violet": "Violet",
  "color.sky": "Sky",
  "color.teal": "Teal",
  "color.emerald": "Emerald",
  "color.green": "Green",
  "color.amber": "Amber",
  "color.cyan": "Cyan",
  "color.rose": "Rose",
  "color.red": "Red",
  "color.fuchsia": "Fuchsia",
  "color.magenta": "Magenta",
  "color.crab": "Crab",
  "profile.changePassword": "Change Password",
  "profile.currentPassword": "Current Password",
  "profile.newPassword": "New Password",
  "profile.confirmPassword": "Confirm Password",
  "profile.passwordMismatch": "Passwords do not match",
  "profile.passwordUpdated": "Password updated successfully.",
  "profile.passkeys": "Passkeys",
  "profile.passkeysDescription":
    "Add or remove passkeys for faster sign-in on your devices.",
  "profile.passkeysEmpty": "No passkeys have been added yet.",
  "profile.passkeyAdd": "Add passkey",
  "profile.passkeyAdding": "Adding passkey…",
  "profile.passkeyLabel": "Passkey",
  "profile.passkeyCreatedAt": "Created",
  "profile.passkeyLastUsedAt": "Last used",
  "profile.passkeyNeverUsed": "Never used",
  "profile.passkeyAdded": "Passkey added.",
  "profile.passkeyDeleted": "Passkey deleted.",
  "profile.passkeyDeleteConfirmTitle": "Delete passkey?",
  "profile.passkeyDeleteConfirmDescription":
    "This passkey will no longer be able to sign in to your account.",
  "profile.passkeyDeleteFailed": "Failed to delete passkey.",
  "profile.passkeyOperationFailed": "Passkey request failed.",
  "profile.totp": "Authenticator app",
  "profile.totpDescription":
    "Use a one-time code from 1Password or another authenticator for step-up verification.",
  "profile.totpStartEnrollment": "Set up authenticator",
  "profile.totpOpenSetupLink": "Open setup link",
  "profile.totpSecret": "Setup key",
  "profile.totpCode": "One-time code",
  "profile.totpVerifyAndEnable": "Verify and enable",
  "profile.totpEnabled": "Authenticator app enabled.",
  "profile.totpDisabled": "Authenticator app unlinked.",
  "profile.totpEnabledAt": "Enabled",
  "profile.totpLastUsedAt": "Last used",
  "profile.totpRecoveryRemaining": "Recovery codes left",
  "profile.totpRecoveryCodes": "Recovery codes",
  "profile.totpActionDescription":
    "Enter an authenticator code to manage this authenticator.",
  "profile.totpDisable": "Unlink authenticator",
  "profile.totpRegenerateRecoveryCodes": "New recovery codes",
  "profile.totpRecoveryCodesRegenerated": "Recovery codes regenerated.",
  "profile.totpOperationFailed": "Authenticator request failed.",
  "profile.linkedAccounts": "Linked accounts",
  "profile.linkedAccountsDescription":
    "Manage Jellyfin identities linked to this Scryer account.",
  "profile.linkedAccountsEmpty": "No external accounts are linked yet.",
  "profile.linkedAccountsLoadFailed": "Failed to load linked accounts.",
  "profile.linkAccountLoadFailed": "Failed to load link options.",
  "profile.linkedAccountConnection": "Connection",
  "profile.linkedAccountUnknownConnection": "Unknown connection",
  "profile.linkedAccountStatus": "Status",
  "profile.linkJellyfinAccount": "Link Jellyfin",
  "profile.linkPlexAccount": "Link Plex",
  "profile.linkAccountConnection": "Connection",
  "profile.linkAccountUsername": "Username",
  "profile.linkAccountPassword": "Password",
  "profile.linkAccountPasswordlessHint":
    "Leave blank if this Jellyfin account has no password. Passwordless accounts can be linked, but cannot be used to sign in to Scryer.",
  "profile.linkAccountSubmit": "Link account",
  "profile.linkAccountCancel": "Cancel",
  "profile.linkAccountNoConnections": "No eligible connection available.",
  "profile.linkAccountLinked": "External account linked.",
  "profile.linkAccountFailed": "Failed to link external account.",
  "profile.signInWithPlexToLink": "Sign in with Plex",
  "profile.unlinkAccount": "Unlink",
  "profile.linkedAccountUnlinked": "Linked account removed.",
  "profile.linkedAccountUnlinkFailed": "Failed to unlink account.",

  "auth.signIn": "Sign in",
  "auth.signingIn": "Signing in…",
  "auth.username": "Username",
  "auth.password": "Password",
  "auth.totpCode": "One-time code",
  "auth.signInFailedGeneric":
    "Sign-in failed. Check your sign-in details and try again.",
  "auth.signInRateLimited":
    "Too many sign-in attempts. Please wait a moment and try again.",
  "auth.signInWithScryerPassword": "Sign in with Scryer password",
  "auth.signInWithPasskey": "Sign in with passkey",
  "auth.signInWithJellyfin": "Sign in with Jellyfin",
  "auth.jellyfinFailed": "Jellyfin sign-in failed.",
  "auth.signInWithPlex": "Sign in with Plex",
  "auth.plexFailed": "Plex sign-in failed.",
  "auth.plexPinFlowPending": "Complete sign-in in the Plex window.",
  "auth.passkeySigningIn": "Signing in with passkey…",
  "auth.passkeyUnsupported": "This browser does not support passkeys.",
  "auth.passkeyCancelled": "Passkey request was cancelled.",
  "auth.totpCodeRequired":
    "Enter your Scryer one-time code to finish signing in.",
  "auth.mfaSetupTitle": "Set up Scryer MFA",
  "auth.mfaSetupDescription":
    "Your sign-in worked. Set up Scryer one-time codes now so future sign-ins can be verified.",
  "auth.mfaSetupStartFailed": "Could not start MFA setup.",
  "auth.mfaSetupCompleteFailed": "Could not complete MFA setup.",
  "auth.mfaSetupCancel": "Return to sign in",
  "auth.mfaSetupRestart": "Restart MFA setup",
  "auth.mfaRecoveryCodesDescription":
    "Save these recovery codes somewhere safe. Each code can be used once if you cannot access your authenticator.",
  "auth.continue": "Continue",
  "auth.logoutButton": "Sign out",

  "nav.wanted": "Wanted",
  "nav.requests": "Requests",
  "requests.empty": "No pending requests for this filter.",
  "requests.emptyMine": "No requests for this filter.",
  "requests.mode.admin": "Queue",
  "requests.mode.mine": "Mine",
  "requests.approve": "Approve",
  "requests.approveTitle": "Approve request",
  "requests.approvedQualityProfile": "Approved quality profile",
  "requests.approvedMonitorType": "Approved monitoring",
  "requests.dismiss": "Dismiss",
  "requests.reopen": "Reopen",
  "requests.requesterFilter.label": "Requester",
  "requests.requesterFilter.all": "All requesters",
  "status.requestReopened": "Reopened request for {{name}}.",
  "requests.modify": "Modify",
  "requests.modifyTitle": "Modify request",
  "requests.saveChanges": "Save changes",
  "requests.cancelRequest": "Cancel request",
  "requests.requesters": "Requesters",
  "requests.requestedQualityProfile": "Requested quality profile",
  "requests.requestedMonitorType": "Requested monitoring",
  "requests.libraryDefaultProfile": "Library default",
  "requests.updated": "Updated",
  "requests.status.pending": "Pending",
  "requests.status.approved": "Approved",
  "requests.status.canceled": "Canceled",
  "status.requestApproved": "Approved {{name}}.",
  "status.requestApprovedSearchFailed":
    "Approved {{name}}, but the wanted search was not queued: {{error}}",
  "status.requestDismissed": "Dismissed request for {{name}}.",
  "status.requestUpdated": "Updated request for {{name}}.",
  "status.requestCanceled": "Canceled request for {{name}}.",
  "wanted.title": "Wanted Items",
  "wanted.refreshing": "Refreshing…",
  "wanted.filterTitle": "Title filter",
  "wanted.filterTitlePlaceholder": "Filter titles…",
  "wanted.totalCount": "{{count}} items",
  "wanted.colTitle": "Title",
  "wanted.colType": "Type",
  "wanted.colStatus": "Status",
  "wanted.colConvergence": "Convergence",
  "wanted.colLastSearch": "Last search",
  "wanted.colIndexers": "Indexers",
  "wanted.convergence.searching": "Searching {{covered}}/{{routed}}",
  "wanted.convergence.converged": "Watching RSS",
  "wanted.convergence.convergedHint":
    "Searched all indexers — new releases arrive via RSS. Run a search to re-check.",
  "wanted.convergence.deferred": "Waiting on indexer quota",
  "wanted.convergence.deferredHint":
    "All remaining indexers are cooling down or out of quota; searching resumes automatically.",
  "wanted.convergence.queuedHot": "Queued · recent",
  "wanted.convergence.queuedCold": "Queued · long tail",
  "wanted.searchJobStarted": "Search started.",
  "wanted.searchJobProgress":
    "{{processed}}/{{total}} search steps completed. {{title}}",
  "wanted.searchNoEligibleWork":
    "No monitored items need downloading or upgrading.",
  "wanted.searchNoAcceptableReleases":
    "Search completed; no acceptable releases found.",
  "wanted.searchSnapshotMissing":
    "The search response was incomplete. Check Activity before trying again.",
  "wanted.searchInvalidSeason":
    "This season could not be identified. Refresh the title and try again.",
  "wanted.searchJobComplete":
    "Search finished: {{processed}} searched, {{grabbed}} grabbed, {{failed}} failed.",
  "wanted.searchJobCancelled":
    "Search cancelled after {{processed}} item(s); {{grabbed}} grabbed.",
  "wanted.colPhase": "Phase",
  "wanted.colLatestDecision": "Latest decision",
  "wanted.colScore": "Score",
  "wanted.searchNow": "Search now",
  "wanted.pause": "Pause",
  "wanted.resume": "Resume",
  "wanted.actionRecoverMismatch": "Recover mismatch",
  "wanted.status.wanted": "Wanted",
  "wanted.status.grabbed": "Grabbed",
  "wanted.status.completed": "Completed",
  "wanted.status.paused": "Paused",
  "wanted.type.movie": "Movie",
  "wanted.type.episode": "Episode",
  "wanted.type.seriesMovie": "Series movie",
  "wanted.decision.eligible": "Eligible",
  "wanted.decision.titleMismatch": "Title mismatch",
  "wanted.decision.titleYearMismatch": "Different year",
  "wanted.decision.episodicReleaseForMovie": "Episodes, not a movie",
  "wanted.decision.episodeMismatch": "Episode mismatch",
  "wanted.decision.categoryMismatch": "Category mismatch",
  "wanted.decision.ambiguousIdentity": "Ambiguous identity",
  "wanted.decision.qualityBlocked": "Quality blocked",
  "wanted.decision.minimumSeeders": "Too few seeders",
  "wanted.decision.packBelowMissingThreshold": "Pack below missing threshold",
  "wanted.decision.packForSingleEpisode": "Pack for a single episode",
  "wanted.decision.upgradeRejected": "Upgrade rejected",
  "wanted.decision.pendingDelay": "Pending delay",
  "wanted.decision.alreadyActive": "Already active",
  "wanted.decision.acceptInitial": "Accept initial",
  "wanted.decision.acceptUpgrade": "Accept upgrade",
  "wanted.decision.rejectInsufficientDelta": "Insufficient delta",
  "wanted.decision.rejectCooldown": "Cooldown",
  "wanted.decision.rejectNotAllowed": "Not allowed",
  "wanted.loadingDecisions": "Loading release decisions…",
  "wanted.noDecisions": "No release decisions recorded yet.",
  "wanted.standby": "Standby · {{count}}",
  "wanted.standbyTooltip":
    "{{count}} saved candidates will be tried if the current grab fails.",
  "wanted.standbyScopeNote":
    "Counts rows keyed to this scope; season packs are counted on the season's anchor episode.",
  "wanted.loadingStandby": "Loading saved candidates…",
  "wanted.standbyCandidates": "Standby candidates · {{count}}",
  "wanted.standbyRank": "Rank {{rank}}",
  "wanted.standbyIndexer": "Indexer",
  "wanted.standbySize": "Size",
  "wanted.standbySeeders": "Seeders",
  "wanted.standbyScore": "Score",
  "wanted.standbyAge": "Age",
  "wanted.seasonPack": "Season pack",
  "wanted.noStandbyCandidates": "no standby candidates",
  "wanted.noItems": "No wanted items found.",
  "wanted.context.movie": "Movie wanted item",
  "wanted.context.episode": "Episode wanted item",
  "wanted.context.seasonEpisode": "Season {{seasonNumber}} episode wanted item",
  "wanted.context.seriesMovie": "Series movie wanted item",
  "wanted.prev": "Previous",
  "wanted.next": "Next",
  "wanted.timeNow": "now",
  "wanted.timeHoursMinutes": "{{hours}}h {{minutes}}m",
  "wanted.timeMinutes": "{{minutes}}m",
  "wanted.decRelease": "Release",
  "wanted.decDecision": "Decision",
  "wanted.decScore": "Score",
  "wanted.decDelta": "Delta",
  "wanted.decSize": "Size",
  "wanted.decDate": "Date",
  "wanted.scoreBreakdown": "Scoring breakdown",
  "wanted.scoreCode": "Code",
  "wanted.tabWanted": "Wanted Items",
  "wanted.tabCutoff": "Cutoff Unmet",

  "cutoff.title": "Upgrades",
  "cutoff.searchAll": "Search All",
  "cutoff.searchProgress": "Searching {{current}} of {{total}}…",
  "cutoff.colTitleEpisode": "Title / Episode",
  "cutoff.colCurrentQuality": "Current",
  "cutoff.colTargetQuality": "Target",
  "cutoff.noItems":
    "All monitored movies and episodes meet their quality profile cutoff.",
  "cutoff.totalCount": "{{count}} items below cutoff",
  "cutoff.filterFacet": "Type",
  "cutoff.allFacets": "All types",
  "cutoff.searchTriggered": "Upgrade search triggered for {{name}}",

  "settings.notifications": "Notifications",
  "settings.notificationChannels": "Channels",
  "settings.notificationSubscriptions": "Subscriptions",
  "settings.notificationChannelCreate": "Add Channel",
  "settings.notificationChannelCreateNew": "Create new channel",
  "settings.notificationChannelUpdate": "Update Channel",
  "settings.notificationSubscriptionCreate": "Add Subscription",
  "settings.notificationSubscriptionCreateNew": "Create new subscription",
  "settings.notificationSubscriptionUpdate": "Update Subscription",
  "settings.notificationChannelConfirmDiscardTitle":
    "Discard current channel edits?",
  "settings.notificationChannelConfirmDiscardDescription":
    "Your current notification channel edits will be lost.",
  "settings.notificationSubscriptionConfirmDiscardTitle":
    "Discard current subscription edits?",
  "settings.notificationSubscriptionConfirmDiscardDescription":
    "Your current notification subscription edits will be lost.",
  "settings.notificationNoChannels": "No notification channels configured.",
  "settings.notificationNoSubscriptions":
    "No notification subscriptions configured.",
  "settings.notificationEventType": "Event Type",
  "settings.notificationScope": "Scope",
  "settings.notificationScope.global": "Global",
  "settings.notificationScope.facet": "Facet",
  "settings.notificationScope.title": "Title",
  "settings.notificationScopeId": "Scope ID",
  "settings.notificationScopeIdPlaceholderTitle": "Title ID",
  "settings.notificationChannel": "Channel",
  "settings.notificationTest": "Test",
  "settings.notificationTestSuccess": "Test notification sent successfully.",
  "settings.notificationTestFailed": "Test notification failed.",
  "settings.notificationProviderType": "Provider Type",
  "settings.notificationNoProviders":
    "No notification plugins installed. Install a notification plugin from the Plugins page first.",
  "settings.notificationJellyfinSubscriptionHint":
    "Import Complete is the usual post-import Jellyfin scan trigger. Only Jellyfin events that can refresh the library are available here.",
  "settings.notificationEvent.grab": "Grabbed Release",
  "settings.notificationEvent.download": "Download Failed",
  "settings.notificationEvent.upgrade": "Upgrade Imported",
  "settings.notificationEvent.importComplete": "Import Completed",
  "settings.notificationEvent.importRejected": "Import Blocked",
  "settings.notificationEvent.rename": "File Renamed",
  "settings.notificationEvent.titleAdded": "Title Added",
  "settings.notificationEvent.titleDeleted": "Title Deleted",
  "settings.notificationEvent.titleMoved": "Title Moved",
  "settings.notificationEvent.fileDeleted": "File Deleted",
  "settings.notificationEvent.fileDeletedForUpgrade":
    "File Deleted For Upgrade",
  "settings.notificationEvent.fileRestored": "File Restored",
  "settings.notificationEvent.postProcessingCompleted":
    "Post-Processing Completed",
  "settings.notificationEvent.subtitleDownloaded": "Subtitle Downloaded",
  "settings.notificationEvent.subtitleSearchFailed": "Subtitle Search Failed",
  "settings.notificationEvent.mediaRequestSubmitted": "Media Request Submitted",
  "settings.notificationEvent.mediaRequestApproved": "Media Request Approved",
  "settings.notificationEvent.mediaRequestRejected": "Media Request Rejected",
  "settings.notificationEvent.mediaRequestCanceled": "Media Request Canceled",
  "settings.notificationEvent.healthIssue": "Health Issue",
  "settings.notificationEvent.healthRestored": "Health Restored",
  "settings.notificationEvent.applicationUpdate": "Application Update",
  "settings.notificationEvent.manualInteractionRequired":
    "Manual Interaction Required",
  "settings.notificationEvent.test": "Test Notification",
  "settings.notificationEvent.listTitleAdded": "List Added Title",
  "settings.notificationEvent.listRequestSubmitted": "List Request Submitted",
  "settings.notificationEvent.listItemHeld": "List Item Held",
  "settings.notificationEvent.listTitleLeft": "Title Left List",
  "settings.notificationEvent.listSyncFailed": "List Sync Failed",
  "settings.notificationEvent.listUnfollowed": "List Unfollowed",
  "status.notificationChannelCreated": "Notification channel saved.",
  "status.notificationChannelUpdated": "Notification channel updated.",
  "status.notificationChannelDeleted": "Deleted notification channel {{name}}.",
  "status.notificationSubscriptionCreated": "Notification subscription saved.",
  "status.notificationSubscriptionUpdated":
    "Notification subscription updated.",
  "status.notificationSubscriptionDeleted":
    "Notification subscription deleted.",
  "status.editingNotificationChannel": "Editing channel {{name}}",
  "status.deletingNotificationChannel": "Delete notification channel {{name}}?",
  "status.editingNotificationSubscription": "Editing subscription",
  "status.deletingNotificationSubscription":
    "Delete this notification subscription?",

  "wanted.tabPending": "Pending",
  "pending.title": "Pending Releases",
  "pending.noItems": "No pending releases.",
  "pending.colRelease": "Release",
  "pending.colScore": "Score",
  "pending.colSize": "Size",
  "pending.colIndexer": "Indexer",
  "pending.colAddedAt": "Added",
  "pending.colDelayUntil": "Available",
  "pending.colReason": "Reason",
  "pending.colRole": "Role",
  "pending.role.primary": "Primary",
  "pending.role.fallback": "Fallback",
  "pending.status.needsReview": "Needs review",
  "pending.phase.needsReview": "Review",
  "pending.forceGrab": "Grab now",
  "pending.dismiss": "Dismiss",
  "pending.grabbed": "Release grabbed.",
  "pending.grabRejected":
    "Grab rejected — the release is blocked or unavailable.",
  "pending.dismissed": "Release dismissed.",

  "setup.welcomeTitle": "Welcome to Scryer",
  "setup.welcomeDescription":
    "Let's configure the essentials to get you started.",
  "setup.freshSetup": "Set up from scratch",
  "setup.freshSetupDescription":
    "Configure media paths, install providers like qBittorrent, and add clients and indexers step by step.",
  "setup.importSetup": "Import from Sonarr / Radarr",
  "setup.importSetupDescription":
    "Bring your existing configuration into Scryer.",
  "setup.restoreSetup": "Restore from Backup",
  "setup.restoreSetupDescription":
    "Upload a Scryer backup bundle and restore this fresh instance.",
  "setup.stepPersona": "Persona",
  "setup.stepMediaPaths": "Media Paths",
  "setup.stepPlugins": "Plugins",
  "setup.stepDownloadClient": "Download Client",
  "setup.stepIndexer": "Indexer",
  "setup.stepSummary": "Summary",
  "setup.stepConnect": "Connect",
  "setup.stepRestore": "Restore",
  "setup.restoreTitle": "Restore from a backup bundle",
  "setup.restoreDescription":
    "Upload a Scryer backup bundle, inspect its manifest, and restore it into this fresh instance.",
  "setup.restoreSelectBundle": "Backup bundle",
  "setup.restoreDropTargetTitle": "Drag and drop a backup bundle here",
  "setup.restoreDropTargetDescription":
    "Choose a backup file from your device or drop it here to inspect before restoring.",
  "setup.restoreDropTargetSelected": "Bundle selected and ready to inspect.",
  "setup.restoreDropTargetEncryptedSelected":
    "Encrypted bundle selected. Enter the password used during backup creation to continue.",
  "setup.restoreDropTargetFormats":
    "Only encrypted .enc backups are supported.",
  "setup.restoreSelectFile": "Select file",
  "setup.restoreClearFile": "Clear selection",
  "setup.restoreNoFile": "Choose a backup bundle to continue.",
  "setup.restorePasswordRequired":
    "Enter the bundle password before continuing.",
  "setup.restoreInspect": "Inspect bundle",
  "setup.restoreApply": "Restore now",
  "setup.restoreChooseAnother": "Choose another bundle",
  "setup.restoreSummaryTitle": "Bundle contents",
  "setup.restoreConfirmDescription":
    "This replaces the fresh setup database, writes restored instance secrets, and restarts Scryer into the restored instance.",
  "setup.restoreRefusal.deadlineExceeded":
    "The restore ran past its one-hour limit and was stopped safely. Try again.",
  "setup.restoreRefusal.maintenanceActive":
    "A restore or recovery is already in progress. Wait for it to finish.",
  "setup.restoreRefusal.interrupted":
    "The restore was interrupted before it finished. Try again.",
  "setup.restoreRefusal.attemptNotFound":
    "This restore attempt no longer exists. Start the restore again.",
  "setup.restoreRefusal.recoveryRequired":
    "The restore could not be confirmed safe. Check the server logs before trying again.",
  "setup.restoreRefusal.operationInvalid":
    "Restore requests must be sent on their own. Reload the page and try again.",
  "setup.restoreRefusal.restartFailed":
    "The restore was prepared, but Scryer could not restart into it. Retry restore runs the restart again without staging the backup twice, or restart Scryer to finish.",
  "setup.restoreCreatedAt": "Created",
  "setup.restoreSourceVersion": "Source version",
  "setup.restoreSourceEngine": "Source engine",
  "setup.restoreMigrationKey": "Source migration key",
  "setup.restoreTotalRows": "Total rows",

  "setup.personaTitle": "Quality Preferences",
  "setup.personaDescription":
    "Choose a quality target and scoring focus for each content type.",
  "setup.facetMovies": "Movies",
  "setup.facetSeries": "Series",
  "setup.facetAnime": "Anime",
  "setup.qualityTarget": "Quality",
  "setup.scoringFocus": "Persona",
  "setup.personaBalancedDesc":
    "Sensible defaults for most setups. Good quality without chasing remux releases.",
  "setup.personaAudiophileDesc":
    "Maximum fidelity. Prioritizes BluRay, remux, lossless audio, HDR, and large files.",
  "setup.personaEfficientDesc":
    "Best quality per GB. Prefers x265, WEB-DL, compact encodes.",
  "setup.personaCompatibleDesc":
    "Plays on everything. H.264 over H.265, avoids Dolby Vision, broad device support.",

  "setup.pluginsTitle": "Install providers you want to use",
  "setup.pluginsDescription":
    "Some services, like qBittorrent, are added as plugins before they can be configured in Scryer.",
  "setup.pluginsBuiltInTitle": "Already included",
  "setup.pluginsBuiltInDescription":
    "NZBGet and SABnzbd are ready to configure without installing anything extra.",
  "setup.pluginsAvailableHeading": "Official plugins",
  "setup.pluginsAvailableHint":
    "Install the official plugins you plan to use on this host. You can still change them later in Settings.",
  "setup.pluginsNoneFound":
    "No plugins were found. Refresh the registry or continue with built-in providers.",
  "setup.recommendedHeading": "Recommended",
  "setup.recommendedHint":
    "The plugins most setups need. Install the ones that match how you use Scryer.",
  "setup.recommendedArchiveTitle": "Archive Extraction",
  "setup.recommendedArchiveReason":
    "Install if you use torrents or download subtitles. Both often arrive packed in RAR or ZIP archives.",
  "setup.recommendedQbittorrentTitle": "qBittorrent",
  "setup.recommendedSubtitlesTitle": "Subtitles",
  "setup.recommendedSubtitlesReason":
    "Install if Scryer manages your subtitles. Enhanced Sync lines subtitles up with your video, OpenSubtitles covers movies and TV, and Jimaku and AnimeTosho cover anime.",
  "setup.recommendedAdvancedIndexersTitle": "Advanced indexers",
  "setup.recommendedAdvancedIndexersReason":
    "Install if you use Prowlarr's more complex indexers, the ones built on its Cardigann engine.",
  "setup.recommendedMediaServerTitle": "Media server",
  "setup.recommendedMediaServerReason":
    "Refreshes your media server when Scryer imports something. Pick the one you use.",
  "setup.rulePacksHeading": "Rule packs",
  "setup.rulePacksHint":
    "Community scoring rules that help Scryer pick the best release. You can adjust them later in Settings → Rules.",
  "setup.rulePackTrashTitle": "TRaSH Guides",
  "setup.rulePackTrashReason":
    "Scores releases by the TRaSH Guides' recommendations for release groups, audio, HDR, size and unwanted releases.",
  "setup.rulePackSeadexTitle": "SeaDex",
  "setup.rulePackSeadexReason":
    "Turn on if you collect anime. Prefers the releases SeaDex lists as the best available.",
  "setup.rulePackUnavailable":
    "Not available right now. Refresh the registry to try again.",
  "setup.otherPluginsHeading": "More plugins",

  "setup.mediaPathsTitle": "Where is your media stored?",
  "setup.mediaPathsDescription":
    "Set the root folders where Scryer will organize your movies and series.",
  "setup.moviesPath": "Movies path",
  "setup.seriesPath": "Series path",
  "setup.animePath": "Anime path",
  "setup.optional": "(optional)",
  "setup.browse": "Browse",
  "setup.addMediaFolder": "Add folder",
  "setup.removeMediaFolder": "Remove folder",
  "setup.mediaPathsNoneChosen":
    "No folders chosen, so this library keeps the folders it has.",
  "setup.mediaPathNotReachable": "Not reachable yet",
  "setup.mediaPathsNotReachableWarning":
    "Some media paths are not reachable yet. They were saved and can be changed later in Settings.",
  "setup.mediaPathsVerificationUnavailable":
    "Could not verify these paths. You can continue and change them later in Settings.",

  "setup.downloadClientTitle": "Add a download client",
  "setup.downloadClientDescription":
    "Scryer sends downloads to your preferred client. Plugins installed in the previous step will appear here.",

  "setup.indexerTitle": "Add an indexer",
  "setup.indexerDescription":
    "Indexers provide the release database Scryer searches.",

  "setup.summaryTitle": "You're all set",
  "setup.summaryDescription":
    "Here's what you configured. You can change any of these later in Settings.",
  "setup.summaryPersona": "Persona",
  "setup.summaryMediaPaths": "Media paths",
  "setup.summaryDownloadClient": "Download client",
  "setup.summaryIndexer": "Indexer",

  "setup.next": "Next",
  "setup.back": "Back",
  "setup.finish": "Finish Setup",
  "setup.importOnly": "Import only",
  "setup.importAndScan": "Import & Scan",
  "setup.connectionSuccess": "Connection successful",
  "setup.connectionFailed": "Connection failed",
  "setup.saved": "Saved",
  "setup.required": "This field is required",
  "setup.skip": "Skip",
  "setup.skipSetup": "Skip setup entirely",
  "setup.skipping": "Skipping…",

  "setup.findApiKey": "Find your API key",
  "setup.connectAndScan": "Connect & Scan",
  "setup.connectError": "Connection failed",

  "setup.connected": "connected",
  "setup.notSupported": "Not supported",
  "setup.importing": "Importing…",
  "setup.summaryImportedClients": "Download clients imported",
  "setup.summaryImportedIndexers": "Indexers imported",
  "setup.monitorWarmupTitle": "Preparing monitored-state import",
  "setup.monitorWarmupDescription":
    "Scryer is pulling monitored status in the background so the final import step is faster.",
  "setup.monitorWarmupQueued": "Waiting for progress details…",
  "setup.monitorWarmupLoadingIndexers": "Discovering indexers",
  "setup.monitorWarmupLoadingMovies": "Loading movies",
  "setup.monitorWarmupLoadingSeries": "Loading series",
  "setup.monitorWarmupLoadingEpisodes": "Loading episodes",
  "setup.monitorWarmupBuildingSnapshot": "Building snapshot",
  "setup.monitorWarmupReady": "Ready",
  "setup.monitorWarmupFailed":
    "Background preparation hit an error. Scryer will retry when you start the import.",
  "setup.monitorWarmupCanceled":
    "Background preparation was canceled. Scryer will restart it if needed.",
  "setup.importFinalizeFailed":
    "Scryer could not finish preparing monitored state for this import.",
  "setup.apiKeyMasked":
    "Sonarr/Radarr hides this API key. Enter it manually to continue importing:",

  // ── Multi-instance import wizard ──────────────────────────────────────────
  "setup.stepLibraries": "Libraries",
  "setup.stepQuality": "Quality",
  "setup.stepSources": "Sources",
  "setup.continue": "Continue",
  "setup.finishImport": "Finish import",
  "setup.connectMultiTitle": "Connect to Sonarr, Radarr & Prowlarr",
  "setup.connectMultiDescription":
    "Add the URL and API key for every instance Scryer should pull from. Connect more than one of each — at least one is required.",
  "setup.librariesTitle": "Libraries & mapping",
  "setup.librariesDescription":
    "Create a library for each way you want to sort content, then map every source root into it.",
  "setup.qualityPersonaTitle": "Quality & persona",
  "setup.qualityPersonaDescription":
    "Pick the quality profile and tuning persona for every library you mapped.",
  "setup.sourcesTitle": "Clients & indexers",
  "setup.sourcesDescription":
    "Choose which download clients and indexers to bring across.",
  "setup.importReadyTitle": "Ready to import",
  "setup.importReadyDescription":
    "Review everything before Scryer pulls it in.",
  // Connect (instances)
  "setup.addInstance": "Add {{product}}",
  "setup.instanceName": "Instance name",
  "setup.unnamedInstance": "Unnamed instance",
  "setup.removeInstance": "Remove instance",
  "setup.testing": "Testing…",
  "setup.couldntConnect": "Couldn’t connect",
  "setup.enterUrlAndKey": "Enter URL & API key to verify",
  "setup.apiKeyHelpHint": "Settings → General",
  "setup.secretDraftOtherUserWarning":
    "Another user has a saved import draft. Entering your own keys here will replace their saved credentials.",
  // Libraries / mapping board
  "setup.sourceRoots": "Source roots",
  "setup.addSourceRoot": "Add source root",
  "setup.chooseFolder": "Choose folder…",
  "setup.unmappedCountOne": "1 unmapped",
  "setup.unmappedCountMany": "{{count}} unmapped",
  "setup.allRootsMapped":
    "All source roots mapped — drag one back here to unassign",
  "setup.dragRootIntoLibrary": "Drag each root into a library",
  "setup.dropRootsHere": "Drop roots here",
  "setup.removeSourceRoot": "Remove source root",
  "setup.manualRootPathAria": "Manual source root path",
  "setup.renameLibraryAria": "Rename {{name}}",
  "setup.previewLoading": "Loading source roots from your instances…",
  "setup.previewError": "Couldn’t load source roots.",
  "setup.retry": "Retry",
  "setup.noCompatibleLibrary": "No compatible library yet.",
  "setup.createFacetLibrary": "Create a {{facet}} library",
  "setup.addLibrary": "Add library",
  "setup.newLibraryFor": "New library for…",
  "setup.cancel": "Cancel",
  "setup.renameLibrary": "Rename",
  "setup.renameInstance": "Rename",
  "setup.deleteLibrary": "Delete library",
  "setup.remap": "Remap",
  "setup.assignToLibrary": "Assign to library",
  "setup.moveToAnotherLibrary": "Move to another library",
  // Assign sheet
  "setup.assignSheetTitle": "Assign to library",
  "setup.moveToUnassigned": "Move to unassigned",
  "setup.noPathSet": "(no path set)",
  // Remap dialog
  "setup.remapTitle": "Remap path",
  "setup.remapExplain":
    "Scryer keeps the path {{instance}} reports as provenance, so it can re-apply monitored status correctly. If those files live at a different path on the Scryer host, set it below.",
  "setup.remapAsReported": "As reported by source",
  "setup.remapScryerHostPath": "Path on Scryer host",
  "setup.remapResetToSource": "Reset to source",
  "setup.remapSave": "Save remap",
  "setup.mappedPathValidationChecking":
    "Checking that Scryer can access every mapped source path…",
  "setup.invalidMappedPathTitle": "Scryer can’t access this mapped source path",
  "setup.invalidMappedPathsTitle":
    "Scryer can’t access {{count}} mapped source paths",
  "setup.invalidMappedPathsHelp":
    "These roots are assigned, but their paths are not visible to Scryer. Remap each one to the equivalent folder on the Scryer host before continuing.",
  "setup.remapInvalidRootAria": "Remap {{name}} from {{path}}",
  // Quality & persona
  "setup.library": "Library",
  "setup.qualityProfile": "Quality profile",
  "setup.persona": "Persona",
  "setup.qualityNoLibraries":
    "Map at least one source root to a library to set its quality.",
  "setup.persona.balanced.name": "Balanced",
  "setup.persona.balanced.desc": "Solid quality at a reasonable size",
  "setup.persona.audiophile.name": "Audiophile",
  "setup.persona.audiophile.desc": "Maximum fidelity, larger files",
  "setup.persona.efficient.name": "Efficient",
  "setup.persona.efficient.desc": "Smaller encodes, save disk",
  "setup.persona.compatible.name": "Compatible",
  "setup.persona.compatible.desc": "Direct-play friendly, wide support",
  // Sources
  "setup.downloadClients": "Download clients",
  "setup.indexers": "Indexers",
  "setup.merged": "MERGED",
  "setup.from": "FROM",
  "setup.passwordRequired": "Password is required.",
  "setup.indexerApiKeyHelpLink": "Find your indexer’s API key",
  "setup.provenanceSource": "Source",
  "setup.provenanceScryer": "Scryer",
  "setup.sourcesLoading": "Loading clients and indexers…",
  "setup.duplicatesMerged": "Duplicates merged across instances",
  // Summary
  "setup.summaryLibraries": "Libraries",
  "setup.summaryLibrariesDetail": "Across Movies, Series and Anime",
  "setup.summaryInstancesConnected": "Instances connected",
  "setup.summaryInstancesConnectedDetail":
    "{{sonarr}} Sonarr · {{radarr}} Radarr",
  "setup.summaryRootsMapped": "Source roots mapped",
  "setup.summaryRootsMappedDetail": "Routed to a Scryer library",
  "setup.summaryPathsRemapped": "Paths remapped",
  "setup.summaryPathsRemappedDetail": "Provenance kept for monitored status",
  "setup.summaryClientsMerged": "Merged across instances",
  "setup.summaryIndexersEnabled": "Supported & enabled",
  "setup.fetchingMonitoredStatus": "Fetching monitored status…",
  "setup.fetchingMonitoredStatusDetail":
    "Pulling each title’s monitored state from your instances in the background.",
  "setup.monitoredStatusSynced": "Monitored status synced",
  "setup.monitoredStatusSyncedDetail":
    "Every title’s monitored state was pulled from your instances.",
  "setup.importGateHint":
    "Import can’t finish until the monitored-status sync completes.",
  "setup.importGateDoneHint": "Done — you can finish import now.",
  "setup.titlesFetched": "{{fetched}} / {{total}} titles",
  "setup.importWarmupFailedTitle": "Monitored-status sync failed",
  "setup.importWarmupFailedDetail":
    "Scryer couldn’t finish pulling monitored state from your instances. Check the connections and try again.",
  "setup.importWarmupRetrying": "Restarting sync…",
  "setup.importWarmupSessionExpiredTitle": "Import preparation expired",
  "setup.importWarmupSessionExpired":
    "Your import preparation is no longer available — Scryer may have restarted, or it sat idle too long. Reconnect to your instances to start a fresh import.",
  "setup.importReconnect": "Reconnect",
  "setup.applyingImport": "Applying import…",
  "setup.applyingImportDetail":
    "Writing the imported monitored state into your libraries. This keeps running if you leave the page.",
  "setup.importApplied": "Import applied",
  "setup.importAppliedDetail":
    "Every imported title's monitored state was written to your libraries.",
  "setup.importFinalizeFailedTitle": "Finishing the import failed",
  "setup.titlesApplied": "{{applied}} / {{total}} titles",
  "setup.importUnmappedRootsNotice":
    "Some detected source folders still need a library. Go back to the Libraries step to map them before finishing.",

  "settings.runSetupWizard": "Run setup wizard",

  "history.title": "History",
  "history.empty": "No history events",
  "history.loadError": "History could not be loaded.",
  "history.retry": "Retry",
  "history.event": "Event",
  "history.initial": "Initial",
  "history.upgrade": "Upgrade",
  "history.prospectiveEpisodes": "Prospective Episodes",
  "history.titleColumn": "Title",
  "history.sourceTitle": "Source Title",
  "history.quality": "Quality",
  "history.date": "Date",
  "history.actor": "Actor",
  "history.allEvents": "All Events",
  "history.eventTypes": "Event types",
  "history.eventTypesSelected": "{{count}} event types",
  "history.grabbed": "Grabbed",
  "history.downloadFailed": "Download Failed",
  "history.blocklisted": "Blocklisted",
  "history.scanned": "Scanned",
  "history.imported": "Imported",
  "history.importFailed": "Import Failed",
  "history.importSkipped": "Import Skipped",
  "history.importRejectedByRule": "Rejected by Rule",
  "history.downloadCompleted": "Downloaded",
  "history.fileUpgraded": "Upgraded",
  "history.fileRecycled": "Recycled",
  "history.fileDeleted": "Deleted",
  "history.fileRestored": "Restored",
  "history.fileRenamed": "Renamed",
  "history.titleMoved": "Moved",
  "history.rematched": "Rematched",
  "history.seedingStarted": "Seeding",
  "history.seedingCompleted": "Seeding done",
  "history.downloadIgnored": "Ignored",
  "history.downloadMissing": "Not visible in client",
  "history.facet": "Facet",
  "history.actions": "Actions",
  "history.expandDetails": "Expand event details",
  "history.collapseDetails": "Collapse event details",
  "history.collectionId": "Collection ID",
  "history.episodeCountSingle": "1 episode",
  "history.episodeCountMultiple": "{{count}} episodes",
  "history.unknownEvent": "Unknown",
  "history.noMore": "No more events",
  "history.loadMore": "Load more",

  // Indexer error diagnostics
  "indexerErrors.title": "Indexer error history",
  "indexerErrors.scopedTitle": "{{indexer}} — error history",
  "indexerErrors.history": "Error history",
  "indexerErrors.refresh": "Refresh",
  "indexerErrors.retry": "Retry",
  "indexerErrors.loading": "Loading indexer errors…",
  "indexerErrors.loadingResponse": "Loading the captured response…",
  "indexerErrors.loadMore": "Load more",
  "indexerErrors.noMore": "No more captured responses",
  "indexerErrors.loadError": "Indexer error history could not be loaded.",
  "indexerErrors.detailLoadError": "The captured response could not be loaded.",
  "indexerErrors.detailMissing":
    "This captured response is no longer available.",
  "indexerErrors.empty": "No indexer errors were recorded in the last 30 days.",
  "indexerErrors.retentionNotice":
    "Indexer errors are retained for 30 days. HTTP responses include captured headers and body; transport failures retain diagnostics only.",
  "indexerErrors.occurred": "Occurred",
  "indexerErrors.indexer": "Indexer",
  "indexerErrors.operation": "Operation",
  "indexerErrors.http": "HTTP",
  "indexerErrors.classification": "Classification",
  "indexerErrors.message": "Message",
  "indexerErrors.contentType": "Content type",
  "indexerErrors.providerCode": "Provider code {{code}}",
  "indexerErrors.expand": "Expand captured response",
  "indexerErrors.collapse": "Collapse captured response",
  "indexerErrors.response": "Captured HTTP response",
  "indexerErrors.noHttpResponse":
    "No HTTP response was received; this transport failure has diagnostic details only.",
  "indexerErrors.headers": "Response headers",
  "indexerErrors.noHeaders": "No response headers were captured.",
  "indexerErrors.body": "Response body",
  "indexerErrors.formatted": "Formatted",
  "indexerErrors.raw": "Raw",
  "indexerErrors.redacted": "[redacted before storage]",
  "indexerErrors.base64Value": "Base64",
  "indexerErrors.previewTruncated":
    "Preview limited to the first 1 MiB of {{size}}. Download the full body to inspect all captured bytes.",
  "indexerErrors.emptyBody": "The response body is empty.",
  "indexerErrors.binaryBody":
    "This {{size}} response is not valid UTF-8. Download it to inspect the captured bytes.",
  "indexerErrors.downloadFullBody": "Download full body",

  // ── Convenience Rules ─────────────────────────────────────────────────

  // Recycle Bin
  "settings.autoBackupsTitle": "Automatic backups",
  "settings.autoBackupsDisabledMissingKeyToastTitle":
    "Automatic backups were disabled",
  "settings.autoBackupsDisabledMissingKeyToastDescription":
    "Automatic backups now require a saved backup key. Open Backups to set a key and re-enable them.",
  "settings.autoBackupsDisabledMissingKeyToastAction": "Open Backups",
  "settings.autoBackupsTime": "Daily backup time",
  "settings.autoBackupsHour": "Hour",
  "settings.autoBackupsMinute": "Minute",
  "settings.autoBackupsNextRun": "Next automatic backup",
  "settings.autoBackupsNextRunNeedsKey": "Disabled (save password first)",
  "settings.autoBackupsKeyLabel": "Automatic backup key",
  "settings.autoBackupsKeyHelpLabel": "About the automatic backup key",
  "settings.autoBackupsSetKey": "Set automatic backup key",
  "settings.autoBackupsKeyAlreadySetHint": "(backup key already set)",
  "settings.autoBackupsKeyCleared": "Backup key cleared.",
  "settings.autoBackupsClearKeyTitle": "Clear backup key?",
  "settings.autoBackupsClearKeyConfirm":
    "The saved automatic backup key will be removed. Existing backups made with it still need it to restore.",
  "settings.autoBackupsClearKeyConfirmBackupsOff":
    "The saved automatic backup key will be removed and automatic backups will turn off until a new key is saved. Existing backups made with it still need it to restore.",
  "settings.autoBackupsKeyClearedBackupsOff":
    "Backup key cleared. Automatic backups are off until a new key is saved.",
  "settings.autoBackupsKeyHelp":
    "Leave the field blank to keep the existing saved key. Enter a new value of at least {{count}} non-whitespace characters to replace it.",
  "settings.autoBackupsKeyRequired":
    "Automatic backups require a saved backup key of at least {{count}} non-whitespace characters.",
  "settings.autoBackupsKeyTooShort":
    "Automatic backup keys must be at least {{count}} non-whitespace characters.",
  "settings.autoBackupsShowKey": "Show automatic backup key",
  "settings.autoBackupsHideKey": "Hide automatic backup key",
  "settings.autoBackupsSaved": "Automatic backup settings saved.",
  "settings.backupLocationTitle": "Backup location",
  "settings.backupLocationCustomPath": "Backup folder",
  "settings.backupLocationBrowse": "Browse",
  "settings.backupLocationEffectivePath": "Backups are saved to",
  "settings.backupLocationReset": "Use default",
  "settings.backupLocationResetTitle": "Use the default backup folder?",
  "settings.backupLocationResetConfirm":
    "New backups will be saved to {{path}}. Existing backups stay where they are and are hidden until you switch back.",
  "settings.backupLocationPickerTitle": "Select backup folder",
  "settings.backupLocationSaved": "Backup location saved.",
  "settings.backupsSection":
    "Create restorable Scryer bundle backups. Manual backups run in the background so you can keep working while the bundle is prepared, even if an automatic backup is already running.",
  "settings.backupsEmpty": "No backups created yet.",
  "settings.backupsCreate": "Create backup",
  "settings.backupsCreateTitle": "Create backup bundle",
  "settings.backupsCreateDescription":
    "Protect this backup with a password. The same password is required during restore.",
  "settings.backupsConfirmPassword": "Confirm password",
  "settings.backupsPasswordMismatch": "Passwords do not match",
  "settings.backupsQueued": "Backup creation started in the background.",
  "settings.backupsDeleted": "Backup deleted.",
  "settings.backupsDeletedCount": "Deleted {{count}} backup(s).",
  "settings.backupsDeleteFailedCount": "Failed to delete {{count}} backup(s).",
  "settings.backupsCreating": "Creating",
  "settings.backupsReady": "Ready",
  "settings.backupsInvalid": "Invalid",
  "settings.backupsFailed": "Failed",
  "settings.backupsEncrypted": "Encrypted",
  "settings.backupsPlaintext": "Plaintext",
  "settings.backupsManual": "Manual",
  "settings.backupsAutomatic": "Automatic",
  "settings.backupsDownload": "Download",
  "settings.backupsDelete": "Delete",
  "settings.backupsDeleteConfirm":
    "Delete this backup bundle from the server? Any downloaded copy will be unaffected.",
  "settings.backupsDeleteSelected": "Delete selected",
  "settings.backupsDeleteSelectedConfirm":
    "Delete {{count}} selected backup bundles from the server? Any downloaded copies will be unaffected.",
  "settings.backupsSelectedCount": "{{count}} selected",
  "settings.backupsSelect": "Select {{name}}",
  "settings.backupsSelectAll": "Select all backups",
  "settings.unsavedBackupChangesTitle": "Unsaved backup settings",
  "settings.unsavedBackupChangesConfirm":
    "You have unsaved backup settings changes. Leave without saving?",
  "settings.recycleBin": "Recycle Bin",
  "settings.recycleBinEnabled": "Enable recycle bin",
  "settings.recycleBinEnabledHelp":
    "Deleted and replaced media files move into the recycle bin before permanent removal.",
  "settings.recycleBinEnabledReadonly":
    "Recycle bin status is controlled by users who can manage configuration.",
  "settings.recycleBinPath": "Recycle bin folder",
  "settings.recycleBinPathPlaceholder": ".scryer-recycle in each library root",
  "settings.recycleBinPathHelp":
    "Leave blank to use a .scryer-recycle folder inside each library root. A custom folder must be an absolute path outside every library root.",
  "settings.recycleBinPathFilesystemWarning":
    "Keep the folder on the same filesystem as your media. On a different filesystem, recycling copies each file and then deletes the original, which is slower and breaks hardlinks shared with your download client.",
  "settings.recycleBinPathExistingItems":
    "Changing the folder moves items that are already recycled into the new folder. An item that cannot be moved stays in the previous folder and is listed here after saving.",
  "settings.recycleBinRetentionDays": "Keep recycled files for (days)",
  "settings.recycleBinRetentionDaysHelp":
    "Recycled files are permanently deleted after this many days.",
  "settings.recycleBinRetentionDaysInvalid":
    "Enter a whole number of days from {{min}} to {{max}}.",
  "settings.recycleBinEffectivePaths": "Deleted files currently go to:",
  "settings.recycleBinNoEffectivePaths": "No library roots are configured yet.",
  "settings.recycleBinValidationError":
    "Automatic cleanup is paused until this is fixed: {{error}}",
  "settings.recycleBinSection": "Manage recycled media files",
  "settings.recycleBinLibraryFilter": "Libraries",
  "settings.recycleBinEmpty": "The recycle bin is empty.",
  "settings.recycleBinNoManageableLibraries":
    "No libraries are available for recycle bin item management.",
  "settings.recycleBinRestore": "Restore",
  "settings.recycleBinDelete": "Delete",
  "settings.recycleBinEmptyAll": "Empty Recycle Bin",
  "settings.recycleBinEmptyConfirm":
    "Permanently delete {{count}} items in the current recycle bin view? This cannot be undone.",
  "settings.recycleBinReason": "Reason",
  "settings.recycleBinSize": "Size",
  "settings.recycleBinRecycled": "Recycled",
  "settings.recycleBinScheduledDeletion": "Scheduled deletion",
  "settings.recycleBinFilterPlaceholder": "Filter by file or title name",
  "settings.recycleBinFilterAria":
    "Filter recycle-bin files by file or title name",
  "settings.recycleBinSelectedCount": "{{count}} selected",
  "settings.recycleBinRestoreSelected": "Restore selected",
  "settings.recycleBinDeleteSelected": "Delete selected",
  "settings.recycleBinClearSelection": "Clear selection",
  "settings.recycleBinNoFilterMatches":
    "No recycle-bin files match this filter.",
  "settings.recycleBinSelectAllFiltered":
    "Select all filtered recycle-bin files",
  "settings.recycleBinSelectTitle": "Select {{title}}",
  "settings.recycleBinSelectFile": "Select {{name}}",
  "settings.recycleBinUnassociatedFiles": "Unassociated files",
  "settings.recycleBinFileCountOne": "{{count}} file",
  "settings.recycleBinFileCountOther": "{{count}} files",
  "settings.recycleBinBatchLimit":
    "Select at most {{count}} recycle-bin files at a time.",
  "settings.recycleBinRestoreQueuedOne":
    "Queued restore for {{count}} recycle-bin file.",
  "settings.recycleBinRestoreQueuedOther":
    "Queued restore for {{count}} recycle-bin files.",
  "settings.recycleBinDeleteQueuedOne":
    "Queued deletion for {{count}} recycle-bin file.",
  "settings.recycleBinDeleteQueuedOther":
    "Queued deletion for {{count}} recycle-bin files.",
  "settings.recycleBinRestoreSelectedTitle": "Restore recycle-bin files",
  "settings.recycleBinRestoreOccupiedOne":
    "{{count}} destination is occupied. Choose whether to keep both files or move the current file to the Recycle Bin before restoring.",
  "settings.recycleBinRestoreOccupiedOther":
    "{{count}} destinations are occupied. Choose whether to keep both files or move the current file to the Recycle Bin before restoring.",
  "settings.recycleBinRestoreDescriptionOne":
    "Restore {{count}} selected file to its original path.",
  "settings.recycleBinRestoreDescriptionOther":
    "Restore {{count}} selected files to their original paths.",
  "settings.recycleBinDeleteSelectedConfirmOne":
    "Permanently delete {{count}} selected recycle-bin file. This cannot be undone.",
  "settings.recycleBinDeleteSelectedConfirmOther":
    "Permanently delete {{count}} selected recycle-bin files. This cannot be undone.",
  "settings.recycleBinKeepBothAndRestore": "Keep both and restore",
  "settings.recycleBinReplaceExistingAndRestore":
    "Replace existing and restore",
  "settings.recycleBinKeepBoth": "Keep both",
  "settings.recycleBinKeepBothDescription":
    "Restore conflicting files with a -restored suffix.",
  "settings.recycleBinReplaceExisting": "Replace existing",
  "settings.recycleBinReplaceExistingDescription":
    "Move each current file to the Recycle Bin, then restore the selected file in place.",
  "status.recycleBinEmptied": "Recycle bin emptied ({{count}} items removed).",
  "status.recycleBinSettingsSaved": "Recycle bin settings saved.",
  "settings.recycleBinRelocatedOne":
    "Moved {{count}} recycled item to the new folder.",
  "settings.recycleBinRelocatedOther":
    "Moved {{count}} recycled items to the new folder.",
  "settings.recycleBinRelocationFailedOne":
    "{{count}} recycled item could not be moved:",
  "settings.recycleBinRelocationFailedOther":
    "{{count}} recycled items could not be moved:",
  "status.recycleBinSettingsSavedWithRelocationFailures":
    "Recycle bin settings saved, but some recycled items could not be moved.",

  "appUpgrade.title": "Application upgrade",
  "appUpgrade.subtitle": "Version and upgrade management",
  "appUpgrade.upgradeNow": "Upgrade now",
  "appUpgrade.confirmTitle": "Upgrade application?",
  "appUpgrade.confirmDescription": "Upgrade to {{version}}?",
  "appUpgrade.confirmAction": "Start upgrade",
  "appUpgrade.restartNotice": "Scryer will restart during this upgrade.",
  "appUpgrade.uacNotice": "Windows will show a local elevation prompt.",
  "appUpgrade.startFailed": "Unable to start the upgrade.",
  "appUpgrade.targetChanged":
    "The available update changed. Review it and try again.",
  "appUpgrade.loading": "Loading upgrade status…",
  "appUpgrade.currentVersion": "Current version",
  "appUpgrade.availableVersion": "Available version",
  "appUpgrade.noUpdate": "No update available",
  "appUpgrade.installationKind": "Installation kind",
  "appUpgrade.managementOwner": "Upgrade manager",
  "appUpgrade.owner.inApp": "Scryer",
  "appUpgrade.owner.operator": "Operator",
  "appUpgrade.eligibility": "Upgrade eligibility",
  "appUpgrade.eligibility.eligible":
    "This installation can upgrade in the app.",
  "appUpgrade.eligibility.managed_by_docker":
    "This installation is managed by Docker.",
  "appUpgrade.eligibility.managed_by_homebrew":
    "This installation is managed by Homebrew.",
  "appUpgrade.eligibility.managed_by_winget":
    "This installation is managed by winget.",
  "appUpgrade.eligibility.windows_supervised":
    "This Windows installation is supervised externally.",
  "appUpgrade.eligibility.disabled_by_operator":
    "In-app upgrades were disabled by the operator.",
  "appUpgrade.eligibility.unsupported_layout":
    "This installation layout is not supported.",
  "appUpgrade.eligibility.install_dir_not_writable":
    "The installation directory is not writable.",
  "appUpgrade.eligibility.app_bundle_translocated":
    "macOS is running Scryer from a temporary copy. Move Scryer to your Applications folder and open it from there.",
  "appUpgrade.eligibility.app_bundle_read_only_volume":
    "Scryer is running from a read-only volume. Drag Scryer to your Applications folder and open it from there.",
  "appUpgrade.eligibility.app_bundle_not_tray_supervised":
    "This app was not started by the Scryer app, so it cannot be restarted after an upgrade.",
  "appUpgrade.eligibility.unknown": "Upgrade eligibility is unavailable.",
  "appUpgrade.rebootRequired": "Restart Windows to finish this upgrade.",
  "appUpgrade.activeRun": "Upgrade in progress",
  "appUpgrade.latestRun": "Latest upgrade",
  "appUpgrade.runStatusLabel": "Status: {{status}}",
  "appUpgrade.runStatus.queued": "Queued",
  "appUpgrade.runStatus.discovering": "Preparing",
  "appUpgrade.runStatus.running": "Running",
  "appUpgrade.runStatus.completed": "Completed",
  "appUpgrade.runStatus.warning": "Completed with warnings",
  "appUpgrade.runStatus.failed": "Failed",
  "appUpgrade.phase.checking": "Checking",
  "appUpgrade.phase.downloading": "Downloading",
  "appUpgrade.phase.verifying": "Verifying",
  "appUpgrade.phase.staging": "Staging",
  "appUpgrade.phase.applying": "Applying",
  "appUpgrade.phase.awaiting_elevation": "Waiting for elevation",
  "appUpgrade.phase.restarting": "Restarting",
  "appUpgrade.phase.reboot_required": "Restart required",
  "appUpgrade.phase.unknown": "Working",
  "appUpgrade.downloadProgress": "Downloaded {{downloaded}} of {{total}}",
  "appUpgrade.downloadedBytes": "Downloaded {{downloaded}}",
  "appUpgrade.operatorGuidance": "Operator-managed upgrade",
  "appUpgrade.guidance.winget": "Run this command in an elevated terminal.",
  "appUpgrade.guidance.wingetCommand": "winget upgrade ScryerMedia.Scryer",
  "appUpgrade.guidance.docker":
    "Pull the new image and recreate the container using your deployment configuration.",
  "appUpgrade.guidance.homebrew": "Run this command in a terminal.",
  "appUpgrade.guidance.homebrewCommand": "brew upgrade scryer",
  "appUpgrade.guidance.disabled":
    "In-app upgrades are disabled by an environment setting.",
  "appUpgrade.guidance.disabledVariable": "SCRYER_DISABLE_SELF_UPGRADE",
  "appUpgrade.guidance.other":
    "Use the upgrade method provided by this installation's operator.",
  "appUpgrade.installationKind.portable": "Portable",
  "appUpgrade.installationKind.directMsi": "Direct MSI",
  "appUpgrade.installationKind.macosAppBundle": "macOS app",
  "appUpgrade.installationKind.docker": "Docker",
  "appUpgrade.installationKind.homebrew": "Homebrew",
  "appUpgrade.installationKind.winget": "winget",
  "appUpgrade.installationKind.windowsSupervised": "Windows supervised",
  "appUpgrade.installationKind.disabled": "Disabled",
  "appUpgrade.installationKind.unsupported": "Unsupported",
  "appUpgrade.success": "Upgraded to {{version}}",
  "appUpgrade.failed": "Upgrade failed.",
  // ── Maintenance rules ──
  "settings.maintenanceRules": "Maintenance Rules",
  "nav.rules": "Rules",
  "settings.rulesScoring": "Scoring Rules",
  "settings.maintenanceNavRules": "Rules",
  "settings.maintenanceNavHistory": "History",
  "settings.maintenanceRuleAction": "Action",
  "settings.maintenanceRuleActionHelp":
    "What this rule would do once maintenance actions are switched on.",
  "settings.maintenanceRuleActionPlaceholder": "Choose an action",
  "settings.maintenanceRuleNoActions": "No title-scoped actions are available.",
  "settings.maintenanceActionSequence": "Action sequence",
  "settings.maintenanceSequenceTitle": "Action sequence",
  "settings.maintenanceSequenceHelp":
    "Run these steps in order after a subject has completed its grace period. An empty sequence records matches without acting on them.",
  "settings.maintenanceSequenceEmpty":
    "Dry run only — no action steps are configured.",
  "settings.maintenanceSequenceObserveOnly": "Dry run only",
  "settings.maintenanceSequenceSummary": "Sequence summary",
  "settings.maintenanceSequenceSummaryTarget": "Targets: {{target}}",
  "settings.maintenanceSequenceSummaryArming": "When enabled: {{arming}}",
  "settings.maintenanceSequenceSummaryEffects": "Effects: {{effects}}",
  "settings.maintenanceSequenceTargetTitle": "matching title",
  "settings.maintenanceSequenceTargetTitleDescendants":
    "matching title, seasons, and episodes",
  "settings.maintenanceSequenceTargetSeason": "matching season",
  "settings.maintenanceSequenceTargetSeasonDescendants":
    "matching season and episodes",
  "settings.maintenanceSequenceTargetEpisode": "matching episode",
  "settings.maintenanceSequencePreviewPlan": "Ordered effects for this preview",
  "settings.maintenanceSequencePreviewDescendants": "include descendants",
  "settings.maintenanceSequencePreviewMatchedScope": "matched scope only",
  "settings.maintenanceSequenceAdd": "Add step",
  "settings.maintenanceSequencePreset": "Sequence preset",
  "settings.maintenanceSequencePresetApply": "Replace with preset",
  "settings.maintenanceSequencePresetProfileThenSearch":
    "Change profile, then search if it changed",
  "settings.maintenanceSequencePresetUnmonitorThenDelete":
    "Unmonitor descendants, then delete files",
  "settings.maintenanceSequenceTerminal": "Terminal",
  "settings.maintenanceSequenceDependencies": "Requires: {{steps}}",
  "settings.maintenanceSequenceIncludeDescendants":
    "Include seasons and episodes",
  "settings.maintenanceSequenceSearchCondition": "Search condition",
  "settings.maintenanceSequenceSearchUnconditional": "Always request search",
  "settings.maintenanceSequenceSearchPreviousProfileChanged":
    "Only after the profile changed",
  "settings.maintenanceSequenceSearchRequested":
    "This step requests a search and records accepted only after the request is accepted. Later acquisition and download effects can still be held.",
  "settings.maintenanceSequenceHistoryWaiting": "waiting for an earlier step",
  "settings.maintenanceSequenceHistorySearchRequested": "search requested",
  "settings.maintenanceSequenceHistorySearchAccepted": "accepted",
  "settings.maintenanceSequenceHistoryJob": "job {{id}}",
  "settings.maintenanceSequenceCompletionAccepted": "Completes when accepted",
  "settings.maintenanceSequenceCompletionCompleted": "Completes when finished",
  "settings.maintenanceSequenceStepUnavailable":
    "This saved step is unavailable from the current catalog. Remove it or restore a compatible catalog before saving or previewing.",
  "settings.maintenanceSequenceCatalogUnavailable":
    "The action-step catalog is unavailable. Saved steps remain visible, but saving and unsaved previews are paused until it loads.",
  "settings.maintenanceSequenceStepUnmonitor": "Unmonitor",
  "settings.maintenanceSequenceStepDeleteFiles": "Delete files",
  "settings.maintenanceSequenceStepChangeQualityProfile":
    "Change quality profile",
  "settings.maintenanceSequenceStepSearch": "Search",
  "settings.maintenanceSequenceStepAddTags": "Add tags",
  "settings.maintenanceSequenceStepRemoveTags": "Remove tags",
  "settings.maintenanceSequenceStepDeleteTitleAndFiles":
    "Delete title and files",
  "settings.maintenanceRuleGraceDays": "Grace days",
  "settings.maintenanceRuleGraceDaysHelp":
    "Days after each subject first matches before its action becomes due.",
  "settings.maintenanceRuleTargetProfile": "Target quality profile",
  "settings.maintenanceRuleTargetProfileHelp":
    "The profile this action would move matching titles to.",
  "settings.maintenanceRuleTargetProfileIdHelp": "Quality profile ID.",
  "settings.maintenanceRuleLibraries": "Library scope",
  "settings.maintenanceRuleLibrariesHelp":
    "Leave every library unchecked to scope the rule to the whole collection.",
  "settings.maintenanceRuleLibrariesAll": "All libraries",
  "settings.maintenanceRuleStorageRoot": "Storage root",
  "settings.maintenanceRuleStorageRootNone": "No storage root selected",
  "settings.maintenanceRuleStorageRootHelp":
    "Select a configured root to read its capacity and limit file deletion to files actually on that root.",
  "settings.maintenanceRuleStorageRootUnmonitorHelp":
    "The selected root limits only file deletion. Unmonitoring still applies to the entire matched subject, so files on other roots are retained but no longer monitored.",
  "settings.maintenanceRuleStorageRootClear": "Clear storage root",
  "settings.maintenanceStorageRootFileSummary":
    "Selected root: {{count}} files · {{size}}",
  "settings.maintenanceRuleCreate": "Create maintenance rule",
  "settings.maintenanceRuleUpdate": "Update maintenance rule",
  "settings.maintenanceRuleCopy": "Copy as a new rule",
  "settings.maintenanceRuleCreateNew": "Create new maintenance rule",
  "settings.maintenanceRuleSaveDisabledNote":
    "New rules start Disabled. Review the preview, try a Dry run, then set the rule to Enabled.",
  "settings.maintenanceRuleValidationRequired":
    "Name, matcher source, and an action are required.",
  "settings.maintenanceRuleConfirmDiscardTitle": "Discard unsaved changes?",
  "settings.maintenanceRuleConfirmDiscardDescription":
    "The maintenance rule editor has unsaved changes that will be lost.",
  "settings.maintenanceRuleDeleting": "Delete the maintenance rule {{name}}?",
  "settings.noMaintenanceRulesFound": "No maintenance rules yet.",
  "settings.maintenanceRuleRevision": "Revision",
  "settings.maintenanceActionDoNothing": "Do nothing",
  "settings.maintenanceActionUnmonitorScopeKeepFiles":
    "Unmonitor the scope, keep files",
  "settings.maintenanceActionDeleteTitleAndFiles":
    "Delete the title and its files",
  "settings.maintenanceActionUnmonitorTitleDeleteAllFiles":
    "Unmonitor the title, delete all files",
  "settings.maintenanceActionUnmonitorShowDeleteExistingFiles":
    "Unmonitor the show, delete existing files",
  "settings.maintenanceActionUnmonitorScopeDeleteFiles":
    "Unmonitor the scope, delete its files",
  "settings.maintenanceActionUnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty":
    "Unmonitor the season, delete its files, then delete the show if it is empty",
  "settings.maintenanceActionUnmonitorSeasonThenUnmonitorShowIfEmpty":
    "Unmonitor the season, then unmonitor the show if it is empty",
  "settings.maintenanceActionChangeQualityProfileAndSearchIfChanged":
    "Change the quality profile and search if it changed",
  "settings.maintenancePreviewTitle": "Preview",
  "settings.maintenancePreviewSubtitle":
    "Run the matcher against titles, seasons, or episodes in its scope. Preview reports what would match and changes nothing.",
  "settings.maintenancePreviewSource": "Matcher",
  "settings.maintenancePreviewSourceStored": "Saved rule",
  "settings.maintenancePreviewSourceDraft": "Editor draft",
  "settings.maintenancePreviewLibrary": "Library",
  "settings.maintenancePreviewLibraryPlaceholder": "Select a library",
  "settings.maintenancePreviewLimit": "Subjects",
  "settings.maintenancePreviewLimitHelp": "Up to {{max}} subjects per run.",
  "settings.maintenancePreviewRun": "Run preview",
  "settings.maintenancePreviewRunning": "Running preview...",
  "settings.maintenancePreviewEmpty": "Choose a library and run the preview.",
  "settings.maintenancePreviewNoTitles": "No titles were evaluated.",
  "settings.maintenancePreviewColOutcome": "Outcome",
  "settings.maintenancePreviewColReasons": "Reason codes",
  "settings.maintenancePreviewOutcomeMatch": "MATCH",
  "settings.maintenancePreviewOutcomeNoMatch": "NO MATCH",
  "settings.maintenancePreviewOutcomeUnknown": "UNKNOWN",
  "settings.maintenancePreviewOutcomeError": "ERROR",
  "settings.maintenancePreviewMeta": "Evaluated {{time}} · matcher {{hash}}",
  "settings.maintenancePreviewRuleSelect": "Rule",
  "settings.maintenancePreviewRuleSelectPlaceholder": "Select a saved rule",
  "settings.maintenanceRuleUpdated": "Maintenance rule updated.",
  "settings.maintenanceRuleCreated":
    "Maintenance rule created, and saved disabled.",
  "settings.maintenanceRuleDeleted": "Maintenance rule {{name}} deleted.",

  // ── Maintenance rules — instance status ──

  // ── Maintenance rules — instance gates ──

  // ── Maintenance rules — mode and arming ──
  "settings.maintenanceArmingNone": "No effects",
  "settings.maintenanceArmingReversible": "Library updates",
  "settings.maintenanceArmingDestructive": "File deletion",
  "settings.maintenanceRuleState": "State",
  "settings.maintenanceRuleStateHelp":
    "Disabled rules are never evaluated. Dry run records candidates without acting on them. Enabled rules act on their candidates once each grace period elapses.",
  "settings.maintenanceRuleStateUpdated": "{{name}} updated.",
  "settings.maintenanceRuleStateDisabled": "Disabled",
  "settings.maintenanceRuleStateDisabledHelp": "Never evaluated.",
  "settings.maintenanceRuleStateDryRun": "Dry run",
  "settings.maintenanceRuleStateDryRunHelp":
    "Records candidates; takes no action.",
  "settings.maintenanceRuleStateEnabled": "Enabled",
  "settings.maintenanceRuleStateEnabledHelp":
    "Acts on candidates when their grace period elapses.",
  "settings.maintenanceRuleEditedSinceEnabled":
    "Edited since it was enabled. Review the candidates and enable it again.",
  "settings.maintenanceRuleSystemOwned": "Built in",
  "settings.expiredRequestLeasesHelp":
    "Deletes a requested title and its files once its request lease has expired and the grace period has passed. Changing the grace period sets the rule to Dry run until you enable it again. Like every maintenance rule, it only runs while Maintenance is on.",
  "settings.maintenanceSwitchEnabled": "Maintenance",
  "settings.maintenanceSwitchEnabledHelp":
    "Evaluate rules and show their candidates on this instance.",
  "settings.maintenanceSwitchActionsPaused": "Pause all actions",
  "settings.maintenanceSwitchActionsPausedHelp":
    "While paused, Enabled rules keep recording candidates but nothing is unmonitored, changed, or deleted.",
  "settings.maintenanceSwitchesLocked":
    "Reading the maintenance switches requires system settings permission. Ask an administrator whether maintenance is on and whether actions are paused.",
  "settings.maintenanceSwitchesUpdated": "Maintenance settings updated.",
  "settings.maintenanceUnpauseConfirmTitle": "Resume maintenance actions?",
  "settings.maintenanceUnpauseConfirmBody":
    "Enabled rules will act on their candidates as each grace period elapses, including rules that delete files.",
  "settings.maintenanceUnpauseConfirmAction": "Resume actions",
  "settings.maintenanceCandidatesEmptyMaintenanceOff":
    "Maintenance is not switched on for this instance, so no candidates are shown.",
  "settings.maintenanceCandidatesEmptyAllDisabled":
    "Every rule is Disabled, so nothing is being evaluated. Set a rule to Dry run first.",
  "settings.maintenanceArmDestructiveIntro":
    "{{name}} deletes files. Once enabled it acts on the candidates below as each grace period elapses, unless actions are paused.",
  "settings.maintenanceArmDestructiveTitle":
    "Enable a rule that deletes files?",
  "settings.maintenanceArmDestructiveLoading":
    "Counting the candidates this rule already holds…",
  "settings.maintenanceArmDestructiveCount":
    "{{count}} candidates could be acted on.",
  "settings.maintenanceArmDestructiveNoCandidates":
    "This rule holds no candidates that could be acted on right now.",
  "settings.maintenanceArmDestructiveCountChanged":
    "The candidate count changed: this rule now holds {{count}}. Confirm the current count to continue.",
  "settings.maintenanceArmDestructiveAcknowledge":
    "Enable file deletion for {{count}} titles.",
  "settings.maintenanceArmDestructiveConfirm": "Enable",

  // ── Maintenance rules — candidates ──
  "settings.maintenanceCandidatesTitle": "Leaving soon",
  "settings.maintenanceCandidatesSubtitle":
    "Titles a rule has matched, soonest due first. Nothing here has been acted on yet.",
  "settings.maintenanceCandidatesFilterRule": "Rule",
  "settings.maintenanceCandidatesFilterState": "State",
  "settings.maintenanceCandidatesFilterLibrary": "Library",
  "settings.maintenanceCandidatesAllRules": "All rules",
  "settings.maintenanceCandidatesAllStates": "All states",
  "settings.maintenanceCandidatesAllLibraries": "All libraries",
  "settings.maintenanceCandidatesColRule": "Rule",
  "settings.maintenanceCandidatesColState": "State",
  "settings.maintenanceCandidatesColFirstMatched": "First matched",
  "settings.maintenanceCandidatesColDue": "Due",
  "settings.maintenanceCandidatesHeldSince": "Held since {{time}}",
  "settings.maintenanceCandidatesEmpty": "No candidates match these filters.",
  "settings.maintenanceCandidateExclude": "Exclude this title",
  "settings.maintenanceCandidateStateObserving": "Observing",
  "settings.maintenanceCandidateStatePendingAction": "Pending action",
  "settings.maintenanceCandidateStateDue": "Due",
  "settings.maintenanceCandidateStateExecuting": "Executing",
  "settings.maintenanceCandidateStateSucceeded": "Succeeded",
  "settings.maintenanceCandidateStateFailed": "Failed",
  "settings.maintenanceCandidateStateCanceled": "Canceled",
  "settings.maintenanceCandidateStateExcluded": "Excluded",
  "settings.maintenanceCandidateStateBlocked": "Blocked",
  "settings.maintenanceCountdownDueNow": "Due now",
  "settings.maintenanceCountdownInMinutes": "in {{count}} min",
  "settings.maintenanceCountdownInHours": "in {{count}} h",
  "settings.maintenanceCountdownInDays": "in {{count}} d",
  "settings.maintenanceCountdownOverdueMinutes": "{{count}} min overdue",
  "settings.maintenanceCountdownOverdueHours": "{{count}} h overdue",
  "settings.maintenanceCountdownOverdueDays": "{{count}} d overdue",

  // ── Maintenance rules — runs and history ──
  "settings.maintenanceRunsTitle": "Runs and history",
  "settings.maintenanceRunsSubtitle":
    "Recent evaluation passes and action attempts, newest first.",
  "settings.maintenanceRunScope": "Scope",
  "settings.maintenanceRunScopeAll": "Every rule",
  "settings.maintenanceRunEvaluationNow": "Evaluate now",
  "settings.maintenanceRunEvaluationRunning": "Evaluating…",
  "settings.maintenanceRunActionsNow": "Run actions now",
  "settings.maintenanceRunActionsRunning": "Running actions…",
  "settings.maintenanceRunStarted": "Run started.",
  "settings.maintenanceRunNotStarted": "Nothing started: {{message}}",
  "settings.maintenanceEvaluationRunsTitle": "Evaluation runs",
  "settings.maintenanceEvaluationRunsEmpty": "No evaluation runs recorded yet.",
  "settings.maintenanceActionRunsTitle": "Action runs",
  "settings.maintenanceActionRunsEmpty": "No action runs recorded yet.",
  "settings.maintenanceRunColStarted": "Started",
  "settings.maintenanceRunColStatus": "Status",
  "settings.maintenanceRunColCounts": "Counts",
  "settings.maintenanceRunColDuration": "Duration",
  "settings.maintenanceRunColError": "Error",
  "settings.maintenanceRunCounts":
    "{{evaluated}} evaluated · {{matched}} matched · {{noMatch}} no match · {{unknown}} unknown · {{errors}} errors",
  "settings.maintenanceRunDuration": "{{seconds}} s",
  "settings.maintenanceRunRevision": "rev {{revision}}",
  "settings.maintenanceRunStatusRunning": "Running",
  "settings.maintenanceRunStatusSucceeded": "Succeeded",
  "settings.maintenanceRunStatusFailed": "Failed",
  "settings.maintenanceRunStatusHeld": "Held",
  "settings.maintenanceRunStatusAlreadySatisfied": "Already satisfied",
  "settings.maintenanceRunStatusSkipped": "Skipped",
  "settings.maintenanceActionRunColAttempt": "Attempt",
  "settings.maintenanceActionRunColHoldReason": "Hold reason",

  // ── Maintenance rules — exclusions ──
  "settings.maintenanceExclusionsTitle": "Exclusions",
  "settings.maintenanceExclusionsSubtitle":
    "Titles a maintenance rule must never act on. A global exclusion applies to every rule.",
  "settings.maintenanceExclusionsEmpty": "No exclusions.",
  "settings.maintenanceExclusionColScope": "Scope",
  "settings.maintenanceExclusionColReason": "Reason",
  "settings.maintenanceExclusionColCreated": "Added",
  "settings.maintenanceExclusionScopeGlobal": "Every rule",
  "settings.maintenanceExclusionRemove": "Remove exclusion",
  "settings.maintenanceExclusionRemoved": "Exclusion for {{title}} removed.",
  "settings.maintenanceExclusionRemoving":
    "Remove the exclusion for {{title}}?",
  "settings.maintenanceExcludeTitle": "Exclude from maintenance",
  "settings.maintenanceExcludeBody":
    "{{title}} will be excluded from the next evaluation onward, and its candidate moves to excluded.",
  "settings.maintenanceExcludeScope": "Scope",
  "settings.maintenanceExcludeScopeGlobal": "Every rule",
  "settings.maintenanceExcludeReason": "Reason",
  "settings.maintenanceExcludeReasonPlaceholder":
    "Why this title is excluded (optional)",
  "settings.maintenanceExcludeConfirm": "Exclude",
  "settings.maintenanceExcluded": "{{title}} excluded from maintenance.",

  // ── Maintenance rules — input reference ──
  "settings.refMaintTitle": "Maintenance input reference",
  "settings.refMaintSubtitle":
    "Click to expand every fact a maintenance matcher can read.",
  "settings.refMaintIntro":
    "Every maintenance rule receives one input object per subject: input.subject identifies what is being evaluated, input.library says where it lives, input.facts carries the fact snapshot, and the evaluation envelope sits on input itself. A fact is a plain value, and the key is simply missing when there is nothing to report — so write input.facts.monitored, not a status check. If a rule reads a fact Scryer could not observe for that subject, Scryer holds the subject for you; you never have to write that guard yourself.",
  "settings.refMaintOutputTitle": "Rule output",
  "settings.refMaintOutputIntro":
    "A matcher defines up to three names. Anything else in the package is ignored, and a decision that is not exactly this shape is rejected rather than coerced.",
  "settings.refMaintOutputMatch":
    "match — boolean. Leaving it undefined means the subject did not match.",
  "settings.refMaintOutputUnknown":
    "unknown — boolean. Set it when the rule itself cannot see enough to decide; unknown always beats a match. Facts Scryer could not observe are already handled for you, so this is only for judgements of your own.",
  "settings.refMaintOutputReasons":
    "reasons — a set of short machine codes explaining the decision. They are stored on the candidate and shown in preview.",
  "settings.refMaintOutputNoPackage":
    "Do not write a package or import line. Scryer adds both when it stores the rule and strips them back off for the editor.",
  "settings.refMaintSectionInput": "— The evaluation envelope",
  "settings.refMaintInputSchemaVersion":
    "Version of the maintenance input document this rule is authored against",
  "settings.refMaintInputEvaluationTime":
    "RFC3339 instant captured once per run, so every subject compares against the same clock. Use it for date maths — time.now_ns() exists, but it would make the same subject decide differently on a retry.",
  "settings.refMaintSectionSubject":
    "— The title, season, or episode under evaluation",
  "settings.maintenanceFileSummary": "{{count}} files · {{size}}",
  "settings.maintenanceTestTitle": "Choose a title to test",
  "settings.maintenanceNoSubjects":
    "No subjects in this title for the selected scope.",
  "settings.maintenancePreviewExcluded": "Excluded from actions",
  "settings.maintenancePreviewEstimatedDue": "If matched now: {{time}}",
  "settings.maintenancePreviewDue": "Grace deadline: {{time}}",
  "settings.maintenancePreviewSubject": "Test subject",
  "settings.maintenancePreviewSample": "Library sample",
  "settings.maintenanceRuleScope": "Rule scope",
  "settings.maintenanceRuleTemplate": "Template",
  "settings.maintenanceRuleTemplatePlaceholder": "Start from a template",
  "settings.maintenanceScopeSeason": "Season",
  "settings.maintenanceScopeEpisode": "Episode",
  "settings.maintenanceFileDeleted": "Deleted",
  "settings.maintenanceFilePending": "Pending",
  "settings.maintenanceFileResults": "File results",
  "settings.maintenanceFileProgress":
    "{{completed}} deleted · {{remaining}} remaining",
  "settings.refMaintSubjectId":
    "Scryer title, collection, or episode ID for the selected scope",
  "settings.refMaintCollectionId":
    "Owning season collection ID for season and episode subjects, when known",
  "settings.refMaintSubjectKind":
    "Granularity under evaluation: title, season, or episode",
  "settings.refMaintSubjectTitleId":
    "Scryer identifier of the title the subject belongs to",
  "settings.refMaintSubjectSeasonNumber":
    "Season number for season and episode subjects, when known",
  "settings.refMaintSubjectEpisodeId":
    "Episode identifier; present only for episode subjects",
  "settings.refMaintSubjectFacet":
    "Media facet of the title (movie, series, anime)",
  "settings.refMaintSubjectName":
    "Display label of the title, season, or episode",
  "settings.refMaintSubjectYear": "Release year of the title, when known",
  "settings.refMaintSectionLibrary": "— The library the subject lives in",
  "settings.refMaintLibraryId": "Identifier of the library holding the subject",
  "settings.refMaintLibraryName": "Display name of that library",
  "settings.refMaintSectionFacts":
    "— The fact snapshot; each fact is a plain value, and the key is missing when the fact is absent or could not be observed",
  "settings.refMaintFactsMonitored": "Whether the subject is monitored",
  "settings.refMaintFactsTags":
    "User-defined tags inherited from the owning title, lowercase, from the tag registry an administrator maintains in Settings; reserved scryer: settings entries are excluded",
  "settings.refMaintFactsQualityProfileId":
    "Quality profile inherited from the owning title",
  "settings.refMaintFactsAddedAt": "When the subject was added to the library",
  "settings.refMaintFactsAddedByUserId":
    "Which user added the subject; absent when a scan or discovery created it",
  "settings.refMaintFactsAddedByUsername":
    "Name of the user who added the subject",
  "settings.refMaintFactsRequested":
    "Whether a media request created the subject",
  "settings.refMaintFactsRequestedByUserIds":
    "Users who requested the subject: the submitter plus any additional requesters",
  "settings.refMaintFactsRequestedByUsernames": "Names of those requesters",
  "settings.refMaintFactsFirstImportedAt":
    "When a file was first imported for the subject",
  "settings.refMaintFactsLastUpgradedAt":
    "When the subject last received an upgrade",
  "settings.refMaintFactsHasFile":
    "Whether the subject has at least one file on disk",
  "settings.refMaintFactsFileCount": "How many files the subject has on disk",
  "settings.refMaintFactsTotalFileSizeBytes":
    "Combined on-disk size of the subject's files",
  "settings.refMaintFactsFiles":
    "The subject's files, with per-file media detail",
  "settings.refMaintFactsEpisodeCount": "How many episodes the subject covers",
  "settings.refMaintFactsEpisodeFileCount":
    "How many of those episodes have a file",
  "settings.refMaintFactsMonitoredEpisodeCount":
    "How many of those episodes are monitored",
  "settings.refMaintFactsEpisodePositionByAirDate":
    "Episode rank by air date within its show",
  "settings.refMaintFactsSeasonPositionByAirDate":
    "Season rank by newest eligible downloaded episode",
  "settings.refMaintFactsStorageRootId":
    "Configured root selected for storage capacity and file deletion",
  "settings.refMaintFactsStorageAvailableBytes":
    "Bytes currently available on the selected root",
  "settings.refMaintFactsStorageTotalBytes":
    "Total capacity in bytes of the selected root",
  "settings.refMaintFactsStorageAvailablePercent":
    "Percentage of the selected root currently available",
  "settings.refMaintFactsActiveDownloads":
    "Whether the subject has a download in flight",
  "settings.refMaintFactsWatchedByUserIds":
    "Users who watched this episode; for seasons, the same users must have watched every aired episode, including missing files. Future episodes are excluded; unresolved air dates or incomplete signals are unknown. Empty seasons are never completed.",
  "settings.refMaintFactsLastWatchedAt":
    "When the subject was last played by anyone",
  "settings.refMaintFactsWatchedByAnyRequester":
    "Whether at least one of the subject's requesters has played it",
  "settings.refMaintFactsWatchedByAllRequesters":
    "Whether every one of the subject's requesters has played it",
  "settings.refMaintFactsRequestLeaseState":
    "Aggregate state of the subject's request leases: none, dormant, active, or expired",
  "settings.refMaintFactsRequestLeaseExpiresAt":
    "When the last live lease on the subject runs out",
  "settings.refMaintFactsActiveRetentionClaims":
    "How many request leases still hold the subject",
  "settings.refMaintFactsListsAddedByList":
    "Whether a list, rather than a person, added the subject to the library",
  "settings.refMaintFactsListsOnEnabledList":
    "Whether any enabled list, public or personal, still lists the subject",
  "settings.refMaintFactsListsNames":
    "Names of the enabled public lists that still list the subject",
  "settings.refMaintFactsListsLeftAll":
    "Whether every list that ever listed the subject has dropped it",
  "settings.refMaintFactsListsLeftAt": "When the last list dropped the subject",
  "settings.refMaintFactsListsLastListName":
    "The public list the subject most recently left",
  "settings.refMaintFactsListsEnabledIds":
    "Identifiers of the enabled lists, public or personal, that still list the subject",
  "settings.refMaintFactsListsPublicCount":
    "How many public lists still list the subject",
  "settings.refMaintFactsListsPersonalCount":
    "How many personal lists still list the subject",
  "settings.refMaintFactsListsFirstSeenAt":
    "When a list that still holds the subject first listed it",
  "settings.refMaintFactsListsLastSeenAt":
    "When a list that still holds the subject last listed it",
  "settings.refMaintFactsListsDaysOnList":
    "Whole days since a list that still holds the subject first listed it",
  "settings.refMaintFactsListsBestRank":
    "The best (lowest) rank any list that still holds the subject gives it",
  "settings.refMaintFactsListsOnLeave":
    "What the enabled lists that hold the subject do when they drop it",
  "settings.refMaintFactsListsLeftHandled":
    "Whether every list departure has had its on-leave action applied",
  "settings.refMaintFactsListsMembershipStates":
    "States of the subject on the lists that still hold it",
  "settings.refMaintFactsRequestOrigin":
    "Where the request that created the title came from",
  "settings.refMaintFactsRequestListName":
    "The public list that requested the title",
  "settings.refMaintObsStatus":
    "known, absent, or unknown. absent means the source answered and there is no value; unknown means Scryer could not find out. Test this before reading value — an unknown fact is never coerced to false, 0, or an empty string.",
  "settings.refMaintSectionObservations":
    "— The same facts as full envelopes, for rules that need to tell absent apart from unknown or to read the reason. Reading a fact here opts that fact out of the automatic hold: you are taking responsibility for the three-valued answer.",
  "settings.refMaintSectionObsFiles":
    "— One entry per file in input.observations.files.value",
  "settings.refMaintObsObservedAt":
    "When the value was observed, if the source recorded a time",
  "settings.refMaintObsReason":
    "Stable machine code explaining why the fact is unknown, or why an absent fact has no value",
  "settings.refMaintFactsMonitoredValue": "True when the subject is monitored",
  "settings.refMaintFactsTagsValue":
    "Lowercase user-defined tag names from the admin tag registry; never reserved scryer: settings entries",
  "settings.refMaintFactsQualityProfileIdValue":
    "Identifier of the assigned quality profile",
  "settings.refMaintFactsAddedAtValue":
    "RFC3339 timestamp the subject was added",
  "settings.refMaintFactsAddedByUserIdValue":
    "Identifier of the user who added the subject; the reason is title_added_by_system when nobody did",
  "settings.refMaintFactsAddedByUsernameValue":
    "Username of that user; unknown with reason user_not_found when the account is gone",
  "settings.refMaintFactsRequestedValue":
    "True when at least one media request created the subject; a confirmed false otherwise",
  "settings.refMaintFactsRequestedByUserIdsValue":
    "Requester identifiers, deduplicated; an empty list when the subject was not requested",
  "settings.refMaintFactsRequestedByUsernamesValue":
    "Requester usernames; unknown with reason user_not_found when any of those accounts is gone",
  "settings.refMaintFactsFirstImportedAtValue":
    "RFC3339 timestamp of the first import",
  "settings.refMaintFactsLastUpgradedAtValue":
    "RFC3339 timestamp of the last upgrade",
  "settings.refMaintFactsHasFileValue":
    "True when at least one file is on disk",
  "settings.refMaintFactsFileCountValue": "Number of files on disk",
  "settings.refMaintFactsTotalFileSizeBytesValue":
    "Combined size of those files in bytes",
  "settings.refMaintFactsFilesValue":
    "One entry per file; see input.observations.files.value[] below",
  "settings.refMaintFactsEpisodeCountValue": "Number of episodes in scope",
  "settings.refMaintFactsEpisodeFileCountValue":
    "Number of those episodes that have a file",
  "settings.refMaintFactsMonitoredEpisodeCountValue":
    "Number of those episodes that are monitored",
  "settings.refMaintFactsEpisodePositionByAirDateValue":
    "Rank 1 is the newest eligible downloaded, aired non-special episode. Equal air dates use season and episode numbers, then stable IDs.",
  "settings.refMaintFactsSeasonPositionByAirDateValue":
    "Rank 1 is the season with the newest eligible downloaded, aired non-special episode. Protected files still occupy a rank.",
  "settings.refMaintFactsStorageRootIdValue":
    "Durable configured library-root identifier selected on this revision",
  "settings.refMaintFactsStorageAvailableBytesValue":
    "Available capacity in bytes, observed from the selected root itself",
  "settings.refMaintFactsStorageTotalBytesValue":
    "Total capacity in bytes, observed from the selected root itself",
  "settings.refMaintFactsStorageAvailablePercentValue":
    "Available capacity divided by total capacity, from 0 to 100",
  "settings.refMaintFactsActiveDownloadsValue":
    "True when a download for the subject is in flight",
  "settings.refMaintFactsWatchedByUserIdsValue":
    "Identifiers of everyone who played it, sorted and deduplicated; an empty list means nobody has. Season completion requires the same person to have watched every aired episode, including missing files. Future episodes are excluded; missing episode history or unresolved air dates are unknown. Watch facts are unknown for title-scoped shows (show_watch_rollup_unavailable), when no media server is connected (no_media_server_connection), when a connection has never synced cleanly (signal_sync_never_succeeded), when the last clean sync is over 48 hours old (signals_stale), when nobody holds a verified linked media-server account (no_linked_participants), and when a play cannot be attributed to a Scryer user (signal_identity_missing).",
  "settings.refMaintFactsLastWatchedAtValue":
    "RFC3339 timestamp of the most recent play; absent with reason never_watched when nobody has played it",
  "settings.refMaintFactsWatchedByAnyRequesterValue":
    "True when at least one requester played it; absent with reason title_not_requested when nobody requested the subject, and unknown with reason requester_not_linked when a requester has no linked media-server account",
  "settings.refMaintFactsWatchedByAllRequestersValue":
    "True when every requester played it; absent with reason title_not_requested when nobody requested the subject, and unknown with reason requester_not_linked when a requester has no linked media-server account",
  "settings.refMaintFactsRequestLeaseStateValue":
    "none when no request lease was ever written for the subject, dormant when a lease is waiting for the first import, active while a lease is running, and expired once every lease has run out. A lease that was released or converted is withdrawn rather than spent, so it does not make the state expired. Expiry is measured against the evaluation time, so a lease reads as expired the moment its window closes.",
  "settings.refMaintFactsRequestLeaseExpiresAtValue":
    "RFC3339 timestamp of the latest expiry among the live leases; absent with reason no_live_lease when nothing is holding the subject",
  "settings.refMaintFactsActiveRetentionClaimsValue":
    "Count of leases still holding the subject, dormant ones included",
  "settings.refMaintFactsListsAddedByListValue":
    "True when a list sync created the title; unknown with reason list_store_unreadable when the list store could not be read",
  "settings.refMaintFactsListsOnEnabledListValue":
    "True while at least one enabled list holds the subject",
  "settings.refMaintFactsListsNamesValue":
    "Public list names only, sorted; a member's personal list is never named",
  "settings.refMaintFactsListsLeftAllValue":
    "True once every list that listed the subject has dropped it; false while one still holds it or when no list ever did",
  "settings.refMaintFactsListsLeftAtValue":
    "RFC3339 timestamp; absent with reason not_left_every_list while any list still holds the subject",
  "settings.refMaintFactsListsLastListNameValue":
    "Name of that public list; absent with reason no_public_list_left when the subject never left a public list",
  "settings.refMaintFactsListsEnabledIdsValue":
    "List identifiers, sorted; never a list name",
  "settings.refMaintFactsListsPublicCountValue":
    "Count of public lists, enabled or paused",
  "settings.refMaintFactsListsPersonalCountValue":
    "Count of personal lists, enabled or paused",
  "settings.refMaintFactsListsFirstSeenAtValue":
    "RFC3339 timestamp; absent with reason not_on_a_list when no list holds the subject",
  "settings.refMaintFactsListsLastSeenAtValue":
    "RFC3339 timestamp; absent with reason not_on_a_list when no list holds the subject",
  "settings.refMaintFactsListsDaysOnListValue":
    "Whole days to the evaluation time; absent with reason not_on_a_list when no list holds the subject",
  "settings.refMaintFactsListsBestRankValue":
    "Rank number; absent with reason no_list_rank when no list that holds the subject ranks it",
  "settings.refMaintFactsListsOnLeaveValue":
    "Sorted values from keep, log, unmonitor and tag",
  "settings.refMaintFactsListsLeftHandledValue":
    "True once every departure was handled; absent with reason no_list_left when no list dropped the subject",
  "settings.refMaintFactsListsMembershipStatesValue":
    "Sorted states such as in_library, added, requested or held",
  "settings.refMaintFactsRequestOriginValue":
    "One of manual, public_list or personal_list; absent with reason title_not_requested when no request created the title",
  "settings.refMaintFactsRequestListNameValue":
    "Name of that public list; absent with reason no_public_request_list for manual and personal-list requests, because a personal list is never named",
  "settings.refMaintSectionFiles": "— One entry per file in input.facts.files",
  "settings.refMaintFileSizeBytes": "Size of the file in bytes",
  "settings.refMaintFileQuality":
    "Detected quality of the file (for example 2160P, 1080P)",
  "settings.refMaintFileVideoCodec":
    "Video codec of the file (for example H.265, H.264)",
  "settings.refMaintFileIs3d":
    "True when media metadata declares 3D video. False means not detected, including files not yet analyzed.",
  "settings.refMaintFileVideoWidth": "Video width in pixels",
  "settings.refMaintFileVideoHeight": "Video height in pixels",
  "settings.refMaintFileAudioLanguages": "Audio languages present in the file",
  "settings.refMaintFileSubtitleLanguages":
    "Subtitle languages present in the file",
  "settings.refMaintFileAddedAt": "RFC3339 timestamp the file was added",
  "settings.refMaintFactsSeriesMovies":
    "Movies linked into this show's timeline, each with its own tags. Present for shows only: the key is missing entirely for a movie subject, and known-empty for a show with no linked movies. add_tags and remove_tags never write these; they act on the subject title.",
  "settings.refMaintFactsSeriesMoviesValue":
    "One entry per linked movie; see input.observations.series_movies.value[] below",
  "settings.refMaintSectionSeriesMovies":
    "— One entry per linked movie in input.facts.series_movies",
  "settings.refMaintSectionObsSeriesMovies":
    "— One entry per linked movie in input.observations.series_movies.value",
  "settings.refMaintSeriesMovieLinkId": "Identifier of the series-movie link",
  "settings.refMaintSeriesMovieName": "Title of the linked movie",
  "settings.refMaintSeriesMovieYear":
    "Release year of the linked movie, when known",
  "settings.refMaintSeriesMovieMonitored":
    "Whether acquisition is enabled for this linked movie",
  "settings.refMaintSeriesMovieHasFile":
    "Whether this linked movie has a file on disk",
  "settings.refMaintSeriesMovieTags":
    "User-defined tags on this linked movie, from the same registry as title tags. A series movie carries its own tags; it does not inherit the show's.",
  "settings.refMaintTagActionsTitle": "Tag actions",
  "settings.refMaintTagActionsNote":
    "add_tags and remove_tags write user tags on the matched title. Every label has to exist in the tag registry an administrator maintains in Settings, and labels are stored lowercase — a rule naming a tag that was deleted afterwards holds with tag_not_defined instead of writing it.",
  "settings.refMaintTagOscillationNote":
    "Two rules that write the opposite patch for the same label on the same title hold both candidates with tag_patch_conflict rather than letting whichever ran last win. A rule that adds a label and another that removes it on a later run is not caught that way: it is an authoring mistake that flips the tag back and forth forever, so scope the two so they cannot both match.",

  "settings.maintenanceTemplateGallery": "Starter templates",
  "settings.maintenanceTemplateGalleryDescription":
    "Load a written-out matcher into the editor. Nothing is saved until you review it and press create.",
  "settings.maintenanceTemplateApply": "Use template",
  "settings.maintenanceTemplateFacetMovie": "Movies",
  "settings.maintenanceTemplateFacetShow": "Shows",
  "settings.maintenanceTemplateGraceBadge": "{{count}}-day grace",
  "settings.maintenanceTemplateNoGraceBadge": "No grace period",
  "settings.maintenanceTemplateDestructiveBadge": "Deletes files",
  "settings.maintenanceTemplateNeedsProfileBadge": "Pick a target profile",
  "settings.maintenanceTemplateDeadWantedTitle": "Dead wanted entries",
  "settings.maintenanceTemplateDeadWantedDescription":
    "Titles you still monitor that no file ever arrived for. Facts are plain values, so a title whose monitoring or file state Scryer cannot see holds instead of matching. After a 30-day grace period the scope is unmonitored and anything already on disk is left alone.",
  "settings.maintenanceTemplateLibraryAgingTitle": "Library aging",
  "settings.maintenanceTemplateLibraryAgingDescription":
    'Caution: this template deletes the title and its files. Anything holding a file ages out unless it carries the "keep" tag. A 180-day grace period runs first, and the instance gates plus this rule\'s own arming still have to agree before anything is removed.',
  "settings.maintenanceTemplateAddedLongAgoTitle": "Added over 180 days ago",
  "settings.maintenanceTemplateAddedLongAgoDescription":
    "Real date math: the matcher subtracts the title's added_at from the evaluation time and matches past 180 days. The scope is unmonitored and the files stay. A title with no added-at value holds rather than matching.",
  "settings.maintenanceTemplateOversizedTitle": "Oversized releases",
  "settings.maintenanceTemplateOversizedDescription":
    "Titles whose files total more than 40 GB. Moves the title onto a quality profile you choose and searches again only if the profile actually changed — pick that target profile in the editor before saving.",
  "settings.maintenanceTemplateFourKPurgeTitle": "4K purge",
  "settings.maintenanceTemplateFourKPurgeDescription":
    "Any title holding a 2160p file. After a 7-day grace period it moves onto a quality profile you choose and searches again if the profile changed. Pick that target profile in the editor before saving.",
  "settings.maintenanceTemplateRequestedExpiryTitle": "Requested media expiry",
  "settings.maintenanceTemplateRequestedExpiryDescription":
    'Caution: this template deletes the title and its files. Titles that came in from a request age out after 120 days unless they carry the "keep" tag.',
  "settings.maintenanceTemplateDepartedRequesterTitle": "Departed requester",
  "settings.maintenanceTemplateDepartedRequesterDescription":
    'Caution: this template deletes the title and its files. It matches titles requested by one named user — edit the "departed-user" placeholder in the matcher to the username you mean before saving, or it will match nothing.',
  "settings.maintenanceTemplateSystemAddedTitle": "System-added cleanup",
  "settings.maintenanceTemplateSystemAddedDescription":
    "Titles a scan discovered rather than a person adding them, and that hold a file. After 60 days the scope is unmonitored and the files stay where they are.",
  "settings.maintenanceTemplateWatchedRequestedTitle":
    "Watched by everyone who asked",
  "settings.maintenanceTemplateWatchedRequestedDescription":
    "Caution: this template deletes the title and its files. It matches requested movies that every one of their requesters has now played, after a 30-day grace period. Movies only: Scryer records plays per movie and per episode, never per show, so a series holds instead of matching. It also holds whenever a requester has no linked media-server account, or the last signal sync is over 48 hours old — a partial watch picture is not evidence to delete on.",
  "settings.maintenanceTemplateExpiredLeasesTitle": "Expired request leases",
  "settings.maintenanceTemplateExpiredLeasesDescription":
    "Caution: this template deletes the title and its files. It matches titles whose request leases have all run out, after a 7-day grace period. A lease that has not started yet — the title has not been imported — is left alone, and so is a title that never had a lease.",
  "settings.maintenanceTemplateNoProfileTitle": "No quality profile",
  "settings.maintenanceTemplateNoProfileDescription":
    "Titles with no quality profile assigned. Takes no action at all: this one is for collecting a list to look at before deciding what should happen to it.",
  "settings.maintenanceTemplateNewestEpisodesTitle":
    "Keep newest 3 downloaded episodes",
  "settings.maintenanceTemplateNewestEpisodesDescription":
    "Caution: this episode-scoped template unmonitors and deletes downloaded episodes ranked older than the newest three by air date. Specials, future episodes, missing files, and unresolved dates receive no rank, so they do not match. Protected episodes still occupy the newest-three slots.",
  "settings.maintenanceTemplateNewestSeasonsTitle":
    "Keep newest 2 downloaded seasons",
  "settings.maintenanceTemplateNewestSeasonsDescription":
    "Caution: this season-scoped template unmonitors and deletes files from seasons ranked older than the newest two by each season's newest eligible downloaded episode. Specials, future episodes, missing files, and unresolved dates receive no rank; protected seasons still occupy slots.",
  "settings.maintenanceTemplateLeftEveryListTitle": "Left every list",
  "settings.maintenanceTemplateLeftEveryListDescription":
    "Titles every list has dropped, public or personal, once each departure has been handled and the last one is more than 30 days old. The scope is unmonitored and the files stay where they are.",
  "settings.maintenanceTemplateListAddedUnwatchedTitle":
    "Added by a list and never watched",
  "settings.maintenanceTemplateListAddedUnwatchedDescription":
    "Adds the needs-review tag to titles a list added more than 90 days ago that nobody has played. Holds whenever the watch picture is incomplete. Nothing is deleted.",

  // ── Request rules ───────────────────────────────────────────────────
  "settings.requestRules": "Request Rules",
  "settings.noRequestRulesFound": "No request rules yet.",
  "settings.requestRulesDefault":
    "With no enabled rule, requests from members with Auto-approve are approved and every other request waits for review.",
  "settings.requestRuleEnabledUpdated": "Updated {{name}}.",
  "settings.requestRuleRequesterFieldsHint":
    "This rule reads requester fields and needs Manage permissions to save.",
  "settings.requestRuleLibraries": "Libraries",
  "settings.requestRuleLibrariesAll": "All libraries",
  "settings.requestRuleLibrariesHelp":
    "Leave every box unticked to apply this rule to requests in every library.",
  "settings.requestRuleRevision": "Revision",
  "settings.requestRuleDecisionCount": "Decisions",
  "settings.requestRuleCopy": "Copy rule",
  "settings.requestRuleCreate": "Create rule",
  "settings.requestRuleUpdate": "Update rule",
  "settings.requestRuleCreateNew": "New request rule",
  "settings.requestRuleCreated":
    "Request rule created. It stays disabled until you enable it.",
  "settings.requestRuleUpdated": "Request rule updated.",
  "settings.requestRuleDeleted": "Deleted request rule {{name}}.",
  "settings.requestRuleDeleting":
    "Delete the request rule {{name}}? Its recorded decisions are kept.",
  "settings.requestRuleValidationRequired":
    "A request rule needs a name and a matcher.",
  "settings.requestRuleSaveDisabledNote":
    "A new rule is saved disabled. Tick Enabled in the table to let it decide requests.",
  "settings.requestRuleConfirmDiscardTitle": "Discard unsaved changes?",
  "settings.requestRuleConfirmDiscardDescription":
    "The matcher in the editor has not been saved. Continuing replaces it.",
  "settings.requestRuleRequesters": "Name people in this rule",
  "settings.requestRuleRequestersHelp":
    "Tick the accounts this rule is about and apply them. The matcher in the editor is rewritten so you can read it before saving.",
  "settings.requestRuleRequestersApply": "Write these names into the matcher",
  "settings.requestRuleRequestersUnavailable":
    "Listing accounts needs permission this page does not carry, so names have to be typed into the matcher by hand.",
  "settings.requestRuleRequestersNoTarget":
    "This matcher does not name anybody, so there is nowhere to write the accounts you picked. Add a requesters set or a username comparison first.",

  "settings.requestPreviewTitle": "Try this rule against a request",
  "settings.requestPreviewSubtitle":
    "Runs one matcher against a request you describe. Nothing is submitted and nothing is recorded.",
  "settings.requestPreviewSource": "Matcher",
  "settings.requestPreviewSourceStored": "A saved rule",
  "settings.requestPreviewSourceDraft": "The editor draft",
  "settings.requestPreviewRuleSelect": "Rule",
  "settings.requestPreviewRuleSelectPlaceholder": "Choose a rule",
  "settings.requestPreviewUser": "Requester",
  "settings.requestPreviewUserPlaceholder": "Choose an account",
  "settings.requestPreviewLibrary": "Library",
  "settings.requestPreviewLibraryPlaceholder": "Choose a library",
  "settings.requestPreviewProfile": "Quality profile",
  "settings.requestPreviewProfilePlaceholder": "Choose a profile",
  "settings.requestPreviewMonitor": "Monitoring",
  "settings.requestPreviewLease": "Keep for",
  "settings.requestPreviewLeaseDays": "Days",
  "settings.requestPreviewMedia": "Media",
  "settings.requestPreviewMediaPlaceholder": "Search for a title",
  "settings.requestPreviewMediaSelected": "Using {{title}}.",
  "settings.requestPreviewRun": "Run preview",
  "settings.requestPreviewRunning": "Running…",
  "settings.requestPreviewEmpty":
    "Describe a request and run the preview to see what this rule would say.",
  "settings.requestPreviewMetadataPartial": "Metadata incomplete",
  "settings.requestPreviewShowInput": "Show the input the matcher saw",
  "settings.requestPreviewHideInput": "Hide the input",
  "settings.requestPreviewInputUnavailable":
    "The input document could not be read back.",
  "settings.requestPreviewUndefinedTags":
    "Define these in Settings › Tags before they can be applied.",

  "settings.requestDecisionsTitle": "Recent decisions",
  "settings.requestDecisionsSubtitle":
    "What request rules have decided lately.",
  "settings.requestDecisionsOutcomeFilter": "Outcome",
  "settings.requestDecisionsOutcomeAll": "Any outcome",
  "settings.requestDecisionsColTime": "Evaluated",
  "settings.requestDecisionsColOutcome": "Outcome",
  "settings.requestDecisionsColVotes": "Votes",
  "settings.requestDecisionsColReasons": "Reasons",
  "settings.requestDecisionsEmpty": "No decisions recorded yet.",
  "settings.requestDecisionsPolicySaid": "The rules said {{outcome}}.",
  "settings.requestDecisionNoVotes": "No rule voted.",
  "settings.requestDecisionHeld": "held",

  "settings.requestTemplateGallery": "Starter rules",
  "settings.requestTemplateGalleryDescription":
    "Worked examples you can load into the editor. Applying one fills the form in; you still read it, name it, scope it and save it.",
  "settings.requestTemplateGalleryTagNote":
    "A template that emits a tag needs that tag defined in Settings › Tags; undefined ones are dropped when a request is decided.",
  "settings.requestTemplateApply": "Load into the editor",
  "settings.requestTemplatePersonTargetedBadge": "Names people",
  "settings.requestTemplateNamedRequestersTitle":
    "Named requesters, family-rated media",
  "settings.requestTemplateNamedRequestersDescription":
    "Approves requests from three named people when the certification is family-rated, and tags the most family-friendly of them. The names are placeholders: use the account picker to replace them before saving.",
  "settings.requestTemplateShortLeaseTitle": "Short leases from one person",
  "settings.requestTemplateShortLeaseDescription":
    "Approves one named person's requests when they ask to keep the media for two weeks or less. A request to keep it forever carries no day count, so it is not approved here.",
  "settings.requestTemplateLowResolutionTitle": "Low-resolution requests",
  "settings.requestTemplateLowResolutionDescription":
    "Approves one named person's requests when the quality profile they chose tops out at 720p or below.",
  "settings.requestTemplateDenyAdultTitle": "Deny adult content",
  "settings.requestTemplateDenyAdultDescription":
    "Denies any request for media the metadata marks as adult, and records why. Names nobody, so it applies to every requester.",
  "settings.requestTemplateMonthlyQuotaTitle": "Monthly approval quota",
  "settings.requestTemplateMonthlyQuotaDescription":
    "Sends a request for review once the requester has had five approved in the last thirty days. It never denies; it asks a person to look.",
  "settings.requestTemplateHoldApprovalListsTitle":
    "Hold list requests from members on Approval policy",
  "settings.requestTemplateHoldApprovalListsDescription":
    "Sends a list request for review when the list's owner is on the Approval list policy. Manual requests are left alone.",
  "settings.requestTemplateDenyPersonalListAbove1080pTitle":
    "Deny personal-list requests above 1080p",
  "settings.requestTemplateDenyPersonalListAbove1080pDescription":
    "Denies a request from a member's personal list when its quality profile goes above 1080p. Public lists and manual requests are left alone.",

  "settings.refReqTitle": "Request input reference",
  "settings.refReqSubtitle":
    "Every value a request matcher can read, and what each one means.",
  "settings.refReqIntro":
    "A request rule reads one document describing the request, the person who made it, the library it targets, and what Scryer knows about the media. Facts are plain values. A fact Scryer could not observe is simply missing, and a rule that reads a missing fact holds the request for a person instead of guessing — you never write that guard yourself.",
  "settings.refReqOutputTitle": "What a request rule defines",
  "settings.refReqOutputIntro":
    "At least one of these has to be defined, or the rule can never vote and Scryer refuses to save it.",
  "settings.refReqOutputApprove":
    "approve — the request may be granted without a person seeing it.",
  "settings.refReqOutputDeny":
    "deny — the request is refused. A denial beats every other vote.",
  "settings.refReqOutputManual":
    "manual — the request goes to a person, whatever else voted.",
  "settings.refReqOutputTags":
    'tags contains "…" — labels for the media, applied only on an approval.',
  "settings.refReqOutputReasons":
    'reasons contains "…" — stable codes the requester is shown.',
  "settings.refReqOutputNoPackage":
    "Do not write a package or import line. Scryer generates both and strips them back off when it hands the matcher to this editor.",

  // ── Request leases, decisions and claims ────────────────────────────
  "search.keepFor": "Keep for",
  "search.keepForForever": "Forever",
  "search.keepForDays": "{{count}} days",
  "search.keepForCustom": "A number of days…",
  "search.keepForCustomLabel": "Days to keep",
  "search.keepForHelp":
    "How long Scryer holds this media for you. When the time is up a maintenance rule may remove it; keeping it forever never expires.",

  "requests.preflightAutoApprove": "This will be approved automatically",
  "requests.preflightManualReview": "This needs approval",
  "requests.preflightDeny": "This would be denied: {{reasons}}",
  "requests.preflightTags": "It will be tagged {{tags}}.",
  "requests.preflightReason": "Reason: {{reason}}.",
  "requests.preflightMetadataPartial":
    "Scryer could not read everything about this title, so a person will decide.",

  "requests.leaseForever": "Kept forever",
  "requests.leaseRequested": "Asked for {{count}} days",
  "requests.leaseDormant": "{{count}} days from first import",
  "requests.leaseActive": "Kept until {{date}}",
  "requests.leaseExpired": "Lease expired",
  "requests.leaseReleased": "Lease released",

  "requests.decisionTitle": "How this was decided",
  "requests.decisionPolicySaid":
    "The rules said {{outcome}}; something else decided the request.",
  "requests.decisionVotes": "Rule votes",
  "requests.decisionNothingToShow":
    "No rule had anything to say about this request.",
  "requests.decisionOutcomeAutoApprove": "Approved by rule",
  "requests.decisionOutcomeManualReview": "Sent for review",
  "requests.decisionOutcomeDeny": "Denied by rule",
  "requests.denyReason": "Denied because: {{reasons}}",
  "requests.policyTagsLabel": "Would be tagged",

  "requests.voteApprove": "approve",
  "requests.voteDeny": "deny",
  "requests.voteManual": "review",
  "requests.voteAbstain": "no opinion",
  "requests.voteNone": "no vote",

  "requests.fallbackRuleManual": "A rule asked for a person to look at this.",
  "requests.fallbackHeld":
    "A rule could not decide, because something it reads was unknown.",
  "requests.fallbackError":
    "A rule failed to run, so the request went to a person.",
  "requests.fallbackNoRuleMatched": "No rule matched this request.",
  "requests.fallbackLibraryPermission":
    "Approved by the requester's Auto-Approve Requests permission.",

  "requests.approvedLease": "Keep for",
  "requests.approvedLeaseKeepForever": "Forever, as asked",
  "requests.approvedLeaseKeepRequested": "{{count}} days, as asked",
  "requests.approvedTags": "Tags",
  "requests.approvedTagsHelp":
    "Applied to the title when you approve. Prefilled with what the rules emitted; add or remove any tag an administrator has defined in Settings > Tags.",
  "requests.approvedTagsNone": "No tags will be applied.",

  "requests.claimsShow": "Retention",
  "requests.claimsHide": "Hide retention",
  "requests.claimsEmpty": "Nothing is holding this title.",
  "requests.claimExpiresAt": "Held until {{date}}",
  "requests.claimDurationDays": "{{count}} days, starting at the first import",
  "requests.claimReleasedReason": "Released: {{reason}}",
  "requests.claimStateDormant": "Waiting for import",
  "requests.claimStateActive": "Holding",
  "requests.claimStateExpired": "Expired",
  "requests.claimStateReleased": "Released",
  "requests.claimStateConverted": "Replaced",
  "requests.claimExtend": "Extend",
  "requests.claimExtendTitle": "Extend this hold",
  "requests.claimExtendUntil": "Hold until",
  "requests.claimMakePermanent": "Keep permanently",
  "requests.claimRelease": "Release",
  "requests.claimReleaseTitle": "Release this hold",
  "requests.claimReleaseReason": "Reason",
  "requests.claimReleaseReasonPlaceholder": "Why this hold is being lifted",
  "requests.claimReleaseReasonHelp":
    "A reason is required; it is recorded against the hold.",

  "status.requestApprovedClaimFailed":
    "Approved {{name}}, but nothing is holding it: {{error}}",
  "status.claimExtended": "Hold extended.",
  "status.claimMadePermanent": "The title is now held permanently.",
  "status.claimReleased": "Hold released.",

  // ── Request rules context reference ─────────────────────────────────
  "settings.refReqSectionInput": "— The request being decided",
  "settings.refReqInputSchemaVersion":
    "Version of this document's shape; it only changes when fields are added",
  "settings.refReqInputEvaluationTime":
    "RFC3339 timestamp the request was evaluated at",
  "settings.refReqSectionNow":
    "— The clock at evaluation time, for rules that depend on when a request arrives",
  "settings.refReqNowWeekday":
    "Day of the week in UTC, lowercase and in English (monday … sunday)",
  "settings.refReqNowHourUtc": "Hour of the day in UTC, 0 to 23",
  "settings.refReqSectionRequester":
    "— The person who made the request. A matcher that reads any of these needs permission-management authority to save",
  "settings.refReqRequesterUserId": "Identifier of the requesting account",
  "settings.refReqRequesterUsername": "Username of the requesting account",
  "settings.refReqRequesterAccountKind":
    "How the account signs in (for example local, or a linked provider)",
  "settings.refReqRequesterAppPermissions":
    "Application-wide permissions the account holds",
  "settings.refReqRequesterLibraryPermissions":
    "Permissions the account holds on the library being requested from",
  "settings.refReqRequesterLinkedProviders":
    "Media-server and identity providers the account has linked",
  "settings.refReqRequesterCreatedAt":
    "When the account was created; absent, because Scryer does not record it yet",
  "settings.refReqSectionLibrary": "— The library the request targets",
  "settings.refReqLibraryId": "Identifier of the library",
  "settings.refReqLibraryName": "Name of the library",
  "settings.refReqLibraryFacet":
    "Kind of media the library holds (MOVIE, SERIES or ANIME)",
  "settings.refReqLibraryIsDefault":
    "True when this is the default library for its kind",
  "settings.refReqSectionRequest": "— What was asked for",
  "settings.refReqRequestOrigin":
    "Where the request came from: manual, public_list or personal_list",
  "settings.refReqRequestTitle": "Title as it read at request time",
  "settings.refReqRequestYear": "Release year, when the provider knew one",
  "settings.refReqRequestExternalIds":
    "Provider identifiers; see input.request.external_ids below",
  "settings.refReqRequestQualityProfileId":
    "Identifier of the quality profile the requester chose",
  "settings.refReqRequestQualityProfileName": "Name of that profile",
  "settings.refReqRequestMonitorType":
    "Monitoring the requester asked for (for example ALL_EPISODES)",
  "settings.refReqRequestMonitorSelectionSeasonCount":
    "How many seasons an advanced monitoring selection named",
  "settings.refReqRequestLeaseForever":
    "True when the requester asked to keep the media forever",
  "settings.refReqRequestLeaseDays":
    "How many days the requester asked to keep it for; absent when they asked for forever",
  "settings.refReqRequestList":
    "The list that submitted the request; see input.request.list below. Absent for a manual request",
  "settings.refReqSectionExternalIds":
    "— Provider identifiers on input.request.external_ids",
  "settings.refReqSectionRequestList":
    "— The list behind a list request, on input.request.list. Absent for a manual request and when the list could not be read",
  "settings.refReqRequestListSubscriptionId": "Identifier of the list",
  "settings.refReqRequestListName":
    "Name of the list; absent for a personal list, which is never named",
  "settings.refReqRequestListScope": "public or personal",
  "settings.refReqRequestListMode":
    "What the list does with its items: add, hold, request or discover",
  "settings.refReqRequestListProvider": "The list provider",
  "settings.refReqRequestListOwnerIsRequester":
    "True when the list's owner is the requester",
  "settings.refReqRequestListOwnerListPolicy":
    "The owner's list policy: auto, approval or none; approval when none is stored",
  "settings.refReqRequestListRank":
    "The item's rank on the list; absent when the list does not rank it",
  "settings.refReqExternalIdsTmdb":
    "TMDB identifier, when the provider had one",
  "settings.refReqExternalIdsImdb":
    "IMDb identifier, when the provider had one",
  "settings.refReqExternalIdsTvdb":
    "TVDB identifier, when the provider had one",
  "settings.refReqSectionFacts":
    "— What Scryer knows about the media and about this requester's history. A fact Scryer could not observe is missing, and reading a missing fact holds the request for a person",
  "settings.refReqFactsAgeRating": "Minimum age the certification implies",
  "settings.refReqFactsCertifications":
    "Every certification the metadata carries; see input.facts.certifications[] below",
  "settings.refReqFactsCertificationLabel":
    "US certification label (for example G, PG, PG-13, R)",
  "settings.refReqFactsCertificationRank":
    "That label as a number, lowest first, so rules can compare rather than list labels",
  "settings.refReqFactsCommonsenseRecommended":
    "Whether Common Sense Media recommends the title for its age band",
  "settings.refReqFactsGenres": "Genre names on the title",
  "settings.refReqFactsCanonicalTagKeys":
    "Scryer's canonical tag keys for the title",
  "settings.refReqFactsThemes": "Theme names on the title",
  "settings.refReqFactsIsAdult":
    "Whether the metadata marks the title as adult",
  "settings.refReqFactsRating": "Combined metadata rating",
  "settings.refReqFactsRatingsBySource":
    "The same ratings keyed by the source that gave them",
  "settings.refReqFactsTmdbVoteAverage": "TMDB's average vote",
  "settings.refReqFactsTmdbVoteCount": "How many votes that average is made of",
  "settings.refReqFactsPopularity": "Provider popularity score",
  "settings.refReqFactsRuntimeMinutes": "Runtime in minutes",
  "settings.refReqFactsOriginalLanguage": "Original language code",
  "settings.refReqFactsCountry": "Country of origin",
  "settings.refReqFactsNetwork": "Network that aired it",
  "settings.refReqFactsStudio": "Studio that made it",
  "settings.refReqFactsContentStatus":
    "Provider content status (for example Continuing, Ended)",
  "settings.refReqFactsReleaseDate": "Release date, for movies",
  "settings.refReqFactsFirstAired": "First air date, for series",
  "settings.refReqFactsReleaseAgeDays":
    "How many days ago it was released or first aired",
  "settings.refReqFactsAwardCount": "How many awards the metadata records",
  "settings.refReqFactsQualityProfileTiers":
    "Tier names in the quality profile the requester chose",
  "settings.refReqFactsQualityProfileMaxResolution":
    "Highest vertical resolution that profile allows, in pixels",
  "settings.refReqFactsQualityProfileAllowsUpgrades":
    "Whether that profile keeps upgrading after a first grab",
  "settings.refReqFactsExistsInLibraryIds":
    "Libraries that already hold this title",
  "settings.refReqFactsPreviousRequestCount":
    "How many times this requester has asked for this title before",
  "settings.refReqFactsPreviouslyDenied":
    "Whether one of those earlier requests was denied",
  "settings.refReqFactsPreviouslyApproved":
    "Whether one of those earlier requests was approved",
  "settings.refReqFactsPendingRequestCount":
    "How many of this requester's requests are still waiting",
  "settings.refReqFactsApprovedLast30d":
    "How many of this requester's requests were approved in the last thirty days",
  "settings.refReqFactsDeniedLast30d":
    "How many were denied in the last thirty days",
  "settings.refReqFactsTotalApproved":
    "How many of this requester's requests have ever been approved",
  "settings.refReqFactsActiveLeaseCount":
    "How many titles this requester currently holds a lease on",
  "settings.refReqFactsDaysSinceLastRequest":
    "Days since this requester last asked for anything",
  "settings.refReqFactsLibraryTitleCount":
    "How many titles the target library holds",
  "settings.refReqSectionCertifications":
    "— One entry per certification in input.facts.certifications",
  "settings.refReqCertCountry": "Country the certification was issued in",
  "settings.refReqCertValue": "The certification itself (for example PG-13)",
  "settings.refReqCertSource": "Where the certification came from",
  "settings.refReqSectionObservations":
    "— The same facts as full envelopes, for rules that need to tell absent apart from unknown or to read the reason. Reading a fact here opts that fact out of the automatic hold: you are taking responsibility for the three-valued answer.",
  "settings.refReqObsStatus":
    "known, absent, or unknown. absent means the source answered and there is no value; unknown means Scryer could not find out. Test this before reading value — an unknown fact is never coerced to false, 0, or an empty string.",
  "settings.refReqObsObservedAt":
    "When the value was observed, if the source recorded a time",
  "settings.refReqObsReason":
    "Stable machine code explaining why the fact is unknown, or why an absent fact has no value",
  "settings.refReqSectionObsCertifications":
    "— One entry per certification in input.observations.certifications.value",
  "settings.refReqFactsAgeRatingValue":
    "Minimum age implied by the certification; absent with reason no_certification when the metadata carries none",
  "settings.refReqFactsCertificationsValue":
    "One entry per certification; see input.observations.certifications.value[] below",
  "settings.refReqFactsCertificationLabelValue":
    "The US certification label; absent when the metadata carries no US certification",
  "settings.refReqFactsCertificationRankValue":
    "That label as a number, lowest first; absent when the label is one Scryer does not rank",
  "settings.refReqFactsCommonsenseRecommendedValue":
    "True when Common Sense Media recommends the title for its age band",
  "settings.refReqFactsGenresValue":
    "Genre names, deduplicated; an empty list when the metadata names none",
  "settings.refReqFactsCanonicalTagKeysValue":
    "Canonical tag keys, deduplicated",
  "settings.refReqFactsThemesValue": "Theme names, deduplicated",
  "settings.refReqFactsIsAdultValue":
    "True when the metadata marks the title as adult",
  "settings.refReqFactsRatingValue": "The combined rating as a number",
  "settings.refReqFactsRatingsBySourceValue":
    "An object keyed by source name, each value that source's rating",
  "settings.refReqFactsTmdbVoteAverageValue": "TMDB's average vote as a number",
  "settings.refReqFactsTmdbVoteCountValue":
    "How many votes that average is made of",
  "settings.refReqFactsPopularityValue": "The popularity score as a number",
  "settings.refReqFactsRuntimeMinutesValue": "Runtime in whole minutes",
  "settings.refReqFactsOriginalLanguageValue": "Original language as a code",
  "settings.refReqFactsCountryValue": "Country of origin",
  "settings.refReqFactsNetworkValue": "Name of the network",
  "settings.refReqFactsStudioValue": "Name of the studio",
  "settings.refReqFactsContentStatusValue":
    "The provider's content status string",
  "settings.refReqFactsReleaseDateValue": "RFC3339 release date",
  "settings.refReqFactsFirstAiredValue": "RFC3339 first air date",
  "settings.refReqFactsReleaseAgeDaysValue":
    "Whole days since release or first airing",
  "settings.refReqFactsAwardCountValue":
    "Number of awards recorded; a confirmed zero when there are none",
  "settings.refReqFactsQualityProfileTiersValue":
    "Tier names in the chosen profile, in profile order",
  "settings.refReqFactsQualityProfileMaxResolutionValue":
    "Highest vertical resolution the profile allows, in pixels",
  "settings.refReqFactsQualityProfileAllowsUpgradesValue":
    "True when the profile keeps upgrading after a first grab",
  "settings.refReqFactsExistsInLibraryIdsValue":
    "Identifiers of libraries already holding the title; an empty list when none do",
  "settings.refReqFactsPreviousRequestCountValue":
    "Number of earlier requests by this requester for this title",
  "settings.refReqFactsPreviouslyDeniedValue":
    "True when one of those earlier requests was denied",
  "settings.refReqFactsPreviouslyApprovedValue":
    "True when one of those earlier requests was approved",
  "settings.refReqFactsPendingRequestCountValue":
    "Number of this requester's requests still waiting on a decision",
  "settings.refReqFactsApprovedLast30dValue":
    "Number approved in the last thirty days, measured against the evaluation time",
  "settings.refReqFactsDeniedLast30dValue":
    "Number denied in the last thirty days, measured against the evaluation time",
  "settings.refReqFactsTotalApprovedValue":
    "Number of this requester's requests ever approved",
  "settings.refReqFactsActiveLeaseCountValue":
    "Number of titles this requester currently holds a lease on, dormant leases included",
  "settings.refReqFactsDaysSinceLastRequestValue":
    "Whole days since the requester's previous request; absent with reason no_previous_request when this is their first",
  "settings.refReqFactsLibraryTitleCountValue":
    "Number of titles in the target library",

  "move.dialogTitle": "Move titles",
  "move.dialogDescription":
    "Review the plan for {{count}} selected title(s) before anything moves.",
  "move.destinationLibrary": "Destination library",
  "move.destinationRoot": "Destination root",
  "move.destinationRootPlaceholder": "Choose a root",
  "move.actionHeading": "Move to…",
  "move.actionDescription":
    "Move this title's files to another root folder, or to another library.",
  "move.actionButton": "Move To…",
  "move.wizardKindDescription": "Choose where this title should go.",
  "move.kindRootHeading": "Move to another root",
  "move.kindRootHelp":
    "Keep the title in this library and move its files to one of its other root folders.",
  "move.kindLibraryHeading": "Move to another library",
  "move.kindLibraryHelp":
    "Transfer the title into a different library, onto one of that library's roots.",
  "move.noOtherRoots":
    "This library has no other root folder to move to. Add one to the library, or move the title to another library.",
  "move.back": "Back",
  "move.next": "Next",
  "move.destinationRootHelp":
    "Changing the destination opens the move workflow instead of saving the title.",
  "move.destinationNoSelection": "No titles are selected.",
  "move.destinationMixedSourceLibraries":
    "The selection spans several libraries; move one library at a time.",
  "move.destinationCrossLibraryNotice":
    "These titles leave their current library and become part of {{library}}.",
  "move.destinationLibraryInMoveWorkflow":
    "Choose a different library in the move workflow, after picking a destination root.",
  "move.transferIntoLibrary": "Transfers into {{library}}.",
  "move.sameNameWarning":
    "{{library}} already has a title named “{{title}}”. Scryer never merges by name, so this transfer leaves two titles with that name.",
  "move.ambiguousDestinationHelp":
    "Several titles in the destination library share this title's identity, and Scryer will not guess between them. Resolve the identity in the destination library first, then move this title again.",
  "move.ambiguousCandidateShared": "Shares {{identities}}",
  "move.mergeStatement":
    "Merges into “{{title}}”. The two become one title: this title's identity is absorbed and “{{title}}”'s settings win.",
  "move.mergeSummaryHeading": "What this merge does",
  "move.mergeNoDetails": "This merge carries nothing else over.",
  "move.mergeBlockedHeading":
    "This merge cannot run: these records cannot be carried over.",
  "move.mergeCarried":
    "{{files}} media file(s) and {{history}} history entries come along.",
  "move.mergeDropped":
    "{{count}} other record(s) on this title (tags, requests, acquisition state) retire with it.",
  "move.mergeRoleChangesHeading": "{{demotions}} file(s) become extra copies",
  "move.mergeRoleChangesHeadingPlain": "File roles",
  "move.mergeRoleGroup.DESTINATION_PRIMARY_RETAINED":
    "“{{title}}” already has a primary file for these episodes. The incoming files are kept as extra copies, and nothing already there is replaced.",
  "move.mergeRoleGroup.DESTINATION_PRIMARY_RETAINED_TITLE":
    "“{{title}}” already has a primary file. The incoming file is kept as an extra copy, and nothing already there is replaced.",
  "move.mergeRoleGroup.SOURCE_PRIMARY_ALREADY_CLAIMED":
    "Another file in this move is already the primary for these episodes, so these are kept as extra copies.",
  "move.mergeRoleGroup.SOURCE_PRIMARY_ALREADY_CLAIMED_TITLE":
    "Another file in this move is already the primary, so this one is kept as an extra copy.",
  "move.mergeRoleGroup.COLLAPSED_SOURCE_EPISODES":
    "These files are listed under two episodes here that are one episode at the destination. Each is listed once there.",
  "move.mergeRoleUnnamedFile": "file {{id}}",
  "move.operationTransferNote": "These titles are moving into another library.",
  "move.modeHeading": "How should the files get there?",
  "move.mode.MOVE_WITH_SCRYER": "Move with Scryer",
  "move.mode.FILES_ALREADY_THERE": "Files are already there",
  "move.modeHelp.MOVE_WITH_SCRYER":
    "Scryer moves or copies the files and verifies every one before the catalog changes.",
  "move.modeHelp.FILES_ALREADY_THERE":
    "Scryer scans the destination and adopts the files you moved yourself.",
  "move.catalogOnlyFastPath":
    "These titles have no files on disk, so this is a catalog-only reassignment: nothing moves and no mode is needed.",
  "move.selectDestinationPrompt": "Choose a destination root to see the plan.",
  "move.previewLoading": "Gathering all the info...",
  "move.gatheringInfo": "Gathering all the info...",
  "move.methodManaged": "Scryer moves files",
  "move.methodManual": "I'll move the files",
  "move.manualLoading": "Loading folder instructions...",
  "move.manualInstructions": "Move the files, then come back and click Next.",
  "move.changeDestination": "Change destination",
  "move.manualMappingChanged":
    "Catalog locations changed. Review the updated folders before clicking Next again.",
  "move.manualStructure":
    "Use the destination folders below, preserving filenames and relative directories. Next updates the catalog and trusts that you moved the files; Scryer will not check or change them.",
  "move.manualSource": "Current folder",
  "move.manualDestination": "Move to folder",
  "move.retryInstructions": "Reload instructions",
  "move.previewFailed": "Failed to preview the move.",
  "move.planChanged":
    "The plan changed since you previewed it. Review the fresh plan before confirming.",
  "move.rePreview": "Preview again",
  "move.summaryTitles": "Titles",
  "move.summaryFiles": "Files",
  "move.summarySize": "Total size",
  "move.sampledItems": "sample",
  "move.blockedHeading": "{{count}} title(s) cannot move yet",
  "move.blockedHelp":
    "Deselect them to move the rest, or wait for their work to finish.",
  "move.deselect": "Deselect",
  "move.deselectedCount": "{{count}} title(s) removed from this move.",
  "move.restoreSelection": "Restore selection",
  "move.titleFileSummary": "{{files}} file(s) · {{size}}",
  "move.freeSpaceRequired":
    "Needs {{required}} on the destination; {{available}} free.",
  "move.freeSpaceUnknown": "unknown",
  "move.freeSpaceInsufficient":
    "The destination does not have enough free space.",
  "move.startRefusedNoSpace":
    "The destination does not have enough free space for this move. Free up space or move fewer titles, then try again.",
  "move.freeSpaceNotProbed": "The destination volume could not be measured.",
  "move.freeSpaceRecycleOtherVolume":
    "The recycle bin is on another volume and needs {{required}}.",
  "move.verificationStatementFull":
    "All {{files}} file(s) ({{bytes}}) will be read back in full and compared.",
  "move.verificationStatementQuick":
    "All {{files}} file(s) ({{bytes}}) will be checked against their head, tail, and size.",
  "move.verificationNotApplicable":
    "No files move, so nothing needs verifying.",
  "move.verificationStampFull": "verified (full)",
  "move.verificationStampQuick": "verified (quick)",
  "move.verificationFallbackCount":
    "{{count}} file(s) fell back to the quick check",
  "move.typedConfirmationPrompt": "Type the phrase to confirm",
  "move.confirm": "Start move",
  "move.startFailed": "Failed to start the move.",
  "move.operationLoading": "Loading the operation…",
  "move.operationLoadFailed": "Failed to load the operation.",
  "move.operationMissing": "That operation no longer exists.",
  "move.operationDismiss": "Close",
  "move.operationProgress": "Titles {{titles}} · Files {{files}} · {{bytes}}",
  "move.counterMerges": "Merged",
  "move.counterDedups": "Deduplicated",
  "move.counterRenames": "Renamed",
  "move.counterNoOps": "No-ops",
  "move.counterUnresolved": "Unresolved",
  "move.counterBlocked": "Blocked",
  "move.checkpointsHeading": "Titles",
  "move.checkpointsPending": "Titles appear here as they enter the operation.",
  "move.checkpointsShowAll": "Show all {{count}} titles",
  "move.checkpointsShowLess": "Show fewer titles",
  "move.transferShowMore": "Show more",
  "move.transferFilesForTitle": "Show files for {{title}}",
  "move.transferFilePath": "File",
  "move.transferCopyProgress": "Placement",
  "move.transferPlacementAwaitingComparison": "Awaiting comparison",
  "move.transferVerifyProgress": "Verification",
  "move.transferVerificationBytes": "Verified {{done}} / {{total}}",
  "move.transferNoFiles": "No file transfers for this title.",
  "move.transferFileState.QUEUED": "Queued",
  "move.transferFileState.WAITING_FOR_STORAGE":
    "Waiting for storage · checking every 2 min",
  "move.transferFileState.DUPLICATE": "Duplicate (no copy)",
  "move.transferFileState.MOVING": "Moving",
  "move.transferFileState.VERIFYING": "Verifying",
  "move.transferFileState.COMPARING": "Comparing files",
  "move.transferFileState.DONE": "Done",
  "move.transferFileState.FAILED": "Error",
  "move.transferFileState.BLOCKED": "Blocked",
  "move.transferFileState.CANCELED": "Canceled",
  "move.transferFileState.NOT_PROCESSED": "Not processed",
  "move.transferShowLess": "Show less",
  "move.transferOverallProgress": "Overall progress",
  "move.transferEstimating": "Estimating…",
  "move.transferDisconnected": "Reconnecting…",
  "move.transferEta": "About {{minutes}} min remaining",
  "move.transferColumnTitle": "Title",
  "move.transferColumnStatus": "Status",
  "move.transferColumnFiles": "Files done/total",
  "move.transferColumnCurrentFile": "Current file",
  "move.transferColumnProgress": "Progress",
  "move.transferRange": "{{from}}–{{to}} of {{total}}",
  "move.transferPrevious": "Previous",
  "move.transferNext": "Next",
  "move.transferException": "Show exception for {{title}}",
  "move.transferActive": "{{copying}} moving · {{verifying}} verifying",
  "move.transferDetailPending": "No further details available.",
  "move.checkpointClass": "Classified as",
  "move.checkpointFrom": "From",
  "move.checkpointTo": "To",
  "move.checkpointMergedInto": "Merged into",
  "move.checkpointVerified": "Verified",
  "move.checkpointVerifiedValue": "{{files}} files · {{bytes}}",
  "move.checkpointDetail": "Detail",
  "move.assetsHeading": "Renamed and deduplicated files",
  "move.assetRenamedAs": "{{from}} → {{to}}",
  "move.assetDeduplicatedAgainst": "{{from}} deduplicated against {{to}}",
  "move.assetUnknownCounterpart": "an unnamed file",
  "move.assetProvenance": "(from {{library}})",
  "move.assetStateDone": "Done",
  "move.assetStatePlanned": "Planned",
  "move.cancelAction": "Cancel operation",
  "move.cancelRequested":
    "Cancellation requested; finishing in-flight work safely.",
  "move.cancelNotPossible": "This operation has already finished.",
  "move.cancelFailed": "Failed to request cancellation.",
  "move.cancelDraining": "Finishing in-flight work",
  "move.resumeAction": "Resume",
  "move.resumeRequested": "Resuming from the last verified checkpoint.",
  "move.resumeNotPossible": "This operation cannot be resumed.",
  "move.resumeFailed": "Failed to resume the operation.",
  "move.retryAction": "Retry",
  "move.retryRequested": "Retrying from the last verified checkpoint.",
  "move.retryNotPossible": "This operation cannot be retried.",
  "move.retryFailed": "Failed to retry the operation.",
  "move.abandonAction": "Abandon",
  "move.abandonRequested":
    "Abandoned. Its titles and roots are released; finished titles are unchanged, and Retry moves the rest once the storage is back.",
  "move.abandonNotPossible": "This operation has already stopped.",
  "move.abandonFailed": "Failed to abandon the operation.",
  "move.stalledNoRunner": "No runner",
  "move.stalledGuidance":
    "No runner is working on this operation. If the storage is back, press Resume. If it is not coming back, press Abandon to release its titles and roots; Retry is available after that.",
  "move.planAgainAction": "Plan again",
  "move.planAgainTitleAction": "Plan this title again",
  "move.planAgainFailed": "Could not open the move dialog.",
  "move.planAgainCatalogBlocked":
    "{{count}} title(s) are left out of Plan again because their catalog update did not finish after the files were verified; Retry finishes them.",
  "move.reason.ownership_conflict":
    "Another operation still owns some of these titles or roots. Wait for it to finish (or cancel it), then retry.",
  "move.reason.storage_error":
    "A file could not be read or written. Nothing was removed from the source. Fix the storage problem (free space, permissions, the mount), then retry; files that already verified are not copied again.",
  "move.reason.verification_mismatch":
    "A copied file did not match its source. The source is untouched and the copy is not trusted. Retry; if the same file fails again, check the destination disk.",
  "move.reason.stale_plan":
    "Something changed under the plan after the preview was taken. Completed titles are consistent. Plan the move again to continue with a fresh preview.",
  "move.reason.merge_unavailable":
    "This title can no longer merge into its destination. Fix the destination title (or deselect it), then plan the move again.",
  "move.reason.catalog_write_failed":
    "The files were copied and verified, but the catalog update failed. Nothing was removed from the source. Retry to finish the catalog update; do not plan a new move for this title until it succeeds.",
  "move.reason.source_cleanup_failed":
    "The move finished but the old copy could not be removed. Retry to remove it, or confirm the title plays from its new location and delete the old folder yourself.",
  "move.reason.stranded":
    "This operation was accepted but never ran. Retry it, or plan the move again.",
  "move.reason.canceled":
    "Canceled at a title boundary. Completed titles are unchanged; retry to move the rest.",
  "move.reason.abandoned":
    "Abandoned. Completed titles are unchanged. Retry once the storage is back.",
  "move.reason.root_unavailable":
    "A root this operation needs is not available. Resume once it is mounted again, or abandon the operation to release its titles.",
  "move.class.ROOT_MOVE": "Moving to the destination root",
  "move.class.CROSS_LIBRARY_TRANSFER": "Transferring to another library",
  "move.class.CATALOG_ONLY": "Catalog only (no files on disk)",
  "move.class.NO_OP": "Already at the destination",
  "move.class.NEEDS_RESOLUTION": "Needs a decision",
  "move.class.INCOMPATIBLE": "Cannot go there",
  "move.planKind.MOVE": "Moves",
  "move.planKind.RENAME": "Renames",
  "move.planKind.MERGE": "Merges",
  "move.planKind.DEDUP": "May merge with identical files",
  "move.planKind.CATALOG_CHANGE": "Catalog changes",
  "move.planKind.ROLE_CHANGE": "Role changes",
  "move.planKind.NO_OP": "No-ops",
  "move.planKind.BLOCKED": "Blocked",
  "move.planKind.UNMANAGED_CONTENT": "Unmanaged content",
  "move.planKind.WARNING": "Warnings",
  "move.operationState.QUEUED": "Queued",
  "move.operationState.PREPARING": "Preparing",
  "move.operationState.MOVING": "Moving",
  "move.operationState.VERIFYING": "Verifying",
  "move.transferDismiss": "Dismiss",
  "move.operationState.RECONCILING": "Finalizing",
  "move.operationState.CLEANING_UP": "Finalizing",
  "move.operationState.COMPLETED": "Completed",
  "move.operationState.COMPLETED_WITH_WARNINGS": "Completed with warnings",
  "move.operationState.CANCELED": "Canceled",
  "move.operationState.FAILED": "Failed",
  "move.checkpointState.PENDING": "Pending",
  "move.checkpointState.MOVING": "Moving",
  "move.checkpointState.VERIFYING": "Verifying",
  "move.checkpointState.RECONCILING": "Finalizing",
  "move.checkpointState.CLEANING_UP": "Finalizing",
  "move.checkpointState.COMPLETED": "Completed",
  "move.checkpointState.COMPLETED_WITH_WARNINGS": "Completed with warnings",
  "move.checkpointState.SKIPPED": "Skipped",
  "move.checkpointState.BLOCKED": "Blocked",
  "move.checkpointState.FAILED": "Failed",
  "move.operationType.TITLE_FOLDER_RECOVERY":
    "Title folder recovery (originals preserved)",
  "move.operationType.FOLDER_REASSIGNMENT": "Folder reassignment",
  "move.operationType.ROOT_MOVE": "Root move",
  "move.operationType.ROOT_CHANGE": "Root change",
  "move.operationType.ROOT_CONSOLIDATION": "Root consolidation",
  "move.operationType.CROSS_LIBRARY_TRANSFER": "Cross-library transfer",
  "move.toastPreparing": "Preparing…",
  "move.toastTransferred": "Transferred",
  "move.toastTitlesProgress": "{{current}} / {{total}} titles",
  "move.toastDoneSubtitle": "Move complete · {{count}} titles",
  "move.toastWarningsSubtitle": "Completed with warnings · {{count}} titles",
  "move.toastWarningsChip": "Some titles need a look in Activity.",
  "move.toastFailedSubtitle": "Move failed",
  "move.toastCanceledSubtitle": "Move canceled",
  "move.startedHeading": "The move started. Follow it in Activity.",
  "move.viewInActivity": "View in Activity",
  "rootChange.rowAction": "Change root",
  "rootChange.unavailableWhileUnsaved":
    "Save or discard the pending root edits first.",
  "rootChange.dialogTitle": "Change root",
  "rootChange.dialogDescription":
    "Everything on {path} moves. A root-wide operation cannot leave a title behind.",
  "rootChange.destinationHeading": "Where this root goes",
  "rootChange.destinationNewPath": "A new path",
  "rootChange.destinationNewPathHelp":
    "The root keeps its identity and its title assignments; only its path changes.",
  "rootChange.destinationExistingRoot": "Another root of this library",
  "rootChange.destinationExistingRootHelp":
    "This root is folded into the one you pick, and its configuration is retired.",
  "rootChange.destinationNoOtherRoots":
    "This library has no other root to fold into.",
  "rootChange.destinationPathPlaceholder": "/mnt/new-disk/movies",
  "rootChange.planChanged":
    "The plan changed since you previewed it. Preview again before confirming.",
  "rootChange.previewFailed": "Failed to preview this root change.",
  "rootChange.previewing": "Reading the whole root.",
  "rootChange.preview": "Preview",
  "rootChange.confirm": "Start",
  "rootChange.startFailed": "Failed to start this root change.",
  "rootChange.startedHeading":
    "The root change started. Follow it in Activity.",
  "rootChange.viewInActivity": "View in Activity",
  "rootChange.identityKeepsRoot":
    "This root keeps its identity and all {titles} of its title assignments.",
  "rootChange.identityKeepsDefault": "It stays this library's default root.",
  "rootChange.identityLosesDefault":
    "It is no longer this library's default root.",
  "rootChange.consolidationStatement":
    "All {titles} titles on this root move onto the root you picked.",
  "rootChange.consolidationRetiresConfiguration":
    "The source root's configuration is retired once everything has landed and all recycling has finished.",
  "rootChange.defaultTransfers":
    "The destination root becomes this library's default, so new content lands there.",
  "rootChange.defaultStays": "This library's default root does not change.",
  "rootChange.accountingHeading": "Every title on this root",
  "rootChange.accountingAssigned": "Assigned",
  "rootChange.accountingRelocating": "Moving",
  "rootChange.accountingCatalogOnly": "Catalog only",
  "rootChange.accountingBlocked": "Blocked",
  "rootChange.accountingDoesNotClose":
    "The counts do not add up to the total. Do not confirm this plan.",
  "rootChange.blockedTitlesHeading":
    "{count} titles cannot enter this operation.",
  "rootChange.blockedTitlesNoExclude":
    "Repair these before starting. A root-wide operation cannot leave a title behind.",
  "rootChange.groupsHeading": "What this consolidation does",
  "rootChange.group.movingIntoUnusedFolders": "Into free folders",
  "rootChange.group.mergingWithDestinationTitles": "Merging",
  "rootChange.group.folderNameCollisions": "Folder name taken",
  "rootChange.group.mediaCollisions": "Media name taken",
  "rootChange.group.dedupEligibleFiles": "May be identical",
  "rootChange.group.companionCollisions": "Companion name taken",
  "rootChange.group.untrackedSourceEntries": "Untracked at the source",
  "rootChange.renamedFoldersHeading": "Folders that change name",
  "rootChange.renamedFoldersSampled": "Only the first folders are listed here.",
  "rootChange.contentHeading": "What is on this root",
  "rootChange.contentManaged": "Tracked media",
  "rootChange.contentCompanions": "Companion files",
  "rootChange.contentUnknown": "Unexplained",
  "rootChange.contentSampled": "Only the first entries are listed here.",
  "rootChange.unknownContentHelp":
    "{bytes} of content nothing tracks. It is never moved, never deleted, and it keeps the old location standing.",
  "rootChange.retirementHeading": "The old location afterwards",
  "rootChange.retirementEmptyDirectoriesOnly":
    "Only empty directories are removed: {count} of them.",
  "rootChange.retirementVerificationFirst":
    "Nothing at the source is removed before its destination copy is verified.",
  "rootChange.retirementRetainedHeading": "Left standing with their contents:",
  "rootChange.retirementBlockedHeading":
    "The old location stays until these are resolved:",
  "rootChange.retirementPermitted":
    "Nothing blocks the old location from being retired.",
  "rootChange.verificationStatement":
    "{files} files ({bytes}) are verified in full at the destination before any source is recycled.",
  "rootChange.insufficientSpace":
    "The destination does not have the {required} this needs.",
  "rootChange.typedConfirmationPrompt": "Type the phrase to confirm",
  "rootChange.reason.root_identity_retained":
    "The root keeps its identity; only its path changes.",
  "rootChange.reason.catalog_only_root_change":
    "No tracked files, so this is a catalog reassignment.",
  "rootChange.reason.title_blocked_for_root_change":
    "This title cannot enter the root change yet.",
  "rootChange.reason.unknown_root_content":
    "Content at the root that no title accounts for.",
  "rootChange.reason.source_retirement_blocked":
    "The old location cannot be retired yet.",
  "rootChange.reason.file_outside_title_folder":
    "A tracked file that sits outside its title's folder.",
  "rootChange.reason.hardlinked_source":
    "The source file has other hard links, which do not follow the move.",
  "rootChange.reason.roots_consolidated":
    "Every title on this root moves onto the destination root.",
  "rootChange.reason.default_root_transferred":
    "The destination root becomes this library's default.",
  "rootChange.reason.moves_into_unused_folder":
    "Moves into a folder nothing occupies.",
  "rootChange.reason.merges_with_destination_title":
    "Merges with a destination title it shares an identity with.",
  "rootChange.reason.folder_name_uniqued":
    "The folder name is taken at the destination, so this one is renamed.",
  "rootChange.reason.catalog_only_consolidation":
    "No tracked files, so this is a catalog reassignment.",
  "rootChange.reason.destination_folder_exists":
    "The destination folder already exists.",
  "rootChange.retirementBlocker.blocked_titles":
    "Some titles cannot enter the operation.",
  "rootChange.retirementBlocker.unexplained_source_content":
    "Content at the source that nothing accounts for.",
  "rootChange.refusal.root_change_path_not_absolute": "Give an absolute path.",
  "rootChange.refusal.root_change_paths_overlap":
    "The new path is inside the old one, or the other way round.",
  "rootChange.refusal.root_change_source_root_is_symlink":
    "This root is a symlink. Point it at the real directory first.",
  "rootChange.refusal.root_change_source_root_unavailable":
    "This root is not readable right now, so its contents cannot be planned.",
  "rootChange.refusal.root_change_destination_not_empty":
    "The new path already holds content. Choose an empty or a new directory.",
  "rootChange.refusal.root_change_destination_parent_missing":
    "The new path's parent directory does not exist.",
  "rootChange.refusal.root_change_destination_is_configured_root":
    "That path is already configured as a root of another library.",
  "rootChange.refusal.root_change_destination_conflicts_with_recycle_bin":
    "That path overlaps the custom recycle bin: it must neither contain the recycle bin nor sit inside it.",
  "rootChange.refusal.root_change_mode_not_supported":
    'A root change always moves the files; "files are already there" is not offered here.',
  "rootChange.refusal.root_consolidation_path_not_absolute":
    "Give an absolute path.",
  "rootChange.refusal.root_consolidation_same_root":
    "A root cannot be folded into itself.",
  "rootChange.refusal.root_consolidation_paths_overlap":
    "One of these roots is inside the other.",
  "rootChange.refusal.root_consolidation_source_root_is_symlink":
    "This root is a symlink. Point it at the real directory first.",
  "rootChange.refusal.root_consolidation_source_root_unavailable":
    "This root is not readable right now, so its contents cannot be planned.",
  "rootChange.refusal.root_consolidation_destination_root_unavailable":
    "The destination root is not readable right now, so what it already holds cannot be planned against.",
  "rootChange.refusal.root_consolidation_mode_not_supported":
    'A consolidation always moves the files; "files are already there" is not offered here.',
  "indexerSearch.searchWithThisIndexer": "Search with this indexer",
  "indexerSearch.queryPlaceholder": "Search every indexer for a release…",
  "indexerSearch.kindLabel": "Kind",
  "indexerSearch.kind.movie": "Movie",
  "indexerSearch.kind.series": "Series",
  "indexerSearch.kind.anime": "Anime",
  "indexerSearch.kind.raw": "Raw",
  "indexerSearch.scope.indexers": "Indexers",
  "indexerSearch.scope.indexersAll": "All ({{count}})",
  "indexerSearch.scope.indexersSome": "{{count}} selected",
  "indexerSearch.scope.indexersAllOption": "All indexers",
  "indexerSearch.scope.categories": "Categories",
  "indexerSearch.scope.categoriesDefault": "All categories",
  "indexerSearch.scope.categoriesHelp":
    "Comma-separated Newznab category ids. Leave empty to use the defaults for the selected kind.",
  "indexerSearch.advanced": "Advanced",
  "indexerSearch.advanced.minSize": "Min size",
  "indexerSearch.advanced.maxSize": "Max size",
  "indexerSearch.advanced.minSeeders": "Min seeders",
  "indexerSearch.advanced.maxAge": "Max age",
  "indexerSearch.advanced.limit": "Upstream page size",
  "indexerSearch.page.next": "Next page",
  "indexerSearch.page.localSort":
    "Sorting and filters apply to this page. Each new page follows the server's newest-first order.",
  "indexerSearch.page.pending":
    "Some indexers are still searching. Continue to check for more results.",
  "indexerSearch.error.capacity":
    "The search cache is full. Start a new search after other searches finish.",
  "indexerSearch.error.cursor":
    "An indexer repeated its page cursor and cannot continue this search.",
  "indexerSearch.error.offset":
    "This search continuation is invalid. Start the search again.",
  "indexerSearch.error.cancelled": "This search was cancelled.",
  "indexerSearch.error.expired":
    "This page has expired. Start the search again.",
  "indexerSearch.error.limited":
    "All selected indexers are cooling down. Try again later.",
  "indexerSearch.error.unavailable":
    "No selected indexer could answer this search.",
  "indexerSearch.unit.gib": "GiB",
  "indexerSearch.unit.days": "days",
  "indexerSearch.unit.perIndexer": "per indexer",
  "indexerSearch.saveSearch": "Saved",
  "indexerSearch.saveCurrentSearch": "Save current search",
  "indexerSearch.savedSearchesEmpty": "No saved searches yet.",
  "indexerSearch.health.releases": "{{count}} releases",
  "indexerSearch.health.indexers": "{{answered}}/{{total}} indexers",
  "indexerSearch.health.elapsed": "{{seconds}}s",
  "indexerSearch.health.stillSearching":
    "{{count}} of {{total}} still searching",
  "indexerSearch.health.failed": "failed",
  "indexerSearch.health.coolingDown": "cooling down",
  "indexerSearch.health.partial": "{{count}} · partial",
  "indexerSearch.health.skipped": "skipped",
  "indexerSearch.health.retryFailed": "Retry failed",
  "indexerSearch.skipReason.indexerDisabled": "Indexer is disabled",
  "indexerSearch.skipReason.temporarilyDisabled": "Temporarily disabled",
  "indexerSearch.skipReason.temporarilyDisabledUntil":
    "Temporarily disabled until {{time}}",
  "indexerSearch.skipReason.backedOff": "In backoff after recent failures",
  "indexerSearch.skipReason.backedOffUntil": "In backoff until {{time}}",
  "indexerSearch.skipReason.noTextSearch": "Does not support text search",
  "indexerSearch.caps.fresh": "Capabilities from cached snapshot",
  "indexerSearch.caps.managed": "Capabilities from managed indexer",
  "indexerSearch.caps.refreshed": "Capabilities refreshed",
  "indexerSearch.caps.stale": "Using previous capabilities",
  "indexerSearch.caps.permissive":
    "Capabilities unknown; attempting requested search",
  "indexerSearch.caps.provider": "Capabilities declared by provider",
  "indexerSearch.caps.unknown": "Capability source unavailable",
  "indexerSearch.caps.refreshFailed": "Capability refresh failed",
  "indexerSearch.caps.refreshDeferred": "Capability refresh deferred",
  "indexerSearch.caps.refreshUnavailable": "Capability refresh unavailable",
  "indexerSearch.caps.retryAfter": "Provider retry delay: {{seconds}} seconds",
  "indexerSearch.refine.title": "Refine",
  "indexerSearch.refine.reset": "Reset",
  "indexerSearch.facet.protocol": "Protocol",
  "indexerSearch.facet.resolution": "Resolution",
  "indexerSearch.facet.source": "Source",
  "indexerSearch.facet.audio": "Audio & HDR",
  "indexerSearch.facet.flags": "Flags",
  "indexerSearch.facet.indexer": "Indexer",
  "indexerSearch.facet.size": "Size",
  "indexerSearch.facet.usenet": "Usenet",
  "indexerSearch.facet.torrent": "Torrent",
  "indexerSearch.facet.atmos": "Atmos",
  "indexerSearch.facet.dolbyVision": "Dolby Vision",
  "indexerSearch.facet.hdr": "HDR",
  "indexerSearch.facet.freeleech": "Freeleech",
  "indexerSearch.facet.uploadFactor": "{{factor}}× upload",
  "indexerSearch.facet.proper": "Proper",
  "indexerSearch.sizeRange": "{{min}} – {{max}} GiB",
  "indexerSearch.results.matched": "{{count}} shown",
  "indexerSearch.sort.label": "Sort",
  "indexerSearch.column.release": "Release",
  "indexerSearch.column.indexer": "Indexer",
  "indexerSearch.column.size": "Size",
  "indexerSearch.column.age": "Age",
  "indexerSearch.column.peers": "Peers / Grabs",
  "indexerSearch.row.select": "Select release",
  "indexerSearch.row.grab": "Grab",
  "indexerSearch.row.download": "Download file",
  "indexerSearch.download.unavailable":
    "This release is a magnet link and has no file to download",
  "indexerSearch.row.expand": "Show details",
  "indexerSearch.row.collapse": "Hide details",
  "indexerSearch.row.grabs": "{{count}} grabs",
  "indexerSearch.row.swarm":
    "{{seeders}} seeders · {{leechers}} leechers · {{peers}} peers",
  "indexerSearch.protocol.nzb": "NZB",
  "indexerSearch.protocol.torrent": "Torrent",
  "indexerSearch.protocol.http": "HTTP",
  "indexerSearch.detail.title": "Release detail",
  "indexerSearch.detail.publishDate": "Published",
  "indexerSearch.detail.releaseGroup": "Release group",
  "indexerSearch.detail.openOnIndexer": "Open on indexer",
  "indexerSearch.detail.notAvailable": "Not reported",
  "indexerSearch.detail.leechers": "Leechers",
  "indexerSearch.detail.volumeFactors": "Download / upload factor",
  "indexerSearch.detail.minimumSeedRatio": "Minimum seed ratio",
  "indexerSearch.detail.minimumSeedTime": "Minimum seed time",
  "indexerSearch.age.hours": "{{count}} h",
  "indexerSearch.age.days": "{{count}} d",
  "indexerSearch.age.months": "{{count}} mo",
  "indexerSearch.age.years": "{{count}} y",
  "indexerSearch.footer.selected": "{{count}} selected",
  "indexerSearch.footer.nothingQueued": "Nothing is queued until you grab.",
  "indexerSearch.footer.grabSelected": "Grab selected",
  "indexerSearch.footer.downloadSelected": "Download {{count}}",
  "indexerSearch.empty.prompt": "Enter a query and search your indexers.",
  "indexerSearch.empty.searching": "Searching indexers…",
  "indexerSearch.empty.noResults": "No releases matched.",
  "grabDialog.title.one": "Grab release",
  "grabDialog.title.many": "Grab {{count}} releases",
  "grabDialog.subtitle":
    "Send releases to a download client, with an optional library title assignment.",
  "grabDialog.summary.mix": "MIX",
  "grabDialog.summary.mixed": "{{count}} selected releases",
  "grabDialog.assign.label": "Assign to title",
  "grabDialog.assign.placeholder": "Search your library titles…",
  "grabDialog.assign.count": "{{shown}} of {{total}} titles",
  "grabDialog.assign.loading": "Searching titles…",
  "grabDialog.assign.empty": "No library title matched.",
  "grabDialog.assign.unlinked":
    "Grab without linking — straight to the client, match later in Import",
  "grabDialog.gap.missing": "{{count}} missing",
  "grabDialog.gap.complete": "Complete",
  "grabDialog.gap.wanted": "Wanted",
  "grabDialog.season": "Season",
  "grabDialog.episode": "Episode",
  "grabDialog.episodic.help":
    "Leave both blank and Scryer reads the season and episode from the release name.",
  "grabDialog.episodic.incomplete":
    "Enter both a season and an episode, or leave both blank.",
  "grabDialog.assign.suggested": "Suggested titles",
  "grabDialog.routing.loading": "Resolving download clients…",
  "grabDialog.routing.single": "1 release",
  "grabDialog.routing.group": "{{count}} releases",
  "grabDialog.category": "Category",
  "grabDialog.category.default": "Client default",
  "grabDialog.footer.partial": "{{count}} submitted",
  "grabDialog.cta.grab": "Grab",
  "grabDialog.client": "Download client",
  "grabDialog.client.routed": "By indexer mapping",
  "grabDialog.client.placeholder": "Choose a download client",
  "grabDialog.client.none": "No download client is enabled.",
  "grabDialog.option.replace": "Replace the existing file with this release",
  "grabDialog.option.acknowledge":
    "I understand this release is rejected by {{codes}}",
  "grabDialog.footer.linked": "Selected title: {{name}}",
  "grabDialog.footer.unlinked":
    "Nothing will be linked — match it later in Import",
  "grabDialog.footer.pickTitle":
    "Pick a library title, or grab without linking.",
  "grabDialog.cta.assign": "Grab & Assign",
  "grabDialog.cta.assignAll": "Grab & assign all",
  "grabDialog.cta.unlinked": "Grab unlinked",
  "grabDialog.conflict": "{{name}} already has a download in progress.",
  "grabDialog.status.unlinked": "Sent {{name}} to {{client}}",
  "grabDialog.status.failed": "{{name}}: {{reason}}",
  "grabDialog.error.expired":
    "That release is no longer in a live search. Run the search again.",
  "grabDialog.error.additionalScope":
    "{{name}} covers more than one episode, so it cannot be queued as an additional file. Only a movie or a single episode can.",
  // Title tags: the administrator-defined registry in Settings, the per-title
  // picker, the bulk pickers, and the catalog filter.
  "settings.titleTags": "Tags",
  "settings.titleTagsExisting": "Defined tags",
  "settings.titleTagsHelp":
    "Tags defined here are the only tags anyone can apply to a title. Renaming one rewrites every title and delay profile that carries it; deleting one strips it from every title.",
  "settings.titleTagsNone": "No tags are defined yet.",
  "settings.titleTagsLoadError": "Tags could not be loaded.",
  "settings.titleTagLabelLabel": "Label",
  "settings.titleTagLabelPlaceholder": "keep",
  "settings.titleTagLabelHelp":
    "Stored in lowercase with extra spaces removed. Up to 64 characters.",
  "settings.titleTagDescriptionLabel": "Description",
  "settings.titleTagDescriptionPlaceholder": "What this tag is for (optional)",
  "settings.titleTagTitleCount": "Titles",
  "settings.titleTagCreate": "Define tag",
  "settings.titleTagCreateNew": "Define a new tag",
  "settings.titleTagEdit": "Edit tag",
  "settings.titleTagSaved": "Tag saved.",
  "settings.titleTagSaveError": "The tag could not be saved.",
  "settings.titleTagDeleteError": "The tag could not be deleted.",
  "settings.titleTagDeleted": "Deleted {{label}} from {{count}} titles.",
  "settings.titleTagDeleteConfirmTitle": "Delete tag",
  "settings.titleTagDeleteConfirm":
    "{{label}} is on {{count}} titles. Deleting it removes it from every one of them and from any delay profile that filters on it. Rules that name it stop matching.",
  "settings.titleTagLabelRequired": "Enter a label for the tag.",
  "settings.titleTagLabelReserved":
    "Labels cannot start with scryer:, which is reserved for settings Scryer writes itself.",
  "settings.titleTagLabelTooLong": "A label can be at most 64 characters.",
  "settings.titleTagLabelInvalid": "A label cannot contain control characters.",
  "settings.titleTagRenameSummary":
    "Renamed to {{label}}. Rewrote {{titles}} titles, {{seriesMovies}} series movies, and {{delayProfiles}} delay profiles.",
  "settings.titleTagRenameWarning":
    "Rule sources are never rewritten, so {{count}} rule references still name the old label {{label}} and will stop matching: {{references}}.",
  "settings.titleTagReferenceMaintenanceRuleSets":
    "{{count}} maintenance rule sets",
  "settings.titleTagReferenceReleaseRuleSets": "{{count}} release rule sets",
  "settings.titleTagReferenceManagedTagFilters":
    "{{count}} managed tag filters",
  "settings.titleTagReferenceRequestRuleSets": "{{count}} request rule sets",
  "title.tagsLabel": "Tags",
  "title.tagsNone": "No tags",
  "title.tagsAdd": "Add a tag…",
  "title.tagsAllApplied": "Every defined tag is already applied",
  "title.tagsRemove": "Remove {{label}}",
  "title.tagsEmptyRegistry":
    "No tags are defined yet. An administrator defines them in Settings > Tags.",
  "title.tagsNoneDefined": "No tags are defined yet",
  "title.tagsCreateMore": "Create more tags…",
  "title.bulkAddTags": "Add tags",
  "title.bulkRemoveTags": "Remove tags",
  "title.catalogFilters.themes": "Themes",
  "title.catalogFilters.selectThemes": "Select themes...",
  "title.catalogFilters.userTags": "Tags",
  "title.catalogFilters.selectUserTags": "Select tags...",
  "title.catalogFilters.files": "Files",
  "title.catalogFilters.incompleteFiles": "Missing or partial",
  "title.catalogFilters.onDiskFiles": "Complete or needs attention",
  "title.catalogFilters.presenceMissing": "Missing",
  "title.catalogFilters.presenceMissingHint": "Nothing on disk at all.",
  "title.catalogFilters.presencePartial": "Partial",
  "title.catalogFilters.presencePartialHint":
    "Some files on disk and some monitored episodes that have aired without one. Movies hold a single file, so they are never partial.",
  "title.catalogFilters.presenceComplete": "Complete",
  "title.catalogFilters.presenceCompleteHint":
    "Everything monitored is on disk: every monitored episode that has aired has a file, or the movie's own file is present.",
  "title.catalogFilters.needsAttention": "Needs attention",
  "title.catalogFilters.needsAttentionHint":
    "Holds a file whose scan failed or that needs review, so something has to be looked at before it is usable.",

  // Series-movie tags and the maintenance tag actions (WP4). The long-form
  // sentences carry the English string where no translation is available yet,
  // the same way the WP3 block above does.
  "settings.titleTagSeriesMovieCount": "Series movies",
  "settings.maintenanceActionAddTags": "Add tags",
  "settings.maintenanceActionRemoveTags": "Remove tags",
  "settings.maintenanceRuleTags": "Tags",
  "settings.maintenanceRuleTagsNone": "No tags picked yet",
  "settings.maintenanceRuleTagsHelp":
    "The rule writes these tags on every title it matches. Only tags an administrator has defined in Settings > Tags can be picked.",
  "settings.maintenanceCandidatesTagPatchAdd": "adds {{tags}}",
  "settings.maintenanceCandidatesTagPatchRemove": "removes {{tags}}",
  "settings.maintenanceTemplateTaggedForRemovalTitle": "Tagged for removal",
  "settings.maintenanceTemplateTaggedForRemovalDescription":
    'Caution: this template deletes the title and its files. It matches anything carrying the "remove" tag, after a week\'s grace, so tagging a title by hand is the whole trigger and untagging it inside the week calls the deletion off. Define "remove" in Settings > Tags first.',
  "settings.maintenanceTemplateFlagForReviewTitle": "Flag for review",
  "settings.maintenanceTemplateFlagForReviewDescription":
    "Adds the needs-review tag to anything imported more than a year ago, so a person can decide what to do with it. Nothing is deleted.",

  "mediaFile.allInfo": "Info",
  "mediaFile.noAnalysis": "No native analysis recorded for this file yet.",
  "mediaInfo.sectionFile": "File",
  "mediaInfo.sectionVideo": "Video",
  "mediaInfo.sectionAudio": "Audio tracks",
  "mediaInfo.sectionSubtitles": "Subtitle tracks",
  "mediaInfo.sectionCaptions": "Caption services",
  "mediaInfo.sectionRelease": "Release",
  "mediaInfo.sectionChapters": "Chapters",
  "mediaInfo.sectionAttachments": "Attachments",
  "mediaInfo.path": "Path",
  "mediaInfo.size": "Size",
  "mediaInfo.container": "Container",
  "mediaInfo.duration": "Duration",
  "mediaInfo.chapterCount": "Chapters",
  "mediaInfo.scanStatus": "Scan status",
  "mediaInfo.role": "Role",
  "mediaInfo.added": "Added",
  "mediaInfo.originalPath": "Original path",
  "mediaInfo.releaseHash": "Release hash",
  "mediaInfo.resolution": "Resolution",
  "mediaInfo.videoCodec": "Codec",
  "mediaInfo.profile": "Profile",
  "mediaInfo.bitDepth": "Bit depth",
  "mediaInfo.hdr": "HDR",
  "mediaInfo.threeD": "3D",
  "mediaInfo.frameRate": "Frame rate",
  "mediaInfo.videoBitrate": "Video bitrate",
  "mediaInfo.pixelFormat": "Pixel format",
  "mediaInfo.fieldOrder": "Field order",
  "mediaInfo.aspectRatio": "Aspect ratio",
  "mediaInfo.rotation": "Rotation",
  "mediaInfo.color": "Color primaries / transfer / matrix",
  "mediaInfo.contentLight": "Content light MaxCLL / MaxFALL",
  "mediaInfo.masteringDisplay": "Mastering display luminance",
  "mediaInfo.dolbyVision": "Dolby Vision profile",
  "mediaInfo.sceneName": "Scene name",
  "mediaInfo.releaseGroup": "Release group",
  "mediaInfo.sourceType": "Source",
  "mediaInfo.parsedResolution": "Parsed resolution",
  "mediaInfo.parsedVideoCodec": "Parsed video codec",
  "mediaInfo.parsedAudioCodec": "Parsed audio codec",
  "mediaInfo.edition": "Edition",
  "mediaInfo.indexer": "Indexer",
  "mediaInfo.grabbedTitle": "Grabbed release",
  "mediaInfo.grabbedAt": "Grabbed at",
  "mediaInfo.acquisitionScore": "Acquisition score",
  "mediaInfo.scoringLog": "Scoring log",
  "mediaInfo.sectionReleaseListing": "Release listing",
  "mediaInfo.listingPublishedAt": "Published",
  "mediaInfo.listingAgeAtGrab": "Age at grab",
  "mediaInfo.listingAgeDay": "{{count}} day",
  "mediaInfo.listingAgeDays": "{{count}} days",
  "mediaInfo.listingThumbsUp": "Votes up",
  "mediaInfo.listingThumbsDown": "Votes down",
  "mediaInfo.listingPasswordProtected": "Password protected",
  "mediaInfo.listingIndexerLanguages": "Indexer languages",
  "mediaInfo.listingCapturedAt": "Captured at",
  "mediaInfo.colIndex": "#",
  "mediaInfo.colLanguage": "Language",
  "mediaInfo.colCodec": "Codec",
  "mediaInfo.colProfile": "Profile",
  "mediaInfo.colChannels": "Channels",
  "mediaInfo.colBitrate": "Bitrate",
  "mediaInfo.colSampleRate": "Sample rate",
  "mediaInfo.colSampleDepth": "Sample depth",
  "mediaInfo.colRoles": "Roles",
  "mediaInfo.colName": "Name",
  "mediaInfo.inferredLanguageFootnote":
    "Inferred from the track name because the track has no language field.",
  "mediaInfo.colForced": "Forced",
  "mediaInfo.colDefault": "Default",
  "mediaInfo.colStart": "Start",
  "mediaInfo.colEnd": "End",
  "mediaInfo.colTitle": "Title",
  "mediaInfo.colType": "Type",
  "mediaInfo.colSize": "Size",
  "mediaInfo.roleDefault": "Default",
  "mediaInfo.roleForced": "Forced",
  "mediaInfo.roleOriginal": "Original",
  "nav.lists": "Lists",
  "lists.commandDescription":
    "Follow public lists and charts and add their titles automatically.",
  "lists.tab.public": "Public lists",
  "lists.tab.exclusions": "Exclusions",
  "lists.exclusions.commandDescription": "Titles that no list may add.",
  "lists.action.enabled": "Follow {{name}}",
  "lists.action.retry": "Try again",
  "lists.action.syncAll": "Sync all",
  "lists.action.syncNow": "Sync now",
  "lists.action.unfollow": "Unfollow",
  "lists.url.label": "Follow a list by URL",
  "lists.url.placeholder": "Paste a list or chart URL",
  "lists.url.submit": "Check list",
  "lists.url.help": "Paste the address of a list from a supported provider.",
  "lists.url.recognized": "{{provider}} · {{list}}",
  "lists.url.unrecognized":
    "This address isn't a list Scryer can follow. Check the provider catalog below for supported lists.",
  "lists.followed.heading": "Followed lists",
  "lists.followed.empty": "No public lists are being followed yet.",
  "lists.followed.emptyManager":
    "No public lists yet. Paste a list URL above or pick one from the catalog below.",
  "lists.table.list": "List",
  "lists.table.coverage": "Coverage",
  "lists.table.mode": "Mode",
  "lists.table.lastSync": "Last sync",
  "lists.table.neverSynced": "Not synced yet",
  "lists.catalog.heading": "Browse lists",
  "lists.catalog.providers": "List providers",
  "lists.catalog.empty": "No list providers are installed.",
  "lists.catalog.follow": "Follow",
  "lists.catalog.followed": "Followed",
  "lists.catalog.every": "Syncs every {{interval}}",
  "lists.catalog.auth.memberAccount": "Member account",
  "lists.catalog.auth.serverKey": "Server API key",
  "lists.interval.minutes": "{{count}} min",
  "lists.interval.hours": "{{count}} h",
  "lists.interval.days": "{{count}} d",
  "lists.kind.movie": "Movies",
  "lists.kind.series": "Series",
  "lists.kind.anime": "Anime",
  "lists.mode.add": "Add",
  "lists.mode.hold": "Hold for approval",
  "lists.mode.request": "Request",
  "lists.mode.discover": "Only show in Discover",
  "lists.modeHelp.add":
    "Adds new titles with the monitoring chosen for each kind. Monitored titles are searched; unmonitored ones are not.",
  "lists.modeHelp.hold":
    "Creates a pending request for each new title so someone can approve it.",
  "lists.modeHelp.request": "Requests new titles for a member.",
  "lists.modeHelp.discover":
    "Records the list's titles without adding anything.",
  "lists.mode.discoverUnavailable": "Available once lists appear in Discover.",
  "lists.onLeave.keep": "Keep the title",
  "lists.onLeave.log": "Log it",
  "lists.onLeave.unmonitor": "Unmonitor the title",
  "lists.onLeave.tag": "Tag the title",
  "lists.syncState.ok": "Synced",
  "lists.syncState.new": "Not synced yet",
  "lists.syncState.waiting": "Waiting to sync",
  "lists.syncState.fail": "Sync failed",
  "lists.syncState.off": "Paused",
  "lists.membershipState.inLibrary": "In library",
  "lists.membershipState.added": "Added",
  "lists.membershipState.requested": "Requested",
  "lists.membershipState.held": "Held for approval",
  "lists.membershipState.rejected": "Declined",
  "lists.membershipState.filtered": "Filtered",
  "lists.membershipState.excluded": "Excluded",
  "lists.membershipState.unresolved": "Couldn't match",
  "lists.membershipState.blockedPermission": "Not permitted",
  "lists.membershipState.discover": "Discover only",
  "lists.membershipState.pending": "Waiting",
  "lists.reason.mediaTypeNotIncluded":
    "This list is not set to keep this kind of title.",
  "lists.reason.noRoute": "No library is chosen for this kind of title.",
  "lists.reason.duplicateTarget":
    "Another entry on this list is the same title.",
  "lists.reason.specials": "Specials are not kept from this list.",
  "lists.reason.filler": "Left out by the filler setting.",
  "lists.reason.recap": "Left out by the recap setting.",
  "lists.reason.rating": "Its rating is below the minimum.",
  "lists.reason.missingRating": "It has no rating to compare with the minimum.",
  "lists.reason.genre": "It has a genre or theme this list leaves out.",
  "lists.reason.missingGenre":
    "Its genres are not known yet, so the genre filter cannot pass it.",
  "lists.reason.adult": "It is an adult title, which this list leaves out.",
  "lists.reason.missingAdult":
    "Whether it is an adult title is not known yet, so the adult filter cannot pass it.",
  "lists.reason.releaseYear": "Its release year is outside the range.",
  "lists.reason.missingReleaseYear": "Its release year is not known.",
  "lists.reason.format": "Its format is not one this list keeps.",
  "lists.reason.language": "Its original language is not one this list keeps.",
  "lists.reason.missingLanguage": "Its original language is not known.",
  "lists.reason.streamingService": "It is on one of your streaming services.",
  "lists.reason.unreleased": "It has not been released yet.",
  "lists.reason.missingReleaseDate": "Its release date is not known.",
  "lists.reason.directorCredit": "It is not a directing credit.",
  "lists.reason.sequelWithoutBase":
    "It is a sequel to a title that is not in the library.",
  "lists.reason.ambiguousSeriesMovie":
    "It could be more than one movie of a series, so it was not matched.",
  "lists.reason.notPermitted":
    "The list's owner is not allowed to add or request it.",
  "lists.reason.refused": "Adding it was refused.",
  "lists.reason.notFound": "It could not be found when it was added.",
  "lists.reason.actionFailed": "Adding it failed. The next sync tries again.",
  "lists.reason.requestRejected": "The request for it was declined.",
  "lists.runOutcome.succeeded": "Succeeded",
  "lists.runOutcome.failed": "Failed",
  "lists.runOutcome.skipped": "Skipped",
  "lists.counts.inLibrary": "In library",
  "lists.counts.added": "Added",
  "lists.counts.requested": "Requested",
  "lists.counts.held": "Held",
  "lists.counts.filtered": "Filtered",
  "lists.counts.excluded": "Excluded",
  "lists.counts.unresolved": "Couldn't match",
  "lists.counts.empty": "Nothing synced yet",
  "lists.follow.title": "Follow list",
  "lists.follow.editTitle": "Edit list",
  "lists.follow.submit": "Follow list",
  "lists.follow.sourceHeading": "List details",
  "lists.follow.order": "List order",
  "lists.follow.orderHint": "Titles are read from the list in this order.",
  "lists.follow.orderUnset": "Default order",
  "lists.follow.order.rank": "The list's own ranking",
  "lists.follow.order.added": "Date added",
  "lists.follow.order.released": "Release date",
  "lists.follow.name": "Name",
  "lists.follow.kinds": "Include",
  "lists.follow.modeHeading": "When a new title appears",
  "lists.follow.routesHeading": "Where titles go",
  "lists.follow.routesHelp":
    "Each kind of title needs a library. Kinds without one are never added.",
  "lists.follow.filtersHeading": "Filters",
  "lists.follow.syncHeading": "Sync",
  "lists.follow.maxPerSync": "Most titles per sync",
  "lists.follow.maxPerSyncPlaceholder": "No limit",
  "lists.follow.maxPerSyncHelp":
    "Titles over the limit wait for the next sync.",
  "lists.follow.onLeave": "When a title leaves the list",
  "lists.follow.onLeaveHelp":
    "Only applies to titles this list added and no other list still holds. Lists never delete titles.",
  "lists.follow.interval":
    "The provider sets the schedule: every {{interval}}.",
  "lists.follow.problem.name": "Give the list a name.",
  "lists.follow.problem.kinds": "Include at least one kind of title.",
  "lists.follow.problem.mode": "Choose what happens to new titles.",
  "lists.follow.problem.route": "Choose a library for every included kind.",
  "lists.follow.problem.maxPerSync": "The limit must be at least 1.",
  "lists.follow.problem.params": "Fill in the required list details.",
  "lists.route.heading": "{{kind}}",
  "lists.route.noLibrary": "You don't manage a library for this kind of title.",
  "lists.filter.rating": "Minimum rating",
  "lists.filter.ratings": "Ratings",
  "lists.filter.all": "All",
  "lists.filter.any": "At least one",
  "lists.filter.match": "Match rating minimums",
  "lists.filter.canonical": "Excluded genres/themes",
  "lists.filter.searchCanonical": "Search genres and themes…",
  "lists.filter.vocabularyError":
    "Canonical genres and themes are unavailable.",
  "lists.filter.retry": "Retry",
  "lists.filter.unavailable": "Unavailable: {{label}}",
  "lists.filter.unresolved": "Needs correction: {{label}}",
  "lists.filter.excludeAdult": "Exclude adult titles",
  "lists.filter.excludeAdultHelp":
    "Leaves out titles marked as adult. A title whose details are not known yet is left out until they are.",
  "title.tagsCreateSelect": "Create “{{label}}” and select",
  "title.tagsSearch": "Search or create a tag…",
  "lists.filter.ratingScale": "Rating source",
  "lists.filter.ratingPlaceholder": "Any",
  "lists.filter.scale.tmdb": "TMDB",
  "lists.filter.scale.imdb": "IMDb",
  "lists.filter.releaseYear": "Release year",
  "lists.filter.yearFrom": "From",
  "lists.filter.yearTo": "To",
  "lists.filter.excludeGenres": "Skip genres",
  "lists.filter.excludeGenresPlaceholder": "horror, documentary",
  "lists.filter.languages": "Original languages",
  "lists.filter.languagesPlaceholder": "en, ja",
  "lists.filter.releasedOnly": "Released titles only",
  "lists.filter.releasedOnlyHelp":
    "Skip titles that haven't been released yet. They are checked again on every sync.",
  "lists.preview.heading": "Preview",
  "lists.preview.nextSyncHeading": "Next sync would add",
  "lists.preview.run": "Preview",
  "lists.preview.notRun":
    "Preview the list to see what the first sync would add.",
  "lists.preview.failed": "Couldn't preview this list.",
  "lists.preview.total": "On list",
  "lists.preview.inLibrary": "In library",
  "lists.preview.wouldAdd": "Would add",
  "lists.preview.filtered": "Filtered",
  "lists.preview.excluded": "Excluded",
  "lists.preview.unresolved": "No match",
  "lists.preview.unresolvedHelp":
    "{{count}} titles couldn't be matched yet. They are retried on every sync.",
  "lists.preview.wouldAddHeading": "Titles the next sync would add",
  "lists.preview.nothingToAdd": "Nothing new to add right now.",
  "lists.detail.coverage": "Coverage",
  "lists.detail.settings": "Settings",
  "lists.detail.interval": "Schedule",
  "lists.detail.nextSync": "Next sync",
  "lists.detail.noCap": "No limit",
  "lists.detail.openAtProvider": "Open at the provider",
  "lists.detail.pausedUntil":
    "The provider asked Scryer to slow down. Syncing resumes after {{time}}.",
  "lists.detail.titles": "Titles ({{count}})",
  "lists.detail.title": "Title",
  "lists.detail.state": "State",
  "lists.detail.noTitles": "No titles recorded yet.",
  "lists.detail.sourceRank": "Source rank",
  "lists.detail.pageRange": "Entries {{from}}–{{to}} of {{total}}",
  "lists.detail.previousPage": "Previous",
  "lists.detail.nextPage": "Next",
  "lists.detail.history": "Sync history",
  "lists.detail.noRuns": "No syncs yet.",
  "lists.detail.olderRuns": "Show older syncs",
  "lists.detail.runCounts":
    "{{total}} on list · {{added}} added · {{held}} held",
  "lists.unfollow.title": "Unfollow {{name}}?",
  "lists.unfollow.description":
    "Scryer stops syncing this list. Titles stay in your library, and pending requests stay in Requests.",
  "lists.status.followed": "Following {{name}}.",
  "lists.status.updated": "Saved {{name}}.",
  "lists.status.unfollowed": "Stopped following {{name}}.",
  "lists.status.syncQueued": "Syncing {{name}}.",
  "lists.status.syncAllQueued": "Syncing all lists.",
  "lists.refusal.alreadyFollowed": "This list is already followed.",
  "lists.refusal.sourceRequired":
    "Choose a provider and a list, or paste a link.",
  "lists.refusal.linkNotRecognized":
    "That link is not a list Scryer can follow.",
  "lists.refusal.providerNotInstalled": "That list provider is not installed.",
  "lists.refusal.providerUnavailable":
    "That list provider is unavailable right now.",
  "lists.refusal.sourceNotOffered": "That provider does not offer this list.",
  "lists.refusal.chartUnavailable": "That chart is no longer available.",
  "lists.refusal.memberAccountOnly":
    "That list needs a member’s own account and cannot be followed for everyone.",
  "lists.refusal.modeNotAllowed":
    "That action is not available for this kind of list.",
  "lists.refusal.kindsRequired": "Choose at least one kind of title.",
  "lists.refusal.routeLibraryRequired":
    "Choose a library for every kind of title.",
  "lists.refusal.routeLibraryNotFound": "A chosen library no longer exists.",
  "lists.refusal.syncCapInvalid":
    "The most titles per sync must be at least 1.",
  "lists.refusal.disabled": "This list is turned off. Turn it on to sync it.",
  "lists.refusal.exclusionIdsRequired": "An exclusion needs at least one ID.",
  "lists.refusal.exclusionTitleRequired": "An exclusion needs a title.",
  "lists.refusal.accountRequired": "Choose a connected account.",
  "lists.refusal.accountReconnectRequired":
    "Reconnect this account to keep using it.",
  "lists.refusal.accountProviderMismatch":
    "That account belongs to a different provider.",
  "lists.refusal.accountIdentityMissing":
    "The provider did not say whose account this is. Connect again.",
  "lists.refusal.accountIdentityUnverified":
    "The account could not be verified. Reconnect it.",
  "lists.refusal.accountLinkingUnsupported":
    "This provider does not support connecting an account.",
  "lists.refusal.accountLinkOriginInvalid":
    "Account linking cannot start from this address.",
  "lists.refusal.accountLinksTooMany":
    "Too many account connections are waiting to be finished. Finish one or wait for it to expire, then connect again.",
  "lists.refusal.accountLinkResultInvalid":
    "The provider’s answer was not usable. Start the connection again.",
  "lists.refusal.accountLinkIssuerInvalid":
    "The answer did not come from the provider. Start the connection again.",
  "lists.refusal.accountKeyUnsupported":
    "This provider’s accounts are not connected with a key.",
  "lists.refusal.accountKeyRequired":
    "This provider connects with a key of your own, and the installed provider plugin does not ask for one. An administrator needs to update the plugin.",
  "lists.refusal.accountKeyInvalid":
    "That does not look like a key. Paste it as one line, without spaces.",
  "lists.refusal.accountKeyRejected":
    "The provider did not accept that key. Check that you copied all of it, then try again.",
  "lists.refusal.accountAuthUnavailable":
    "Account connections are not available on this server.",
  "lists.refusal.accountAuthAccessDenied":
    "The provider did not grant access. Connect again and approve the request.",
  "lists.refusal.accountAuthAuthorizationFailed":
    "The provider did not accept the approval. Start the connection again.",
  "lists.refusal.accountAuthBusy":
    "Scryer is handling too many account connections right now. Try again in a moment.",
  "lists.refusal.accountAuthInstanceUnrecognized":
    "Scryer’s sign-in service could not confirm this server. Try again in a few minutes. If it keeps happening, an administrator should check that the server’s clock is right and that it can reach the internet.",
  "lists.refusal.accountAuthInvalidProviderApp":
    "This server’s own app for the provider has a callback address that does not match the address Scryer is opened at. An administrator needs to correct it in the provider’s settings.",
  "lists.refusal.accountAuthInvalidProviderResponse":
    "The provider’s answer was not usable. Start the connection again.",
  "lists.refusal.accountAuthInvalidRequest":
    "The connection request was not usable. Start the connection again.",
  "lists.refusal.accountAuthProviderNotConfigured":
    "This provider is not set up for account connections yet. An administrator needs to finish setting the provider up.",
  "lists.refusal.accountAuthProviderRejected":
    "The provider refused the connection. Start the connection again.",
  "lists.refusal.accountAuthProviderUnavailable":
    "The provider is not answering right now. Try again later.",
  "lists.refusal.accountAuthRateLimited":
    "The provider asked Scryer to slow down. Wait a minute, then try again.",
  "lists.refusal.accountAuthReconnectRequired":
    "The provider no longer accepts this sign-in. Start the connection again.",
  "lists.refusal.accountAuthRelayUnavailable":
    "Scryer’s sign-in service is not answering right now. Try again later.",
  "lists.refusal.accountAuthTransportUnavailable":
    "This server could not reach the provider. Check its internet connection, then try again.",
  "lists.refusal.accountAuthUnsupported":
    "This provider’s accounts cannot be connected this way.",
  "lists.refusal.accountAuthUnsupportedProviderApp":
    "This provider does not take an app of your own. An administrator needs to remove it from the provider’s settings.",
  "lists.refusal.providerAppUnsupported":
    "This provider does not take an app of your own.",
  "lists.refusal.providerAppRedirectInvalid":
    "The redirect URI is not a valid URL.",
  "lists.refusal.providerAppIncomplete":
    "A provider app needs a client ID, a secret where the provider requires one, and a valid account callback URI.",
  "lists.exclusions.addHeading": "Exclude a title",
  "lists.exclusions.titleLabel": "Title",
  "lists.exclusions.titlePlaceholder": "Search for a title",
  "lists.exclusions.yearLabel": "Year",
  "lists.exclusions.kindLabel": "Kind",
  "lists.exclusions.idsLabel": "External IDs",
  "lists.exclusions.idsHelp": "Separate with commas, as source:id.",
  "lists.exclusions.scopeLabel": "Applies to",
  "lists.exclusions.scopeAll": "All lists",
  "lists.exclusions.scopeList": "Only {{name}}",
  "lists.exclusions.add": "Exclude",
  "lists.exclusions.added": "{{name}} is excluded from lists.",
  "lists.exclusions.removed": "{{name}} can be added by lists again.",
  "lists.exclusions.empty": "No titles are excluded.",
  "lists.exclusions.created": "Added",
  "lists.exclusions.remove": "Remove exclusion for {{name}}",
  "lists.exclusions.removeTitle": "Remove exclusion for {{name}}?",
  "lists.exclusions.removeDescription":
    "Lists may add this title again on their next sync.",
  "lists.exclusions.deleteCheckbox": "Also exclude from lists",
  "lists.exclusions.deleteNoIds":
    "{{name}} was removed, but it has no external IDs to exclude from lists.",
  "lists.exclusions.deleteFailed":
    "{{name}} was removed, but excluding it from lists failed. Add the exclusion on the Lists page.",
  "lists.exclusions.bulkDeleteNoIds":
    "{{count}} of the removed titles have no external IDs to exclude from lists.",
  "lists.exclusions.bulkDeleteFailed":
    "Excluding {{count}} of the removed titles from lists failed. Add those exclusions on the Lists page.",
  "lists.policy.heading": "Member list policy",
  "lists.policy.help":
    "Controls how members' personal lists turn into requests.",
  "lists.policy.empty": "No members to show.",
  "lists.policy.member": "Member",
  "lists.policy.requests30d": "List requests (30 days)",
  "lists.policy.policy": "Policy",
  "lists.policy.policyFor": "List policy for {{name}}",
  "lists.policy.value.auto": "Auto-approve",
  "lists.policy.value.approval": "Needs approval",
  "lists.policy.value.none": "Lists not allowed",
  "lists.providerSettings.heading": "Provider settings",
  "lists.providerSettings.copy":
    "Shared by every list from this provider on this server.",
  "lists.providerSettings.required": "Required",
  "lists.providerSettings.secretStored": "Stored",
  "lists.providerSettings.secretEmpty": "Not set",
  "lists.providerSettings.secretPlaceholder":
    "Leave blank to keep the stored value",
  "lists.providerSettings.clearSecret": "Clear",
  "lists.providerSettings.keepSecret": "Keep",
  "lists.providerSettings.save": "Save settings",
  "lists.providerSettings.saved": "Provider settings saved.",
  "lists.tab.personal": "Personal",
  "lists.accounts.heading": "Your accounts",
  "lists.accounts.copy":
    "Connect an account to follow your private lists and watchlists. Only you can see your accounts and personal lists.",
  "lists.accounts.connect": "Connect",
  "lists.accounts.installProviders":
    "Install and enable a personal-list provider below, then connect your account here.",
  "lists.plugins.heading": "List-provider plugins",
  "lists.accounts.askForProviders":
    "Ask an administrator to install and enable a list-provider plugin so you can connect your account and follow private lists or watchlists.",
  "profile.listAccounts": "Personal-list accounts",
  "profile.mediaServerAccounts": "Media-server accounts",
  "lists.accounts.connected": "Connected",
  "lists.accounts.reconnect": "Reconnect",
  "lists.accounts.connectAnother": "Connect another account",
  "lists.accounts.connectAnotherHint":
    "The provider connects whichever account is signed in there. To add a different one, sign in to it in the provider window.",
  "lists.accounts.notConnected": "Not connected",
  "lists.accounts.needsReconnect": "Needs reconnecting",
  "lists.accounts.manage": "Manage",
  "lists.accounts.manageTitle": "Manage {{provider}} account",
  "lists.accounts.private":
    "These lists and account details are visible only to you.",
  "lists.accounts.waiting":
    "Finish authorization in the provider window. Approval is checked automatically.",
  "lists.accounts.key.checking": "Checking the key with the provider…",
  "lists.accounts.key.where": "Where to find it",
  "lists.accounts.key.private":
    "Your key is stored encrypted on this server, used only for your own lists, and never shown again.",
  "lists.accounts.key.replace":
    "Saving a new key replaces the one stored for this account.",
  "lists.accounts.key.approval":
    "After the key is accepted, a provider window opens for you to approve the connection.",
  "lists.accounts.key.missing": "Enter your key first.",
  "lists.accounts.linkFailed":
    "Account linking failed. Start again to reconnect.",
  "lists.accounts.pollRetrying":
    "Couldn’t check approval just now. Still trying until the session expires.",
  "lists.accounts.originNotAllowed":
    "Account linking is unavailable from this address. If Scryer is reached through a published container port, a reverse proxy or a hostname, ask an administrator to set Scryer’s public URL; otherwise open Scryer at localhost or the address it listens on.",
  "lists.accounts.linkExpired":
    "This account-link session has expired. Start a new connection from Lists.",
  "lists.accounts.popupBlocked":
    "Allow popups to approve this account, then connect again.",
  "lists.accounts.finishing": "Completing your account connection…",
  "lists.accounts.linked": "Account connected.",
  "lists.accounts.returnToLists": "Return to your personal lists",
  "lists.accounts.noProviders": "No personal-list providers are installed.",
  "lists.accounts.ownedLists": "Your lists",
  "lists.accounts.statuses": "Available statuses: {{statuses}}",
  "lists.accounts.unlink": "Unlink account",
  "lists.accounts.confirmUnlink": "Confirm unlink",
  "lists.accounts.unlinkHelp":
    "Unlinking removes subscriptions using this account. Your library files are preserved.",
  "lists.accounts.unlinked": "Account unlinked.",
  "lists.accounts.personalRouting":
    "Personal lists belong to you. Add and search modes require title-management access to each selected library. Requests follow your list policy and request rules.",
  "lists.providerApps.heading": "Custom OAuth",
  "lists.providerApps.settingsLabel": "Lists · Custom OAuth",
  "lists.providerApps.copy":
    "Default project apps work without registration. Optionally register your own Trakt, AniList or MyAnimeList app to connect directly from this instance.",
  "lists.providerApps.simklDefault": "Simkl uses the default project app.",
  "lists.providerApps.useOwn": "Use your own provider app",
  "lists.providerApps.clientId": "Client ID",
  "lists.providerApps.clientSecret": "Client secret",
  "lists.providerApps.traktSecretHint":
    "Optional for Trakt. Trakt signs members in with PKCE; only legacy Trakt apps need a secret.",
  "lists.providerApps.redirectUri": "Redirect URI",
  "lists.providerApps.redirectHelp":
    "Register this instance's /lists/oauth/callback URL with the provider. Leave the secret blank to keep the stored value.",
  "title.listProvenance.added": "Added by list {{name}}",
  "title.listProvenance.left": "Left the list {{name}}",
  "script.editor.jobNamePlaceholder": "e.g. Nightly cleanup",
  "script.editor.inline": "Inline",
  "script.editor.interpreter": "Interpreter",
  "script.editor.interpreterHelp":
    "The interpreter that runs the script. Set interpreter paths in Settings > Scripts.",
  "script.editor.format": "Format indentation",
  "script.editor.formatHelp":
    "Reindent the script using the selected language (Shift+Alt+F).",
  "script.editor.assistanceHint":
    "Ctrl+Space: suggestions · Tab: indent · Shift+Alt+F: format indentation",
  "script.editor.injectedVariable": "Scryer environment variable",
  "script.editor.snippet": "Language snippet",
  "script.editor.env.SCRYER_METADATA":
    "JSON metadata describing this import or scheduled job. Read it from the environment and parse it as JSON.",
  "script.editor.env.SCRYER_EVENT":
    "The event that started the script: post_import or scheduled_job.",
  "script.editor.env.SCRYER_FILE_PATH": "Full path to the imported media file.",
  "script.editor.env.SCRYER_FACET":
    "The imported title's media category: movie, series, or anime.",
  "script.editor.env.SCRYER_TITLE_NAME": "Name of the imported title.",
  "script.editor.env.SCRYER_TITLE_ID":
    "Scryer identifier of the imported title.",
  "script.editor.env.SCRYER_JOB_ID": "Scryer identifier of the scheduled job.",
  "script.editor.env.SCRYER_JOB_NAME": "Name of the scheduled job.",
  "script.editor.env.SCRYER_RUN_ID": "Identifier of this scheduled job run.",
  "script.editor.env.SCRYER_TRIGGER_SOURCE":
    "How this job run was started, such as manual, startup, or schedule.",
  "script.editor.inlineHelp":
    "The script runs with the language selected above.",
  "script.editor.filePathHelp": "Absolute path to a script on the Scryer host.",
  "script.editor.selectFile": "Select script file",
  "script.editor.fileRequired": "Select a script file before saving.",
  "script.editor.required": "A name and a script are required.",
  "script.editor.schedule": "Schedule",
  "script.editor.blockingHelp":
    "Waits for the script to finish and records its exit code and output.",
  "script.editor.fireAndForgetHelp":
    "Starts the script and records only that it started.",
  "script.language.shell": "Shell",
  "script.language.python": "Python",
  "script.language.powershell": "PowerShell",
  "script.language.batch": "Batch",
  "script.language.go": "Go",
  "script.output.filter": "Output stream",
  "script.output.filter.combined": "Combined",
  "script.output.filter.stdout": "Stdout",
  "script.output.filter.stderr": "Stderr",
  "script.output.stream.stdout": "stdout",
  "script.output.stream.stderr": "stderr",
  "script.schedule.kind": "Runs",
  "script.schedule.kind.manual": "Manual",
  "script.schedule.kind.interval": "Interval",
  "script.schedule.kind.daily": "Daily",
  "script.schedule.kind.weekly": "Weekly",
  "script.schedule.kind.cron": "Cron",
  "script.schedule.runOnStartup": "Also run when Scryer starts",
  "script.schedule.manualHelp": "The job runs only when you choose Run now.",
  "script.schedule.every": "Every",
  "script.schedule.unit": "Unit",
  "script.schedule.unit.minutes": "Minutes",
  "script.schedule.unit.hours": "Hours",
  "script.schedule.unit.days": "Days",
  "script.schedule.minimumInterval": "The shortest interval is one minute.",
  "script.schedule.time": "Time",
  "script.schedule.days": "Days",
  "script.schedule.day.monday": "Mon",
  "script.schedule.day.tuesday": "Tue",
  "script.schedule.day.wednesday": "Wed",
  "script.schedule.day.thursday": "Thu",
  "script.schedule.day.friday": "Fri",
  "script.schedule.day.saturday": "Sat",
  "script.schedule.day.sunday": "Sun",
  "script.schedule.cronExpression": "Cron expression",
  "script.schedule.cronHelp":
    "Five fields: minute, hour, day of month, month, day of week.",
  "script.schedule.cronUnreadable": "This expression can't be described.",
  "script.schedule.invalid": "Invalid schedule",
  "script.schedule.nextRuns": "Next runs",
  "settings.scriptInterpreters.title": "Script interpreters",
  "settings.scriptInterpreters.description":
    "Programs Scryer uses to run scripts written in each language. Leave a field empty to use the program found on the PATH.",
  "settings.scriptInterpreters.python": "Python",
  "settings.scriptInterpreters.powershell": "PowerShell",
  "settings.scriptInterpreters.batch": "Batch",
  "settings.scriptInterpreters.go": "Go",
  "settings.scriptInterpreters.containerHelp":
    "The Scryer container image ships no interpreters. Install the ones you need and point these paths at them.",
  "settings.scriptInterpreters.powershellHelp":
    "PowerShell scripts run with pwsh, which is PowerShell 7.",
  "settings.scriptInterpreters.saved": "Interpreter paths saved",
  "episodeAssign.action": "Reassign",
  "episodeAssign.dialogTitle": "Assign to episode",
  "episodeAssign.dialogDescription":
    "Choose the episode {{name}} belongs to. Scryer renames the file for that episode and moves it into place.",
  "episodeAssign.pendingDescription":
    "Scryer could not tell which episode {{name}} is. Choose the episode and Scryer imports it there.",
  "episodeAssign.unnamed": "this file",
  "episodeAssign.targetLabel": "Episode",
  "episodeAssign.loadingEpisodes": "Loading episodes…",
  "episodeAssign.loadFailed": "Failed to load episodes.",
  "episodeAssign.unchanged": "The file is already assigned to this episode.",
  "episodeAssign.apply": "Assign & Rename",
  "episodeAssign.originalReleaseName": "Original release name",
  "episodeAssign.applying": "Assigning…",
  "episodeAssign.replaceTitle": "This episode already has a file",
  "episodeAssign.replaceDescription":
    "Assigning this file makes it the episode's primary file. The current file stays on disk as an additional file.",
  "episodeAssign.replaceApply": "Replace and assign",
  "episodeAssign.success": "File assigned and renamed.",
  "episodeAssign.failed": "Failed to assign the file.",
  "episodeAssign.seasonGroup": "Season {{season}}",
  "episodeAssign.filterPlaceholder": "Filter episodes…",
  "episodeAssign.filterLabel": "Filter episodes",
  "episodeAssign.listLabel": "Episodes",
  "episodeAssign.noMatches": "No matching episodes.",
  "episodeAssign.selectPlaceholder": "Select an episode…",
  "episodeAssign.refusal.noEpisodes": "Choose at least one episode.",
  "episodeAssign.refusal.outsideTitle":
    "That episode belongs to a different title.",
  "episodeAssign.refusal.primaryConflict":
    "This episode already has a primary file.",
  "episodeAssign.refusal.notEpisodic":
    "Only series and anime files can be assigned to episodes.",
  "episodeAssign.refusal.unsupportedFile":
    "This file cannot be moved between episodes. Series movie files and disc images keep their place.",
  "episodeAssign.refusal.destinationOccupied":
    "Another file already has the name this episode renders. Move or remove it, then try again.",
  "episodeAssign.refusal.renameRefused":
    "Scryer could not name the file for this episode. Check the title's library folder and naming template.",
  "episodeAssign.refusal.renameFailed":
    "Moving the file to its new name failed. The file and its episodes were left as they were.",
  "pendingImports.reason.anime_numbering_ambiguous":
    "Episode numbering is ambiguous",
};

export default en;
