//! Storage facts for lifecycle: the configured roots a subject's files are on,
//! with each root's capacity.
//!
//! Membership is read from the file paths already in hand against the
//! configured roots, with the same boundary-aware root predicate the library
//! workflows use. It makes no filesystem call. Capacity comes only from the
//! storage capacity owner, read once per root per pass.

use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use scryer_domain::LibraryRoot;
use scryer_rules::lifecycle::{LifecycleFactsDoc, LifecycleStorageRootDoc, Observation};

use crate::catalog_workflow::library_path_is_under_root;
use crate::{AppResult, AppUseCase};

const ROOT_STAT_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
pub(crate) type TestCapacityProbe =
    Arc<dyn Fn(&str) -> Option<crate::helpers::FilesystemSpace> + Send + Sync + 'static>;

#[cfg(test)]
static TEST_CAPACITY_PROBE: OnceLock<Mutex<Option<TestCapacityProbe>>> = OnceLock::new();
#[cfg(test)]
static TEST_CAPACITY_PROBE_SERIALIZER: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

#[cfg(test)]
fn test_capacity_probe_slot() -> &'static Mutex<Option<TestCapacityProbe>> {
    TEST_CAPACITY_PROBE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn test_capacity_probe_serializer() -> Arc<tokio::sync::Semaphore> {
    TEST_CAPACITY_PROBE_SERIALIZER
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
}

/// Test-only, per-process capacity source. The guard serializes test cases that
/// opt into a scripted filesystem reading while ordinary tests keep using the
/// real capacity helper.
#[cfg(test)]
pub(crate) struct LifecycleStorageCapacityProbeGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
impl Drop for LifecycleStorageCapacityProbeGuard {
    fn drop(&mut self) {
        *test_capacity_probe_slot()
            .lock()
            .expect("lifecycle storage capacity probe lock") = None;
    }
}

#[cfg(test)]
pub(crate) async fn install_lifecycle_storage_capacity_probe_for_test(
    probe: TestCapacityProbe,
) -> LifecycleStorageCapacityProbeGuard {
    let permit = test_capacity_probe_serializer()
        .acquire_owned()
        .await
        .expect("lifecycle storage capacity probe semaphore");
    *test_capacity_probe_slot()
        .lock()
        .expect("lifecycle storage capacity probe lock") = Some(probe);
    LifecycleStorageCapacityProbeGuard { _permit: permit }
}

pub(crate) const STORAGE_ROOT_UNAVAILABLE: &str = "storage_root_unavailable";
pub(crate) const STORAGE_CAPACITY_UNAVAILABLE: &str =
    crate::storage_capacity::reason::STORAGE_CAPACITY_UNAVAILABLE;

/// The fact a rule reads storage through, at any level.
const STORAGE_ROOTS_FACT: &str = "storage_roots";

/// Whether a rule's source reads storage facts. A rule that does decides on a
/// moving condition, so its deletions are serialized and rechecked per file.
/// Any mention counts, including one in a comment: reading too much into a
/// rule only adds safety checks.
pub(crate) fn rule_reads_storage(rego_source: &str) -> bool {
    rego_source.contains(STORAGE_ROOTS_FACT)
}

/// How a pass reads a root's free space.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LifecycleStorageRead {
    /// A figure taken now through the capacity owner, bounded by a timeout.
    /// Preview and the guards that act on free space use it.
    Refresh,
    /// The capacity owner's last sample as of this instant, with no disk
    /// call. Scheduled evaluation uses it: the sampler keeps every library
    /// root current, so a pass over thousands of subjects never touches a
    /// share. A root never sampled, or sampled too long ago, is unknown.
    Sampled(chrono::DateTime<chrono::Utc>),
}

/// A configured root as the current library configuration states it.
#[derive(Clone, Debug)]
pub(crate) struct LifecycleStorageRoot {
    pub id: String,
    pub path: String,
}

/// The configured roots one pass reads storage facts against. `None` when
/// the library configuration could not be read: every storage fact is then
/// unknown for the pass.
#[derive(Clone, Debug, Default)]
pub(crate) struct LifecycleStorageRoots {
    roots: Option<Vec<LifecycleStorageRoot>>,
}

impl LifecycleStorageRoots {
    /// Roots for a pass that could not read its configuration.
    pub(crate) fn unavailable() -> Self {
        Self { roots: None }
    }

    #[cfg(test)]
    pub(crate) fn from_roots(roots: Vec<LifecycleStorageRoot>) -> Self {
        Self { roots: Some(roots) }
    }

    /// The configured root a path is under. A path under nested roots
    /// belongs to the innermost one, which is the one the file is filed in.
    fn root_of(&self, path: &str) -> Option<&LifecycleStorageRoot> {
        self.roots
            .as_deref()?
            .iter()
            .filter(|root| library_path_is_under_root(path, &root.path))
            .max_by_key(|root| root.path.len())
    }
}

/// The configured roots read at most once for one unit of work, such as a
/// candidate's execution with every safety check its steps and files run. A
/// failed read is not kept: the next check reads again.
pub(crate) type LifecycleStorageRootsOnce = tokio::sync::OnceCell<LifecycleStorageRoots>;

/// Capacity figures one pass has read, by root id, so a root is read once
/// per pass however many subjects have files on it. A failed read is
/// remembered for the pass too: the root stays unknown throughout it rather
/// than flickering between subjects.
#[derive(Clone, Debug, Default)]
pub(crate) struct LifecycleStorageCapacityMemo {
    figures: HashMap<String, Result<(i64, i64), &'static str>>,
}

/// The configured roots `files` are on, in configuration order, each once.
/// A file under no configured root adds none. `None` when the roots could not
/// be read.
fn roots_of_files<'a>(
    roots: &'a LifecycleStorageRoots,
    files: &[crate::types::TitleMediaFile],
) -> Option<Vec<&'a LifecycleStorageRoot>> {
    let configured = roots.roots.as_deref()?;
    let mut on: Vec<&LifecycleStorageRoot> = Vec::new();
    for file in files {
        if let Some(root) = roots.root_of(&file.file_path)
            && !on.iter().any(|seen| seen.id == root.id)
        {
            on.push(root);
        }
    }
    on.sort_by_key(|root| {
        configured
            .iter()
            .position(|candidate| candidate.id == root.id)
            .unwrap_or(usize::MAX)
    });
    Some(on)
}

impl AppUseCase {
    /// The configured roots, read once for a pass.
    pub(crate) async fn lifecycle_storage_roots_context(&self) -> AppResult<LifecycleStorageRoots> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(LifecycleStorageRoots {
            roots: Some(
                libraries
                    .into_iter()
                    .flat_map(|library| library.roots)
                    .filter_map(storage_root_from_library_root)
                    .collect(),
            ),
        })
    }

    /// Set `storage_roots` from `files`: one entry per configured root they
    /// are on, with its capacity. Known and empty when no file is under a
    /// configured root. Unknown when the roots could not be read, or when any
    /// root the files are on has no usable capacity figure.
    pub(crate) async fn populate_lifecycle_storage_roots(
        &self,
        facts: &mut LifecycleFactsDoc,
        files: &[crate::types::TitleMediaFile],
        roots: &LifecycleStorageRoots,
        memo: &mut LifecycleStorageCapacityMemo,
        read: LifecycleStorageRead,
    ) {
        let Some(on) = roots_of_files(roots, files) else {
            facts.storage_roots = Observation::unknown(STORAGE_ROOT_UNAVAILABLE);
            return;
        };
        let mut docs = Vec::with_capacity(on.len());
        for root in on {
            let figures = match memo.figures.get(&root.id) {
                Some(figures) => *figures,
                None => {
                    let figures = self
                        .lifecycle_storage_capacity(root.path.clone(), read)
                        .await;
                    memo.figures.insert(root.id.clone(), figures);
                    figures
                }
            };
            match figures {
                Ok((available, total)) => docs.push(LifecycleStorageRootDoc {
                    root_id: root.id.clone(),
                    path: root.path.clone(),
                    available_bytes: available,
                    total_bytes: total,
                    available_percent: (available as f64 * 100.0) / total as f64,
                }),
                Err(reason) => {
                    facts.storage_roots = Observation::unknown(reason);
                    return;
                }
            }
        }
        facts.storage_roots = Observation::known(docs);
    }
}

fn storage_root_from_library_root(root: LibraryRoot) -> Option<LifecycleStorageRoot> {
    let path = root.path.trim();
    (!path.is_empty()).then(|| LifecycleStorageRoot {
        id: root.id,
        path: path.to_string(),
    })
}

/// Keep validation separate from the system probe so tests can feed a precise
/// capacity sequence without depending on a real disk becoming full.
fn storage_capacity_values(space: Option<crate::helpers::FilesystemSpace>) -> Option<(i64, i64)> {
    let space = space?;
    let total = i64::try_from(space.total_bytes).ok()?;
    let available = i64::try_from(space.available_bytes).ok()?;
    (total > 0 && available >= 0 && available <= total).then_some((available, total))
}

impl AppUseCase {
    /// A root's figures as `(available, total)` through the canonical
    /// capacity owner, or the unknown reason. [`LifecycleStorageRead::Refresh`]
    /// is bounded by [`ROOT_STAT_TIMEOUT`] and single-flighted with every other
    /// caller asking about the same root; [`LifecycleStorageRead::Sampled`]
    /// reads the cache and never calls the disk.
    async fn lifecycle_storage_capacity(
        &self,
        path: String,
        read: LifecycleStorageRead,
    ) -> Result<(i64, i64), &'static str> {
        // A scripted figure stands in for the capacity owner in either mode,
        // as the sampler's last reading or as a fresh one.
        #[cfg(test)]
        let scripted = test_capacity_probe_slot()
            .lock()
            .expect("lifecycle storage capacity probe lock")
            .clone();
        #[cfg(test)]
        if let Some(probe) = scripted {
            let space = tokio::time::timeout(
                ROOT_STAT_TIMEOUT,
                tokio::task::spawn_blocking(move || probe(&path)),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .flatten();
            return storage_capacity_values(space).ok_or(STORAGE_CAPACITY_UNAVAILABLE);
        }
        let capacity = &self.runtime.health.storage_capacity;
        match read {
            LifecycleStorageRead::Refresh => {
                let space = capacity
                    .refresh(std::path::PathBuf::from(path), ROOT_STAT_TIMEOUT)
                    .await
                    .ok();
                storage_capacity_values(space).ok_or(STORAGE_CAPACITY_UNAVAILABLE)
            }
            LifecycleStorageRead::Sampled(now) => match capacity.reading(&path, now) {
                crate::storage_capacity::StorageCapacityReading::Known {
                    available_bytes,
                    total_bytes,
                    ..
                } => Ok((available_bytes, total_bytes)),
                other => Err(other
                    .unknown_reason()
                    .unwrap_or(STORAGE_CAPACITY_UNAVAILABLE)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LifecycleStorageRoot, LifecycleStorageRoots, roots_of_files, rule_reads_storage,
        storage_capacity_values,
    };

    fn file(path: &str) -> crate::types::TitleMediaFile {
        crate::types::TitleMediaFile {
            file_path: path.to_string(),
            ..crate::types::TitleMediaFile::default()
        }
    }

    fn roots() -> LifecycleStorageRoots {
        LifecycleStorageRoots::from_roots(vec![
            LifecycleStorageRoot {
                id: "root-fixture-a".into(),
                path: "/fixture/a".into(),
            },
            LifecycleStorageRoot {
                id: "root-fixture-b".into(),
                path: "/fixture/b".into(),
            },
            LifecycleStorageRoot {
                id: "root-fixture-b-nested".into(),
                path: "/fixture/b/nested".into(),
            },
        ])
    }

    fn ids(files: &[crate::types::TitleMediaFile]) -> Option<Vec<String>> {
        let roots = roots();
        roots_of_files(&roots, files).map(|on| on.into_iter().map(|root| root.id.clone()).collect())
    }

    #[test]
    fn files_on_two_roots_name_each_root_once_in_configuration_order() {
        assert_eq!(
            ids(&[
                file("/fixture/b/Show Fixture/S01E01.mkv"),
                file("/fixture/a/Show Fixture/S01E02.mkv"),
                file("/fixture/b/Show Fixture/S01E03.mkv"),
            ]),
            Some(vec![
                "root-fixture-a".to_string(),
                "root-fixture-b".to_string()
            ])
        );
    }

    #[test]
    fn membership_is_lexical_on_path_boundaries_and_takes_the_innermost_root() {
        // A sibling directory sharing a root's prefix is not under the root.
        assert_eq!(
            ids(&[file("/fixture/ab/Show Fixture/S01E01.mkv")]),
            Some(vec![])
        );
        assert_eq!(
            ids(&[file("/fixture/b/nested/Show Fixture/S01E01.mkv")]),
            Some(vec!["root-fixture-b-nested".to_string()])
        );
        assert!(
            roots_of_files(
                &LifecycleStorageRoots::unavailable(),
                &[file("/fixture/a/x.mkv")]
            )
            .is_none(),
            "unreadable roots are unknown, never no roots"
        );
    }

    #[test]
    fn a_file_under_no_configured_root_is_left_out_and_the_rest_stay_known() {
        for stray in [
            "/fixture/ab/Show Fixture/S01E01.mkv",
            "/elsewhere/Show Fixture/S01E01.mkv",
        ] {
            assert_eq!(
                ids(&[file("/fixture/a/Show Fixture/S01E02.mkv"), file(stray)]),
                Some(vec!["root-fixture-a".to_string()]),
                "{stray} adds no root and does not make the list unknown"
            );
        }
    }

    #[test]
    fn a_rule_reads_storage_when_it_names_the_storage_fact_at_any_level() {
        assert!(rule_reads_storage(
            "x if { some r in input.facts.storage_roots }"
        ));
        assert!(rule_reads_storage(
            "delete_season_files if { some r in input.season.facts.storage_roots }"
        ));
        assert!(!rule_reads_storage("unmonitor if input.facts.has_file"));
    }

    #[test]
    fn capacity_values_accept_a_recovered_probe_after_an_unknown_measurement() {
        let low = crate::helpers::FilesystemSpace {
            total_bytes: 1_000,
            available_bytes: 100,
        };
        let recovered = crate::helpers::FilesystemSpace {
            total_bytes: 1_000,
            available_bytes: 700,
        };
        let sequence = [Some(low), None, Some(recovered)]
            .into_iter()
            .map(storage_capacity_values)
            .collect::<Vec<_>>();
        assert_eq!(sequence, vec![Some((100, 1_000)), None, Some((700, 1_000))]);
    }
}
