//! Filesystem evidence only: catalog ownership is read anew for every claim.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use super::*;

tokio::task_local! {
    static OWNERSHIP_CONTEXT: Arc<ScanOwnershipContext>;
}

pub(crate) async fn with_scan_ownership<F: std::future::Future>(
    context: Option<Arc<ScanOwnershipContext>>,
    future: F,
) -> F::Output {
    match context {
        Some(context) => OWNERSHIP_CONTEXT.scope(context, future).await,
        None => future.await,
    }
}

pub(crate) struct ScanOwnershipContext {
    admission: Arc<tokio::sync::Semaphore>,
    cache: Mutex<PathEvidence>,
}

impl ScanOwnershipContext {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            admission: Arc::new(tokio::sync::Semaphore::new(1)),
            cache: Mutex::new(PathEvidence::default()),
        })
    }

    pub(crate) fn current() -> Option<Arc<Self>> {
        OWNERSHIP_CONTEXT.try_with(Arc::clone).ok()
    }

    pub(super) async fn check(
        self: Arc<Self>,
        app: &AppUseCase,
        title: &Title,
        candidate: String,
    ) -> AppResult<()> {
        // Admission precedes reads and dispatch. A cancelled waiter cannot leave
        // an unbounded queue of blocking tasks or reuse an old ownership result.
        let permit = self
            .admission
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| {
                AppError::Repository(format!("ownership admission failed: {error}"))
            })?;
        let owners: Vec<_> = app
            .services
            .catalog
            .titles
            .list_delete_preview_info()
            .await?
            .into_iter()
            .filter(|owner| owner.title_id != title.id)
            .filter_map(|owner| {
                owner
                    .folder_path
                    .filter(|path| !path.is_empty())
                    .map(|path| (owner.title_id, path))
            })
            .collect();
        // Roots come from the scan's snapshot when one is in scope, the same
        // view the root and library-boundary checks use for this claim.
        let roots: Vec<_> = match ScanFolderRoots::current_all() {
            Some(roots) => roots,
            None => app
                .all_library_root_folders()
                .await?
                .into_iter()
                .map(|root| root.path)
                .collect(),
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let started = Instant::now();
            let mut cache = self.cache.lock().map_err(|_| AppError::Repository("ownership evidence lock poisoned".into()))?;
            let result = (|| {
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots)?;
                cache.set_roots(&keys.roots);
                let mut parents = HashMap::new();
                for (id, path) in owners {
                    let compare = keys.needs_exact_comparison_with_evidence(&path, &mut |path| cache.verify(path, &mut parents));
                    cache.probes += keys.link_probes;
                    keys.link_probes = 0;
                    if !compare? {
                        continue;
                    }
                    cache.fallbacks += 1;
                    let overlaps = title_folders_overlap(&candidate, &path).map_err(|error| {
                        AppError::validation_refused(crate::error_reasons::folder_ownership::CANNOT_COMPARE_FOLDER_FOLDER_OWNED_TITLE,
                            format!("cannot compare folder {candidate} with folder {path} owned by title {id}: {error}"))
                    })?;
                    let spans_root = matches!(crate::title_folder_rules::title_folder_root_violation(&path, &roots, &roots),
                        Some(crate::title_folder_rules::TitleFolderRootViolation::EqualsRoot | crate::title_folder_rules::TitleFolderRootViolation::ContainsRoot));
                    if overlaps && !spans_root {
                        return Err(AppError::validation_refused(crate::error_reasons::folder_ownership::FOLDER_OVERLAPS_FOLDER_OWNED_TITLE,
                            format!("folder {candidate} overlaps the folder owned by title {id}")));
                    }
                }
                Ok(())
            })();
            cache.checks += 1;
            cache.elapsed += started.elapsed();
            result
        }).await.map_err(|error| AppError::Repository(format!("ownership verification task failed to join: {error}")))?
    }
}

impl Drop for ScanOwnershipContext {
    fn drop(&mut self) {
        if let Ok(cache) = self.cache.lock() {
            tracing::debug!(
                checks = cache.checks,
                hits = cache.hits,
                invalidations = cache.invalidations,
                fallback_checks = cache.fallbacks,
                evidence_probes = cache.probes,
                elapsed_ms = cache.elapsed.as_millis(),
                entries = cache.directories.len(),
                estimated_cache_bytes = cache.estimated_bytes(),
                "scan ownership evidence summary"
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

fn stamp(metadata: &std::fs::Metadata) -> Option<Stamp> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Absent identity/change markers and coarse timestamps cannot prove
        // freshness between rapid claims. Keep those filesystems on fallback.
        (metadata.is_dir()
            && metadata.ino() != 0
            && metadata.ctime() != 0
            && metadata.ctime_nsec() != 0)
            .then(|| Stamp {
                device: metadata.dev(),
                inode: metadata.ino(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            })
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

#[derive(Default)]
struct PathEvidence {
    // Each proven non-link directory is bound to its containing directory's
    // identity/change stamp. Never store missing paths or linked entries.
    directories: HashMap<PathBuf, Stamp>,
    roots: Vec<(String, Option<String>)>,
    probes: u64,
    hits: u64,
    invalidations: u64,
    fallbacks: u64,
    checks: u64,
    elapsed: std::time::Duration,
}

impl PathEvidence {
    fn estimated_bytes(&self) -> usize {
        self.directories.capacity()
            * (std::mem::size_of::<PathBuf>() + std::mem::size_of::<Stamp>() + 1)
            + self
                .directories
                .keys()
                .map(|path| path.as_os_str().len())
                .sum::<usize>()
    }
    fn set_roots(&mut self, roots: &[(String, Option<String>)]) {
        if self.roots != roots {
            self.invalidations += self.directories.len() as u64;
            self.directories.clear();
            self.roots = roots.to_vec();
        }
    }

    fn verify(&mut self, path: &Path, parents: &mut HashMap<PathBuf, Option<Stamp>>) -> bool {
        if !cfg!(unix) {
            return false;
        }
        let Some(parent) = path.parent() else {
            return false;
        };
        let current = *parents.entry(parent.to_path_buf()).or_insert_with(|| {
            self.probes += 1;
            std::fs::symlink_metadata(parent)
                .ok()
                .as_ref()
                .and_then(stamp)
        });
        if let Some(previous) = self.directories.get(path) {
            if Some(*previous) == current {
                self.hits += 1;
                return true;
            }
            // The ancestor walk also verifies every intermediate component.
            // Invalidate the containing group once, rather than rescanning the
            // cache separately for every sibling after a directory change.
            let before = self.directories.len();
            self.directories
                .retain(|entry, _| !entry.starts_with(parent));
            self.invalidations += (before - self.directories.len()) as u64;
        }
        let Some(current) = current else {
            return false;
        };
        self.probes += 1;
        let verified = std::fs::symlink_metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.is_dir());
        if verified && self.directories.len() < 65_536 {
            self.directories.insert(path.to_path_buf(), current);
        }
        verified
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn compare(cache: &mut PathEvidence, root: &Path, candidate: &Path, owner: &Path) -> bool {
        let roots = vec![path_to_stored_string(root)];
        let candidate = path_to_stored_string(candidate);
        let owner = path_to_stored_string(owner);
        let mut cached = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
        cache.set_roots(&cached.roots);
        let actual = cached.needs_exact_comparison_with_evidence(&owner, &mut |path| {
            cache.verify(path, &mut HashMap::new())
        });
        let mut original = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
        let expected = original.needs_exact_comparison(&owner);
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => {
                assert_eq!(actual, expected);
                actual
            }
            (Err(actual), Err(expected)) => {
                assert_eq!(actual.validation_reason(), expected.validation_reason());
                true
            }
            _ => panic!("cached and original decisions differ"),
        }
    }

    #[test]
    fn scan_ownership_evidence_tracks_replacements_links_and_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let candidate = root.join("candidate");
        let owner = root.join("owner");
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::create_dir(&owner).unwrap();
        let sentinel = candidate.join("preserved.mkv");
        std::fs::write(&sentinel, b"untouched").unwrap();
        let mut cache = PathEvidence::default();
        assert!(!compare(&mut cache, &root, &candidate, &owner));
        assert!(!compare(&mut cache, &root, &candidate, &owner));
        let reusable = stamp(&std::fs::symlink_metadata(&root).unwrap()).is_some();
        assert_eq!(cache.hits > 0, reusable);
        assert!(compare(
            &mut cache,
            &root,
            &candidate,
            &candidate.join("child")
        ));
        assert!(compare(&mut cache, &root, &candidate, &root));
        std::fs::rename(&owner, root.join("saved-owner")).unwrap();
        symlink(&candidate, &owner).unwrap();
        assert!(compare(&mut cache, &root, &candidate, &owner));
        std::fs::remove_file(&owner).unwrap();
        symlink(root.join("saved-owner"), &owner).unwrap();
        assert!(compare(&mut cache, &root, &candidate, &owner));
        assert_eq!(cache.invalidations > 0, reusable);
        assert!(!compare(
            &mut cache,
            &root,
            &candidate,
            &root.join("missing")
        ));
        let alias = temp.path().join("alias");
        symlink(&root, &alias).unwrap();
        assert!(compare(
            &mut cache,
            &alias,
            &candidate,
            &alias.join("candidate")
        ));
        assert_eq!(std::fs::read(sentinel).unwrap(), b"untouched");
    }

    #[test]
    fn scan_ownership_evidence_rechecks_replaced_ancestors_and_inaccessible_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let candidate = root.join("candidate");
        let group = root.join("group");
        let owner = group.join("owner");
        std::fs::create_dir(&candidate).unwrap();
        std::fs::create_dir_all(&owner).unwrap();
        let mut cache = PathEvidence::default();
        assert!(!compare(&mut cache, root, &candidate, &owner));
        std::fs::rename(&group, root.join("saved-group")).unwrap();
        symlink(&candidate, &group).unwrap();
        assert!(compare(&mut cache, root, &candidate, &owner));
        let inaccessible = root.join("restricted");
        std::fs::create_dir(&inaccessible).unwrap();
        std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0)).unwrap();
        compare(&mut cache, root, &candidate, &inaccessible.join("child"));
        std::fs::set_permissions(&inaccessible, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn scan_ownership_evidence_rechecks_replaced_and_retargeted_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let candidate = root.join("candidate");
        let owner = root.join("owner");
        std::fs::create_dir_all(&candidate).unwrap();
        std::fs::create_dir(&owner).unwrap();
        let mut cache = PathEvidence::default();
        assert!(!compare(&mut cache, &root, &candidate, &owner));
        std::fs::rename(&root, temp.path().join("old-root")).unwrap();
        std::fs::create_dir_all(&candidate).unwrap();
        symlink(&candidate, &owner).unwrap();
        assert!(compare(&mut cache, &root, &candidate, &owner));
        let alias = temp.path().join("alias");
        symlink(temp.path().join("old-root"), &alias).unwrap();
        assert!(!compare(
            &mut cache,
            &alias,
            &candidate,
            &alias.join("owner")
        ));
        std::fs::remove_file(&alias).unwrap();
        symlink(&root, &alias).unwrap();
        assert!(compare(
            &mut cache,
            &alias,
            &candidate,
            &alias.join("owner")
        ));
    }

    #[tokio::test]
    async fn scan_ownership_cancelled_admission_does_not_dispatch_or_touch_files() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("preserved.mkv");
        std::fs::write(&sentinel, b"preserved").unwrap();
        let (app, actor) = crate::lib_tests::bootstrap();
        let title = app
            .create_title_without_hydration_in_library(
                &actor,
                crate::NewTitle {
                    name: "Cancellation Fixture".into(),
                    facet: scryer_domain::MediaFacet::Movie,
                    ..Default::default()
                },
                scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie),
            )
            .await
            .unwrap()
            .title;
        let context = ScanOwnershipContext::new();
        let permit = context.admission.clone().acquire_owned().await.unwrap();
        {
            let check = context
                .clone()
                .check(&app, &title, path_to_stored_string(temp.path()));
            tokio::pin!(check);
            tokio::select! {
                biased;
                _ = &mut check => panic!("ownership work must wait for admission"),
                _ = std::future::ready(()) => {}
            }
        }
        drop(permit);
        assert_eq!(context.admission.available_permits(), 1);
        assert_eq!(context.cache.lock().unwrap().checks, 0);
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserved");
    }

    async fn set_movie_roots(app: &AppUseCase, roots: &[&Path]) -> String {
        let library_id =
            scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie);
        let library = app
            .services
            .catalog
            .libraries
            .get_by_id(&library_id)
            .await
            .unwrap()
            .unwrap();
        app.services
            .catalog
            .libraries
            .update(
                &library_id,
                library.name,
                library.slug,
                roots
                    .iter()
                    .enumerate()
                    .map(|(index, root)| crate::LibraryRootDraft {
                        id: None,
                        path: path_to_stored_string(root),
                        is_default: index == 0,
                    })
                    .collect(),
            )
            .await
            .unwrap();
        library_id
    }

    #[tokio::test]
    async fn scan_ownership_claims_use_the_scan_root_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let moved_root = temp.path().join("moved-library");
        std::fs::create_dir_all(&moved_root).unwrap();
        let folders: Vec<_> = (0..3)
            .map(|index| root.join(format!("Synthetic Fixture {index} (2001)")))
            .collect();
        for folder in &folders {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("preserved.mkv"), b"preserved").unwrap();
        }
        let (app, actor) = crate::lib_tests::bootstrap();
        let library_id = set_movie_roots(&app, &[&root]).await;
        let mut titles = Vec::new();
        for index in 0..folders.len() {
            titles.push(
                app.create_title_without_hydration_in_library(
                    &actor,
                    crate::NewTitle {
                        name: format!("Synthetic Fixture {index}"),
                        facet: scryer_domain::MediaFacet::Movie,
                        ..Default::default()
                    },
                    library_id.clone(),
                )
                .await
                .unwrap()
                .title,
            );
        }
        let snapshot = ScanFolderRoots::load(&app).await.unwrap();
        let snapshot_keys =
            FolderOverlapKeys::from_roots(&path_to_stored_string(&folders[0]), &snapshot.all)
                .unwrap()
                .roots;
        // The catalog moves the library elsewhere mid-scan. Every claim in this
        // scan still answers from the snapshot, so none of them re-reads the
        // roots: a fresh read would place each folder outside its library.
        set_movie_roots(&app, &[&moved_root]).await;
        let context = ScanOwnershipContext::new();
        SCAN_FOLDER_ROOTS
            .scope(
                snapshot,
                with_scan_ownership(Some(context.clone()), async {
                    for (title, folder) in titles.iter_mut().zip(&folders) {
                        let candidate = path_to_stored_string(folder);
                        assert!(
                            !title_folder_is_outside_its_library(&app, title, &candidate)
                                .await
                                .unwrap()
                        );
                        assert!(!folder_spans_a_library_root(&app, &candidate).await.unwrap());
                        claim_title_folder_for_scan_if_missing(&app, title, folder)
                            .await
                            .unwrap();
                        assert_eq!(title.folder_path.as_deref(), Some(candidate.as_str()));
                    }
                }),
            )
            .await;
        let cache = context.cache.lock().unwrap();
        assert_eq!(cache.roots, snapshot_keys);
        assert!(cache.checks >= folders.len() as u64);
        drop(cache);
        // Outside the scan the same check reads the catalog's current roots.
        assert!(
            title_folder_is_outside_its_library(
                &app,
                &titles[0],
                &path_to_stored_string(&folders[0])
            )
            .await
            .unwrap()
        );
        for folder in &folders {
            assert_eq!(
                std::fs::read(folder.join("preserved.mkv")).unwrap(),
                b"preserved"
            );
        }
    }

    #[test]
    fn scan_ownership_flat_roots_probe_shared_parent_once_per_claim() {
        for count in [100, 10_000] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let roots = vec![path_to_stored_string(root)];
            let candidate = root.join("candidate");
            std::fs::create_dir(&candidate).unwrap();
            let candidate = path_to_stored_string(&candidate);
            let owners: Vec<_> = (0..count)
                .map(|index| {
                    let path = root.join(format!("title-{index}"));
                    std::fs::create_dir(&path).unwrap();
                    path_to_stored_string(&path)
                })
                .collect();
            let claims = 32;
            let reusable = stamp(&std::fs::symlink_metadata(root).unwrap()).is_some();
            let start = Instant::now();
            let mut original_probes = 0;
            for _ in 0..claims {
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
                for owner in &owners {
                    assert!(!keys.needs_exact_comparison(owner).unwrap());
                }
                original_probes += keys.link_probes;
            }
            let original_elapsed = start.elapsed();
            let start = Instant::now();
            let mut cache = PathEvidence::default();
            for claim in 0..claims {
                let before = cache.probes;
                let mut keys = FolderOverlapKeys::from_roots(&candidate, &roots).unwrap();
                cache.set_roots(&keys.roots);
                let mut parents = HashMap::new();
                for owner in &owners {
                    assert!(
                        !keys
                            .needs_exact_comparison_with_evidence(owner, &mut |path| cache
                                .verify(path, &mut parents))
                            .unwrap()
                    );
                }
                assert_eq!(keys.link_probes, if reusable { 0 } else { count });
                cache.probes += keys.link_probes;
                assert_eq!(
                    cache.probes - before,
                    if claim == 0 || !reusable {
                        count + 1
                    } else {
                        1
                    }
                );
            }
            assert_eq!(original_probes, count * claims);
            eprintln!(
                "ownership prefilter benchmark: titles={count} claims={claims} original_probes={original_probes} cached_probes={} original_ms={} cached_ms={} estimated_cache_bytes={}",
                cache.probes,
                original_elapsed.as_millis(),
                start.elapsed().as_millis(),
                cache.estimated_bytes()
            );
        }
    }
}
