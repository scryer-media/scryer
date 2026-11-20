//! The request input document and the reads that fill it.
//!
//! Two halves, deliberately separated:
//!
//! - [`build_request_input`] is **pure**. Context in, document out, no clock, no
//!   repository. Every rule about how a fact turns into an [`Observation`] —
//!   what makes it `unknown` rather than `absent`, which certification is the
//!   one that ranks, how an age in days is derived — lives here and is unit
//!   testable without a harness.
//! - [`AppUseCase::assemble_request_input_context`] is the async half: it
//!   performs each read exactly once and hands the result over.
//!
//! # Unknown is not absent
//!
//! The distinction is the entire safety story. `absent` means a source answered
//! and there is no value ("this film carries no US certification"); a rule reads
//! that as a real answer. `unknown` means Scryer could not find out, and the
//! policy core holds any rule that reads it, which arbitrates to manual review.
//! Everything derived from a snapshot group the enrichment could not establish
//! is `unknown`; everything a source answered "nothing" to is `absent`, with the
//! reason recorded so an approver sees *why*.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use scryer_domain::{
    AppPermission, ExternalAccountStatus, ExternalId, Library, LibraryPermission, ListScope,
    MediaFacet, MediaRequestOrigin, MonitorSelection, User, UserAccountKind,
};
use scryer_rules::request::{
    Observation, REQUEST_INPUT_SCHEMA_VERSION, RequestAwardDoc, RequestCertificationDoc,
    RequestClockDoc, RequestDoc, RequestFactsDoc, RequestInput, RequestLibraryDoc, RequestListDoc,
    RequestRequesterDoc, certification_rank_for_label, max_resolution_for_quality_tiers,
};

use crate::media_requests::snapshot::{MediaRequestMetadataSnapshot, SNAPSHOT_GROUP_ALL};
use crate::ports::TitleExternalIdLookup;
use crate::{AppResult, AppUseCase};

/// Snapshot group names the fact builder asks [`MediaRequestMetadataSnapshot::is_missing`]
/// about. They match the group vocabulary WP3 writes into `missing`.
const GROUP_CONTENT_RATINGS: &str = "content_ratings";
const GROUP_GENRES: &str = "genres";
const GROUP_MDBLIST: &str = "mdblist";
const GROUP_RATINGS: &str = "ratings";
const GROUP_AWARDS: &str = "awards";

/// Reason code used for every fact derived from a snapshot group enrichment
/// could not establish. It is the code a held rule reports, so an approver sees
/// "Scryer could not read the metadata", not "the film is not rated".
pub const REASON_METADATA_UNAVAILABLE: &str = fact_reason::METADATA_UNAVAILABLE;

/// Why a request fact is unknown or absent. Stable API: these codes reach rule
/// authors through `input.observations`, and an approver sees them on a held
/// rule. Reword nothing here; add new codes instead.
pub mod fact_reason {
    /// A snapshot group the enrichment could not establish.
    pub const METADATA_UNAVAILABLE: &str = "metadata_unavailable";
    /// The source has not been read for this title yet.
    pub const NOT_YET_COLLECTED: &str = "not_yet_collected";
    /// A series-shape fact asked of a movie.
    pub const NOT_A_SERIES: &str = "not_a_series";
    /// A release-date fact asked of a series.
    pub const NOT_A_MOVIE: &str = "not_a_movie";
    /// No dated episode airs after the evaluation day.
    pub const NO_SCHEDULED_EPISODE: &str = "no_scheduled_episode";
    pub const NO_THEATRICAL_RELEASE_DATE: &str = "no_theatrical_release_date";
    pub const NO_DIGITAL_RELEASE_DATE: &str = "no_digital_release_date";
    pub const NO_PHYSICAL_RELEASE_DATE: &str = "no_physical_release_date";
    /// The movie has no release date of any kind.
    pub const NO_RELEASE_DATE: &str = "no_release_date";
    /// The movie has a release on or before the evaluation day.
    pub const ALREADY_RELEASED: &str = "already_released";
    /// A catalog or request-history read failed.
    pub const CATALOG_UNAVAILABLE: &str = "catalog_unavailable";
    /// The requester has no linked watch-later list.
    pub const NO_WATCHLIST_LINKED: &str = "no_watchlist_linked";
    /// The title is not on any of the requester's watch-later lists.
    pub const NOT_ON_A_WATCHLIST: &str = "not_on_a_watchlist";
    /// A watch-later list has not completed a sync, or its last one failed.
    pub const WATCHLIST_NOT_SYNCED: &str = "watchlist_not_synced";
    /// A watch-later list's last sync is older than its freshness window.
    pub const WATCHLIST_STALE: &str = "watchlist_stale";
    /// The list store could not be read.
    pub const LIST_STORE_UNREADABLE: &str = "list_store_unreadable";
    /// The list exclusion store could not be read.
    pub const LIST_EXCLUSIONS_UNAVAILABLE: &str = "list_exclusions_unavailable";
    /// The requester's created titles could not be counted.
    pub const REQUESTER_TITLES_UNAVAILABLE: &str = "requester_titles_unavailable";
    /// The target library has no usable root.
    pub const STORAGE_ROOT_UNAVAILABLE: &str = "storage_root_unavailable";
    /// No capacity sample of the root since startup.
    pub const STORAGE_NOT_SAMPLED: &str = crate::disk::capacity::reason::STORAGE_NOT_SAMPLED;
    /// The root's capacity sample is older than two sampling intervals.
    pub const STORAGE_SAMPLE_STALE: &str = crate::disk::capacity::reason::STORAGE_SAMPLE_STALE;
    /// The root's newest capacity sample could not read the filesystem.
    pub const STORAGE_CAPACITY_UNAVAILABLE: &str =
        crate::disk::capacity::reason::STORAGE_CAPACITY_UNAVAILABLE;

    pub const ALL: &[&str] = &[
        METADATA_UNAVAILABLE,
        NOT_YET_COLLECTED,
        NOT_A_SERIES,
        NOT_A_MOVIE,
        NO_SCHEDULED_EPISODE,
        NO_THEATRICAL_RELEASE_DATE,
        NO_DIGITAL_RELEASE_DATE,
        NO_PHYSICAL_RELEASE_DATE,
        NO_RELEASE_DATE,
        ALREADY_RELEASED,
        CATALOG_UNAVAILABLE,
        NO_WATCHLIST_LINKED,
        NOT_ON_A_WATCHLIST,
        WATCHLIST_NOT_SYNCED,
        WATCHLIST_STALE,
        LIST_STORE_UNREADABLE,
        LIST_EXCLUSIONS_UNAVAILABLE,
        REQUESTER_TITLES_UNAVAILABLE,
        STORAGE_ROOT_UNAVAILABLE,
        STORAGE_NOT_SAMPLED,
        STORAGE_SAMPLE_STALE,
        STORAGE_CAPACITY_UNAVAILABLE,
    ];
}

/// The facts drawn from the requester's watch-later lists.
pub(crate) const WATCHLIST_FACT_NAMES: &[&str] =
    &["on_requester_watchlist", "requester_watchlist_days"];

/// A watch-later list is fresh for eight of its sync intervals, and never
/// less than two days: the same window lifecycle watchlist facts use.
fn watchlist_freshness_window(interval_seconds: i64) -> chrono::Duration {
    chrono::Duration::seconds(interval_seconds.max(0).saturating_mul(8))
        .max(chrono::Duration::hours(48))
}

/// Country codes that count as the United States certification. The rank ladder
/// [`certification_rank_for_label`] implements is the US one, so ranking a
/// German FSK label against it would be a category error; a title with only
/// foreign certifications leaves the label absent and the rank with it.
const US_COUNTRY_CODES: [&str; 4] = ["us", "usa", "united states", "united states of america"];

/// The draft a rule is evaluated against — the same shape whether it came from
/// a pre-flight preview, a submit, or an edit of a pending request.
#[derive(Clone, Debug)]
pub struct RequestDraft {
    pub facet: MediaFacet,
    pub title: String,
    pub year: Option<i32>,
    pub external_ids: Vec<ExternalId>,
    pub identity_fingerprint: String,
    pub quality_profile_id: Option<String>,
    pub quality_profile_name: Option<String>,
    pub monitor_type: Option<String>,
    pub monitor_selection: Option<MonitorSelection>,
    /// `None` means forever.
    pub requested_lease_days: Option<i64>,
    /// `None` leaves it to the library; `Some(vec![])` asks for no requirement.
    pub requested_required_audio_languages: Option<Vec<String>>,
    /// `None` leaves it to the library.
    pub requested_preferred_audio_languages: Option<Vec<String>>,
    pub origin: MediaRequestOrigin,
    /// The item's rank on the list that submitted it, when the list ranks it.
    pub list_rank: Option<i64>,
}

/// The quality profile the draft names, resolved once.
#[derive(Clone, Debug)]
pub struct RequestQualityContext {
    pub tiers: Vec<String>,
    pub allow_upgrades: bool,
}

/// What the catalog knows about this identity.
#[derive(Clone, Debug, Default)]
pub struct RequestCatalogContext {
    /// Libraries of the same facet that already hold the identity, excluding
    /// the request's own target library.
    pub exists_in_library_ids: Vec<String>,
    pub previous_request_count: i64,
    pub previously_denied: bool,
    pub previously_approved: bool,
    /// Distinct people other than the requester who created or joined a
    /// request for this identity.
    pub other_requester_count: i64,
    /// Someone other than the requester has a pending request for it.
    pub pending_by_other_requester: bool,
    /// False when a read failed; every catalog fact then reads unknown.
    pub readable: bool,
}

/// The requester's own watch-later lists, as they bear on this title.
#[derive(Clone, Debug)]
pub enum RequestWatchlistContext {
    /// The requester has no linked watch-later list.
    NotLinked,
    /// A list exists but cannot be trusted or read; the code says why.
    Unavailable(&'static str),
    /// The lists are current. `first_seen_at` is when the title first
    /// appeared on any of them, `None` when it is on none.
    Known {
        first_seen_at: Option<DateTime<Utc>>,
    },
}

/// The capacity of the target library's root, from the capacity sampler.
#[derive(Clone, Debug)]
pub struct RequestStorageContext {
    /// The root a new title would land on, `None` when the library has none.
    pub root_id: Option<String>,
    /// The figures, or the reason code they are unknown.
    pub capacity: Result<RequestStorageFigures, &'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RequestStorageFigures {
    pub sampled_at: DateTime<Utc>,
    pub available_bytes: i64,
    pub total_bytes: i64,
    pub available_percent: f64,
}

impl RequestStorageContext {
    /// A library with no root to sample.
    pub fn no_root() -> Self {
        Self {
            root_id: None,
            capacity: Err(fact_reason::STORAGE_ROOT_UNAVAILABLE),
        }
    }
}

/// What the instance knows about this requester's own history.
#[derive(Clone, Debug, Default)]
pub struct RequestRequesterHistoryContext {
    pub pending_request_count: i64,
    pub approved_last_30d: i64,
    pub denied_last_30d: i64,
    pub total_approved: i64,
    /// `None` when the claim store could not be read — distinct from zero.
    pub active_lease_count: Option<i64>,
    /// `None` is a real answer: this person has never submitted anything.
    pub last_request_at: Option<DateTime<Utc>>,
    /// False when a read failed; the four counters then read unknown.
    pub readable: bool,
}

/// Everything one evaluation needs, already read.
#[derive(Clone, Debug)]
pub struct RequestInputContext {
    pub evaluation_time: DateTime<Utc>,
    pub requester: RequestRequesterDoc,
    pub library: RequestLibraryDoc,
    pub request: RequestDoc,
    pub snapshot: MediaRequestMetadataSnapshot,
    /// `None` when the draft's profile could not be resolved; the three quality
    /// facts then read unknown rather than defaulting to a permissive tier list.
    pub quality: Option<RequestQualityContext>,
    pub catalog: RequestCatalogContext,
    pub history: RequestRequesterHistoryContext,
    /// `None` when no count is available; the fact reads unknown.
    pub library_title_count: Option<i64>,
    pub watchlist: RequestWatchlistContext,
    /// `None` when the exclusion store could not be read.
    pub list_excluded: Option<bool>,
    /// `None` when the count could not be read.
    pub requester_titles_in_library_count: Option<i64>,
    pub storage: RequestStorageContext,
}

// ── The pure builder ────────────────────────────────────────────────────────

/// Turn a fully-read context into the document the engine evaluates.
pub fn build_request_input(context: RequestInputContext) -> RequestInput {
    let facts = build_facts(&context);
    RequestInput {
        schema_version: REQUEST_INPUT_SCHEMA_VERSION,
        evaluation_time: context.evaluation_time,
        now: RequestClockDoc::at(context.evaluation_time),
        requester: context.requester,
        library: context.library,
        request: context.request,
        facts,
    }
}

fn build_facts(context: &RequestInputContext) -> RequestFactsDoc {
    let snapshot = &context.snapshot;
    let ratings_known = !snapshot.is_missing(GROUP_RATINGS);

    RequestFactsDoc {
        // ── content rating ──
        age_rating: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            snapshot_age_rating(snapshot).ok_or("no_age_rating")
        }),
        certifications: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            let certifications = certification_docs(&snapshot.content_ratings);
            if certifications.is_empty() {
                Err("no_certifications")
            } else {
                Ok(certifications)
            }
        }),
        certification_label: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            us_certification_label(snapshot).ok_or("no_us_certification")
        }),
        certification_rank: snapshot_fact(snapshot, GROUP_CONTENT_RATINGS, || {
            let Some(label) = us_certification_label(snapshot) else {
                return Err("no_us_certification");
            };
            // A label off the ladder is an *absence* of a rank, not an unknown:
            // the source answered, and Scryer's ladder simply does not place
            // that string. A rule comparing ranks then does not match, which is
            // the same thing it does for an unrated title.
            certification_rank_for_label(&label).ok_or("unrankable_certification")
        }),
        commonsense_recommended: snapshot_fact(snapshot, GROUP_MDBLIST, || {
            snapshot
                .mdblist
                .as_ref()
                .and_then(|mdblist| mdblist.commonsense)
                .ok_or("no_commonsense_rating")
        }),

        // ── title metadata ──
        genres: snapshot_fact(snapshot, GROUP_GENRES, || {
            let genres: Vec<String> = snapshot.genres.clone();
            if genres.is_empty() {
                Err("no_genres")
            } else {
                Ok(genres)
            }
        }),
        canonical_tag_keys: snapshot_fact(snapshot, GROUP_GENRES, || {
            let keys: Vec<String> = snapshot
                .canonical_tags
                .iter()
                .map(|tag| tag.key.clone())
                .collect();
            if keys.is_empty() {
                Err("no_canonical_tags")
            } else {
                Ok(keys)
            }
        }),
        themes: snapshot_fact(snapshot, GROUP_GENRES, || {
            let themes: Vec<String> = snapshot
                .canonical_tags
                .iter()
                .filter(|tag| tag.category.eq_ignore_ascii_case("theme"))
                .map(|tag| {
                    if tag.name.trim().is_empty() {
                        tag.key.clone()
                    } else {
                        tag.name.clone()
                    }
                })
                .collect();
            if themes.is_empty() {
                Err("no_themes")
            } else {
                Ok(themes)
            }
        }),
        // The adult flag is derived from the canonical tags, so it is known
        // exactly when they are — and `false` there is a real answer, not a
        // default: the tags were read and none of them is adult.
        is_adult: snapshot_fact(snapshot, GROUP_GENRES, || Ok(snapshot.is_adult)),
        rating: snapshot_fact(snapshot, GROUP_RATINGS, || {
            snapshot.ratings.rating.ok_or("no_rating")
        }),
        ratings_by_source: snapshot_fact(snapshot, GROUP_RATINGS, || {
            let by_source: BTreeMap<String, f64> = snapshot
                .ratings
                .external_ratings
                .iter()
                .filter(|rating| !rating.source.trim().is_empty())
                .map(|rating| (rating.source.trim().to_ascii_lowercase(), rating.normalized))
                .collect();
            if by_source.is_empty() {
                Err("no_external_ratings")
            } else {
                Ok(by_source)
            }
        }),
        tmdb_vote_average: optional_snapshot_fact(
            ratings_known,
            snapshot.tmdb_vote_average,
            "no_tmdb_vote_average",
        ),
        tmdb_vote_count: optional_snapshot_fact(
            ratings_known,
            snapshot.tmdb_vote_count,
            "no_tmdb_vote_count",
        ),
        popularity: optional_snapshot_fact(ratings_known, snapshot.popularity, "no_popularity"),
        runtime_minutes: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.runtime_minutes.map(i64::from),
            "no_runtime",
        ),
        original_language: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.original_language.clone(),
            "no_original_language",
        ),
        country: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.country.clone(),
            "no_country",
        ),
        network: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.network.clone(),
            "no_network",
        ),
        studio: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.studio.clone(),
            "no_studio",
        ),
        content_status: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.content_status.clone(),
            "no_content_status",
        ),
        release_date: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.release_date.clone(),
            "no_release_date",
        ),
        first_aired: optional_snapshot_fact(
            !snapshot.is_missing(SNAPSHOT_GROUP_ALL),
            snapshot.first_aired.clone(),
            "no_first_aired",
        ),
        release_age_days: if snapshot.is_missing(SNAPSHOT_GROUP_ALL) {
            Observation::unknown(REASON_METADATA_UNAVAILABLE)
        } else {
            // A series' first air date is its release date; a movie has only
            // the one. Whichever is present is what the age is measured from.
            match snapshot
                .release_date
                .as_deref()
                .or(snapshot.first_aired.as_deref())
            {
                None => Observation::absent_because("no_release_date"),
                Some(value) => match parse_release_date(value) {
                    None => Observation::absent_because("unparseable_release_date"),
                    Some(date) => {
                        Observation::known((context.evaluation_time.date_naive() - date).num_days())
                    }
                },
            }
        },
        award_count: snapshot_fact(snapshot, GROUP_AWARDS, || Ok(snapshot.awards.len() as i64)),
        awards: snapshot_fact(snapshot, GROUP_AWARDS, || Ok(award_docs(&snapshot.awards))),

        // ── quality ──
        quality_profile_tiers: match &context.quality {
            None => Observation::unknown("quality_profile_unavailable"),
            Some(quality) if quality.tiers.is_empty() => {
                Observation::absent_because("no_quality_tiers")
            }
            Some(quality) => Observation::known(quality.tiers.clone()),
        },
        quality_profile_max_resolution: match &context.quality {
            None => Observation::unknown("quality_profile_unavailable"),
            Some(quality) => match max_resolution_for_quality_tiers(&quality.tiers) {
                None => Observation::absent_because("no_resolution_in_quality_tiers"),
                Some(resolution) => Observation::known(resolution),
            },
        },
        quality_profile_allows_upgrades: match &context.quality {
            None => Observation::unknown("quality_profile_unavailable"),
            Some(quality) => Observation::known(quality.allow_upgrades),
        },

        // ── catalog ──
        exists_in_library_ids: catalog_fact(&context.catalog, || {
            context.catalog.exists_in_library_ids.clone()
        }),
        previous_request_count: catalog_fact(&context.catalog, || {
            context.catalog.previous_request_count
        }),
        previously_denied: catalog_fact(&context.catalog, || context.catalog.previously_denied),
        previously_approved: catalog_fact(&context.catalog, || context.catalog.previously_approved),

        // ── requester history ──
        pending_request_count: history_fact(&context.history, || {
            context.history.pending_request_count
        }),
        approved_last_30d: history_fact(&context.history, || context.history.approved_last_30d),
        denied_last_30d: history_fact(&context.history, || context.history.denied_last_30d),
        total_approved: history_fact(&context.history, || context.history.total_approved),
        active_lease_count: match context.history.active_lease_count {
            None => Observation::unknown("lifecycle_claims_unavailable"),
            Some(count) => Observation::known(count),
        },
        days_since_last_request: if !context.history.readable {
            Observation::unknown("request_history_unavailable")
        } else {
            match context.history.last_request_at {
                // Never having asked for anything is an answer, not a gap: a
                // rule can legitimately say "first-time requesters go to a
                // human" by testing `not input.facts.days_since_last_request`.
                None => Observation::absent_because("never_requested"),
                Some(last) => Observation::known(
                    (context.evaluation_time.date_naive() - last.date_naive()).num_days(),
                ),
            }
        },

        // ── library ──
        library_title_count: match context.library_title_count {
            None => Observation::unknown("not_yet_collected"),
            Some(count) => Observation::known(count),
        },
        ..series_shape_facts(context)
    }
}

/// The facts added after the first contract, built together so each group's
/// unknown and absent rules sit in one place.
fn series_shape_facts(context: &RequestInputContext) -> RequestFactsDoc {
    let today = context.evaluation_time.date_naive();
    let is_movie = context.request_facet_is_movie();
    let mut facts = empty_facts();

    // ── series shape ──
    if is_movie {
        facts.season_count = Observation::absent_because(fact_reason::NOT_A_SERIES);
        facts.episode_count = Observation::absent_because(fact_reason::NOT_A_SERIES);
        facts.aired_episode_count = Observation::absent_because(fact_reason::NOT_A_SERIES);
        facts.unaired_episode_count = Observation::absent_because(fact_reason::NOT_A_SERIES);
        facts.next_episode_air_date = Observation::absent_because(fact_reason::NOT_A_SERIES);
    } else {
        match context
            .snapshot
            .series_shape
            .as_ref()
            .filter(|_| !context.snapshot.is_missing(SNAPSHOT_GROUP_ALL))
        {
            None => {
                let unknown = fact_reason::METADATA_UNAVAILABLE;
                facts.season_count = Observation::unknown(unknown);
                facts.episode_count = Observation::unknown(unknown);
                facts.aired_episode_count = Observation::unknown(unknown);
                facts.unaired_episode_count = Observation::unknown(unknown);
                facts.next_episode_air_date = Observation::unknown(unknown);
            }
            Some(shape) => {
                let aired = shape.aired_on_or_before(today);
                facts.season_count = Observation::known(shape.season_count);
                facts.episode_count = Observation::known(shape.episode_count);
                facts.aired_episode_count = Observation::known(aired);
                facts.unaired_episode_count =
                    Observation::known((shape.episode_count - aired).max(0));
                facts.next_episode_air_date = match shape.next_air_date_after(today) {
                    Some(day) => Observation::known(day),
                    None => Observation::absent_because(fact_reason::NO_SCHEDULED_EPISODE),
                };
            }
        }
    }

    // ── release dates ──
    if !is_movie {
        facts.theatrical_release_date = Observation::absent_because(fact_reason::NOT_A_MOVIE);
        facts.digital_release_date = Observation::absent_because(fact_reason::NOT_A_MOVIE);
        facts.physical_release_date = Observation::absent_because(fact_reason::NOT_A_MOVIE);
        facts.home_release_available = Observation::absent_because(fact_reason::NOT_A_MOVIE);
        facts.days_until_release = Observation::absent_because(fact_reason::NOT_A_MOVIE);
    } else {
        let dates = context.snapshot.release_dates.as_ref();
        let unknown = match dates {
            _ if context.snapshot.is_missing(SNAPSHOT_GROUP_ALL) => {
                Some(fact_reason::METADATA_UNAVAILABLE)
            }
            None => Some(fact_reason::METADATA_UNAVAILABLE),
            Some(dates) if !dates.fetched => Some(fact_reason::NOT_YET_COLLECTED),
            Some(_) => None,
        };
        match (unknown, dates) {
            (None, Some(dates)) => {
                let day = |value: &Option<String>| {
                    value
                        .as_deref()
                        .and_then(crate::media_requests::snapshot::parse_metadata_day)
                };
                let date_fact = |value: &Option<String>, absent: &'static str| match day(value) {
                    Some(date) => Observation::known(date.format("%Y-%m-%d").to_string()),
                    None => Observation::absent_because(absent),
                };
                facts.theatrical_release_date =
                    date_fact(&dates.theatrical, fact_reason::NO_THEATRICAL_RELEASE_DATE);
                facts.digital_release_date =
                    date_fact(&dates.digital, fact_reason::NO_DIGITAL_RELEASE_DATE);
                facts.physical_release_date =
                    date_fact(&dates.physical, fact_reason::NO_PHYSICAL_RELEASE_DATE);
                let home = [day(&dates.digital), day(&dates.physical)];
                facts.home_release_available =
                    Observation::known(home.iter().flatten().any(|released| *released <= today));
                let all: Vec<NaiveDate> = [day(&dates.theatrical), home[0], home[1]]
                    .into_iter()
                    .flatten()
                    .collect();
                facts.days_until_release = match all.iter().min() {
                    None => Observation::absent_because(fact_reason::NO_RELEASE_DATE),
                    Some(earliest) if *earliest <= today => {
                        Observation::absent_because(fact_reason::ALREADY_RELEASED)
                    }
                    Some(earliest) => Observation::known((*earliest - today).num_days()),
                };
            }
            (reason, _) => {
                let reason = reason.unwrap_or(fact_reason::METADATA_UNAVAILABLE);
                facts.theatrical_release_date = Observation::unknown(reason);
                facts.digital_release_date = Observation::unknown(reason);
                facts.physical_release_date = Observation::unknown(reason);
                facts.home_release_available = Observation::unknown(reason);
                facts.days_until_release = Observation::unknown(reason);
            }
        }
    }

    // ── demand ──
    facts.other_requester_count =
        catalog_fact(&context.catalog, || context.catalog.other_requester_count);
    facts.pending_by_other_requester = catalog_fact(&context.catalog, || {
        context.catalog.pending_by_other_requester
    });

    // ── the requester's watch-later lists ──
    match &context.watchlist {
        RequestWatchlistContext::NotLinked => {
            facts.on_requester_watchlist =
                Observation::absent_because(fact_reason::NO_WATCHLIST_LINKED);
            facts.requester_watchlist_days =
                Observation::absent_because(fact_reason::NO_WATCHLIST_LINKED);
        }
        RequestWatchlistContext::Unavailable(reason) => {
            facts.on_requester_watchlist = Observation::unknown(*reason);
            facts.requester_watchlist_days = Observation::unknown(*reason);
        }
        RequestWatchlistContext::Known { first_seen_at } => {
            facts.on_requester_watchlist = Observation::known(first_seen_at.is_some());
            facts.requester_watchlist_days = match first_seen_at {
                None => Observation::absent_because(fact_reason::NOT_ON_A_WATCHLIST),
                Some(first) => Observation::known((today - first.date_naive()).num_days().max(0)),
            };
        }
    }

    // ── list exclusions and the requester's library footprint ──
    facts.list_excluded = match context.list_excluded {
        None => Observation::unknown(fact_reason::LIST_EXCLUSIONS_UNAVAILABLE),
        Some(excluded) => Observation::known(excluded),
    };
    facts.requester_titles_in_library_count = match context.requester_titles_in_library_count {
        None => Observation::unknown(fact_reason::REQUESTER_TITLES_UNAVAILABLE),
        Some(count) => Observation::known(count),
    };

    // ── storage ──
    facts.storage_root_id = match &context.storage.root_id {
        None => Observation::unknown(fact_reason::STORAGE_ROOT_UNAVAILABLE),
        Some(root_id) => Observation::known(root_id.clone()),
    };
    match context.storage.capacity {
        Err(reason) => {
            facts.storage_available_bytes = Observation::unknown(reason);
            facts.storage_total_bytes = Observation::unknown(reason);
            facts.storage_available_percent = Observation::unknown(reason);
        }
        Ok(figures) => {
            let sampled_at = figures.sampled_at.to_rfc3339();
            facts.storage_available_bytes =
                Observation::known_at(figures.available_bytes, sampled_at.clone());
            facts.storage_total_bytes =
                Observation::known_at(figures.total_bytes, sampled_at.clone());
            facts.storage_available_percent =
                Observation::known_at(figures.available_percent, sampled_at);
        }
    }

    facts
}

/// A fact document whose every field is unknown, for [`series_shape_facts`] to
/// fill; [`build_facts`] overrides every field it owns.
fn empty_facts() -> RequestFactsDoc {
    let unset = || fact_reason::NOT_YET_COLLECTED;
    macro_rules! unset_facts {
        ($($field:ident),* $(,)?) => {
            RequestFactsDoc { $($field: Observation::unknown(unset()),)* }
        };
    }
    unset_facts!(
        age_rating,
        certifications,
        certification_label,
        certification_rank,
        commonsense_recommended,
        genres,
        canonical_tag_keys,
        themes,
        is_adult,
        rating,
        ratings_by_source,
        tmdb_vote_average,
        tmdb_vote_count,
        popularity,
        runtime_minutes,
        original_language,
        country,
        network,
        studio,
        content_status,
        release_date,
        first_aired,
        release_age_days,
        award_count,
        awards,
        quality_profile_tiers,
        quality_profile_max_resolution,
        quality_profile_allows_upgrades,
        exists_in_library_ids,
        previous_request_count,
        previously_denied,
        previously_approved,
        pending_request_count,
        approved_last_30d,
        denied_last_30d,
        total_approved,
        active_lease_count,
        days_since_last_request,
        library_title_count,
        season_count,
        episode_count,
        aired_episode_count,
        unaired_episode_count,
        next_episode_air_date,
        theatrical_release_date,
        digital_release_date,
        physical_release_date,
        home_release_available,
        days_until_release,
        other_requester_count,
        pending_by_other_requester,
        on_requester_watchlist,
        requester_watchlist_days,
        list_excluded,
        requester_titles_in_library_count,
        storage_root_id,
        storage_available_bytes,
        storage_total_bytes,
        storage_available_percent,
    )
}

impl RequestInputContext {
    fn request_facet_is_movie(&self) -> bool {
        self.library.facet == MediaFacet::Movie.as_str()
    }
}

/// A fact derived from one snapshot group: unknown when the group is missing,
/// otherwise whatever the closure decided (`Ok` known, `Err(reason)` absent).
fn snapshot_fact<T, F>(
    snapshot: &MediaRequestMetadataSnapshot,
    group: &str,
    resolve: F,
) -> Observation<T>
where
    T: serde::Serialize,
    F: FnOnce() -> Result<T, &'static str>,
{
    if snapshot.is_missing(group) {
        return Observation::unknown(REASON_METADATA_UNAVAILABLE);
    }
    match resolve() {
        Ok(value) => Observation::known(value),
        Err(reason) => Observation::absent_because(reason),
    }
}

/// The scalar case of [`snapshot_fact`], where the group is already resolved to
/// a boolean and the value is an `Option`.
fn optional_snapshot_fact<T: serde::Serialize>(
    group_known: bool,
    value: Option<T>,
    absent_reason: &'static str,
) -> Observation<T> {
    if !group_known {
        return Observation::unknown(REASON_METADATA_UNAVAILABLE);
    }
    match value {
        Some(value) => Observation::known(value),
        None => Observation::absent_because(absent_reason),
    }
}

fn catalog_fact<T: serde::Serialize, F: FnOnce() -> T>(
    catalog: &RequestCatalogContext,
    resolve: F,
) -> Observation<T> {
    if catalog.readable {
        Observation::known(resolve())
    } else {
        Observation::unknown("catalog_unavailable")
    }
}

fn history_fact<T: serde::Serialize, F: FnOnce() -> T>(
    history: &RequestRequesterHistoryContext,
    resolve: F,
) -> Observation<T> {
    if history.readable {
        Observation::known(resolve())
    } else {
        Observation::unknown("request_history_unavailable")
    }
}

/// The US row of a list of content ratings, if there is one.
fn us_content_rating_in(
    content_ratings: &[crate::types::ContentRating],
) -> Option<&crate::types::ContentRating> {
    content_ratings.iter().find(|rating| {
        US_COUNTRY_CODES
            .iter()
            .any(|code| rating.country.trim().eq_ignore_ascii_case(code))
    })
}

/// The minimum age a snapshot's content ratings imply.
///
/// Content ratings first, MDBList second: the certification body's own minimum
/// age is the authoritative one, and MDBList's is a derived aggregate that only
/// helps when nobody published a rating.
///
/// Public because the GraphQL read model shows an approver the same number the
/// `age_rating` fact reads, and deriving it twice would let the two drift.
pub fn snapshot_age_rating(snapshot: &MediaRequestMetadataSnapshot) -> Option<i64> {
    content_ratings_age_rating(&snapshot.content_ratings)
        .or_else(|| {
            if snapshot.is_missing(GROUP_MDBLIST) {
                None
            } else {
                snapshot
                    .mdblist
                    .as_ref()
                    .and_then(|mdblist| mdblist.age_rating)
            }
        })
        .map(i64::from)
}

/// The minimum age a list of content ratings implies: the US row's, else the
/// first row that gives one. Lifecycle rules read a library title's stored
/// ratings through it, so both rule kinds read the same age from the same
/// ratings.
pub fn content_ratings_age_rating(content_ratings: &[crate::types::ContentRating]) -> Option<i32> {
    us_content_rating_in(content_ratings)
        .and_then(|rating| rating.age_rating)
        .or_else(|| content_ratings.iter().find_map(|rating| rating.age_rating))
}

/// Every certification of a list of content ratings, flattened in order, as
/// both rule kinds read them.
pub fn certification_docs(
    content_ratings: &[crate::types::ContentRating],
) -> Vec<RequestCertificationDoc> {
    content_ratings
        .iter()
        .flat_map(|rating| {
            rating
                .certifications
                .iter()
                .map(|certification| RequestCertificationDoc {
                    country: rating.country.clone(),
                    value: certification.value.clone(),
                    source: certification.source.clone(),
                })
        })
        .collect()
}

/// A title's awards as both rule kinds read them.
pub fn award_docs(awards: &[crate::types::TitleAward]) -> Vec<RequestAwardDoc> {
    awards
        .iter()
        .map(|award| RequestAwardDoc {
            award: award.award_label.clone(),
            award_id: award.award_qid.clone(),
            year: award.year.map(i64::from),
            recipient: award.recipient_label.clone(),
            recipient_id: award.recipient_qid.clone(),
            given_to: award.claim_side.clone(),
        })
        .collect()
}

/// The US certification value, if there is one. Public for the same reason
/// [`snapshot_age_rating`] is.
pub fn us_certification_label(snapshot: &MediaRequestMetadataSnapshot) -> Option<String> {
    content_ratings_us_certification_label(&snapshot.content_ratings)
}

/// The US certification value of a list of content ratings, if there is one.
///
/// The one place the label is chosen: the `certification_label` fact reads a
/// request snapshot through it, and a library title's badge reads the title's
/// stored ratings through it, so the two always show the same value.
pub fn content_ratings_us_certification_label(
    content_ratings: &[crate::types::ContentRating],
) -> Option<String> {
    us_content_rating_in(content_ratings).and_then(|rating| {
        rating
            .certifications
            .iter()
            .map(|certification| certification.value.trim())
            .find(|value| !value.is_empty())
            .map(str::to_string)
    })
}

/// Accepts what the metadata sources actually publish: an RFC3339 timestamp or
/// a bare `YYYY-MM-DD`.
fn parse_release_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date);
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc).date_naive())
}

// ── The async half ──────────────────────────────────────────────────────────

impl AppUseCase {
    /// Perform every read one evaluation needs, exactly once each.
    ///
    /// Most fact-source failures degrade that group to unknown, which holds
    /// the rules that read it and lands the request in manual review. External
    /// account links are the exception: their document shape has no unknown
    /// state, so a failed read must reach the evaluation fallback rather than
    /// fabricate an empty provider list.
    ///
    /// Without `read_watchlist` the requester's lists are not read and the
    /// watchlist facts read unknown as not yet collected: for an evaluation
    /// whose rules cannot read them.
    pub(crate) async fn assemble_request_input_context(
        &self,
        actor: &User,
        library: &Library,
        draft: &crate::request_rules::facts::RequestDraft,
        snapshot: MediaRequestMetadataSnapshot,
        evaluation_time: DateTime<Utc>,
        excluding_request_id: Option<&str>,
        read_watchlist: bool,
    ) -> AppResult<RequestInputContext> {
        let requester = self
            .request_requester_doc(actor, &library.id, evaluation_time)
            .await?;
        let list = self.request_list_doc(actor, draft).await;
        let request = request_doc(draft, list);
        let quality = self.request_quality_context(draft).await;
        let catalog = self
            .request_catalog_context(actor, library, draft, excluding_request_id)
            .await;
        let history = self
            .request_history_context(actor, evaluation_time, excluding_request_id)
            .await;
        let library_title_count = self.request_library_title_count(&library.id).await;
        let watchlist = if read_watchlist {
            self.request_watchlist_context(actor, draft, evaluation_time)
                .await
        } else {
            RequestWatchlistContext::Unavailable(fact_reason::NOT_YET_COLLECTED)
        };
        let list_excluded = self.request_list_excluded(draft).await;
        let requester_titles_in_library_count = self
            .request_requester_titles_in_library_count(actor, &library.id)
            .await;
        let storage = self.request_storage_context(library, evaluation_time);

        Ok(RequestInputContext {
            evaluation_time,
            requester,
            library: RequestLibraryDoc {
                id: library.id.clone(),
                name: library.name.clone(),
                facet: library.facet.as_str().to_string(),
                is_default: library.is_default,
            },
            request,
            snapshot,
            quality,
            catalog,
            history,
            library_title_count,
            watchlist,
            list_excluded,
            requester_titles_in_library_count,
            storage,
        })
    }

    /// The requester's own watch-later lists: one read of their enabled
    /// personal subscriptions, then, when any is a current watch-later list,
    /// one read of those lists' active rows. Nobody else's lists are read and
    /// no list is synced or previewed.
    async fn request_watchlist_context(
        &self,
        actor: &User,
        draft: &RequestDraft,
        evaluation_time: DateTime<Utc>,
    ) -> RequestWatchlistContext {
        let lists = &self.services.lists;
        let subscriptions = match lists
            .subscriptions
            .list(crate::lists::ListSubscriptionQuery {
                enabled: Some(true),
                ..crate::lists::ListSubscriptionQuery::personal_for(actor.id.clone())
            })
            .await
        {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                tracing::warn!(error = %error, "could not read the requester's lists; watchlist facts are unknown");
                return RequestWatchlistContext::Unavailable(fact_reason::LIST_STORE_UNREADABLE);
            }
        };
        let watchlists: Vec<_> = subscriptions
            .into_iter()
            .filter(|subscription| {
                subscription.owner_user_id == actor.id
                    && subscription.scope == ListScope::Personal
                    && subscription.enabled
                    && subscription.sync.state != scryer_domain::ListSyncState::Off
                    && subscription.credential_id.is_some()
                    && crate::discovery::discovery_watchlist_source_matches(subscription)
            })
            .collect();
        if watchlists.is_empty() {
            return RequestWatchlistContext::NotLinked;
        }
        for watchlist in &watchlists {
            let Some(last_at) = watchlist.sync.last_at else {
                return RequestWatchlistContext::Unavailable(fact_reason::WATCHLIST_NOT_SYNCED);
            };
            if matches!(
                watchlist.sync.state,
                scryer_domain::ListSyncState::New | scryer_domain::ListSyncState::Fail
            ) {
                return RequestWatchlistContext::Unavailable(fact_reason::WATCHLIST_NOT_SYNCED);
            }
            if evaluation_time - last_at > watchlist_freshness_window(watchlist.interval_seconds) {
                return RequestWatchlistContext::Unavailable(fact_reason::WATCHLIST_STALE);
            }
        }
        let ids: Vec<String> = watchlists
            .iter()
            .map(|subscription| subscription.id.clone())
            .collect();
        let rows = match lists
            .memberships
            .list_active_identities_by_subscriptions(&ids)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "could not read the requester's watchlist rows; watchlist facts are unknown");
                return RequestWatchlistContext::Unavailable(fact_reason::LIST_STORE_UNREADABLE);
            }
        };
        let draft_is_movie = draft.facet == MediaFacet::Movie;
        let first_seen_at = rows
            .iter()
            .filter(|row| (row.kind == MediaFacet::Movie) == draft_is_movie)
            .filter(|row| {
                row.external_ids.iter().any(|listed| {
                    draft
                        .external_ids
                        .iter()
                        .any(|wanted| listed.same_entity_as(wanted))
                })
            })
            .map(|row| row.first_seen_at)
            .min();
        RequestWatchlistContext::Known { first_seen_at }
    }

    /// Whether a list exclusion covers this title, for the draft's own list
    /// when it came from one: one indexed existence check.
    async fn request_list_excluded(&self, draft: &RequestDraft) -> Option<bool> {
        match self
            .services
            .lists
            .exclusions
            .any_matching(
                draft.facet.clone(),
                &draft.external_ids,
                draft.origin.subscription_id(),
            )
            .await
        {
            Ok(excluded) => Some(excluded),
            Err(error) => {
                tracing::warn!(error = %error, "could not read list exclusions; the fact is unknown");
                None
            }
        }
    }

    async fn request_requester_titles_in_library_count(
        &self,
        actor: &User,
        library_id: &str,
    ) -> Option<i64> {
        match self
            .services
            .catalog
            .media_requests
            .count_created_titles_in_library_for_requester(&actor.id, library_id)
            .await
        {
            Ok(count) => Some(i64::try_from(count).unwrap_or(i64::MAX)),
            Err(error) => {
                tracing::warn!(
                    library_id,
                    error = %error,
                    "could not count the requester's titles in the library; the fact is unknown"
                );
                None
            }
        }
    }

    /// The target root's capacity from the capacity sampler's cache. Never
    /// calls the filesystem. A library with several roots reports the one a
    /// new title lands on: the default root, else the first configured.
    fn request_storage_context(
        &self,
        library: &Library,
        evaluation_time: DateTime<Utc>,
    ) -> RequestStorageContext {
        let Some(root) = library
            .roots
            .iter()
            .find(|root| root.is_default && !root.path.trim().is_empty())
            .or_else(|| {
                library
                    .roots
                    .iter()
                    .find(|root| !root.path.trim().is_empty())
            })
        else {
            return RequestStorageContext::no_root();
        };
        let reading = self
            .runtime
            .health
            .storage_capacity
            .reading(root.path.trim(), evaluation_time);
        RequestStorageContext {
            root_id: Some(root.id.clone()),
            capacity: storage_figures(reading),
        }
    }

    /// The list section for a list-sourced draft: one read of the
    /// subscription and one of its owner's list policy. A failed read leaves
    /// the section absent so the request still evaluates.
    async fn request_list_doc(&self, actor: &User, draft: &RequestDraft) -> Option<RequestListDoc> {
        let subscription_id = draft.origin.subscription_id()?;
        let lists = &self.services.lists;
        let read = async {
            let Some(subscription) = lists.subscriptions.get_by_id(subscription_id).await? else {
                return Ok(None);
            };
            let policy = lists.policies.get(&subscription.owner_user_id).await?;
            AppResult::Ok(Some((subscription, policy)))
        };
        let (subscription, policy) = match read.await {
            Ok(found) => found?,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read the request's list; its list section is absent"
                );
                return None;
            }
        };
        let is_watchlist = subscription.scope == ListScope::Personal
            && crate::discovery::discovery_watchlist_source_matches(&subscription);
        let source_type = subscription.source.source_type.trim().to_ascii_lowercase();
        Some(RequestListDoc {
            subscription_id: subscription.id,
            name: (subscription.scope == ListScope::Public).then_some(subscription.name),
            scope: subscription.scope.as_str().to_string(),
            mode: subscription.mode.as_str().to_string(),
            provider: subscription.source.provider,
            owner_is_requester: subscription.owner_user_id == actor.id,
            owner_list_policy: policy
                .map(|policy| policy.policy)
                .unwrap_or_default()
                .as_str()
                .to_string(),
            rank: draft.list_rank,
            is_watchlist,
            source_type,
        })
    }

    /// The requester document: identity, the permissions they actually hold,
    /// and their verified external links.
    ///
    /// Permissions are emitted as the same `as_str()` names the rest of Scryer
    /// uses, so a rule saying `"manage_titles" in input.requester.library_permissions`
    /// reads the vocabulary an operator already knows from the permissions
    /// screen.
    async fn request_requester_doc(
        &self,
        actor: &User,
        library_id: &str,
        evaluation_time: DateTime<Utc>,
    ) -> AppResult<RequestRequesterDoc> {
        let mut app_permissions = Vec::new();
        for &permission in AppPermission::ALL {
            if self.has_app_permission(actor, permission).await? {
                app_permissions.push(permission.as_str().to_string());
            }
        }

        let mut library_permissions = Vec::new();
        for permission in [
            LibraryPermission::View,
            LibraryPermission::ManageTitles,
            LibraryPermission::ResolveImports,
            LibraryPermission::ManageLibrary,
            LibraryPermission::Request,
            LibraryPermission::AutoApproveRequests,
            LibraryPermission::ManageSubtitles,
        ] {
            if self
                .has_library_permission(actor, library_id, permission)
                .await?
            {
                library_permissions.push(permission.as_str().to_string());
            }
        }

        // Verified links only: a pending claim is an assertion nobody has
        // confirmed, and a rule that trusted it would let anyone grant
        // themselves a provider identity by typing a username.
        // One primary-key read. A failure, or a row written before account
        // creation times were stored, leaves the three fields absent.
        let timestamps = match self
            .services
            .identity
            .users
            .account_timestamps(&actor.id)
            .await
        {
            Ok(timestamps) => timestamps.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(error = %error, "could not read the requester's account times; they are absent");
                crate::UserAccountTimestamps::default()
            }
        };

        let linked_providers = self
            .services
            .identity
            .external_accounts
            .list_by_user_id(&actor.id)
            .await?
            .into_iter()
            .filter(|account| {
                account.status == ExternalAccountStatus::Active && account.verified_at.is_some()
            })
            .map(|account| account.provider.as_str().to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        Ok(RequestRequesterDoc {
            user_id: actor.id.clone(),
            username: actor.username.clone(),
            account_kind: match actor.account_kind {
                UserAccountKind::Local => "local".to_string(),
                UserAccountKind::ExternalAutoProvisioned => "external_auto_provisioned".to_string(),
            },
            app_permissions,
            library_permissions,
            linked_providers,
            created_at: timestamps
                .created_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            last_login_at: timestamps
                .last_login_at
                .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            account_age_days: timestamps.created_at.map(|created| {
                (evaluation_time.date_naive() - created.date_naive())
                    .num_days()
                    .max(0)
            }),
        })
    }

    async fn request_quality_context(
        &self,
        draft: &crate::request_rules::facts::RequestDraft,
    ) -> Option<RequestQualityContext> {
        let profile_id = draft.quality_profile_id.as_deref()?;
        let settings = match self.load_quality_profile_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read quality profiles; request rules see unknown quality facts"
                );
                return None;
            }
        };
        let profile =
            crate::settings::runtime::quality_profile_by_id(&settings.profiles, profile_id)
                .ok()
                .flatten()?;
        Some(RequestQualityContext {
            tiers: profile.criteria.quality_tiers.clone(),
            allow_upgrades: profile.criteria.allow_upgrades,
        })
    }

    async fn request_catalog_context(
        &self,
        actor: &User,
        library: &Library,
        draft: &crate::request_rules::facts::RequestDraft,
        excluding_request_id: Option<&str>,
    ) -> RequestCatalogContext {
        let mut context = RequestCatalogContext {
            readable: true,
            ..RequestCatalogContext::default()
        };

        let lookups = draft
            .external_ids
            .iter()
            .enumerate()
            .map(|(lookup_index, external_id)| TitleExternalIdLookup {
                lookup_index,
                source: external_id.source.clone(),
                external_id: external_id.value.clone(),
            })
            .collect::<Vec<_>>();
        let matches = match self
            .services
            .catalog
            .titles
            .list_by_external_id_lookups(&lookups)
            .await
        {
            Ok(matches) => matches,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read the catalog for a request rule fact"
                );
                context.readable = false;
                return context;
            }
        };
        let mut library_ids: Vec<String> = Vec::new();
        for matched in matches {
            let title = matched.title;
            // The request's own library is excluded: submit already refuses a
            // title that is there, so including it would make the fact read as
            // "already elsewhere" for a request that is not.
            if title.facet != draft.facet || title.library_id == library.id {
                continue;
            }
            if !library_ids.contains(&title.library_id) {
                library_ids.push(title.library_id);
            }
        }
        context.exists_in_library_ids = library_ids;

        // One aggregate statement: counts and flags, no request rows decoded.
        match self
            .services
            .catalog
            .media_requests
            .fingerprint_history_summary(
                &draft.identity_fingerprint,
                &actor.id,
                excluding_request_id,
            )
            .await
        {
            Ok(summary) => {
                context.previous_request_count =
                    i64::try_from(summary.request_count).unwrap_or(i64::MAX);
                context.previously_denied = summary.any_rejected;
                context.previously_approved = summary.any_approved;
                context.other_requester_count =
                    i64::try_from(summary.other_requester_count).unwrap_or(i64::MAX);
                context.pending_by_other_requester = summary.pending_by_other_requester;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read request history for a request rule fact"
                );
                context.readable = false;
            }
        }

        context
    }

    async fn request_history_context(
        &self,
        actor: &User,
        evaluation_time: DateTime<Utc>,
        excluding_request_id: Option<&str>,
    ) -> RequestRequesterHistoryContext {
        use scryer_domain::MediaRequestStatus;

        let thirty_days_ago = evaluation_time - chrono::Duration::days(30);
        let requests = &self.services.catalog.media_requests;
        let mut context = RequestRequesterHistoryContext {
            readable: true,
            ..RequestRequesterHistoryContext::default()
        };

        let counters = async {
            Ok::<_, crate::AppError>((
                requests
                    .count_for_requester(
                        &actor.id,
                        Some(MediaRequestStatus::Pending),
                        None,
                        excluding_request_id,
                    )
                    .await?,
                requests
                    .count_for_requester(
                        &actor.id,
                        Some(MediaRequestStatus::Approved),
                        Some(thirty_days_ago),
                        excluding_request_id,
                    )
                    .await?,
                requests
                    .count_for_requester(
                        &actor.id,
                        Some(MediaRequestStatus::Rejected),
                        Some(thirty_days_ago),
                        excluding_request_id,
                    )
                    .await?,
                requests
                    .count_for_requester(
                        &actor.id,
                        Some(MediaRequestStatus::Approved),
                        None,
                        excluding_request_id,
                    )
                    .await?,
                requests
                    .latest_request_at_for_user(&actor.id, excluding_request_id)
                    .await?,
            ))
        }
        .await;

        match counters {
            Ok((pending, approved_30d, denied_30d, total_approved, last_request_at)) => {
                context.pending_request_count = pending as i64;
                context.approved_last_30d = approved_30d as i64;
                context.denied_last_30d = denied_30d as i64;
                context.total_approved = total_approved as i64;
                context.last_request_at = last_request_at;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read requester history for request rule facts"
                );
                context.readable = false;
            }
        }

        // Counted separately because it comes from a different store: an
        // unreadable claim store must leave *this* fact unknown without also
        // blanking the request counters, which were read successfully.
        context.active_lease_count = match self
            .services
            .catalog
            .lifecycle_claims
            .count_live_for_user(&actor.id)
            .await
        {
            Ok(count) => Some(count as i64),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not count live lifecycle claims; the lease fact is unknown"
                );
                None
            }
        };

        context
    }

    async fn request_library_title_count(&self, library_id: &str) -> Option<i64> {
        match self
            .services
            .catalog
            .titles
            .count_titles_in_library(library_id)
            .await
        {
            Ok(count) => Some(count as i64),
            Err(error) => {
                tracing::warn!(
                    library_id,
                    error = %error,
                    "could not count titles in the library; the fact is unknown"
                );
                None
            }
        }
    }
}

fn storage_figures(
    reading: crate::disk::capacity::StorageCapacityReading,
) -> Result<RequestStorageFigures, &'static str> {
    match reading {
        crate::disk::capacity::StorageCapacityReading::Known {
            sampled_at,
            available_bytes,
            total_bytes,
        } => Ok(RequestStorageFigures {
            sampled_at,
            available_bytes,
            total_bytes,
            available_percent: reading.available_percent().unwrap_or_default(),
        }),
        other => Err(other
            .unknown_reason()
            .unwrap_or(fact_reason::STORAGE_CAPACITY_UNAVAILABLE)),
    }
}

/// The draft half of the input document. Always known: it is what the requester
/// is looking at.
fn request_doc(draft: &RequestDraft, list: Option<RequestListDoc>) -> RequestDoc {
    RequestDoc {
        origin: draft.origin.as_str().to_string(),
        title: draft.title.clone(),
        year: draft.year,
        external_ids: draft
            .external_ids
            .iter()
            .map(|external_id| {
                (
                    external_id.source.trim().to_ascii_lowercase(),
                    external_id.value.trim().to_string(),
                )
            })
            .collect(),
        quality_profile_id: draft.quality_profile_id.clone(),
        quality_profile_name: draft.quality_profile_name.clone(),
        monitor_type: draft.monitor_type.clone(),
        monitor_selection_season_count: draft
            .monitor_selection
            .as_ref()
            .map(|selection| selection.seasons.len() as i64),
        monitor_selection_seasons: draft.monitor_selection.as_ref().map(|selection| {
            selection
                .seasons
                .iter()
                .map(|season| i64::from(*season))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        }),
        lease_forever: draft.requested_lease_days.is_none(),
        lease_days: draft.requested_lease_days,
        required_audio_languages: draft.requested_required_audio_languages.clone(),
        preferred_audio_languages: draft.requested_preferred_audio_languages.clone(),
        list,
    }
}
