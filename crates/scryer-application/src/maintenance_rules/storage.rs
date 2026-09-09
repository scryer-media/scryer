//! Configured-root storage facts and manifest filtering for maintenance.
//!
//! A storage matcher is about one configured root, never an inferred title
//! destination or a parent mount. This module therefore resolves the root by
//! its durable id, probes that exact path, and filters actual media file paths
//! with the same boundary-aware root predicate used by library workflows.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use scryer_domain::LibraryRoot;
use scryer_rules::maintenance::{MaintenanceInput, Observation};

use crate::catalog_workflow::library_path_is_under_root;
use crate::library::user_delete::PolicyMediaFileDeletePlan;
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
pub(crate) struct MaintenanceStorageCapacityProbeGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
impl Drop for MaintenanceStorageCapacityProbeGuard {
    fn drop(&mut self) {
        *test_capacity_probe_slot()
            .lock()
            .expect("maintenance storage capacity probe lock") = None;
    }
}

#[cfg(test)]
pub(crate) async fn install_maintenance_storage_capacity_probe_for_test(
    probe: TestCapacityProbe,
) -> MaintenanceStorageCapacityProbeGuard {
    let permit = test_capacity_probe_serializer()
        .acquire_owned()
        .await
        .expect("maintenance storage capacity probe semaphore");
    *test_capacity_probe_slot()
        .lock()
        .expect("maintenance storage capacity probe lock") = Some(probe);
    MaintenanceStorageCapacityProbeGuard { _permit: permit }
}

pub(crate) const STORAGE_ROOT_UNAVAILABLE: &str = "storage_root_unavailable";
pub(crate) const STORAGE_CAPACITY_UNAVAILABLE: &str = "storage_capacity_unavailable";

/// A configured root proven from the current library configuration.
#[derive(Clone, Debug)]
pub(crate) struct MaintenanceStorageRoot {
    pub id: String,
    pub path: String,
}

/// Exact root-filtered file totals for preview and deletion evidence.
#[derive(Clone, Debug, Default)]
pub(crate) struct MaintenanceRootFileSummary {
    pub file_count: Option<i64>,
    pub total_size_bytes: Option<i64>,
}

/// One evaluation pass samples a root once. The cache belongs to the caller,
/// never the application instance, so preview and execution always take a new
/// measurement while scheduled evaluation avoids inconsistent per-subject
/// readings and unbounded filesystem probes.
#[derive(Clone, Debug)]
pub(crate) struct MaintenanceStorageFactSnapshot {
    storage_root_id: Observation<String>,
    storage_available_bytes: Observation<i64>,
    storage_total_bytes: Observation<i64>,
    storage_available_percent: Observation<f64>,
}

pub(crate) type MaintenanceStorageFactCache = HashMap<String, MaintenanceStorageFactSnapshot>;

impl AppUseCase {
    /// Resolve a root from its immutable configuration ID. A missing or invalid
    /// root is an unknown observation rather than a fallback to any related
    /// title root; fallback would silently widen a destructive rule.
    pub(crate) async fn maintenance_storage_root(
        &self,
        storage_root_id: Option<&str>,
    ) -> AppResult<Option<MaintenanceStorageRoot>> {
        let Some(storage_root_id) = storage_root_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        let libraries = self.services.catalog.libraries.list(None).await?;
        Ok(libraries
            .into_iter()
            .flat_map(|library| library.roots)
            .find(|root| root.id == storage_root_id)
            .and_then(storage_root_from_library_root))
    }

    /// Immutable evidence for the configured root as it exists *now*. The
    /// checkpoint records it so a root id retargeted to another path or volume
    /// cannot resume a deletion journal made for the old location.
    pub(crate) async fn maintenance_storage_root_identity(
        &self,
        storage_root_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let Some(root) = self.maintenance_storage_root(storage_root_id).await? else {
            return Ok(None);
        };
        let path = root.path;
        Ok(tokio::time::timeout(
            ROOT_STAT_TIMEOUT,
            tokio::task::spawn_blocking(move || physical_root_identity(&path)),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .flatten())
    }

    /// Populate only the root-scoped facts. Callers deliberately build all
    /// ordinary subject facts first, because root selection neither changes
    /// title ownership nor changes the normal file facts a rule can inspect.
    pub(crate) async fn populate_maintenance_storage_facts(
        &self,
        input: &mut MaintenanceInput,
        storage_root_id: Option<&str>,
    ) -> AppResult<()> {
        let Some(storage_root_id) = storage_root_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return Ok(());
        };
        let Some(root) = self.maintenance_storage_root(Some(storage_root_id)).await? else {
            set_storage_unknown(input, STORAGE_ROOT_UNAVAILABLE);
            return Ok(());
        };

        input.facts.storage_root_id = Observation::known(root.id.clone());
        let path = root.path.clone();
        let capacity = tokio::time::timeout(
            ROOT_STAT_TIMEOUT,
            tokio::task::spawn_blocking(move || storage_capacity_probe(&path)),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
        let capacity = storage_capacity_values(capacity);
        let Some((available, total)) = capacity else {
            input.facts.storage_available_bytes =
                Observation::unknown(STORAGE_CAPACITY_UNAVAILABLE);
            input.facts.storage_total_bytes = Observation::unknown(STORAGE_CAPACITY_UNAVAILABLE);
            input.facts.storage_available_percent =
                Observation::unknown(STORAGE_CAPACITY_UNAVAILABLE);
            return Ok(());
        };
        input.facts.storage_available_bytes = Observation::known(available);
        input.facts.storage_total_bytes = Observation::known(total);
        input.facts.storage_available_percent =
            Observation::known((available as f64 * 100.0) / total as f64);
        Ok(())
    }

    /// Scheduled evaluation's bounded batch variant. A root id is the cache
    /// key even when its configuration cannot currently be resolved, so an
    /// unavailable root remains consistently unknown throughout the pass.
    pub(crate) async fn populate_maintenance_storage_facts_from_cache(
        &self,
        input: &mut MaintenanceInput,
        storage_root_id: Option<&str>,
        cache: &mut MaintenanceStorageFactCache,
    ) -> AppResult<()> {
        let Some(storage_root_id) = storage_root_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return Ok(());
        };
        if let Some(snapshot) = cache.get(storage_root_id) {
            apply_storage_snapshot(input, snapshot);
            return Ok(());
        }
        self.populate_maintenance_storage_facts(input, Some(storage_root_id))
            .await?;
        cache.insert(storage_root_id.to_string(), storage_snapshot(input));
        Ok(())
    }

    /// Count only files whose actual current location is under the selected
    /// root. `None` totals mean the root cannot be resolved, never that it has
    /// zero files.
    pub(crate) async fn maintenance_root_file_summary(
        &self,
        files: &[crate::types::TitleMediaFile],
        storage_root_id: Option<&str>,
    ) -> AppResult<MaintenanceRootFileSummary> {
        let Some(root) = self.maintenance_storage_root(storage_root_id).await? else {
            return Ok(MaintenanceRootFileSummary::default());
        };
        let Some(matching) = root_filtered_files(files, &root).await else {
            return Ok(MaintenanceRootFileSummary::default());
        };
        Ok(MaintenanceRootFileSummary {
            file_count: Some(matching.len() as i64),
            total_size_bytes: Some(matching.iter().map(|file| file.size_bytes).sum()),
        })
    }

    /// Filter a previously-authorized candidate manifest to an exact root.
    /// Callers must still fingerprint each retained file immediately before
    /// deletion; this only keeps a storage rule from authorizing another root.
    pub(crate) async fn maintenance_files_on_root(
        &self,
        files: Vec<crate::types::TitleMediaFile>,
        storage_root_id: Option<&str>,
    ) -> AppResult<Option<Vec<crate::types::TitleMediaFile>>> {
        let Some(root) = self.maintenance_storage_root(storage_root_id).await? else {
            return Ok(None);
        };
        Ok(root_filtered_files(&files, &root).await)
    }

    /// A policy media-file delete can include tracked sidecars on roots other
    /// than its primary media path. Storage pressure owns only the selected
    /// root, so all persisted plan paths must be physically proven there both
    /// when the journal is made and immediately before the shared deleter runs.
    /// `None` means that the proof was unavailable; callers must hold.
    pub(crate) async fn maintenance_policy_plan_on_root(
        &self,
        plan: &PolicyMediaFileDeletePlan,
        storage_root_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        let Some(root) = self.maintenance_storage_root(storage_root_id).await? else {
            return Ok(None);
        };
        let Some(paths) = plan.path_strings() else {
            return Ok(None);
        };
        Ok(root_path_membership(&paths, &root)
            .await
            .map(|membership| membership.into_iter().all(|on_root| on_root)))
    }

    /// Resume-only proof for a journal whose exact, previously authorized
    /// paths may already have been unlinked when a process crashed before the
    /// catalog cleanup/checkpoint update. Existing paths must still be on the
    /// selected root; a path which is conclusively absent is safe only because
    /// it is named in the persisted policy manifest and the caller separately
    /// verifies the saved root identity.
    pub(crate) async fn maintenance_policy_plan_on_root_or_absent(
        &self,
        plan: &PolicyMediaFileDeletePlan,
        storage_root_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        let Some(root) = self.maintenance_storage_root(storage_root_id).await? else {
            return Ok(None);
        };
        let Some(paths) = plan.path_strings() else {
            return Ok(None);
        };
        Ok(root_path_membership_or_absent(&paths, &root)
            .await
            .map(|membership| {
                membership
                    .into_iter()
                    .all(|on_root_or_absent| on_root_or_absent)
            }))
    }
}

fn storage_root_from_library_root(root: LibraryRoot) -> Option<MaintenanceStorageRoot> {
    let path = root.path.trim();
    (!path.is_empty()).then(|| MaintenanceStorageRoot {
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

fn storage_capacity_probe(path: &str) -> Option<crate::helpers::FilesystemSpace> {
    #[cfg(test)]
    if let Some(probe) = test_capacity_probe_slot()
        .lock()
        .expect("maintenance storage capacity probe lock")
        .clone()
    {
        return probe(path);
    }
    crate::filesystem_space(path)
}

/// Resolve physical ownership in one bounded blocking task. `None` means that
/// the root or one candidate path could not be proved, and callers must hold
/// rather than reinterpret uncertainty as an empty root-local manifest.
async fn root_filtered_files(
    files: &[crate::types::TitleMediaFile],
    root: &MaintenanceStorageRoot,
) -> Option<Vec<crate::types::TitleMediaFile>> {
    let paths: Vec<String> = files.iter().map(|file| file.file_path.clone()).collect();
    let membership = root_path_membership(&paths, root).await?;
    Some(
        files
            .iter()
            .zip(membership)
            .filter_map(|(file, on_root)| on_root.then(|| file.clone()))
            .collect(),
    )
}

async fn root_path_membership(
    paths: &[String],
    root: &MaintenanceStorageRoot,
) -> Option<Vec<bool>> {
    let root_path = root.path.clone();
    let paths = paths.to_vec();
    tokio::time::timeout(
        ROOT_STAT_TIMEOUT,
        tokio::task::spawn_blocking(move || physical_root_membership(&root_path, &paths)),
    )
    .await
    .ok()
    .and_then(Result::ok)?
}

async fn root_path_membership_or_absent(
    paths: &[String],
    root: &MaintenanceStorageRoot,
) -> Option<Vec<bool>> {
    let root_path = root.path.clone();
    let paths = paths.to_vec();
    tokio::time::timeout(
        ROOT_STAT_TIMEOUT,
        tokio::task::spawn_blocking(move || physical_root_membership_or_absent(&root_path, &paths)),
    )
    .await
    .ok()
    .and_then(Result::ok)?
}

fn physical_root_membership(root: &str, paths: &[String]) -> Option<Vec<bool>> {
    // A configured root and a catalog file can each be symlinks. The manifest
    // is authorized for the physical location, not merely a lexical prefix:
    // `/media/root/escape -> /other` must never become root-local deletion.
    let root_path = std::fs::canonicalize(root).ok()?;
    let Some(root_path) = root_path.to_str() else {
        return None;
    };
    paths
        .iter()
        .map(|path| {
            if path.trim().is_empty() || !Path::new(path).is_absolute() {
                return None;
            }
            let file_path = std::fs::canonicalize(path).ok()?;
            let file_path = file_path.to_str()?;
            if !library_path_is_under_root(file_path, root_path) {
                return Some(false);
            }
            paths_share_filesystem(root_path, file_path)
        })
        .collect()
}

/// This is intentionally distinct from [`physical_root_membership`]. General
/// candidate filtering treats a missing catalog path as unknown. Only a
/// checkpoint's immutable, exact policy paths can use an absent path as proof
/// of completed on-disk work after the root identity has been checked.
fn physical_root_membership_or_absent(root: &str, paths: &[String]) -> Option<Vec<bool>> {
    let root_path = std::fs::canonicalize(root).ok()?;
    let root_path = root_path.to_str()?;
    paths
        .iter()
        .map(|path| {
            if path.trim().is_empty() || !Path::new(path).is_absolute() {
                return None;
            }
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == ErrorKind::NotFound => Some(true),
                Err(_) => None,
                Ok(_) => {
                    let file_path = std::fs::canonicalize(path).ok()?;
                    let file_path = file_path.to_str()?;
                    if !library_path_is_under_root(file_path, root_path) {
                        return Some(false);
                    }
                    paths_share_filesystem(root_path, file_path)
                }
            }
        })
        .collect()
}

fn physical_root_identity(root: &str) -> Option<String> {
    let canonical = std::fs::canonicalize(root).ok()?;
    let canonical = canonical.to_str()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(canonical).ok()?;
        return Some(format!("{canonical}:device={}", metadata.dev()));
    }
    #[cfg(windows)]
    {
        return Some(format!("{canonical}:volume={}", volume_path(canonical)?));
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(unix)]
fn paths_share_filesystem(root: &str, file: &str) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(root), std::fs::metadata(file)) {
        (Ok(root), Ok(file)) => Some(root.dev() == file.dev()),
        _ => None,
    }
}

#[cfg(windows)]
fn paths_share_filesystem(root: &str, file: &str) -> Option<bool> {
    // `GetVolumePathNameW` resolves mounted folders as well as drive letters,
    // so files below a nested volume do not claim capacity from the configured
    // root's filesystem.
    let (root, file) = volume_path(root).zip(volume_path(file))?;
    Some(root.eq_ignore_ascii_case(&file))
}

#[cfg(windows)]
fn volume_path(path: &str) -> Option<String> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;

    let input: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let mut output = vec![0_u16; 32_768];
    let ok =
        unsafe { GetVolumePathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
    if ok == 0 {
        return None;
    }
    let end = output.iter().position(|&value| value == 0)?;
    String::from_utf16(&output[..end]).ok()
}

#[cfg(not(any(unix, windows)))]
fn paths_share_filesystem(_root: &str, _file: &str) -> Option<bool> {
    // Unknown platform identity is not a reason to widen a destructive
    // manifest. A supported platform needs an explicit identity probe first.
    None
}

fn set_storage_unknown(input: &mut MaintenanceInput, reason: &'static str) {
    input.facts.storage_root_id = Observation::unknown(reason);
    input.facts.storage_available_bytes = Observation::unknown(reason);
    input.facts.storage_total_bytes = Observation::unknown(reason);
    input.facts.storage_available_percent = Observation::unknown(reason);
}

fn storage_snapshot(input: &MaintenanceInput) -> MaintenanceStorageFactSnapshot {
    MaintenanceStorageFactSnapshot {
        storage_root_id: input.facts.storage_root_id.clone(),
        storage_available_bytes: input.facts.storage_available_bytes.clone(),
        storage_total_bytes: input.facts.storage_total_bytes.clone(),
        storage_available_percent: input.facts.storage_available_percent.clone(),
    }
}

fn apply_storage_snapshot(input: &mut MaintenanceInput, snapshot: &MaintenanceStorageFactSnapshot) {
    input.facts.storage_root_id = snapshot.storage_root_id.clone();
    input.facts.storage_available_bytes = snapshot.storage_available_bytes.clone();
    input.facts.storage_total_bytes = snapshot.storage_total_bytes.clone();
    input.facts.storage_available_percent = snapshot.storage_available_percent.clone();
}

#[cfg(test)]
mod tests {
    use super::{physical_root_membership, storage_capacity_values};

    #[test]
    fn physical_membership_rejects_outside_and_missing_paths_without_emptying_the_result() {
        let workspace = tempfile::tempdir().expect("temp directory");
        let root = workspace.path().join("root");
        let outside = workspace.path().join("outside");
        std::fs::create_dir_all(&root).expect("root directory");
        std::fs::create_dir_all(&outside).expect("outside directory");
        let inside_file = root.join("inside.mkv");
        let outside_file = outside.join("outside.mkv");
        std::fs::write(&inside_file, b"inside").expect("inside file");
        std::fs::write(&outside_file, b"outside").expect("outside file");

        let membership = physical_root_membership(
            root.to_str().expect("root utf8"),
            &[
                inside_file.to_string_lossy().to_string(),
                outside_file.to_string_lossy().to_string(),
            ],
        );
        assert_eq!(membership, Some(vec![true, false]));

        assert_eq!(
            physical_root_membership(
                root.to_str().expect("root utf8"),
                &[root.join("gone.mkv").to_string_lossy().to_string()],
            ),
            None,
            "unreadable media is unknown, never an empty root manifest"
        );
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

    #[cfg(unix)]
    #[test]
    fn physical_membership_rejects_a_symlink_escape() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().expect("temp directory");
        let root = workspace.path().join("root");
        let outside = workspace.path().join("outside");
        std::fs::create_dir_all(&root).expect("root directory");
        std::fs::create_dir_all(&outside).expect("outside directory");
        let outside_file = outside.join("outside.mkv");
        std::fs::write(&outside_file, b"outside").expect("outside file");
        let escaped = root.join("escaped.mkv");
        symlink(&outside_file, &escaped).expect("symlink");

        assert_eq!(
            physical_root_membership(
                root.to_str().expect("root utf8"),
                &[escaped.to_string_lossy().to_string()],
            ),
            Some(vec![false])
        );
    }
}
