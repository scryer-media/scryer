use super::keys::{
    LEGACY_TOTP_REQUIRE_CONFIG_STEP_UP_KEY, LEGACY_TOTP_REQUIRE_PASSWORD_LOGIN_KEY,
    RENAME_ENABLED_KEY, default_indexer_routing_categories_for_scope,
};
use super::*;
use crate::acquisition_policy::AcquisitionThresholds;
use crate::location::model::VerificationDepth;
use crate::ports::ImportFilePermissions;
use crate::scoring_weights::ScoringPersona;
use crate::subtitles::{normalize_subtitle_language_code, wanted::SubtitleLanguagePref};
use crate::{
    AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
    AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY, AUTO_BACKUP_ENABLED_KEY, AUTO_BACKUP_KEY_KEY,
    BACKUP_PATH_KEY, DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL, FORM_LOGIN_ENABLED_KEY,
    HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY, LibraryRootDraft,
    PLUGIN_AUTO_UPDATE_ENABLED_KEY, PLUGIN_HTTP_CA_BUNDLE_PEM_KEY, RECYCLE_BIN_ENABLED_KEY,
    RECYCLE_BIN_PATH_KEY, RECYCLE_BIN_RETENTION_DAYS_KEY, REQUIRED_AUDIO_LANGUAGES_KEY,
    SCORING_PERSONA_KEY, SETTINGS_SOURCE_TYPED_GRAPHQL, SETUP_COMPLETE_KEY,
    SKIP_LOGIN_FOR_LOCAL_IPS_KEY, TITLE_REQUIRED_AUDIO_OVERRIDE_KEY, USE_SEASON_FOLDERS_KEY,
    VERIFICATION_DEPTH_KEY,
};
use aws_lc_rs::digest as aws_lc_digest;
use regex::Regex;
use rustls_pki_types::{CertificateDer, pem::PemObject};
use scryer_domain::RootFolderEntry;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::{info, warn};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("shared.rs");
include!("subtitles.rs");
include!("acquisition.rs");
include!("general.rs");
include!("security.rs");
include!("recycle_bin.rs");
include!("verification.rs");
include!("library.rs");
include!("media.rs");
include!("quality_profiles.rs");
include!("routing.rs");
include!("external_import_paths.rs");
include!("title_image_cache.rs");
