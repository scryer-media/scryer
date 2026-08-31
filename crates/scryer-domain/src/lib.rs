use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

pub mod download_identity;
mod title_sort;
pub use title_sort::{
    title_catalog_name_tie_key, title_catalog_sort_input, title_catalog_sort_key,
    title_catalog_sort_key_for_title,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Id(pub String);

impl Id {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Generate an ID safe for use as a Rego package segment.
    /// Format: `r` + 32 hex chars (UUID without hyphens).
    pub fn new_rego_safe() -> Self {
        Self(format!("r{}", Uuid::new_v4().to_string().replace('-', "")))
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MediaFacet {
    #[default]
    Movie,
    Series,
    Anime,
}

impl MediaFacet {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            _ => None,
        }
    }
}

pub fn default_library_id_for_facet(facet: &MediaFacet) -> String {
    format!("{}_default_library", facet.as_str())
}

pub fn default_library_slug_for_facet(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "movies",
        MediaFacet::Series => "series",
        MediaFacet::Anime => "anime",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootFolderEntry {
    pub path: String,
    #[serde(rename = "isDefault")]
    pub is_default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: String,
    pub library_id: String,
    pub path: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn is_windows_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn is_unc_share_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() < 3
        || !matches!(bytes[0], b'/' | b'\\')
        || !matches!(bytes[1], b'/' | b'\\')
        || matches!(bytes[2], b'/' | b'\\')
    {
        return false;
    }

    path[2..]
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .count()
        == 2
}

pub fn trim_library_root_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed
        .chars()
        .all(|character| matches!(character, '/' | '\\'))
    {
        return trimmed.chars().next().unwrap_or('/').to_string();
    }
    if is_windows_drive_root(trimmed) || is_unc_share_root(trimmed) {
        return trimmed.to_string();
    }

    trimmed.trim_end_matches(['/', '\\']).to_string()
}

pub fn normalize_library_root_path(path: &str) -> String {
    let stored_path = trim_library_root_path(path);
    if stored_path.is_empty() {
        return stored_path;
    }
    let canonical_path = if is_unc_share_root(&stored_path) {
        stored_path.trim_end_matches(['/', '\\'])
    } else {
        stored_path.as_str()
    };

    #[cfg(windows)]
    {
        canonical_path.replace('/', "\\").to_ascii_lowercase()
    }

    #[cfg(not(windows))]
    {
        canonical_path.replace('\\', "/")
    }
}

/// Prefix marking a library root id as synthetic — allocated, never derived.
pub const SYNTHETIC_ROOT_ID_PREFIX: &str = "root_";

/// Allocate a fresh identity for a newly configured library root.
///
/// A root's id is opaque and permanent: it is minted once, when the root is
/// first configured, and is never recomputed. That is what lets a root's path be
/// a mutable attribute — changing where a root points does not change which root
/// it is, so titles keep valid references across path changes and consolidations
/// (FR-078). Resolving an *existing* root always means reading its stored id, not
/// recomputing one from its path.
pub fn allocate_root_folder_id() -> String {
    format!(
        "{SYNTHETIC_ROOT_ID_PREFIX}{}",
        uuid::Uuid::new_v4().simple()
    )
}

/// The pre-0204 path-derived root id.
///
/// **Legacy.** Root identity was a pure function of the platform-normalized path
/// until migration 0204 re-keyed roots onto synthetic ids. This survives for the
/// migration hooks that have to reproduce the old scheme to recognize and remap
/// it, and for resolving a legacy id a caller may still be holding through
/// `library_root_id_remaps`. New code allocates with [`allocate_root_folder_id`]
/// and resolves existing roots by reading `library_roots.id`.
pub fn root_folder_id_for_path(path: &str) -> String {
    root_folder_id_for_normalized_path(&normalize_library_root_path(path))
}

/// See [`root_folder_id_for_path`] — legacy, migration and lookup use only.
pub fn root_folder_id_for_normalized_path(normalized_path: &str) -> String {
    blake3::hash(normalized_path.as_bytes())
        .to_hex()
        .to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Library {
    pub id: String,
    pub facet: MediaFacet,
    pub name: String,
    pub slug: String,
    pub is_default: bool,
    pub roots: Vec<LibraryRoot>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaRequestStatus {
    Pending,
    Approved,
    Rejected,
    Canceled,
}

impl MediaRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Canceled => "canceled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaRequestRequester {
    pub user_id: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub requested_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaRequest {
    pub id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    pub status: MediaRequestStatus,
    pub identity_fingerprint: String,
    pub title: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
    pub requested_quality_profile_id: Option<String>,
    pub requested_quality_profile_name: Option<String>,
    pub requested_monitor_type: Option<String>,
    pub resolved_by_user_id: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_title_id: Option<String>,
    pub approved_quality_profile_id: Option<String>,
    pub approved_quality_profile_name: Option<String>,
    pub external_ids: Vec<ExternalId>,
    pub requesters: Vec<MediaRequestRequester>,
    pub created_by_user_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AppPermission {
    ManageUsers,
    ManagePermissions,
    ManageSystemSettings,
    ManageCatalogSettings,
}

impl AppPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManageUsers => "manage_users",
            Self::ManagePermissions => "manage_permissions",
            Self::ManageSystemSettings => "manage_system_settings",
            Self::ManageCatalogSettings => "manage_catalog_settings",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "manage_users" => Some(Self::ManageUsers),
            "manage_permissions" => Some(Self::ManagePermissions),
            "manage_system_settings" => Some(Self::ManageSystemSettings),
            "manage_catalog_settings" => Some(Self::ManageCatalogSettings),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActorCapability {
    ManageOwnAccount,
}

impl ActorCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManageOwnAccount => "manage_own_account",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "manage_own_account" => Some(Self::ManageOwnAccount),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPermission {
    View,
    ManageTitles,
    ResolveImports,
    ManageLibrary,
    Request,
    AutoApproveRequests,
}

impl LibraryPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::ManageTitles => "manage_titles",
            Self::ResolveImports => "resolve_imports",
            Self::ManageLibrary => "manage_library",
            Self::Request => "request",
            Self::AutoApproveRequests => "auto_approve_requests",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "view" => Some(Self::View),
            "manage_titles" | "manage_title" => Some(Self::ManageTitles),
            "scan" | "scan_library" => Some(Self::ManageLibrary),
            "resolve_imports" | "resolve_import" => Some(Self::ResolveImports),
            "manage_library" => Some(Self::ManageLibrary),
            "request" | "request_title" => Some(Self::Request),
            "auto_approve_requests" | "auto_approve_request" => Some(Self::AutoApproveRequests),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct AppPermissionMask(u64);

impl AppPermissionMask {
    pub const NONE: Self = Self(0);
    pub const MANAGE_USERS: Self = Self(1 << 0);
    pub const MANAGE_PERMISSIONS: Self = Self(1 << 1);
    pub const MANAGE_SYSTEM_SETTINGS: Self = Self(1 << 2);
    pub const MANAGE_CATALOG_SETTINGS: Self = Self(1 << 3);

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn from_bits_retain(bits: u64) -> Self {
        Self(bits)
    }

    pub fn from_permission(permission: AppPermission) -> Self {
        match permission {
            AppPermission::ManageUsers => Self::MANAGE_USERS,
            AppPermission::ManagePermissions => Self::MANAGE_PERMISSIONS,
            AppPermission::ManageSystemSettings => Self::MANAGE_SYSTEM_SETTINGS,
            AppPermission::ManageCatalogSettings => Self::MANAGE_CATALOG_SETTINGS,
        }
    }

    pub fn from_permissions(permissions: impl IntoIterator<Item = AppPermission>) -> Self {
        permissions
            .into_iter()
            .fold(Self::NONE, |mut mask, permission| {
                mask.insert(Self::from_permission(permission));
                mask
            })
    }

    pub fn to_permissions(self) -> Vec<AppPermission> {
        [
            (Self::MANAGE_USERS, AppPermission::ManageUsers),
            (Self::MANAGE_PERMISSIONS, AppPermission::ManagePermissions),
            (
                Self::MANAGE_SYSTEM_SETTINGS,
                AppPermission::ManageSystemSettings,
            ),
            (
                Self::MANAGE_CATALOG_SETTINGS,
                AppPermission::ManageCatalogSettings,
            ),
        ]
        .into_iter()
        .filter_map(|(mask, permission)| self.contains(mask).then_some(permission))
        .collect()
    }

    pub fn contains(self, required: Self) -> bool {
        (self.0 & required.0) == required.0
    }

    pub fn intersects(self, required: Self) -> bool {
        (self.0 & required.0) != 0
    }

    pub fn insert(&mut self, permission: Self) {
        self.0 |= permission.0;
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ActorCapabilityMask(u64);

impl ActorCapabilityMask {
    pub const NONE: Self = Self(0);
    pub const MANAGE_OWN_ACCOUNT: Self = Self(1 << 0);

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn from_bits_retain(bits: u64) -> Self {
        Self(bits)
    }

    pub fn from_capability(capability: ActorCapability) -> Self {
        match capability {
            ActorCapability::ManageOwnAccount => Self::MANAGE_OWN_ACCOUNT,
        }
    }

    pub fn from_capabilities(capabilities: impl IntoIterator<Item = ActorCapability>) -> Self {
        capabilities
            .into_iter()
            .fold(Self::NONE, |mut mask, capability| {
                mask.insert(Self::from_capability(capability));
                mask
            })
    }

    pub fn to_capabilities(self) -> Vec<ActorCapability> {
        [(Self::MANAGE_OWN_ACCOUNT, ActorCapability::ManageOwnAccount)]
            .into_iter()
            .filter_map(|(mask, capability)| self.contains(mask).then_some(capability))
            .collect()
    }

    pub fn contains(self, required: Self) -> bool {
        (self.0 & required.0) == required.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct LibraryPermissionMask(u64);

impl LibraryPermissionMask {
    pub const NONE: Self = Self(0);
    pub const VIEW: Self = Self(1 << 0);
    pub const MANAGE_TITLES: Self = Self(1 << 1);
    pub const RESOLVE_IMPORTS: Self = Self(1 << 2);
    pub const MANAGE_LIBRARY: Self = Self(1 << 3);
    pub const REQUEST: Self = Self(1 << 4);
    pub const AUTO_APPROVE_REQUESTS: Self = Self(1 << 5);

    pub fn bits(self) -> u64 {
        self.0
    }

    pub fn from_bits_retain(bits: u64) -> Self {
        Self(bits)
    }

    pub fn from_permission(permission: LibraryPermission) -> Self {
        match permission {
            LibraryPermission::View => Self::VIEW,
            LibraryPermission::ManageTitles => Self::MANAGE_TITLES,
            LibraryPermission::ResolveImports => Self::RESOLVE_IMPORTS,
            LibraryPermission::ManageLibrary => Self::MANAGE_LIBRARY,
            LibraryPermission::Request => Self::REQUEST,
            LibraryPermission::AutoApproveRequests => Self::AUTO_APPROVE_REQUESTS,
        }
    }

    pub fn from_permissions(permissions: impl IntoIterator<Item = LibraryPermission>) -> Self {
        permissions
            .into_iter()
            .fold(Self::NONE, |mut mask, permission| {
                mask.insert(Self::from_permission(permission));
                mask
            })
    }

    pub fn to_permissions(self) -> Vec<LibraryPermission> {
        [
            (Self::VIEW, LibraryPermission::View),
            (Self::MANAGE_TITLES, LibraryPermission::ManageTitles),
            (Self::RESOLVE_IMPORTS, LibraryPermission::ResolveImports),
            (Self::MANAGE_LIBRARY, LibraryPermission::ManageLibrary),
            (Self::REQUEST, LibraryPermission::Request),
            (
                Self::AUTO_APPROVE_REQUESTS,
                LibraryPermission::AutoApproveRequests,
            ),
        ]
        .into_iter()
        .filter_map(|(mask, permission)| self.contains(mask).then_some(permission))
        .collect()
    }

    pub fn with_request_shadowing(self) -> Self {
        let mut mask = self;
        if mask.contains(Self::MANAGE_TITLES) {
            mask.insert(Self::AUTO_APPROVE_REQUESTS);
            mask.insert(Self::REQUEST);
        } else if mask.contains(Self::AUTO_APPROVE_REQUESTS) {
            mask.insert(Self::REQUEST);
        }
        mask
    }

    pub fn normalized_for_storage(self) -> Self {
        if self.contains(Self::MANAGE_TITLES) {
            Self(self.0 & !Self::AUTO_APPROVE_REQUESTS.0 & !Self::REQUEST.0)
        } else if self.contains(Self::AUTO_APPROVE_REQUESTS) {
            Self(self.0 & !Self::REQUEST.0)
        } else {
            self
        }
    }

    pub fn is_strictly_requestable(self) -> bool {
        !self.contains(Self::MANAGE_TITLES) && self.with_request_shadowing().contains(Self::REQUEST)
    }

    pub fn can_auto_approve_requests(self) -> bool {
        !self.contains(Self::MANAGE_TITLES)
            && self
                .with_request_shadowing()
                .contains(Self::AUTO_APPROVE_REQUESTS)
    }

    pub fn contains(self, required: Self) -> bool {
        (self.0 & required.0) == required.0
    }

    pub fn intersects(self, required: Self) -> bool {
        (self.0 & required.0) != 0
    }

    pub fn insert(&mut self, permission: Self) {
        self.0 |= permission.0;
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryGrant {
    pub user_id: String,
    pub library_id: String,
    pub permissions: LibraryPermissionMask,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserAuthorization {
    pub app: AppPermissionMask,
    pub libraries: std::collections::HashMap<String, LibraryPermissionMask>,
    pub default_library: LibraryPermissionMask,
    #[serde(default)]
    pub actor_capabilities: ActorCapabilityMask,
    // Keep identity state behind `User` accessors so frozen migration-owned
    // `User` literals never need to change when authentication state evolves.
    #[serde(default)]
    pub login_status: UserLoginStatus,
    pub loaded: bool,
}

impl Default for UserAuthorization {
    fn default() -> Self {
        Self {
            app: AppPermissionMask::NONE,
            libraries: std::collections::HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: ActorCapabilityMask::NONE,
            login_status: UserLoginStatus::Enabled,
            loaded: false,
        }
    }
}

impl UserAuthorization {
    pub fn full_admin() -> Self {
        Self {
            app: AppPermissionMask::from_permissions([
                AppPermission::ManageUsers,
                AppPermission::ManagePermissions,
                AppPermission::ManageSystemSettings,
                AppPermission::ManageCatalogSettings,
            ]),
            libraries: std::collections::HashMap::new(),
            default_library: LibraryPermissionMask::from_permissions([
                LibraryPermission::View,
                LibraryPermission::ManageTitles,
                LibraryPermission::ResolveImports,
                LibraryPermission::ManageLibrary,
                LibraryPermission::Request,
                LibraryPermission::AutoApproveRequests,
            ]),
            actor_capabilities: ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: UserLoginStatus::Enabled,
            loaded: true,
        }
    }

    pub fn library_permissions(&self, library_id: &str) -> LibraryPermissionMask {
        self.libraries
            .get(library_id)
            .copied()
            .unwrap_or(self.default_library)
    }

    pub fn has_app_permission(&self, permission: AppPermission) -> bool {
        self.app
            .contains(AppPermissionMask::from_permission(permission))
    }

    pub fn has_any_app_permission(&self, permissions: AppPermissionMask) -> bool {
        self.app.intersects(permissions)
    }

    pub fn has_library_permission(&self, library_id: &str, permission: LibraryPermission) -> bool {
        match permission {
            LibraryPermission::Request => self
                .library_permissions(library_id)
                .is_strictly_requestable(),
            LibraryPermission::AutoApproveRequests => self
                .library_permissions(library_id)
                .can_auto_approve_requests(),
            _ => self
                .library_permissions(library_id)
                .contains(LibraryPermissionMask::from_permission(permission)),
        }
    }

    pub fn has_any_library_permission(&self, permission: LibraryPermission) -> bool {
        match permission {
            LibraryPermission::Request => {
                self.default_library.is_strictly_requestable()
                    || self
                        .libraries
                        .values()
                        .any(|permissions| permissions.is_strictly_requestable())
            }
            LibraryPermission::AutoApproveRequests => {
                self.default_library.can_auto_approve_requests()
                    || self
                        .libraries
                        .values()
                        .any(|permissions| permissions.can_auto_approve_requests())
            }
            _ => {
                let required = LibraryPermissionMask::from_permission(permission);
                self.default_library.contains(required)
                    || self
                        .libraries
                        .values()
                        .any(|permissions| permissions.contains(required))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalId {
    pub source: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CanonicalMediaTag {
    pub key: String,
    pub category: String,
    pub name: String,
    pub confidence: Option<f64>,
    pub sources: Vec<String>,
    pub source_tag_keys: Vec<String>,
    pub is_adult: bool,
    pub is_spoiler: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaggedAlias {
    pub name: String,
    pub language: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Title {
    pub id: String,
    pub library_id: String,
    pub name: String,
    pub facet: MediaFacet,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Vec<ExternalId>,
    pub root_folder_id: String,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    // rich metadata (hydrated from metadata gateway)
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub poster_source_url: Option<String>,
    pub background_url: Option<String>,
    pub background_source_url: Option<String>,
    pub sort_title: Option<String>,
    pub catalog_sort_key: String,
    pub slug: Option<String>,
    pub imdb_id: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub popularity: Option<f64>,
    pub canonical_tags: Vec<CanonicalMediaTag>,
    pub content_status: Option<String>,
    pub language: Option<String>,
    pub first_aired: Option<String>,
    pub network: Option<String>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub aliases: Vec<String>,
    pub tagged_aliases: Vec<TaggedAlias>,
    pub metadata_language: Option<String>,
    pub metadata_fetched_at: Option<DateTime<Utc>>,
    pub min_availability: Option<String>,
    pub digital_release_date: Option<String>,
    pub folder_path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TitleExternalRating {
    pub source: String,
    pub value: Option<f64>,
    pub score: Option<f64>,
    pub normalized: f64,
    pub votes: Option<i32>,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TitleRatingSummary {
    pub rating: Option<f64>,
    pub rating_sources: Vec<String>,
    pub external_ratings: Vec<TitleExternalRating>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TitleCredit {
    pub kind: String,
    pub person_id: String,
    pub person_name: String,
    pub person_original_name: String,
    pub person_image_url: String,
    pub person_source: String,
    pub person_external_id: String,
    pub character_name: String,
    pub language: String,
    pub billing_order: i32,
    pub episode_count: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MovieEntity {
    pub id: String,
    pub title: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub language: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub content_status: Option<String>,
    pub studio: Option<String>,
    pub digital_release_date: Option<String>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub mal_id: Option<String>,
    pub anidb_id: Option<String>,
    #[serde(default)]
    pub ratings: Option<TitleRatingSummary>,
    #[serde(default)]
    pub credits: Option<Vec<TitleCredit>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SeriesMovieLink {
    pub id: String,
    pub series_title_id: String,
    pub movie: MovieEntity,
    pub placement: Option<String>,
    pub narrative_order: Option<String>,
    pub after_season: Option<i32>,
    pub before_season: Option<i32>,
    pub linked_episode_id: Option<String>,
    pub association_confidence: Option<String>,
    pub continuity_status: Option<String>,
    pub movie_form: Option<String>,
    pub confidence: Option<String>,
    pub signal_summary: Option<String>,
    pub source: Option<String>,
    /// Explicit operator choice; absent values follow title/metadata policy.
    pub monitoring_override: Option<bool>,
    /// False when metadata no longer reports this derived relationship.
    pub metadata_active: bool,
    pub monitored: bool,
    pub legacy_collection_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionType {
    #[default]
    Season,
    Movie,
    Arc,
    Specials,
}

impl CollectionType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Season => "season",
            Self::Movie => "movie",
            Self::Arc => "arc",
            Self::Specials => "specials",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "season" => Some(Self::Season),
            "movie" => Some(Self::Movie),
            "arc" => Some(Self::Arc),
            "specials" => Some(Self::Specials),
            // Legacy persisted rows are migrated to series_movie_links; treat
            // any stragglers as movie-like collections instead of exposing a
            // first-class collection type again.
            "interstitial" => Some(Self::Movie),
            _ => None,
        }
    }
}

impl std::fmt::Display for CollectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Collection {
    pub id: String,
    pub title_id: String,
    pub collection_type: CollectionType,
    pub collection_index: String,
    pub label: Option<String>,
    pub ordered_path: Option<String>,
    pub narrative_order: Option<String>,
    pub first_episode_number: Option<String>,
    pub last_episode_number: Option<String>,
    pub monitored: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeType {
    #[default]
    Standard,
    Special,
    Official,
    Ova,
    Ona,
    Alternate,
}

impl EpisodeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Special => "special",
            Self::Official => "official",
            Self::Ova => "ova",
            Self::Ona => "ona",
            Self::Alternate => "alternate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "standard" => Some(Self::Standard),
            "special" => Some(Self::Special),
            "official" => Some(Self::Official),
            "ova" => Some(Self::Ova),
            "ona" => Some(Self::Ona),
            "alternate" => Some(Self::Alternate),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Episode {
    pub id: String,
    pub title_id: String,
    pub collection_id: Option<String>,
    pub episode_type: EpisodeType,
    pub episode_number: Option<String>,
    pub season_number: Option<String>,
    pub episode_label: Option<String>,
    pub title: Option<String>,
    pub air_date: Option<String>,
    pub duration_seconds: Option<i64>,
    pub has_multi_audio: bool,
    pub has_subtitle: bool,
    pub is_filler: bool,
    pub is_recap: bool,
    pub absolute_number: Option<String>,
    pub overview: Option<String>,
    pub tvdb_id: Option<String>,
    pub image_url: Option<String>,
    pub monitored: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CalendarEpisode {
    pub id: String,
    pub title_id: String,
    pub library_id: String,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    pub title_name: String,
    pub title_slug: Option<String>,
    pub title_facet: String,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub episode_title: Option<String>,
    pub overview: Option<String>,
    pub image_url: Option<String>,
    pub air_date: Option<String>,
    pub monitored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerConfig {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_encrypted: Option<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub disabled_until: Option<DateTime<Utc>>,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub indexer_proxy_config_id: Option<String>,
    pub download_client_id: Option<String>,
    pub seeding_profile_id: Option<String>,
    pub managed_parent_config_id: Option<String>,
    pub managed_child_key: Option<String>,
    pub managed_metadata_json: Option<String>,
    pub caps_snapshot_json: Option<String>,
    pub last_health_status: Option<String>,
    pub last_error_message: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub config_json: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexerProxyProviderType {
    Byparr,
    Trawl,
}

impl IndexerProxyProviderType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Byparr => "byparr",
            Self::Trawl => "trawl",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "byparr" => Some(Self::Byparr),
            "trawl" => Some(Self::Trawl),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeSolverProtocol {
    RequestSolutionV1,
}

impl ChallengeSolverProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequestSolutionV1 => "request_solution_v1",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "request_solution_v1" => Some(Self::RequestSolutionV1),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexerProxyHealthStatus {
    Unknown,
    Healthy,
    Unhealthy,
}

impl IndexerProxyHealthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Unhealthy => "unhealthy",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "unknown" => Some(Self::Unknown),
            "healthy" => Some(Self::Healthy),
            "unhealthy" => Some(Self::Unhealthy),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerProxyConfig {
    pub id: String,
    pub name: String,
    pub provider_type: IndexerProxyProviderType,
    pub protocol: ChallengeSolverProtocol,
    pub base_url: String,
    pub request_timeout_seconds: u32,
    pub is_enabled: bool,
    pub last_health_status: Option<IndexerProxyHealthStatus>,
    pub last_error_message: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NabTransportKind {
    DirectNab,
    ProwlarrNabProxy,
}

impl NabTransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectNab => "direct_nab",
            Self::ProwlarrNabProxy => "prowlarr_nab_proxy",
        }
    }
}

pub fn is_nab_provider_type(provider_type: &str) -> bool {
    matches!(
        provider_type.trim().to_ascii_lowercase().as_str(),
        "amenzb" | "animetosho-xyz" | "dognzb" | "newznab" | "nzbgeek" | "torznab"
    )
}

impl IndexerConfig {
    pub fn rate_limit_domain_key(&self) -> String {
        let parent_id = self
            .managed_parent_config_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .unwrap_or(&self.id);
        indexer_rate_limit_domain_key(parent_id, self.managed_child_key.as_deref())
    }

    pub fn nab_transport_kind(&self) -> Option<NabTransportKind> {
        if !is_nab_provider_type(&self.provider_type) {
            return None;
        }

        Some(if self.managed_parent_config_id.is_some() {
            NabTransportKind::ProwlarrNabProxy
        } else {
            NabTransportKind::DirectNab
        })
    }

    pub fn is_direct_nab(&self) -> bool {
        self.nab_transport_kind() == Some(NabTransportKind::DirectNab)
    }

    pub fn is_prowlarr_nab_proxy(&self) -> bool {
        self.nab_transport_kind() == Some(NabTransportKind::ProwlarrNabProxy)
    }
}

pub fn indexer_rate_limit_domain_key(config_id: &str, child_key: Option<&str>) -> String {
    let config_id = config_id.trim();
    match child_key.map(str::trim).filter(|key| !key.is_empty()) {
        Some(child_key) => format!("{config_id}:{child_key}"),
        None => config_id.to_string(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewIndexerConfig {
    pub name: String,
    pub provider_type: String,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub indexer_proxy_config_id: Option<String>,
    pub download_client_id: Option<String>,
    pub config_json: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadClientStatus {
    #[default]
    Healthy,
    Error,
    Failed,
}

impl DownloadClientStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Error => "error",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "healthy" => Some(Self::Healthy),
            "error" => Some(Self::Error),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadClientConfig {
    pub id: String,
    pub name: String,
    pub client_type: String,
    pub config_json: String,
    pub client_priority: i64,
    pub is_enabled: bool,
    pub status: DownloadClientStatus,
    pub last_error: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewDownloadClientConfig {
    pub name: String,
    pub client_type: String,
    pub config_json: String,
    pub client_priority: i64,
    pub is_enabled: bool,
}

/// How a seeding profile treats season packs relative to its own goals.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeasonPackSeedMode {
    #[default]
    Inherit,
    Override,
}

impl SeasonPackSeedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Override => "override",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "inherit" => Some(Self::Inherit),
            "override" => Some(Self::Override),
            _ => None,
        }
    }
}

/// What happens to a torrent once its resolved seeding goal is met.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeedGoalMetAction {
    #[default]
    RemoveEntry,
    StopSeeding,
    Keep,
}

impl SeedGoalMetAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoveEntry => "remove_entry",
            Self::StopSeeding => "stop_seeding",
            Self::Keep => "keep",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "remove_entry" => Some(Self::RemoveEntry),
            "stop_seeding" => Some(Self::StopSeeding),
            "keep" => Some(Self::Keep),
            _ => None,
        }
    }
}

/// Whether Scryer keeps managing a torrent after it has been imported.
///
/// Sonarr's equivalent is the post-import category: recategorizing drops the
/// torrent out of Sonarr's category-filtered listing, so tracking ends and
/// Sonarr forfeits all further management of it. Scryer lists every torrent and
/// classifies core-side, so the same intent has to be an explicit flag rather
/// than a category trick.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostImportTracking {
    /// Keep managing the torrent after import: park it while it seeds and act
    /// on the profile's goal-met action once the obligation is discharged.
    #[default]
    Park,
    /// Stop managing the torrent after import. The client entry stays exactly
    /// as it is, is never auto-removed, and leaves Scryer's queue.
    HandOff,
}

impl PostImportTracking {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Park => "park",
            Self::HandOff => "hand_off",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "park" => Some(Self::Park),
            "hand_off" => Some(Self::HandOff),
            _ => None,
        }
    }

    pub fn is_hand_off(self) -> bool {
        self == Self::HandOff
    }
}

/// Named torrent seeding policy assignable to indexers, download-client
/// routing entries, or the global default.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SeedingProfile {
    pub id: String,
    pub name: String,
    /// Share ratio goal, or null to defer to the client's own limits.
    pub ratio: Option<f64>,
    /// Seed time goal in minutes, or null to defer to the client's own limits.
    pub seed_time_minutes: Option<i64>,
    pub season_pack_mode: SeasonPackSeedMode,
    pub season_pack_ratio: Option<f64>,
    pub season_pack_seed_time_minutes: Option<i64>,
    /// Clamp resolved goals up to tracker-declared minimums when the release
    /// carries them.
    pub honor_tracker_minimums: bool,
    pub goal_met_action: SeedGoalMetAction,
    /// Never auto-remove torrents grabbed under this profile.
    pub never_remove: bool,
    /// Fewest seeders a candidate may report and still be grabbed.
    ///
    /// The odd one out on this profile: every other field is retention policy
    /// applied after the grab, while this is an admission filter applied
    /// before it. It lives here because the tracker that dictates seeding
    /// terms is the same tracker whose swarm health decides whether a grab can
    /// finish at all, so operators reason about both together.
    ///
    /// `None` inherits the system floor, `Some(0)` disables the check, and
    /// `Some(n)` requires at least `n`. A candidate whose seeder count is
    /// unknown is always eligible, whatever this says.
    pub minimum_seeders: Option<i32>,
    /// Whether Scryer keeps managing torrents grabbed under this profile once
    /// they have been imported.
    pub post_import_tracking: PostImportTracking,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SeedingProfile {
    /// Season-pack goals only apply in `Override` mode; drop stray values so
    /// stored and in-memory profiles agree.
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        if self.season_pack_mode == SeasonPackSeedMode::Inherit {
            self.season_pack_ratio = None;
            self.season_pack_seed_time_minutes = None;
        }
        self
    }

    /// Ratio goal for a release, honoring the season-pack override.
    pub fn effective_ratio(&self, season_pack: bool) -> Option<f64> {
        match (season_pack, self.season_pack_mode) {
            (true, SeasonPackSeedMode::Override) => self.season_pack_ratio,
            _ => self.ratio,
        }
    }

    /// Seed-time goal in minutes for a release, honoring the season-pack override.
    pub fn effective_seed_time_minutes(&self, season_pack: bool) -> Option<i64> {
        match (season_pack, self.season_pack_mode) {
            (true, SeasonPackSeedMode::Override) => self.season_pack_seed_time_minutes,
            _ => self.seed_time_minutes,
        }
    }

    /// Field-level validation shared by create and update paths. Name
    /// uniqueness is enforced by the store.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("seeding profile name is required".to_string());
        }
        validate_seed_ratio("ratio", self.ratio)?;
        validate_seed_time_minutes("seed time", self.seed_time_minutes)?;
        validate_seed_ratio("season pack ratio", self.season_pack_ratio)?;
        validate_seed_time_minutes("season pack seed time", self.season_pack_seed_time_minutes)?;
        // Zero is legal here and means "do not check", unlike the goal fields
        // above where zero would be indistinguishable from unset.
        if self.minimum_seeders.is_some_and(|value| value < 0) {
            return Err("seeding profile minimum seeders cannot be negative".to_string());
        }
        Ok(())
    }
}

fn validate_seed_ratio(label: &str, value: Option<f64>) -> Result<(), String> {
    match value {
        Some(value) if !value.is_finite() || value <= 0.0 => {
            Err(format!("seeding profile {label} must be greater than 0"))
        }
        _ => Ok(()),
    }
}

fn validate_seed_time_minutes(label: &str, value: Option<i64>) -> Result<(), String> {
    match value {
        Some(value) if value <= 0 => Err(format!("seeding profile {label} must be greater than 0")),
        _ => Ok(()),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtitleProviderConfig {
    pub id: String,
    pub name: String,
    pub provider_type: String,
    pub config_json: String,
    #[serde(default)]
    pub enabled_facets: Vec<String>,
    pub is_enabled: bool,
    pub last_health_status: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub disabled_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewSubtitleProviderConfig {
    pub name: String,
    pub provider_type: String,
    pub config_json: String,
    #[serde(default)]
    pub enabled_facets: Vec<String>,
    pub is_enabled: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadQueueState {
    Queued,
    Downloading,
    Verifying,
    Repairing,
    Extracting,
    Paused,
    Completed,
    ImportPending,
    /// The client reports a problem the operator can still fix — a disk that
    /// filled up, a permission error, files moved out from under the torrent,
    /// a tracker that stopped answering.
    ///
    /// Deliberately distinct from [`DownloadQueueState::Failed`]: the entry
    /// stays visible with its message and must never enter failed-download
    /// handling (no blocklist, no removal, no re-search). Only `Failed` means
    /// "this grab is over".
    Warning,
    Failed,
}

// ── TrackedDownloads ──────────────────────────────────────────────

/// Scryer's internal workflow state for a download, independent of the
/// download client's reported status.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrackedDownloadState {
    /// Download in progress (queued, downloading, verifying, repairing, extracting).
    Downloading,
    /// Client reports completed; scryer validated path + title; queued for import.
    ImportPending,
    /// Import actively running.
    Importing,
    /// All expected files imported; download can be removed from client.
    Imported,
    /// All expected files imported; torrent still seeding toward its goal; not
    /// removable yet.
    ImportedSeeding,
    /// Completed but can't auto-import (title mismatch, bad path, ID-only match).
    ImportBlocked,
    /// Client reports failure or encryption detected; queued for failure processing.
    FailedPending,
    /// Failure processed; redownload triggered if enabled.
    Failed,
    /// User manually dismissed.
    Ignored,
}

impl TrackedDownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Downloading => "downloading",
            Self::ImportPending => "import_pending",
            Self::Importing => "importing",
            Self::Imported => "imported",
            Self::ImportedSeeding => "imported_seeding",
            Self::ImportBlocked => "import_blocked",
            Self::FailedPending => "failed_pending",
            Self::Failed => "failed",
            Self::Ignored => "ignored",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "downloading" => Some(Self::Downloading),
            "import_pending" => Some(Self::ImportPending),
            "importing" => Some(Self::Importing),
            "imported" => Some(Self::Imported),
            "imported_seeding" => Some(Self::ImportedSeeding),
            "import_blocked" => Some(Self::ImportBlocked),
            "failed_pending" => Some(Self::FailedPending),
            "failed" => Some(Self::Failed),
            "ignored" => Some(Self::Ignored),
            _ => None,
        }
    }

    /// Terminal states survive restart; non-terminal states are re-derived.
    ///
    /// `ImportedSeeding` is deliberately **not** terminal: the download is
    /// still live in the client and has to be re-evaluated against its seeding
    /// goal on every poll until the goal is met.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Imported | Self::Failed | Self::Ignored)
    }

    /// Import work is finished for this download and must not be dispatched
    /// again. `ImportedSeeding` is settled even though it is not terminal: the
    /// files are already in the library and only the seeding obligation is
    /// outstanding, so re-import or re-observation must not reopen it.
    pub fn is_import_settled(self) -> bool {
        self.is_terminal() || matches!(self, Self::ImportedSeeding)
    }

    /// The download's payload reached the library, whether or not the client
    /// entry has been released yet.
    pub fn counts_as_imported(self) -> bool {
        matches!(self, Self::Imported | Self::ImportedSeeding)
    }
}

/// Health/warning overlay orthogonal to state.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrackedDownloadStatus {
    #[default]
    Ok,
    Warning,
    Error,
}

/// Records how a download was matched to a scryer title.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TitleMatchType {
    /// Direct link via download_submissions (scryer grabbed it).
    Submission,
    /// Matched by embedded client parameters (*scryer_title_id).
    ClientParameter,
    /// Matched by parsing the release title against library.
    TitleParse,
    /// Matched by external ID only (IMDB, TVDB) — ambiguous.
    IdOnly,
    /// No match found.
    #[default]
    Unmatched,
}

impl TitleMatchType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Submission => "submission",
            Self::ClientParameter => "client_parameter",
            Self::TitleParse => "title_parse",
            Self::IdOnly => "id_only",
            Self::Unmatched => "unmatched",
        }
    }
}

/// Per-file import outcome recorded in download_import_artifacts.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportArtifactResult {
    Imported,
    AlreadyPresent,
    Rejected,
}

impl ImportArtifactResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::AlreadyPresent => "already_present",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "imported" => Some(Self::Imported),
            "already_present" => Some(Self::AlreadyPresent),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }

    /// Counts toward download completion verification.
    pub fn counts_as_imported(self) -> bool {
        matches!(self, Self::Imported | Self::AlreadyPresent)
    }
}

/// Torrent seeding state for one download-queue row.
///
/// The first block is **observed**: the download-client plugin reports it on
/// every poll (`PluginDownloadItem::{can_remove, can_move_files, completed_at}`
/// plus `PluginTorrentItem::{seed_ratio, seed_time_seconds, is_private,
/// uploaded_bytes}`), and the download-client adapter copies it verbatim onto
/// the queue item. Every field is `Option` because the clients differ wildly in
/// what they expose, and each `None` is load-bearing: it means *unknown*, never
/// `false` and never `true`.
///
/// The second block is **policy**: the seeding goals the grab was resolved
/// under, frozen on the download-submission row at grab time and joined back
/// onto the queue item during enrichment. They are not client state and are
/// absent for downloads Scryer did not grab under a seeding profile.
///
/// `Eq` is deliberately not derived (and is dropped from `DownloadQueueItem`
/// along with it): a seed ratio is a real number and only `PartialEq` is
/// meaningful for it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DownloadSeedingSnapshot {
    /// The client's own verdict on whether the seeding obligation is
    /// discharged. Tri-state: `None` means this client cannot answer.
    #[serde(default)]
    pub can_remove: Option<bool>,
    /// Whether the payload is complete and stable on disk. **Not** permission
    /// to move it — that additionally requires seeding to be done.
    #[serde(default)]
    pub can_move_files: Option<bool>,
    #[serde(default)]
    pub seed_ratio: Option<f64>,
    #[serde(default)]
    pub seed_time_seconds: Option<i64>,
    /// `Some(true)` arms the private-tracker rail. `None` is unknown, never
    /// "public".
    #[serde(default)]
    pub is_private: Option<bool>,
    #[serde(default)]
    pub uploaded_bytes: Option<i64>,
    /// When the payload finished downloading, RFC3339. The wall-clock fallback
    /// for the time axis on clients with no seed-time counter.
    #[serde(default)]
    pub completed_at: Option<String>,
    /// Resolved ratio goal from the seeding profile this grab used.
    #[serde(default)]
    pub seed_goal_ratio: Option<f64>,
    /// Resolved seed-time goal, in seconds.
    #[serde(default)]
    pub seed_goal_seconds: Option<i64>,
    /// The profile said "seed forever"; this entry is never auto-removed.
    #[serde(default)]
    pub never_remove: bool,
}

impl DownloadSeedingSnapshot {
    /// Whether anything here could only have come from a torrent.
    ///
    /// `can_remove` / `can_move_files` are reported by usenet clients too, so
    /// they do not count; the torrent sub-object fields and the resolved goals
    /// do.
    pub fn has_torrent_signal(&self) -> bool {
        self.seed_ratio.is_some()
            || self.seed_time_seconds.is_some()
            || self.is_private.is_some()
            || self.uploaded_bytes.is_some()
            || self.seed_goal_ratio.is_some()
            || self.seed_goal_seconds.is_some()
            || self.never_remove
    }

    /// Whether the client reported anything at all worth carrying.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DownloadQueueItem {
    pub id: String,
    pub title_id: Option<String>,
    pub episode_id: Option<String>,
    pub title_name: String,
    pub facet: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub client_id: String,
    pub client_name: String,
    pub client_type: String,
    pub state: DownloadQueueState,
    pub progress_percent: u8,
    #[serde(default)]
    pub import_transfer_phase: Option<ImportTransferPhase>,
    #[serde(default)]
    pub import_transfer_bytes: Option<i64>,
    #[serde(default)]
    pub import_transfer_total_bytes: Option<i64>,
    #[serde(default)]
    pub import_transfer_started_at: Option<String>,
    #[serde(default)]
    pub import_transfer_updated_at: Option<String>,
    pub size_bytes: Option<i64>,
    pub remaining_seconds: Option<i64>,
    pub queued_at: Option<String>,
    pub last_updated_at: Option<String>,
    pub attention_required: bool,
    pub attention_reason: Option<String>,
    pub download_client_item_id: String,
    #[serde(default)]
    pub download_id: Option<String>,
    pub import_status: Option<ImportStatus>,
    pub import_error_code: Option<ImportErrorCode>,
    pub import_error_message: Option<String>,
    pub imported_at: Option<String>,
    #[serde(default)]
    pub delete_status: Option<DownloadQueueDeleteStatus>,
    #[serde(default)]
    pub delete_error_message: Option<String>,
    pub is_scryer_origin: bool,
    #[serde(default)]
    pub source_provider: Option<String>,
    /// Scryer's tracked workflow state (populated by TrackedDownloadService).
    #[serde(default)]
    pub tracked_state: Option<TrackedDownloadState>,
    /// Tracked status overlay (Ok/Warning/Error).
    #[serde(default)]
    pub tracked_status: Option<TrackedDownloadStatus>,
    /// Human-readable status messages from tracking.
    #[serde(default)]
    pub tracked_status_messages: Vec<String>,
    /// How the title was resolved for tracking.
    #[serde(default)]
    pub tracked_match_type: Option<TitleMatchType>,
    /// Torrent seeding observation from the latest client poll, plus the goals
    /// the grab resolved to. `None` for usenet and for clients that report
    /// nothing.
    #[serde(default)]
    pub seeding: Option<DownloadSeedingSnapshot>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadQueueDeleteStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl DownloadQueueDeleteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "wmv", "mov", "m4v", "ts", "m2ts", "webm", "flv", "ogv", "strm",
];

pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "sub", "vtt", "idx"];

pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif"];

pub fn canonical_video_extension(path: &std::path::Path) -> Option<&'static str> {
    let normalized = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.trim()
                .trim_end_matches(|ch: char| {
                    ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | '`' | '_' | '.')
                })
                .to_ascii_lowercase()
        })?;

    VIDEO_EXTENSIONS.iter().copied().find(|known| {
        normalized
            .strip_suffix(*known)
            .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1)
    })
}

pub fn is_video_file(path: &std::path::Path) -> bool {
    canonical_video_extension(path).is_some()
}

pub fn is_subtitle_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SUBTITLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn is_image_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub const ARCHIVE_EXTENSIONS: &[&str] = &["rar", "7z", "zip"];

/// Check if a path is a RAR volume file (.rar, .r00, .r01, etc.)
pub fn is_rar_volume(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lower = ext.to_ascii_lowercase();
    lower == "rar"
        || (lower.starts_with('r')
            && lower.len() >= 2
            && lower[1..].chars().all(|c| c.is_ascii_digit()))
}

pub fn is_archive_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ARCHIVE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletedDownload {
    pub client_type: String,
    pub client_id: String,
    pub download_client_item_id: String,
    #[serde(default)]
    pub download_id: Option<String>,
    /// Downloader-provided display/group label. This is non-semantic: import
    /// identity and scoring must use durable submission provenance or
    /// `release_name`, never this value.
    pub name: String,
    /// The release name to parse and score.
    ///
    /// For a download Scryer grabbed this is the indexer release title that was
    /// persisted at grab time (`download_submissions.source_title`) and is
    /// stamped here when the completed download is resolved against its
    /// submission (a submission recorded without one keeps the client-reported
    /// name below). For a download Scryer merely observes it is the best
    /// client-reported original release/artifact name, which is
    /// client-specific and only as good as the client's field:
    /// SABnzbd `nzb_name` (upload filename, extension stripped), Weaver
    /// `originalTitle` (stamped at submit), NZBGet `NZBName` (which NZBGet
    /// keeps identical to the mutable `Name`), or a plugin-provided value
    /// (torrent clients rarely have one). `None` when the client exposes
    /// nothing better than `name`.
    #[serde(default, alias = "nzb_name")]
    pub release_name: Option<String>,
    pub dest_dir: String,
    pub category: Option<String>,
    pub size_bytes: Option<i64>,
    pub completed_at: Option<DateTime<Utc>>,
    pub parameters: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    #[default]
    Pending,
    Running,
    Processing,
    Completed,
    Failed,
    Skipped,
}

impl ImportStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "processing" => Some(Self::Processing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Processing)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportTransferPhase {
    Extracting,
    Copying,
    Finalizing,
}

impl ImportTransferPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extracting => "extracting",
            Self::Copying => "copying",
            Self::Finalizing => "finalizing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "extracting" => Some(Self::Extracting),
            "copying" => Some(Self::Copying),
            "finalizing" => Some(Self::Finalizing),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportType {
    MovieDownload,
    SeriesDownload,
    ManualImport,
    RenamePreview,
    RenameApplyTitle,
    RenameApplyFacet,
    RenameApplyResult,
    RenameIoFailed,
    RenameMove,
    RenameStalePlan,
}

impl ImportType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MovieDownload => "movie_download",
            Self::SeriesDownload => "series_download",
            Self::ManualImport => "manual_import",
            Self::RenamePreview => "rename_preview",
            Self::RenameApplyTitle => "rename_apply_title",
            Self::RenameApplyFacet => "rename_apply_facet",
            Self::RenameApplyResult => "rename_apply_result",
            Self::RenameIoFailed => "rename_io_failed",
            Self::RenameMove => "rename_move",
            Self::RenameStalePlan => "rename_stale_plan",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "movie_download" => Some(Self::MovieDownload),
            "series_download" => Some(Self::SeriesDownload),
            "manual_import" => Some(Self::ManualImport),
            "rename_preview" => Some(Self::RenamePreview),
            "rename_apply_title" => Some(Self::RenameApplyTitle),
            "rename_apply_facet" => Some(Self::RenameApplyFacet),
            "rename_apply_result" => Some(Self::RenameApplyResult),
            "rename_io_failed" => Some(Self::RenameIoFailed),
            "rename_move" => Some(Self::RenameMove),
            "rename_stale_plan" => Some(Self::RenameStalePlan),
            _ => None,
        }
    }

    pub fn is_rename(self) -> bool {
        matches!(
            self,
            Self::RenamePreview
                | Self::RenameApplyTitle
                | Self::RenameApplyFacet
                | Self::RenameApplyResult
                | Self::RenameIoFailed
                | Self::RenameMove
                | Self::RenameStalePlan
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportErrorCode {
    FileNotFound,
    EpisodeNotFound,
    EpisodeLookupFailed,
    SourceJobFailed,
    PolicyMismatch,
    IoFailed,
    PermissionDenied,
    DiskFull,
    Unknown,
}

impl ImportErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileNotFound => "file_not_found",
            Self::EpisodeNotFound => "episode_not_found",
            Self::EpisodeLookupFailed => "episode_lookup_failed",
            Self::SourceJobFailed => "source_job_failed",
            Self::PolicyMismatch => "policy_mismatch",
            Self::IoFailed => "io_failed",
            Self::PermissionDenied => "permission_denied",
            Self::DiskFull => "disk_full",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "file_not_found" => Some(Self::FileNotFound),
            "episode_not_found" => Some(Self::EpisodeNotFound),
            "episode_lookup_failed" => Some(Self::EpisodeLookupFailed),
            "source_job_failed" => Some(Self::SourceJobFailed),
            "policy_mismatch" => Some(Self::PolicyMismatch),
            "io_failed" => Some(Self::IoFailed),
            "permission_denied" => Some(Self::PermissionDenied),
            "disk_full" => Some(Self::DiskFull),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportDecision {
    Imported,
    Rejected,
    Skipped,
    Conflict,
    Unmatched,
    Failed,
}

impl ImportDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Rejected => "rejected",
            Self::Skipped => "skipped",
            Self::Conflict => "conflict",
            Self::Unmatched => "unmatched",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportSkipReason {
    AlreadyImported,
    DuplicateFile,
    PostDownloadRuleBlocked,
    PolicyMismatch,
    UnresolvedIdentity,
    UnparseableEpisode,
    NoVideoFiles,
    DownloadInProgress,
    DiskFull,
    PermissionDenied,
    PasswordRequired,
    ArchiveExtractionPluginRequired,
    ArchiveExtractionTimedOut,
}

impl ImportSkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AlreadyImported => "already_imported",
            Self::DuplicateFile => "duplicate_file",
            Self::PostDownloadRuleBlocked => "post_download_rule_blocked",
            Self::PolicyMismatch => "policy_mismatch",
            Self::UnresolvedIdentity => "unresolved_identity",
            Self::UnparseableEpisode => "unparseable_episode",
            Self::NoVideoFiles => "no_video_files",
            Self::DownloadInProgress => "download_in_progress",
            Self::DiskFull => "disk_full",
            Self::PermissionDenied => "permission_denied",
            Self::PasswordRequired => "password_required",
            Self::ArchiveExtractionPluginRequired => "archive_extraction_plugin_required",
            Self::ArchiveExtractionTimedOut => "archive_extraction_timed_out",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    HardlinkOrCopy,
    Move,
}

impl ImportMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HardlinkOrCopy => "hardlink_or_copy",
            Self::Move => "move",
        }
    }

    pub fn from_setting(value: &str) -> Result<Self, String> {
        match value.trim() {
            "hardlink_or_copy" => Ok(Self::HardlinkOrCopy),
            "move" => Ok(Self::Move),
            other => Err(format!("unsupported import mode: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportStrategy {
    HardLink,
    Copy,
    Symlink,
    Move,
}

impl ImportStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HardLink => "hardlink",
            Self::Copy => "copy",
            Self::Symlink => "symlink",
            Self::Move => "move",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub import_id: String,
    pub decision: ImportDecision,
    pub skip_reason: Option<ImportSkipReason>,
    pub title_id: Option<String>,
    pub source_system: Option<String>,
    pub source_ref: Option<String>,
    pub source_title: Option<String>,
    pub source_path: String,
    pub dest_path: Option<String>,
    pub quality: Option<String>,
    pub episode_ids: Vec<String>,
    pub file_size_bytes: Option<i64>,
    pub link_type: Option<ImportStrategy>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub release_burned: bool,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportRecord {
    pub id: String,
    pub source_client_id: Option<String>,
    pub source_system: String,
    pub source_ref: String,
    pub import_type: ImportType,
    pub status: ImportStatus,
    pub payload_json: String,
    pub result_json: Option<String>,
    pub download_id: Option<String>,
    pub import_transfer_phase: Option<ImportTransferPhase>,
    pub import_transfer_bytes: Option<i64>,
    pub import_transfer_total_bytes: Option<i64>,
    pub import_transfer_started_at: Option<String>,
    pub import_transfer_updated_at: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportDestinationDisposition {
    #[default]
    Created,
    AlreadyPresent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportFileResult {
    pub strategy: ImportStrategy,
    pub source_path: std::path::PathBuf,
    pub dest_path: std::path::PathBuf,
    pub size_bytes: u64,
    #[serde(default)]
    pub destination_disposition: ImportDestinationDisposition,
    pub source_cleanup: Option<ImportSourceCleanupGuard>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportSourceCleanupGuard {
    pub source_path: std::path::PathBuf,
    pub dest_path: std::path::PathBuf,
    pub size_bytes: u64,
    pub source_identity: ImportSourceIdentity,
    pub source_proof: ImportContentProof,
    pub dest_proof: ImportContentProof,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportSourceSnapshot {
    pub identity: ImportSourceIdentity,
    pub proof: ImportContentProof,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportContentProof {
    pub size_bytes: u64,
    pub sample_bytes: u64,
    pub sample_blake3: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportSourceIdentity {
    pub file: ImportFileIdentity,
    pub kind: ImportSourceIdentityKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportFileIdentity {
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    pub dev: u64,
    #[cfg(unix)]
    pub ino: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImportSourceIdentityKind {
    Regular,
    Symlink {
        source_link_target: std::path::PathBuf,
        resolved_target: std::path::PathBuf,
    },
}

// ── Title history ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TitleHistoryEventType {
    Requested,
    Grabbed,
    DownloadFailed,
    Blocklisted,
    DownloadCompleted,
    Scanned,
    Imported,
    ImportFailed,
    ImportSkipped,
    FileUpgraded,
    FileRecycled,
    FileDeleted,
    FileRenamed,
    DownloadIgnored,
    Rematched,
    /// A torrent was imported and its client entry is being retained while it
    /// seeds. Opens the seeding-retention window; recorded once per torrent.
    SeedingStarted,
    /// The seeding obligation was discharged and the gate acted on the client
    /// entry. Closes the window opened by `SeedingStarted`.
    SeedingCompleted,
}

impl TitleHistoryEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Grabbed => "grabbed",
            Self::DownloadFailed => "download_failed",
            Self::Blocklisted => "blocklisted",
            Self::DownloadCompleted => "download_completed",
            Self::Scanned => "scanned",
            Self::Imported => "imported",
            Self::ImportFailed => "import_failed",
            Self::ImportSkipped => "import_skipped",
            Self::FileUpgraded => "file_upgraded",
            Self::FileRecycled => "file_recycled",
            Self::FileDeleted => "file_deleted",
            Self::FileRenamed => "file_renamed",
            Self::DownloadIgnored => "download_ignored",
            Self::Rematched => "rematched",
            Self::SeedingStarted => "seeding_started",
            Self::SeedingCompleted => "seeding_completed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "requested" => Some(Self::Requested),
            "grabbed" => Some(Self::Grabbed),
            "download_failed" => Some(Self::DownloadFailed),
            "blocklisted" => Some(Self::Blocklisted),
            "download_completed" => Some(Self::DownloadCompleted),
            "scanned" => Some(Self::Scanned),
            "imported" => Some(Self::Imported),
            "import_failed" => Some(Self::ImportFailed),
            "import_skipped" => Some(Self::ImportSkipped),
            "file_upgraded" => Some(Self::FileUpgraded),
            "file_recycled" => Some(Self::FileRecycled),
            "file_deleted" => Some(Self::FileDeleted),
            "file_renamed" => Some(Self::FileRenamed),
            "download_ignored" => Some(Self::DownloadIgnored),
            "rematched" => Some(Self::Rematched),
            "seeding_started" => Some(Self::SeedingStarted),
            "seeding_completed" => Some(Self::SeedingCompleted),
            _ => None,
        }
    }

    pub const ALL: &[Self] = &[
        Self::Requested,
        Self::Grabbed,
        Self::DownloadFailed,
        Self::Blocklisted,
        Self::DownloadCompleted,
        Self::Scanned,
        Self::Imported,
        Self::ImportFailed,
        Self::ImportSkipped,
        Self::FileUpgraded,
        Self::FileRecycled,
        Self::FileDeleted,
        Self::FileRenamed,
        Self::DownloadIgnored,
        Self::Rematched,
        Self::SeedingStarted,
        Self::SeedingCompleted,
    ];
}

impl std::fmt::Display for TitleHistoryEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEventType {
    Grabbed,
    Completed,
    Deleted,
}

impl HistoryEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grabbed => "grabbed",
            Self::Completed => "completed",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "grabbed" => Some(Self::Grabbed),
            "completed" => Some(Self::Completed),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TitleHistoryRecord {
    pub id: String,
    pub title_id: String,
    #[serde(default)]
    pub title_name: Option<String>,
    /// Poster of the title, resolved during projection hydration like
    /// `library_id`. `None` when the title row is gone or has no poster.
    #[serde(default)]
    pub poster_url: Option<String>,
    /// Library owning the title, resolved during projection hydration. `None`
    /// when the title row is gone and the event cannot be attributed.
    #[serde(default)]
    pub library_id: Option<String>,
    #[serde(default)]
    pub facet: Option<MediaFacet>,
    pub episode_id: Option<String>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
    pub collection_id: Option<String>,
    pub event_type: TitleHistoryEventType,
    #[serde(default)]
    pub actor_kind: Option<DomainEventActorKind>,
    #[serde(default)]
    pub actor_user_id: Option<String>,
    #[serde(default)]
    pub actor_display_name: Option<String>,
    pub source_title: Option<String>,
    #[serde(default)]
    pub display_title: Option<String>,
    #[serde(default)]
    pub source_system: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub client_id: Option<String>,
    pub client_name: Option<String>,
    #[serde(default)]
    pub import_id: Option<String>,
    #[serde(default)]
    pub skip_reason: Option<String>,
    #[serde(default)]
    pub retry_requires_password: bool,
    pub failure_reason: Option<String>,
    pub blocklist_reason: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub dest_path: Option<String>,
    /// Bytes involved in the event: the imported total for `Imported` rows and
    /// the new file's size for `FileUpgraded` rows. `None` for other event
    /// types and for events persisted before the payload carried a size.
    #[serde(default)]
    pub size_bytes: Option<i64>,
    pub data_json: Option<String>,
    pub occurred_at: String,
    pub created_at: String,
}

/// One blocked release for one title.
///
/// A blocklist row blocks a *release*, not an episode and not a download. Its
/// identity is the infohash when the release has one -- content identity is the
/// same wherever the torrent came from -- and otherwise the indexer it failed
/// on plus its normalized name.
#[derive(Clone, Debug)]
pub struct BlocklistEntry {
    pub id: String,
    pub title_id: String,
    /// The release name as the indexer presented it. Display only.
    pub release_name: String,
    /// `release_name` trimmed and ASCII-lowercased: the matcher and the key.
    pub normalized_release_name: String,
    /// The indexer this release failed on; empty blocks it on every indexer.
    ///
    /// Empty is not only a pre-migration shim: a manual replacement blocklists
    /// a name read off a file on disk, where the only recorded provenance is
    /// the indexer's display name rather than a stable id.
    pub indexer_id: String,
    /// Set for a torrent whose infohash was known, which keys the block
    /// independently of the indexer that served it.
    pub info_hash: Option<String>,
    pub reason: Option<String>,
    pub created_at: String,
}

// ── Titles ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewTitle {
    pub name: String,
    pub facet: MediaFacet,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub external_ids: Vec<ExternalId>,
    #[serde(default)]
    pub root_folder_id: Option<String>,
    #[serde(default)]
    pub min_availability: Option<String>,
    #[serde(default)]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub sort_title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub runtime_minutes: Option<i32>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub content_status: Option<String>,
}

impl NewTitle {
    pub fn with_defaults(name: impl Into<String>, facet: MediaFacet) -> Self {
        Self {
            name: name.into(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: None,
            min_availability: None,
            poster_url: None,
            year: None,
            overview: None,
            sort_title: None,
            slug: None,
            runtime_minutes: None,
            language: None,
            content_status: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEvent {
    pub id: String,
    pub event_type: EventType,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TitleAdded,
    TitleUpdated,
    PolicyEvaluated,
    ActionTriggered,
    ActionCompleted,
    FileUpgraded,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
#[derive(strum::EnumIter)]
pub enum DomainEventType {
    MediaRequestSubmitted,
    MediaRequestUpdated,
    MediaRequestApproved,
    MediaRequestRejected,
    MediaRequestCanceled,
    TitleAdded,
    TitleUpdated,
    TitleRematched,
    TitleDeleted,
    ConfigurationChanged,
    DiscoverySearchCompleted,
    MetadataHydrationUpdated,
    ReleaseGrabbed,
    DownloadFailed,
    ReleaseBlocklisted,
    ImportCompleted,
    ImportRejected,
    MediaFileImported,
    MediaFileAnalyzed,
    MediaFileRenamed,
    MediaFileDeleted,
    MediaFileUpgraded,
    AcquisitionSearchCompleted,
    AcquisitionCandidateRejected,
    ImportRequested,
    ImportRecoveryCompleted,
    DownloadQueueItemCommandIssued,
    PostProcessingCompleted,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    LibraryScanStarted,
    LibraryScanTitleDiscovered,
    LibraryScanDeltaRecorded,
    LibraryScanProgressed,
    LibraryScanCompleted,
    LibraryScanCanceled,
    LibraryScanFailed,
    JobRunStarted,
    JobRunCompleted,
    JobRunFailed,
    JobNextRunUpdated,
    DownloadQueueItemUpserted,
    DownloadQueueItemRemoved,
    DownloadIgnored,
    SeedingStarted,
    SeedingCompleted,
}

impl DomainEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MediaRequestSubmitted => "media_request_submitted",
            Self::MediaRequestUpdated => "media_request_updated",
            Self::MediaRequestApproved => "media_request_approved",
            Self::MediaRequestRejected => "media_request_rejected",
            Self::MediaRequestCanceled => "media_request_canceled",
            Self::TitleAdded => "title_added",
            Self::TitleUpdated => "title_updated",
            Self::TitleRematched => "title_rematched",
            Self::TitleDeleted => "title_deleted",
            Self::ConfigurationChanged => "configuration_changed",
            Self::DiscoverySearchCompleted => "discovery_search_completed",
            Self::MetadataHydrationUpdated => "metadata_hydration_updated",
            Self::ReleaseGrabbed => "release_grabbed",
            Self::DownloadFailed => "download_failed",
            Self::ReleaseBlocklisted => "release_blocklisted",
            Self::ImportCompleted => "import_completed",
            Self::ImportRejected => "import_rejected",
            Self::MediaFileImported => "media_file_imported",
            Self::MediaFileAnalyzed => "media_file_analyzed",
            Self::MediaFileRenamed => "media_file_renamed",
            Self::MediaFileDeleted => "media_file_deleted",
            Self::MediaFileUpgraded => "media_file_upgraded",
            Self::AcquisitionSearchCompleted => "acquisition_search_completed",
            Self::AcquisitionCandidateRejected => "acquisition_candidate_rejected",
            Self::ImportRequested => "import_requested",
            Self::ImportRecoveryCompleted => "import_recovery_completed",
            Self::DownloadQueueItemCommandIssued => "download_queue_item_command_issued",
            Self::PostProcessingCompleted => "post_processing_completed",
            Self::SubtitleDownloaded => "subtitle_downloaded",
            Self::SubtitleSearchFailed => "subtitle_search_failed",
            Self::LibraryScanStarted => "library_scan_started",
            Self::LibraryScanTitleDiscovered => "library_scan_title_discovered",
            Self::LibraryScanDeltaRecorded => "library_scan_delta_recorded",
            Self::LibraryScanProgressed => "library_scan_progressed",
            Self::LibraryScanCompleted => "library_scan_completed",
            Self::LibraryScanCanceled => "library_scan_canceled",
            Self::LibraryScanFailed => "library_scan_failed",
            Self::JobRunStarted => "job_run_started",
            Self::JobRunCompleted => "job_run_completed",
            Self::JobRunFailed => "job_run_failed",
            Self::JobNextRunUpdated => "job_next_run_updated",
            Self::DownloadQueueItemUpserted => "download_queue_item_upserted",
            Self::DownloadQueueItemRemoved => "download_queue_item_removed",
            Self::DownloadIgnored => "download_ignored",
            Self::SeedingStarted => "seeding_started",
            Self::SeedingCompleted => "seeding_completed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "media_request_submitted" => Some(Self::MediaRequestSubmitted),
            "media_request_updated" => Some(Self::MediaRequestUpdated),
            "media_request_approved" => Some(Self::MediaRequestApproved),
            "media_request_rejected" => Some(Self::MediaRequestRejected),
            "media_request_canceled" => Some(Self::MediaRequestCanceled),
            "title_added" => Some(Self::TitleAdded),
            "title_updated" => Some(Self::TitleUpdated),
            "title_rematched" => Some(Self::TitleRematched),
            "title_deleted" => Some(Self::TitleDeleted),
            "configuration_changed" => Some(Self::ConfigurationChanged),
            "discovery_search_completed" => Some(Self::DiscoverySearchCompleted),
            "metadata_hydration_updated" => Some(Self::MetadataHydrationUpdated),
            "release_grabbed" => Some(Self::ReleaseGrabbed),
            "download_failed" => Some(Self::DownloadFailed),
            "release_blocklisted" => Some(Self::ReleaseBlocklisted),
            "import_completed" => Some(Self::ImportCompleted),
            "import_rejected" => Some(Self::ImportRejected),
            "media_file_imported" => Some(Self::MediaFileImported),
            "media_file_analyzed" => Some(Self::MediaFileAnalyzed),
            "media_file_renamed" => Some(Self::MediaFileRenamed),
            "media_file_deleted" => Some(Self::MediaFileDeleted),
            "media_file_upgraded" => Some(Self::MediaFileUpgraded),
            "acquisition_search_completed" => Some(Self::AcquisitionSearchCompleted),
            "acquisition_candidate_rejected" => Some(Self::AcquisitionCandidateRejected),
            "import_requested" => Some(Self::ImportRequested),
            "import_recovery_completed" => Some(Self::ImportRecoveryCompleted),
            "download_queue_item_command_issued" => Some(Self::DownloadQueueItemCommandIssued),
            "post_processing_completed" => Some(Self::PostProcessingCompleted),
            "subtitle_downloaded" => Some(Self::SubtitleDownloaded),
            "subtitle_search_failed" => Some(Self::SubtitleSearchFailed),
            "library_scan_started" => Some(Self::LibraryScanStarted),
            "library_scan_title_discovered" => Some(Self::LibraryScanTitleDiscovered),
            "library_scan_delta_recorded" => Some(Self::LibraryScanDeltaRecorded),
            "library_scan_progressed" => Some(Self::LibraryScanProgressed),
            "library_scan_completed" => Some(Self::LibraryScanCompleted),
            "library_scan_canceled" => Some(Self::LibraryScanCanceled),
            "library_scan_failed" => Some(Self::LibraryScanFailed),
            "job_run_started" => Some(Self::JobRunStarted),
            "job_run_completed" => Some(Self::JobRunCompleted),
            "job_run_failed" => Some(Self::JobRunFailed),
            "job_next_run_updated" => Some(Self::JobNextRunUpdated),
            "download_queue_item_upserted" => Some(Self::DownloadQueueItemUpserted),
            "download_queue_item_removed" => Some(Self::DownloadQueueItemRemoved),
            "download_ignored" => Some(Self::DownloadIgnored),
            "seeding_started" => Some(Self::SeedingStarted),
            "seeding_completed" => Some(Self::SeedingCompleted),
            _ => None,
        }
    }

    pub fn variants() -> impl Iterator<Item = Self> {
        <Self as strum::IntoEnumIterator>::iter()
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaUpdateType {
    #[default]
    Created,
    Modified,
    Deleted,
}

impl MediaUpdateType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaPathUpdate {
    pub path: String,
    pub update_type: MediaUpdateType,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainExternalIds {
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub anidb_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleContextSnapshot {
    pub title_name: String,
    pub facet: MediaFacet,
    pub external_ids: DomainExternalIds,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaRequestSubmittedEventData {
    pub request_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    pub title_name: String,
    pub external_ids: Vec<ExternalId>,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    pub requested_quality_profile_id: Option<String>,
    pub requested_quality_profile_name: Option<String>,
    pub requested_monitor_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaRequestResolvedEventData {
    pub request_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    pub title_name: String,
    pub external_ids: Vec<ExternalId>,
    pub created_title_id: Option<String>,
    pub requested_quality_profile_id: Option<String>,
    pub requested_quality_profile_name: Option<String>,
    pub requested_monitor_type: Option<String>,
    pub approved_quality_profile_id: Option<String>,
    pub approved_quality_profile_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleAddedEventData {
    pub title: TitleContextSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleUpdatedEventData {
    pub title: TitleContextSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleRematchedEventData {
    pub title: TitleContextSnapshot,
    pub old_tvdb_id: Option<String>,
    pub new_tvdb_id: String,
    pub smg_id: Option<i64>,
    pub tmdb_id: Option<i64>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleDeletedEventData {
    pub title: TitleContextSnapshot,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationChangeAction {
    Saved,
    Updated,
    Deleted,
    Reordered,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigurationChangedEventData {
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub action: ConfigurationChangeAction,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoverySearchCompletedEventData {
    pub search_type: String,
    pub query: Option<String>,
    pub result_count: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetadataHydrationState {
    Started,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetadataHydrationUpdatedEventData {
    pub title: TitleContextSnapshot,
    pub state: MetadataHydrationState,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseGrabbedEventData {
    pub title: TitleContextSnapshot,
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    #[serde(default)]
    pub source_provider: Option<String>,
    pub download_id: Option<String>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadFailedEventData {
    #[serde(default)]
    pub title: Option<TitleContextSnapshot>,
    #[serde(default)]
    pub source_title: Option<String>,
    #[serde(default)]
    pub source_hint: Option<String>,
    #[serde(default)]
    pub download_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub client_type: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default, alias = "error_message")]
    pub reason: Option<String>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
    #[serde(default)]
    pub collection_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseBlocklistedEventData {
    #[serde(default)]
    pub title: Option<TitleContextSnapshot>,
    #[serde(default)]
    pub source_title: Option<String>,
    #[serde(default)]
    pub source_hint: Option<String>,
    #[serde(default)]
    pub download_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub client_type: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
    #[serde(default)]
    pub collection_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportCompletedEventData {
    pub title: TitleContextSnapshot,
    pub media_updates: Vec<MediaPathUpdate>,
    pub imported_count: i32,
    #[serde(default)]
    pub import_id: Option<String>,
    #[serde(default)]
    pub source_system: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub source_title: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub dest_path: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
    /// Total bytes imported by this event. `#[serde(default)]` keeps events
    /// persisted before this field existed deserializable; they read back as
    /// `None`.
    #[serde(default)]
    pub size_bytes: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRejectedEventData {
    pub title: Option<TitleContextSnapshot>,
    pub status: ImportStatus,
    #[serde(default)]
    pub import_id: Option<String>,
    #[serde(default)]
    pub source_system: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub source_title: Option<String>,
    pub source_path: Option<String>,
    #[serde(default)]
    pub dest_path: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    pub reason: Option<String>,
    #[serde(default)]
    pub skip_reason: Option<ImportSkipReason>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaFileImportedEventData {
    pub title: TitleContextSnapshot,
    pub media_updates: Vec<MediaPathUpdate>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaFileAnalyzedEventData {
    pub title: TitleContextSnapshot,
    pub media_updates: Vec<MediaPathUpdate>,
    pub file_id: String,
    pub analysis_status: String,
    #[serde(default)]
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaFileRenamedEventData {
    pub title: TitleContextSnapshot,
    pub media_updates: Vec<MediaPathUpdate>,
    pub renamed_count: i32,
    #[serde(default)]
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaFileDeletedReason {
    Deleted,
    UpgradeCleanup,
    RecycleBinPurged,
    MissingOnDisk,
}

impl MediaFileDeletedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deleted => "deleted",
            Self::UpgradeCleanup => "upgrade_cleanup",
            Self::RecycleBinPurged => "recycle_bin_purged",
            Self::MissingOnDisk => "missing_on_disk",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaFileDeletedEventData {
    pub title: TitleContextSnapshot,
    pub media_updates: Vec<MediaPathUpdate>,
    pub file_id: Option<String>,
    pub reason: MediaFileDeletedReason,
    #[serde(default)]
    pub episode_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaFileUpgradedEventData {
    pub title: TitleContextSnapshot,
    pub media_updates: Vec<MediaPathUpdate>,
    #[serde(default)]
    pub episode_ids: Vec<String>,
    pub previous_file_id: Option<String>,
    pub current_file_id: Option<String>,
    pub old_score: Option<i32>,
    pub new_score: Option<i32>,
    /// Size of the NEW (current) file. `#[serde(default)]` keeps events
    /// persisted before this field existed deserializable; they read back as
    /// `None`.
    #[serde(default)]
    pub size_bytes: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcquisitionSearchCompletedEventData {
    pub title: TitleContextSnapshot,
    pub result_count: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcquisitionCandidateRejectedEventData {
    pub title: TitleContextSnapshot,
    pub source_title: String,
    pub reason_code: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportRequestKind {
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRequestedEventData {
    pub title: Option<TitleContextSnapshot>,
    pub client_type: String,
    pub source_ref: String,
    pub request_kind: ImportRequestKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRecoveryCompletedEventData {
    pub recovered_count: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadQueueCommandAction {
    Pause,
    Resume,
    Delete,
}

impl DownloadQueueCommandAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Delete => "delete",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadQueueItemCommandIssuedEventData {
    pub item_id: String,
    pub action: DownloadQueueCommandAction,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostProcessingResult {
    Succeeded,
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostProcessingCompletedEventData {
    pub title: TitleContextSnapshot,
    pub script_name: String,
    pub result: PostProcessingResult,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtitleDownloadedEventData {
    pub title: TitleContextSnapshot,
    pub subtitle_path: Option<String>,
    pub language: Option<String>,
    pub provider: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtitleSearchFailedEventData {
    pub title: TitleContextSnapshot,
    pub language: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanStartedEventData {
    pub session_id: String,
    #[serde(default)]
    pub library_id: Option<String>,
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanTitleDiscoveredEventData {
    pub session_id: String,
    pub title_id: String,
    pub title_name: String,
    pub facet: MediaFacet,
    pub discovered_file_count: i64,
    pub folder_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanDeltaRecordedEventData {
    pub session_id: String,
    pub found_titles_total: Option<i64>,
    #[serde(default)]
    pub found_titles_delta: i64,
    #[serde(default)]
    pub title_match_completed_delta: i64,
    #[serde(default)]
    pub title_match_failed_delta: i64,
    pub title_match_total_known: Option<bool>,
    #[serde(default)]
    pub metadata_total_delta: i64,
    #[serde(default)]
    pub metadata_completed_delta: i64,
    #[serde(default)]
    pub metadata_failed_delta: i64,
    pub metadata_total_known: Option<bool>,
    #[serde(default)]
    pub file_total_delta: i64,
    #[serde(default)]
    pub file_completed_delta: i64,
    #[serde(default)]
    pub file_failed_delta: i64,
    pub file_total_known: Option<bool>,
    #[serde(default)]
    pub summary: Option<LibraryScanSummaryEventData>,
    #[serde(default)]
    pub summary_is_delta: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanProgressedEventData {
    pub session_id: String,
    pub status: String,
    pub found_titles: i64,
    #[serde(default)]
    pub title_match_completed: i64,
    #[serde(default)]
    pub title_match_total_known: bool,
    pub titles_completed: i64,
    pub titles_total: Option<i64>,
    pub files_completed: i64,
    pub files_total: Option<i64>,
    #[serde(default)]
    pub warning_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanSummaryEventData {
    pub scanned: i64,
    pub matched: i64,
    pub imported: i64,
    pub skipped: i64,
    pub unmatched: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanCompletedEventData {
    pub session_id: String,
    pub status: String,
    pub found_titles: i64,
    #[serde(default)]
    pub title_match_completed: i64,
    #[serde(default)]
    pub title_match_total_known: bool,
    pub titles_completed: i64,
    pub titles_total: Option<i64>,
    pub files_completed: i64,
    pub files_total: Option<i64>,
    #[serde(default)]
    pub summary: Option<LibraryScanSummaryEventData>,
    #[serde(default)]
    pub warning_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanCanceledEventData {
    pub session_id: String,
    pub status: String,
    pub found_titles: i64,
    #[serde(default)]
    pub title_match_completed: i64,
    #[serde(default)]
    pub title_match_total_known: bool,
    pub titles_completed: i64,
    pub titles_total: Option<i64>,
    pub files_completed: i64,
    pub files_total: Option<i64>,
    #[serde(default)]
    pub summary: Option<LibraryScanSummaryEventData>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryScanFailedEventData {
    pub session_id: String,
    pub error_message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobRunStartedEventData {
    pub run_id: String,
    pub job_key: String,
    pub operation_type: String,
    pub trigger_source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobRunCompletedEventData {
    pub run_id: String,
    pub job_key: String,
    pub summary_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobRunFailedEventData {
    pub run_id: String,
    pub job_key: String,
    pub error_text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobNextRunUpdatedEventData {
    pub job_key: String,
    pub next_run_at: Option<String>,
}

/// `Eq` is not derivable here: the queue item carries an observed seed ratio,
/// which is an `f64`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DownloadQueueItemUpsertedEventData {
    pub item: DownloadQueueItem,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadQueueItemRemovedEventData {
    pub download_client_item_id: String,
    pub client_id: Option<String>,
    pub client_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadIgnoredEventData {
    pub title: Option<TitleContextSnapshot>,
    pub download_client_item_id: String,
    pub client_id: Option<String>,
    pub client_type: Option<String>,
    pub source_provider: Option<String>,
    /// Release title the download was grabbed under; `default` so events
    /// appended before this field existed still deserialize.
    #[serde(default)]
    pub source_title: Option<String>,
}

/// A torrent was imported and its client entry retained while it seeds.
///
/// Recorded once, on the first transition into `TrackedDownloadState::
/// ImportedSeeding` — never once per held poll tick. `Eq` is not derivable:
/// the resolved ratio goal is an `f64`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SeedingStartedEventData {
    pub title: Option<TitleContextSnapshot>,
    pub download_client_item_id: String,
    pub client_id: Option<String>,
    pub client_type: Option<String>,
    pub source_provider: Option<String>,
    /// Release title the download was grabbed under.
    pub source_title: Option<String>,
    /// The seeding gate's reason constant for holding this entry, verbatim.
    pub reason: String,
    /// Observed at the moment the entry was parked, when the client reports it.
    pub seed_ratio: Option<f64>,
    pub seed_time_seconds: Option<i64>,
}

/// The seeding obligation was discharged and the gate acted on the client
/// entry. `Eq` is not derivable: the observed ratio is an `f64`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SeedingCompletedEventData {
    pub title: Option<TitleContextSnapshot>,
    pub download_client_item_id: String,
    pub client_id: Option<String>,
    pub client_type: Option<String>,
    pub source_provider: Option<String>,
    /// Release title the download was grabbed under.
    pub source_title: Option<String>,
    /// What actually happened to the client entry: `removed`, `paused`,
    /// `kept`, or `vanished` (the client had already dropped it).
    pub action: String,
    /// The seeding gate's reason constant for releasing the entry, verbatim.
    pub reason: String,
    /// Observed at release, when the client reports it.
    pub seed_ratio: Option<f64>,
    pub seed_time_seconds: Option<i64>,
}

/// `Eq` is not derivable: the download-queue payload carries an observed seed
/// ratio (`f64`). Nothing keys a map or set on an event payload, so `PartialEq`
/// is the whole requirement.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum DomainEventPayload {
    MediaRequestSubmitted(MediaRequestSubmittedEventData),
    MediaRequestUpdated(MediaRequestSubmittedEventData),
    MediaRequestApproved(MediaRequestResolvedEventData),
    MediaRequestRejected(MediaRequestResolvedEventData),
    MediaRequestCanceled(MediaRequestResolvedEventData),
    TitleAdded(TitleAddedEventData),
    TitleUpdated(TitleUpdatedEventData),
    TitleRematched(TitleRematchedEventData),
    TitleDeleted(TitleDeletedEventData),
    ConfigurationChanged(ConfigurationChangedEventData),
    DiscoverySearchCompleted(DiscoverySearchCompletedEventData),
    MetadataHydrationUpdated(MetadataHydrationUpdatedEventData),
    ReleaseGrabbed(ReleaseGrabbedEventData),
    DownloadFailed(DownloadFailedEventData),
    ReleaseBlocklisted(ReleaseBlocklistedEventData),
    ImportCompleted(ImportCompletedEventData),
    ImportRejected(ImportRejectedEventData),
    MediaFileImported(MediaFileImportedEventData),
    MediaFileAnalyzed(MediaFileAnalyzedEventData),
    MediaFileRenamed(MediaFileRenamedEventData),
    MediaFileDeleted(MediaFileDeletedEventData),
    MediaFileUpgraded(MediaFileUpgradedEventData),
    AcquisitionSearchCompleted(AcquisitionSearchCompletedEventData),
    AcquisitionCandidateRejected(AcquisitionCandidateRejectedEventData),
    ImportRequested(ImportRequestedEventData),
    ImportRecoveryCompleted(ImportRecoveryCompletedEventData),
    DownloadQueueItemCommandIssued(DownloadQueueItemCommandIssuedEventData),
    PostProcessingCompleted(PostProcessingCompletedEventData),
    SubtitleDownloaded(SubtitleDownloadedEventData),
    SubtitleSearchFailed(SubtitleSearchFailedEventData),
    LibraryScanStarted(LibraryScanStartedEventData),
    LibraryScanTitleDiscovered(LibraryScanTitleDiscoveredEventData),
    LibraryScanDeltaRecorded(LibraryScanDeltaRecordedEventData),
    LibraryScanProgressed(LibraryScanProgressedEventData),
    LibraryScanCompleted(LibraryScanCompletedEventData),
    LibraryScanCanceled(LibraryScanCanceledEventData),
    LibraryScanFailed(LibraryScanFailedEventData),
    JobRunStarted(JobRunStartedEventData),
    JobRunCompleted(JobRunCompletedEventData),
    JobRunFailed(JobRunFailedEventData),
    JobNextRunUpdated(JobNextRunUpdatedEventData),
    /// Boxed: this payload embeds a whole `DownloadQueueItem` and would
    /// otherwise set the size of every `DomainEventPayload` in the system.
    DownloadQueueItemUpserted(Box<DownloadQueueItemUpsertedEventData>),
    DownloadQueueItemRemoved(DownloadQueueItemRemovedEventData),
    DownloadIgnored(DownloadIgnoredEventData),
    SeedingStarted(SeedingStartedEventData),
    SeedingCompleted(SeedingCompletedEventData),
}

impl DomainEventPayload {
    pub fn event_type(&self) -> DomainEventType {
        match self {
            Self::MediaRequestSubmitted(_) => DomainEventType::MediaRequestSubmitted,
            Self::MediaRequestUpdated(_) => DomainEventType::MediaRequestUpdated,
            Self::MediaRequestApproved(_) => DomainEventType::MediaRequestApproved,
            Self::MediaRequestRejected(_) => DomainEventType::MediaRequestRejected,
            Self::MediaRequestCanceled(_) => DomainEventType::MediaRequestCanceled,
            Self::TitleAdded(_) => DomainEventType::TitleAdded,
            Self::TitleUpdated(_) => DomainEventType::TitleUpdated,
            Self::TitleRematched(_) => DomainEventType::TitleRematched,
            Self::TitleDeleted(_) => DomainEventType::TitleDeleted,
            Self::ConfigurationChanged(_) => DomainEventType::ConfigurationChanged,
            Self::DiscoverySearchCompleted(_) => DomainEventType::DiscoverySearchCompleted,
            Self::MetadataHydrationUpdated(_) => DomainEventType::MetadataHydrationUpdated,
            Self::ReleaseGrabbed(_) => DomainEventType::ReleaseGrabbed,
            Self::DownloadFailed(_) => DomainEventType::DownloadFailed,
            Self::ReleaseBlocklisted(_) => DomainEventType::ReleaseBlocklisted,
            Self::ImportCompleted(_) => DomainEventType::ImportCompleted,
            Self::ImportRejected(_) => DomainEventType::ImportRejected,
            Self::MediaFileImported(_) => DomainEventType::MediaFileImported,
            Self::MediaFileAnalyzed(_) => DomainEventType::MediaFileAnalyzed,
            Self::MediaFileRenamed(_) => DomainEventType::MediaFileRenamed,
            Self::MediaFileDeleted(_) => DomainEventType::MediaFileDeleted,
            Self::MediaFileUpgraded(_) => DomainEventType::MediaFileUpgraded,
            Self::AcquisitionSearchCompleted(_) => DomainEventType::AcquisitionSearchCompleted,
            Self::AcquisitionCandidateRejected(_) => DomainEventType::AcquisitionCandidateRejected,
            Self::ImportRequested(_) => DomainEventType::ImportRequested,
            Self::ImportRecoveryCompleted(_) => DomainEventType::ImportRecoveryCompleted,
            Self::DownloadQueueItemCommandIssued(_) => {
                DomainEventType::DownloadQueueItemCommandIssued
            }
            Self::PostProcessingCompleted(_) => DomainEventType::PostProcessingCompleted,
            Self::SubtitleDownloaded(_) => DomainEventType::SubtitleDownloaded,
            Self::SubtitleSearchFailed(_) => DomainEventType::SubtitleSearchFailed,
            Self::LibraryScanStarted(_) => DomainEventType::LibraryScanStarted,
            Self::LibraryScanTitleDiscovered(_) => DomainEventType::LibraryScanTitleDiscovered,
            Self::LibraryScanDeltaRecorded(_) => DomainEventType::LibraryScanDeltaRecorded,
            Self::LibraryScanProgressed(_) => DomainEventType::LibraryScanProgressed,
            Self::LibraryScanCompleted(_) => DomainEventType::LibraryScanCompleted,
            Self::LibraryScanCanceled(_) => DomainEventType::LibraryScanCanceled,
            Self::LibraryScanFailed(_) => DomainEventType::LibraryScanFailed,
            Self::JobRunStarted(_) => DomainEventType::JobRunStarted,
            Self::JobRunCompleted(_) => DomainEventType::JobRunCompleted,
            Self::JobRunFailed(_) => DomainEventType::JobRunFailed,
            Self::JobNextRunUpdated(_) => DomainEventType::JobNextRunUpdated,
            Self::DownloadQueueItemUpserted(_) => DomainEventType::DownloadQueueItemUpserted,
            Self::DownloadQueueItemRemoved(_) => DomainEventType::DownloadQueueItemRemoved,
            Self::DownloadIgnored(_) => DomainEventType::DownloadIgnored,
            Self::SeedingStarted(_) => DomainEventType::SeedingStarted,
            Self::SeedingCompleted(_) => DomainEventType::SeedingCompleted,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DomainEventStream {
    Global,
    Title { title_id: String },
    LibraryScan { session_id: String },
    JobRun { run_id: String },
    DownloadQueueItem { item_id: String },
}

impl DomainEventStream {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Title { .. } => "title",
            Self::LibraryScan { .. } => "library_scan",
            Self::JobRun { .. } => "job_run",
            Self::DownloadQueueItem { .. } => "download_queue_item",
        }
    }

    pub fn identifier(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::Title { title_id } => Some(title_id.as_str()),
            Self::LibraryScan { session_id } => Some(session_id.as_str()),
            Self::JobRun { run_id } => Some(run_id.as_str()),
            Self::DownloadQueueItem { item_id } => Some(item_id.as_str()),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DomainEventActorKind {
    User,
    Anonymous,
    System,
}

impl DomainEventActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Anonymous => "anonymous",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "anonymous" => Some(Self::Anonymous),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// `Eq` is not derivable: a download-queue payload carries an observed seed
/// ratio (`f64`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DomainEvent {
    pub sequence: i64,
    pub event_id: String,
    pub occurred_at: DateTime<Utc>,
    pub actor_kind: DomainEventActorKind,
    pub actor_user_id: Option<String>,
    pub actor_display_name: String,
    pub title_id: Option<String>,
    pub facet: Option<MediaFacet>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub schema_version: i32,
    pub stream: DomainEventStream,
    pub payload: DomainEventPayload,
}

/// `Eq` is not derivable: a download-queue payload carries an observed seed
/// ratio (`f64`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NewDomainEvent {
    pub event_id: String,
    pub occurred_at: DateTime<Utc>,
    pub actor_kind: DomainEventActorKind,
    pub actor_user_id: Option<String>,
    pub actor_display_name: String,
    pub title_id: Option<String>,
    pub facet: Option<MediaFacet>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub schema_version: i32,
    pub stream: DomainEventStream,
    pub payload: DomainEventPayload,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainEventFilter {
    pub event_types: Option<Vec<DomainEventType>>,
    pub title_id: Option<String>,
    pub facet: Option<MediaFacet>,
    pub after_sequence: Option<i64>,
    pub before_sequence: Option<i64>,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedMode {
    #[default]
    Automatic,
    Manual,
}

impl RequestedMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "automatic" => Some(Self::Automatic),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyInput {
    pub title_id: String,
    pub facet: MediaFacet,
    pub has_existing_file: bool,
    pub candidate_quality: Option<String>,
    pub requested_mode: RequestedMode,
    pub release_title: Option<String>,
    pub quality_profile_id: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub is_anime: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyOutput {
    pub decision: bool,
    pub score: f32,
    pub reason_codes: Vec<String>,
    pub explanation: String,
    pub scoring_log: Vec<PolicyScoringEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PolicyScoringEntry {
    pub code: String,
    pub delta: i32,
    pub source: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginSourceKind {
    Bundled,
    #[default]
    Downloaded,
    Community,
    Manual,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginSupportTier {
    #[default]
    Official,
    VerifiedCommunity,
    Unverified,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginWasmEncoding {
    #[default]
    Identity,
    Brotli,
    Zstd,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedPluginWasmPayload {
    pub encoding: PluginWasmEncoding,
    pub bytes: Vec<u8>,
}

/// A plugin installation record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginInstallation {
    pub id: String,
    /// Unique plugin identifier from the registry (e.g. "nzbgeek", "newznab").
    pub plugin_id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub sdk_version: String,
    pub sdk_constraint: String,
    pub scryer_constraint: Option<String>,
    pub plugin_type: String,
    pub provider_type: String,
    pub source_kind: PluginSourceKind,
    pub is_enabled: bool,
    pub is_builtin: bool,
    pub wasm_encoding: PluginWasmEncoding,
    pub wasm_digest_algo: Option<String>,
    pub source_url: Option<String>,
    pub support_tier: PluginSupportTier,
    pub publisher: Option<String>,
    pub docs_url: Option<String>,
    pub source_repo: Option<String>,
    pub manifest_url: Option<String>,
    pub wasm_digest: Option<String>,
    pub artifact_digest: Option<String>,
    pub descriptor_json: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginCatalogSource {
    pub source_key: String,
    pub source_kind: String,
    pub source_url: String,
    pub github_repo: Option<String>,
    pub support_tier: PluginSupportTier,
    pub catalog_json: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginCatalogStatusRecord {
    pub status_key: String,
    pub status_json: String,
    pub checked_at: DateTime<Utc>,
}

/// A user-authored rule set definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleSet {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub enabled: bool,
    pub priority: i32,
    /// Facets this rule applies to. Empty = all facets.
    pub applied_facets: Vec<MediaFacet>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_managed: bool,
    pub managed_key: Option<String>,
    /// Tags a managed pack narrows itself to. `None` or empty means the pack
    /// applies wherever its facts match; tags restrict it to titles carrying
    /// one of them. Only meaningful for managed rows.
    pub managed_tag_filter: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserAccountKind {
    #[default]
    Local,
    ExternalAutoProvisioned,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserLoginStatus {
    #[default]
    Enabled,
    Disabled,
}

impl UserLoginStatus {
    pub const fn as_storage_str(self) -> &'static str {
        match self {
            Self::Enabled => "active",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse_storage(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

impl UserAccountKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::ExternalAutoProvisioned => "external_auto_provisioned",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "external_auto_provisioned" => Some(Self::ExternalAutoProvisioned),
            _ => None,
        }
    }

    pub fn allows_local_credentials(&self) -> bool {
        matches!(self, Self::Local)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: Option<String>,
    #[serde(default)]
    pub password_change_required: bool,
    #[serde(default)]
    pub account_kind: UserAccountKind,
    #[serde(default)]
    pub authorization: UserAuthorization,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaServerProvider {
    Jellyfin,
    Plex,
    Emby,
}

impl MediaServerProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jellyfin => "jellyfin",
            Self::Plex => "plex",
            Self::Emby => "emby",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "jellyfin" => Some(Self::Jellyfin),
            "plex" => Some(Self::Plex),
            "emby" => Some(Self::Emby),
            _ => None,
        }
    }

    pub fn supports_external_auth(&self) -> bool {
        matches!(self, Self::Jellyfin | Self::Plex | Self::Emby)
    }

    pub fn external_account_provider(&self) -> Option<ExternalAccountProvider> {
        match self {
            Self::Jellyfin => Some(ExternalAccountProvider::Jellyfin),
            Self::Plex => Some(ExternalAccountProvider::Plex),
            Self::Emby => Some(ExternalAccountProvider::Emby),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaServerPathMapping {
    pub source_path: String,
    pub destination_path: String,
    pub sort_order: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaServerDefaultLibraryGrant {
    pub library_id: String,
    pub permissions: LibraryPermissionMask,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaServerConnection {
    pub id: String,
    pub provider: MediaServerProvider,
    pub display_name: String,
    pub base_url: String,
    /// Browser-facing server URL used only for playback deep links.
    #[serde(default)]
    pub external_url: Option<String>,
    pub enabled: bool,
    pub login_enabled: bool,
    pub linking_enabled: bool,
    pub auto_add_enabled: bool,
    pub default_app_permissions: AppPermissionMask,
    pub default_library_grants: Vec<MediaServerDefaultLibraryGrant>,
    pub machine_id: Option<String>,
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    pub emby_server_id: Option<String>,
    pub emby_connect_enabled: bool,
    pub path_mappings: Vec<MediaServerPathMapping>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for MediaServerConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MediaServerConnection")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("display_name", &self.display_name)
            .field("base_url", &self.base_url)
            .field("external_url", &self.external_url)
            .field("enabled", &self.enabled)
            .field("login_enabled", &self.login_enabled)
            .field("linking_enabled", &self.linking_enabled)
            .field("auto_add_enabled", &self.auto_add_enabled)
            .field("default_app_permissions", &self.default_app_permissions)
            .field("default_library_grants", &self.default_library_grants)
            .field("machine_id", &self.machine_id)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("emby_server_id", &self.emby_server_id)
            .field("emby_connect_enabled", &self.emby_connect_enabled)
            .field("path_mappings", &self.path_mappings)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl MediaServerConnection {
    pub fn api_key_present(&self) -> bool {
        self.api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaServerPlaybackEntityKind {
    Title,
    Episode,
}

impl MediaServerPlaybackEntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Episode => "episode",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "title" => Some(Self::Title),
            "episode" => Some(Self::Episode),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaServerPlaybackItem {
    pub connection_id: String,
    pub entity_kind: MediaServerPlaybackEntityKind,
    pub entity_id: String,
    pub provider_item_id: String,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAccountProvider {
    Plex,
    Jellyfin,
    Emby,
}

impl ExternalAccountProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plex => "plex",
            Self::Jellyfin => "jellyfin",
            Self::Emby => "emby",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "plex" => Some(Self::Plex),
            "jellyfin" => Some(Self::Jellyfin),
            "emby" => Some(Self::Emby),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAccountStatus {
    PendingClaim,
    Active,
    Disabled,
}

impl ExternalAccountStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PendingClaim => "pending_claim",
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pending_claim" => Some(Self::PendingClaim),
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserExternalAccount {
    pub id: String,
    pub user_id: String,
    pub provider: ExternalAccountProvider,
    pub connection_id: String,
    pub external_user_id: Option<String>,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub status: ExternalAccountStatus,
    pub verified_at: Option<DateTime<Utc>>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl UserExternalAccount {
    pub fn pending_claim(
        user_id: impl Into<String>,
        provider: ExternalAccountProvider,
        connection_id: impl Into<String>,
        external_user_id: Option<String>,
        username: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new().0,
            user_id: user_id.into(),
            provider,
            connection_id: connection_id.into(),
            external_user_id,
            username: username.into(),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::PendingClaim,
            verified_at: None,
            last_login_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

impl User {
    pub const SYSTEM_EXECUTION_ID: &'static str = "__scryer_system__";

    pub const fn login_status(&self) -> UserLoginStatus {
        self.authorization.login_status
    }

    pub fn set_login_status(&mut self, status: UserLoginStatus) {
        self.authorization.login_status = status;
    }

    pub fn new_admin(username: impl Into<String>) -> Self {
        Self {
            id: Id::new().0,
            username: username.into(),
            password_hash: None,
            password_change_required: false,
            account_kind: UserAccountKind::Local,
            authorization: UserAuthorization::full_admin(),
        }
    }

    pub fn system_execution_actor() -> Self {
        Self {
            id: Self::SYSTEM_EXECUTION_ID.to_string(),
            username: "System".to_string(),
            password_hash: None,
            password_change_required: false,
            account_kind: UserAccountKind::Local,
            authorization: UserAuthorization::full_admin(),
        }
    }

    pub fn is_system_execution_actor(&self) -> bool {
        self.id == Self::SYSTEM_EXECUTION_ID
    }

    pub fn with_password_hash(
        username: impl Into<String>,
        password_hash: impl Into<String>,
    ) -> Self {
        Self {
            id: Id::new().0,
            username: username.into(),
            password_hash: Some(password_hash.into()),
            password_change_required: false,
            account_kind: UserAccountKind::Local,
            authorization: UserAuthorization::full_admin(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewUser {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("repository error: {0}")]
    Repository(String),
}

pub type DomainResult<T> = Result<T, DomainError>;

/// Indexer capabilities declared by a plugin. Used by the dispatcher to skip
/// indexers that don't support a given search type.
fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexerProtocolCapability {
    Usenet,
    Torrent,
    Mixed,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexerFeedModeCapability {
    Recent,
    Rss,
    AutomaticSearch,
    InteractiveSearch,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexerSearchInputCapability {
    TextQuery,
    TitleQuery,
    IdQuery,
    AggregateIdQuery,
    Season,
    Episode,
    AbsoluteEpisode,
    AirDate,
    SpecialEpisodeTitle,
    Category,
    Offset,
    Limit,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexerCategoryValueKind {
    Numeric,
    #[default]
    String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerCategoryDescriptor {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub value_kind: IndexerCategoryValueKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerCategoryModel {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub value_kinds: Vec<IndexerCategoryValueKind>,
    #[serde(default)]
    pub separate_anime_categories: bool,
    #[serde(default)]
    pub provider_category_metadata: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<IndexerCategoryDescriptor>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerCapsSearchNode {
    #[serde(default)]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_engine: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerCapsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits_default: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits_max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<IndexerCapsSearchNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tv_search: Option<IndexerCapsSearchNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movie_search: Option<IndexerCapsSearchNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_search: Option<IndexerCapsSearchNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_search: Option<IndexerCapsSearchNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_search: Option<IndexerCapsSearchNode>,
    #[serde(default)]
    pub categories: IndexerCategoryModel,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerLimitCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_page_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_hint_seconds: Option<u32>,
    #[serde(default)]
    pub api_quota_supported: bool,
    #[serde(default)]
    pub grab_quota_supported: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerTorrentCapabilities {
    #[serde(default)]
    pub reports_seeders: bool,
    #[serde(default)]
    pub reports_peers: bool,
    #[serde(default)]
    pub reports_leechers: bool,
    #[serde(default)]
    pub reports_info_hash: bool,
    #[serde(default)]
    pub reports_magnet_uri: bool,
    #[serde(default)]
    pub reports_volume_factors: bool,
    #[serde(default)]
    pub supports_private_tracker_flags: bool,
    #[serde(default)]
    pub supports_seed_requirements: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerResponseFeatures {
    #[serde(default)]
    pub languages: bool,
    #[serde(default)]
    pub subtitles: bool,
    #[serde(default)]
    pub grabs: bool,
    #[serde(default)]
    pub votes: bool,
    #[serde(default)]
    pub comments: bool,
    #[serde(default)]
    pub info_url: bool,
    #[serde(default)]
    pub guid: bool,
    #[serde(default)]
    pub raw_provider_metadata: bool,
    #[serde(default)]
    pub password_hint: bool,
    #[serde(default)]
    pub protection_hint: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerProviderCapabilities {
    #[serde(default = "default_true")]
    pub rss: bool,

    /// Which search facets this indexer supports, and which well-known IDs
    /// it can search on for each facet. Values must be from the core vocabulary:
    /// `"imdb_id"`, `"tvdb_id"`, `"anidb_id"` — matching the field names on
    /// `PluginSearchRequest`. The plugin maps these to its own query format
    /// internally (for example, an anime indexer may map `anidb_id` to its own
    /// provider-specific ID parameter).
    ///
    /// Examples:
    ///   NZBGeek:    {"movie": ["imdb_id"], "series": ["tvdb_id"]}
    ///   Id-only anime indexer: {"anime": ["anidb_id"], "movie": ["anidb_id"]}
    ///   RSS:        {} (empty — feed-only, no structured search)
    #[serde(default)]
    pub supported_ids: HashMap<String, Vec<String>>,

    /// Does this indexer index all title aliases internally?
    /// When true, the search orchestrator does NOT send alias title variants.
    #[serde(default)]
    pub deduplicates_aliases: bool,

    /// Query param name for season filtering, if supported.
    /// e.g. Some("season") → appends &season=1
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_param: Option<String>,

    /// Query param name for episode filtering, if supported.
    /// e.g. Some("ep") → appends &ep=5
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_param: Option<String>,

    /// Query param name for freetext search, if supported.
    /// e.g. Some("q") → appends &q=Demon+Slayer+S01E01
    /// None → indexer does not accept freetext queries (RSS-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_param: Option<String>,
    /// Facets for which the provider can accept title/freetext searches.
    ///
    /// Empty means legacy inference: ID-declared facets are query-capable, and
    /// query-only providers with no `supported_ids` are assumed to accept all
    /// current search facets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_query_facets: Vec<String>,

    // -- Legacy boolean fields kept for backward compat during migration.
    // -- New code should use supported_ids / query_param instead.
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub imdb_search: bool,
    #[serde(default)]
    pub tvdb_search: bool,
    #[serde(default)]
    pub anidb_search: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<IndexerProtocolCapability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feed_modes: Vec<IndexerFeedModeCapability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_inputs: Vec<IndexerSearchInputCapability>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_external_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_model: Option<IndexerCategoryModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<IndexerLimitCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<IndexerTorrentCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_features: Option<IndexerResponseFeatures>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexerManagementCapabilities {
    #[serde(default)]
    pub supports_validate_config: bool,
    #[serde(default)]
    pub supports_managed_children_sync: bool,
}

impl IndexerProviderCapabilities {
    pub const QUERY_FACETS: [&'static str; 3] = ["movie", "series", "anime"];

    /// Whether this indexer supports any structured or freetext search at all.
    pub fn supports_any_search(&self) -> bool {
        self.query_param.is_some() || !self.supported_ids.is_empty() || self.search
    }

    /// Whether this indexer can accept a title/freetext query for `facet`.
    pub fn supports_query_for_facet(&self, facet: &str) -> bool {
        if self.query_param.is_none() {
            return false;
        }

        let facet = facet.trim();
        if facet.is_empty() {
            return false;
        }

        if !self.supported_query_facets.is_empty() {
            return self
                .supported_query_facets
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(facet));
        }

        if self.supported_ids.is_empty() {
            return Self::QUERY_FACETS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(facet));
        }

        self.has_facet(facet)
    }

    /// Whether this indexer has any ID types for the given facet.
    pub fn has_facet(&self, facet: &str) -> bool {
        self.supported_ids
            .get(facet)
            .is_some_and(|ids| !ids.is_empty())
    }

    /// Get the supported ID types for a given facet.
    pub fn id_types_for_facet(&self, facet: &str) -> &[String] {
        self.supported_ids
            .get(facet)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    #[default]
    String,
    #[serde(alias = "secret")]
    Password,
    Multiline,
    Bool,
    Select,
    Number,
    Path,
    Tag,
}

impl ConfigFieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Password => "password",
            Self::Multiline => "multiline",
            Self::Bool => "bool",
            Self::Select => "select",
            Self::Number => "number",
            Self::Path => "path",
            Self::Tag => "tag",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "string" => Some(Self::String),
            "password" | "secret" => Some(Self::Password),
            "multiline" => Some(Self::Multiline),
            "bool" => Some(Self::Bool),
            "select" => Some(Self::Select),
            "number" => Some(Self::Number),
            "path" => Some(Self::Path),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldValueSource {
    #[default]
    User,
    HostBinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldRole {
    ConnectionUrl,
}

impl ConfigFieldRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionUrl => "connection_url",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "connection_url" => Some(Self::ConnectionUrl),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginHostBindingId {
    #[serde(rename = "smg.opensubtitles_api_key")]
    SmgOpenSubtitlesApiKey,
}

impl PluginHostBindingId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SmgOpenSubtitlesApiKey => "smg.opensubtitles_api_key",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "smg.opensubtitles_api_key" => Some(Self::SmgOpenSubtitlesApiKey),
            _ => None,
        }
    }
}

/// Describes a single configuration field a plugin expects.
/// Used by the plugin system to advertise what config keys are needed,
/// and by the frontend to render dynamic form fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigFieldDef {
    /// Config key name (e.g. "custom_endpoint"). Used as the JSON key in
    /// `config_json` and the Extism config key.
    pub key: String,
    /// Human-readable label for the form field.
    pub label: String,
    /// Field type: "string", "password", "multiline", "bool", "select", "number".
    pub field_type: ConfigFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default)]
    pub value_source: ConfigFieldValueSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ConfigFieldRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_binding: Option<PluginHostBindingId>,
    /// For "select" fields: the available options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ConfigFieldOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

/// A single option for "select"-type config fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigFieldOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config_overrides: BTreeMap<String, String>,
}

// ── Notification types ──────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelType(String);

impl ChannelType {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return None;
        }
        Some(Self(normalized))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationChannelConfig {
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub config_json: String,
    pub media_server_connection_id: Option<String>,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewNotificationChannelConfig {
    pub name: String,
    pub channel_type: ChannelType,
    pub config_json: String,
    pub media_server_connection_id: Option<String>,
    pub is_enabled: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTargetKind {
    PluginChannel,
    MediaServerConnection,
}

impl NotificationTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PluginChannel => "plugin_channel",
            Self::MediaServerConnection => "media_server_connection",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "plugin_channel" => Some(Self::PluginChannel),
            "media_server_connection" => Some(Self::MediaServerConnection),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationTarget {
    pub id: String,
    pub target_kind: NotificationTargetKind,
    pub name: String,
    pub provider_type: String,
    pub media_server_provider: Option<MediaServerProvider>,
    pub media_server_connection_id: Option<String>,
    pub is_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotificationSubscription {
    pub id: String,
    pub channel_id: Option<String>,
    pub target_kind: NotificationTargetKind,
    pub target_id: String,
    pub event_type: NotificationEventType,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewNotificationSubscription {
    pub channel_id: Option<String>,
    pub target_kind: NotificationTargetKind,
    pub target_id: String,
    pub event_type: NotificationEventType,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
}

/// All notification event types supported by the system.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEventType {
    Grab,
    Download,
    Upgrade,
    ImportComplete,
    ImportRejected,
    Rename,
    TitleAdded,
    TitleDeleted,
    FileDeleted,
    FileDeletedForUpgrade,
    PostProcessingCompleted,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    MediaRequestSubmitted,
    MediaRequestApproved,
    MediaRequestRejected,
    MediaRequestCanceled,
    HealthIssue,
    HealthRestored,
    ApplicationUpdate,
    ManualInteractionRequired,
    Test,
}

impl NotificationEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grab => "grab",
            Self::Download => "download",
            Self::Upgrade => "upgrade",
            Self::ImportComplete => "import_complete",
            Self::ImportRejected => "import_rejected",
            Self::Rename => "rename",
            Self::TitleAdded => "title_added",
            Self::TitleDeleted => "title_deleted",
            Self::FileDeleted => "file_deleted",
            Self::FileDeletedForUpgrade => "file_deleted_for_upgrade",
            Self::PostProcessingCompleted => "post_processing_completed",
            Self::SubtitleDownloaded => "subtitle_downloaded",
            Self::SubtitleSearchFailed => "subtitle_search_failed",
            Self::MediaRequestSubmitted => "media_request_submitted",
            Self::MediaRequestApproved => "media_request_approved",
            Self::MediaRequestRejected => "media_request_rejected",
            Self::MediaRequestCanceled => "media_request_canceled",
            Self::HealthIssue => "health_issue",
            Self::HealthRestored => "health_restored",
            Self::ApplicationUpdate => "application_update",
            Self::ManualInteractionRequired => "manual_interaction_required",
            Self::Test => "test",
        }
    }

    pub fn all() -> &'static [NotificationEventType] {
        &[
            Self::Grab,
            Self::Download,
            Self::Upgrade,
            Self::ImportComplete,
            Self::ImportRejected,
            Self::Rename,
            Self::TitleAdded,
            Self::TitleDeleted,
            Self::FileDeleted,
            Self::FileDeletedForUpgrade,
            Self::PostProcessingCompleted,
            Self::SubtitleDownloaded,
            Self::SubtitleSearchFailed,
            Self::MediaRequestSubmitted,
            Self::MediaRequestApproved,
            Self::MediaRequestRejected,
            Self::MediaRequestCanceled,
            Self::HealthIssue,
            Self::HealthRestored,
            Self::ApplicationUpdate,
            Self::ManualInteractionRequired,
            Self::Test,
        ]
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "grab" => Some(Self::Grab),
            "download" => Some(Self::Download),
            "upgrade" => Some(Self::Upgrade),
            "import_complete" => Some(Self::ImportComplete),
            "import_rejected" => Some(Self::ImportRejected),
            "rename" => Some(Self::Rename),
            "title_added" => Some(Self::TitleAdded),
            "title_deleted" => Some(Self::TitleDeleted),
            "file_deleted" => Some(Self::FileDeleted),
            "file_deleted_for_upgrade" => Some(Self::FileDeletedForUpgrade),
            "post_processing_completed" => Some(Self::PostProcessingCompleted),
            "subtitle_downloaded" => Some(Self::SubtitleDownloaded),
            "subtitle_search_failed" => Some(Self::SubtitleSearchFailed),
            "media_request_submitted" => Some(Self::MediaRequestSubmitted),
            "media_request_approved" => Some(Self::MediaRequestApproved),
            "media_request_rejected" => Some(Self::MediaRequestRejected),
            "media_request_canceled" => Some(Self::MediaRequestCanceled),
            "health_issue" => Some(Self::HealthIssue),
            "health_restored" => Some(Self::HealthRestored),
            "application_update" => Some(Self::ApplicationUpdate),
            "manual_interaction_required" => Some(Self::ManualInteractionRequired),
            "test" => Some(Self::Test),
            "release_grabbed" => Some(Self::Grab),
            "download_failed" => Some(Self::Download),
            "media_file_upgraded" => Some(Self::Upgrade),
            "import_completed" => Some(Self::ImportComplete),
            "media_file_renamed" => Some(Self::Rename),
            "media_file_deleted" => Some(Self::FileDeleted),
            _ => None,
        }
    }
}

impl std::str::FromStr for NotificationEventType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(())
    }
}

// ── Post-Processing Scripts ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    #[default]
    Inline,
    File,
}

impl ScriptType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::File => "file",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "inline" => Some(Self::Inline),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Blocking,
    FireAndForget,
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocking => "blocking",
            Self::FireAndForget => "fire_and_forget",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "blocking" => Some(Self::Blocking),
            "fire_and_forget" => Some(Self::FireAndForget),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostProcessingScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub script_type: ScriptType,
    pub script_content: String,
    pub applied_facets: Vec<String>,
    pub execution_mode: ExecutionMode,
    pub timeout_secs: i64,
    pub priority: i32,
    pub enabled: bool,
    pub debug: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptRunStatus {
    Success,
    Failed,
    Timeout,
}

impl ScriptRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Self::Success),
            "failed" => Some(Self::Failed),
            "timeout" => Some(Self::Timeout),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostProcessingScriptRun {
    pub id: String,
    pub script_id: String,
    pub script_name: String,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub facet: Option<String>,
    pub file_path: Option<String>,
    pub status: ScriptRunStatus,
    pub exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub duration_ms: Option<i64>,
    pub env_payload_json: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

pub fn parse_query(value: &str) -> String {
    value.trim().to_lowercase()
}

pub fn match_fuzzy(candidate: &str, query: &str) -> bool {
    let target = parse_query(candidate);
    let q = parse_query(query);
    if q.is_empty() {
        return true;
    }
    target.contains(&q)
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut output = HashSet::new();
    for tag in tags {
        let trimmed = tag.trim();
        if !trimmed.is_empty() {
            // Preserve case for structured scryer: tags (they may contain paths)
            if trimmed.starts_with("scryer:") {
                output.insert(trimmed.to_string());
            } else {
                output.insert(trimmed.to_lowercase());
            }
        }
    }
    let mut ordered: Vec<String> = output.into_iter().collect();
    ordered.sort_unstable();
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_round_trip() {
        let id = Id::new();
        assert!(!id.0.is_empty());
    }

    #[test]
    fn direct_nab_provider_family_includes_specialized_newznab_indexers() {
        for provider_type in [
            "amenzb",
            "animetosho-xyz",
            "dognzb",
            "newznab",
            "nzbgeek",
            "torznab",
        ] {
            assert!(is_nab_provider_type(provider_type), "{provider_type}");
        }
        assert!(!is_nab_provider_type("aninzb"));
        assert!(!is_nab_provider_type("torrent-rss"));
    }

    #[test]
    fn tags_normalize() {
        assert_eq!(
            normalize_tags(&["Anime".into(), "anime".into(), " series ".into()]),
            vec!["anime".to_string(), "series".to_string()]
        );
    }

    #[test]
    fn fuzzy_search_matches_partial() {
        assert!(match_fuzzy("Velvet Comet", "come"));
        assert!(!match_fuzzy("Velvet Comet", "harbor"));
    }

    #[test]
    fn admin_has_full_permission_masks() {
        let admin = User::new_admin("root");
        assert!(
            admin
                .authorization
                .app
                .contains(AppPermissionMask::MANAGE_SYSTEM_SETTINGS)
        );
        assert!(
            admin
                .authorization
                .app
                .contains(AppPermissionMask::MANAGE_USERS)
        );
    }
}

// ── Subtitle management ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExternalSubtitleSourceKind {
    Downloaded,
    Discovered,
}

impl ExternalSubtitleSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Downloaded => "downloaded",
            Self::Discovered => "discovered",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "downloaded" => Some(Self::Downloaded),
            "discovered" => Some(Self::Discovered),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtitleDownload {
    pub id: String,
    pub media_file_id: String,
    pub title_id: String,
    pub episode_id: Option<String>,
    pub source_kind: ExternalSubtitleSourceKind,
    pub language: String,
    pub provider: Option<String>,
    pub provider_file_id: Option<String>,
    pub file_path: String,
    pub score: Option<i32>,
    pub hearing_impaired: bool,
    pub forced: bool,
    pub ai_translated: bool,
    pub machine_translated: bool,
    pub uploader: Option<String>,
    pub release_info: Option<String>,
    pub synced: bool,
    pub downloaded_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubtitleBlocklistEntry {
    pub id: String,
    pub media_file_id: String,
    pub provider: String,
    pub provider_file_id: String,
    pub language: String,
    pub reason: Option<String>,
    pub created_at: String,
}

#[cfg(test)]
#[path = "domain_tests.rs"]
mod domain_tests;
