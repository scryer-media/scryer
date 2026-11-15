//! Maintenance policy family: match existing library media against user rules.
//!
//! The matcher answers one question — does this subject match this rule right
//! now — and returns match / no-match / unknown. It never selects, names, or
//! performs an action: the action comes from the validated rule revision that
//! owns the matcher, so a malformed or timed-out evaluation can never authorize
//! one.
//!
//! Media-server play history is in the schema now that a provider-neutral
//! signal store stands behind it, and it is fact-shaped like everything else:
//! `watched_by_user_ids`, `last_watched_at`, the two requester rollups, and the
//! anonymous `watched_by_anyone` pair are plain values that go missing when the
//! host could not observe them. Watchlists come from members' own list
//! subscriptions. Favourites come from Jellyfin and Emby, read on their own so
//! a favourite nobody played is seen; Plex has none, and its members are never
//! read as "not a favourite".
//!
//! Two things make a rule safe to write. The rule reads facts as plain values,
//! so it says what it means (`input.facts.monitored`, not a status dance), and
//! the *host* — not the author — decides that a rule reading a fact Scryer
//! could not observe for this subject must be held. The set of facts a rule
//! reads is extracted from its source at build time, which is why fact names
//! have to be literal and why `input` cannot be imported.

use crate::RulesError;
use crate::maintenance_actions::{self, MaintenanceActionHead, MaintenanceActions, decode_actions};
use crate::policy::decode::decode_reasons;
use crate::policy::engine::{EvalOutcome, EvalRecord, PolicyEngine, PolicyEvaluator, RuleHandle};
use crate::policy::observation::serialize_fact_namespaces;
use crate::policy::wrapper::{WrapperField, object_wrapper_source};
use crate::policy::{PolicyFamily, PolicyRecord};
use crate::runtime::RuntimeLimits;
use crate::validation;
use chrono::{DateTime, Utc};
use regorus::Value;
use serde::Serialize;
use serde::ser::{SerializeStruct, Serializer};
use std::collections::BTreeSet;

#[cfg(test)]
use crate::policy::decode::{MAX_REASON_CODE_LEN, MAX_REASON_CODES};
#[cfg(test)]
use crate::runtime;

/// Three-valued availability envelope wrapping every maintenance fact. Owned by
/// the shared policy core; re-exported here because maintenance facts are what
/// it was written for and what most callers reach for it through.
pub use crate::policy::observation::Observation;

// ── Contract constants ──────────────────────────────────────────────────────

/// Version of the maintenance input document. Rules are authored against a
/// specific version; bumping it is a breaking change to every stored matcher.
///
/// v2 split the fact snapshot in two: `input.facts.<name>` is the bare value
/// and is simply missing when the fact is absent or unknown, while
/// `input.observations.<name>` keeps the full three-valued envelope for rules
/// that need to tell those two apart.
pub const MAINTENANCE_INPUT_SCHEMA_VERSION: u32 = 2;

/// Package prefix for user-authored maintenance matchers.
pub(crate) const USER_PACKAGE_PREFIX: &str = "scryer.maintenance.user";

/// Package prefix for the generated evaluation wrapper. The wrapper lives in a
/// separate package because it reads the whole user package document; putting
/// it inside that package would make the module self-referential.
const WRAPPER_PACKAGE_PREFIX: &str = "scryer.maintenance.wrapper";

/// The head an author writes to hold a subject themselves. Maintenance says
/// `unknown if { … }`; the name is a per-family parameter of the policy core,
/// and the request family says `manual if { … }` instead.
const HOLD_RULE_NAME: &str = "unknown";

// ── Input document ──────────────────────────────────────────────────────────

/// Input document set once per subject for maintenance rule evaluation.
///
/// `evaluation_time` is supplied by the host and captured once per run so every
/// subject in that run compares against the same instant. Policies must never
/// reach for a clock of their own — `time.now_ns()` exists, but it would make
/// the same subject decide differently on a retry.
///
/// The fact snapshot is held once, as envelopes, and serializes into two
/// namespaces: `observations` is the envelope map verbatim, and `facts` is
/// derived from it by unwrapping the known values and dropping everything else.
/// Deriving rather than storing both is the point — the simple surface and the
/// advanced one cannot drift apart.
#[derive(Debug, Clone)]
pub struct MaintenanceInput {
    pub schema_version: u32,
    pub evaluation_time: DateTime<Utc>,
    pub subject: MaintenanceSubjectDoc,
    pub library: MaintenanceLibraryDoc,
    pub facts: MaintenanceFactsDoc,
}

impl Serialize for MaintenanceInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let namespaces =
            serialize_fact_namespaces(&self.facts).map_err(serde::ser::Error::custom)?;
        let mut state = serializer.serialize_struct("MaintenanceInput", 6)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("evaluation_time", &self.evaluation_time)?;
        state.serialize_field("subject", &self.subject)?;
        state.serialize_field("library", &self.library)?;
        state.serialize_field("facts", &namespaces.facts)?;
        state.serialize_field("observations", &namespaces.observations)?;
        state.end()
    }
}

/// Granularity the rule set is scoped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceSubjectKind {
    Title,
    Season,
    Episode,
}

/// Identity of the subject under evaluation. `season_number` and `episode_id`
/// are populated only for the matching subject kind.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceSubjectDoc {
    pub subject_id: String,
    pub collection_id: Option<String>,
    pub kind: MaintenanceSubjectKind,
    pub title_id: String,
    pub season_number: Option<i32>,
    pub episode_id: Option<String>,
    pub facet: String,
    pub name: String,
    pub year: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceLibraryDoc {
    pub id: String,
    pub name: String,
}

/// The fact snapshot, and the single source of truth for both input
/// namespaces. Every field is an observation envelope: a fact Scryer failed to
/// resolve stays unknown rather than defaulting to a value the rule would read
/// as decisive.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceFactsDoc {
    pub monitored: Observation<bool>,
    pub tags: Observation<Vec<String>>,
    pub quality_profile_id: Observation<String>,
    /// RFC3339 timestamps.
    pub added_at: Observation<String>,
    /// Provenance: who put the title in the library, and whether a media
    /// request is what created it. These are facts, not a classification —
    /// there is deliberately no derived `origin` field, because "managed",
    /// "requested", and "scan-discovered" are conclusions a rule composes for
    /// itself out of these five observations.
    pub added_by_user_id: Observation<String>,
    pub added_by_username: Observation<String>,
    pub requested: Observation<bool>,
    pub requested_by_user_ids: Observation<Vec<String>>,
    pub requested_by_usernames: Observation<Vec<String>>,
    pub first_imported_at: Observation<String>,
    pub last_upgraded_at: Observation<String>,
    pub has_file: Observation<bool>,
    pub file_count: Observation<i64>,
    pub total_file_size_bytes: Observation<i64>,
    pub files: Observation<Vec<MaintenanceFileDoc>>,
    pub episode_count: Observation<i64>,
    pub episode_file_count: Observation<i64>,
    pub monitored_episode_count: Observation<i64>,
    /// One-based retention positions within the parent show. Rank 1 is newest
    /// by air date; unavailable/ineligible coverage is unknown rather than
    /// absent so `not input.facts.<rank>` cannot authorize deletion.
    pub episode_position_by_air_date: Observation<i64>,
    pub season_position_by_air_date: Observation<i64>,
    pub active_downloads: Observation<bool>,
    /// Media-server watch signals (RFC 137 section 7.3). Every one of these is
    /// unknown unless a signal-sync provider is connected *and* every enabled
    /// connection of it swept cleanly and recently: a partial watch picture
    /// would make "nobody watched this" a lie, and these are the facts a rule
    /// deletes media on.
    pub watched_by_user_ids: Observation<Vec<String>>,
    pub last_watched_at: Observation<String>,
    pub watched_by_any_requester: Observation<bool>,
    pub watched_by_all_requesters: Observation<bool>,
    /// Whether any account on any enabled media server has played the
    /// subject, linked to a Scryer user or not, and when the most recent such
    /// play was. Anonymous: they say a play happened, never whose. Behind the
    /// same sweep gate as the facts above, except that they do not wait for a
    /// linked user, and also unknown until every enabled connection's full
    /// audience has been read recently. For a season or a show they cover
    /// every aired episode in scope: one play of one episode is enough for
    /// `watched_by_anyone`.
    ///
    /// On Jellyfin and Emby the audience is every user's own played state,
    /// marks included. On Plex it is the owner's own played state, marks
    /// included, and the server's history for every other account, which
    /// holds organic plays only: an item another account marked watched, or
    /// scrobbled, without playing it is not a play here.
    pub watched_by_anyone: Observation<bool>,
    pub last_watched_by_anyone_at: Observation<String>,
    /// Lifecycle claims (spec 0003 FR-041..FR-044). A claim is what a request
    /// approval leaves behind on the title, and these three describe its
    /// lease. Every one of them is unknown when the claim store could
    /// not be read for the pass: a rule that deletes on "the lease expired"
    /// must be held rather than told there is no lease.
    ///
    /// `request_lease_state` is one of `none`, `dormant`, `active`, `expired`,
    /// aggregated over the title's retention claims; expiry is derived against
    /// the evaluation clock, so a lease whose window elapsed reads as expired
    /// before the sweep that flips the row has run.
    pub request_lease_state: Observation<String>,
    pub request_lease_expires_at: Observation<String>,
    pub active_retention_claims: Observation<i64>,
    /// List membership. `lists_added_by_list` says a list, not a person, put
    /// the title in the library. `lists_on_enabled_list` says whether any
    /// enabled list of either scope still lists it; `lists_names` names only
    /// the enabled public lists that do, because a member's personal list
    /// name is private to them. `lists_left_all` is true once every list that
    /// ever listed the title has dropped it, and `lists_left_at` is when the
    /// last one did. `lists_last_list_name` is the public list the title most
    /// recently left. Every one of them is unknown when the list store could
    /// not be read: a rule that removes titles no list wants any more must be
    /// held rather than told nothing lists them.
    pub lists_added_by_list: Observation<bool>,
    pub lists_on_enabled_list: Observation<bool>,
    pub lists_names: Observation<Vec<String>>,
    pub lists_left_all: Observation<bool>,
    pub lists_left_at: Observation<String>,
    pub lists_last_list_name: Observation<String>,
    /// `lists_enabled_ids` are the ids, never the names, of the enabled lists
    /// of either scope that hold the title, and the counts split every list
    /// that holds it by scope. The sightings, days on a list and best rank are
    /// taken over the lists that hold the title now and are absent when none
    /// does. `lists_left_handled` says every departure has had its on-leave
    /// action applied, and is absent when no list ever dropped the title.
    pub lists_enabled_ids: Observation<Vec<String>>,
    pub lists_public_count: Observation<i64>,
    pub lists_personal_count: Observation<i64>,
    /// RFC3339 timestamps.
    pub lists_first_seen_at: Observation<String>,
    pub lists_last_seen_at: Observation<String>,
    pub lists_days_on_list: Observation<i64>,
    pub lists_best_rank: Observation<i64>,
    pub lists_on_leave: Observation<Vec<String>>,
    pub lists_left_handled: Observation<bool>,
    pub lists_membership_states: Observation<Vec<String>>,
    /// Where the request that created the title came from: `manual`,
    /// `public_list` or `personal_list`, and the public list's name. Both are
    /// absent for a title no request created, and the name is absent for
    /// every origin but a public list.
    pub request_origin: Observation<String>,
    pub request_list_name: Observation<String>,
    /// The series movies linked to a show subject: one entry per
    /// `series_movie_links` row, each with its own tags.
    ///
    /// Known and empty for a show with no linked movies — Scryer looked and
    /// there are none — and absent for a movie subject, which cannot have them
    /// at all. A rule reading it on a movie therefore sees a missing key rather
    /// than being held.
    pub series_movies: Observation<Vec<MaintenanceSeriesMovieDoc>>,
    /// Storage capacity is exposed only after the host applies a selected
    /// configured root. Without that root these remain absent; a storage-aware
    /// rule must be validated with an explicit root selection before it runs.
    ///
    /// Scheduled evaluation reads the capacity sampler's last figure for the
    /// root and never calls the disk: a root not yet sampled is unknown with
    /// `storage_not_sampled`, one sampled more than two sampling intervals ago
    /// with `storage_sample_stale`, and one whose last sample failed with
    /// `storage_capacity_unavailable`. Preview and the execution-time guard
    /// take a figure now, which is unknown only with
    /// `storage_capacity_unavailable`.
    pub storage_root_id: Observation<String>,
    pub storage_available_bytes: Observation<i64>,
    pub storage_total_bytes: Observation<i64>,
    pub storage_available_percent: Observation<f64>,
    /// Metadata Scryer already stores on the title row. Each is absent when
    /// the metadata provider gave no value, and the movie release dates are
    /// absent for a show. `first_aired` and the release dates are dates as the
    /// provider gave them (`YYYY-MM-DD`), not timestamps.
    pub runtime_minutes: Observation<i64>,
    pub original_language: Observation<String>,
    pub series_status: Observation<String>,
    pub first_aired: Observation<String>,
    pub network: Observation<String>,
    pub studio: Observation<String>,
    pub country: Observation<String>,
    pub popularity: Observation<f64>,
    pub theatrical_release_date: Observation<String>,
    pub digital_release_date: Observation<String>,
    pub physical_release_date: Observation<String>,
    /// Whole days since the subject was added, against the evaluation clock.
    pub days_since_added: Observation<i64>,
    /// The tallest video stream among the subject's files, in pixels; absent
    /// when no file reports one. Unknown (`not_yet_collected`) while any of
    /// the files has no completed scan, including one whose scan failed or
    /// needs review.
    pub max_video_height: Observation<i64>,
    /// The shape of a show's whole episode list, whatever the subject: its
    /// seasons and its regular episodes, specials left out. An episode has
    /// aired once the calendar day of its air date (UTC) is on or before the
    /// evaluation day, the same test the scoped watch facts use; one with no
    /// air date counts as not aired here, and one with a later day has not
    /// aired. Missing means aired, monitored and without a file. Absent for
    /// movies; unknown when the episode store could not be read, and unknown
    /// (`episode_list_empty`) for a show with no regular episode stored, which
    /// is what a show looks like before its episode list has been read.
    pub season_count: Observation<i64>,
    pub aired_episode_count: Observation<i64>,
    pub unaired_episode_count: Observation<i64>,
    pub missing_episode_count: Observation<i64>,
    /// Aired episodes with a file, as a percentage of aired episodes; absent
    /// when nothing has aired.
    pub aired_episode_file_percent: Observation<f64>,
    pub latest_season_number: Observation<i64>,
    pub latest_aired_episode_date: Observation<String>,
    /// The earliest air date after the evaluation day; absent when none is
    /// scheduled.
    pub next_episode_air_date: Observation<String>,
    /// Genre names from the stored metadata tags. Unknown when the tag store
    /// could not be read.
    pub genres: Observation<Vec<String>>,
    /// The stored rating summary and every external rating behind it. Unknown
    /// when the rating store could not be read; `rating` is absent when no
    /// source rated the title.
    pub rating: Observation<f64>,
    pub ratings: Observation<Vec<MaintenanceRatingDoc>>,
    /// Plays of the subject by every account on every enabled media server,
    /// linked to a Scryer user or not, summed. Anonymous, and behind the same
    /// gate as `watched_by_anyone`.
    pub play_count_total: Observation<i64>,
    /// Watch-later lists. A member's watchlist is one of their own personal
    /// list subscriptions to a source that holds what they want to watch
    /// (the Plex, Trakt, TMDb and MDBList watchlists, Simkl and MyAnimeList
    /// plan-to-watch, and the AniList planning list), enabled and signed in.
    /// Every one of these is unknown while any such list has failed its last
    /// sync, never synced, or has not synced recently: a watchlist Scryer has
    /// not read could hold the title. A member with no watchlist contributes
    /// nothing.
    pub watchlisted_by_user_ids: Observation<Vec<String>>,
    pub watchlisted_by_usernames: Observation<Vec<String>>,
    pub watchlisted_by_anyone: Observation<bool>,
    pub watchlisted_by_any_requester: Observation<bool>,
    /// When Scryer first saw the title on any watchlist (RFC3339), and the
    /// providers whose watchlists hold it now.
    pub watchlist_added_at: Observation<String>,
    pub watchlist_origins: Observation<Vec<String>>,
    /// Members who marked the subject a favourite on an enabled Jellyfin or
    /// Emby server. A series title counts a favourite of the series or of any
    /// of its seasons or episodes. A provider's seasons need not be numbered
    /// like Scryer's, so a favourite of the series or of any season counts for
    /// every season and episode subject of the series; an episode favourite
    /// counts for that episode, the season Scryer files it under, and the
    /// series. Scryer cannot read Plex favourites: an instance whose only
    /// enabled servers are Plex has these unknown (`favorites_unsupported`),
    /// as they are while any linked member can only be observed through Plex.
    /// `favorited_by_anyone` is never false: favourites are read only for
    /// linked members, so with none found it is unknown
    /// (`audience_favorites_not_read`).
    pub favorited_by_user_ids: Observation<Vec<String>>,
    pub favorited_by_usernames: Observation<Vec<String>>,
    pub favorited_by_anyone: Observation<bool>,
    pub favorited_by_any_requester: Observation<bool>,
}

/// One external rating as a rule sees it: the source's own value, its value on
/// Scryer's common scale, and its vote count when the source gives one.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct MaintenanceRatingDoc {
    pub source: String,
    pub value: Option<f64>,
    pub normalized: f64,
    pub votes: Option<i64>,
}

/// Facts that name, or are computed from, identifiable people.
///
/// Authoring a matcher that reads one of these is a way of asking the instance
/// who added, requested, or watched something, so the host requires the author
/// to already hold the instance's permission-management authority. The list
/// lives here, beside the fact document, because it is a property of the facts
/// themselves rather than of any one caller.
///
/// `requested`, `last_watched_at`, `watched_by_anyone`,
/// `last_watched_by_anyone_at`, `play_count_total`, `watchlisted_by_anyone`,
/// `watchlist_added_at`, `watchlist_origins` and `favorited_by_anyone` are
/// deliberately absent: they are anonymous aggregates that say something
/// happened, never who did it.
pub const PERSON_TARGETED_MAINTENANCE_FACTS: [&str; 13] = [
    "added_by_user_id",
    "added_by_username",
    "requested_by_user_ids",
    "requested_by_usernames",
    "watched_by_user_ids",
    "watched_by_any_requester",
    "watched_by_all_requesters",
    "watchlisted_by_user_ids",
    "watchlisted_by_usernames",
    "watchlisted_by_any_requester",
    "favorited_by_user_ids",
    "favorited_by_usernames",
    "favorited_by_any_requester",
];

/// Facts read from members' Jellyfin and Emby favourites. The host reads the
/// stored favourites only for a rule that reads one.
pub const FAVORITE_MAINTENANCE_FACTS: [&str; 4] = [
    "favorited_by_user_ids",
    "favorited_by_usernames",
    "favorited_by_anyone",
    "favorited_by_any_requester",
];

/// Facts read from members' watch-later lists. The host loads the list
/// subscriptions behind them only for a rule that reads one.
pub const WATCHLIST_MAINTENANCE_FACTS: [&str; 6] = [
    "watchlisted_by_user_ids",
    "watchlisted_by_usernames",
    "watchlisted_by_anyone",
    "watchlisted_by_any_requester",
    "watchlist_added_at",
    "watchlist_origins",
];

/// Facts read from the stored metadata tags and ratings. The host reads those
/// stores only for a rule that reads one.
pub const METADATA_MAINTENANCE_FACTS: [&str; 3] = ["genres", "rating", "ratings"];

/// Facts read from one grouped count over a show's stored episodes. The host
/// reads it only for a rule that reads one.
pub const EPISODE_SHAPE_MAINTENANCE_FACTS: [&str; 8] = [
    "season_count",
    "aired_episode_count",
    "unaired_episode_count",
    "missing_episode_count",
    "aired_episode_file_percent",
    "latest_season_number",
    "latest_aired_episode_date",
    "next_episode_air_date",
];

/// Facts computed from media-server watch history.
///
/// A rule that reads one decided on plays stored by the last sweep, so the
/// executor reads the server again before acting on it: a play since that
/// sweep is evidence the stored facts do not have yet. The favourites are in
/// this group too: they come from the same sweep.
pub const WATCH_MAINTENANCE_FACTS: [&str; 11] = [
    "watched_by_user_ids",
    "last_watched_at",
    "watched_by_any_requester",
    "watched_by_all_requesters",
    "watched_by_anyone",
    "last_watched_by_anyone_at",
    "play_count_total",
    "favorited_by_user_ids",
    "favorited_by_usernames",
    "favorited_by_anyone",
    "favorited_by_any_requester",
];

/// One series movie as a rule sees it.
///
/// A series movie is a link between a show and a shared movie record, not a
/// title, so it has no library, no facet, and no id a title-scoped action could
/// be pointed at. What it does have is its own tag bag, which is why it is here:
/// `add_tags` and `remove_tags` act on the subject title only, so a rule that
/// wants to reason about "this show has an untagged movie" needs to read the
/// movies rather than act on them.
#[derive(Debug, Clone, Serialize)]
pub struct MaintenanceSeriesMovieDoc {
    pub link_id: String,
    pub name: String,
    pub year: Option<i32>,
    pub monitored: bool,
    pub has_file: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct MaintenanceFileDoc {
    /// Explicit media metadata detected 3D. False also covers unexamined files.
    #[serde(default)]
    pub is_3d: bool,
    pub size_bytes: Option<i64>,
    pub quality: Option<String>,
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub added_at: Option<String>,
}

// ── Output contract ─────────────────────────────────────────────────────────

/// Closed output of one maintenance matcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceOutcome {
    Match,
    NoMatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceDecision {
    pub outcome: MaintenanceOutcome,
    pub reason_codes: Vec<String>,
    /// The actions the rule declared for this subject. Always empty when the
    /// outcome is [`MaintenanceOutcome::Unknown`]: a rule that was held, by the
    /// host or by its own `unknown`, asks for nothing.
    pub actions: MaintenanceActions,
}

/// One rule's decision, attributed to the exact policy revision that produced
/// it via `policy_content_hash`.
pub type MaintenanceEvalRecord = EvalRecord<MaintenanceDecision>;

/// A per-rule failure. A rule that errors produces no record at all, so it can
/// never advance a candidate.
#[derive(Debug, Clone)]
pub struct MaintenanceEvalError {
    pub rule_set_id: String,
    pub rule_set_name: String,
    /// What went wrong, as one of the codes in [`crate::diagnostic_codes`]:
    /// `RULE_RUNTIME_ERROR`, `RULE_OUTPUT_INVALID`, or the code of the
    /// condition that refused the rule's source at load.
    pub code: &'static str,
    /// Display text; code never reads it.
    pub message: String,
}

pub type MaintenanceEvalResult = EvalOutcome<MaintenanceDecision, MaintenanceEvalError>;

// ── Package rewriting and wrapper generation ────────────────────────────────

/// Rewrite (or insert) the package declaration so stored maintenance source
/// always carries the maintenance prefix plus the system-assigned rule ID.
pub fn rewrite_package_declaration(rego_source: &str, rule_id: &str) -> String {
    crate::runtime::rewrite_package_declaration_with_prefix(
        rego_source,
        USER_PACKAGE_PREFIX,
        rule_id,
    )
}

/// Source for the generated evaluation entry point.
///
/// `object.get` with a default is what makes `unknown`, `reasons` and every
/// action head optional: a rule whose body fails leaves the name off the
/// package document entirely, and the default fills in. A head that is defined
/// with the wrong type survives to the host, which rejects it rather than
/// coercing it. `match` is not projected: it is an ordinary author helper.
pub(crate) fn decision_wrapper_source(rule_id: &str) -> String {
    object_wrapper_source(
        WRAPPER_PACKAGE_PREFIX,
        USER_PACKAGE_PREFIX,
        rule_id,
        "decision",
        &[
            WrapperField::new("unknown", HOLD_RULE_NAME, "false"),
            WrapperField::new("reasons", "reasons", "[]"),
        ]
        .into_iter()
        .chain(MaintenanceActionHead::ALL.into_iter().map(|head| {
            WrapperField::new(head.rule_name(), head.rule_name(), head.wrapper_default())
        }))
        .collect::<Vec<_>>(),
    )
}

pub(crate) fn decision_wrapper_rule_path(rule_id: &str) -> String {
    format!("data.{WRAPPER_PACKAGE_PREFIX}.{rule_id}.decision")
}

pub(crate) fn decision_wrapper_policy_path(rule_id: &str) -> String {
    format!("internal/{rule_id}_maintenance_wrapper.rego")
}

pub(crate) fn user_policy_path(rule_id: &str) -> String {
    format!("maintenance/{rule_id}.rego")
}

// ── Engine ──────────────────────────────────────────────────────────────────

/// A maintenance matcher loaded from the database.
#[derive(Debug, Clone)]
pub struct MaintenancePolicy {
    pub id: String,
    pub name: String,
    pub rego_source: String,
}

/// The maintenance policy family: what the shared core needs to know to run
/// maintenance matchers, and nothing else.
pub struct MaintenanceFamily;

impl PolicyRecord for MaintenancePolicy {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn rego_source(&self) -> &str {
        &self.rego_source
    }
}

impl PolicyFamily for MaintenanceFamily {
    const NAME: &'static str = "maintenance";
    const USER_PACKAGE_PREFIX: &'static str = USER_PACKAGE_PREFIX;
    const WRAPPER_PACKAGE_PREFIX: &'static str = WRAPPER_PACKAGE_PREFIX;
    /// Host-derived unknownness is the whole point of the maintenance family:
    /// a matcher that reads a fact Scryer could not observe is held before it
    /// is consulted.
    const TRACKS_REFERENCED_FACTS: bool = true;

    type Policy = MaintenancePolicy;
    type Input = MaintenanceInput;
    type Decision = MaintenanceDecision;
    type RuleExtra = ();
    type EvalContext = ();
    type EvalError = MaintenanceEvalError;

    fn limits() -> RuntimeLimits {
        RuntimeLimits::maintenance_defaults()
    }

    fn user_policy_path(rule_id: &str) -> String {
        user_policy_path(rule_id)
    }

    fn wrapper_policy_path(rule_id: &str) -> String {
        decision_wrapper_policy_path(rule_id)
    }

    fn wrapper_source(rule_id: &str) -> String {
        decision_wrapper_source(rule_id)
    }

    fn wrapper_rule_path(rule_id: &str) -> String {
        decision_wrapper_rule_path(rule_id)
    }

    fn rule_extra(_policy: &Self::Policy) -> Self::RuleExtra {}

    /// The host helpers (`days_since`) live in the rule's own package so an
    /// author calls them unqualified.
    fn support_modules(
        policy: &Self::Policy,
        module: &regorus::unstable::Module,
    ) -> Vec<(String, String)> {
        maintenance_actions::helper_modules(&policy.id, module)
    }

    /// A matcher whose fact dependencies cannot be read off its source must not
    /// load at all: the host could not then tell whether it is deciding on
    /// evidence Scryer actually has.
    fn referenced_facts(
        policy: &Self::Policy,
        policy_path: &str,
    ) -> Result<BTreeSet<String>, String> {
        validation::maintenance_fact_references(&policy.rego_source, policy_path)
    }

    fn referenced_facts_from_module(
        _policy: &Self::Policy,
        _policy_path: &str,
        module: &regorus::unstable::Module,
    ) -> Result<BTreeSet<String>, String> {
        validation::maintenance_fact_references_from_module(module)
    }

    fn hold_rule_name() -> Option<&'static str> {
        Some(HOLD_RULE_NAME)
    }

    fn held_decision(reason_codes: Vec<String>) -> Self::Decision {
        MaintenanceDecision {
            outcome: MaintenanceOutcome::Unknown,
            reason_codes,
            actions: MaintenanceActions::default(),
        }
    }

    fn decode(
        value: &Value,
        _rule_id: &str,
        _rule_name: &str,
        _extra: &Self::RuleExtra,
    ) -> Result<Self::Decision, String> {
        decode_decision(value)
    }

    /// The same action scan validation runs. A stored source that would no
    /// longer save, or whose actions the host cannot read, never decides.
    fn load_refusal(
        _policy: &Self::Policy,
        module: &regorus::unstable::Module,
    ) -> Option<crate::policy::PolicyRefusal> {
        maintenance_actions::load_refusal(module)
    }

    fn eval_error(
        rule: &RuleHandle<Self::RuleExtra>,
        kind: crate::policy::PolicyErrorKind,
        message: String,
    ) -> Self::EvalError {
        MaintenanceEvalError {
            rule_set_id: rule.id.clone(),
            rule_set_name: rule.name.clone(),
            code: kind.code(),
            message,
        }
    }
}

/// Pre-compiled engine holding every active maintenance matcher.
///
/// Built once per rule-set revision and shared; evaluators are cheap clones
/// created per evaluation run.
pub type MaintenanceRulesEngine = PolicyEngine<MaintenanceFamily>;

/// Evaluates every loaded matcher against one subject at a time.
pub type MaintenanceRulesEvaluator = PolicyEvaluator<MaintenanceFamily>;

impl MaintenanceRulesEvaluator {
    /// Evaluate every loaded matcher against one subject.
    ///
    /// Per-rule failures — compilation-clean rules that error at runtime,
    /// exceed the execution budget, or return a malformed decision — are
    /// collected and never abort the batch. A failing rule contributes no
    /// record, so it cannot advance or cancel a candidate.
    ///
    /// A rule that reads a fact Scryer could not observe for this subject is
    /// held before it is consulted at all. On the simple surface an unknown
    /// fact is simply a missing key, which a policy would otherwise read as a
    /// decisive "no" — so the host, not the author, is what makes an
    /// unobservable fact fail closed. Rules that opt out by reading
    /// `input.observations.*` are consulted normally and may still declare
    /// their own `unknown`, which composes with this one: either is enough to
    /// hold the subject.
    ///
    /// A held rule is not evaluated, so its reason codes are the observations'
    /// own — the operator sees *which fact* Scryer could not read, which is the
    /// actionable half. A policy's `reasons` still apply on every path where
    /// the policy is consulted at all, including its own `unknown`.
    pub fn evaluate(
        &mut self,
        input: &MaintenanceInput,
    ) -> Result<MaintenanceEvalResult, RulesError> {
        self.evaluate_policies(input, &())
    }
}

// ── Output validation ───────────────────────────────────────────────────────

/// Convert the wrapper's decision object into the closed output contract.
///
/// Fails closed: anything that is not exactly the declared shape becomes an
/// error for that rule instead of a coerced decision.
pub(crate) fn decode_decision(value: &Value) -> Result<MaintenanceDecision, String> {
    if matches!(value, Value::Undefined) {
        return Err("decision rule produced no value".to_string());
    }
    if value.as_object().is_err() {
        return Err("decision must be an object".to_string());
    }

    let unknown = *value["unknown"]
        .as_bool()
        .map_err(|_| "'unknown' must be a boolean".to_string())?;
    let reason_codes = decode_reasons(&value["reasons"])?;
    // Decoded before the outcome is known, so a head of the wrong type is an
    // error for the rule even on a subject the rule holds.
    let actions = decode_actions(value)?;

    // Unknown wins over every action: a rule that cannot see enough to decide
    // must hold the candidate rather than advance it. Otherwise a subject
    // matches exactly when at least one action head fired for it.
    let outcome = if unknown {
        MaintenanceOutcome::Unknown
    } else if !actions.is_empty() {
        MaintenanceOutcome::Match
    } else {
        MaintenanceOutcome::NoMatch
    };

    // A held subject gets no actions, whatever the heads evaluated to.
    let actions = if outcome == MaintenanceOutcome::Unknown {
        MaintenanceActions::default()
    } else {
        actions
    };

    Ok(MaintenanceDecision {
        outcome,
        reason_codes,
        actions,
    })
}

// ── Synthetic input ─────────────────────────────────────────────────────────

/// Build a representative input for validation dry-runs: a Title subject with
/// every fact known, so a rule reaching for any documented fact executes its
/// real path. Nothing is unknown here on purpose — validation is about whether
/// the rule is well-formed, and a rule held for unobservable facts would prove
/// nothing about that.
pub(crate) fn synthetic_maintenance_input() -> MaintenanceInput {
    MaintenanceInput {
        schema_version: MAINTENANCE_INPUT_SCHEMA_VERSION,
        evaluation_time: DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
            .expect("fixed synthetic timestamp is in range"),
        subject: MaintenanceSubjectDoc {
            subject_id: "title-1".into(),
            collection_id: None,
            kind: MaintenanceSubjectKind::Title,
            title_id: "title-1".to_string(),
            season_number: None,
            episode_id: None,
            facet: "movie".to_string(),
            name: "Test Movie".to_string(),
            year: Some(2024),
        },
        library: MaintenanceLibraryDoc {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
        },
        facts: MaintenanceFactsDoc {
            monitored: Observation::known(true),
            tags: Observation::known(vec!["keep".to_string()]),
            quality_profile_id: Observation::known("profile-1".to_string()),
            added_at: Observation::known("2024-01-01T00:00:00Z".to_string()),
            added_by_user_id: Observation::known("user-1".to_string()),
            added_by_username: Observation::known("operator".to_string()),
            requested: Observation::known(true),
            requested_by_user_ids: Observation::known(vec!["user-1".to_string()]),
            requested_by_usernames: Observation::known(vec!["operator".to_string()]),
            first_imported_at: Observation::known("2024-01-02T00:00:00Z".to_string()),
            last_upgraded_at: Observation::known("2024-02-01T00:00:00Z".to_string()),
            has_file: Observation::known(true),
            file_count: Observation::known(1),
            total_file_size_bytes: Observation::known(8_000_000_000),
            files: Observation::known(vec![MaintenanceFileDoc {
                is_3d: false,
                size_bytes: Some(8_000_000_000),
                quality: Some("2160P".to_string()),
                video_codec: Some("hevc".to_string()),
                video_width: Some(3840),
                video_height: Some(2160),
                audio_languages: vec!["eng".to_string()],
                subtitle_languages: vec!["eng".to_string()],
                audio_codec: Some("eac3".to_string()),
                audio_channels: Some(6),
                audio_bitrate_kbps: Some(640),
                video_bitrate_kbps: Some(16_000),
                video_bit_depth: Some(10),
                video_hdr_format: Some("hdr10".to_string()),
                added_at: Some("2024-01-02T00:00:00Z".to_string()),
            }]),
            episode_count: Observation::known(0),
            episode_file_count: Observation::known(0),
            monitored_episode_count: Observation::known(0),
            episode_position_by_air_date: Observation::known(1),
            season_position_by_air_date: Observation::known(1),
            active_downloads: Observation::known(false),
            watched_by_user_ids: Observation::known(vec!["user-1".to_string()]),
            last_watched_at: Observation::known("2024-03-01T00:00:00Z".to_string()),
            watched_by_any_requester: Observation::known(true),
            watched_by_all_requesters: Observation::known(true),
            watched_by_anyone: Observation::known(true),
            last_watched_by_anyone_at: Observation::known("2024-03-02T00:00:00Z".to_string()),
            request_lease_state: Observation::known("none".to_string()),
            // The one deliberately absent fact in the synthetic document: a
            // title with no live lease has no expiry, and inventing one would
            // make a dry-run of "the lease lapsed before <date>" pass against a
            // date no instance ever produced.
            request_lease_expires_at: Observation::absent(),
            active_retention_claims: Observation::known(0),
            lists_added_by_list: Observation::known(true),
            lists_on_enabled_list: Observation::known(false),
            lists_names: Observation::known(Vec::new()),
            lists_left_all: Observation::known(true),
            lists_left_at: Observation::known("2024-04-01T00:00:00Z".to_string()),
            lists_last_list_name: Observation::known("Fixture Picks".to_string()),
            lists_enabled_ids: Observation::known(Vec::new()),
            lists_public_count: Observation::known(0),
            lists_personal_count: Observation::known(0),
            lists_first_seen_at: Observation::known("2024-01-01T00:00:00Z".to_string()),
            lists_last_seen_at: Observation::known("2024-03-31T00:00:00Z".to_string()),
            lists_days_on_list: Observation::known(120),
            lists_best_rank: Observation::known(1),
            lists_on_leave: Observation::known(Vec::new()),
            lists_left_handled: Observation::known(true),
            lists_membership_states: Observation::known(vec!["in_library".to_string()]),
            request_origin: Observation::known("public_list".to_string()),
            request_list_name: Observation::known("Fixture Picks".to_string()),
            // A movie subject: series movies do not apply to it at all.
            series_movies: Observation::absent(),
            storage_root_id: Observation::absent(),
            storage_available_bytes: Observation::absent(),
            storage_total_bytes: Observation::absent(),
            storage_available_percent: Observation::absent(),
            runtime_minutes: Observation::known(118),
            original_language: Observation::known("en".to_string()),
            // A movie subject: a show's status does not apply to it.
            series_status: Observation::absent(),
            first_aired: Observation::absent(),
            network: Observation::absent(),
            studio: Observation::known("Fixture Studio".to_string()),
            country: Observation::known("us".to_string()),
            popularity: Observation::known(12.5),
            theatrical_release_date: Observation::known("2023-11-10".to_string()),
            digital_release_date: Observation::known("2024-01-05".to_string()),
            physical_release_date: Observation::known("2024-02-20".to_string()),
            days_since_added: Observation::known(120),
            max_video_height: Observation::known(2160),
            season_count: Observation::known(3),
            aired_episode_count: Observation::known(28),
            unaired_episode_count: Observation::known(2),
            missing_episode_count: Observation::known(1),
            aired_episode_file_percent: Observation::known(96.4),
            latest_season_number: Observation::known(3),
            latest_aired_episode_date: Observation::known("2024-02-01".to_string()),
            next_episode_air_date: Observation::known("2024-02-08".to_string()),
            genres: Observation::known(vec!["Drama".to_string()]),
            rating: Observation::known(7.4),
            ratings: Observation::known(vec![MaintenanceRatingDoc {
                source: "imdb".to_string(),
                value: Some(7.4),
                normalized: 7.4,
                votes: Some(1200),
            }]),
            play_count_total: Observation::known(3),
            watchlisted_by_user_ids: Observation::known(vec!["user-1".to_string()]),
            watchlisted_by_usernames: Observation::known(vec!["operator".to_string()]),
            watchlisted_by_anyone: Observation::known(true),
            watchlisted_by_any_requester: Observation::known(true),
            watchlist_added_at: Observation::known("2023-12-01T00:00:00Z".to_string()),
            watchlist_origins: Observation::known(vec!["trakt".to_string()]),
            favorited_by_user_ids: Observation::known(vec!["user-1".to_string()]),
            favorited_by_usernames: Observation::known(vec!["member".to_string()]),
            favorited_by_anyone: Observation::known(true),
            favorited_by_any_requester: Observation::absent(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU32;
    use core::time::Duration;

    fn policy(id: &str, body: &str) -> MaintenancePolicy {
        MaintenancePolicy {
            id: id.to_string(),
            name: format!("rule {id}"),
            rego_source: rewrite_package_declaration(body, id),
        }
    }

    fn evaluate(policies: &[MaintenancePolicy]) -> MaintenanceEvalResult {
        evaluate_against(policies, synthetic_maintenance_input())
    }

    fn evaluate_against(
        policies: &[MaintenancePolicy],
        input: MaintenanceInput,
    ) -> MaintenanceEvalResult {
        let engine = MaintenanceRulesEngine::build(policies).expect("policies should compile");
        engine
            .evaluator()
            .evaluate(&input)
            .expect("evaluation should succeed")
    }

    #[test]
    fn stereo_boolean_is_available_to_maintenance_rules() {
        let policy = policy(
            "stereo_file",
            "unmonitor if { input.facts.files[_].is_3d == true }",
        );
        let validation =
            crate::validation::validate_maintenance_rule(&policy.rego_source, &policy.id).unwrap();
        assert!(validation.valid, "{:?}", validation.errors);
        for is_3d in [false, true] {
            let mut input = synthetic_maintenance_input();
            input.facts.files = Observation::known(vec![MaintenanceFileDoc {
                is_3d,
                size_bytes: None,
                quality: None,
                video_codec: None,
                video_width: None,
                video_height: None,
                audio_languages: vec![],
                subtitle_languages: vec![],
                audio_codec: None,
                audio_channels: None,
                audio_bitrate_kbps: None,
                video_bitrate_kbps: None,
                video_bit_depth: None,
                video_hdr_format: None,
                added_at: None,
            }]);
            let result = evaluate_against(std::slice::from_ref(&policy), input);
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(
                result.records[0].decision.outcome,
                if is_3d {
                    MaintenanceOutcome::Match
                } else {
                    MaintenanceOutcome::NoMatch
                }
            );
        }
        let mut input = synthetic_maintenance_input();
        input.facts.files = Observation::unknown("not_examined");
        let result = evaluate_against(&[policy], input);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown
        );
    }

    #[test]
    fn matching_rule_reports_match() {
        let result = evaluate(&[policy(
            "monitored_movie",
            "unmonitor if {\n  input.facts.monitored\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.records.len(), 1);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
        assert!(result.records[0].decision.reason_codes.is_empty());
    }

    #[test]
    fn evaluator_reuse_tracks_changed_known_and_unknown_facts() {
        let engine = MaintenanceRulesEngine::build(&[policy(
            "monitored_movie",
            "unmonitor if {\n  input.facts.monitored\n}\n",
        )])
        .expect("policy should compile");
        let mut evaluator = engine.evaluator();

        let mut input = synthetic_maintenance_input();
        input.facts.monitored = Observation::known(true);
        assert_eq!(
            evaluator.evaluate(&input).unwrap().records[0]
                .decision
                .outcome,
            MaintenanceOutcome::Match
        );

        input.facts.monitored = Observation::known(false);
        assert_eq!(
            evaluator.evaluate(&input).unwrap().records[0]
                .decision
                .outcome,
            MaintenanceOutcome::NoMatch
        );

        input.facts.monitored = Observation::unknown("source_unavailable");
        let held = evaluator.evaluate(&input).unwrap();
        assert_eq!(
            held.records[0].decision.outcome,
            MaintenanceOutcome::Unknown
        );
        assert_eq!(
            held.records[0].decision.reason_codes,
            vec!["source_unavailable".to_string()]
        );

        input.facts.monitored = Observation::known(true);
        assert_eq!(
            evaluator.evaluate(&input).unwrap().records[0]
                .decision
                .outcome,
            MaintenanceOutcome::Match
        );
    }

    #[test]
    fn undefined_match_reports_no_match() {
        let result = evaluate(&[policy(
            "never_matches",
            "unmonitor if {\n  input.subject.facet == \"series\"\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::NoMatch
        );
    }

    /// The manual surface: `input.observations.*` never triggers host-derived
    /// unknownness, so a rule that opts into it still owns its own `unknown`.
    #[test]
    fn a_policy_declared_unknown_takes_precedence_over_match() {
        let result = evaluate(&[policy(
            "holds_on_unknown",
            "unmonitor := true\n\nunknown if {\n  input.observations.last_upgraded_at.status == \"known\"\n}\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown
        );
    }

    /// The core of schema v2: no `unknown if` is written anywhere, and the rule
    /// is still held because a fact it reads is one Scryer could not observe.
    #[test]
    fn a_rule_reading_an_unobservable_fact_is_held_without_declaring_unknown() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");

        let result = evaluate_against(
            &[policy(
                "downloads_in_flight",
                "unmonitor if {\n  not input.facts.active_downloads\n}\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown,
            "a missing key would otherwise read as a decisive 'no downloads'"
        );
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["not_yet_collected".to_string()]
        );
    }

    #[test]
    fn a_negated_unavailable_retention_rank_is_held_not_matched() {
        let mut input = synthetic_maintenance_input();
        input.facts.episode_position_by_air_date =
            Observation::unknown("retention_rank_unavailable");

        let result = evaluate_against(
            &[policy(
                "not_ranked_episode",
                "unmonitor if {\n  not input.facts.episode_position_by_air_date\n}\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Unknown,
            "a missing rank must never become a destructive retention match"
        );
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["retention_rank_unavailable".to_string()]
        );
    }

    #[test]
    fn retention_examples_keep_the_newest_downloaded_episodes_and_seasons() {
        // These are the editor's scope-specific examples: rank one is newest,
        // so a policy deletes only positions after the retained window.
        const KEEP_NEWEST_TWO_DOWNLOADED_EPISODES: &str =
            "unmonitor if {\n  input.facts.episode_position_by_air_date > 2\n}\n";
        const KEEP_NEWEST_ONE_DOWNLOADED_SEASON: &str =
            "unmonitor if {\n  input.facts.season_position_by_air_date > 1\n}\n";

        let mut episode_input = synthetic_maintenance_input();
        episode_input.subject.kind = MaintenanceSubjectKind::Episode;
        episode_input.subject.episode_id = Some("episode-3".to_string());
        episode_input.facts.episode_position_by_air_date = Observation::known(3);
        let episode_result = evaluate_against(
            &[policy(
                "keep_newest_two_downloaded_episodes",
                KEEP_NEWEST_TWO_DOWNLOADED_EPISODES,
            )],
            episode_input,
        );

        let mut season_input = synthetic_maintenance_input();
        season_input.subject.kind = MaintenanceSubjectKind::Season;
        season_input.subject.collection_id = Some("season-2".to_string());
        season_input.subject.season_number = Some(2);
        season_input.facts.season_position_by_air_date = Observation::known(2);
        let season_result = evaluate_against(
            &[policy(
                "keep_newest_one_downloaded_season",
                KEEP_NEWEST_ONE_DOWNLOADED_SEASON,
            )],
            season_input,
        );

        for result in [&episode_result, &season_result] {
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(
                result.records[0].decision.outcome,
                MaintenanceOutcome::Match,
                "ranked subjects after the retained window must match their example"
            );
        }
    }

    /// Absence is an answer. `added_by_user_id` is absent for a scan-created
    /// title, and a rule keying on that must decide rather than hold.
    #[test]
    fn a_rule_matching_an_absent_fact_decides_rather_than_holding() {
        let mut input = synthetic_maintenance_input();
        input.facts.added_by_user_id = Observation::absent_because("title_added_by_system");

        let result = evaluate_against(
            &[policy(
                "system_added",
                "unmonitor if {\n  not input.facts.added_by_user_id\n  input.facts.has_file\n}\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
        assert!(result.records[0].decision.reason_codes.is_empty());
    }

    /// Every unobservable fact the rule reads contributes its reason, once.
    #[test]
    fn held_rules_report_the_union_of_the_reasons_that_held_them() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");
        input.facts.last_upgraded_at = Observation::unknown("not_yet_collected");
        input.facts.added_by_username = Observation::unknown("user_not_found");

        let result = evaluate_against(
            &[policy(
                "many_unknowns",
                "unmonitor if {\n  input.facts.active_downloads\n  input.facts.last_upgraded_at\n  \
                 input.facts.added_by_username == \"operator\"\n}\n",
            )],
            input,
        );

        assert_eq!(
            result.records[0].decision.reason_codes,
            vec![
                "not_yet_collected".to_string(),
                "user_not_found".to_string()
            ],
            "one code per distinct reason, in fact-name order"
        );
    }

    /// The opt-out has to actually opt out, or the advanced surface is
    /// unreachable: a rule reading the envelope sees the unknown itself.
    #[test]
    fn observation_references_do_not_trigger_host_derived_unknownness() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");

        let result = evaluate_against(
            &[policy(
                "inspects_the_envelope",
                "unmonitor if {\n  input.observations.active_downloads.status == \"unknown\"\n}\n\n\
                 reasons contains reason if {\n  \
                   reason := input.observations.active_downloads.reason\n\
                 }\n",
            )],
            input,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["not_yet_collected".to_string()]
        );
    }

    /// `evaluation_time` is the clock rules are meant to use, and date maths on
    /// it has to work — this is what the regorus `time` feature buys.
    #[test]
    fn evaluation_time_supports_date_arithmetic() {
        const AGE_MATCHER: &str = "day_ns := (24 * 60 * 60) * 1000000000\n\n\
             unmonitor if {\n  \
               age := time.parse_rfc3339_ns(input.evaluation_time) - \
             time.parse_rfc3339_ns(input.facts.added_at)\n  \
               age > 180 * day_ns\n\
             }\n";

        // Synthetic input is evaluated at 2023-11-14.
        let mut old = synthetic_maintenance_input();
        old.facts.added_at = Observation::known("2020-01-01T00:00:00Z".to_string());
        let matched = evaluate_against(&[policy("added_long_ago", AGE_MATCHER)], old);
        assert!(matched.errors.is_empty(), "{:?}", matched.errors);
        assert_eq!(
            matched.records[0].decision.outcome,
            MaintenanceOutcome::Match,
            "date maths must actually run, not evaluate to undefined"
        );

        let mut recent = synthetic_maintenance_input();
        recent.facts.added_at = Observation::known("2023-11-01T00:00:00Z".to_string());
        let unmatched = evaluate_against(&[policy("added_recently", AGE_MATCHER)], recent);
        assert!(unmatched.errors.is_empty(), "{:?}", unmatched.errors);
        assert_eq!(
            unmatched.records[0].decision.outcome,
            MaintenanceOutcome::NoMatch
        );
    }

    #[test]
    fn reason_codes_are_collected_from_a_set_rule() {
        let result = evaluate(&[policy(
            "set_reasons",
            "unmonitor := true\n\nreasons contains \"stale\"\n\nreasons contains \"unwatched\"\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["stale".to_string(), "unwatched".to_string()]
        );
    }

    #[test]
    fn reason_codes_are_collected_from_an_array_rule() {
        let result = evaluate(&[policy(
            "array_reasons",
            "unmonitor := true\n\nreasons := [\"first\", \"second\"]\n",
        )]);

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.records[0].decision.reason_codes,
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn non_boolean_match_is_an_error_not_a_decision() {
        let result = evaluate(&[policy("numeric_match", "unmonitor := 1\n")]);

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(
            result.errors[0].code,
            crate::diagnostic_codes::RULE_OUTPUT_INVALID,
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn oversized_reason_list_is_rejected() {
        let entries = (0..MAX_REASON_CODES + 1)
            .map(|i| format!("\"reason_{i}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let result = evaluate(&[policy(
            "too_many_reasons",
            &format!("unmonitor := true\n\nreasons := [{entries}]\n"),
        )]);

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("at most 32"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn overlong_reason_code_is_rejected() {
        let long = "x".repeat(MAX_REASON_CODE_LEN + 1);
        let result = evaluate(&[policy(
            "long_reason",
            &format!("unmonitor := true\n\nreasons := [\"{long}\"]\n"),
        )]);

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert!(
            result.errors[0].message.contains("at most 120"),
            "{}",
            result.errors[0].message
        );
    }

    #[test]
    fn a_broken_rule_does_not_stop_the_batch() {
        let result = evaluate(&[
            policy(
                "broken",
                "unmonitor if {\n  lower(input.subject.year) == \"x\"\n}\n",
            ),
            policy("healthy", "unmonitor := true\n"),
        ]);

        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].rule_set_id, "broken");
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].rule_set_id, "healthy");
        assert_eq!(
            result.records[0].decision.outcome,
            MaintenanceOutcome::Match
        );
    }

    #[test]
    fn execution_budget_turns_a_runaway_rule_into_an_error() {
        let mut limits = RuntimeLimits::maintenance_defaults();
        limits.max_execution_time = Duration::from_millis(1);
        limits.timer_check_interval = NonZeroU32::new(1).expect("non-zero");

        let policies = [policy(
            "runaway",
            "unmonitor if {\n  count([1 |\n    some i in numbers.range(1, 3000)\n    some j in numbers.range(1, 3000)\n    i == j\n  ]) > 0\n}\n",
        )];
        let engine =
            MaintenanceRulesEngine::build_with_limits(&policies, limits).expect("should compile");
        let result = engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect("evaluation should return, not hang");

        assert!(result.records.is_empty(), "{:?}", result.records);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].rule_set_id, "runaway");
    }

    #[test]
    fn records_carry_a_stable_policy_content_hash() {
        let policies = [policy("hashed", "unmonitor := true\n")];
        let first = evaluate(&policies);
        let second = evaluate(&policies);

        let hash = &first.records[0].policy_content_hash;
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, &second.records[0].policy_content_hash);
        assert_eq!(
            hash,
            &runtime::content_hash(&policies[0].rego_source),
            "record hash must be the hash of the stored source"
        );
    }

    #[test]
    fn empty_engine_evaluates_to_nothing() {
        let engine = MaintenanceRulesEngine::empty();
        assert!(engine.is_empty());
        assert_eq!(engine.rule_count(), 0);
        let result = engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect("evaluation should succeed");
        assert!(result.records.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn oversized_input_is_rejected_before_evaluation() {
        let mut limits = RuntimeLimits::maintenance_defaults();
        limits.max_input_bytes = 16;
        let engine = MaintenanceRulesEngine::build_with_limits(
            &[policy("tiny", "unmonitor := true\n")],
            limits,
        )
        .expect("should compile");
        let err = engine
            .evaluator()
            .evaluate(&synthetic_maintenance_input())
            .expect_err("input should exceed the bound");
        assert!(matches!(err, RulesError::InputTooLarge { .. }), "{err:?}");
    }

    #[test]
    fn the_input_document_carries_bare_facts_and_full_observations() {
        let mut input = synthetic_maintenance_input();
        input.facts.active_downloads = Observation::unknown("not_yet_collected");
        input.facts.added_by_user_id = Observation::absent_because("title_added_by_system");

        let doc = serde_json::to_value(&input).expect("input serializes");

        assert_eq!(doc["schema_version"], 2);
        // Known: bare value on facts, envelope on observations.
        assert_eq!(doc["facts"]["monitored"], true);
        assert_eq!(doc["observations"]["monitored"]["status"], "known");
        assert_eq!(doc["observations"]["monitored"]["value"], true);
        // Unknown and absent: missing from facts entirely, still on
        // observations with the reason that explains them.
        for fact in ["active_downloads", "added_by_user_id"] {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert!(doc["observations"][fact]["reason"].is_string(), "{fact}");
        }
        assert_eq!(doc["observations"]["active_downloads"]["status"], "unknown");
        assert_eq!(doc["observations"]["added_by_user_id"]["status"], "absent");
        // The two namespaces describe the same facts, so neither can gain a
        // key the other never heard of.
        for fact in doc["facts"].as_object().expect("facts object").keys() {
            assert!(doc["observations"].get(fact).is_some(), "{fact}");
        }
    }

    #[test]
    fn unknown_observations_serialize_without_a_value() {
        let doc = serde_json::to_value(Observation::<bool>::unknown("source_unreachable"))
            .expect("serializable");
        assert_eq!(doc["status"], "unknown");
        assert_eq!(doc["reason"], "source_unreachable");
        assert!(doc.get("value").is_none());

        let absent = serde_json::to_value(Observation::<bool>::absent()).expect("serializable");
        assert_eq!(absent["status"], "absent");
        assert!(absent.get("observed_at").is_none());
    }

    /// The wrapper is what every stored matcher is evaluated through, so its
    /// text is a contract with the rules already in people's databases. It is
    /// generated by the shared core now; it must still come out character for
    /// character as it always did.
    #[test]
    fn the_generated_wrapper_is_unchanged_and_projects_the_family_hold_head() {
        assert_eq!(
            decision_wrapper_source("rule_1"),
            "package scryer.maintenance.wrapper.rule_1\n\
             import rego.v1\n\n\
             decision := {\n\
             \t\"unknown\": object.get(data.scryer.maintenance.user.rule_1, \"unknown\", false),\n\
             \t\"reasons\": object.get(data.scryer.maintenance.user.rule_1, \"reasons\", []),\n\
             \t\"unmonitor\": object.get(data.scryer.maintenance.user.rule_1, \"unmonitor\", false),\n\
             \t\"delete_files\": object.get(data.scryer.maintenance.user.rule_1, \"delete_files\", false),\n\
             \t\"delete_title\": object.get(data.scryer.maintenance.user.rule_1, \"delete_title\", false),\n\
             \t\"search\": object.get(data.scryer.maintenance.user.rule_1, \"search\", false),\n\
             \t\"set_quality_profile\": object.get(data.scryer.maintenance.user.rule_1, \"set_quality_profile\", null),\n\
             \t\"add_tags\": object.get(data.scryer.maintenance.user.rule_1, \"add_tags\", []),\n\
             \t\"remove_tags\": object.get(data.scryer.maintenance.user.rule_1, \"remove_tags\", []),\n\
             \t\"monitor\": object.get(data.scryer.maintenance.user.rule_1, \"monitor\", false),\n\
             \t\"fetch_subtitles\": object.get(data.scryer.maintenance.user.rule_1, \"fetch_subtitles\", false),\n\
             \t\"move_to_library\": object.get(data.scryer.maintenance.user.rule_1, \"move_to_library\", null),\n\
             \t\"move_to_root\": object.get(data.scryer.maintenance.user.rule_1, \"move_to_root\", null),\n\
             \t\"notify\": object.get(data.scryer.maintenance.user.rule_1, \"notify\", null),\n\
             }\n"
        );
        assert_eq!(
            decision_wrapper_rule_path("rule_1"),
            "data.scryer.maintenance.wrapper.rule_1.decision"
        );
        assert_eq!(user_policy_path("rule_1"), "maintenance/rule_1.rego");
        assert_eq!(
            decision_wrapper_policy_path("rule_1"),
            "internal/rule_1_maintenance_wrapper.rego"
        );

        // Maintenance says `unknown if`; the request family will say
        // `manual if`. The head the wrapper reads is exactly what the family
        // declares, so the two cannot drift apart.
        let hold = MaintenanceFamily::hold_rule_name().expect("maintenance holds");
        assert_eq!(hold, "unknown");
        assert!(
            decision_wrapper_source("rule_1").contains(&format!("\"{hold}\", false)")),
            "the wrapper must read the head the family declares"
        );
    }

    #[test]
    fn an_action_head_written_under_data_is_refused_at_compile() {
        // The action scan reads heads by their leading name, so a head
        // spelled as a full `data.` path would slip past it. The Rego compiler
        // refuses any rule that shadows `data`, which keeps such a head from
        // ever deciding an action.
        for body in [
            "data.scryer.maintenance.user.data_head.delete_files := true\n",
            "data.scryer.maintenance.user.data_head.delete_title if { true }\n",
        ] {
            assert!(
                MaintenanceRulesEngine::build(&[policy("data_head", body)]).is_err(),
                "{body}"
            );
            let validated = crate::validation::validate_maintenance_rule(
                &rewrite_package_declaration(body, "data_head"),
                "data_head",
            )
            .expect("validation runs");
            assert!(!validated.valid, "{body}");
            let codes: Vec<&str> = validated
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect();
            assert_eq!(
                codes,
                vec![crate::diagnostic_codes::RULE_COMPILATION_ERROR],
                "{body}"
            );
        }
    }
}
