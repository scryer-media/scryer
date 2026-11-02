//! Provider item → Scryer subject identity (RFC 137, "Identity mapping").
//!
//! The RFC's five numbered rules, and where each one lives:
//!
//! 1. cache provider item ids per connection — the stored signal row keys on
//!    `provider_item_id`, so a re-sync updates in place;
//! 2. extract TMDB, TVDB, and IMDb ids from provider metadata — the adapter
//!    does that, and hands this module a normalized
//!    [`ProviderPlayedItem`](crate::ports::ProviderPlayedItem);
//! 3. match the correct Scryer title/episode **and facet** — [`resolve_title`]
//!    refuses a cross-facet match, so a movie's TMDB id can never land on a
//!    series that happens to share it;
//! 4. **retain ambiguous/unmatched observations without applying them to a
//!    subject** — every resolver here returns `None` rather than guessing, and
//!    the caller still writes the row;
//! 5. retry mapping after catalog refresh — mapping is redone from scratch on
//!    every sweep, so a title imported later is picked up without refetching
//!    history.
//!
//! Everything in this file is a pure function over already-loaded indexes. The
//! I/O that builds those indexes lives in [`super::sync`], which is what keeps
//! mapping batched: one external-id lookup for the whole sweep, and one episode
//! read per resolved series, never one query per watched item.

use std::collections::HashMap;

use scryer_domain::{MediaFacet, MediaServerPathMapping, MediaServerSignalKind};

use crate::media_servers::map_media_server_path;
use crate::ports::ProviderPlayedItem;
use crate::title_folder_rules::stored_path_is_inside_folder;

/// External id sources joined against `title_external_ids`, strongest first.
///
/// TMDB leads because both Jellyfin and Scryer treat it as the primary id for
/// movies and shows; IMDb is last because its ids are the most widely
/// copy-pasted and therefore the most likely to be wrong on a stale library.
pub const SIGNAL_EXTERNAL_ID_SOURCES: [&str; 3] = ["tmdb", "tvdb", "imdb"];

/// Which Scryer subject an observation was attributed to.
///
/// The two fields move together by design. A movie signal carries a title and
/// no episode. An episode signal carries **both** or **neither**: attributing
/// an episode observation to its series alone would be a show-level rollup,
/// and this wave stores none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MappedSignalSubject {
    pub title_id: Option<String>,
    pub episode_id: Option<String>,
}

impl MappedSignalSubject {
    /// The unmapped subject: retained, attributed to nothing.
    pub fn unmapped() -> Self {
        Self::default()
    }

    pub fn is_mapped(&self) -> bool {
        self.title_id.is_some()
    }
}

/// Titles reachable by external id, as one sweep's worth of catalog.
///
/// Keyed by `(lowercase source, external id)`; the value is every distinct
/// title that claims that id, with its facet. More than one entry is a catalog
/// ambiguity, not a choice to make here.
pub type TitleExternalIdIndex = HashMap<(String, String), Vec<(String, MediaFacet)>>;

/// Episodes of the resolved series, keyed by `(title id, season, episode)`.
///
/// The value is every episode id at those coordinates. Scryer stores season and
/// episode numbers as text, so the index is built from parsed integers: `"01"`
/// and `"1"` are the same episode, and a non-numeric label simply never enters
/// the index rather than matching something by accident.
pub type EpisodeNumberIndex = HashMap<(String, i64, i64), Vec<String>>;

/// Whether a facet may answer for this kind of observation.
///
/// This is RFC rule 3's "and facet". Jellyfin will happily report a movie and a
/// series that share a TVDB id; only one of them can be what was watched.
fn facet_accepts(kind: MediaServerSignalKind, facet: &MediaFacet) -> bool {
    match kind {
        MediaServerSignalKind::Movie => *facet == MediaFacet::Movie,
        // Anime is a distinct facet in Scryer but the same thing to a provider:
        // an episode signal must be allowed to land on either.
        MediaServerSignalKind::Episode => matches!(facet, MediaFacet::Series | MediaFacet::Anime),
    }
}

/// The Scryer title an item's external ids point at, or `None`.
///
/// For an episode this resolves the **series**: episode-level provider ids are
/// not reliably present or consistent, so the join that actually holds is the
/// series plus the season/episode numbers ([`resolve_episode`]).
///
/// Sources are tried strongest first. A source that matches nothing falls
/// through to the next. A source that matches *two different* titles of an
/// acceptable facet stops the resolution entirely: the catalog is claiming one
/// external id for two subjects, and picking either one would be a guess. RFC
/// rule 4 says retain it unattributed instead.
pub fn resolve_title(item: &ProviderPlayedItem, index: &TitleExternalIdIndex) -> Option<String> {
    let ids = match item.kind {
        MediaServerSignalKind::Movie => &item.external_ids,
        MediaServerSignalKind::Episode => &item.series_external_ids,
    };

    for source in SIGNAL_EXTERNAL_ID_SOURCES {
        let Some(external_id) = ids.get(source).map(|value| value.trim()) else {
            continue;
        };
        if external_id.is_empty() {
            continue;
        }
        let Some(candidates) = index.get(&(source.to_string(), external_id.to_string())) else {
            continue;
        };

        let mut accepted = candidates
            .iter()
            .filter(|(_, facet)| facet_accepts(item.kind, facet))
            .map(|(title_id, _)| title_id.as_str())
            .collect::<Vec<_>>();
        accepted.sort_unstable();
        accepted.dedup();

        match accepted.as_slice() {
            [] => continue,
            [only] => return Some((*only).to_string()),
            _ => return None,
        }
    }

    None
}

/// Where Scryer keeps files, as a check on a media server's file paths: every
/// library root, plus the connection's path mappings read from the server's
/// side back to Scryer's.
pub struct ScryerLibraryPaths {
    /// `None` when the roots could not be read: then no path can be shown to
    /// lie outside them.
    roots: Option<Vec<String>>,
    /// The connection's mappings, server path to Scryer path.
    to_scryer: Vec<MediaServerPathMapping>,
}

impl ScryerLibraryPaths {
    pub fn new(roots: Option<Vec<String>>, mappings: &[MediaServerPathMapping]) -> Self {
        Self {
            roots,
            to_scryer: mappings
                .iter()
                .map(|mapping| MediaServerPathMapping {
                    source_path: mapping.destination_path.clone(),
                    destination_path: mapping.source_path.clone(),
                    sort_order: mapping.sort_order,
                })
                .collect(),
        }
    }

    /// Whether a server file path could be one of Scryer's files: once mapped
    /// to Scryer's spelling it lies inside a library root, or the roots are
    /// unknown.
    pub fn may_hold(&self, server_path: &str) -> bool {
        let Some(roots) = &self.roots else {
            return true;
        };
        let path = map_media_server_path(server_path, &self.to_scryer);
        roots
            .iter()
            .any(|root| stored_path_is_inside_folder(root, &path))
    }
}

/// Whether an item lies outside Scryer's catalog: it carries at least one
/// usable external id, no Scryer title of any facet claims any of them, and
/// none of its files the server reports lies inside a Scryer library root.
///
/// An item with no usable id is never foreign. It could be a Scryer title the
/// server failed to identify, so nothing can be concluded about it. An item
/// whose ids reach a Scryer title that its facet cannot accept, or reach two
/// titles at once, is not foreign either: Scryer knows the id, it just cannot
/// settle which title was watched. An item whose file is in a Scryer library is
/// not foreign whatever its ids say: the server may have identified Scryer's
/// file as a different film. With no file paths reported, the ids decide.
pub fn is_foreign_item(
    item: &ProviderPlayedItem,
    index: &TitleExternalIdIndex,
    library: &ScryerLibraryPaths,
) -> bool {
    if item.file_paths.iter().any(|path| library.may_hold(path)) {
        return false;
    }
    let ids = match item.kind {
        MediaServerSignalKind::Movie => &item.external_ids,
        MediaServerSignalKind::Episode => &item.series_external_ids,
    };
    let mut usable = SIGNAL_EXTERNAL_ID_SOURCES
        .iter()
        .filter_map(|source| {
            let value = ids.get(*source)?.trim();
            (!value.is_empty()).then(|| ((*source).to_string(), value.to_string()))
        })
        .peekable();
    if usable.peek().is_none() {
        return false;
    }
    usable.all(|key| !index.contains_key(&key))
}

/// The Scryer episode at this item's coordinates under `title_id`, or `None`.
///
/// Requires both numbers: a played item with no season or no episode index has
/// no coordinates to join on. Two episodes sharing coordinates under one series
/// is an ambiguity, and stays unmapped for the same reason as above.
pub fn resolve_episode(
    item: &ProviderPlayedItem,
    title_id: &str,
    index: &EpisodeNumberIndex,
) -> Option<String> {
    let season = item.season_number?;
    let episode = item.episode_number?;
    match index
        .get(&(title_id.to_string(), season, episode))
        .map(Vec::as_slice)
    {
        Some([only]) => Some(only.clone()),
        _ => None,
    }
}

/// Full subject resolution for one item.
///
/// An episode whose series resolves but whose episode does not comes back fully
/// unmapped, not half-mapped: see [`MappedSignalSubject`].
pub fn resolve_subject(
    item: &ProviderPlayedItem,
    titles: &TitleExternalIdIndex,
    episodes: &EpisodeNumberIndex,
) -> MappedSignalSubject {
    let Some(title_id) = resolve_title(item, titles) else {
        return MappedSignalSubject::unmapped();
    };

    match item.kind {
        MediaServerSignalKind::Movie => MappedSignalSubject {
            title_id: Some(title_id),
            episode_id: None,
        },
        MediaServerSignalKind::Episode => match resolve_episode(item, &title_id, episodes) {
            Some(episode_id) => MappedSignalSubject {
                title_id: Some(title_id),
                episode_id: Some(episode_id),
            },
            None => MappedSignalSubject::unmapped(),
        },
    }
}
