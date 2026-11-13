//! What the rename guard against overlapping title folders reads once and
//! reuses: the folders the titles of a facet record, and the disk facts their
//! comparison spellings are built from.
//!
//! A title the rename leaves where it is gets its spelling from facts about
//! the directory holding its folder (resolved and listed once) and the case
//! behaviour of the library root it sits under (probed once), so judging a
//! plan costs no filesystem call per catalog title. A folder those facts do
//! not settle (outside every library root, under a root that cannot be
//! resolved, a link, or an entry the listing cannot match exactly) is
//! resolved on its own, as the guard always did. Folders a rename is about to
//! use are always resolved on their own.
//!
//! A bulk rename holds one context for the life of its job; any other rename
//! builds a fresh one for its own plan.
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use scryer_domain::MediaFacet;

use super::RenamePlanTitleFolder;
use crate::catalog_workflow::{library_path_is_under_root, normalize_library_root_path};
use crate::folder_ownership::{
    folder_case_insensitive, resolved_folder_identity, run_folder_check_blocking,
};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppResult, AppUseCase};

tokio::task_local! {
    static RENAME_OVERLAP_CONTEXT: Arc<RenameOverlapContext>;
}

/// Run `future` with `context` serving every overlap check it makes.
pub(super) async fn with_rename_overlap_context<F: std::future::Future>(
    context: Arc<RenameOverlapContext>,
    future: F,
) -> F::Output {
    RENAME_OVERLAP_CONTEXT.scope(context, future).await
}

/// The caller's held context, or a fresh one that lives for one check.
pub(super) fn current_rename_overlap_context() -> Arc<RenameOverlapContext> {
    RENAME_OVERLAP_CONTEXT
        .try_with(Arc::clone)
        .unwrap_or_default()
}

/// The folder one title records, trimmed and not empty.
#[derive(Debug, Clone)]
pub(super) struct RecordedTitleFolder {
    pub(super) title_id: String,
    pub(super) library_id: String,
    pub(super) folder_path: String,
}

struct HeldRecordedFolders {
    facet: MediaFacet,
    /// The catalog's folder-write counter when the folders were read.
    generation: u64,
    folders: Arc<Vec<RecordedTitleFolder>>,
}

#[derive(Default)]
pub(super) struct RenameOverlapContext {
    recorded: tokio::sync::Mutex<Option<HeldRecordedFolders>>,
    /// Taken out while a blocking pass works on it and put back after.
    disk: tokio::sync::Mutex<Option<DiskFacts>>,
    #[cfg(test)]
    recorded_reads: std::sync::atomic::AtomicUsize,
}

impl RenameOverlapContext {
    /// Every folder a title of `facet` records. The held folders are reused
    /// while the catalog's folder-write counter shows no folder write has
    /// finished since they were read, so they are what a fresh read would
    /// return; a title renamed earlier in the same job advanced the counter,
    /// and is read at its new folder. Without a counter they are read anew.
    pub(super) async fn recorded_title_folders(
        &self,
        app: &AppUseCase,
        facet: &MediaFacet,
    ) -> AppResult<Arc<Vec<RecordedTitleFolder>>> {
        let titles = &app.services.catalog.titles;
        let mut held = self.recorded.lock().await;
        // Taken before the read: a write finishing during it advances the
        // counter past this value, and the next check reads again.
        let generation = titles.title_folder_write_generation();
        if let Some(current) = held
            .as_ref()
            .filter(|held| &held.facet == facet && Some(held.generation) == generation)
        {
            return Ok(Arc::clone(&current.folders));
        }
        #[cfg(test)]
        self.recorded_reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let folders = titles
            .list_delete_preview_info()
            .await?
            .into_iter()
            .filter(|title| &title.facet == facet)
            .filter_map(|title| {
                let folder_path = title.folder_path.as_deref()?.trim().to_string();
                (!folder_path.is_empty()).then_some(RecordedTitleFolder {
                    title_id: title.title_id,
                    library_id: title.library_id,
                    folder_path,
                })
            })
            .collect::<Vec<_>>();
        let folders = Arc::new(folders);
        *held = generation.map(|generation| HeldRecordedFolders {
            facet: facet.clone(),
            generation,
            folders: Arc::clone(&folders),
        });
        Ok(folders)
    }

    /// The comparison spelling of every planned and recorded folder, by
    /// stored folder, worked out off the async runtime. A folder both planned
    /// and recorded is spelled as planned.
    pub(super) async fn comparison_spellings(
        &self,
        planned: &[RenamePlanTitleFolder],
        recorded: &Arc<Vec<RecordedTitleFolder>>,
        roots: Vec<String>,
    ) -> HashMap<String, String> {
        let mut held = self.disk.lock().await;
        let facts = held.take().unwrap_or_default();
        let planned = planned
            .iter()
            .map(|folder| folder.folder_path.clone())
            .collect::<Vec<_>>();
        let recorded = Arc::clone(recorded);
        let (facts, spellings) = run_folder_check_blocking(move || {
            let mut facts = facts;
            let mut spellings = HashMap::with_capacity(planned.len() + recorded.len());
            for raw in planned {
                if let std::collections::hash_map::Entry::Vacant(slot) = spellings.entry(raw) {
                    let spelling = facts.planned_spelling(slot.key(), &roots);
                    slot.insert(spelling);
                }
            }
            for folder in recorded.iter() {
                if !spellings.contains_key(&folder.folder_path) {
                    let spelling = facts.standing_spelling(&folder.folder_path, &roots);
                    spellings.insert(folder.folder_path.clone(), spelling);
                }
            }
            (facts, spellings)
        })
        .await;
        *held = Some(facts);
        spellings
    }

    #[cfg(test)]
    pub(super) fn recorded_reads(&self) -> usize {
        self.recorded_reads
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) async fn disk_counts(&self) -> DiskCounts {
        self.disk
            .lock()
            .await
            .as_ref()
            .map(|facts| facts.counts)
            .unwrap_or_default()
    }
}

/// How often the disk facts went to disk, for tests.
#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct DiskCounts {
    /// Library roots resolved and probed for case behaviour.
    pub(super) root_probes: usize,
    /// Directories holding a recorded folder, resolved and listed.
    pub(super) parent_reads: usize,
    /// Recorded folders resolved on their own.
    pub(super) folder_resolutions: usize,
}

#[derive(Default)]
struct DiskFacts {
    /// Whether each library root is case-insensitive, by stored root; `None`
    /// when it cannot be resolved.
    root_case: HashMap<String, Option<bool>>,
    /// Each directory holding a recorded folder, as stored; `None` when it
    /// cannot be resolved or listed.
    parents: HashMap<PathBuf, Option<ParentListing>>,
    /// Recorded folders already spelled.
    standing: HashMap<String, String>,
    #[cfg(test)]
    counts: DiskCounts,
}

impl DiskFacts {
    /// Case behaviour of the deepest library root holding `raw`, probed once
    /// per root. `None` when no root holds it or that root cannot be resolved.
    fn root_case_insensitive(&mut self, raw: &str, roots: &[String]) -> Option<bool> {
        let root = roots
            .iter()
            .filter(|root| library_path_is_under_root(raw, root))
            .max_by_key(|root| normalize_library_root_path(root).len())?;
        if let Some(known) = self.root_case.get(root) {
            return *known;
        }
        #[cfg(test)]
        {
            self.counts.root_probes += 1;
        }
        let known = resolved_folder_identity(&stored_path_to_path_buf(root))
            .ok()
            .map(|resolved| folder_case_insensitive(&resolved));
        self.root_case.insert(root.clone(), known);
        known
    }

    /// A folder a rename is about to use: resolved on disk on its own, with
    /// the case behaviour of its library root.
    fn planned_spelling(&mut self, raw: &str, roots: &[String]) -> String {
        let resolved = resolved_on_its_own(raw);
        let case_insensitive = match self.root_case_insensitive(raw, roots) {
            Some(case_insensitive) => case_insensitive,
            None => folder_case_insensitive(&resolved),
        };
        comparison_spelling(&resolved, case_insensitive)
    }

    /// A recorded folder, spelled from its directory's listing and its root's
    /// case behaviour, or on its own when those do not settle it.
    fn standing_spelling(&mut self, raw: &str, roots: &[String]) -> String {
        if let Some(spelling) = self.standing.get(raw) {
            return spelling.clone();
        }
        let spelling = match self.root_case_insensitive(raw, roots) {
            Some(case_insensitive) => {
                let resolved = match self.resolved_from_listing(raw) {
                    Some(resolved) => resolved,
                    None => {
                        #[cfg(test)]
                        {
                            self.counts.folder_resolutions += 1;
                        }
                        resolved_on_its_own(raw)
                    }
                };
                comparison_spelling(&resolved, case_insensitive)
            }
            None => {
                #[cfg(test)]
                {
                    self.counts.folder_resolutions += 1;
                }
                per_folder_spelling(raw)
            }
        };
        self.standing.insert(raw.to_string(), spelling.clone());
        spelling
    }

    /// What resolving `raw` on its own returns, from the resolved and listed
    /// directory holding it, or `None` when the listing cannot tell.
    fn resolved_from_listing(&mut self, raw: &str) -> Option<PathBuf> {
        let path = stored_path_to_path_buf(raw);
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return None;
        }
        let name = path.file_name()?.to_os_string();
        let parent = path.parent()?;
        if !self.parents.contains_key(parent) {
            #[cfg(test)]
            {
                self.counts.parent_reads += 1;
            }
            let listing = ParentListing::read(parent);
            self.parents.insert(parent.to_path_buf(), listing);
        }
        self.parents
            .get_mut(parent)?
            .as_mut()?
            .resolved_child(&name)
    }
}

/// One directory holding recorded folders: where it resolves to and what it
/// held when it was listed.
struct ParentListing {
    resolved: PathBuf,
    entries: HashMap<OsString, ListedEntry>,
    /// Every entry name, lowercased.
    folded: HashSet<String>,
}

enum ListedEntry {
    Unread(Box<std::fs::DirEntry>),
    /// Whether the entry is a directory and not a link.
    Read(bool),
}

impl ListedEntry {
    fn plain_directory(&mut self) -> bool {
        if let Self::Unread(entry) = self {
            // The listing's own type when it has one; not a follow of a link.
            let plain = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
            *self = Self::Read(plain);
        }
        matches!(self, Self::Read(true))
    }
}

impl ParentListing {
    fn read(parent: &Path) -> Option<Self> {
        let resolved = resolved_folder_identity(parent).ok()?;
        let mut entries = HashMap::new();
        let mut folded = HashSet::new();
        match std::fs::read_dir(parent) {
            Ok(listing) => {
                for entry in listing {
                    let entry = entry.ok()?;
                    let name = entry.file_name();
                    folded.insert(name.to_string_lossy().to_lowercase());
                    entries.insert(name, ListedEntry::Unread(Box::new(entry)));
                }
            }
            // A directory that does not exist yet holds nothing; resolving
            // it walks up to the ancestor that does, as a lone folder would.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        Some(Self {
            resolved,
            entries,
            folded,
        })
    }

    /// `resolved/name` when that is what resolving the child on its own
    /// gives: it is a plain directory, or nothing by any spelling of its
    /// name is there. A link, a file, or an entry that only matches with
    /// another letter case or another Unicode form is left to that.
    fn resolved_child(&mut self, name: &OsStr) -> Option<PathBuf> {
        match self.entries.get_mut(name) {
            Some(entry) => {
                if !entry.plain_directory() {
                    return None;
                }
            }
            None => {
                let ascii = name.to_str().is_some_and(|name| name.is_ascii());
                if !ascii || self.folded.contains(&name.to_string_lossy().to_lowercase()) {
                    return None;
                }
            }
        }
        Some(self.resolved.join(name))
    }
}

/// `raw` with the links in its existing prefix resolved, or as stored when
/// it cannot be resolved.
fn resolved_on_its_own(raw: &str) -> PathBuf {
    let path = stored_path_to_path_buf(raw);
    resolved_folder_identity(&path).unwrap_or(path)
}

fn comparison_spelling(resolved: &Path, case_insensitive: bool) -> String {
    let spelling = path_to_stored_string(resolved);
    if case_insensitive {
        spelling.to_lowercase()
    } else {
        spelling
    }
}

/// A folder spelled from disk on its own: resolved, and lowercased when the
/// folder itself probes case-insensitive.
pub(super) fn per_folder_spelling(raw: &str) -> String {
    let resolved = resolved_on_its_own(raw);
    let case_insensitive = folder_case_insensitive(&resolved);
    comparison_spelling(&resolved, case_insensitive)
}

#[cfg(test)]
#[path = "rename_overlap_tests.rs"]
mod tests;
