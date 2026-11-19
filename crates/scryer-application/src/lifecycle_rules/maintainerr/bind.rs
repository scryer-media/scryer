//! Binds Maintainerr properties to Scryer lifecycle facts by application and
//! property name.
//!
//! Each entry also names the Maintainerr rule type it expects, so a property
//! whose type changed between Maintainerr releases is not bound to a fact of
//! another shape. A property with no entry has no faithful Scryer fact.

use super::decode::MediaLevel;

/// The shape of a value as the comparison sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Shape {
    Number,
    Bool,
    /// An RFC 3339 timestamp.
    Instant,
    /// A `YYYY-MM-DD` date, read as midnight UTC.
    Day,
    Text,
    TextList,
}

impl Shape {
    pub(super) fn is_date(self) -> bool {
        matches!(self, Self::Instant | Self::Day)
    }
}

/// Where a bound value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    /// A fact of the evaluated level.
    Fact(&'static str),
    /// A byte-count fact as mebibytes, rounded as Maintainerr rounds it.
    MebibytesRounded(&'static str),
    /// A byte-count fact as mebibytes, unrounded.
    Mebibytes(&'static str),
    /// A field of the evaluated season or episode itself.
    Subject(&'static str),
    /// The named user's plays of the subject.
    UserPlayCount,
    /// The named user's latest play of the subject.
    UserLastPlayedAt,
    /// A fact of the title, read at every level: a season or an episode
    /// reads its show's value.
    TitleFact(&'static str),
    /// Whether a text fact of the title equals the given value, ignoring case.
    TitleTextIs(&'static str, &'static str),
    /// The title's first stored rating from the given source: its own value,
    /// or its vote count when `votes` is set.
    TitleRating { source: &'static str, votes: bool },
    /// The earliest of the given `YYYY-MM-DD` facts that are present.
    EarliestDay(&'static [&'static str]),
    /// When Scryer recorded the subject's newest file.
    LatestFileAddedAt,
}

impl Source {
    pub(super) fn is_per_user(self) -> bool {
        matches!(self, Self::UserPlayCount | Self::UserLastPlayedAt)
    }
}

/// How a Scryer fact can differ from the Maintainerr property it binds, in
/// the ways that decide whether a comparison could match titles Maintainerr
/// would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fidelity {
    /// The fact holds what Maintainerr reads, or differs only as its caveat says.
    Exact,
    /// A list that holds only some of the values Maintainerr's list holds.
    PartialList,
    /// A flag Scryer sets for only some of the titles Maintainerr sets it for.
    PartialFlag,
    /// A date that can be later than Maintainerr's.
    LaterDate,
    /// A date that can be earlier or later than Maintainerr's.
    EitherDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Binding {
    pub source: Source,
    pub shape: Shape,
    /// The values are usernames, checked against Scryer's users.
    pub usernames: bool,
    /// How the Scryer fact differs from the Maintainerr property, if it does.
    pub caveat: Option<&'static str>,
    pub fidelity: Fidelity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BindRefusal {
    /// No Scryer fact matches the property.
    Unmapped,
    /// The property reads media-server collections, playlists or labels.
    Collection,
    /// A Scryer fact matches, but not at this group's level.
    Level,
}

const MEDIA_SERVERS: &[&str] = &["plex", "jellyfin", "emby"];
const FAVOURITE_SERVERS: &[&str] = &["jellyfin", "emby"];
const WATCH_HISTORIES: &[&str] = &["plex", "jellyfin", "emby", "tautulli", "tracearr"];
const PER_USER_HISTORIES: &[&str] = &["tautulli", "tracearr", "streamystats"];
const REQUEST_APPS: &[&str] = &["seerr", "overseerr", "jellyseerr", "ombi"];

const ALL_LEVELS: &[MediaLevel] = &[
    MediaLevel::Movie,
    MediaLevel::Show,
    MediaLevel::Season,
    MediaLevel::Episode,
];
const SHOW_LEVELS: &[MediaLevel] = &[MediaLevel::Show, MediaLevel::Season, MediaLevel::Episode];
const TITLES: &[MediaLevel] = &[MediaLevel::Movie, MediaLevel::Show];

const LINKED_USERS_ONLY: &str =
    "Scryer lists only media-server accounts linked to a Scryer user, by their Scryer username";
const FIRST_IMPORT: &str =
    "Scryer uses when the first file was imported, not when the media server added the item";
const UNCONFIRMED_PER_USER: &str =
    "a user who never played the item reads as no plays or no play date";
const REQUESTED: &str =
    "Scryer says whether a request created the title, not whether a request exists";
const OWN_RATING: &str =
    "Scryer reads the rating from its own metadata, which it refreshes on its own schedule";
const OWN_STATUS: &str = "Scryer reads the show's status from its own metadata provider";
const FILE_RECORDED: &str =
    "Scryer uses when it recorded the newest file, not when Radarr imported it";
const OWN_RELEASE_DATES: &str =
    "Scryer reads the digital and physical release dates from its own metadata";
const ADDED_TO_SCRYER: &str =
    "Scryer uses when the title was added to Scryer, not when it was added to Sonarr or Radarr";
const OWN_TAGS: &str =
    "Scryer reads its own tags; the Sonarr and Radarr import does not carry tags over";
const SCRYER_REQUESTS: &str = "Scryer knows only the requests made in Scryer";

struct Entry {
    apps: &'static [&'static str],
    property: &'static str,
    type_key: &'static str,
    levels: &'static [MediaLevel],
    source: Source,
    shape: Shape,
    usernames: bool,
    caveat: Option<&'static str>,
    fidelity: Fidelity,
}

const fn entry(
    apps: &'static [&'static str],
    property: &'static str,
    type_key: &'static str,
    levels: &'static [MediaLevel],
    source: Source,
    shape: Shape,
) -> Entry {
    Entry {
        apps,
        property,
        type_key,
        levels,
        source,
        shape,
        usernames: false,
        caveat: None,
        fidelity: Fidelity::Exact,
    }
}

/// Scryer's lists hold only the accounts linked to a Scryer user.
const fn users(mut entry: Entry) -> Entry {
    entry.usernames = true;
    entry.fidelity = Fidelity::PartialList;
    entry
}

const fn fidelity(mut entry: Entry, fidelity: Fidelity) -> Entry {
    entry.fidelity = fidelity;
    entry
}

const fn caveat(mut entry: Entry, text: &'static str) -> Entry {
    entry.caveat = Some(text);
    entry
}

/// Maintainerr rule type keys.
const NUMBER: &str = "0";
const DATE: &str = "1";
const TEXT: &str = "2";
const BOOL: &str = "3";
const LIST: &str = "4";

const ENTRIES: &[Entry] = &[
    // Watch history from the media servers and their companions.
    caveat(
        users(entry(
            WATCH_HISTORIES,
            "seenBy",
            LIST,
            &[MediaLevel::Movie],
            Source::Fact("watched_by_usernames"),
            Shape::TextList,
        )),
        LINKED_USERS_ONLY,
    ),
    caveat(
        users(entry(
            WATCH_HISTORIES,
            "sw_watchers",
            LIST,
            SHOW_LEVELS,
            Source::Fact("watched_by_usernames"),
            Shape::TextList,
        )),
        LINKED_USERS_ONLY,
    ),
    entry(
        WATCH_HISTORIES,
        "lastViewedAt",
        DATE,
        ALL_LEVELS,
        Source::Fact("last_watched_by_anyone_at"),
        Shape::Instant,
    ),
    entry(
        WATCH_HISTORIES,
        "sw_lastWatched",
        DATE,
        &[MediaLevel::Show, MediaLevel::Season],
        Source::Fact("last_watched_by_anyone_at"),
        Shape::Instant,
    ),
    entry(
        WATCH_HISTORIES,
        "viewCount",
        NUMBER,
        // A show's or season's summed plays are not observed per title.
        &[MediaLevel::Movie, MediaLevel::Episode],
        Source::Fact("play_count_total"),
        Shape::Number,
    ),
    entry(
        WATCH_HISTORIES,
        "sw_amountOfViews",
        NUMBER,
        &[MediaLevel::Show],
        Source::Fact("play_count_total"),
        Shape::Number,
    ),
    entry(
        MEDIA_SERVERS,
        "genre",
        LIST,
        TITLES,
        Source::Fact("genres"),
        Shape::TextList,
    ),
    fidelity(
        caveat(
            entry(
                &["plex", "jellyfin", "emby", "tautulli"],
                "addDate",
                DATE,
                TITLES,
                Source::Fact("first_imported_at"),
                Shape::Instant,
            ),
            FIRST_IMPORT,
        ),
        Fidelity::LaterDate,
    ),
    caveat(
        users(entry(
            FAVOURITE_SERVERS,
            "favoritedBy",
            LIST,
            &[MediaLevel::Movie],
            Source::Fact("favorited_by_usernames"),
            Shape::TextList,
        )),
        LINKED_USERS_ONLY,
    ),
    caveat(
        entry(
            PER_USER_HISTORIES,
            "viewCountByUser",
            NUMBER,
            ALL_LEVELS,
            Source::UserPlayCount,
            Shape::Number,
        ),
        UNCONFIRMED_PER_USER,
    ),
    caveat(
        entry(
            PER_USER_HISTORIES,
            "lastViewedAtByUser",
            DATE,
            ALL_LEVELS,
            Source::UserLastPlayedAt,
            Shape::Instant,
        ),
        UNCONFIRMED_PER_USER,
    ),
    // Radarr.
    fidelity(
        caveat(
            entry(
                &["radarr"],
                "addDate",
                DATE,
                &[MediaLevel::Movie],
                Source::Fact("added_at"),
                Shape::Instant,
            ),
            ADDED_TO_SCRYER,
        ),
        Fidelity::LaterDate,
    ),
    fidelity(
        caveat(
            entry(
                &["radarr"],
                "tags",
                LIST,
                &[MediaLevel::Movie],
                Source::Fact("tags"),
                Shape::TextList,
            ),
            OWN_TAGS,
        ),
        Fidelity::PartialList,
    ),
    entry(
        &["radarr"],
        "monitored",
        BOOL,
        &[MediaLevel::Movie],
        Source::Fact("monitored"),
        Shape::Bool,
    ),
    entry(
        &["radarr"],
        "inCinemas",
        DATE,
        &[MediaLevel::Movie],
        Source::Fact("theatrical_release_date"),
        Shape::Day,
    ),
    entry(
        &["radarr"],
        "fileSize",
        NUMBER,
        &[MediaLevel::Movie],
        Source::MebibytesRounded("total_file_size_bytes"),
        Shape::Number,
    ),
    fidelity(
        caveat(
            entry(
                &["radarr"],
                "releaseDate",
                DATE,
                &[MediaLevel::Movie],
                Source::EarliestDay(&["digital_release_date", "physical_release_date"]),
                Shape::Day,
            ),
            OWN_RELEASE_DATES,
        ),
        Fidelity::EitherDate,
    ),
    fidelity(
        caveat(
            entry(
                &["radarr"],
                "fileDate",
                DATE,
                &[MediaLevel::Movie],
                Source::LatestFileAddedAt,
                Shape::Instant,
            ),
            FILE_RECORDED,
        ),
        Fidelity::LaterDate,
    ),
    caveat(
        entry(
            &["radarr"],
            "imdbRating",
            NUMBER,
            &[MediaLevel::Movie],
            Source::TitleRating {
                source: "imdb",
                votes: false,
            },
            Shape::Number,
        ),
        OWN_RATING,
    ),
    caveat(
        entry(
            &["radarr"],
            "imdbRatingVotes",
            NUMBER,
            &[MediaLevel::Movie],
            Source::TitleRating {
                source: "imdb",
                votes: true,
            },
            Shape::Number,
        ),
        OWN_RATING,
    ),
    // Plex: the IMDb and TMDB audience ratings Plex shows, both out of ten.
    caveat(
        entry(
            &["plex"],
            "rating_imdb",
            NUMBER,
            TITLES,
            Source::TitleRating {
                source: "imdb",
                votes: false,
            },
            Shape::Number,
        ),
        OWN_RATING,
    ),
    caveat(
        entry(
            &["plex"],
            "rating_imdbShow",
            NUMBER,
            &[MediaLevel::Season, MediaLevel::Episode],
            Source::TitleRating {
                source: "imdb",
                votes: false,
            },
            Shape::Number,
        ),
        OWN_RATING,
    ),
    caveat(
        entry(
            &["plex"],
            "rating_tmdb",
            NUMBER,
            TITLES,
            Source::TitleRating {
                source: "tmdb",
                votes: false,
            },
            Shape::Number,
        ),
        OWN_RATING,
    ),
    caveat(
        entry(
            &["plex"],
            "rating_tmdbShow",
            NUMBER,
            &[MediaLevel::Season, MediaLevel::Episode],
            Source::TitleRating {
                source: "tmdb",
                votes: false,
            },
            Shape::Number,
        ),
        OWN_RATING,
    ),
    // Sonarr.
    fidelity(
        caveat(
            entry(
                &["sonarr"],
                "addDate",
                DATE,
                &[MediaLevel::Show],
                Source::Fact("added_at"),
                Shape::Instant,
            ),
            ADDED_TO_SCRYER,
        ),
        Fidelity::LaterDate,
    ),
    fidelity(
        caveat(
            entry(
                &["sonarr"],
                "tags",
                LIST,
                SHOW_LEVELS,
                Source::Fact("tags"),
                Shape::TextList,
            ),
            OWN_TAGS,
        ),
        Fidelity::PartialList,
    ),
    entry(
        &["sonarr"],
        "monitored",
        BOOL,
        SHOW_LEVELS,
        Source::Fact("monitored"),
        Shape::Bool,
    ),
    entry(
        &["sonarr"],
        "firstAirDate",
        DATE,
        &[MediaLevel::Show],
        Source::Fact("first_aired"),
        Shape::Day,
    ),
    entry(
        &["sonarr"],
        "diskSizeEntireShow",
        NUMBER,
        SHOW_LEVELS,
        Source::Mebibytes("total_file_size_bytes"),
        Shape::Number,
    ),
    entry(
        &["sonarr"],
        "missing_episodes_show",
        NUMBER,
        SHOW_LEVELS,
        Source::Fact("missing_episode_count"),
        Shape::Number,
    ),
    caveat(
        entry(
            &["sonarr"],
            "status",
            TEXT,
            &[MediaLevel::Show, MediaLevel::Season],
            Source::TitleFact("series_status"),
            Shape::Text,
        ),
        OWN_STATUS,
    ),
    caveat(
        entry(
            &["sonarr"],
            "ended",
            BOOL,
            &[MediaLevel::Show, MediaLevel::Season],
            Source::TitleTextIs("series_status", "ended"),
            Shape::Bool,
        ),
        OWN_STATUS,
    ),
    entry(
        &["sonarr"],
        "episodeFileRank",
        NUMBER,
        &[MediaLevel::Episode],
        Source::Fact("episode_position_by_air_date"),
        Shape::Number,
    ),
    entry(
        &["sonarr"],
        "seasonNumber",
        NUMBER,
        &[MediaLevel::Season, MediaLevel::Episode],
        Source::Subject("season_number"),
        Shape::Number,
    ),
    entry(
        &["sonarr"],
        "episodeNumber",
        NUMBER,
        &[MediaLevel::Episode],
        Source::Subject("episode_number"),
        Shape::Number,
    ),
    // Request managers.
    fidelity(
        caveat(
            entry(
                REQUEST_APPS,
                "requestDate",
                DATE,
                TITLES,
                Source::Fact("requested_at"),
                Shape::Instant,
            ),
            SCRYER_REQUESTS,
        ),
        Fidelity::EitherDate,
    ),
    caveat(
        users(entry(
            REQUEST_APPS,
            "addUser",
            TEXT,
            TITLES,
            Source::Fact("requested_by_usernames"),
            Shape::TextList,
        )),
        LINKED_USERS_ONLY,
    ),
    fidelity(
        caveat(
            entry(
                REQUEST_APPS,
                "isRequested",
                BOOL,
                TITLES,
                Source::Fact("requested"),
                Shape::Bool,
            ),
            REQUESTED,
        ),
        Fidelity::PartialFlag,
    ),
];

/// Properties that read the media server's collections, playlists or labels.
const COLLECTION_PROPERTIES: &[&str] = &[
    "collections",
    "collection_names",
    "collectionsIncludingSmart",
    "collection_names_including_smart",
    "sw_collections_including_parent",
    "sw_collection_names_including_parent",
    "sw_collections_including_parent_and_smart",
    "sw_collection_names_including_parent_and_smart",
    "collection_siblings_lastViewedAt",
    "playlists",
    "playlist_names",
    "labels",
];

pub(super) fn bind(
    app_name: &str,
    property: &str,
    type_key: &str,
    level: MediaLevel,
) -> Result<Binding, BindRefusal> {
    let app = app_name.trim().to_ascii_lowercase();
    let mut matched_elsewhere = false;
    for entry in ENTRIES {
        if entry.property != property
            || entry.type_key != type_key
            || !entry.apps.contains(&app.as_str())
        {
            continue;
        }
        if entry.levels.contains(&level) {
            return Ok(Binding {
                source: entry.source,
                shape: entry.shape,
                usernames: entry.usernames,
                caveat: entry.caveat,
                fidelity: entry.fidelity,
            });
        }
        matched_elsewhere = true;
    }
    if matched_elsewhere {
        Err(BindRefusal::Level)
    } else if MEDIA_SERVERS.contains(&app.as_str()) && COLLECTION_PROPERTIES.contains(&property) {
        Err(BindRefusal::Collection)
    } else {
        Err(BindRefusal::Unmapped)
    }
}
