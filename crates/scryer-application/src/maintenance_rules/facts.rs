//! Fact snapshots for maintenance rule evaluation (RFC 137 section 8).
//!
//! Every fact the matcher can read is built here, from ports Scryer already
//! has. A fact Scryer cannot resolve becomes [`Observation::unknown`] with a
//! stable code rather than a plausible-looking default: the engine holds any
//! rule that reads an unknown fact, which is the fail-closed behaviour the RFC
//! demands. Guessing `false`, `0`, or "the newest file" would silently
//! authorize a destructive action on evidence Scryer never had.
//!
//! The distinction between `absent` and `unknown` is what drives that, so it is
//! decided here and nowhere else. `absent` is an answer — the source replied
//! and there is nothing there — and rules see it as a missing key they may act
//! on. `unknown` is a gap, and rules never see it at all on `input.facts`;
//! they get held instead.

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Utc};
use scryer_domain::{MediaFacet, Title, UserMediaSignal};
use scryer_rules::maintenance::{
    MAINTENANCE_INPUT_SCHEMA_VERSION, MaintenanceFactsDoc, MaintenanceFileDoc, MaintenanceInput,
    MaintenanceLibraryDoc, MaintenanceSeriesMovieDoc, MaintenanceSubjectDoc,
    MaintenanceSubjectKind, Observation,
};

use crate::types::TitleMediaFile;

/// Structured tag prefix carrying a title's quality profile. Kept in sync with
/// [`crate::ports::TitleRepository::count_by_quality_profile_id`], which
/// defines the trim-after-strip resolver semantics reused here.
pub(crate) const QUALITY_PROFILE_TAG_PREFIX: &str = "scryer:quality-profile:";

/// Stable reason codes for facts this wave cannot observe.
///
/// These are part of the rule-authoring contract twice over: they are the
/// reason codes recorded on a candidate the engine held, and a matcher reading
/// `input.observations.<fact>.reason` compares against exactly these strings.
pub mod unknown_reason {
    /// Scryer does not record this signal yet. Applies to `last_upgraded_at`
    /// (no distinct upgrade event exists — the newest file timestamp is a
    /// different fact and must not stand in for it), `active_downloads`
    /// (download-state wiring is out of scope for the foundation wave), and the
    /// three episode counts for series-facet titles (only per-title episode
    /// queries exist, so a preview over 50 titles would fan out).
    pub const NOT_YET_COLLECTED: &str = "not_yet_collected";

    /// The title has files, but none of their rows carry a parseable
    /// timestamp, so the first-import instant is genuinely unavailable.
    pub const FILE_TIMESTAMPS_UNAVAILABLE: &str = "file_timestamps_unavailable";

    /// The title carries no `created_by`, which is what a scan- or
    /// discovery-created title looks like: nobody added it through the add
    /// flow. This is an *absence* reason, not an unknown one — the source
    /// answered, and the answer is that there is no adding user.
    pub const TITLE_ADDED_BY_SYSTEM: &str = "title_added_by_system";

    /// A user id Scryer holds no longer resolves to a user row. The id itself
    /// stays known; only the name is unavailable, so a rule matching on names
    /// is held rather than told the wrong one.
    pub const USER_NOT_FOUND: &str = "user_not_found";

    /// Watch signals are recorded per movie and per episode, never per show
    /// (RFC 137 "Normalized observation model"). A series- or anime-facet title
    /// therefore has no watch answer at all in this MVP, and a silent `false`
    /// would let "delete what nobody watched" fire on every show in the
    /// library.
    pub const SHOW_WATCH_ROLLUP_UNAVAILABLE: &str = "show_watch_rollup_unavailable";

    /// No enabled media-server connection of a signal-sync provider exists, so
    /// nothing could have reported a play. Distinct from "nobody watched it".
    pub const NO_MEDIA_SERVER_CONNECTION: &str = "no_media_server_connection";

    /// An enabled signal-sync connection has never completed a clean sweep, so
    /// its part of the watch picture has never been read at all.
    pub const SIGNAL_SYNC_NEVER_SUCCEEDED: &str = "signal_sync_never_succeeded";

    /// An enabled signal-sync connection last swept cleanly longer ago than
    /// [`super::WATCH_SIGNAL_FRESHNESS_HOURS`], so what it reported may no
    /// longer be true.
    pub const SIGNALS_STALE: &str = "signals_stale";

    /// A played row carries no Scryer user id. Participants are verified links,
    /// so this should not occur — but an unattributable watcher must fail
    /// closed rather than silently vanish from the watcher set.
    pub const SIGNAL_IDENTITY_MISSING: &str = "signal_identity_missing";

    /// The gate passed and no play was ever recorded for the subject. An
    /// *absence* reason: the signal store answered, and the answer is nothing.
    pub const NEVER_WATCHED: &str = "never_watched";

    /// The subject has no requesters, so "did the requesters watch it" has no
    /// subject set to be true or false over. An absence reason, not an unknown:
    /// Scryer looked and there is nobody to ask about.
    pub const TITLE_NOT_REQUESTED: &str = "title_not_requested";

    /// A requester holds no verified linked account on any enabled signal-sync
    /// connection. Their watch state is unknowable, so neither requester
    /// rollup can be honest about them.
    pub const REQUESTER_NOT_LINKED: &str = "requester_not_linked";

    /// Every enabled signal-sync connection swept cleanly, but not one of them
    /// carries a verified linked account — so nobody in the instance is
    /// observable at all. A clean sweep over an empty roster reports no plays
    /// for every subject, which is "Scryer can see nobody", not "nobody watched
    /// it", and the difference is what a "delete what nobody watched" rule
    /// deletes on.
    pub const NO_LINKED_PARTICIPANTS: &str = "no_linked_participants";
}

/// How recently every enabled signal-sync connection must have swept cleanly
/// before watch facts may be reported at all.
///
/// Watch facts are what a "delete what nobody watched" rule deletes on, so the
/// window is a freshness *floor*, not a cache hint: past it, Scryer says it
/// does not know rather than reporting a play count that may be two days out of
/// date. The signal sync job runs every six hours, so 48 hours tolerates seven
/// consecutive missed sweeps before rules stop deciding.
pub const WATCH_SIGNAL_FRESHNESS_HOURS: i64 = 48;

/// Whether media-server watch signals may be reported for this run at all.
///
/// Deliberately whole-instance and all-or-nothing: one stale or never-swept
/// connection poisons the gate for every subject, because a partial watch
/// picture makes "nobody watched this" a lie rather than an approximation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchSignalFreshness {
    /// Every enabled signal-sync connection swept cleanly inside the window.
    Fresh,
    /// Watch facts are unknown, for this stable reason code.
    Unavailable(&'static str),
}

impl Default for WatchSignalFreshness {
    /// An unconfigured context reports the most conservative answer there is:
    /// a caller that forgot to resolve the gate must not get `Fresh`.
    fn default() -> Self {
        Self::Unavailable(unknown_reason::NO_MEDIA_SERVER_CONNECTION)
    }
}

/// Watch-signal state resolved once per evaluation run, shared by every subject
/// in it.
///
/// Both members are run-scoped on purpose. Re-reading the sync states or the
/// participant roster per title would make the job's cost scale with the
/// library, and — worse — would let two subjects in one run disagree about
/// whether the signal picture is fresh.
#[derive(Clone, Debug, Default)]
pub struct MaintenanceWatchContext {
    pub freshness: WatchSignalFreshness,
    /// Every Scryer user with a verified linked account on some enabled
    /// signal-sync connection. A requester outside this set is a participant
    /// Scryer cannot observe.
    pub linked_user_ids: HashSet<String>,
}

impl MaintenanceWatchContext {
    /// The gate answer, or `None` when watch facts may be reported.
    fn unavailable_reason(&self) -> Option<&'static str> {
        match self.freshness {
            WatchSignalFreshness::Fresh => None,
            WatchSignalFreshness::Unavailable(reason) => Some(reason),
        }
    }
}

/// The watch signals for one title, resolved by the caller in batch.
#[derive(Clone, Copy, Debug)]
pub struct MaintenanceTitleWatch<'a> {
    pub context: &'a MaintenanceWatchContext,
    /// This title's movie-level signals. `None` means the store held no rows
    /// for it, which — once the gate has passed — is a confirmed "nobody
    /// watched it", not a gap.
    pub signals: Option<&'a [UserMediaSignal]>,
}

/// Everything the builder needs about one library, resolved once per run.
#[derive(Clone, Debug)]
pub struct MaintenanceLibraryRef {
    pub id: String,
    pub name: String,
}

/// The people signals for one title, resolved by the caller in batch.
///
/// Both fields are prefetched once per evaluation run, exactly like the media
/// files: resolving them per title would make the job's cost scale with the
/// library.
#[derive(Clone, Copy, Debug)]
pub struct MaintenanceTitlePeople<'a> {
    /// Every user linked to this title through a media request — the original
    /// submitter plus any additional requesters — deduped, in a stable order.
    /// `None` means no media request created this title, which is a confirmed
    /// answer and becomes `requested = false`, not an unknown.
    pub requester_user_ids: Option<&'a [String]>,
    /// Username by user id for every user that existed when the run started.
    /// A missing id is a user that no longer exists, not a lookup that failed.
    pub usernames: &'a HashMap<String, String>,
}

/// Build the title-scoped input document for one title.
///
/// `files` must be this title's files only; callers batch-load for the whole
/// selection and group by `title_id` rather than querying per title. The same
/// holds for [`MaintenanceTitlePeople`].
pub fn build_title_input(
    evaluation_time: DateTime<Utc>,
    title: &Title,
    library: &MaintenanceLibraryRef,
    files: &[TitleMediaFile],
    people: MaintenanceTitlePeople<'_>,
    watch: MaintenanceTitleWatch<'_>,
    series_movies: &[MaintenanceSeriesMovieDoc],
) -> MaintenanceInput {
    MaintenanceInput {
        schema_version: MAINTENANCE_INPUT_SCHEMA_VERSION,
        evaluation_time,
        subject: MaintenanceSubjectDoc {
            kind: MaintenanceSubjectKind::Title,
            title_id: title.id.clone(),
            season_number: None,
            episode_id: None,
            facet: title.facet.as_str().to_string(),
            name: title.name.clone(),
            year: title.year,
        },
        library: MaintenanceLibraryDoc {
            id: library.id.clone(),
            name: library.name.clone(),
        },
        facts: build_facts(title, files, people, watch, series_movies),
    }
}

fn build_facts(
    title: &Title,
    files: &[TitleMediaFile],
    people: MaintenanceTitlePeople<'_>,
    watch: MaintenanceTitleWatch<'_>,
    series_movies: &[MaintenanceSeriesMovieDoc],
) -> MaintenanceFactsDoc {
    let file_docs: Vec<MaintenanceFileDoc> = files.iter().map(file_doc).collect();
    let total_size: i64 = files.iter().map(|file| file.size_bytes).sum();
    let (added_by_user_id, added_by_username) =
        added_by_observations(title.created_by.as_deref(), people.usernames);
    let (requested, requested_by_user_ids, requested_by_usernames) =
        requested_observations(people.requester_user_ids, people.usernames);
    let watched = watch_observations(&title.facet, people.requester_user_ids, watch);

    MaintenanceFactsDoc {
        monitored: Observation::known(title.monitored),
        tags: Observation::known(user_title_tags(&title.tags)),
        quality_profile_id: quality_profile_observation(&title.tags),
        added_at: Observation::known(title.created_at.to_rfc3339()),
        added_by_user_id,
        added_by_username,
        requested,
        requested_by_user_ids,
        requested_by_usernames,
        first_imported_at: first_imported_observation(files),
        // Scryer records no distinct upgrade event. Approximating it with the
        // newest file timestamp would make "upgraded" and "imported" the same
        // fact, so rules asking about upgrades are held instead.
        last_upgraded_at: Observation::unknown(unknown_reason::NOT_YET_COLLECTED),
        // An empty file set is a confirmed answer, not a missing one.
        has_file: Observation::known(!files.is_empty()),
        file_count: Observation::known(files.len() as i64),
        total_file_size_bytes: Observation::known(total_size),
        files: Observation::known(file_docs),
        episode_count: episode_count_observation(&title.facet),
        episode_file_count: episode_count_observation(&title.facet),
        monitored_episode_count: episode_count_observation(&title.facet),
        active_downloads: Observation::unknown(unknown_reason::NOT_YET_COLLECTED),
        watched_by_user_ids: watched.watched_by_user_ids,
        last_watched_at: watched.last_watched_at,
        watched_by_any_requester: watched.watched_by_any_requester,
        watched_by_all_requesters: watched.watched_by_all_requesters,
        series_movies: series_movies_observation(&title.facet, series_movies),
    }
}

/// The show's series movies, or a confirmed absence for a movie subject.
///
/// A movie has no series movies the way a movie has no episodes: the question
/// does not apply, the source answered, and the rule sees a missing key rather
/// than being held. A show with no linked movies is a known-empty list, so
/// `count(input.facts.series_movies) == 0` is a decisive answer instead of
/// something the engine holds on.
fn series_movies_observation(
    facet: &MediaFacet,
    series_movies: &[MaintenanceSeriesMovieDoc],
) -> Observation<Vec<MaintenanceSeriesMovieDoc>> {
    match facet {
        MediaFacet::Movie => Observation::absent(),
        MediaFacet::Series | MediaFacet::Anime => Observation::known(series_movies.to_vec()),
    }
}

/// The user-defined half of a title's tag bag.
///
/// `input.facts.tags` is the rule author's tag vocabulary, and that vocabulary
/// is the admin registry. Reserved `scryer:` entries are per-title *settings*
/// stored in the same bag; surfacing them here would let a rule read a quality
/// profile or a monitor type through a fact named "tags", and every one of
/// those settings already has its own fact or none at all on purpose.
fn user_title_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .filter(|tag| !crate::is_reserved_title_tag(tag))
        .cloned()
        .collect()
}

/// Known when the structured tag is present and non-empty; absent when the
/// title carries no such tag, which is a confirmed "no profile assigned"
/// rather than a lookup Scryer failed to perform.
fn quality_profile_observation(tags: &[String]) -> Observation<String> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(QUALITY_PROFILE_TAG_PREFIX))
        .map(str::trim)
        .filter(|profile_id| !profile_id.is_empty())
        .map_or_else(Observation::absent, |profile_id| {
            Observation::known(profile_id.to_string())
        })
}

/// Who added the title, as an id and as a name.
///
/// No `created_by` is a confirmed answer, not a gap: the title arrived from a
/// scan or from discovery rather than through the add flow, so both facts are
/// absent and carry [`unknown_reason::TITLE_ADDED_BY_SYSTEM`]. An id that no
/// longer resolves is the opposite case — the id is still known, but the name
/// is genuinely unavailable, so the name is unknown rather than falling back
/// to the raw id, which a rule comparing usernames would read as a real name.
fn added_by_observations(
    created_by: Option<&str>,
    usernames: &HashMap<String, String>,
) -> (Observation<String>, Observation<String>) {
    let Some(user_id) = created_by.map(str::trim).filter(|id| !id.is_empty()) else {
        return (
            Observation::absent_because(unknown_reason::TITLE_ADDED_BY_SYSTEM),
            Observation::absent_because(unknown_reason::TITLE_ADDED_BY_SYSTEM),
        );
    };

    let username = usernames.get(user_id).map_or_else(
        || Observation::unknown(unknown_reason::USER_NOT_FOUND),
        |username| Observation::known(username.clone()),
    );
    (Observation::known(user_id.to_string()), username)
}

/// Whether a media request created this title, and who asked for it.
///
/// `requested` is never absent: "no media request is linked to this title" is
/// something Scryer looked up and confirmed, so it is a known `false`, and the
/// two lists are then known-empty rather than absent.
///
/// Username resolution is deliberately all-or-nothing: if any requester id
/// fails to resolve, `requested_by_usernames` is unknown instead of a shorter
/// list. A partial list is worse than no list, because the rule cannot tell it
/// apart from a complete one — `not "alice" in input.facts.requested_by_usernames`
/// would silently pass on a run where alice's user row is the one that is
/// missing. Unknown instead means the engine holds the subject. The ids stay
/// known in that case, so a rule that wants to proceed can match on those.
fn requested_observations(
    requester_user_ids: Option<&[String]>,
    usernames: &HashMap<String, String>,
) -> (
    Observation<bool>,
    Observation<Vec<String>>,
    Observation<Vec<String>>,
) {
    let Some(user_ids) = requester_user_ids else {
        return (
            Observation::known(false),
            Observation::known(Vec::new()),
            Observation::known(Vec::new()),
        );
    };

    let resolved: Option<Vec<String>> = user_ids
        .iter()
        .map(|user_id| usernames.get(user_id).cloned())
        .collect();

    (
        Observation::known(true),
        Observation::known(user_ids.to_vec()),
        resolved.map_or_else(
            || Observation::unknown(unknown_reason::USER_NOT_FOUND),
            Observation::known,
        ),
    )
}

/// The four watch facts for one subject.
struct WatchObservations {
    watched_by_user_ids: Observation<Vec<String>>,
    last_watched_at: Observation<String>,
    watched_by_any_requester: Observation<bool>,
    watched_by_all_requesters: Observation<bool>,
}

impl WatchObservations {
    /// Every watch fact unknown for one reason. Used wherever Scryer has no
    /// watch picture at all for the subject, rather than an empty one.
    fn all_unknown(reason: &'static str) -> Self {
        Self {
            watched_by_user_ids: Observation::unknown(reason),
            last_watched_at: Observation::unknown(reason),
            watched_by_any_requester: Observation::unknown(reason),
            watched_by_all_requesters: Observation::unknown(reason),
        }
    }
}

/// Who watched the subject, when it was last watched, and whether its
/// requesters have (RFC 137 section 7.3).
///
/// Three gates run before any signal is read, in this order, because each one
/// makes the next question meaningless rather than merely harder:
///
/// 1. **Facet.** Signals exist per movie and per episode; there is no
///    show-level rollup in this MVP, so a series or anime title has no watch
///    answer at all.
/// 2. **Freshness.** Resolved once per run by the caller. A missing, never-swept,
///    or stale connection means the watch picture is incomplete, and an
///    incomplete picture reported as complete is what turns "nobody watched
///    this" into a deletion of something somebody did watch.
/// 3. **Attribution.** A played row with no Scryer user id is a watcher Scryer
///    cannot name, so the watcher set — and anything computed from it — fails
///    closed rather than quietly omitting them.
///
/// Past those, an empty signal set is an *answer*: a known-empty watcher list
/// and an absent `last_watched_at`, so `not input.facts.last_watched_at` is a
/// decisive "never watched" rather than something the engine holds on.
fn watch_observations(
    facet: &MediaFacet,
    requester_user_ids: Option<&[String]>,
    watch: MaintenanceTitleWatch<'_>,
) -> WatchObservations {
    if !matches!(facet, MediaFacet::Movie) {
        return WatchObservations::all_unknown(unknown_reason::SHOW_WATCH_ROLLUP_UNAVAILABLE);
    }
    if let Some(reason) = watch.context.unavailable_reason() {
        return WatchObservations::all_unknown(reason);
    }

    let played: Vec<&UserMediaSignal> = watch
        .signals
        .unwrap_or_default()
        .iter()
        .filter(|signal| signal.played)
        .collect();

    let mut watchers: BTreeSet<&str> = BTreeSet::new();
    let mut unattributed = false;
    for signal in &played {
        match signal
            .scryer_user_id
            .as_deref()
            .map(str::trim)
            .filter(|user_id| !user_id.is_empty())
        {
            Some(user_id) => {
                watchers.insert(user_id);
            }
            None => unattributed = true,
        }
    }

    let watched_by_user_ids = if unattributed {
        Observation::unknown(unknown_reason::SIGNAL_IDENTITY_MISSING)
    } else {
        Observation::known(watchers.iter().map(|id| (*id).to_string()).collect())
    };

    let last_watched_at = played
        .iter()
        .filter_map(|signal| signal.last_played_at)
        .max()
        .map_or_else(
            || Observation::absent_because(unknown_reason::NEVER_WATCHED),
            |latest| Observation::known(latest.to_rfc3339()),
        );

    let (watched_by_any_requester, watched_by_all_requesters) =
        requester_watch_observations(requester_user_ids, &watchers, unattributed, watch.context);

    WatchObservations {
        watched_by_user_ids,
        last_watched_at,
        watched_by_any_requester,
        watched_by_all_requesters,
    }
}

/// The two requester rollups.
///
/// A subject nobody requested makes both facts *absent* rather than false: with
/// an empty requester set, "all of them watched it" is vacuously true and "any
/// of them watched it" is vacuously false, and a rule that deletes on either
/// would then fire on every unrequested title in the library. Absence says the
/// question does not apply, which a rule can test for with `not`.
///
/// A requester with no verified link on any enabled signal-sync connection is
/// an unknown participant: Scryer never sees their plays, so it cannot honestly
/// answer either rollup and both are held.
fn requester_watch_observations(
    requester_user_ids: Option<&[String]>,
    watchers: &BTreeSet<&str>,
    unattributed: bool,
    context: &MaintenanceWatchContext,
) -> (Observation<bool>, Observation<bool>) {
    let requesters = requester_user_ids.unwrap_or_default();
    if requesters.is_empty() {
        return (
            Observation::absent_because(unknown_reason::TITLE_NOT_REQUESTED),
            Observation::absent_because(unknown_reason::TITLE_NOT_REQUESTED),
        );
    }
    // An unnamed watcher could be any of these requesters, so neither rollup
    // can be computed without possibly attributing a play to the wrong person.
    if unattributed {
        return (
            Observation::unknown(unknown_reason::SIGNAL_IDENTITY_MISSING),
            Observation::unknown(unknown_reason::SIGNAL_IDENTITY_MISSING),
        );
    }
    if requesters
        .iter()
        .any(|requester| !context.linked_user_ids.contains(requester.as_str()))
    {
        return (
            Observation::unknown(unknown_reason::REQUESTER_NOT_LINKED),
            Observation::unknown(unknown_reason::REQUESTER_NOT_LINKED),
        );
    }

    (
        Observation::known(
            requesters
                .iter()
                .any(|requester| watchers.contains(requester.as_str())),
        ),
        Observation::known(
            requesters
                .iter()
                .all(|requester| watchers.contains(requester.as_str())),
        ),
    )
}

/// Earliest file timestamp. Absent when the title has no files at all; unknown
/// only when files exist but carry no readable timestamp.
fn first_imported_observation(files: &[TitleMediaFile]) -> Observation<String> {
    if files.is_empty() {
        return Observation::absent();
    }

    files
        .iter()
        .filter_map(|file| parse_timestamp(&file.created_at))
        .min()
        .map_or_else(
            || Observation::unknown(unknown_reason::FILE_TIMESTAMPS_UNAVAILABLE),
            |earliest| Observation::known(earliest.to_rfc3339()),
        )
}

/// Absent for movies — a movie has no episodes, and that is a fact, not a gap.
/// Unknown for series and anime until an episode-count port exists that a
/// batched preview can afford.
fn episode_count_observation(facet: &MediaFacet) -> Observation<i64> {
    match facet {
        MediaFacet::Movie => Observation::absent(),
        MediaFacet::Series | MediaFacet::Anime => {
            Observation::unknown(unknown_reason::NOT_YET_COLLECTED)
        }
    }
}

/// Fields the media-file row does not carry stay `None`; the file document
/// declares them optional, so a rule reading one gets undefined rather than a
/// fabricated value.
fn file_doc(file: &TitleMediaFile) -> MaintenanceFileDoc {
    MaintenanceFileDoc {
        size_bytes: Some(file.size_bytes),
        quality: file.quality_label.clone(),
        video_codec: file
            .video_codec
            .or(file.video_codec_parsed)
            .map(|codec| codec.as_str().to_string()),
        video_width: file.video_width,
        video_height: file.video_height,
        audio_languages: file.audio_languages.clone(),
        subtitle_languages: file.subtitle_languages.clone(),
        added_at: parse_timestamp(&file.created_at).map(|added| added.to_rfc3339()),
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(created_by: Option<&str>) -> Title {
        faceted_title(created_by, MediaFacet::Movie)
    }

    fn faceted_title(created_by: Option<&str>, facet: MediaFacet) -> Title {
        Title {
            id: "title-1".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            name: "Provenance Fixture".to_string(),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/Movies"),
            created_by: created_by.map(str::to_string),
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
            canonical_tags: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn usernames(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(id, username)| ((*id).to_string(), (*username).to_string()))
            .collect()
    }

    fn library() -> MaintenanceLibraryRef {
        MaintenanceLibraryRef {
            id: "library-1".to_string(),
            name: "Movies".to_string(),
        }
    }

    /// The serialized input document, which is what a rule actually reads:
    /// bare values under `facts`, full envelopes under `observations`.
    fn document(
        title: &Title,
        requester_user_ids: Option<&[String]>,
        usernames: &HashMap<String, String>,
    ) -> serde_json::Value {
        watch_document(
            title,
            requester_user_ids,
            usernames,
            &MaintenanceWatchContext::default(),
            None,
        )
    }

    /// The same document with the watch inputs spelled out, so a test can pin
    /// one gate, one signal set, or one participant roster at a time.
    fn watch_document(
        title: &Title,
        requester_user_ids: Option<&[String]>,
        usernames: &HashMap<String, String>,
        context: &MaintenanceWatchContext,
        signals: Option<&[UserMediaSignal]>,
    ) -> serde_json::Value {
        let input = build_title_input(
            Utc::now(),
            title,
            &library(),
            &[],
            MaintenanceTitlePeople {
                requester_user_ids,
                usernames,
            },
            MaintenanceTitleWatch { context, signals },
            &[],
        );
        serde_json::to_value(input).expect("input serializes")
    }

    const WATCH_FACTS: [&str; 4] = [
        "watched_by_user_ids",
        "last_watched_at",
        "watched_by_any_requester",
        "watched_by_all_requesters",
    ];

    fn fresh_context(linked: &[&str]) -> MaintenanceWatchContext {
        MaintenanceWatchContext {
            freshness: WatchSignalFreshness::Fresh,
            linked_user_ids: linked.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    fn signal(scryer_user_id: Option<&str>, played: bool, watched_at: &str) -> UserMediaSignal {
        UserMediaSignal {
            id: scryer_domain::Id::new().0,
            connection_id: "connection-1".to_string(),
            provider: scryer_domain::MediaServerProvider::Jellyfin,
            external_user_id: "jf-user".to_string(),
            scryer_user_id: scryer_user_id.map(str::to_string),
            provider_item_id: "jf-item".to_string(),
            kind: scryer_domain::MediaServerSignalKind::Movie,
            scryer_title_id: Some("title-1".to_string()),
            scryer_episode_id: None,
            played,
            play_count: i64::from(played),
            last_played_at: Some(
                DateTime::parse_from_rfc3339(watched_at)
                    .expect("fixture timestamp parses")
                    .with_timezone(&Utc),
            ),
            observed_at: Utc::now(),
            sync_generation: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Assert every watch fact is missing from the simple surface and unknown
    /// with `reason` on the envelope one.
    fn assert_watch_facts_unknown(doc: &serde_json::Value, reason: &str) {
        for fact in WATCH_FACTS {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert_eq!(doc["observations"][fact]["status"], "unknown", "{fact}");
            assert_eq!(doc["observations"][fact]["reason"], reason, "{fact}");
        }
    }

    #[test]
    fn a_title_added_through_the_add_flow_reports_who_added_it() {
        let names = usernames(&[("user-1", "operator-one")]);
        let doc = document(&title(Some("user-1")), None, &names);

        assert_eq!(doc["facts"]["added_by_user_id"], "user-1");
        assert_eq!(doc["facts"]["added_by_username"], "operator-one");
        assert_eq!(doc["observations"]["added_by_user_id"]["status"], "known");
        assert_eq!(doc["observations"]["added_by_user_id"]["value"], "user-1");
    }

    #[test]
    fn a_scan_created_title_is_absent_with_the_system_reason_not_unknown() {
        let doc = document(&title(None), None, &usernames(&[]));

        // Absent, not unknown: Scryer looked and confirmed nobody added it, so
        // `not input.facts.added_by_user_id` is a decisive match rather than
        // something the engine holds the title on.
        for fact in ["added_by_user_id", "added_by_username"] {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert_eq!(doc["observations"][fact]["status"], "absent", "{fact}");
            assert_eq!(
                doc["observations"][fact]["reason"], "title_added_by_system",
                "{fact}"
            );
            assert!(doc["observations"][fact].get("value").is_none(), "{fact}");
        }
    }

    #[test]
    fn a_blank_created_by_reads_as_system_added() {
        let doc = document(&title(Some("   ")), None, &usernames(&[]));

        assert!(doc["facts"].get("added_by_user_id").is_none());
        assert_eq!(doc["observations"]["added_by_user_id"]["status"], "absent");
        assert_eq!(
            doc["observations"]["added_by_user_id"]["reason"],
            "title_added_by_system"
        );
    }

    #[test]
    fn an_adding_user_that_no_longer_exists_leaves_the_name_unknown() {
        let doc = document(&title(Some("user-gone")), None, &usernames(&[]));

        // The id is still a fact Scryer holds; only the name is unavailable,
        // so only the name disappears from the simple surface.
        assert_eq!(doc["facts"]["added_by_user_id"], "user-gone");
        assert!(doc["facts"].get("added_by_username").is_none());
        assert_eq!(
            doc["observations"]["added_by_username"]["status"],
            "unknown"
        );
        assert_eq!(
            doc["observations"]["added_by_username"]["reason"],
            "user_not_found"
        );
    }

    #[test]
    fn a_title_no_request_created_is_a_known_false_with_empty_lists() {
        let doc = document(&title(Some("user-1")), None, &usernames(&[]));

        // Known false, not missing: `input.facts.requested` is present and
        // false, which is a different thing from a fact Scryer could not read.
        assert_eq!(doc["facts"]["requested"], false);
        assert_eq!(doc["facts"]["requested_by_user_ids"], serde_json::json!([]));
        assert_eq!(
            doc["facts"]["requested_by_usernames"],
            serde_json::json!([])
        );
        assert_eq!(doc["observations"]["requested"]["status"], "known");
    }

    #[test]
    fn a_requested_title_carries_every_requester_in_the_order_it_was_given() {
        let names = usernames(&[("user-1", "operator-one"), ("user-2", "viewer-two")]);
        let ids = vec!["user-1".to_string(), "user-2".to_string()];
        let doc = document(&title(Some("user-1")), Some(&ids), &names);

        assert_eq!(doc["facts"]["requested"], true);
        assert_eq!(
            doc["facts"]["requested_by_user_ids"],
            serde_json::json!(["user-1", "user-2"])
        );
        assert_eq!(
            doc["facts"]["requested_by_usernames"],
            serde_json::json!(["operator-one", "viewer-two"])
        );
    }

    #[test]
    fn an_empty_requester_list_still_reports_the_title_as_requested() {
        // Key presence is the answer, not list length: a linked request with no
        // resolvable requester rows is still a request.
        let doc = document(&title(None), Some(&[]), &usernames(&[]));

        assert_eq!(doc["facts"]["requested"], true);
        assert_eq!(doc["facts"]["requested_by_user_ids"], serde_json::json!([]));
        assert_eq!(
            doc["facts"]["requested_by_usernames"],
            serde_json::json!([])
        );
    }

    #[test]
    fn one_unresolvable_requester_makes_the_whole_username_list_unknown() {
        let names = usernames(&[("user-1", "operator-one")]);
        let ids = vec!["user-1".to_string(), "user-gone".to_string()];
        let doc = document(&title(Some("user-1")), Some(&ids), &names);

        // A partial list is indistinguishable from a complete one, so it is
        // withheld entirely; the ids stay known for rules that can use them.
        assert!(doc["facts"].get("requested_by_usernames").is_none());
        assert_eq!(
            doc["observations"]["requested_by_usernames"]["status"],
            "unknown"
        );
        assert_eq!(
            doc["observations"]["requested_by_usernames"]["reason"],
            "user_not_found"
        );
        assert_eq!(
            doc["facts"]["requested_by_user_ids"],
            serde_json::json!(["user-1", "user-gone"])
        );
    }

    // ── Watch signals ───────────────────────────────────────────────────────

    #[test]
    fn a_show_has_no_watch_answer_at_all() {
        // No show-level rollup exists, and a silent false here is exactly what
        // would delete a series somebody is halfway through.
        for facet in [MediaFacet::Series, MediaFacet::Anime] {
            let doc = watch_document(
                &faceted_title(Some("user-1"), facet),
                Some(&["user-1".to_string()]),
                &usernames(&[("user-1", "operator-one")]),
                &fresh_context(&["user-1"]),
                Some(&[signal(Some("user-1"), true, "2024-05-01T00:00:00Z")]),
            );

            assert_watch_facts_unknown(&doc, "show_watch_rollup_unavailable");
        }
    }

    #[test]
    fn watch_facts_are_unknown_without_a_media_server_connection() {
        let doc = watch_document(
            &title(Some("user-1")),
            None,
            &usernames(&[]),
            &MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::NO_MEDIA_SERVER_CONNECTION,
                ),
                ..Default::default()
            },
            None,
        );

        assert_watch_facts_unknown(&doc, "no_media_server_connection");
    }

    #[test]
    fn watch_facts_are_unknown_while_a_connection_has_never_swept_cleanly() {
        let doc = watch_document(
            &title(Some("user-1")),
            None,
            &usernames(&[]),
            &MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(
                    unknown_reason::SIGNAL_SYNC_NEVER_SUCCEEDED,
                ),
                ..Default::default()
            },
            None,
        );

        assert_watch_facts_unknown(&doc, "signal_sync_never_succeeded");
    }

    /// Stale signals are held even when rows exist: the rows may simply be old,
    /// and "watched two days ago, according to a sweep two days old" is not
    /// evidence a deletion may run on.
    #[test]
    fn stale_signals_hold_even_though_rows_exist() {
        let doc = watch_document(
            &title(Some("user-1")),
            None,
            &usernames(&[]),
            &MaintenanceWatchContext {
                freshness: WatchSignalFreshness::Unavailable(unknown_reason::SIGNALS_STALE),
                ..Default::default()
            },
            Some(&[signal(Some("user-1"), true, "2024-05-01T00:00:00Z")]),
        );

        assert_watch_facts_unknown(&doc, "signals_stale");
    }

    #[test]
    fn a_title_nobody_watched_reports_an_empty_watcher_list_and_no_watch_time() {
        // Known-empty, not unknown: the gate passed, so "nobody watched it" is
        // an answer a rule may act on.
        let doc = watch_document(
            &title(Some("user-1")),
            None,
            &usernames(&[]),
            &fresh_context(&[]),
            None,
        );

        assert_eq!(
            doc["facts"]["watched_by_user_ids"],
            serde_json::json!([]),
            "an absent signal row set is a confirmed empty watcher list"
        );
        assert!(doc["facts"].get("last_watched_at").is_none());
        assert_eq!(doc["observations"]["last_watched_at"]["status"], "absent");
        assert_eq!(
            doc["observations"]["last_watched_at"]["reason"],
            "never_watched"
        );
    }

    #[test]
    fn watchers_are_deduplicated_sorted_and_dated_by_the_latest_play() {
        let doc = watch_document(
            &title(Some("user-1")),
            None,
            &usernames(&[]),
            &fresh_context(&[]),
            Some(&[
                signal(Some("viewer-two"), true, "2024-05-01T00:00:00Z"),
                signal(Some("operator-one"), true, "2024-06-01T00:00:00Z"),
                signal(Some("viewer-two"), true, "2024-04-01T00:00:00Z"),
            ]),
        );

        assert_eq!(
            doc["facts"]["watched_by_user_ids"],
            serde_json::json!(["operator-one", "viewer-two"])
        );
        assert_eq!(
            doc["facts"]["last_watched_at"], "2024-06-01T00:00:00+00:00",
            "the newest play is the one a rule ages against"
        );
    }

    #[test]
    fn an_unplayed_row_is_not_a_watcher() {
        let doc = watch_document(
            &title(Some("user-1")),
            None,
            &usernames(&[]),
            &fresh_context(&[]),
            Some(&[signal(Some("viewer-two"), false, "2024-05-01T00:00:00Z")]),
        );

        assert_eq!(doc["facts"]["watched_by_user_ids"], serde_json::json!([]));
        assert_eq!(
            doc["observations"]["last_watched_at"]["reason"], "never_watched",
            "a row that was never played carries no watch time"
        );
    }

    /// Participants are verified links, so this should not happen — but a
    /// watcher Scryer cannot name has to fail closed rather than disappear from
    /// the list and make it look shorter than it is.
    #[test]
    fn an_unattributable_play_makes_the_watcher_set_unknown() {
        let doc = watch_document(
            &title(Some("user-1")),
            Some(&["user-1".to_string()]),
            &usernames(&[("user-1", "operator-one")]),
            &fresh_context(&["user-1"]),
            Some(&[
                signal(Some("user-1"), true, "2024-05-01T00:00:00Z"),
                signal(None, true, "2024-05-02T00:00:00Z"),
            ]),
        );

        for fact in [
            "watched_by_user_ids",
            "watched_by_any_requester",
            "watched_by_all_requesters",
        ] {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert_eq!(
                doc["observations"][fact]["reason"], "signal_identity_missing",
                "{fact}"
            );
        }
        // The anonymous aggregate is still answerable: something was played,
        // and when is not a question about who.
        assert_eq!(doc["facts"]["last_watched_at"], "2024-05-02T00:00:00+00:00");
    }

    #[test]
    fn an_unrequested_title_has_no_requester_rollup_to_report() {
        for requesters in [None, Some(&[] as &[String])] {
            let doc = watch_document(
                &title(Some("user-1")),
                requesters,
                &usernames(&[]),
                &fresh_context(&[]),
                Some(&[signal(Some("viewer-two"), true, "2024-05-01T00:00:00Z")]),
            );

            // Absent, not a vacuous true: "all of nobody watched it" would
            // otherwise match every unrequested title in the library.
            for fact in ["watched_by_any_requester", "watched_by_all_requesters"] {
                assert!(doc["facts"].get(fact).is_none(), "{fact}");
                assert_eq!(doc["observations"][fact]["status"], "absent", "{fact}");
                assert_eq!(
                    doc["observations"][fact]["reason"], "title_not_requested",
                    "{fact}"
                );
            }
        }
    }

    #[test]
    fn an_unlinked_requester_holds_both_rollups() {
        let doc = watch_document(
            &title(Some("user-1")),
            Some(&["user-1".to_string(), "user-2".to_string()]),
            &usernames(&[("user-1", "operator-one"), ("user-2", "viewer-two")]),
            // user-2 never linked an account, so Scryer never sees their plays.
            &fresh_context(&["user-1"]),
            Some(&[signal(Some("user-1"), true, "2024-05-01T00:00:00Z")]),
        );

        for fact in ["watched_by_any_requester", "watched_by_all_requesters"] {
            assert!(doc["facts"].get(fact).is_none(), "{fact}");
            assert_eq!(doc["observations"][fact]["status"], "unknown", "{fact}");
            assert_eq!(
                doc["observations"][fact]["reason"], "requester_not_linked",
                "{fact}"
            );
        }
        // The watcher list itself is unaffected: it reports who Scryer saw.
        assert_eq!(
            doc["facts"]["watched_by_user_ids"],
            serde_json::json!(["user-1"])
        );
    }

    #[test]
    fn one_of_two_linked_requesters_watching_is_any_but_not_all() {
        let doc = watch_document(
            &title(Some("user-1")),
            Some(&["user-1".to_string(), "user-2".to_string()]),
            &usernames(&[("user-1", "operator-one"), ("user-2", "viewer-two")]),
            &fresh_context(&["user-1", "user-2"]),
            Some(&[signal(Some("user-1"), true, "2024-05-01T00:00:00Z")]),
        );

        assert_eq!(doc["facts"]["watched_by_any_requester"], true);
        assert_eq!(doc["facts"]["watched_by_all_requesters"], false);
    }

    #[test]
    fn every_linked_requester_watching_satisfies_both_rollups() {
        let doc = watch_document(
            &title(Some("user-1")),
            Some(&["user-1".to_string(), "user-2".to_string()]),
            &usernames(&[("user-1", "operator-one"), ("user-2", "viewer-two")]),
            &fresh_context(&["user-1", "user-2"]),
            Some(&[
                signal(Some("user-1"), true, "2024-05-01T00:00:00Z"),
                signal(Some("user-2"), true, "2024-05-03T00:00:00Z"),
            ]),
        );

        assert_eq!(doc["facts"]["watched_by_any_requester"], true);
        assert_eq!(doc["facts"]["watched_by_all_requesters"], true);
    }

    /// A watcher who is not a requester counts towards nothing on the rollups:
    /// the rollups are about the people who asked for the title.
    #[test]
    fn a_non_requester_watching_does_not_satisfy_the_requester_rollups() {
        let doc = watch_document(
            &title(Some("user-1")),
            Some(&["user-1".to_string()]),
            &usernames(&[("user-1", "operator-one")]),
            &fresh_context(&["user-1"]),
            Some(&[signal(Some("user-9"), true, "2024-05-01T00:00:00Z")]),
        );

        assert_eq!(doc["facts"]["watched_by_any_requester"], false);
        assert_eq!(doc["facts"]["watched_by_all_requesters"], false);
    }
    /// `input.facts.tags` is the rule author's tag vocabulary, and reserved
    /// `scryer:` entries are settings that happen to share the storage. A rule
    /// must never be able to read a quality profile or a monitor type through a
    /// fact named "tags", and the structured entries must not push the user's
    /// own labels around either.
    #[test]
    fn structured_settings_entries_never_appear_in_the_tags_fact() {
        let mut subject = title(Some("user-1"));
        subject.tags = vec![
            "scryer:quality-profile:profile-one".to_string(),
            "keep".to_string(),
            "scryer:monitor-type:all".to_string(),
            "needs review".to_string(),
        ];

        let doc = document(&subject, None, &usernames(&[]));

        assert_eq!(
            doc["facts"]["tags"],
            serde_json::json!(["keep", "needs review"])
        );
        // The settings themselves are still readable through the fact that
        // exists for them, so nothing is lost by filtering the bag.
        assert_eq!(doc["facts"]["quality_profile_id"], "profile-one");
    }

    /// A title carrying settings and no user labels reports an empty list, not
    /// a hold: "this title has no tags" is a confirmed answer.
    #[test]
    fn a_title_with_only_structured_entries_reports_an_empty_tags_fact() {
        let mut subject = title(Some("user-1"));
        subject.tags = vec!["scryer:monitor-specials:false".to_string()];

        let doc = document(&subject, None, &usernames(&[]));

        assert_eq!(doc["facts"]["tags"], serde_json::json!([]));
    }
}
