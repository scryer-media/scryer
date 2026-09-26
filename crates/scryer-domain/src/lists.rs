//! Lists: public and personal list subscriptions and everything that hangs off
//! them.
//!
//! A subscription names a *source* (a provider plus what to read from it), the
//! *routes* that say where each kind of title goes, and the *mode* that says
//! what to do with a title the list contains and the library does not. The sync
//! engine turns the provider's items into [`ListMembership`] rows, one per
//! provider-native item id, and acts through the existing catalog and request
//! use cases. Nothing here removes a title: leaving a list is recorded and
//! surfaced, never acted on destructively.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{ExternalId, Id, MediaFacet};

/// Which side of the privacy boundary a subscription sits on.
///
/// `Public` rows are the instance's: anyone who can view a catalog sees them
/// and managers change them. `Personal` rows belong to one member and are read
/// through owner-scoped queries only.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListScope {
    #[default]
    Public,
    Personal,
}

impl ListScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Personal => "personal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "public" => Some(Self::Public),
            "personal" => Some(Self::Personal),
            _ => None,
        }
    }
}

/// What a sync does with a list item the library does not have.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListMode {
    /// Create the title monitored and search for it right away.
    #[default]
    Search,
    /// Create the title monitored and let the normal wanted cycle find it.
    Add,
    /// Submit a request held for review, whatever the request rules say.
    Hold,
    /// Submit a request as the member, subject to their list policy.
    Request,
    /// Record the item for the Discover rail only.
    Discover,
}

impl ListMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Add => "add",
            Self::Hold => "hold",
            Self::Request => "request",
            Self::Discover => "discover",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "search" => Some(Self::Search),
            "add" => Some(Self::Add),
            "hold" => Some(Self::Hold),
            "request" => Some(Self::Request),
            "discover" => Some(Self::Discover),
            _ => None,
        }
    }
}

/// What happens to a title the list added when the item leaves the list.
///
/// There is deliberately no removal variant. Removing titles a list no longer
/// wants is a maintenance rule with its own approval boundary; Lists only
/// records the departure and exposes it as facts.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListOnLeave {
    #[default]
    Keep,
    Log,
    Unmonitor,
    Tag,
}

impl ListOnLeave {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Log => "log",
            Self::Unmonitor => "unmonitor",
            Self::Tag => "tag",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "keep" => Some(Self::Keep),
            "log" => Some(Self::Log),
            "unmonitor" => Some(Self::Unmonitor),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

/// The subscription's health as of its last sync.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListSyncState {
    Ok,
    /// Subscribed, never synced.
    #[default]
    New,
    Fail,
    /// Disabled, or a personal list whose owner's policy is `None`.
    Off,
}

impl ListSyncState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::New => "new",
            Self::Fail => "fail",
            Self::Off => "off",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ok" => Some(Self::Ok),
            "new" => Some(Self::New),
            "fail" => Some(Self::Fail),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Where a subscription's items come from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ListSourceOrigin {
    /// A parameterless chart the metadata gateway already observes; read from
    /// the gateway, never from the provider.
    SmgChart { chart_key: String, scope: String },
    /// Fetched by the provider plugin with the subscription's parameters.
    ProviderFetch,
    /// A public IMDb list the metadata gateway proxies; IMDb has no API and
    /// the instance never reads it directly. The list id is the source's
    /// `list_id` parameter.
    SmgImdbList,
}

impl ListSourceOrigin {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SmgChart { .. } => "smg_chart",
            Self::ProviderFetch => "provider_fetch",
            Self::SmgImdbList => "smg_imdb_list",
        }
    }
}

/// The source parameter naming a proxied IMDb list.
pub const LIST_SOURCE_IMDB_LIST_ID_PARAM: &str = "list_id";

/// A provider plus the concrete thing to read from it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSource {
    /// The plugin's `provider_type`: `trakt`, `tmdb`, `imdb`, `anilist`,
    /// `mal`, `plex`, `simkl`, `mdblist`, `custom`.
    pub provider: String,
    /// Provider-defined: `chart:trending`, `user_list`, `person`, `watchlist`,
    /// `status:plan_to_watch`, `rss`, ...
    pub source_type: String,
    /// Validated against the provider manifest's parameter schema.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    pub origin: ListSourceOrigin,
}

/// Where titles of one kind go. A subscription carries at most one route per
/// kind; a kind without a route is filtered rather than guessed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListRoute {
    pub kind: MediaFacet,
    pub library_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_folder_id: Option<String>,
    /// The existing monitor-type vocabulary.
    pub monitor_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_availability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_season_folders: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_numbering: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// A per-subscription item filter, re-evaluated on every sync.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "filter", rename_all = "snake_case")]
pub enum ListFilter {
    RatingAtLeast {
        scale: String,
        value: f64,
    },
    ReleaseYear {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<i32>,
    },
    ExcludeGenres {
        genres: Vec<String>,
    },
    Format {
        formats: Vec<String>,
    },
    Language {
        languages: Vec<String>,
    },
    SkipOnMyStreamingServices,
    ReleasedOnly,
    DirectorCreditsOnly,
    NotSequelWithoutBase,
}

/// The sync bookkeeping a subscription carries.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSyncStatus {
    pub state: ListSyncState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_at: Option<DateTime<Utc>>,
    /// Set by a provider's rate limit; the sync skips the subscription until
    /// then. Capped by the engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<DateTime<Utc>>,
    /// The provider's change fingerprint from the last fetch, so an unchanged
    /// list is a touch rather than a re-evaluation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_fingerprint: Option<String>,
}

/// The coverage bar. Every item is in exactly one bucket except `total`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListCounts {
    pub total: u64,
    pub in_library: u64,
    pub added: u64,
    pub requested: u64,
    pub held: u64,
    pub filtered: u64,
    pub excluded: u64,
    pub unresolved: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ListSubscription {
    pub id: String,
    pub scope: ListScope,
    /// Always set. For a public subscription this is the manager who
    /// subscribed; for a personal one it is the member whose list it is.
    pub owner_user_id: String,
    pub source: ListSource,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_url: Option<String>,
    /// The kinds the list can contain, as declared by the manifest item.
    pub kinds: Vec<MediaFacet>,
    pub enabled: bool,
    pub mode: ListMode,
    pub routes: Vec<ListRoute>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<ListFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_sync: Option<u32>,
    pub on_leave: ListOnLeave,
    /// Fixed per provider by the manifest; stored so a manifest change does
    /// not silently re-pace an existing subscription.
    pub interval_seconds: i64,
    pub sync: ListSyncStatus,
    pub counts: ListCounts,
    /// The member's provider account a personal subscription reads with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ListSubscription {
    pub fn route_for(&self, kind: MediaFacet) -> Option<&ListRoute> {
        self.routes.iter().find(|route| route.kind == kind)
    }

    pub fn is_personal(&self) -> bool {
        self.scope == ListScope::Personal
    }
}

/// What the engine last decided about one list item.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListMembershipState {
    InLibrary,
    Added,
    Requested,
    Held,
    /// A held or requested personal-list item whose request was rejected.
    /// Not re-requested until the item leaves the list and returns.
    Rejected,
    Filtered,
    Excluded,
    #[default]
    Unresolved,
    /// The member may not request into the routed library, or is disabled.
    BlockedPermission,
    Discover,
    /// A resolved candidate the engine has not acted on yet: it fell beyond
    /// this sync's cap, or its action failed and is retried next sync. Not
    /// counted as filtered.
    Pending,
}

impl ListMembershipState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InLibrary => "in_library",
            Self::Added => "added",
            Self::Requested => "requested",
            Self::Held => "held",
            Self::Rejected => "rejected",
            Self::Filtered => "filtered",
            Self::Excluded => "excluded",
            Self::Unresolved => "unresolved",
            Self::BlockedPermission => "blocked_permission",
            Self::Discover => "discover",
            Self::Pending => "pending",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "in_library" => Some(Self::InLibrary),
            "added" => Some(Self::Added),
            "requested" => Some(Self::Requested),
            "held" => Some(Self::Held),
            "rejected" => Some(Self::Rejected),
            "filtered" => Some(Self::Filtered),
            "excluded" => Some(Self::Excluded),
            "unresolved" => Some(Self::Unresolved),
            "blocked_permission" => Some(Self::BlockedPermission),
            "discover" => Some(Self::Discover),
            "pending" => Some(Self::Pending),
            _ => None,
        }
    }

    /// States that hold their item: a later sync keeps them rather than
    /// re-deciding.
    pub const fn is_settled(self) -> bool {
        matches!(
            self,
            Self::Added | Self::Requested | Self::Held | Self::Rejected | Self::Discover
        )
    }
}

/// One row per (subscription, provider-native item).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListMembership {
    pub subscription_id: String,
    /// The provider's own id for the item, stable across syncs.
    pub item_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i32>,
    /// The title the list shows for the item, as the provider named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_ids: Vec<ExternalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smg_title_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub kind: MediaFacet,
    pub state: ListMembershipState,
    /// Plain-words reason for `Filtered`, `Excluded`, `Unresolved`, or
    /// `BlockedPermission`; never a list name or a title from another list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_reason: Option<String>,
    /// The list created this title, so on-leave may act on it.
    pub added_by_list: bool,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_at: Option<DateTime<Utc>>,
    pub left_handled: bool,
}

/// Which subscriptions an exclusion applies to. Exclusions are instance-level;
/// the `List` scope names a public subscription only.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ListExclusionScope {
    AllLists,
    List { subscription_id: String },
}

impl ListExclusionScope {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AllLists => "all_lists",
            Self::List { .. } => "list",
        }
    }

    pub fn subscription_id(&self) -> Option<&str> {
        match self {
            Self::AllLists => None,
            Self::List { subscription_id } => Some(subscription_id),
        }
    }
}

/// Where a media request came from.
///
/// A list-originated request carries the subscription that submitted it, so
/// rejecting it can be remembered against that list. Personal origins are the
/// member's own; readers outside the owner see their kind only.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaRequestOrigin {
    #[default]
    Manual,
    PublicList {
        subscription_id: String,
    },
    PersonalList {
        subscription_id: String,
    },
}

impl MediaRequestOrigin {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PublicList { .. } => "public_list",
            Self::PersonalList { .. } => "personal_list",
        }
    }

    pub fn subscription_id(&self) -> Option<&str> {
        match self {
            Self::Manual => None,
            Self::PublicList { subscription_id } | Self::PersonalList { subscription_id } => {
                Some(subscription_id)
            }
        }
    }

    /// The origin a stored `(kind, subscription id)` pair names. An unknown
    /// kind, or a list kind that lost its subscription id, reads as manual.
    pub fn from_parts(kind: &str, subscription_id: Option<String>) -> Self {
        match (kind, subscription_id) {
            ("public_list", Some(subscription_id)) => Self::PublicList { subscription_id },
            ("personal_list", Some(subscription_id)) => Self::PersonalList { subscription_id },
            _ => Self::Manual,
        }
    }

    /// The origin a subscription's requests carry.
    pub fn for_subscription(subscription: &ListSubscription) -> Self {
        match subscription.scope {
            ListScope::Public => Self::PublicList {
                subscription_id: subscription.id.clone(),
            },
            ListScope::Personal => Self::PersonalList {
                subscription_id: subscription.id.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListExclusion {
    pub id: String,
    pub kind: MediaFacet,
    /// The same id fingerprint the request pipeline matches on.
    pub external_ids: Vec<ExternalId>,
    pub display_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    pub scope: ListExclusionScope,
    /// `None` once the creating user is deleted; the exclusion outlives them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_user_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ListExclusion {
    /// Whether this exclusion covers an item of `kind` with `external_ids`
    /// seen on `subscription_id`.
    pub fn matches(
        &self,
        kind: MediaFacet,
        external_ids: &[ExternalId],
        subscription_id: &str,
    ) -> bool {
        if self.kind != kind {
            return false;
        }
        if let ListExclusionScope::List {
            subscription_id: scoped,
        } = &self.scope
            && scoped != subscription_id
        {
            return false;
        }
        external_ids.iter().any(|candidate| {
            self.external_ids.iter().any(|excluded| {
                excluded.source.eq_ignore_ascii_case(&candidate.source)
                    && excluded.value.eq_ignore_ascii_case(&candidate.value)
            })
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum UserListAccountStatus {
    #[default]
    Active,
    Expired,
    Revoked,
}

impl UserListAccountStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "expired" => Some(Self::Expired),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// The token material a member's provider link holds. Stored encrypted with
/// the datastore key; decrypted only in memory for a fetch or a renewal, and
/// never projected to any API.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListAccountCredential {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// A member's link to a list provider. Distinct from a login-provider account:
/// this grants Scryer a read of the member's lists, nothing else.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserListAccount {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub external_user_id: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub credential: ListAccountCredential,
    pub status: UserListAccountStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub linked_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// How a member's personal-list requests are admitted.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListPolicy {
    /// Approved under the system actor when the request rules allow it.
    Auto,
    /// Submitted as an ordinary request for review.
    #[default]
    Approval,
    /// Personal lists sync nothing for this member.
    None,
}

impl ListPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Approval => "approval",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "approval" => Some(Self::Approval),
            "none" => Some(Self::None),
            _ => Option::None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserListPolicy {
    pub user_id: String,
    pub policy: ListPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by_user_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListSyncRunOutcome {
    #[default]
    Succeeded,
    Failed,
    /// Not due, paused, or the owner's policy is `None`.
    Skipped,
}

impl ListSyncRunOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// One sync of one subscription.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSyncRun {
    pub id: String,
    pub subscription_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_run_id: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: ListSyncRunOutcome,
    pub counts: ListCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl ListSyncRun {
    pub fn started(subscription_id: impl Into<String>, job_run_id: Option<String>) -> Self {
        Self {
            id: Id::new().0,
            subscription_id: subscription_id.into(),
            job_run_id,
            started_at: Utc::now(),
            finished_at: None,
            outcome: ListSyncRunOutcome::Succeeded,
            counts: ListCounts::default(),
            error_message: None,
        }
    }
}

/// The public list an event is about. Events are only written for public
/// subscriptions, so the name is safe to show to anyone who reads the feed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListEventSubject {
    pub subscription_id: String,
    pub list_name: String,
    pub provider: String,
}

impl ListEventSubject {
    pub fn of(subscription: &ListSubscription) -> Self {
        Self {
            subscription_id: subscription.id.clone(),
            list_name: subscription.name.clone(),
            provider: subscription.source.provider.clone(),
        }
    }
}

/// A public list added a title to a library.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListTitleAddedEventData {
    pub list: ListEventSubject,
    pub title: crate::TitleContextSnapshot,
    pub library_id: String,
    /// Whether the add also started a search.
    pub searched: bool,
}

/// A public list submitted a media request. `held` requests wait for review
/// whatever the owner's grants allow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListRequestSubmittedEventData {
    pub list: ListEventSubject,
    pub request_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    pub title_name: String,
    pub year: Option<i32>,
    pub held: bool,
}

/// A title a public list added has left that list, and what was done.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListTitleLeftEventData {
    pub list: ListEventSubject,
    pub title: crate::TitleContextSnapshot,
    pub action: ListOnLeave,
}

/// A public list's sync failed. `reason` is the sentence shown on the
/// subscription; `failure_class` is its short stable label.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSyncFailedEventData {
    pub list: ListEventSubject,
    pub reason: String,
    pub failure_class: String,
}

/// A public list was unfollowed. Titles it added stay in the library.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListUnfollowedEventData {
    pub list: ListEventSubject,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exclusion(scope: ListExclusionScope) -> ListExclusion {
        ListExclusion {
            id: "exclusion-1".into(),
            kind: MediaFacet::Movie,
            external_ids: vec![ExternalId::new("tmdb", "424242")],
            display_title: "Fixture Feature".into(),
            year: Some(2020),
            scope,
            created_by_user_id: Some("manager-1".into()),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn all_lists_exclusion_matches_any_subscription_by_case_insensitive_id() {
        let excluded = exclusion(ListExclusionScope::AllLists);
        let ids = vec![ExternalId::new("TMDB", "424242")];
        assert!(excluded.matches(MediaFacet::Movie, &ids, "sub-a"));
        assert!(excluded.matches(MediaFacet::Movie, &ids, "sub-b"));
    }

    #[test]
    fn list_scoped_exclusion_matches_only_its_subscription() {
        let excluded = exclusion(ListExclusionScope::List {
            subscription_id: "sub-a".into(),
        });
        let ids = vec![ExternalId::new("tmdb", "424242")];
        assert!(excluded.matches(MediaFacet::Movie, &ids, "sub-a"));
        assert!(!excluded.matches(MediaFacet::Movie, &ids, "sub-b"));
    }

    #[test]
    fn exclusion_never_crosses_kinds_or_unrelated_ids() {
        let excluded = exclusion(ListExclusionScope::AllLists);
        let same_id = vec![ExternalId::new("tmdb", "424242")];
        assert!(!excluded.matches(MediaFacet::Series, &same_id, "sub-a"));
        let other_id = vec![ExternalId::new("tmdb", "1")];
        assert!(!excluded.matches(MediaFacet::Movie, &other_id, "sub-a"));
    }

    #[test]
    fn storage_strings_round_trip() {
        for scope in [ListScope::Public, ListScope::Personal] {
            assert_eq!(ListScope::parse(scope.as_str()), Some(scope));
        }
        for mode in [
            ListMode::Search,
            ListMode::Add,
            ListMode::Hold,
            ListMode::Request,
            ListMode::Discover,
        ] {
            assert_eq!(ListMode::parse(mode.as_str()), Some(mode));
        }
        for on_leave in [
            ListOnLeave::Keep,
            ListOnLeave::Log,
            ListOnLeave::Unmonitor,
            ListOnLeave::Tag,
        ] {
            assert_eq!(ListOnLeave::parse(on_leave.as_str()), Some(on_leave));
        }
        for state in [
            ListMembershipState::InLibrary,
            ListMembershipState::Added,
            ListMembershipState::Requested,
            ListMembershipState::Held,
            ListMembershipState::Rejected,
            ListMembershipState::Filtered,
            ListMembershipState::Excluded,
            ListMembershipState::Unresolved,
            ListMembershipState::BlockedPermission,
            ListMembershipState::Discover,
            ListMembershipState::Pending,
        ] {
            assert_eq!(ListMembershipState::parse(state.as_str()), Some(state));
        }
        for policy in [ListPolicy::Auto, ListPolicy::Approval, ListPolicy::None] {
            assert_eq!(ListPolicy::parse(policy.as_str()), Some(policy));
        }
        for outcome in [
            ListSyncRunOutcome::Succeeded,
            ListSyncRunOutcome::Failed,
            ListSyncRunOutcome::Skipped,
        ] {
            assert_eq!(ListSyncRunOutcome::parse(outcome.as_str()), Some(outcome));
        }
    }

    #[test]
    fn settled_states_hold_their_item() {
        assert!(ListMembershipState::Added.is_settled());
        assert!(ListMembershipState::Rejected.is_settled());
        assert!(!ListMembershipState::Filtered.is_settled());
        assert!(!ListMembershipState::Unresolved.is_settled());
        assert!(!ListMembershipState::InLibrary.is_settled());
    }
}
