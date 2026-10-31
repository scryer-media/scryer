use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use super::*;
#[cfg(test)]
use crate::library_filename_parser::library_title_walk;
#[cfg(test)]
use crate::library_filename_parser::{LibraryQueryEvidence, parse_library_filename};
pub(crate) use crate::library_filename_parser::{
    LibraryTitleWalk, normalize_folder_name, strip_year_suffix,
};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use scryer_domain::VIDEO_EXTENSIONS;
use unicode_normalization::UnicodeNormalization;

const LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY: usize = 16;
// `_v2` marks the move from SHA-256 to domain-separated BLAKE3. The change
// needs no migration: the freshness check compares the stored scheme to the
// live one, so every `_v1` row reads as changed, is recomputed, and is upserted
// on its next probe. Cost is one extra probe per path on the first pass.
const LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME: &str = "immediate_children_v2";
const LIBRARY_PROBE_SIGNATURE_FILE_SCHEME: &str = "file_snapshot_v2";
pub(crate) const LIBRARY_SCAN_MAX_RECURSIVE_DEPTH: usize = 3;

// Aligned with Sonarr/Radarr special-folder and root-folder exclusion behavior.
const LIBRARY_IGNORED_DIR_NAMES: &[&str] = &[
    "@eadir",
    ".@__thumb",
    "plex versions",
    "$recycle.bin",
    "#recycle",
    "recycler",
    "trash",
    ".trashes",
    "system volume information",
    "lost+found",
    "boot",
    "bootmgr",
    "cache",
    "caches",
    "cachedmessages",
    "msocache",
    "recovery",
    "temporary internet files",
    "windows",
    ".fseventd",
    ".spotlight",
    ".vol",
    ".appledb",
    ".appledesktop",
    ".appledouble",
    ".grab",
];
const LIBRARY_IGNORED_MEDIA_SUBDIR_NAMES: &[&str] = &[
    "extras",
    "extrafanart",
    "backdrops",
    "behind the scenes",
    "deleted scenes",
    "featurette",
    "featurettes",
    "interview",
    "interviews",
    "other",
    "scene",
    "scenes",
    "sample",
    "samples",
    "short",
    "shorts",
    "theme music",
    "trailer",
    "trailers",
];
const LIBRARY_IGNORED_MEDIA_EXTRA_FILE_SUFFIXES: &[&str] = &[
    "trailer",
    "other",
    "behindthescenes",
    "deleted",
    "featurette",
    "interview",
    "scene",
    "short",
];

#[derive(Clone, Debug)]
pub(crate) struct MovieTopLevelEntry {
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
}

#[cfg(test)]
pub(crate) type MovieTopLevelEntryBatchReceiver =
    tokio::sync::mpsc::Receiver<AppResult<Vec<MovieTopLevelEntry>>>;

#[derive(Debug)]
pub(crate) struct TitleFolderBoundary {
    pub(crate) path: PathBuf,
    pub(crate) rejection: Option<String>,
}

pub(crate) struct TitleFolderBoundaryPolicy {
    pub(crate) facet: MediaFacet,
    pub(crate) stored_folders: Vec<PathBuf>,
    pub(crate) other_roots: Vec<PathBuf>,
}

impl TitleFolderBoundaryPolicy {
    fn contains_stored_folder(root: &Path, folder: &Path) -> bool {
        let root = crate::stored_paths::folder_path_identity_key(&path_to_stored_string(root));
        let folder = crate::stored_paths::folder_path_identity_key(&path_to_stored_string(folder));
        match (root, folder) {
            (Some(root), Some(folder)) => root != folder && Path::new(&folder).starts_with(&root),
            _ => false,
        }
    }

    pub(crate) async fn load(
        app: &AppUseCase,
        facet: &MediaFacet,
        library_id: &str,
        root: &Path,
    ) -> AppResult<Self> {
        let titles = app
            .services
            .catalog
            .titles
            .list_delete_preview_info()
            .await?;
        let stored_folders = titles
            .iter()
            .filter(|title| title.library_id == library_id)
            .filter_map(|title| title.folder_path.as_deref())
            .map(stored_path_to_path_buf)
            .filter(|path| Self::contains_stored_folder(root, path))
            .collect();
        let mut other_roots: Vec<_> = app
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .flat_map(|library| library.roots)
            .map(|root| stored_path_to_path_buf(&root.path))
            .filter(|path| Self::contains_stored_folder(root, path))
            .collect();
        other_roots.extend(
            titles
                .into_iter()
                .filter(|title| title.library_id != library_id)
                .filter_map(|title| title.folder_path)
                .map(|path| stored_path_to_path_buf(&path))
                .filter(|path| {
                    Self::contains_stored_folder(root, path)
                        || crate::stored_paths::folder_paths_match(
                            &path_to_stored_string(root),
                            &path_to_stored_string(path),
                        )
                }),
        );
        Ok(Self {
            facet: facet.clone(),
            stored_folders,
            other_roots,
        })
    }
}

pub(crate) async fn episode_folder_patterns(
    app: &AppUseCase,
    facet: &MediaFacet,
) -> AppResult<Vec<regex::Regex>> {
    if *facet == MediaFacet::Movie {
        return Ok(Vec::new());
    }
    let season = crate::library::rename::normalize_season_folder_template_or_default(
        app.read_setting_string_value(SEASON_FOLDER_TEMPLATE_KEY, Some(facet.as_str()))
            .await?,
    );
    let specials = crate::library::rename::normalize_specials_folder_template_or_default(
        app.read_setting_string_value(SPECIALS_FOLDER_TEMPLATE_KEY, Some(facet.as_str()))
            .await?,
    );
    // Naming changes do not rename existing library directories. Keep the
    // conventional episode layouts recognizable alongside current templates.
    [
        season,
        specials,
        "S{season:00}".into(),
        "Season {season}".into(),
        "Specials".into(),
    ]
    .iter()
    .map(|template| crate::library::rename::episode_folder_discovery_pattern(template))
    .collect()
}

const MAX_UNOWNED_TITLE_DEPTH: usize = 8;

#[derive(Default)]
struct BoundaryEvidence {
    has_media: bool,
    title: Option<String>,
    ids: [Option<String>; 3],
    conflict: bool,
    names_differ: bool,
}

impl BoundaryEvidence {
    fn merge(&mut self, other: Self) {
        self.has_media |= other.has_media;
        self.conflict |= other.conflict;
        self.names_differ |= other.names_differ;
        Self::merge_value(&mut self.title, other.title, &mut self.names_differ);
        for (current, incoming) in self.ids.iter_mut().zip(other.ids) {
            Self::merge_value(current, incoming, &mut self.conflict);
        }
    }

    fn merge_value(current: &mut Option<String>, incoming: Option<String>, conflict: &mut bool) {
        if let Some(incoming) = incoming {
            if current.as_ref().is_some_and(|current| current != &incoming) {
                *conflict = true;
                if current.as_ref().is_some_and(|value| incoming < *value) {
                    *current = Some(incoming);
                }
            } else {
                *current = Some(incoming);
            }
        }
    }

    fn from_name(name: &str) -> Self {
        let (walk, conflict) =
            crate::library_filename_parser::library_title_walk_with_id_conflicts(name);
        let title = crate::helpers::parse_usable_release_title(name)
            .map(|parsed| crate::title_matching::canonical_lookup_key(&parsed.normalized_title))
            .filter(|title| !title.is_empty());
        Self {
            title,
            conflict,
            ids: walk
                .map(|walk| [walk.imdb_id, walk.tmdb_id, walk.tvdb_id])
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn from_external_ids(external_ids: &[scryer_domain::ExternalId]) -> Self {
        let mut identity = Self::default();
        for external in external_ids {
            match external.source.as_str() {
                "imdb" => identity.ids[0] = crate::normalize::normalize_imdb_id(&external.value),
                "tmdb" => identity.ids[1] = crate::normalize::normalize_numeric_id(&external.value),
                "tvdb" => identity.ids[2] = crate::normalize::normalize_numeric_id(&external.value),
                _ => {}
            }
        }
        identity
    }

    fn conflicts_with(&self, other: &Self) -> bool {
        self.conflict
            || other.conflict
            || self
                .ids
                .iter()
                .zip(&other.ids)
                .any(|(a, b)| a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a != b))
    }

    #[cfg(test)]
    async fn contradicts_folder(&self, path: &Path, episodic: bool) -> bool {
        let folder = Self::from_name(&path.file_name().unwrap_or_default().to_string_lossy());
        let Ok(local) = boundary_sidecar_evidence(path, episodic).await else {
            return true;
        };
        self.conflicts_with(&folder)
            || local
                .as_ref()
                .is_some_and(|local| self.conflicts_with(local) || folder.conflicts_with(local))
    }
}

async fn boundary_sidecar_evidence(
    path: &Path,
    episodic: bool,
) -> std::io::Result<Option<BoundaryEvidence>> {
    // Keep the existing NFO-before-plexmatch precedence, but preserve I/O
    // failures: an unreadable sidecar is not evidence that identity is absent.
    let files: &[&str] = if episodic {
        &["tvshow.nfo", ".plexmatch"]
    } else {
        &["movie.nfo"]
    };
    for name in files {
        let content = match read_discovery_nfo(&path.join(name)).await {
            Ok(Some(content)) => content,
            Ok(None) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let meta = if *name == ".plexmatch" {
            crate::nfo::parse_plexmatch(&content)
        } else {
            crate::nfo::parse_nfo(&content)
        };
        if !meta.has_external_ids()
            && meta
                .title
                .as_deref()
                .is_none_or(|title| title.trim().is_empty())
        {
            continue;
        }
        return Ok(Some(BoundaryEvidence {
            title: meta
                .title
                .as_deref()
                .map(crate::title_matching::canonical_lookup_key),
            ids: [
                meta.imdb_id
                    .as_deref()
                    .and_then(crate::normalize::normalize_imdb_id),
                meta.tmdb_id
                    .as_deref()
                    .and_then(crate::normalize::normalize_numeric_id),
                meta.tvdb_id
                    .as_deref()
                    .and_then(crate::normalize::normalize_numeric_id),
            ],
            ..Default::default()
        }));
    }
    Ok(None)
}

struct ScanBoundaryOwner<'a> {
    name: &'a str,
    aliases: &'a [String],
    identity: BoundaryEvidence,
}

struct BoundaryFrame {
    path: PathBuf,
    entries: tokio::fs::ReadDir,
    depth: usize,
    season: bool,
    part: bool,
    evidence: BoundaryEvidence,
    has_title_nfo: bool,
    local_identity: Option<BoundaryEvidence>,
    matches_folder: bool,
    matches_parent: bool,
    child_titles: bool,
    blocked: bool,
    inspection_error: Option<String>,
    rejection_emitted: bool,
    skip_extras: bool,
    deferred_extras: bool,
    extras_pass: bool,
}

fn conventional_season(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            crate::library::workflow::scan_title_files::recognize_season_folder_name(name).is_some()
        })
}

/// Disc, disc-structure and anime bonus folders that hold part of their
/// parent title's media rather than another title. Their files stay visible
/// to the per-title walker; only boundary discovery folds them into the parent.
fn is_title_part_folder_name(name: &str) -> bool {
    const FIXED: &[&str] = &[
        "bdmv",
        "video_ts",
        "stream",
        "certificate",
        "ncop",
        "nced",
        "ova",
        "oad",
        "ona",
        "sp",
        "sps",
    ];
    let name = name.trim().to_ascii_lowercase();
    if FIXED.contains(&name.as_str()) {
        return true;
    }
    ["disc", "disk", "cd", "dvd"].iter().any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|rest| {
            let digits = rest.strip_prefix([' ', '_', '-', '.']).unwrap_or(rest);
            (1..=2).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

fn title_part_folder(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_title_part_folder_name)
}

#[cfg(test)]
tokio::task_local! {
    static BOUNDARY_ENTRY_METADATA_FAILURE: PathBuf;
}

async fn boundary_entry_file_type(
    entry: &tokio::fs::DirEntry,
) -> std::io::Result<std::fs::FileType> {
    #[cfg(test)]
    if BOUNDARY_ENTRY_METADATA_FAILURE
        .try_with(|path| path == &entry.path())
        .unwrap_or(false)
    {
        return Err(std::io::Error::other("injected entry metadata failure"));
    }
    entry.file_type().await
}

pub(crate) async fn read_discovery_nfo(path: &Path) -> std::io::Result<Option<String>> {
    use tokio::io::AsyncReadExt;
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Ok(None);
    }
    let file = tokio::fs::File::open(path).await?;
    if file.metadata().await?.len() > MAX_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > MAX_BYTES {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

impl BoundaryFrame {
    fn identity_conflicts(&self) -> bool {
        let folder = BoundaryEvidence::from_name(
            &self.path.file_name().unwrap_or_default().to_string_lossy(),
        );
        self.evidence.conflicts_with(&folder)
            || self.local_identity.as_ref().is_some_and(|identity| {
                self.evidence.conflicts_with(identity) || folder.conflicts_with(identity)
            })
    }

    fn is_episode_container_of(
        &self,
        parent: &Self,
        owner: Option<&ScanBoundaryOwner<'_>>,
    ) -> bool {
        if !self.evidence.has_media
            || self.blocked
            || self.child_titles
            || self.identity_conflicts()
            || self.evidence.conflicts_with(&parent.evidence)
        {
            return false;
        }
        let parent_identity = owner
            .map(|owner| &owner.identity)
            .or(parent.local_identity.as_ref());
        if let Some(child) = self.local_identity.as_ref() {
            let Some(parent) = parent_identity else {
                return false;
            };
            if child.conflicts_with(parent) {
                return false;
            }
            // A sidecar declares a title boundary unless an explicit identity
            // proves it is the same title. Similar names are insufficient.
            return child
                .ids
                .iter()
                .zip(&parent.ids)
                .any(|(a, b)| a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a == b));
        }
        if parent_identity
            .as_ref()
            .is_some_and(|identity| self.evidence.conflicts_with(identity))
        {
            return false;
        }
        let matches_owner = owner.is_some_and(|title| {
            let folder = BoundaryEvidence::from_name(
                &self.path.file_name().unwrap_or_default().to_string_lossy(),
            );
            folder
                .title
                .iter()
                .chain(self.evidence.title.iter())
                .any(|name| {
                    std::iter::once(title.name)
                        .chain(title.aliases.iter().map(String::as_str))
                        .any(|alias| crate::title_matching::canonical_lookup_key(alias) == *name)
                })
        });
        if self.matches_folder && !self.matches_parent && !self.season && !matches_owner {
            return false;
        }
        self.season
            || self.matches_parent
            || matches_owner
            || (parent.has_title_nfo && self.evidence.has_media)
    }

    async fn open(
        path: PathBuf,
        depth: usize,
        season: bool,
        part: bool,
        episodic: bool,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> AppResult<Self> {
        let folded = season || part;
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| AppError::Repository(e.to_string()))?;
        let mut evidence = BoundaryEvidence::default();

        let folder_name =
            BoundaryEvidence::from_name(&path.file_name().unwrap_or_default().to_string_lossy())
                .title;
        let parent_name = path
            .parent()
            .and_then(Path::file_name)
            .map(|name| BoundaryEvidence::from_name(&name.to_string_lossy()))
            .and_then(|e| e.title);
        let mut matches_folder = false;
        let mut matches_parent = false;
        let mut inspection_error = None;
        // Inspect immediate entries only. Reopen for child traversal so memory
        // stays independent of directory width and extras do not depend on order.
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    inspection_error.get_or_insert_with(|| error.to_string());
                    break;
                }
            };
            let kind = match boundary_entry_file_type(&entry).await {
                Ok(kind) => kind,
                Err(error) => {
                    inspection_error.get_or_insert_with(|| error.to_string());
                    continue;
                }
            };
            let child = entry.path();
            if kind.is_file()
                && is_allowed_video_path(&child)
                && !should_skip_library_top_level_entry(&child, false)
                && !child
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_ignored_media_extra_file_name)
            {
                let mut file = BoundaryEvidence::from_name(
                    &child.file_stem().unwrap_or_default().to_string_lossy(),
                );
                file.has_media = true;
                matches_folder |= file
                    .title
                    .as_ref()
                    .zip(folder_name.as_ref())
                    .is_some_and(|(a, b)| a == b);
                matches_parent |= file
                    .title
                    .as_ref()
                    .zip(parent_name.as_ref())
                    .is_some_and(|(a, b)| a == b);
                evidence.merge(file);
            }
        }
        let entries = tokio::fs::read_dir(&path).await.map_err(|error| {
            AppError::Repository(
                inspection_error
                    .clone()
                    .unwrap_or_else(|| error.to_string()),
            )
        })?;
        // Disc and bonus folders fold only media presence into their title,
        // so their sidecars are not read as title identity.
        let local_identity = if part {
            None
        } else {
            match boundary_sidecar_evidence(&path, episodic).await {
                Ok(identity) => identity,
                Err(error) => {
                    inspection_error.get_or_insert_with(|| error.to_string());
                    None
                }
            }
        };
        let has_title_nfo = local_identity.is_some();
        let skip_extras = depth > 0 && (evidence.has_media || has_title_nfo || folded);
        Ok(Self {
            path,
            entries,
            depth,
            season,
            part,
            evidence,
            has_title_nfo,
            local_identity,
            matches_folder,
            matches_parent,
            child_titles: false,
            blocked: inspection_error.is_some(),
            inspection_error,
            rejection_emitted: false,
            skip_extras,
            deferred_extras: false,
            extras_pass: false,
        })
    }
}

/// Discover existing title boundaries independently of future naming settings.
/// Only immediate-directory summaries are retained along the active DFS path.
pub(crate) fn stream_title_folder_boundaries(
    root: PathBuf,
    policy: TitleFolderBoundaryPolicy,
    batch_size: usize,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::sync::mpsc::Receiver<AppResult<Vec<TitleFolderBoundary>>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY);
    tokio::spawn(async move {
        let result =
            discover_title_folder_boundaries(root, policy, batch_size, &cancel, &sender).await;
        if let Err(error) = result {
            tokio::select! { _ = cancel.cancelled() => {}, _ = sender.send(Err(error)) => {} }
        }
    });
    receiver
}

async fn discover_title_folder_boundaries(
    root: PathBuf,
    policy: TitleFolderBoundaryPolicy,
    batch_size: usize,
    cancel: &tokio_util::sync::CancellationToken,
    sender: &tokio::sync::mpsc::Sender<AppResult<Vec<TitleFolderBoundary>>>,
) -> AppResult<()> {
    if batch_size == 0 {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::BATCH_SIZE_OUT_OF_RANGE,
            "batch size must be positive",
        ));
    }
    let resolved_root = tokio::fs::canonicalize(&root)
        .await
        .map_err(|e| AppError::Repository(e.to_string()))?;
    let episodic = policy.facet != MediaFacet::Movie;
    let mut stack = vec![BoundaryFrame::open(root, 0, false, false, episodic, cancel).await?];
    let mut batch = Vec::with_capacity(batch_size.min(256));
    let mut first_io_error = None;
    while let Some(frame) = stack.last_mut() {
        if cancel.is_cancelled() || sender.is_closed() {
            return Ok(());
        }
        if let Some(error) = frame.inspection_error.take() {
            first_io_error.get_or_insert_with(|| format!("{}: {error}", frame.path.display()));
            frame.rejection_emitted = true;
            batch.push(TitleFolderBoundary {
                path: frame.path.clone(),
                rejection: Some(format!("cannot inspect title boundary: {error}")),
            });
            if batch.len() >= batch_size {
                tokio::select! { _ = cancel.cancelled() => return Ok(()), result = sender.send(Ok(std::mem::take(&mut batch))) => if result.is_err() { return Ok(()); } }
            }
        }
        let entry = match frame.entries.next_entry().await {
            Ok(entry) => entry,
            Err(error) => {
                frame.blocked = true;
                first_io_error.get_or_insert_with(|| format!("{}: {error}", frame.path.display()));
                if !frame.rejection_emitted {
                    frame.rejection_emitted = true;
                    batch.push(TitleFolderBoundary {
                        path: frame.path.clone(),
                        rejection: Some(format!("cannot inspect title boundary: {error}")),
                    });
                }
                None
            }
        };
        if let Some(entry) = entry {
            let path = entry.path();
            let depth = frame.depth + 1;
            let kind = match entry.file_type().await {
                Ok(kind) => kind,
                Err(error) => {
                    frame.blocked = true;
                    first_io_error.get_or_insert_with(|| format!("{}: {error}", path.display()));
                    batch.push(TitleFolderBoundary {
                        path,
                        rejection: Some(format!("cannot inspect title boundary: {error}")),
                    });
                    // Flush errors through the same bounded channel below.
                    if !batch.is_empty() {
                        tokio::select! { _ = cancel.cancelled() => return Ok(()), result = sender.send(Ok(std::mem::take(&mut batch))) => if result.is_err() { return Ok(()); } }
                    }
                    continue;
                }
            };
            if !kind.is_dir() && !kind.is_symlink() {
                continue;
            }
            if should_skip_library_top_level_entry(&path, true) {
                continue;
            }
            if policy.other_roots.iter().any(|other| {
                crate::stored_paths::folder_paths_match(
                    &path_to_stored_string(other),
                    &path_to_stored_string(&path),
                )
            }) {
                frame.child_titles = true;
                continue;
            }
            let owned = policy.stored_folders.iter().any(|stored| {
                crate::stored_paths::folder_paths_match(
                    &path_to_stored_string(stored),
                    &path_to_stored_string(&path),
                )
            });
            let contains_owner = policy
                .stored_folders
                .iter()
                .any(|stored| TitleFolderBoundaryPolicy::contains_stored_folder(&path, stored));
            let extra = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                LIBRARY_IGNORED_MEDIA_SUBDIR_NAMES.contains(&n.to_ascii_lowercase().as_str())
            });
            if frame.extras_pass && (!extra || owned || contains_owner) {
                continue;
            }
            if frame.depth > 0 && extra && !owned && !contains_owner {
                if frame.skip_extras {
                    continue;
                }
                // Inspect ordinary children first: only actual season media,
                // not a season-looking organizing name, establishes extras.
                if !frame.extras_pass {
                    frame.deferred_extras = true;
                    continue;
                }
            }
            // A conventional season is evidence inside its parent title, not
            // another title level. Inspect it even at the discovery limit.
            let season = episodic
                && frame.depth > 0
                && frame.depth <= MAX_UNOWNED_TITLE_DEPTH
                && !frame.season
                && !frame.part
                && conventional_season(&path)
                && !contains_owner;
            // Disc and bonus folders hold part of the parent title's media.
            // Root-level buckets with these names stay organizing folders.
            let part = frame.depth > 0
                && (frame.depth <= MAX_UNOWNED_TITLE_DEPTH || frame.season || frame.part)
                && !season
                && title_part_folder(&path)
                && !contains_owner;
            // A recorded title may own a linked folder. It is emitted, never
            // walked, and must still resolve inside this library root.
            let rejection = if kind.is_symlink() && !owned {
                Some(
                    "linked folder requires explicit ownership resolution before scanning"
                        .to_owned(),
                )
            } else if depth > MAX_UNOWNED_TITLE_DEPTH
                && !owned
                && !contains_owner
                && !season
                && !part
            {
                Some("automatic title discovery depth limit reached".to_owned())
            } else {
                match tokio::fs::canonicalize(&path).await {
                    Ok(resolved)
                        if resolved != resolved_root && resolved.starts_with(&resolved_root) =>
                    {
                        None
                    }
                    Ok(_) => Some("folder resolves outside its library root".to_owned()),
                    Err(error) => {
                        first_io_error
                            .get_or_insert_with(|| format!("{}: {error}", path.display()));
                        Some(format!("cannot inspect title boundary: {error}"))
                    }
                }
            };
            if let Some(rejection) = rejection {
                frame.blocked = true;
                batch.push(TitleFolderBoundary {
                    path,
                    rejection: Some(rejection),
                });
            } else if owned {
                frame.child_titles = true;
                batch.push(TitleFolderBoundary {
                    path,
                    rejection: contains_owner
                        .then(|| "folder contains another recorded title boundary".into()),
                });
            } else {
                match BoundaryFrame::open(path.clone(), depth, season, part, episodic, cancel).await
                {
                    Ok(mut child) => {
                        // A missing recorded descendant still owns its path;
                        // do not let discovery claim the ancestor around it.
                        child.child_titles |= contains_owner;
                        stack.push(child);
                    }
                    Err(error) => {
                        stack.last_mut().unwrap().blocked = true;
                        first_io_error
                            .get_or_insert_with(|| format!("{}: {error}", path.display()));
                        batch.push(TitleFolderBoundary {
                            path,
                            rejection: Some(format!("cannot inspect title boundary: {error}")),
                        });
                    }
                }
            }
        } else {
            if frame.deferred_extras && !frame.extras_pass && !frame.skip_extras {
                frame.extras_pass = true;
                match tokio::fs::read_dir(&frame.path).await {
                    Ok(entries) => {
                        frame.entries = entries;
                        continue;
                    }
                    Err(error) => {
                        frame.blocked = true;
                        first_io_error
                            .get_or_insert_with(|| format!("{}: {error}", frame.path.display()));
                        if !frame.rejection_emitted {
                            frame.rejection_emitted = true;
                            batch.push(TitleFolderBoundary {
                                path: frame.path.clone(),
                                rejection: Some(format!("cannot inspect title boundary: {error}")),
                            });
                        }
                    }
                }
            }
            let frame = stack.pop().unwrap();
            let Some(parent) = stack.last_mut() else {
                break;
            };
            if frame.part {
                // Part file names such as `00001` or `NCOP1` are not title
                // identities; fold presence of media only.
                parent.skip_extras |= parent.depth > 0 && frame.evidence.has_media;
                parent.evidence.has_media |= frame.evidence.has_media;
                parent.child_titles |= frame.child_titles;
                parent.blocked |= frame.blocked;
            } else if episodic && parent.depth > 0 && frame.is_episode_container_of(parent, None) {
                parent.skip_extras |= parent.depth > 0 && frame.evidence.has_media;
                parent.evidence.merge(frame.evidence);
                parent.child_titles |= frame.child_titles;
                parent.blocked |= frame.blocked;
            } else {
                let is_candidate = frame.evidence.has_media || frame.has_title_nfo;
                parent.child_titles |= is_candidate || frame.child_titles || frame.blocked;
                if is_candidate && !frame.rejection_emitted {
                    let rejection = if frame.blocked || frame.child_titles {
                        Some("folder contains another title boundary or an unreadable branch; select individual title folders".into())
                    } else if frame.identity_conflicts() {
                        Some("media identity conflicts with the proposed title folder".into())
                    } else {
                        None
                    };
                    batch.push(TitleFolderBoundary {
                        path: frame.path,
                        rejection,
                    });
                }
            }
        }
        if batch.len() >= batch_size {
            tokio::select! { _ = cancel.cancelled() => return Ok(()), result = sender.send(Ok(std::mem::take(&mut batch))) => if result.is_err() { return Ok(()); } }
        }
    }
    if !batch.is_empty() {
        tokio::select! { _ = cancel.cancelled() => return Ok(()), result = sender.send(Ok(batch)) => if result.is_err() { return Ok(()); } }
    }
    match first_io_error {
        Some(error) => Err(AppError::Repository(error)),
        None => Ok(()),
    }
}

struct TitleBoundaryContent {
    has_media: bool,
    has_child_titles: bool,
}

enum EpisodeFolderBoundaryPolicy<'a> {
    Configured(&'a [regex::Regex]),
}

impl EpisodeFolderBoundaryPolicy<'_> {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Configured(patterns) if patterns.is_empty())
    }

    fn matches(&self, name: &str) -> bool {
        let conventional = || {
            crate::library::workflow::scan_title_files::recognize_season_folder_name(name).is_some()
        };
        match self {
            // Scan claims accept conventional season names, so every guard on
            // an existing episodic title accepts them too. Movies carry no
            // episode patterns and recognise no season folders.
            Self::Configured(patterns) => {
                !patterns.is_empty()
                    && (patterns.iter().any(|pattern| pattern.is_match(name)) || conventional())
            }
        }
    }
}

async fn directory_has_direct_media(path: &Path) -> AppResult<bool> {
    Ok(count_series_loose_root_files(path).await? > 0)
}

async fn directory_has_episode_layout(
    path: &Path,
    episode_folders: &EpisodeFolderBoundaryPolicy<'_>,
) -> AppResult<bool> {
    if episode_folders.is_empty() {
        return Ok(false);
    }
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    {
        if entry
            .file_type()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
            .is_dir()
            && episode_folders.matches(&entry.file_name().to_string_lossy())
            && directory_has_direct_media(&entry.path()).await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

const MAX_TITLE_PART_NESTING: usize = 4;

async fn inspect_title_boundary_content(
    path: &Path,
    episode_folders: &EpisodeFolderBoundaryPolicy<'_>,
) -> AppResult<TitleBoundaryContent> {
    inspect_title_boundary_children(path, episode_folders, 0).await
}

/// `part_nesting` is zero for the title folder itself and counts recognised
/// disc or bonus folders below it, which fold their media into the title.
async fn inspect_title_boundary_children(
    path: &Path,
    episode_folders: &EpisodeFolderBoundaryPolicy<'_>,
    part_nesting: usize,
) -> AppResult<TitleBoundaryContent> {
    let mut content = TitleBoundaryContent {
        has_media: directory_has_direct_media(path).await?,
        has_child_titles: false,
    };
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    {
        let child = entry.path();
        if !entry
            .file_type()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?
            .is_dir()
            || should_skip_library_top_level_entry(&child, true)
            || child
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    LIBRARY_IGNORED_MEDIA_SUBDIR_NAMES.contains(&name.to_ascii_lowercase().as_str())
                })
        {
            continue;
        }
        if part_nesting < MAX_TITLE_PART_NESTING && title_part_folder(&child) {
            let part = Box::pin(inspect_title_boundary_children(
                &child,
                episode_folders,
                part_nesting + 1,
            ))
            .await?;
            content.has_media |= part.has_media;
            content.has_child_titles |= part.has_child_titles;
            continue;
        }
        if directory_has_direct_media(&child).await? {
            let season = part_nesting == 0
                && child
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| episode_folders.matches(name));
            if season {
                content.has_media = true;
            } else {
                content.has_child_titles = true;
            }
        } else if directory_has_episode_layout(&child, episode_folders).await? {
            content.has_child_titles = true;
        }
    }
    Ok(content)
}

pub(crate) async fn ensure_existing_title_boundary(
    path: &Path,
    episode_folders: &[regex::Regex],
) -> AppResult<()> {
    let content = inspect_title_boundary_content(
        path,
        &EpisodeFolderBoundaryPolicy::Configured(episode_folders),
    )
    .await?;
    if !content.has_media || content.has_child_titles {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::SELECTED_FOLDER_ORGANIZING_FOLDER_MIXED_TITLE_LAYOUT,
            "selected folder is an organizing folder or mixed title layout; select the title folder containing its media",
        ));
    }
    Ok(())
}

/// An import may reserve a destination that already holds stray files or
/// empty subfolders, but never one that holds other titles' folders.
pub(crate) async fn ensure_folder_holds_no_child_titles(
    path: &Path,
    episode_folders: &[regex::Regex],
) -> AppResult<()> {
    let content = inspect_title_boundary_content(
        path,
        &EpisodeFolderBoundaryPolicy::Configured(episode_folders),
    )
    .await?;
    if content.has_child_titles {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::FOLDER_NOT_TITLE_BOUNDARY,
            "destination folder already contains other title folders; it is an organizing folder, not a title folder",
        ));
    }
    Ok(())
}

pub(crate) async fn ensure_scanned_folder_holds_no_child_titles(
    path: &Path,
    title: &Title,
) -> AppResult<()> {
    verify_scan_boundary(
        path,
        &title.facet,
        ScanBoundaryOwner {
            name: &title.name,
            aliases: &title.aliases,
            identity: BoundaryEvidence::from_external_ids(&title.external_ids),
        },
    )
    .await
}

pub(crate) async fn verify_new_scan_title_boundary(
    path: &Path,
    title: &scryer_domain::NewTitle,
) -> AppResult<()> {
    verify_scan_boundary(
        path,
        &title.facet,
        ScanBoundaryOwner {
            name: &title.name,
            aliases: &[],
            identity: BoundaryEvidence::from_external_ids(&title.external_ids),
        },
    )
    .await
}

async fn verify_scan_boundary(
    path: &Path,
    facet: &MediaFacet,
    owner: ScanBoundaryOwner<'_>,
) -> AppResult<()> {
    let episodic = *facet != MediaFacet::Movie;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut stack =
        vec![BoundaryFrame::open(path.to_path_buf(), 0, false, false, episodic, &cancel).await?];
    while let Some(frame) = stack.last_mut() {
        if let Some(error) = frame.inspection_error.take() {
            return Err(AppError::Repository(error));
        }
        if frame.evidence.conflicts_with(&owner.identity)
            || frame.identity_conflicts()
            || BoundaryEvidence::from_name(
                &frame.path.file_name().unwrap_or_default().to_string_lossy(),
            )
            .conflicts_with(&owner.identity)
            || frame
                .local_identity
                .as_ref()
                .is_some_and(|identity| identity.conflicts_with(&owner.identity))
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::library::MEDIA_IDENTITY_CONFLICTS_PROPOSED_TITLE_FOLDER,
                "media identity conflicts with the proposed title folder",
            ));
        }
        let entry = frame
            .entries
            .next_entry()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if let Some(entry) = entry {
            let child = entry.path();
            let kind = entry
                .file_type()
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
            if should_skip_library_top_level_entry(&child, true)
                || child
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        LIBRARY_IGNORED_MEDIA_SUBDIR_NAMES
                            .contains(&name.to_ascii_lowercase().as_str())
                    })
            {
                continue;
            }
            if kind.is_symlink() {
                return Err(AppError::validation_refused(crate::error_reasons::library::LINKED_FOLDER_REQUIRES_EXPLICIT_OWNERSHIP_RESOLUTION_BEFORE_SCANNING, "linked folder requires explicit ownership resolution before scanning"));
            }
            if !kind.is_dir() {
                continue;
            }
            let depth = frame.depth + 1;
            if depth > MAX_UNOWNED_TITLE_DEPTH {
                return Err(AppError::validation_refused(
                    crate::error_reasons::library::AUTOMATIC_TITLE_DISCOVERY_DEPTH_LIMIT_REACHED,
                    "automatic title discovery depth limit reached",
                ));
            }
            let season = episodic && !frame.part && conventional_season(&child);
            let part = !season && title_part_folder(&child);
            stack.push(BoundaryFrame::open(child, depth, season, part, episodic, &cancel).await?);
        } else {
            let child = stack.pop().unwrap();
            let Some(parent) = stack.last_mut() else {
                break;
            };
            if !(child.evidence.has_media || child.has_title_nfo) {
                continue;
            }
            // Disc and bonus folders hold part of the title's own media;
            // anything else with media below one is another title.
            if child.part {
                parent.evidence.has_media |= child.evidence.has_media;
                continue;
            }
            if parent.part {
                return Err(AppError::validation_refused(crate::error_reasons::library::DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY, "destination folder contains an independent or unresolved title boundary"
                        ));
            }
            if !episodic || !child.is_episode_container_of(parent, Some(&owner)) {
                return Err(AppError::validation_refused(crate::error_reasons::library::DESTINATION_FOLDER_CONTAINS_INDEPENDENT_UNRESOLVED_TITLE_BOUNDARY, "destination folder contains an independent or unresolved title boundary"
                        ));
            }
            parent.evidence.merge(child.evidence);
        }
    }
    Ok(())
}

#[cfg(test)]
fn extract_library_query_evidence(path: &str, library_root: &str) -> LibraryQueryEvidence {
    parse_library_filename(
        &crate::library_filename_parser::LibraryFilenameParseInput::title_only(
            &stored_path_to_path_buf(path),
            Some(stored_path_to_path_buf(library_root).as_path()),
        ),
    )
    .query_evidence
}

#[cfg(test)]
pub(crate) fn extract_library_queries(
    path: &str,
    library_root: &str,
) -> (Vec<String>, Option<u32>) {
    let evidence = extract_library_query_evidence(path, library_root);
    (evidence.queries, evidence.year)
}

pub(crate) fn elapsed_ms_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
pub(crate) async fn list_child_directories(root: &Path) -> AppResult<Vec<PathBuf>> {
    Ok(crate::filesystem_walk::FilesystemWalker::new()
        .list_child_directories(root)?
        .into_iter()
        .filter(|path| !should_skip_library_top_level_entry(path, true))
        .collect())
}

pub(crate) async fn count_series_loose_root_files(root: &Path) -> AppResult<usize> {
    let mut entries = tokio::fs::read_dir(root).await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })?;
    let mut count = 0usize;

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        AppError::Repository(format!("failed to read {}: {error}", root.display()))
    })? {
        let path = entry.path();
        let file_type = entry.file_type().await.map_err(|error| {
            AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
        })?;
        if !file_type.is_file()
            || should_skip_library_top_level_entry(&path, false)
            || path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(is_ignored_media_extra_file_name)
            || !is_allowed_video_path(&path)
        {
            continue;
        }

        count = count.saturating_add(1);
    }

    Ok(count)
}

#[cfg(test)]
pub(crate) async fn list_movie_top_level_entries(
    root: &Path,
) -> AppResult<Vec<MovieTopLevelEntry>> {
    let mut receiver = stream_movie_top_level_entries_batched(root, 64).await?;
    let mut entries = Vec::new();
    while let Some(batch) = receiver.recv().await {
        entries.extend(batch?);
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

#[cfg(test)]
pub(crate) async fn stream_movie_top_level_entries_batched(
    root: &Path,
    batch_size: usize,
) -> AppResult<MovieTopLevelEntryBatchReceiver> {
    let policy = TitleFolderBoundaryPolicy {
        facet: MediaFacet::Movie,
        stored_folders: vec![],
        other_roots: vec![],
    };
    let mut boundaries =
        stream_title_folder_boundaries(root.to_path_buf(), policy, batch_size, Default::default());
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(batch) = boundaries.recv().await {
            let batch = batch.map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| MovieTopLevelEntry {
                        path: entry.path,
                        is_dir: true,
                    })
                    .collect()
            });
            if sender.send(batch).await.is_err() {
                break;
            }
        }
    });
    Ok(receiver)
}

pub(crate) fn is_ignored_library_dir_name(name: &str) -> bool {
    let trimmed = name.trim();
    let normalized = trimmed.to_ascii_lowercase();
    normalized.starts_with('.')
        || normalized.ends_with(".trickplay")
        || is_mangled_short_dir_name(trimmed)
        || LIBRARY_IGNORED_DIR_NAMES.contains(&normalized.as_str())
}

fn is_mangled_short_dir_name(name: &str) -> bool {
    let Some((stem, suffix)) = name.split_once('~') else {
        return false;
    };
    stem.len() == 6
        && !suffix.is_empty()
        && suffix.len() <= 6
        && stem.chars().all(|ch| ch.is_ascii_alphanumeric())
        && suffix.chars().all(|ch| ch.is_ascii_digit())
}

pub(crate) fn is_ignored_library_file_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    normalized == ".ds_store"
        || normalized == "thumbs.db"
        || normalized.starts_with("._")
        || normalized.starts_with(".unmanic")
}

fn normalized_library_name_words(name: &str) -> String {
    name.trim()
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn is_trailer_like_library_dir_name(name: &str) -> bool {
    let normalized = normalized_library_name_words(name);

    normalized
        .split_whitespace()
        .any(|token| token == "trailer" || token == "trailers")
}

pub(crate) fn is_ignored_media_subdir_name(name: &str) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    let normalized_words = normalized_library_name_words(name);
    LIBRARY_IGNORED_MEDIA_SUBDIR_NAMES.contains(&normalized.as_str())
        || LIBRARY_IGNORED_MEDIA_SUBDIR_NAMES.contains(&normalized_words.as_str())
        || is_trailer_like_library_dir_name(name)
}

pub(crate) fn is_ignored_media_extra_file_name(name: &str) -> bool {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    LIBRARY_IGNORED_MEDIA_EXTRA_FILE_SUFFIXES
        .iter()
        .any(|suffix| stem.ends_with(&format!("-{suffix}")))
}

pub(crate) fn should_skip_library_top_level_entry(path: &Path, is_dir: bool) -> bool {
    let Some(name) = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
    else {
        return false;
    };

    if is_dir {
        is_ignored_library_dir_name(name.as_str())
    } else {
        is_ignored_library_file_name(name.as_str())
    }
}

pub(crate) fn should_skip_library_subpath(root: &Path, path: &Path, is_dir: bool) -> bool {
    let Some(relative) = path.strip_prefix(root).ok() else {
        return false;
    };

    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        if ((components.peek().is_some() || is_dir) && is_ignored_library_dir_name(name))
            || is_ignored_library_file_name(name)
        {
            return true;
        }
    }

    false
}

pub(crate) fn should_skip_movie_library_subpath(root: &Path, path: &Path, is_dir: bool) -> bool {
    should_skip_media_library_subpath(root, path, is_dir)
}

pub(crate) fn should_skip_episodic_library_subpath(root: &Path, path: &Path, is_dir: bool) -> bool {
    should_skip_media_library_subpath(root, path, is_dir)
}

fn should_skip_media_library_subpath(root: &Path, path: &Path, is_dir: bool) -> bool {
    if should_skip_library_subpath(root, path, is_dir) {
        return true;
    }

    let Some(relative) = path.strip_prefix(root).ok() else {
        return false;
    };

    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        if components.peek().is_some() || is_dir {
            if is_ignored_media_subdir_name(name) {
                return true;
            }
        } else if is_ignored_media_extra_file_name(name) {
            return true;
        }
    }

    false
}

fn is_allowed_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.as_str()))
}

pub(crate) fn matching_movie_nfo_path(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if same_stem.is_file() {
        return Some(path_to_stored_string(&same_stem));
    }

    let parent = path.parent()?;
    let movie_nfo = parent.join("movie.nfo");
    if movie_nfo.is_file() {
        return Some(path_to_stored_string(&movie_nfo));
    }

    None
}

pub(crate) async fn matching_movie_nfo_path_async(path: &Path) -> Option<String> {
    let same_stem = path.with_extension("nfo");
    if tokio::fs::metadata(&same_stem)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(path_to_stored_string(&same_stem));
    }

    let parent = path.parent()?;
    let movie_nfo = parent.join("movie.nfo");
    if tokio::fs::metadata(&movie_nfo)
        .await
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
    {
        return Some(path_to_stored_string(&movie_nfo));
    }

    None
}

pub(crate) fn derive_movie_probe_path(
    root: &Path,
    title: &Title,
    collections: &[Collection],
) -> Option<PathBuf> {
    if let Some(folder_path) = title
        .folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(stored_path_to_path_buf)
        .filter(|path| path.starts_with(root))
    {
        return Some(folder_path);
    }

    let mut ordered_paths = collections
        .iter()
        .filter_map(|collection| collection.ordered_path.as_deref())
        .map(stored_path_to_path_buf)
        .filter(|path| path.starts_with(root))
        .collect::<Vec<_>>();
    ordered_paths.sort();
    ordered_paths.dedup();

    let first = ordered_paths.into_iter().next()?;
    if let Some(parent) = first.parent()
        && parent != root
    {
        return Some(parent.to_path_buf());
    }

    Some(first)
}

async fn compute_library_probe_signature(path: &Path) -> AppResult<(String, String)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || compute_library_probe_signature_blocking(path))
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
}

#[derive(Clone, Debug)]
struct PendingLibraryProbe {
    path: String,
    scheme: String,
    value: String,
    now: chrono::DateTime<Utc>,
    stored_probe: Option<LibraryProbeSignature>,
}

pub(crate) enum BackgroundRefreshProbeOutcome<T> {
    Unchanged,
    Changed(T),
}

async fn begin_background_refresh_probe(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
) -> AppResult<Option<PendingLibraryProbe>> {
    let path_string = path_to_stored_string(path);
    let now = Utc::now();
    let (scheme, value) = compute_library_probe_signature(path).await?;
    let stored_probe = app
        .services
        .library
        .library_probe_signatures
        .get_probe_signature(title_id)
        .await?;
    let unchanged = stored_probe.as_ref().is_some_and(|probe| {
        probe.path == path_string
            && probe.probe_signature_scheme.as_deref() == Some(scheme.as_str())
            && probe.probe_signature_value.as_deref() == Some(value.as_str())
    });

    if unchanged {
        app.services
            .library
            .library_probe_signatures
            .upsert_probe_signature(&LibraryProbeSignature {
                title_id: title_id.to_string(),
                path: path_string,
                probe_signature_scheme: Some(scheme),
                probe_signature_value: Some(value),
                last_probed_at: Some(now),
                last_changed_at: stored_probe.and_then(|probe| probe.last_changed_at),
            })
            .await?;
        return Ok(None);
    }

    Ok(Some(PendingLibraryProbe {
        path: path_string,
        scheme,
        value,
        now,
        stored_probe,
    }))
}

async fn persist_background_refresh_probe_result(
    app: &AppUseCase,
    title_id: &str,
    probe: PendingLibraryProbe,
    has_delta: bool,
) -> AppResult<()> {
    app.services
        .library
        .library_probe_signatures
        .upsert_probe_signature(&LibraryProbeSignature {
            title_id: title_id.to_string(),
            path: probe.path,
            probe_signature_scheme: Some(probe.scheme),
            probe_signature_value: Some(probe.value),
            last_probed_at: Some(probe.now),
            last_changed_at: has_delta
                .then_some(probe.now)
                .or_else(|| probe.stored_probe.and_then(|stored| stored.last_changed_at)),
        })
        .await
}

pub(crate) async fn run_background_refresh_probe_with_delta<T, Fut>(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
    scan_and_diff: Fut,
) -> AppResult<BackgroundRefreshProbeOutcome<T>>
where
    Fut: std::future::Future<Output = AppResult<(T, HashSet<String>, HashSet<String>)>>,
{
    let Some(probe) = begin_background_refresh_probe(app, title_id, path).await? else {
        return Ok(BackgroundRefreshProbeOutcome::Unchanged);
    };

    let (payload, discovered_paths, existing_paths) = scan_and_diff.await?;
    let has_delta = discovered_paths != existing_paths;
    persist_background_refresh_probe_result(app, title_id, probe, has_delta).await?;

    if has_delta {
        Ok(BackgroundRefreshProbeOutcome::Changed(payload))
    } else {
        Ok(BackgroundRefreshProbeOutcome::Unchanged)
    }
}

fn compute_library_probe_signature_blocking(path: PathBuf) -> AppResult<(String, String)> {
    let metadata = std::fs::metadata(&path).map_err(|error| {
        AppError::Repository(format!("failed to inspect {}: {error}", path.display()))
    })?;

    if metadata.is_dir() {
        let mut markers = Vec::new();
        let entries = std::fs::read_dir(&path).map_err(|error| {
            AppError::Repository(format!("failed to read {}: {error}", path.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                AppError::Repository(format!(
                    "failed to read entry in {}: {error}",
                    path.display()
                ))
            })?;
            let child_path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect filesystem entry {}: {error}",
                    child_path.display()
                ))
            })?;

            let (kind, child_metadata) = if file_type.is_dir() {
                ("dir", std::fs::metadata(&child_path).ok())
            } else if file_type.is_file() {
                ("file", std::fs::metadata(&child_path).ok())
            } else if file_type.is_symlink() {
                match std::fs::metadata(&child_path) {
                    Ok(metadata) if metadata.is_dir() => ("dir", Some(metadata)),
                    Ok(metadata) if metadata.is_file() => ("file", Some(metadata)),
                    _ => continue,
                }
            } else {
                continue;
            };

            if should_skip_library_top_level_entry(&child_path, kind == "dir") {
                continue;
            }

            let marker = child_metadata
                .as_ref()
                .map(metadata_probe_marker)
                .unwrap_or_else(|| "unknown".to_string());
            let name = child_path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            markers.push(format!("{name}|{kind}|{marker}"));
        }
        markers.sort();
        let payload = markers.join("\n");
        Ok((
            LIBRARY_PROBE_SIGNATURE_DIRECTORY_SCHEME.to_string(),
            blake3_identity_hex(HashDomain::LibraryProbe, payload),
        ))
    } else {
        let payload = metadata_probe_marker(&metadata);
        Ok((
            LIBRARY_PROBE_SIGNATURE_FILE_SCHEME.to_string(),
            blake3_identity_hex(HashDomain::LibraryProbe, payload),
        ))
    }
}

fn metadata_probe_marker(metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| format!("{}:{}", value.as_secs(), value.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_string());
    format!("{modified}|{}", metadata.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[cfg(windows)]
    #[test]
    fn stored_folder_selection_uses_windows_path_identity() {
        let root = Path::new(r"C:\Media");
        assert!(TitleFolderBoundaryPolicy::contains_stored_folder(
            root,
            Path::new(r"c:\media\Movie"),
        ));
        for excluded in [r"c:\media", r"C:\Media-other\Movie", r"D:\Media\Movie"] {
            assert!(!TitleFolderBoundaryPolicy::contains_stored_folder(
                root,
                Path::new(excluded)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn stored_folder_selection_preserves_case_sensitive_boundaries() {
        assert!(TitleFolderBoundaryPolicy::contains_stored_folder(
            Path::new("/media/"),
            Path::new("/media/./Movie"),
        ));
        assert!(!TitleFolderBoundaryPolicy::contains_stored_folder(
            Path::new("/media"),
            Path::new("/Media/Movie"),
        ));
        for excluded in ["/media", "/media-other/Movie", "/other/Movie"] {
            assert!(!TitleFolderBoundaryPolicy::contains_stored_folder(
                Path::new("/media"),
                Path::new(excluded)
            ));
        }
    }

    #[test]
    fn episode_discovery_requires_mandatory_structure() {
        use crate::library::rename::episode_folder_discovery_pattern as pattern;
        for template in [
            "{title}",
            "{tvdb_id}",
            "{title} - {tvdb_id}",
            "{?tvdb_id:Specials {tvdb_id}}",
        ] {
            let pattern = pattern(template).unwrap();
            for name in [
                "Show Alpha",
                "12345",
                "Show Alpha - 12345",
                "Specials 12345",
                "",
            ] {
                assert!(!pattern.is_match(name), "{template}: {name}");
            }
        }
        for (template, name) in [
            ("Season {season}", "Season 1"),
            ("{season:02}", "01"),
            ("Extras", "Extras"),
            ("Specials {tvdb_id}", "Specials 12345"),
        ] {
            assert!(pattern(template).unwrap().is_match(name));
        }
    }

    async fn collect_nested_boundaries(
        root: &Path,
        policy: TitleFolderBoundaryPolicy,
    ) -> Vec<TitleFolderBoundary> {
        let mut receiver =
            stream_title_folder_boundaries(root.into(), policy, 1, Default::default());
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut entries = Vec::new();
            while let Some(batch) = receiver.recv().await {
                entries.extend(batch.unwrap());
            }
            entries
        })
        .await
        .unwrap()
    }

    fn discovery_policy(facet: MediaFacet) -> TitleFolderBoundaryPolicy {
        TitleFolderBoundaryPolicy {
            facet,
            stored_folders: vec![],
            other_roots: vec![],
        }
    }

    #[tokio::test]
    async fn nested_title_nfo_only_discovery_ignores_empty_organizers() {
        let root = tempfile::tempdir().unwrap();
        assert!(read_discovery_nfo(root.path()).await.unwrap().is_none());
        #[cfg(unix)]
        {
            let link = root.path().join("sidecar-link");
            std::os::unix::fs::symlink(root.path(), &link).unwrap();
            assert!(read_discovery_nfo(&link).await.unwrap().is_none());
            std::fs::remove_file(link).unwrap();
        }
        for relative in ["Flat", "Network/A/Nested", "Mixed", "Mixed/Child"] {
            let folder = root.path().join(relative);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("tvshow.nfo"),
                "<tvshow><title>Example Show</title><tvdbid>123</tvdbid></tvshow>",
            )
            .unwrap();
        }
        for relative in [
            "Empty",
            "Invalid",
            "Oversized",
            "Oversized [Group]",
            "Season 1",
        ] {
            let folder = root.path().join(relative);
            std::fs::create_dir_all(&folder).unwrap();
            if relative == "Invalid" {
                std::fs::write(folder.join("tvshow.nfo"), "<tvshow></tvshow>").unwrap();
            }
            if relative.starts_with("Oversized") {
                let path = folder.join("tvshow.nfo");
                std::fs::write(&path, "<tvshow><title>Oversized</title></tvshow>").unwrap();
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_len(10 * 1024 * 1024 + 1)
                    .unwrap();
            }
            if relative == "Oversized [Group]" {
                std::fs::write(folder.join("Oversized.S01E01.mkv"), b"media").unwrap();
                assert!(
                    boundary_sidecar_evidence(&folder, true)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        }
        for facet in [MediaFacet::Series, MediaFacet::Anime] {
            let entries = collect_nested_boundaries(root.path(), discovery_policy(facet)).await;
            assert_eq!(entries.len(), 5);
            for relative in ["Flat", "Network/A/Nested", "Mixed/Child"] {
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry.path == root.path().join(relative)
                            && entry.rejection.is_none()),
                    "{relative}"
                );
            }
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.path == root.path().join("Mixed")
                        && entry.rejection.is_some())
            );
        }
    }

    #[tokio::test]
    async fn nested_title_conflicts_and_mixed_parent_preserve_safe_children() {
        let root = tempfile::tempdir().unwrap();
        for relative in [
            "Mixed/Mixed.S01E01.mkv",
            "Mixed/Child/Child.S01E01.mkv",
            "Wrong/Actual.S01E01.mkv",
            "Conflict/Conflict.S01E01.mkv",
            "Conflict/Other.S01E02.mkv",
            "Ids/Ids.S01E01.{tvdb-123}.mkv",
            "Ids/Ids.S01E02.{tvdb-456}.mkv",
            "Bare/Season 1/S01E01.mkv",
            "Bare/Specials/S00E01.mkv",
            "Empty/Extras/trailer.mkv",
        ] {
            write_discovery_fixture(root.path(), relative);
        }
        let entries =
            collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Series)).await;
        for relative in ["Mixed", "Ids"] {
            assert!(
                entries
                    .iter()
                    .any(|e| e.path == root.path().join(relative) && e.rejection.is_some()),
                "missing rejection: {relative}"
            );
        }
        // Name disagreement is deferred to title/episode matching; it is not an ID conflict.
        for relative in ["Mixed/Child", "Bare", "Wrong", "Conflict"] {
            assert!(
                entries
                    .iter()
                    .any(|e| e.path == root.path().join(relative) && e.rejection.is_none()),
                "missing accepted boundary: {relative}"
            );
        }
        assert!(!entries.iter().any(|e| e.path == root.path().join("Empty")));
    }

    #[tokio::test]
    async fn nested_title_decorated_folders_preserve_local_identity_precedence() {
        for (folder, file) in [
            ("Anime Scan [SubsPlease]", "Anime.Scan.S01E01.1080p.WEB-DL"),
            ("Existing Anime [BD]", "Existing.Anime.S01E01.1080p.WEB-DL"),
            (
                "Iron Vale!! [BD]",
                "Iron.Vale!!.S01E01.1080p.BluRay.x264-GRP",
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(folder);
            std::fs::create_dir(&path).unwrap();
            let evidence = BoundaryEvidence::from_name(file);
            let title = evidence.title.as_ref().expect("usable filename identity");
            std::fs::write(
                path.join("tvshow.nfo"),
                format!("<tvshow><title>{title}</title><tvdbid>123</tvdbid></tvshow>"),
            )
            .unwrap();
            assert!(!evidence.contradicts_folder(&path, true).await, "{folder}");
            let conflicting = BoundaryEvidence::from_name("Other.Show.S01E01.{tvdb-456}");
            assert!(conflicting.contradicts_folder(&path, true).await);
        }
    }

    #[tokio::test]
    async fn nested_title_sidecar_corroboration_cannot_override_identity_conflicts() {
        let root = tempfile::tempdir().unwrap();
        for (folder, file, nfo, plexmatch) in [
            (
                "Show [Group]",
                "Show.S01E01.{tvdb-456}",
                "<tvshow><title>Show</title><tvdbid>123</tvdbid></tvshow>",
                "",
            ),
            (
                "Show [Group] {tvdb-456}",
                "Show.S01E01",
                "<tvshow><title>Show</title><tvdbid>123</tvdbid></tvshow>",
                "",
            ),
            (
                "Show [Other Group]",
                "Show.S01E01.{tvdb-456}",
                "<tvshow><tvdbid>123</tvdbid></tvshow>",
                "title: Show\ntvdbid: 456\n",
            ),
        ] {
            let path = root.path().join(folder);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("tvshow.nfo"), nfo).unwrap();
            std::fs::write(path.join(".plexmatch"), plexmatch).unwrap();
            let evidence = BoundaryEvidence::from_name(file);
            assert_eq!(evidence.title.as_deref(), Some("show"));
            assert!(evidence.contradicts_folder(&path, true).await, "{folder}");
        }
        let path = root.path().join("Show [Plex Group]");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join(".plexmatch"), "title: Show\ntvdbid: 123\n").unwrap();
        let mut evidence = BoundaryEvidence::from_name("Show.S01E01.{tvdb-123}");
        assert!(!evidence.contradicts_folder(&path, true).await);
        evidence.merge(BoundaryEvidence::from_name("Other.Show.S01E02.{tvdb-123}"));
        assert!(evidence.names_differ);
        assert!(!evidence.contradicts_folder(&path, true).await);
    }

    #[tokio::test]
    async fn nested_title_movie_sidecar_allows_generic_media_and_preserves_ids() {
        let root = tempfile::tempdir().unwrap();
        let relative = "Spirited Away (2001) Vault";
        write_discovery_fixture(root.path(), &format!("{relative}/vault-movie.mkv"));
        let folder = root.path().join(relative);
        std::fs::write(
            folder.join("movie.nfo"),
            "<movie><title>Spirited Away</title><imdbid>tt0245429</imdbid></movie>",
        )
        .unwrap();
        let entries =
            collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Movie)).await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].rejection.is_none());
        assert_eq!(
            std::fs::read(folder.join("vault-movie.mkv")).unwrap(),
            b"synthetic media"
        );
        let conflict = BoundaryEvidence::from_name("Other.{imdb-tt1234567}");
        assert!(conflict.contradicts_folder(&folder, false).await);
    }

    #[tokio::test]
    async fn nested_title_folder_name_differences_are_not_identity_conflicts() {
        // A decorated movie folder and a series folder whose files use another
        // spelling or a romanised name disagree on names only, never on ids.
        let movies = tempfile::tempdir().unwrap();
        write_discovery_fixture(
            movies.path(),
            "Archive/Lantern Vale (2001) Vault/Lantern Vale (2001).mkv",
        );
        let entries =
            collect_nested_boundaries(movies.path(), discovery_policy(MediaFacet::Movie)).await;
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(
            entries[0].path,
            movies.path().join("Archive/Lantern Vale (2001) Vault")
        );
        assert!(entries[0].rejection.is_none(), "{entries:?}");
        verify_scan_boundary(
            &movies.path().join("Archive/Lantern Vale (2001) Vault"),
            &MediaFacet::Movie,
            ScanBoundaryOwner {
                name: "Lantern Vale",
                aliases: &[],
                identity: BoundaryEvidence::from_external_ids(&[scryer_domain::ExternalId::new(
                    "imdb",
                    "tt7654321",
                )]),
            },
        )
        .await
        .unwrap();

        let shows = tempfile::tempdir().unwrap();
        for relative in [
            "Copper Tide - Reprise/Copper Tide Reprise.S01E11.mkv",
            "Copper Tide - Reprise/Akagane no Shio - 05.mkv",
            "Copper Tide - Reprise/Copper Tide Reprise Shared Cour - 06.mkv",
            "Copper Tide - Reprise/Glass Meridian S03 Complete/Copper Tide Reprise.S03E01.mkv",
        ] {
            write_discovery_fixture(shows.path(), relative);
        }
        assert_eq!(
            accepted_boundaries(shows.path(), MediaFacet::Anime).await,
            [PathBuf::from("Copper Tide - Reprise")]
        );
        let aliases = ["Akagane no Shio".to_owned()];
        verify_scan_boundary(
            &shows.path().join("Copper Tide - Reprise"),
            &MediaFacet::Anime,
            ScanBoundaryOwner {
                name: "Copper Tide - Reprise",
                aliases: &aliases,
                identity: BoundaryEvidence::default(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(
                shows
                    .path()
                    .join("Copper Tide - Reprise/Akagane no Shio - 05.mkv")
            )
            .unwrap(),
            b"synthetic media"
        );
    }

    #[tokio::test]
    async fn nested_title_cour_containers_do_not_become_titles_or_swallow_siblings() {
        for reverse in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut paths = vec![
                "Studio/Show/Glass Meridian S03 Complete/Show.S03E01.mkv",
                "Studio/Show/Glass Meridian S03 Complete/Show.S03E02.mkv",
                "Studio/Other/Other.S01E01.mkv",
                "Flat/Flat.S01E01.mkv",
            ];
            if reverse {
                paths.reverse();
            }
            for path in paths {
                write_discovery_fixture(root.path(), path);
            }
            let entries =
                collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Anime)).await;
            let mut accepted: Vec<_> = entries
                .iter()
                .filter(|entry| entry.rejection.is_none())
                .map(|entry| entry.path.strip_prefix(root.path()).unwrap().to_path_buf())
                .collect();
            accepted.sort();
            assert_eq!(
                accepted,
                ["Flat", "Studio/Other", "Studio/Show"].map(PathBuf::from)
            );
        }
    }

    #[tokio::test]
    async fn nested_title_sidecar_conflict_in_season_is_not_absorbed() {
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), "Show/Season 1/Show.S01E01.mkv");
        std::fs::write(
            root.path().join("Show/tvshow.nfo"),
            "<tvshow><title>Show</title><tvdbid>123</tvdbid></tvshow>",
        )
        .unwrap();
        std::fs::write(
            root.path().join("Show/Season 1/tvshow.nfo"),
            "<tvshow><title>Other</title><tvdbid>456</tvdbid></tvshow>",
        )
        .unwrap();
        let entries =
            collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Anime)).await;
        assert!(
            entries
                .iter()
                .any(|entry| entry.path == root.path().join("Show") && entry.rejection.is_some())
        );
    }

    #[tokio::test]
    async fn nested_title_repeated_ids_reject_conflicts_in_files_and_folders() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            "File Conflict",
            "Folder Conflict {tvdb-123} {tvdb-456}",
            "Safe {tvdb-123} {tvdb-123}",
        ];
        for (folder, file) in folders.iter().zip([
            "File.Conflict.S01E01.{tvdb-123}.{tvdb-456}.mkv",
            "Folder.Conflict.S01E01.{tvdb-123}.mkv",
            "Safe.S01E01.{tvdb-123}.{tvdb-123}.mkv",
        ]) {
            write_discovery_fixture(root.path(), &format!("{folder}/{file}"));
        }
        let entries =
            collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Series)).await;
        assert_eq!(entries.len(), folders.len());
        for folder in &folders[..2] {
            let boundary = entries
                .iter()
                .find(|entry| entry.path == root.path().join(folder))
                .unwrap();
            assert!(
                boundary
                    .rejection
                    .as_deref()
                    .is_some_and(|reason| { reason.contains("media identity conflicts") })
            );
        }
        assert!(entries.iter().any(|entry| {
            entry.path == root.path().join(folders[2]) && entry.rejection.is_none()
        }));
    }

    #[tokio::test]
    async fn nested_title_depth_limit_preserves_deeper_owned_titles() {
        let root = tempfile::tempdir().unwrap();
        let prefix = "A/B/C/D/E/F/G";
        write_discovery_fixture(root.path(), &format!("{prefix}/Allowed/Allowed.2024.mkv"));
        write_discovery_fixture(
            root.path(),
            &format!("{prefix}/H/Too Deep/Too.Deep.2024.mkv"),
        );
        let owned = format!("{prefix}/H/I/J/Owned");
        write_discovery_fixture(root.path(), &format!("{owned}/Arc One/file.mkv"));
        let mut policy = discovery_policy(MediaFacet::Movie);
        policy.stored_folders.push(root.path().join(&owned));
        let entries = collect_nested_boundaries(root.path(), policy).await;
        for path in [format!("{prefix}/Allowed"), owned] {
            assert!(
                entries
                    .iter()
                    .any(|e| e.path == root.path().join(path.as_str()) && e.rejection.is_none()),
                "{path}"
            );
        }
        assert!(entries.iter().any(|e| {
            e.path == root.path().join(format!("{prefix}/H/Too Deep"))
                && e.rejection
                    .as_deref()
                    .is_some_and(|r| r.contains("depth limit"))
        }));
    }

    #[tokio::test]
    async fn nested_title_cancelled_discovery_closes_without_results() {
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), "Show/Show.S01E01.mkv");
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let mut receiver = stream_title_folder_boundaries(
            root.path().into(),
            discovery_policy(MediaFacet::Series),
            1,
            cancel,
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(30), receiver.recv())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn nested_title_at_depth_limit_includes_conventional_season() {
        let root = tempfile::tempdir().unwrap();
        let title = "A/B/C/D/E/F/G/Last Show";
        write_discovery_fixture(
            root.path(),
            &format!("{title}/Season 1/Last.Show.S01E01.mkv"),
        );
        let entries =
            collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Series)).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, root.path().join(title));
        assert!(entries[0].rejection.is_none());
    }

    #[tokio::test]
    async fn nested_title_overlapping_ownership_is_not_accepted() {
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), "Parent/Child/Child.S01E01.mkv");
        let mut policy = discovery_policy(MediaFacet::Series);
        policy.stored_folders = vec![root.path().join("Parent"), root.path().join("Parent/Child")];
        let entries = collect_nested_boundaries(root.path(), policy).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, root.path().join("Parent"));
        assert!(
            entries[0]
                .rejection
                .as_deref()
                .unwrap()
                .contains("recorded title")
        );
    }

    #[tokio::test]
    async fn nested_title_missing_owned_descendant_does_not_allow_ancestor_claim() {
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), "Parent/Parent.S01E01.mkv");
        let mut policy = discovery_policy(MediaFacet::Series);
        policy
            .stored_folders
            .push(root.path().join("Parent/Missing"));
        let entries = collect_nested_boundaries(root.path(), policy).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, root.path().join("Parent"));
        assert!(entries[0].rejection.is_some());
    }

    #[tokio::test]
    async fn nested_title_entry_metadata_error_preserves_healthy_children() {
        let root = tempfile::tempdir().unwrap();
        let media = [
            "Parent/Parent.S01E01.mkv",
            "Parent/Child/Child.S01E01.mkv",
            "Sibling/Sibling.S01E01.mkv",
        ];
        for relative in media {
            write_discovery_fixture(root.path(), relative);
        }
        for relative in ["Parent", "Parent/Child"] {
            let fault = root.path().join(relative);
            let affected = fault.parent().unwrap().to_path_buf();
            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let cancel = tokio_util::sync::CancellationToken::new();
            let discovery = async {
                let result = BOUNDARY_ENTRY_METADATA_FAILURE
                    .scope(
                        fault,
                        discover_title_folder_boundaries(
                            root.path().into(),
                            discovery_policy(MediaFacet::Series),
                            1,
                            &cancel,
                            &sender,
                        ),
                    )
                    .await;
                if let Err(error) = result {
                    sender.send(Err(error)).await.unwrap();
                }
                drop(sender);
            };
            let collect = async {
                let mut entries = Vec::new();
                let mut terminal_error = None;
                while let Some(batch) = receiver.recv().await {
                    match batch {
                        Ok(batch) => {
                            assert!(terminal_error.is_none());
                            entries.extend(batch);
                        }
                        Err(error) => {
                            assert!(terminal_error.replace(error).is_none());
                        }
                    }
                }
                (entries, terminal_error)
            };
            let (_, (entries, terminal_error)) =
                tokio::time::timeout(std::time::Duration::from_secs(30), async {
                    tokio::join!(discovery, collect)
                })
                .await
                .unwrap();
            assert!(
                terminal_error
                    .expect("discovery must retain its first I/O error")
                    .to_string()
                    .contains("injected entry metadata failure")
            );
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry.path == affected)
                    .count(),
                1,
                "{relative}"
            );
            assert!(!entries.iter().any(|entry| {
                entry.path == root.path().join("Parent") && entry.rejection.is_none()
            }));
            for healthy in ["Parent/Child", "Sibling"] {
                assert!(entries.iter().any(|entry| {
                    entry.path == root.path().join(healthy) && entry.rejection.is_none()
                }));
            }
        }
        for relative in media {
            assert_eq!(
                std::fs::read(root.path().join(relative)).unwrap(),
                b"synthetic media"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nested_title_unreadable_branch_does_not_hide_healthy_sibling() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), "Blocked/Blocked.S01E01.mkv");
        write_discovery_fixture(root.path(), "Healthy/Healthy.S01E01.mkv");
        let blocked = root.path().join("Blocked");
        let original = std::fs::metadata(&blocked).unwrap().permissions();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o0)).unwrap();
        let unreadable = std::fs::read_dir(&blocked).is_err();
        let mut receiver = stream_title_folder_boundaries(
            root.path().into(),
            discovery_policy(MediaFacet::Series),
            1,
            Default::default(),
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut entries = Vec::new();
            let mut terminal_error = None;
            while let Some(batch) = receiver.recv().await {
                match batch {
                    Ok(batch) => entries.extend(batch),
                    Err(error) => terminal_error = Some(error),
                }
            }
            (entries, terminal_error)
        })
        .await;
        std::fs::set_permissions(&blocked, original).unwrap();
        let (entries, terminal_error) = result.unwrap();
        // Privileged runners can bypass filesystem permissions.
        if unreadable {
            assert!(terminal_error.is_some());
            assert!(
                entries
                    .iter()
                    .any(|e| e.path == blocked && e.rejection.is_some())
            );
        }
        assert!(
            entries
                .iter()
                .any(|e| e.path == root.path().join("Healthy") && e.rejection.is_none())
        );
    }

    fn write_discovery_fixture(root: &Path, relative: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"synthetic media").unwrap();
    }

    #[tokio::test]
    async fn nested_title_organizing_folders_never_become_episode_folders() {
        let root = tempfile::tempdir().unwrap();
        let titles = [
            "Alpha",
            "A/Alpha Two",
            "Example Network/Show Alpha",
            "Example Network/Show Beta",
            "GroupA/Release Show",
            "Other Network/Extras",
            "Season 1/Another Network/Deep Show",
            "Season Organizer/Season 1/Deep Series",
            "Season Organizer/Extras",
        ];
        for title in titles {
            let leaf = Path::new(title).file_name().unwrap().to_str().unwrap();
            write_discovery_fixture(root.path(), &format!("{title}/{leaf}.S01E01.mkv"));
        }
        let entries =
            collect_nested_boundaries(root.path(), discovery_policy(MediaFacet::Series)).await;
        assert!(entries.iter().all(|entry| entry.rejection.is_none()));
        let mut accepted: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        accepted.sort();
        let mut expected = titles.map(|title| root.path().join(title)).to_vec();
        expected.sort();
        assert_eq!(accepted, expected);
    }

    #[tokio::test]
    async fn nested_title_custom_seasons_use_corroborating_identity() {
        let root = tempfile::tempdir().unwrap();
        for relative in [
            "Network/Custom/Arc One/Custom.S01E01.mkv",
            "Network/Conventional/Season 1/Conventional.S01E01.mkv",
        ] {
            write_discovery_fixture(root.path(), relative);
        }
        for owned in [false, true] {
            let mut policy = discovery_policy(MediaFacet::Series);
            if owned {
                policy
                    .stored_folders
                    .push(root.path().join("Network/Custom"));
            }
            let entries = collect_nested_boundaries(root.path(), policy).await;
            assert!(entries.iter().any(|entry| entry.path
                == root.path().join("Network/Conventional")
                && entry.rejection.is_none()));
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.path == root.path().join("Network/Custom")
                        && entry.rejection.is_none())
            );
            assert!(
                !entries
                    .iter()
                    .any(|entry| entry.path == root.path().join("Network/Custom/Arc One"))
            );
        }
    }

    async fn accepted_boundaries(root: &Path, facet: MediaFacet) -> Vec<PathBuf> {
        let entries = collect_nested_boundaries(root, discovery_policy(facet)).await;
        assert!(
            entries.iter().all(|entry| entry.rejection.is_none()),
            "{entries:?}"
        );
        let mut accepted: Vec<_> = entries
            .into_iter()
            .map(|entry| entry.path.strip_prefix(root).unwrap().to_path_buf())
            .collect();
        accepted.sort();
        accepted
    }

    #[test]
    fn title_part_folder_names_are_recognised() {
        for name in [
            "Disc 1",
            "disc2",
            "DISK 3",
            "Disk_4",
            "CD1",
            "cd 2",
            "DVD1",
            "dvd-2",
            "BDMV",
            "VIDEO_TS",
            "STREAM",
            "CERTIFICATE",
            "NCOP",
            "nced",
            "OVA",
            "OAD",
            "ONA",
            "ona",
            "SP",
            "SPs",
        ] {
            assert!(is_title_part_folder_name(name), "{name}");
        }
        for name in [
            "Disc",
            "Discovery",
            "CD",
            "cd123",
            "Disc One",
            "Season 1",
            "Extras",
            "OVA Show",
            "Spectrum",
        ] {
            assert!(!is_title_part_folder_name(name), "{name}");
        }
    }

    #[tokio::test]
    async fn nested_title_disc_and_bonus_folders_fold_into_parent_title() {
        let movies = tempfile::tempdir().unwrap();
        for relative in [
            "Movie Alpha (2020)/Disc 1/00001.mkv",
            "Movie Alpha (2020)/Disc 2/00002.mkv",
            "Movie Beta (2021)/CD1/part.avi",
            "Movie Gamma (2022)/BDMV/STREAM/00001.m2ts",
            "Collection/Movie Delta (2023)/Disc 1/NCOP1.mkv",
        ] {
            write_discovery_fixture(movies.path(), relative);
        }
        assert_eq!(
            accepted_boundaries(movies.path(), MediaFacet::Movie).await,
            [
                "Collection/Movie Delta (2023)",
                "Movie Alpha (2020)",
                "Movie Beta (2021)",
                "Movie Gamma (2022)",
            ]
            .map(PathBuf::from)
        );

        let shows = tempfile::tempdir().unwrap();
        for relative in [
            "Show Alpha/NCOP/NCOP1.mkv",
            "Show Alpha/NCED/NCED1.mkv",
            "Show Alpha/Season 1/Show.Alpha.S01E01.mkv",
            "Show Beta/OVA/OVA1.mkv",
            "Show Beta/SPs/SP1.mkv",
            "Show Gamma/ONA/ONA1.mkv",
            "Show Gamma/Show.Gamma.S01E01.mkv",
            "OVA/Real Title/Real.Title.S01E01.mkv",
        ] {
            write_discovery_fixture(shows.path(), relative);
        }
        assert_eq!(
            accepted_boundaries(shows.path(), MediaFacet::Anime).await,
            ["OVA/Real Title", "Show Alpha", "Show Beta", "Show Gamma"].map(PathBuf::from)
        );
    }

    #[tokio::test]
    async fn existing_title_boundary_accepts_disc_and_bonus_folders() {
        let root = tempfile::tempdir().unwrap();
        for relative in [
            "Movie Alpha/Disc 1/a.mkv",
            "Movie Alpha/Disc 2/b.mkv",
            "Movie Beta/BDMV/STREAM/00001.m2ts",
            "Show Alpha/NCOP/NCOP1.mkv",
            "Show Alpha/Season 1/Show.Alpha.S01E01.mkv",
            "Mixed/Disc 1/Other Title/other.mkv",
        ] {
            write_discovery_fixture(root.path(), relative);
        }
        let patterns =
            [
                crate::library::rename::episode_folder_discovery_pattern("Season {season}")
                    .unwrap(),
            ];
        for (relative, episode_folders) in [
            ("Movie Alpha", &[][..]),
            ("Movie Beta", &[][..]),
            ("Show Alpha", &patterns[..]),
        ] {
            ensure_existing_title_boundary(&root.path().join(relative), episode_folders)
                .await
                .unwrap_or_else(|error| panic!("{relative}: {error}"));
            verify_scan_boundary(
                &root.path().join(relative),
                &MediaFacet::Anime,
                ScanBoundaryOwner {
                    name: relative,
                    aliases: &[],
                    identity: BoundaryEvidence::default(),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{relative}: {error}"));
        }
        assert!(
            ensure_existing_title_boundary(&root.path().join("Mixed"), &[])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn existing_title_boundary_accepts_conventional_season_with_other_template() {
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), "Show/Series 1/Show.S01E01.mkv");
        write_discovery_fixture(
            root.path(),
            "Organizer/Show Two/Series 1/Show.Two.S01E01.mkv",
        );
        let patterns =
            [
                crate::library::rename::episode_folder_discovery_pattern("Arc {season:00}")
                    .unwrap(),
            ];
        ensure_existing_title_boundary(&root.path().join("Show"), &patterns)
            .await
            .unwrap();
        ensure_folder_holds_no_child_titles(&root.path().join("Show"), &patterns)
            .await
            .unwrap();
        assert!(
            ensure_existing_title_boundary(&root.path().join("Organizer"), &patterns)
                .await
                .is_err()
        );
        // Movies recognise no season folders.
        assert!(
            ensure_existing_title_boundary(&root.path().join("Show"), &[])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn nested_title_owned_layouts_survive_template_changes() {
        let root = tempfile::tempdir().unwrap();
        for relative in ["Series/S01/episode.mkv", "Movie/Disc 1/movie.mkv"] {
            let path = root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"owned media").unwrap();
        }
        let policy = TitleFolderBoundaryPolicy {
            facet: MediaFacet::Series,
            stored_folders: vec![root.path().join("Series"), root.path().join("Movie")],
            other_roots: vec![],
        };
        let mut receiver =
            stream_title_folder_boundaries(root.path().into(), policy, 1, Default::default());
        let accepted = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut accepted = Vec::new();
            while let Some(batch) = receiver.recv().await {
                for entry in batch.unwrap() {
                    assert!(entry.rejection.is_none());
                    accepted.push(entry.path);
                }
            }
            accepted
        })
        .await
        .unwrap();
        assert_eq!(accepted.len(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nested_title_linked_folder_is_reported_without_walking_target() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("movie.mkv"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("Linked")).unwrap();
        let policy = TitleFolderBoundaryPolicy {
            facet: MediaFacet::Movie,
            stored_folders: vec![],
            other_roots: vec![],
        };
        let mut receiver =
            stream_title_folder_boundaries(root.path().into(), policy, 1, Default::default());
        let batch = tokio::time::timeout(std::time::Duration::from_secs(30), receiver.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(batch.len(), 1);
        assert!(
            batch[0]
                .rejection
                .as_deref()
                .unwrap()
                .contains("linked folder")
        );
        assert_eq!(
            std::fs::read(outside.path().join("movie.mkv")).unwrap(),
            b"outside"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nested_title_owned_linked_folder_is_emitted_without_rejection() {
        let root = tempfile::tempdir().unwrap();
        write_discovery_fixture(root.path(), ".store/Linked Target/Linked.S01E01.mkv");
        let link = root.path().join("Network/Linked Show");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(root.path().join(".store/Linked Target"), &link).unwrap();
        for owned in [true, false] {
            let mut policy = discovery_policy(MediaFacet::Series);
            if owned {
                policy.stored_folders.push(link.clone());
            }
            let entries = collect_nested_boundaries(root.path(), policy).await;
            assert_eq!(entries.len(), 1, "{entries:?}");
            assert_eq!(entries[0].path, link);
            if owned {
                assert!(entries[0].rejection.is_none(), "{entries:?}");
            } else {
                assert!(
                    entries[0]
                        .rejection
                        .as_deref()
                        .is_some_and(|reason| reason.contains("linked folder"))
                );
            }
        }
        assert_eq!(
            std::fs::read(root.path().join(".store/Linked Target/Linked.S01E01.mkv")).unwrap(),
            b"synthetic media"
        );
    }

    #[tokio::test]
    async fn nested_title_boundaries_keep_siblings_separate_and_stored_depth_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let paths = [
            "E/Emberfall/Emberfall.2024.mkv",
            "E/Echo/Echo.2023.mkv",
            "Legacy/movie.mkv",
            "Flat/Flat.2022.mkv",
        ];
        let mut before = Vec::new();
        for path in paths {
            let path = root.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"fixture media").unwrap();
            before.push((path.clone(), blake3::hash(&std::fs::read(path).unwrap())));
        }
        let policy = TitleFolderBoundaryPolicy {
            facet: MediaFacet::Movie,
            stored_folders: vec![root.path().join("Legacy")],
            other_roots: vec![],
        };
        let mut receiver =
            stream_title_folder_boundaries(root.path().into(), policy, 1, Default::default());
        let results = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut results = Vec::new();
            while let Some(batch) = receiver.recv().await {
                results.extend(batch.unwrap());
            }
            results
        })
        .await
        .unwrap();
        let mut accepted = results
            .iter()
            .filter(|entry| entry.rejection.is_none())
            .map(|entry| entry.path.strip_prefix(root.path()).unwrap().to_path_buf())
            .collect::<Vec<_>>();
        accepted.sort();
        assert_eq!(
            accepted,
            vec![
                PathBuf::from("E/Echo"),
                PathBuf::from("E/Emberfall"),
                PathBuf::from("Flat"),
                PathBuf::from("Legacy")
            ]
        );
        assert!(results.iter().all(|entry| entry.rejection.is_none()));
        for (path, hash) in before {
            assert_eq!(blake3::hash(&std::fs::read(path).unwrap()), hash);
        }
    }

    #[tokio::test]
    async fn nested_title_boundaries_use_conventional_seasons_and_skip_other_roots() {
        let root = tempfile::tempdir().unwrap();
        for path in [
            "E/Emberfall/Series 01/Emberfall.S01E01.mkv",
            "E/Echo/Special Episodes/Echo.S00E01.mkv",
            "Other/Hidden/movie.mkv",
        ] {
            let path = root.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"media").unwrap();
        }
        let policy = TitleFolderBoundaryPolicy {
            facet: MediaFacet::Series,
            stored_folders: vec![],
            other_roots: vec![root.path().join("Other")],
        };
        let mut receiver =
            stream_title_folder_boundaries(root.path().into(), policy, 1, Default::default());
        let mut accepted = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while let Some(batch) = receiver.recv().await {
                accepted.extend(
                    batch
                        .unwrap()
                        .into_iter()
                        .filter(|entry| entry.rejection.is_none())
                        .map(|entry| entry.path),
                );
            }
        })
        .await
        .unwrap();
        accepted.sort();
        assert_eq!(
            accepted,
            vec![root.path().join("E/Echo"), root.path().join("E/Emberfall")]
        );
    }

    #[test]
    fn library_title_walk_extracts_simple_title_year_and_ids() {
        let walk = library_title_walk(
            "Correct Movie (2024) [imdb:(tt6263850)] {tmdbid=12345} tvdb://67890 2160p",
        )
        .expect("title walk");

        assert_eq!(walk.title.as_deref(), Some("Correct Movie"));
        assert_eq!(walk.year, Some(2024));
        assert_eq!(walk.imdb_id.as_deref(), Some("tt6263850"));
        assert_eq!(walk.tmdb_id.as_deref(), Some("12345"));
        assert_eq!(walk.tvdb_id.as_deref(), Some("67890"));
    }

    #[test]
    fn library_title_walk_rejects_numeric_only_imdb_values() {
        let walk = library_title_walk("Mislabelled Movie (2024) [imdbid=438631]")
            .expect("title/year should still parse");

        assert_eq!(walk.title.as_deref(), Some("Mislabelled Movie"));
        assert_eq!(walk.year, Some(2024));
        assert_eq!(walk.imdb_id, None);
    }

    #[test]
    fn library_title_walk_preserves_max_inside_title() {
        let walk = library_title_walk("Sand Kettle Fury Road (2015) 2160p").expect("title walk");

        assert_eq!(walk.title.as_deref(), Some("Sand Kettle Fury Road"));
        assert_eq!(walk.year, Some(2015));
    }

    #[test]
    fn library_title_walk_extracts_tvdb_uri_from_series_folder() {
        let walk = library_title_walk("Fathomline (2021) tvdb://366972").expect("title walk");

        assert_eq!(walk.title.as_deref(), Some("Fathomline"));
        assert_eq!(walk.year, Some(2021));
        assert_eq!(walk.tvdb_id.as_deref(), Some("366972"));
    }

    #[test]
    fn ignored_library_dir_name_skips_mangled_short_names() {
        for name in ["D061DC~9", "ABC123~1", "abcdef~12"] {
            assert!(
                is_ignored_library_dir_name(name),
                "{name} should be ignored as a mangled short name"
            );
        }

        for name in ["Canonical Show", "Season 01", "Show~Archive", "ABCDEF"] {
            assert!(
                !is_ignored_library_dir_name(name),
                "{name} should remain a valid library directory"
            );
        }
    }

    #[tokio::test]
    async fn list_child_directories_skips_library_junk_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::create_dir_all(dir.path().join("Show A"))
            .await
            .expect("show a");
        tokio::fs::create_dir_all(dir.path().join("Canonical Show"))
            .await
            .expect("canonical show dir");
        tokio::fs::create_dir_all(dir.path().join("D061DC~9"))
            .await
            .expect("mangled short name dir");
        tokio::fs::create_dir_all(dir.path().join("@eaDir"))
            .await
            .expect("@eaDir");
        tokio::fs::create_dir_all(dir.path().join(".stfolder"))
            .await
            .expect(".stfolder");
        tokio::fs::create_dir_all(dir.path().join("Show A.trickplay"))
            .await
            .expect(".trickplay");

        let child_dirs = list_child_directories(dir.path())
            .await
            .expect("child dirs");

        assert_eq!(
            child_dirs,
            vec![dir.path().join("Canonical Show"), dir.path().join("Show A"),]
        );
    }

    #[tokio::test]
    async fn list_movie_top_level_entries_skips_junk_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_discovery_fixture(dir.path(), "Movie A/Movie.A.2024.mkv");
        write_discovery_fixture(dir.path(), "Extras/Extras.2024.mkv");
        write_discovery_fixture(dir.path(), "Only Extras/Only.Extras.2024-trailer.mkv");
        std::fs::create_dir_all(dir.path().join("Empty")).unwrap();
        tokio::fs::create_dir_all(dir.path().join("Movie A"))
            .await
            .expect("movie dir");
        tokio::fs::create_dir_all(dir.path().join("@eaDir"))
            .await
            .expect("@eaDir");
        tokio::fs::create_dir_all(dir.path().join("Extras"))
            .await
            .expect("extras movie dir");
        tokio::fs::write(dir.path().join("Movie.B.2024.mkv"), b"video")
            .await
            .expect("movie file");
        tokio::fs::write(dir.path().join("Movie.B.2024-trailer.mkv"), b"video")
            .await
            .expect("movie trailer file");
        tokio::fs::write(dir.path().join(".DS_Store"), b"junk")
            .await
            .expect(".DS_Store");

        let entries = list_movie_top_level_entries(dir.path())
            .await
            .expect("movie entries");

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string())
                .collect::<Vec<_>>(),
            vec!["Extras".to_string(), "Movie A".to_string(),]
        );
    }

    #[tokio::test]
    async fn stream_movie_top_level_entries_batched_skips_extra_suffix_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_discovery_fixture(dir.path(), "Movie A/Movie.A.2024.mkv");
        write_discovery_fixture(dir.path(), "Only Extras/Only.Extras.2024-trailer.mkv");
        tokio::fs::create_dir_all(dir.path().join("Movie A"))
            .await
            .expect("movie dir");
        tokio::fs::write(dir.path().join("Movie.B.2024.mkv"), b"video")
            .await
            .expect("movie file");
        tokio::fs::write(dir.path().join("Movie.B.2024-trailer.mkv"), b"video")
            .await
            .expect("movie trailer file");

        let mut receiver = stream_movie_top_level_entries_batched(dir.path(), 1)
            .await
            .expect("movie entry stream");
        let mut entries = Vec::new();
        while let Some(batch) = receiver.recv().await {
            entries.extend(batch.expect("movie entry batch"));
        }

        entries.sort_by(|left, right| left.path.cmp(&right.path));
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string())
                .collect::<Vec<_>>(),
            vec!["Movie A".to_string()]
        );
    }

    #[test]
    fn should_skip_movie_library_subpath_allows_sample_leaf_files() {
        let root = Path::new("/library");
        let path = Path::new("/library/Movie Title/Sample.2024.BluRay.mkv");

        assert!(!should_skip_movie_library_subpath(root, path, false));
    }

    #[test]
    fn should_skip_movie_library_subpath_skips_extra_suffix_leaf_files() {
        let root = Path::new("/library");

        for path in [
            Path::new("/library/Movie Title/Movie.Title-trailer.mkv"),
            Path::new("/library/Movie Title/Movie.Title-other.mkv"),
            Path::new("/library/Movie Title/Movie.Title-behindthescenes.mkv"),
            Path::new("/library/Movie Title/Movie.Title-deleted.mkv"),
            Path::new("/library/Movie Title/Movie.Title-featurette.mkv"),
            Path::new("/library/Movie Title/Movie.Title-interview.mkv"),
            Path::new("/library/Movie Title/Movie.Title-scene.mkv"),
            Path::new("/library/Movie Title/Movie.Title-short.mkv"),
        ] {
            assert!(
                should_skip_movie_library_subpath(root, path, false),
                "expected extra suffix file to be skipped: {}",
                path.display()
            );
        }
    }

    #[test]
    fn should_skip_movie_library_subpath_skips_sonarr_radarr_extra_subdirectories() {
        let root = Path::new("/library");

        for path in [
            Path::new("/library/Movie Title/extras/foo.mkv"),
            Path::new("/library/Movie Title/extrafanart/foo.mkv"),
            Path::new("/library/Movie Title/backdrops/foo.mkv"),
            Path::new("/library/Movie Title/behind the scenes/foo.mkv"),
            Path::new("/library/Movie Title/deleted scenes/foo.mkv"),
            Path::new("/library/Movie Title/featurette/foo.mkv"),
            Path::new("/library/Movie Title/featurettes/foo.mkv"),
            Path::new("/library/Movie Title/interview/foo.mkv"),
            Path::new("/library/Movie Title/interviews/foo.mkv"),
            Path::new("/library/Movie Title/other/foo.mkv"),
            Path::new("/library/Movie Title/scene/foo.mkv"),
            Path::new("/library/Movie Title/scenes/foo.mkv"),
            Path::new("/library/Movie Title/sample/foo.mkv"),
            Path::new("/library/Movie Title/samples/foo.mkv"),
            Path::new("/library/Movie Title/short/foo.mkv"),
            Path::new("/library/Movie Title/shorts/foo.mkv"),
            Path::new("/library/Movie Title/theme.music/foo.mkv"),
            Path::new("/library/Movie Title/theme music/foo.mkv"),
            Path::new("/library/Movie Title/theme-music/foo.mkv"),
            Path::new("/library/Movie Title/theme_music/foo.mkv"),
            Path::new("/library/Movie Title/Trailers/foo.mkv"),
            Path::new("/library/Movie Title/Movie Trailers/foo.mkv"),
            Path::new("/library/Movie Title/12 Tides a Shore (Trailers)/foo.mkv"),
        ] {
            assert!(
                should_skip_movie_library_subpath(root, path, false),
                "expected movie extra folder to be skipped: {}",
                path.display()
            );
        }
    }

    #[test]
    fn should_skip_episodic_library_subpath_skips_sonarr_radarr_extra_subdirectories() {
        let root = Path::new("/library/Anime Show");

        for path in [
            Path::new("/library/Anime Show/extras/foo.mkv"),
            Path::new("/library/Anime Show/Featurettes/foo.mkv"),
            Path::new("/library/Anime Show/Movie Trailers/foo.mkv"),
            Path::new("/library/Anime Show/12 Tides a Shore (Trailers)/foo.mkv"),
            Path::new("/library/Anime Show/theme.music/foo.mkv"),
            Path::new("/library/Anime Show/theme-music/foo.mkv"),
            Path::new("/library/Anime Show/theme_music/foo.mkv"),
        ] {
            assert!(
                should_skip_episodic_library_subpath(root, path, false),
                "expected episodic extra folder to be skipped: {}",
                path.display()
            );
        }
    }

    #[test]
    fn should_skip_media_library_subpath_allows_root_named_extras() {
        let root = Path::new("/library/Extras");

        assert!(!should_skip_movie_library_subpath(
            root,
            Path::new("/library/Extras/Movie.Title.2024.mkv"),
            false,
        ));
        assert!(!should_skip_episodic_library_subpath(
            root,
            Path::new("/library/Extras/Season 1/Episode.S01E01.mkv"),
            false,
        ));
        assert!(should_skip_episodic_library_subpath(
            root,
            Path::new("/library/Extras/Season 1/Extras/Bonus.mkv"),
            false,
        ));
    }

    #[test]
    fn should_skip_episodic_library_subpath_allows_sample_leaf_files() {
        let root = Path::new("/library/Anime Show");
        let path = Path::new("/library/Anime Show/Sample.2024.BluRay.mkv");

        assert!(!should_skip_episodic_library_subpath(root, path, false));
    }

    #[test]
    fn should_skip_episodic_library_subpath_skips_extra_suffix_leaf_files() {
        let root = Path::new("/library/Anime Show");
        let path = Path::new("/library/Anime Show/Episode.S01E01-trailer.mkv");

        assert!(should_skip_episodic_library_subpath(root, path, false));
    }

    #[test]
    fn should_skip_library_subpath_for_trickplay_directories() {
        let root = Path::new("/library");
        let path = Path::new("/library/Show Name/Show.Name.S01E01.trickplay");

        assert!(should_skip_library_subpath(root, path, true));
    }

    #[test]
    fn should_skip_arr_special_and_recycle_directories() {
        for path in [
            Path::new("/library/$RECYCLE.BIN"),
            Path::new("/library/#recycle"),
            Path::new("/library/recycler"),
            Path::new("/library/trash"),
            Path::new("/library/.Trashes"),
            Path::new("/library/System Volume Information"),
            Path::new("/library/lost+found"),
            Path::new("/library/Windows"),
            Path::new("/library/Cache"),
            Path::new("/library/.grab"),
            Path::new("/library/.AppleDouble"),
        ] {
            assert!(
                should_skip_library_top_level_entry(path, true),
                "expected special folder to be skipped: {}",
                path.display()
            );
        }

        let root = Path::new("/library");
        for path in [
            Path::new("/library/Movie Title/$RECYCLE.BIN/foo.mkv"),
            Path::new("/library/Movie Title/#recycle/foo.mkv"),
            Path::new("/library/Movie Title/lost+found/foo.mkv"),
            Path::new("/library/Movie Title/trash/foo.mkv"),
        ] {
            assert!(
                should_skip_movie_library_subpath(root, path, false),
                "expected nested special folder to be skipped: {}",
                path.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn should_not_skip_non_utf8_top_level_entries() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = Path::new(OsStr::from_bytes(b"/library/\xFFMovie.mkv"));
        assert!(!should_skip_library_top_level_entry(path, false));
    }

    #[test]
    fn derive_movie_probe_path_ignores_stale_folder_path_from_other_root() {
        let root = Path::new("/Volumes/Media/Movies");
        let title = Title {
            id: "title-1".into(),
            name: "Existing Movie".into(),
            facet: MediaFacet::Movie,
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
            monitored: true,
            tags: vec![],
            canonical_tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
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
            movie_release_dates: None,
            folder_path: Some("/Volumes/Archive/Movies/Existing Movie".into()),
        };
        let collections = vec![Collection {
            id: "collection-1".into(),
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".into(),
            label: None,
            ordered_path: Some(
                "/Volumes/Media/Movies/Existing Movie/Existing.Movie.2024.2160p.WEB-DL.mkv".into(),
            ),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: Utc::now(),
        }];

        assert_eq!(
            derive_movie_probe_path(root, &title, &collections),
            Some(PathBuf::from("/Volumes/Media/Movies/Existing Movie"))
        );
    }
}
