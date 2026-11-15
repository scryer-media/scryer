//! Which configured root a path belongs to, and how the owner treats it.
//!
//! Resolution is pure path arithmetic over the installed roots: no
//! filesystem call. A path under no configured root is [`VolumeClass::AppData`]:
//! passed straight through, never cached, never bounded by a timeout, so the
//! owner works before any root is configured and without an async runtime.
//!
//! What the owner learns about a root's filesystem by watching it answer
//! (an exclusive rename it does not offer, say) is kept per root here, so the
//! next operation goes straight to what works. Callers never see it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The kinds of configured root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VolumeClass {
    LibraryRoot,
    DownloadFolder,
    RecycleBin,
    /// Anything not under a configured root: the database directory, logs,
    /// caches, temporary files.
    AppData,
}

/// How the owner treats paths of one class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VolumeTraits {
    /// Readings may be kept for [`super::cache::DISK_STAT_REUSE`] and seeded
    /// by listings.
    pub cacheable: bool,
    /// Bound on one read from the async forms. A timed-out read is recorded
    /// as unreachable.
    pub probe_timeout: Option<Duration>,
}

/// Bound on one read of a configured root, the same bound a capacity sample
/// of a root gets: long enough for a slow share, short enough that a dead
/// mount does not hold a request.
const ROOT_PROBE_TIMEOUT: Duration = super::capacity::STORAGE_CAPACITY_SAMPLE_TIMEOUT;

impl VolumeClass {
    pub(crate) fn traits(self) -> VolumeTraits {
        match self {
            Self::LibraryRoot | Self::DownloadFolder | Self::RecycleBin => VolumeTraits {
                cacheable: true,
                probe_timeout: Some(ROOT_PROBE_TIMEOUT),
            },
            Self::AppData => VolumeTraits {
                cacheable: false,
                probe_timeout: None,
            },
        }
    }
}

/// Behaviour learned from a root's answers.
#[derive(Debug, Default)]
pub(crate) struct Learned {
    /// The filesystem answered an exclusive rename with "unsupported".
    pub no_exclusive_rename: AtomicBool,
}

#[derive(Clone, Debug)]
pub(crate) struct Volume {
    pub root: PathBuf,
    pub class: VolumeClass,
    pub learned: Arc<Learned>,
}

/// The installed roots.
#[derive(Debug, Default)]
pub(crate) struct Volumes {
    roots: Vec<Volume>,
}

impl Volumes {
    /// Roots carry over what was learned about them when they are installed
    /// again unchanged.
    pub(crate) fn replaced(&self, roots: Vec<(PathBuf, VolumeClass)>) -> Self {
        let roots = roots
            .into_iter()
            .filter(|(root, _)| !root.as_os_str().is_empty())
            .map(|(root, class)| {
                let learned = self
                    .roots
                    .iter()
                    .find(|volume| volume.root == root)
                    .map(|volume| volume.learned.clone())
                    .unwrap_or_default();
                Volume {
                    root,
                    class,
                    learned,
                }
            })
            .collect();
        Self { roots }
    }

    /// The most specific configured root containing `path`, or `None` for an
    /// application-data path.
    pub(crate) fn resolve(&self, path: &Path) -> Option<&Volume> {
        self.roots
            .iter()
            .filter(|volume| crate::recycle_bin::path_is_under_configured_root(path, &volume.root))
            .max_by_key(|volume| volume.root.components().count())
    }

    pub(crate) fn class_of(&self, path: &Path) -> VolumeClass {
        self.resolve(path)
            .map_or(VolumeClass::AppData, |volume| volume.class)
    }

    pub(crate) fn note_no_exclusive_rename(&self, path: &Path) {
        if let Some(volume) = self.resolve(path) {
            volume
                .learned
                .no_exclusive_rename
                .store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn lacks_exclusive_rename(&self, path: &Path) -> bool {
        self.resolve(path)
            .is_some_and(|volume| volume.learned.no_exclusive_rename.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volumes() -> Volumes {
        Volumes::default().replaced(vec![
            ("/srv/fixture-media".into(), VolumeClass::LibraryRoot),
            (
                "/srv/fixture-media/.recycle".into(),
                VolumeClass::RecycleBin,
            ),
            ("/srv/fixture-downloads".into(), VolumeClass::DownloadFolder),
        ])
    }

    #[test]
    fn a_path_resolves_to_the_most_specific_root() {
        let volumes = volumes();
        assert_eq!(
            volumes.class_of(Path::new("/srv/fixture-media/Invented Show/S01")),
            VolumeClass::LibraryRoot
        );
        assert_eq!(
            volumes.class_of(Path::new("/srv/fixture-media/.recycle/old.mkv")),
            VolumeClass::RecycleBin
        );
        assert_eq!(
            volumes.class_of(Path::new("/srv/fixture-downloads/job")),
            VolumeClass::DownloadFolder
        );
        assert_eq!(
            volumes.class_of(Path::new("/srv/fixture-media-sibling/x")),
            VolumeClass::AppData
        );
        assert_eq!(
            volumes.class_of(Path::new("/var/lib/invented-app/scryer.db")),
            VolumeClass::AppData
        );
    }

    #[test]
    fn with_no_roots_everything_is_app_data() {
        let volumes = Volumes::default();
        assert_eq!(
            volumes.class_of(Path::new("/srv/fixture-media/x")),
            VolumeClass::AppData
        );
        assert!(!VolumeClass::AppData.traits().cacheable);
        assert_eq!(VolumeClass::AppData.traits().probe_timeout, None);
    }

    #[test]
    fn learned_behaviour_survives_reinstalling_a_root() {
        let first = volumes();
        first.note_no_exclusive_rename(Path::new("/srv/fixture-media/a"));
        let second = first.replaced(vec![
            ("/srv/fixture-media".into(), VolumeClass::LibraryRoot),
            ("/srv/fixture-other".into(), VolumeClass::LibraryRoot),
        ]);
        assert!(second.lacks_exclusive_rename(Path::new("/srv/fixture-media/b")));
        assert!(!second.lacks_exclusive_rename(Path::new("/srv/fixture-other/b")));
        // Application data learns nothing: it has no root to keep it on.
        second.note_no_exclusive_rename(Path::new("/tmp/invented"));
        assert!(!second.lacks_exclusive_rename(Path::new("/tmp/invented")));
    }
}
