//! GraphQL types for public lists: the provider catalog, followed lists,
//! their memberships and sync history, previews, exclusions, and members'
//! list policies. Personal lists have no type here.

use super::{ExternalIdInput, ExternalIdPayload, MediaFacetValue, PluginConfigFieldTypeValue};
use async_graphql::{Enum, ID, InputObject, MaybeUndefined, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_application::lists::catalog::{ListAuthBadge, ListNoteTone, ListSourceParamType};
use scryer_domain::{
    ListExclusionScope, ListMembershipState, ListMode, ListOnLeave, ListPolicy, ListScope,
    ListSyncRunOutcome, ListSyncState,
};

/// Which side of the privacy boundary a list sits on.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListScope", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListScopeValue {
    /// The instance's list, visible to everyone signed in.
    Public,
    /// One member's own list.
    Personal,
}

impl ListScopeValue {
    pub fn from_domain(value: ListScope) -> Self {
        match value {
            ListScope::Public => Self::Public,
            ListScope::Personal => Self::Personal,
        }
    }
}

/// What a sync does with a list title the library does not have.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListMode", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListModeValue {
    /// Add the title monitored and search for it at once.
    Search,
    /// Add the title monitored and let the wanted cycle find it.
    Add,
    /// Submit a request that waits for review.
    Hold,
    /// Submit a request as the member. Personal lists only.
    Request,
    /// Record the title for discovery only. Not available yet.
    Discover,
}

impl ListModeValue {
    pub fn from_domain(value: ListMode) -> Self {
        match value {
            ListMode::Search => Self::Search,
            ListMode::Add => Self::Add,
            ListMode::Hold => Self::Hold,
            ListMode::Request => Self::Request,
            ListMode::Discover => Self::Discover,
        }
    }

    pub fn into_domain(self) -> ListMode {
        match self {
            Self::Search => ListMode::Search,
            Self::Add => ListMode::Add,
            Self::Hold => ListMode::Hold,
            Self::Request => ListMode::Request,
            Self::Discover => ListMode::Discover,
        }
    }
}

/// What happens to a title the list added when it leaves the list. Lists
/// never remove a title.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListOnLeave", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListOnLeaveValue {
    /// Keep the title as it is.
    Keep,
    /// Keep the title and record the departure.
    Log,
    /// Stop monitoring the title.
    Unmonitor,
    /// Tag the title.
    Tag,
}

impl ListOnLeaveValue {
    pub fn from_domain(value: ListOnLeave) -> Self {
        match value {
            ListOnLeave::Keep => Self::Keep,
            ListOnLeave::Log => Self::Log,
            ListOnLeave::Unmonitor => Self::Unmonitor,
            ListOnLeave::Tag => Self::Tag,
        }
    }

    pub fn into_domain(self) -> ListOnLeave {
        match self {
            Self::Keep => ListOnLeave::Keep,
            Self::Log => ListOnLeave::Log,
            Self::Unmonitor => ListOnLeave::Unmonitor,
            Self::Tag => ListOnLeave::Tag,
        }
    }
}

/// A list's health as of its last sync.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListSyncState", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListSyncStateValue {
    /// The last sync read the list.
    Ok,
    /// Followed and not synced yet.
    New,
    /// The last sync could not read the list.
    Fail,
    /// Turned off.
    Off,
}

impl ListSyncStateValue {
    pub fn from_domain(value: ListSyncState) -> Self {
        match value {
            ListSyncState::Ok => Self::Ok,
            ListSyncState::New => Self::New,
            ListSyncState::Fail => Self::Fail,
            ListSyncState::Off => Self::Off,
        }
    }
}

/// How a member's personal-list requests are admitted.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListPolicy", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListPolicyValue {
    /// Approved when the request rules allow it.
    Auto,
    /// Submitted for review.
    Approval,
    /// Personal lists sync nothing for this member.
    None,
}

impl ListPolicyValue {
    pub fn from_domain(value: ListPolicy) -> Self {
        match value {
            ListPolicy::Auto => Self::Auto,
            ListPolicy::Approval => Self::Approval,
            ListPolicy::None => Self::None,
        }
    }

    pub fn into_domain(self) -> ListPolicy {
        match self {
            Self::Auto => ListPolicy::Auto,
            Self::Approval => ListPolicy::Approval,
            Self::None => ListPolicy::None,
        }
    }
}

/// Which lists an exclusion applies to.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListExclusionScope", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListExclusionScopeValue {
    /// Every list.
    AllLists,
    /// One public list.
    List,
}

impl ListExclusionScopeValue {
    pub fn from_domain(value: &ListExclusionScope) -> Self {
        match value {
            ListExclusionScope::AllLists => Self::AllLists,
            ListExclusionScope::List { .. } => Self::List,
        }
    }
}

/// How one sync of one list ended.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListSyncRunOutcome", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListSyncRunOutcomeValue {
    /// The list was read and acted on.
    Succeeded,
    /// The list could not be read.
    Failed,
    /// Nothing was read.
    Skipped,
}

impl ListSyncRunOutcomeValue {
    pub fn from_domain(value: ListSyncRunOutcome) -> Self {
        match value {
            ListSyncRunOutcome::Succeeded => Self::Succeeded,
            ListSyncRunOutcome::Failed => Self::Failed,
            ListSyncRunOutcome::Skipped => Self::Skipped,
        }
    }
}

/// What the last sync decided about one list title.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListMembershipState", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListMembershipStateValue {
    /// Already in a library.
    InLibrary,
    /// Added by the list.
    Added,
    /// Requested by the list.
    Requested,
    /// Requested and waiting for review.
    Held,
    /// Its request was rejected.
    Rejected,
    /// Left out by a filter or a missing route.
    Filtered,
    /// Left out by an exclusion.
    Excluded,
    /// Not matched to a known title yet.
    Unresolved,
    /// The list's owner may not add or request into the routed library.
    BlockedPermission,
    /// Recorded for discovery only.
    Discover,
    /// Waiting for a later sync.
    Pending,
}

impl ListMembershipStateValue {
    pub fn from_domain(value: ListMembershipState) -> Self {
        match value {
            ListMembershipState::InLibrary => Self::InLibrary,
            ListMembershipState::Added => Self::Added,
            ListMembershipState::Requested => Self::Requested,
            ListMembershipState::Held => Self::Held,
            ListMembershipState::Rejected => Self::Rejected,
            ListMembershipState::Filtered => Self::Filtered,
            ListMembershipState::Excluded => Self::Excluded,
            ListMembershipState::Unresolved => Self::Unresolved,
            ListMembershipState::BlockedPermission => Self::BlockedPermission,
            ListMembershipState::Discover => Self::Discover,
            ListMembershipState::Pending => Self::Pending,
        }
    }
}

/// The kind of one list filter.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListFilterKind", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListFilterKindValue {
    /// The provider's rating is at least `value` on `scale`.
    RatingAtLeast,
    /// Released between `from` and `to`, either end open.
    ReleaseYear,
    /// Not in any of the genres in `values`.
    ExcludeGenres,
    /// In one of the formats in `values`.
    Format,
    /// In one of the languages in `values`.
    Language,
    /// Not on the member's streaming services.
    SkipOnMyStreamingServices,
    /// Already released.
    ReleasedOnly,
    /// Only the director's own credits.
    DirectorCreditsOnly,
    /// No sequel whose earlier entry is missing.
    NotSequelWithoutBase,
}

/// The account a provider group's lists need.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListAuthBadge", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListAuthBadgeValue {
    /// No account.
    NoAccount,
    /// No account, but the list needs a value such as a list id.
    NoAccountNeedsValue,
    /// The member's own linked account.
    MemberAccount,
    /// The instance's API key for the provider.
    ServerApiKey,
}

impl ListAuthBadgeValue {
    pub fn from_domain(value: ListAuthBadge) -> Self {
        match value {
            ListAuthBadge::NoAccount => Self::NoAccount,
            ListAuthBadge::NoAccountNeedsValue => Self::NoAccountNeedsValue,
            ListAuthBadge::MemberAccount => Self::MemberAccount,
            ListAuthBadge::ServerApiKey => Self::ServerApiKey,
        }
    }
}

/// How a list parameter is entered.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListSourceParamType", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListSourceParamTypeValue {
    /// Free text.
    Text,
    /// A link.
    Url,
    /// One of `options`.
    Enum,
    /// A season.
    Season,
}

impl ListSourceParamTypeValue {
    pub fn from_domain(value: ListSourceParamType) -> Self {
        match value {
            ListSourceParamType::Text => Self::Text,
            ListSourceParamType::Url => Self::Url,
            ListSourceParamType::Enum => Self::Enum,
            ListSourceParamType::Season => Self::Season,
        }
    }
}

/// How prominent a provider note is.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ListNoteTone", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ListNoteToneValue {
    /// Background information.
    Info,
    /// A caveat worth reading.
    Warn,
    /// A limitation that stops something working.
    Bad,
}

impl ListNoteToneValue {
    pub fn from_domain(value: ListNoteTone) -> Self {
        match value {
            ListNoteTone::Info => Self::Info,
            ListNoteTone::Warn => Self::Warn,
            ListNoteTone::Bad => Self::Bad,
        }
    }
}

/// Where a media request came from.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "MediaRequestOriginKind", rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaRequestOriginKindValue {
    /// A person asked for it.
    Manual,
    /// A public list submitted it.
    PublicList,
    /// A member's personal list submitted it.
    PersonalList,
}

/// Where a media request came from. A personal list is named by kind only.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestOriginPayload {
    /// Whether a person or a list submitted the request.
    pub kind: MediaRequestOriginKindValue,
    /// The public list that submitted it, or null for any other origin.
    pub public_subscription_id: Option<ID>,
    /// That public list's name, or null when it is no longer followed.
    pub public_list_name: Option<String>,
}

/// A provider's tile colours and abbreviation.
#[derive(SimpleObject, Clone)]
pub struct ListProviderTilePayload {
    /// Background colour.
    pub bg: String,
    /// Text colour.
    pub ink: String,
    /// Short label shown on the tile.
    pub abbr: String,
}

/// One parameter a list needs.
#[derive(SimpleObject, Clone)]
pub struct ListSourceParamPayload {
    /// Parameter key sent back when following the list.
    pub key: String,
    /// Label shown next to the field.
    pub label: String,
    /// How the value is entered.
    #[graphql(name = "type")]
    pub param_type: ListSourceParamTypeValue,
    /// The allowed values of an `ENUM` parameter.
    pub options: Vec<String>,
    /// Whether the list cannot be followed without it.
    pub required: bool,
}

/// One list a provider offers.
#[derive(SimpleObject, Clone)]
pub struct ListProviderItemPayload {
    /// Stable id of the offered list within its provider.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One-line description, or null.
    pub description: Option<String>,
    /// Kinds of title the list can contain.
    pub kinds: Vec<MediaFacetValue>,
    /// Source type sent back when following the list.
    pub source_type: String,
    /// Parameters the list needs.
    pub params: Vec<ListSourceParamPayload>,
    /// Whether the list belongs to one member and cannot be followed publicly.
    pub personal: bool,
    /// How often the list is read, in seconds.
    pub default_interval_seconds: i32,
}

/// A group of a provider's lists that need the same account.
#[derive(SimpleObject, Clone)]
pub struct ListProviderGroupPayload {
    /// Group heading.
    pub label: String,
    /// The account the group's lists need.
    pub auth_badge: ListAuthBadgeValue,
    /// The lists in the group.
    pub items: Vec<ListProviderItemPayload>,
}

/// A caveat shown on a provider's page.
#[derive(SimpleObject, Clone)]
pub struct ListProviderNotePayload {
    /// How prominent the note is.
    pub tone: ListNoteToneValue,
    /// Translation key of the note's text.
    pub text_key: String,
}

/// Maps one named capture group of a URL pattern to a list parameter.
#[derive(SimpleObject, Clone)]
pub struct ListUrlPatternCapturePayload {
    /// Named capture group.
    pub group: String,
    /// Parameter the captured text fills.
    pub param: String,
}

/// A link shape the provider's lists are recognised by.
#[derive(SimpleObject, Clone)]
pub struct ListUrlPatternPayload {
    /// Regular expression matched against the whole link.
    pub pattern: String,
    /// Source type a matching link names.
    pub source_type: String,
    /// Parameters taken from the link.
    pub captures: Vec<ListUrlPatternCapturePayload>,
}

/// A provider public lists can be followed from.
#[derive(SimpleObject, Clone)]
pub struct ListProviderPayload {
    /// Provider key, such as `tmdb` or `imdb`.
    pub provider_type: String,
    /// Display name.
    pub name: String,
    /// One-line summary, or null.
    pub summary: Option<String>,
    /// Longer description, or null.
    pub blurb: Option<String>,
    /// Tile colours, or null for the default tile.
    pub tile: Option<ListProviderTilePayload>,
    /// Kinds of title the provider's lists cover.
    pub coverage: Vec<MediaFacetValue>,
    /// The provider's lists, grouped by the account they need.
    pub groups: Vec<ListProviderGroupPayload>,
    /// Caveats to show.
    pub notes: Vec<ListProviderNotePayload>,
    /// Link shapes that name the provider's lists.
    pub url_patterns: Vec<ListUrlPatternPayload>,
    /// Server-wide settings the provider declares, such as an instance API
    /// key. `isSet` is reported only to callers who manage lists, and a value
    /// is never included here.
    pub config_fields: Vec<ListProviderSettingFieldPayload>,
}

/// One server-wide setting of a list provider.
#[derive(SimpleObject, Clone)]
pub struct ListProviderSettingFieldPayload {
    /// Setting key sent back when changing it.
    pub key: String,
    /// Label shown next to the field.
    pub label: String,
    /// Help text, or null.
    pub help_text: Option<String>,
    /// How the value is entered.
    #[graphql(name = "type")]
    pub field_type: PluginConfigFieldTypeValue,
    /// Whether the provider cannot fetch without it.
    pub required: bool,
    /// Whether the value is secret. A secret is never returned, only whether
    /// one is stored.
    pub secret: bool,
    /// Whether a value is stored.
    pub is_set: bool,
    /// The stored value of a setting that is not secret, or null.
    pub value: Option<String>,
}

/// A list provider's server-wide settings.
#[derive(SimpleObject, Clone)]
pub struct ListProviderSettingsPayload {
    /// Provider key, such as `mdblist`.
    pub provider_type: String,
    /// The provider's server-wide settings.
    pub fields: Vec<ListProviderSettingFieldPayload>,
}

/// One change to a list provider's server-wide setting.
#[derive(InputObject, Clone)]
pub struct ListProviderSettingChangeInput {
    /// Setting key.
    pub key: String,
    /// New value. Null or blank clears the stored value.
    pub value: Option<String>,
}

/// A public list that holds, or held, a title.
#[derive(SimpleObject, Clone)]
pub struct TitleListMembershipPayload {
    /// ID of the followed public list.
    pub subscription_id: ID,
    /// The list's name.
    pub name: String,
    /// What the list's last sync decided about the title.
    pub state: ListMembershipStateValue,
    /// Whether the list created this library title.
    pub added_by_list: bool,
    /// When the title left the list, or null while it is on it.
    pub left_at: Option<DateTime<Utc>>,
}

/// One parameter of a list source.
#[derive(SimpleObject, Clone)]
pub struct ListParamPayload {
    /// Parameter key.
    pub key: String,
    /// Parameter value.
    pub value: String,
}

/// One parameter of a list source, as entered.
#[derive(InputObject, Clone)]
pub struct ListParamInput {
    /// Parameter key.
    pub key: String,
    /// Parameter value.
    pub value: String,
}

/// What a followed list reads.
#[derive(SimpleObject, Clone)]
pub struct ListSourcePayload {
    /// Provider key.
    pub provider: String,
    /// Source type within the provider.
    pub source_type: String,
    /// The source's parameters.
    pub params: Vec<ListParamPayload>,
}

/// Where a list's titles of one kind go.
#[derive(SimpleObject, Clone)]
pub struct ListRoutePayload {
    /// Kind of title the route takes.
    pub kind: MediaFacetValue,
    /// Library the titles go to.
    pub library_id: ID,
    /// Quality profile for new titles, or null for the library default.
    pub quality_profile_id: Option<ID>,
    /// Root folder for new titles, or null for the library default.
    pub root_folder_id: Option<ID>,
    /// Monitoring for new titles.
    pub monitor_type: String,
    /// Minimum availability for new movies, or null for the default.
    pub min_availability: Option<String>,
    /// Whether new series use season folders, or null for the default.
    pub use_season_folders: Option<bool>,
    /// Episode numbering for new anime, or null for the default.
    pub release_numbering: Option<String>,
    /// Tags put on new titles.
    pub tags: Vec<String>,
}

/// Where a list's titles of one kind go, as entered.
#[derive(InputObject, Clone)]
pub struct ListRouteInput {
    /// Kind of title the route takes.
    pub kind: MediaFacetValue,
    /// Library the titles go to. It must be of the same kind, and the caller
    /// must be allowed to manage its titles.
    pub library_id: ID,
    /// Quality profile for new titles; null uses the library default.
    pub quality_profile_id: Option<ID>,
    /// Root folder for new titles; null uses the library default.
    pub root_folder_id: Option<ID>,
    /// Monitoring for new titles.
    #[graphql(default)]
    pub monitor_type: String,
    /// Minimum availability for new movies; null uses the default.
    pub min_availability: Option<String>,
    /// Whether new series use season folders; null uses the default.
    pub use_season_folders: Option<bool>,
    /// Episode numbering for new anime; null uses the default.
    pub release_numbering: Option<String>,
    /// Tags put on new titles.
    #[graphql(default)]
    pub tags: Vec<String>,
}

/// One list filter. Only the fields its kind uses are set.
#[derive(SimpleObject, Clone)]
pub struct ListFilterPayload {
    /// The filter's kind.
    pub kind: ListFilterKindValue,
    /// Rating scale, for `RATING_AT_LEAST`.
    pub scale: Option<String>,
    /// Minimum rating, for `RATING_AT_LEAST`.
    pub value: Option<f64>,
    /// First year, for `RELEASE_YEAR`.
    pub from: Option<i32>,
    /// Last year, for `RELEASE_YEAR`.
    pub to: Option<i32>,
    /// Genres, formats or languages, for the kinds that take a set.
    pub values: Vec<String>,
}

/// One list filter, as entered. Fields its kind does not use are ignored.
#[derive(InputObject, Clone)]
pub struct ListFilterInput {
    /// The filter's kind.
    pub kind: ListFilterKindValue,
    /// Rating scale, required for `RATING_AT_LEAST`.
    pub scale: Option<String>,
    /// Minimum rating, required for `RATING_AT_LEAST`.
    pub value: Option<f64>,
    /// First year, for `RELEASE_YEAR`.
    pub from: Option<i32>,
    /// Last year, for `RELEASE_YEAR`.
    pub to: Option<i32>,
    /// Genres, formats or languages, for the kinds that take a set.
    #[graphql(default)]
    pub values: Vec<String>,
}

/// A list's sync bookkeeping.
#[derive(SimpleObject, Clone)]
pub struct ListSyncStatusPayload {
    /// Health as of the last sync.
    pub state: ListSyncStateValue,
    /// When the list was last read, or null.
    pub last_at: Option<DateTime<Utc>>,
    /// When the list is next due, or null.
    pub next_at: Option<DateTime<Utc>>,
    /// Why the last sync failed, for callers who manage lists; otherwise null.
    pub error_message: Option<String>,
    /// When the last sync failed, or null.
    pub error_at: Option<DateTime<Utc>>,
    /// Until when the provider asked for a pause, or null.
    pub paused_until: Option<DateTime<Utc>>,
}

/// How a list's titles are spread across outcomes.
#[derive(SimpleObject, Clone)]
pub struct ListCountsPayload {
    /// Titles on the list.
    pub total: i32,
    /// Titles already in a library.
    pub in_library: i32,
    /// Titles the list added.
    pub added: i32,
    /// Titles the list requested.
    pub requested: i32,
    /// Titles waiting for review.
    pub held: i32,
    /// Titles left out by a filter or a missing route.
    pub filtered: i32,
    /// Titles left out by an exclusion.
    pub excluded: i32,
    /// Titles not matched yet.
    pub unresolved: i32,
}

/// A followed public list.
#[derive(SimpleObject, Clone)]
pub struct ListSubscriptionPayload {
    /// ID of the followed list.
    pub id: ID,
    /// Always `PUBLIC` here.
    pub scope: ListScopeValue,
    /// Display name.
    pub name: String,
    /// The link the list was followed from, or null.
    pub provider_url: Option<String>,
    /// What the list reads.
    pub source: ListSourcePayload,
    /// Kinds of title the list keeps.
    pub kinds: Vec<MediaFacetValue>,
    /// Whether the list syncs.
    pub enabled: bool,
    /// What a sync does with a new title.
    pub mode: ListModeValue,
    /// Where each kind of title goes.
    pub routes: Vec<ListRoutePayload>,
    /// Filters every title must pass.
    pub filters: Vec<ListFilterPayload>,
    /// Most titles acted on per sync, or null for no cap.
    pub max_per_sync: Option<i32>,
    /// What happens to a title the list added when it leaves the list.
    pub on_leave: ListOnLeaveValue,
    /// How often the list is read, in seconds.
    pub interval_seconds: i32,
    /// Sync bookkeeping.
    pub sync: ListSyncStatusPayload,
    /// Outcome counts as of the last sync.
    pub counts: ListCountsPayload,
    /// When the list was followed.
    pub created_at: DateTime<Utc>,
    /// When the list's settings last changed.
    pub updated_at: DateTime<Utc>,
}

/// One title on a followed list.
#[derive(SimpleObject, Clone)]
pub struct ListMembershipPayload {
    /// The provider's own key for the title.
    pub item_key: String,
    /// Position on the list, or null.
    pub rank: Option<i32>,
    /// Season the entry names, or null.
    pub season: Option<i32>,
    /// Kind of title.
    pub kind: MediaFacetValue,
    /// What the last sync decided.
    pub state: ListMembershipStateValue,
    /// Short reason for the state, such as a filter name, or null.
    pub state_reason: Option<String>,
    /// The title as the list names it, or null.
    pub display_title: Option<String>,
    /// Year as the list gives it, or null.
    pub year: Option<i32>,
    /// Library title, or null.
    pub title_id: Option<ID>,
    /// Request the list submitted, or null.
    pub request_id: Option<ID>,
    /// Whether the list created the library title.
    pub added_by_list: bool,
    /// When the title first appeared on the list.
    pub first_seen_at: DateTime<Utc>,
    /// When a sync last saw the title on the list.
    pub last_seen_at: DateTime<Utc>,
    /// When the title left the list, or null while it is on it.
    pub left_at: Option<DateTime<Utc>>,
}

/// A page of a list's titles, in list order.
#[derive(SimpleObject, Clone)]
pub struct ListMembershipPagePayload {
    /// Titles currently on the list.
    pub total_count: i32,
    /// The requested page.
    pub items: Vec<ListMembershipPayload>,
}

/// One sync of one list.
#[derive(SimpleObject, Clone)]
pub struct ListSyncRunPayload {
    /// ID of the sync.
    pub id: ID,
    /// When the sync started.
    pub started_at: DateTime<Utc>,
    /// When the sync finished, or null.
    pub finished_at: Option<DateTime<Utc>>,
    /// How it ended.
    pub outcome: ListSyncRunOutcomeValue,
    /// Outcome counts it left.
    pub counts: ListCountsPayload,
    /// Why it failed, for callers who manage lists; otherwise null.
    pub error_message: Option<String>,
}

/// One title the next sync would act on.
#[derive(SimpleObject, Clone)]
pub struct ListPreviewItemPayload {
    /// The provider's own key for the title.
    pub item_key: String,
    /// The title as the list names it, or its key when the list gives no name.
    pub display_title: String,
    /// Year as the list gives it, or null.
    pub year: Option<i32>,
    /// Kind of title, or null when the list does not say.
    pub kind: Option<MediaFacetValue>,
    /// Poster link, or null.
    pub poster_url: Option<String>,
}

/// What following a list would do right now. Nothing is written to make it.
#[derive(SimpleObject, Clone)]
pub struct ListPreviewPayload {
    /// Whether the link or source names a list Scryer can follow.
    pub recognized: bool,
    /// Provider key, or null when not recognised.
    pub provider: Option<String>,
    /// Source type, or null when not recognised.
    pub source_type: Option<String>,
    /// The source's parameters.
    pub params: Vec<ListParamPayload>,
    /// Suggested name, or null when not recognised.
    pub name: Option<String>,
    /// Kinds of title the list contains.
    pub kinds: Vec<MediaFacetValue>,
    /// Titles on the list.
    pub total: i32,
    /// Titles already in a library.
    pub in_library: i32,
    /// Titles a filter or a missing route would leave out.
    pub filtered: i32,
    /// Titles an exclusion would leave out.
    pub excluded: i32,
    /// Titles not matched yet.
    pub unresolved: i32,
    /// Titles the next sync would act on, in list order, at most one hundred.
    pub would_add: Vec<ListPreviewItemPayload>,
}

/// A title no list may add.
#[derive(SimpleObject, Clone)]
pub struct ListExclusionPayload {
    /// ID of the exclusion.
    pub id: ID,
    /// Kind of title.
    pub kind: MediaFacetValue,
    /// Identifiers the exclusion matches on.
    pub external_ids: Vec<ExternalIdPayload>,
    /// The title's name.
    pub display_title: String,
    /// The title's year, or null.
    pub year: Option<i32>,
    /// Which lists it applies to.
    pub scope: ListExclusionScopeValue,
    /// The public list a `LIST` exclusion belongs to, or null.
    pub subscription_id: Option<ID>,
    /// That list's name, or null.
    pub subscription_name: Option<String>,
    /// When the exclusion was made.
    pub created_at: DateTime<Utc>,
}

/// Lists a sync was started for.
#[derive(SimpleObject, Clone)]
pub struct ListSyncEnqueuedPayload {
    /// IDs of the lists now due.
    pub subscription_ids: Vec<ID>,
}

/// The member a list policy belongs to.
#[derive(SimpleObject, Clone)]
pub struct ListMemberPayload {
    /// ID of the user.
    pub id: ID,
    /// Login username.
    pub username: String,
}

/// One member's list policy.
#[derive(SimpleObject, Clone)]
pub struct MemberListPolicyPayload {
    /// The member.
    pub user: ListMemberPayload,
    /// How the member's personal-list requests are admitted.
    pub policy: ListPolicyValue,
    /// Requests the member's lists submitted in the last thirty days.
    #[graphql(name = "listRequestsLast30d")]
    pub list_requests_last_30d: i32,
}

/// A list source to preview.
#[derive(InputObject, Clone)]
pub struct ListSourceInput {
    /// Provider key; with `sourceType`, names the source directly.
    pub provider: Option<String>,
    /// Source type within the provider.
    pub source_type: Option<String>,
    /// The source's parameters.
    #[graphql(default)]
    pub params: Vec<ListParamInput>,
    /// A link to recognise when provider and source type are not given.
    pub url: Option<String>,
}

/// A public list to follow.
#[derive(InputObject, Clone)]
pub struct SubscribeListInput {
    /// Must be `PUBLIC`.
    pub scope: ListScopeValue,
    /// Provider key; with `sourceType`, names the source directly.
    pub provider: Option<String>,
    /// Source type within the provider.
    pub source_type: Option<String>,
    /// The source's parameters.
    #[graphql(default)]
    pub params: Vec<ListParamInput>,
    /// The link the list was found at; recognised when provider and source
    /// type are not given, and kept as the link back.
    pub url: Option<String>,
    /// Display name; null uses the list's own name.
    pub name: Option<String>,
    /// Kinds of title to keep; null keeps every kind the list contains.
    pub kinds: Option<Vec<MediaFacetValue>>,
    /// What a sync does with a new title. `SEARCH`, `ADD` or `HOLD`.
    pub mode: ListModeValue,
    /// Where each kind of title goes. A kind with no route is never added.
    #[graphql(default)]
    pub routes: Vec<ListRouteInput>,
    /// Filters every title must pass.
    #[graphql(default)]
    pub filters: Vec<ListFilterInput>,
    /// Most titles acted on per sync; null for no cap.
    pub max_per_sync: Option<i32>,
    /// What happens to a title the list added when it leaves the list.
    pub on_leave: ListOnLeaveValue,
}

/// Changes to a followed public list. Omitted fields stay as they are.
#[derive(InputObject, Clone)]
pub struct UpdateListSubscriptionInput {
    /// New display name.
    pub name: Option<String>,
    /// Kinds of title to keep.
    pub kinds: Option<Vec<MediaFacetValue>>,
    /// What a sync does with a new title.
    pub mode: Option<ListModeValue>,
    /// Replacement routes.
    pub routes: Option<Vec<ListRouteInput>>,
    /// Replacement filters.
    pub filters: Option<Vec<ListFilterInput>>,
    /// Most titles acted on per sync; null lifts the cap.
    #[graphql(default)]
    pub max_per_sync: MaybeUndefined<i32>,
    /// What happens to a title the list added when it leaves the list.
    pub on_leave: Option<ListOnLeaveValue>,
}

/// A title no list may add.
#[derive(InputObject, Clone)]
pub struct AddListExclusionInput {
    /// Kind of title.
    pub kind: MediaFacetValue,
    /// Identifiers to match on; at least one.
    pub external_ids: Vec<ExternalIdInput>,
    /// The title's name.
    pub display_title: String,
    /// The title's year.
    pub year: Option<i32>,
    /// Every list, or one public list.
    pub scope: ListExclusionScopeValue,
    /// The public list, for a `LIST` exclusion.
    pub subscription_id: Option<ID>,
}
