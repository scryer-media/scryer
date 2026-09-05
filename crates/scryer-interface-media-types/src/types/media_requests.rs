use super::{
    DomainEventTypeValue, ExternalIdInput, MediaFacetValue, MediaRequestLease,
    MediaRequestMetadataPayload, MediaRequestStatusValue, MonitorTypeValue, RequestRuleDecision,
    WantedSearchPayload,
};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

/// External identifier from a provider.
#[derive(SimpleObject, Clone)]
pub struct ExternalIdPayload {
    /// Provider or source name.
    pub source: String,
    /// Identifier assigned by that source.
    pub value: String,
}

/// One canon series movie inside an advanced monitoring selection.
#[derive(SimpleObject, Clone)]
pub struct MonitorSelectionMoviePayload {
    /// Movie name as it was presented when the selection was made.
    pub name: String,
    /// Provider identifiers for the selected movie.
    pub external_ids: Vec<ExternalIdPayload>,
}

/// Seasons and canon series movies chosen under the `ADVANCED` monitor type.
/// Anything absent from the selection stays unmonitored.
#[derive(SimpleObject, Clone)]
pub struct MonitorSelectionPayload {
    /// Season numbers to monitor; 0 is specials.
    pub season_numbers: Vec<i32>,
    /// Canon series movies to monitor.
    pub series_movies: Vec<MonitorSelectionMoviePayload>,
}

/// One canon series movie inside an advanced monitoring selection.
#[derive(InputObject, Clone)]
pub struct MonitorSelectionMovieInput {
    /// Movie name to show back to the approver.
    pub name: String,
    /// Provider identifiers for the selected movie.
    pub external_ids: Vec<ExternalIdInput>,
}

/// Seasons and canon series movies chosen under the `ADVANCED` monitor type.
#[derive(InputObject, Clone)]
pub struct MonitorSelectionInput {
    /// Season numbers to monitor; 0 is specials.
    pub season_numbers: Vec<i32>,
    /// Canon series movies to monitor.
    pub series_movies: Option<Vec<MonitorSelectionMovieInput>>,
}

/// User who submitted a media request.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestRequesterPayload {
    /// ID of the requesting user.
    pub user_id: ID,
    /// Requesting user's username.
    pub username: String,
    /// Avatar URL, or null when unavailable.
    pub avatar_url: Option<String>,
    /// UTC time when this user submitted the request.
    pub requested_at: DateTime<Utc>,
}

/// One provider-specific rating captured when a media request was submitted.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestExternalRatingPayload {
    /// Rating provider name.
    pub source: String,
    /// Provider rating value before normalization, or null when absent.
    pub value: Option<f64>,
    /// Provider score on its native scale, or null when absent.
    pub score: Option<f64>,
    /// Score normalized to the shared comparison scale.
    pub normalized: f64,
    /// Number of votes reported by the provider, or null when absent.
    pub votes: Option<i32>,
    /// Provider page for this rating.
    pub url: String,
}

/// One provider-specific rating submitted with a media request.
#[derive(InputObject, Clone)]
pub struct MediaRequestExternalRatingInput {
    /// Rating provider name.
    pub source: String,
    /// Provider rating value before normalization, or null when absent.
    pub value: Option<f64>,
    /// Provider score on its native scale, or null when absent.
    pub score: Option<f64>,
    /// Score normalized to the shared comparison scale.
    pub normalized: f64,
    /// Number of votes reported by the provider, or null when absent.
    pub votes: Option<i32>,
    /// Provider page for this rating.
    pub url: String,
}

/// Media request with current status, title identity, and resolution metadata.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestPayload {
    /// ID of the media request.
    pub id: ID,
    /// ID of the library targeted by the request.
    pub library_id: ID,
    /// Media facet targeted by the request.
    pub facet: MediaFacetValue,
    /// Current request lifecycle status.
    pub status: MediaRequestStatusValue,
    /// Stable identity fingerprint used to deduplicate requests.
    pub identity_fingerprint: String,
    /// Display title at request time.
    pub title: String,
    /// Sort title, or null when unavailable.
    pub sort_title: Option<String>,
    /// Provider slug, or null when unavailable.
    pub slug: Option<String>,
    /// Poster URL, or null when unavailable.
    pub poster_url: Option<String>,
    /// Background art URL captured at submit time, or null when unavailable.
    pub background_url: Option<String>,
    /// Release year, or null when unknown.
    pub year: Option<i32>,
    /// Overview text, or null when unavailable.
    pub overview: Option<String>,
    /// Runtime in minutes, or null when unknown.
    pub runtime_minutes: Option<i32>,
    /// Original language code, or null when unknown.
    pub language: Option<String>,
    /// Provider content status, or null when unavailable.
    pub content_status: Option<String>,
    /// Combined metadata rating, or null when unavailable.
    pub rating: Option<f64>,
    /// Sources contributing to the combined metadata rating.
    pub rating_sources: Vec<String>,
    /// Provider-specific metadata ratings.
    pub external_ratings: Vec<MediaRequestExternalRatingPayload>,
    /// ID of the quality profile requested, or null when none was selected.
    pub requested_quality_profile_id: Option<ID>,
    /// Name of the requested quality profile, or null when none was selected.
    pub requested_quality_profile_name: Option<String>,
    /// Requested monitoring mode, or null when not specified.
    pub requested_monitor_type: Option<MonitorTypeValue>,
    /// Seasons and series movies requested under `ADVANCED` monitoring, or null.
    pub requested_monitor_selection: Option<MonitorSelectionPayload>,
    /// ID of the user who resolved the request, or null while unresolved.
    pub resolved_by_user_id: Option<ID>,
    /// UTC time when the request was resolved, or null while unresolved.
    pub resolved_at: Option<DateTime<Utc>>,
    /// ID of the title created from the request, or null when not created.
    pub created_title_id: Option<ID>,
    /// ID of the approved quality profile, or null before approval.
    pub approved_quality_profile_id: Option<ID>,
    /// Name of the approved quality profile, or null before approval.
    pub approved_quality_profile_name: Option<String>,
    /// Provider identifiers associated with the request.
    pub external_ids: Vec<ExternalIdPayload>,
    /// Users who submitted or joined the request.
    pub requesters: Vec<MediaRequestRequesterPayload>,
    /// ID of the user who created the request.
    pub created_by_user_id: ID,
    /// UTC time when the request was created.
    pub created_at: DateTime<Utc>,
    /// UTC time when the request was last changed.
    pub updated_at: DateTime<Utc>,
    /// Days the requester asked the media to be kept for; null means forever.
    pub requested_lease_days: Option<i32>,
    /// Days the approver granted; null means forever, and it stays null until
    /// the request is approved.
    pub approved_lease_days: Option<i32>,
    /// The lease actually holding the created title, or null until an approval
    /// creates the claim.
    pub lease: Option<MediaRequestLease>,
    /// The decision request rules recorded for this request, or null when it
    /// was never evaluated. A requester reading their own request gets it with
    /// `votes` emptied.
    pub decision: Option<RequestRuleDecision>,
    /// Tags the policy emitted for this request. Stamped on the title only when
    /// the request is approved.
    pub policy_tags: Vec<String>,
    /// The metadata the request was decided against, as captured at submit
    /// time.
    pub metadata: MediaRequestMetadataPayload,
}

/// Event payload identifying a changed media request.
#[derive(SimpleObject, Clone)]
pub struct MediaRequestChangedPayload {
    /// ID of the event.
    pub event_id: ID,
    /// Domain event type that caused the notification.
    pub event_type: DomainEventTypeValue,
    /// ID of the changed media request.
    pub request_id: ID,
    /// ID of the library containing the request.
    pub library_id: ID,
}

/// Provider catalog family used when describing configurable providers.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ProviderCatalogFamilyValue {
    /// Subtitle provider.
    Subtitle,
    /// Notification provider.
    Notification,
    /// Indexer provider.
    Indexer,
    /// Download-client provider.
    DownloadClient,
    /// Archive-extractor provider.
    ArchiveExtractor,
}

/// Result containing the ID of the submitted or deduplicated request.
#[derive(SimpleObject, Clone)]
pub struct SubmitMediaRequestPayload {
    /// ID of the submitted or deduplicated media request.
    pub request_id: ID,
}

#[derive(InputObject, Clone)]
/// Metadata and preferences submitted with a media request.
pub struct SubmitMediaRequestInput {
    /// Library identity in which the requested title belongs.
    pub library_id: ID,
    /// Requested media facet.
    pub facet: MediaFacetValue,
    /// Requested title name.
    pub title: String,
    /// External provider identifiers for the request.
    pub external_ids: Vec<ExternalIdInput>,
    /// Release year when known.
    pub year: Option<i32>,
    /// Plot summary when known.
    pub overview: Option<String>,
    /// Sort key for title ordering.
    pub sort_title: Option<String>,
    /// URL-safe title slug.
    pub slug: Option<String>,
    /// Runtime in minutes.
    pub runtime_minutes: Option<i32>,
    /// Metadata language code.
    pub language: Option<String>,
    /// Provider content-status label.
    pub content_status: Option<String>,
    /// Combined metadata rating, or null when unavailable.
    pub rating: Option<f64>,
    /// Sources contributing to the combined metadata rating.
    pub rating_sources: Option<Vec<String>>,
    /// Provider-specific metadata ratings.
    pub external_ratings: Option<Vec<MediaRequestExternalRatingInput>>,
    /// Quality profile identity requested for approval.
    pub requested_quality_profile_id: Option<ID>,
    /// Monitoring policy requested for approval.
    pub requested_monitor_type: Option<MonitorTypeValue>,
    /// Seasons and series movies to monitor; required with `ADVANCED`.
    pub requested_monitor_selection: Option<MonitorSelectionInput>,
    /// How long the requester wants the media kept, in days. Omitted means
    /// forever, which is what Scryer granted before leases existed.
    pub requested_lease_days: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Approval choices for a media request.
pub struct ApproveMediaRequestInput {
    /// Media request identity to approve.
    pub request_id: ID,
    /// Quality profile identity to apply to the approved title.
    pub quality_profile_id: ID,
    /// Optional monitoring policy to apply to the approved title.
    pub monitor_type: Option<MonitorTypeValue>,
    /// Optional approver override for the requested advanced selection; when
    /// omitted the request's stored selection is applied.
    pub monitor_selection: Option<MonitorSelectionInput>,
    /// Approver override for the lease, in days. Omitting both this and
    /// `leaseForever` grants exactly what the requester asked for.
    pub lease_days: Option<i32>,
    /// Set to true to grant the title forever regardless of what was asked.
    /// Rejected together with `leaseDays`.
    pub lease_forever: Option<bool>,
    /// Approver override for the policy tags stamped on the created title. A
    /// supplied list **replaces** the policy's tags outright; omitting it keeps
    /// them.
    pub tags: Option<Vec<String>>,
}

#[derive(InputObject, Clone)]
/// Replacement preferences for the caller's media request.
pub struct UpdateMediaRequestInput {
    /// Media request identity to update.
    pub request_id: ID,
    /// Quality profile identity requested for the title.
    pub requested_quality_profile_id: ID,
    /// Optional monitoring policy requested for the title.
    pub requested_monitor_type: Option<MonitorTypeValue>,
    /// Seasons and series movies to monitor; required with `ADVANCED`.
    pub requested_monitor_selection: Option<MonitorSelectionInput>,
    /// Replacement lease in days; omitted means forever. Always applied,
    /// because the edit form always carries the current value.
    pub requested_lease_days: Option<i32>,
}

#[derive(SimpleObject, Clone)]
/// Identifier returned after a media-request action.
pub struct MediaRequestActionPayload {
    /// The media request the action applied to.
    pub request_id: ID,
}

#[derive(SimpleObject, Clone)]
/// Result of approving a media request.
pub struct ApproveMediaRequestPayload {
    /// Title identity created or updated by approval.
    pub title_id: ID,
    /// Search counts when approval queued acquisition work.
    pub wanted_search: Option<WantedSearchPayload>,
    /// Non-fatal search error when approval succeeded but search could not be queued.
    pub search_error: Option<String>,
    /// Non-fatal claim error when the title was created and the request
    /// resolved, but the retention claim could not be written. The approval is
    /// deliberately **not** rolled back: the requester has their title, and an
    /// operator can re-pin it by hand.
    pub claim_error: Option<String>,
}
