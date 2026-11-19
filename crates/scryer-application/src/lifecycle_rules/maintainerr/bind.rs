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
}

impl Source {
    pub(super) fn is_per_user(self) -> bool {
        matches!(self, Self::UserPlayCount | Self::UserLastPlayedAt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Binding {
    pub source: Source,
    pub shape: Shape,
    /// The values are usernames, checked against Scryer's users.
    pub usernames: bool,
    /// How the Scryer fact differs from the Maintainerr property, if it does.
    pub caveat: Option<&'static str>,
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

struct Entry {
    apps: &'static [&'static str],
    property: &'static str,
    type_key: &'static str,
    levels: &'static [MediaLevel],
    source: Source,
    shape: Shape,
    usernames: bool,
    caveat: Option<&'static str>,
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
    }
}

const fn users(mut entry: Entry) -> Entry {
    entry.usernames = true;
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
    entry(
        &["radarr"],
        "addDate",
        DATE,
        &[MediaLevel::Movie],
        Source::Fact("added_at"),
        Shape::Instant,
    ),
    entry(
        &["radarr"],
        "tags",
        LIST,
        &[MediaLevel::Movie],
        Source::Fact("tags"),
        Shape::TextList,
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
    // Sonarr.
    entry(
        &["sonarr"],
        "addDate",
        DATE,
        &[MediaLevel::Show],
        Source::Fact("added_at"),
        Shape::Instant,
    ),
    entry(
        &["sonarr"],
        "tags",
        LIST,
        SHOW_LEVELS,
        Source::Fact("tags"),
        Shape::TextList,
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
    entry(
        REQUEST_APPS,
        "requestDate",
        DATE,
        TITLES,
        Source::Fact("requested_at"),
        Shape::Instant,
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
