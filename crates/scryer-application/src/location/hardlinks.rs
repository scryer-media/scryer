//! Hardlink detection for move previews (FR-085).
//!
//! A source file with a link count greater than one is almost always a seeding
//! copy sharing an inode with the download client's payload. Moving it across
//! volumes cannot preserve the link: the copy at the destination is a new inode,
//! the seeding copy is orphaned at the old path, and the bytes now occupy disk
//! twice. Recycling one of several links frees no space at all.
//!
//! Detection is a `st_nlink` read on unix. Other platforms do not report a
//! portable link count through `std::fs`, so they report
//! [`LinkCount::Unsupported`] rather than guessing "1" — an unknown link count
//! must never be presented as "no hardlinks".
//!
//! Warning construction is pure so previews and the executor summary can build
//! the same warnings from the same facts. Wiring into the preview model happens
//! where the preview is assembled; this module only detects and describes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{AppError, AppResult};

/// The link count of one file, or the fact that this platform does not report
/// one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LinkCount {
    /// Link count read from filesystem metadata (`st_nlink` on unix).
    Known(u64),
    /// This platform does not expose a hardlink count: not detected, and
    /// explicitly not "one".
    Unsupported,
}

impl LinkCount {
    /// The count, when the platform reports one.
    pub fn count(&self) -> Option<u64> {
        match self {
            Self::Known(count) => Some(*count),
            Self::Unsupported => None,
        }
    }

    /// Whether the file is provably hardlinked (more than one directory entry
    /// points at these bytes). An unsupported platform is never "hardlinked",
    /// but it is also not proof of the opposite — see [`LinkCount::is_known`].
    pub fn is_hardlinked(&self) -> bool {
        matches!(self, Self::Known(count) if *count > 1)
    }

    pub fn is_known(&self) -> bool {
        matches!(self, Self::Known(_))
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Known(_) => "known",
            Self::Unsupported => "unsupported",
        }
    }
}

/// A detected fact about one source file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardlinkFact {
    /// The source path the count was read from.
    pub path: String,
    pub link_count: LinkCount,
    /// Size of the file, so a preview can state how much disk a broken link
    /// would cost.
    pub size_bytes: u64,
}

impl HardlinkFact {
    pub fn is_hardlinked(&self) -> bool {
        self.link_count.is_hardlinked()
    }
}

/// Read the link count of `path` (blocking).
///
/// On unix this is `st_nlink` from `symlink_metadata`, so a symlink reports its
/// own link count rather than its target's. On other platforms the file is
/// still stat'ed — so a missing or unreadable path is still an error — but the
/// count comes back as [`LinkCount::Unsupported`].
pub fn link_count(path: &Path) -> AppResult<LinkCount> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to stat path for hardlink detection: {}: {error}",
            path.display()
        ))
    })?;
    Ok(link_count_from_metadata(&metadata))
}

/// Read the link count and size of `path` (blocking).
pub fn hardlink_fact(path: &Path) -> AppResult<HardlinkFact> {
    hardlink_fact_optional(path)?.ok_or_else(|| {
        AppError::NotFound(format!(
            "path not found for hardlink detection: {}",
            path.display()
        ))
    })
}

/// Read the link count and size of `path` (blocking), returning `Ok(None)` when
/// the path no longer exists.
pub fn hardlink_fact_optional(path: &Path) -> AppResult<Option<HardlinkFact>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat path for hardlink detection: {}: {error}",
                path.display()
            )));
        }
    };
    Ok(Some(HardlinkFact {
        path: path.display().to_string(),
        link_count: link_count_from_metadata(&metadata),
        size_bytes: metadata.len(),
    }))
}

#[cfg(unix)]
fn link_count_from_metadata(metadata: &std::fs::Metadata) -> LinkCount {
    use std::os::unix::fs::MetadataExt;
    LinkCount::Known(metadata.nlink())
}

#[cfg(not(unix))]
fn link_count_from_metadata(_metadata: &std::fs::Metadata) -> LinkCount {
    LinkCount::Unsupported
}

/// Detect hardlink facts for a batch of source paths on a blocking thread, so a
/// large preview does not stall the async runtime.
///
/// Paths that no longer exist are skipped rather than failing the batch: a
/// vanished source is the staleness check's problem (FR-081/FR-089), not the
/// hardlink warning's. Any other IO error fails the batch, because silently
/// reporting "no hardlinks" for an unreadable file would understate the risk.
pub async fn detect_hardlinks(paths: Vec<PathBuf>) -> AppResult<Vec<HardlinkFact>> {
    tokio::task::spawn_blocking(move || {
        let mut facts = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(fact) = hardlink_fact_optional(&path)? {
                facts.push(fact);
            }
        }
        Ok(facts)
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!("hardlink detection task failed to join: {error}"))
    })?
}

/// A preview warning about hardlinked source files (FR-085).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HardlinkWarning {
    /// The move crosses volumes, so the link cannot be preserved.
    CrossVolumeMoveBreaksLink {
        file_count: usize,
        link_bytes: u64,
        sample_paths: Vec<String>,
    },
    /// The move may cross volumes (the relationship is not known yet).
    PossibleCrossVolumeMoveBreaksLink {
        file_count: usize,
        link_bytes: u64,
        sample_paths: Vec<String>,
    },
    /// Recycling one of several links frees no space.
    RecycleFreesNoSpace {
        file_count: usize,
        link_bytes: u64,
        sample_paths: Vec<String>,
    },
}

impl HardlinkWarning {
    pub fn code(&self) -> &'static str {
        match self {
            Self::CrossVolumeMoveBreaksLink { .. } => "cross_volume_move_breaks_hardlink",
            Self::PossibleCrossVolumeMoveBreaksLink { .. } => {
                "possible_cross_volume_move_breaks_hardlink"
            }
            Self::RecycleFreesNoSpace { .. } => "recycle_frees_no_space",
        }
    }

    pub fn file_count(&self) -> usize {
        match self {
            Self::CrossVolumeMoveBreaksLink { file_count, .. }
            | Self::PossibleCrossVolumeMoveBreaksLink { file_count, .. }
            | Self::RecycleFreesNoSpace { file_count, .. } => *file_count,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::CrossVolumeMoveBreaksLink {
                file_count,
                link_bytes,
                ..
            } => format!(
                "{file_count} source file(s) are hardlinked (for example, still seeding). This move crosses volumes, so the link will be broken: the other copy is orphaned at its current path and these {link_bytes} byte(s) will occupy disk twice."
            ),
            Self::PossibleCrossVolumeMoveBreaksLink {
                file_count,
                link_bytes,
                ..
            } => format!(
                "{file_count} source file(s) are hardlinked (for example, still seeding). If this move crosses volumes the link will be broken: the other copy is orphaned at its current path and these {link_bytes} byte(s) will occupy disk twice."
            ),
            Self::RecycleFreesNoSpace {
                file_count,
                link_bytes,
                ..
            } => format!(
                "{file_count} source file(s) are hardlinked, so recycling them frees no space: the other link keeps all {link_bytes} byte(s) on disk."
            ),
        }
    }
}

/// How many sample paths a warning carries; previews show complete counts with
/// sampled items (FR-081).
pub const HARDLINK_WARNING_SAMPLE_LIMIT: usize = 5;

/// Build the FR-085 warnings for a set of detected facts.
///
/// * `same_volume` follows the preview convention: `Some(true)` for a
///   same-volume move, `Some(false)` for cross-volume, `None` when the
///   relationship is not known yet.
/// * `recycles_source` is true when the operation will recycle the source copy
///   (any cross-volume move that removes the source, or a dedup decision).
///
/// This is pure: the same facts always produce the same warnings, so the
/// preview and the completion summary cannot disagree.
pub fn hardlink_warnings(
    facts: &[HardlinkFact],
    same_volume: Option<bool>,
    recycles_source: bool,
) -> Vec<HardlinkWarning> {
    let linked: Vec<&HardlinkFact> = facts.iter().filter(|fact| fact.is_hardlinked()).collect();
    if linked.is_empty() {
        return Vec::new();
    }

    let file_count = linked.len();
    let link_bytes = linked.iter().map(|fact| fact.size_bytes).sum();
    let sample_paths: Vec<String> = linked
        .iter()
        .take(HARDLINK_WARNING_SAMPLE_LIMIT)
        .map(|fact| fact.path.clone())
        .collect();

    let mut warnings = Vec::new();
    match same_volume {
        Some(true) => {}
        Some(false) => warnings.push(HardlinkWarning::CrossVolumeMoveBreaksLink {
            file_count,
            link_bytes,
            sample_paths: sample_paths.clone(),
        }),
        None => warnings.push(HardlinkWarning::PossibleCrossVolumeMoveBreaksLink {
            file_count,
            link_bytes,
            sample_paths: sample_paths.clone(),
        }),
    }
    if recycles_source {
        warnings.push(HardlinkWarning::RecycleFreesNoSpace {
            file_count,
            link_bytes,
            sample_paths,
        });
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(path: &str, count: u64, size: u64) -> HardlinkFact {
        HardlinkFact {
            path: path.to_string(),
            link_count: LinkCount::Known(count),
            size_bytes: size,
        }
    }

    #[test]
    fn a_single_link_file_is_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("solo.mkv");
        std::fs::write(&path, b"solo").expect("write");

        let detected = hardlink_fact(&path).expect("fact");
        assert!(!detected.is_hardlinked());
        assert_eq!(detected.size_bytes, 4);
        if cfg!(unix) {
            assert_eq!(detected.link_count, LinkCount::Known(1));
        } else {
            assert_eq!(detected.link_count, LinkCount::Unsupported);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_hardlinked_file_is_detected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("seeding.mkv");
        let link = dir.path().join("library.mkv");
        std::fs::write(&original, b"payload").expect("write");
        std::fs::hard_link(&original, &link).expect("hard link");

        let detected = hardlink_fact(&original).expect("fact");
        assert_eq!(detected.link_count, LinkCount::Known(2));
        assert!(detected.is_hardlinked());
        assert_eq!(link_count(&link).expect("count"), LinkCount::Known(2));
    }

    #[test]
    fn a_missing_path_is_an_error_for_the_single_file_probe() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(hardlink_fact(&dir.path().join("gone.mkv")).is_err());
    }

    #[tokio::test]
    async fn the_batch_probe_skips_paths_that_vanished() {
        let dir = tempfile::tempdir().expect("tempdir");
        let present = dir.path().join("present.mkv");
        std::fs::write(&present, b"here").expect("write");

        let facts = detect_hardlinks(vec![present.clone(), dir.path().join("gone.mkv")])
            .await
            .expect("detect");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].path, present.display().to_string());
    }

    #[test]
    fn no_hardlinks_means_no_warnings() {
        let facts = vec![fact("/media/a.mkv", 1, 100)];
        assert!(hardlink_warnings(&facts, Some(false), true).is_empty());
    }

    #[test]
    fn a_cross_volume_move_warns_that_the_link_breaks() {
        let facts = vec![fact("/media/a.mkv", 2, 100), fact("/media/b.mkv", 1, 50)];
        let warnings = hardlink_warnings(&facts, Some(false), false);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "cross_volume_move_breaks_hardlink");
        assert_eq!(warnings[0].file_count(), 1);
        let message = warnings[0].message();
        assert!(message.contains("orphaned"), "{message}");
        assert!(message.contains("twice"), "{message}");
    }

    #[test]
    fn a_same_volume_move_does_not_warn_about_a_broken_link() {
        let facts = vec![fact("/media/a.mkv", 2, 100)];
        assert!(hardlink_warnings(&facts, Some(true), false).is_empty());
    }

    #[test]
    fn an_unknown_volume_relationship_warns_conditionally() {
        let facts = vec![fact("/media/a.mkv", 2, 100)];
        let warnings = hardlink_warnings(&facts, None, false);
        assert_eq!(
            warnings[0].code(),
            "possible_cross_volume_move_breaks_hardlink"
        );
    }

    #[test]
    fn recycling_a_link_warns_that_it_frees_no_space() {
        let facts = vec![fact("/media/a.mkv", 3, 100)];
        let warnings = hardlink_warnings(&facts, Some(true), true);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code(), "recycle_frees_no_space");
        assert!(warnings[0].message().contains("frees no space"));
    }

    #[test]
    fn sample_paths_are_capped_but_counts_are_complete() {
        let facts: Vec<HardlinkFact> = (0..12)
            .map(|index| fact(&format!("/media/{index}.mkv"), 2, 10))
            .collect();
        let warnings = hardlink_warnings(&facts, Some(false), false);
        assert_eq!(warnings[0].file_count(), 12);
        match &warnings[0] {
            HardlinkWarning::CrossVolumeMoveBreaksLink {
                sample_paths,
                link_bytes,
                ..
            } => {
                assert_eq!(sample_paths.len(), HARDLINK_WARNING_SAMPLE_LIMIT);
                assert_eq!(*link_bytes, 120);
            }
            other => panic!("unexpected warning: {other:?}"),
        }
    }

    #[test]
    fn an_unsupported_platform_is_not_reported_as_unlinked_proof() {
        assert!(!LinkCount::Unsupported.is_known());
        assert!(!LinkCount::Unsupported.is_hardlinked());
        assert_eq!(LinkCount::Unsupported.count(), None);
    }
}
