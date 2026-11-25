use std::collections::HashMap;
use std::path::Path;

use scryer_domain::{Collection, ExternalId, MediaFacet, NewTitle, Title};

use crate::library_discovery::derive_movie_probe_path;
use crate::library_scan::MetadataSearchItem;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

fn normalize_title_key(name: &str) -> String {
    crate::title_matching::canonical_lookup_key(name)
}

/// Every existing title (and alias) under one normalized name key. A name is
/// not an identity — remakes and originals share it — so a lookup by name must
/// see ALL same-name titles and pick by year; a single slot per key hid every
/// title but the last inserted (#148: a remake's folder could only ever be
/// compared with the original's title, or the other way round).
pub(crate) type TitleNameIndex = HashMap<String, Vec<usize>>;

fn index_title_name(existing_titles_by_name: &mut TitleNameIndex, name: &str, index: usize) {
    let indexes = existing_titles_by_name
        .entry(normalize_title_key(name))
        .or_default();
    if !indexes.contains(&index) {
        indexes.push(index);
    }
}

/// Whether a scanned/selected year is compatible with a catalog title's year:
/// equal, or unknown on either side. Two known, different years are two
/// different titles that merely share a name.
pub(crate) fn title_year_compatible(title: &Title, year: Option<u32>) -> bool {
    match (title.year, year) {
        (Some(title_year), Some(year)) => title_year >= 0 && title_year as u32 == year,
        _ => true,
    }
}

fn index_movie_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_smg_id: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) {
    index_title_name(existing_titles_by_name, &title.name, index);
    for alias in &title.aliases {
        index_title_name(existing_titles_by_name, alias, index);
    }
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("smg") {
            existing_titles_by_smg_id.insert(external_id.value.clone(), index);
        } else if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        } else if external_id.source.eq_ignore_ascii_case("imdb")
            && let Some(imdb_id) = crate::normalize::normalize_imdb_id(&external_id.value)
        {
            existing_titles_by_imdb_id.insert(imdb_id, index);
        } else if external_id.source.eq_ignore_ascii_case("tmdb") {
            existing_titles_by_tmdb_id.insert(external_id.value.clone(), index);
        }
    }
}

fn index_series_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) {
    index_title_name(existing_titles_by_name, &title.name, index);
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        } else if external_id.source.eq_ignore_ascii_case("imdb")
            && let Some(imdb_id) = crate::normalize::normalize_imdb_id(&external_id.value)
        {
            existing_titles_by_imdb_id.insert(imdb_id, index);
        } else if external_id.source.eq_ignore_ascii_case("tmdb") {
            existing_titles_by_tmdb_id.insert(external_id.value.clone(), index);
        }
    }
}

pub(crate) fn append_movie_title(
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_smg_id: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    title: Title,
) -> usize {
    let index = existing_titles.len();
    existing_titles.push(title);
    index_movie_title(
        &existing_titles[index],
        index,
        existing_titles_by_name,
        existing_titles_by_smg_id,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    );
    index
}

pub(crate) fn append_series_title(
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    title: Title,
) -> usize {
    let index = existing_titles.len();
    existing_titles.push(title);
    index_series_title(
        &existing_titles[index],
        index,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    );
    index
}

pub(crate) fn update_series_title_folder_path_index(
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    title: &Title,
    index: usize,
) {
    if let Some(folder_path) = title
        .folder_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .and_then(crate::stored_paths::folder_path_identity_key)
    {
        existing_titles_by_folder_path.insert(folder_path, index);
    }
}

pub(crate) fn update_movie_probe_path_index(
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    root: &Path,
    file_path: &str,
    index: usize,
) {
    let file_path_buf = stored_path_to_path_buf(file_path);
    if let Some(parent) = file_path_buf.parent()
        && parent != root
    {
        existing_titles_by_probe_path.insert(path_to_stored_string(parent), index);
    } else {
        existing_titles_by_probe_path.insert(file_path.to_string(), index);
    }
}

pub(crate) type MovieTitleIndexes = (
    TitleNameIndex,
    HashMap<String, usize>,
    HashMap<String, usize>,
    HashMap<String, usize>,
    HashMap<String, usize>,
);

pub(crate) fn build_movie_title_indexes(existing_titles: &[Title]) -> MovieTitleIndexes {
    let mut existing_titles_by_name = HashMap::new();
    let mut existing_titles_by_smg_id = HashMap::new();
    let mut existing_titles_by_tvdb_id = HashMap::new();
    let mut existing_titles_by_imdb_id = HashMap::new();
    let mut existing_titles_by_tmdb_id = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        index_movie_title(
            title,
            index,
            &mut existing_titles_by_name,
            &mut existing_titles_by_smg_id,
            &mut existing_titles_by_tvdb_id,
            &mut existing_titles_by_imdb_id,
            &mut existing_titles_by_tmdb_id,
        );
    }

    (
        existing_titles_by_name,
        existing_titles_by_smg_id,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
}

pub(crate) type SeriesTitleIndexes = (
    TitleNameIndex,
    HashMap<String, usize>,
    HashMap<String, usize>,
    HashMap<String, usize>,
);

pub(crate) fn build_series_title_indexes(existing_titles: &[Title]) -> SeriesTitleIndexes {
    let mut existing_titles_by_name = HashMap::new();
    let mut existing_titles_by_tvdb_id = HashMap::new();
    let mut existing_titles_by_imdb_id = HashMap::new();
    let mut existing_titles_by_tmdb_id = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        index_series_title(
            title,
            index,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
            &mut existing_titles_by_imdb_id,
            &mut existing_titles_by_tmdb_id,
        );
    }

    (
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
}

pub(crate) fn build_series_title_folder_path_index(
    existing_titles: &[Title],
) -> HashMap<String, usize> {
    let mut existing_titles_by_folder_path = HashMap::new();
    for (index, title) in existing_titles.iter().enumerate() {
        update_series_title_folder_path_index(&mut existing_titles_by_folder_path, title, index);
    }
    existing_titles_by_folder_path
}

pub(crate) fn build_movie_probe_path_indexes(
    root: &Path,
    existing_titles: &[Title],
    collections_by_title: &HashMap<String, Vec<Collection>>,
) -> HashMap<String, usize> {
    let mut existing_titles_by_probe_path = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        let collections = collections_by_title
            .get(&title.id)
            .cloned()
            .unwrap_or_default();
        if let Some(probe_path) = derive_movie_probe_path(root, title, &collections)
            && let Some(key) =
                crate::stored_paths::folder_path_identity_key(&path_to_stored_string(&probe_path))
        {
            existing_titles_by_probe_path.insert(key, index);
        }
    }

    existing_titles_by_probe_path
}

/// Whether a metadata match whose ids bound no catalog title may still bind a
/// same-name title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataMatchNameFallback {
    /// The scanned entry names an external id (folder or file tag, NFO,
    /// plexmatch, arr import). The match binds an existing title only through
    /// an id; a same-name title is a different title.
    IdOnly,
    /// A text-matched entry. A same-name title binds when its year agrees with
    /// the match's year, or with the entry's own year when the match has none.
    ByName { year_hint: Option<u32> },
}

impl MetadataMatchNameFallback {
    pub(crate) fn for_candidate(
        identity_hint: Option<&crate::library_scan_metadata::MetadataIdentityHint>,
        year_hint: Option<u32>,
    ) -> Self {
        if identity_hint.is_some_and(|hint| hint.has_external_ids()) {
            Self::IdOnly
        } else {
            Self::ByName { year_hint }
        }
    }
}

/// The existing catalog title a metadata match refers to, if any: the same
/// canonical (tvdb) id, else, when `name_fallback` allows it, a same-name title
/// whose year agrees with the match. A same-name title with a *different* year
/// is a different film (the remake vs the original) and must not absorb the
/// match — that misbinding is what left an original's folder refused as
/// "already owns another folder" against the remake's title.
pub(crate) fn find_existing_title_index_for_metadata_match(
    selected: &MetadataSearchItem,
    existing_titles: &[Title],
    existing_titles_by_name: &TitleNameIndex,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    name_fallback: MetadataMatchNameFallback,
) -> Option<usize> {
    if !selected.tvdb_id.trim().is_empty()
        && let Some(&index) = existing_titles_by_tvdb_id.get(&selected.tvdb_id)
    {
        return Some(index);
    }
    let MetadataMatchNameFallback::ByName { year_hint } = name_fallback else {
        return None;
    };
    let key = normalize_title_key(&selected.name);
    let selected_year = selected
        .year
        .and_then(|year| u32::try_from(year).ok())
        .or(year_hint);
    existing_titles_by_name
        .get(&key)?
        .iter()
        .copied()
        .find(|index| {
            existing_titles
                .get(*index)
                .is_some_and(|title| title_year_compatible(title, selected_year))
        })
}

/// The existing catalog series a metadata match refers to.
///
/// A TVDB-backed match binds by its TVDB id exactly as before. A match with no
/// TVDB id (a TMDB-primary series) binds on SMG's title id, then its TMDB id,
/// before falling back to the name and year. Those titles are rare, so they are
/// found by walking the catalog rather than by another index.
pub(crate) fn find_existing_series_title_index_for_metadata_match(
    selected: &MetadataSearchItem,
    existing_titles: &[Title],
    existing_titles_by_name: &TitleNameIndex,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    name_fallback: MetadataMatchNameFallback,
) -> Option<usize> {
    if selected.tvdb_id.trim().is_empty() {
        let smg_id = selected.smg_id.map(|value| value.to_string());
        let tmdb_id = selected
            .external_ids
            .iter()
            .find(|external_id| {
                external_id.source.eq_ignore_ascii_case("tmdb")
                    && crate::normalize::external_id_kind_fits_facet(
                        external_id,
                        &MediaFacet::Series,
                    )
            })
            .map(|external_id| external_id.value.trim().to_string())
            .filter(|value| !value.is_empty());
        // TMDB reuses one number for a movie and a series, and an anime title
        // also carries its mapped movies' TMDB ids (`tmdb:movie:N`), so a
        // stored TMDB id binds only when it is kinded as a series or unkinded.
        let has_id = |title: &Title, source: &str, value: &str| {
            title.external_ids.iter().any(|external_id| {
                external_id.source.eq_ignore_ascii_case(source)
                    && external_id.value.trim() == value
                    && crate::normalize::external_id_kind_fits_facet(
                        external_id,
                        &MediaFacet::Series,
                    )
            })
        };
        for (source, value) in [("smg", smg_id), ("tmdb", tmdb_id)] {
            let Some(value) = value else {
                continue;
            };
            if let Some(index) = existing_titles
                .iter()
                .position(|title| has_id(title, source, &value))
            {
                return Some(index);
            }
        }
    }
    find_existing_title_index_for_metadata_match(
        selected,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        name_fallback,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "a movie match reads every in-memory id index and the name index together"
)]
pub(crate) fn find_existing_movie_title_index_for_metadata_match(
    selected: &MetadataSearchItem,
    existing_titles: &[Title],
    existing_titles_by_name: &TitleNameIndex,
    existing_titles_by_smg_id: &HashMap<String, usize>,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
    existing_titles_by_imdb_id: &HashMap<String, usize>,
    existing_titles_by_tmdb_id: &HashMap<String, usize>,
    name_fallback: MetadataMatchNameFallback,
) -> Option<usize> {
    if let Some(smg_id) = selected.smg_id.map(|value| value.to_string())
        && let Some(&index) = existing_titles_by_smg_id.get(&smg_id)
    {
        return Some(index);
    }
    if !selected.tvdb_id.trim().is_empty()
        && let Some(&index) = existing_titles_by_tvdb_id.get(&selected.tvdb_id)
    {
        return Some(index);
    }

    for external_id in &selected.external_ids {
        if external_id.source.eq_ignore_ascii_case("tmdb")
            && let Some(&index) = existing_titles_by_tmdb_id.get(external_id.value.trim())
        {
            return Some(index);
        }
        if external_id.source.eq_ignore_ascii_case("imdb")
            && let Some(imdb_id) = crate::normalize::normalize_imdb_id(&external_id.value)
            && let Some(&index) = existing_titles_by_imdb_id.get(&imdb_id)
        {
            return Some(index);
        }
    }

    find_existing_title_index_for_metadata_match(
        selected,
        existing_titles,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        name_fallback,
    )
}

fn push_metadata_match_external_id(external_ids: &mut Vec<ExternalId>, source: &str, value: &str) {
    let value = value.trim();
    if value.is_empty()
        || external_ids
            .iter()
            .any(|external_id| external_id.source.eq_ignore_ascii_case(source))
    {
        return;
    }

    external_ids.push(ExternalId::new(source.to_string(), value.to_string()));
}

pub(crate) fn build_new_title_from_metadata_match(
    facet: &MediaFacet,
    selected: &MetadataSearchItem,
) -> NewTitle {
    // The batched search returns smg/tmdb/imdb ids for every facet, and every
    // facet keeps them. Indexer search subjects, RSS candidate indexes,
    // notification payloads and the `externalIds` readback all read these ids
    // without checking the facet, so a scan-created series or anime title must
    // carry the same identity set a movie does.
    let mut external_ids = Vec::new();
    if let Some(smg_id) = selected.smg_id {
        push_metadata_match_external_id(&mut external_ids, "smg", &smg_id.to_string());
    }
    push_metadata_match_external_id(&mut external_ids, "tvdb", &selected.tvdb_id);
    for external_id in &selected.external_ids {
        let source = if external_id.source.eq_ignore_ascii_case("smg") {
            Some("smg")
        } else if external_id.source.eq_ignore_ascii_case("tvdb") {
            Some("tvdb")
        } else if external_id.source.eq_ignore_ascii_case("tmdb") {
            Some("tmdb")
        } else if external_id.source.eq_ignore_ascii_case("imdb") {
            Some("imdb")
        } else {
            None
        };
        if let Some(source) = source {
            push_metadata_match_external_id(&mut external_ids, source, &external_id.value);
        }
    }

    NewTitle {
        name: selected.name.clone(),
        facet: facet.clone(),
        monitored: false,
        tags: vec![],
        external_ids,
        min_availability: None,
        year: selected.year,
        ..Default::default()
    }
}
