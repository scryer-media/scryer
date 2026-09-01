//! FR-088: when a location operation finishes, tell the connected media servers
//! about the folders it actually changed — and only those folders.
//!
//! # Why this reads checkpoints rather than the plan
//!
//! The confirmed plan says what the operation *intended* to do. The checkpoint
//! rows say what it *did*, they survive a restart, and they are already the
//! unit resume and Activity are built on (FR-092). A canceled operation that
//! settled four titles before it stopped changed four folders on disk, and the
//! media server is stale for exactly those four — the plan cannot tell you
//! that, and a per-operation "did it succeed?" gate would either notify nothing
//! (leaving a server pointing at files that moved) or notify everything
//! (including titles that never started).
//!
//! So the decision is **per title, from the persisted checkpoint**, and the
//! operation-level state is used only to decide *when* to look: once, when the
//! run reaches a terminal state. A run that stops for resume writes no
//! notification, because its checkpoints are still there and the terminal run
//! that eventually follows covers them.
//!
//! # What counts as a changed folder
//!
//! A title contributes its destination folder when it settled `Completed` or
//! `CompletedWithWarnings` **and** it had files to place. Deliberately excluded:
//!
//! - `Skipped` / `Blocked` / `Failed` titles — nothing of theirs was placed.
//! - `CatalogOnly` and `NoOp` titles (FR-076) — the catalog moved, the bytes did
//!   not, so no media server has anything to re-read.
//!
//! A merge title contributes the surviving title's folder, because that is where
//! its files were placed: the checkpoint's `destination_folder_path` for a merge
//! is the destination title's folder, and `merged_into_title_id` is set beside
//! it (US7, D8).
//!
//! # Failure is never the operation's problem
//!
//! Same discipline as the Activity progress observer and the Group 6 post-merge
//! scheduler: this runs *after* the operation has settled, its failures are
//! logged and dropped, and nothing it does can change an operation's outcome.
//! The worst case for a lost notification is a media server that re-reads the
//! folder on its own schedule instead of within seconds.

use std::collections::BTreeSet;
use std::path::Path;

use async_trait::async_trait;

use crate::AppResult;
use crate::location::classify::TitleLocationClass;
use crate::location::model::{LocationOperationState, TitleCheckpoint, TitleCheckpointState};
use crate::ports::LocationOperationRepository;

/// Above this many distinct destination folders the notification is collapsed to
/// the folders' parents (see [`refresh_folders`]).
///
/// A title-scoped move touches a handful of folders; a root change touches every
/// title in the root. Sending ten thousand paths to a media server one at a time
/// is worse for that server than the whole-library scan FR-088 is trying to
/// avoid, and every one of those folders shares a parent — the destination root
/// — so collapsing loses nothing but the request count.
pub const MAX_TARGETED_FOLDERS: usize = 200;

/// The folders one finished operation changed, addressed to whatever knows how
/// to reach the media servers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaServerRefreshRequest {
    pub operation_id: String,
    /// Destination folders, in Scryer's own filesystem namespace. Translating
    /// them into each server's namespace is the dispatcher's job, because the
    /// mapping is per connection.
    pub folders: Vec<String>,
}

/// Delivers FR-088's notification.
///
/// A seam because the location subsystem has no business knowing how a media
/// server is reached — which connections exist, which are enabled, how a Plex
/// section is resolved — and because a test that wants to prove "a completed
/// move notifies, a canceled one does not" must be able to observe the call
/// without an HTTP server.
#[async_trait]
pub trait LocationMediaServerRefresh: Send + Sync {
    async fn refresh_media_servers(&self, request: MediaServerRefreshRequest) -> AppResult<()>;
}

/// Notifies nobody. The right default for a runner wired without the use-case
/// layer: a media server that is not told simply re-reads on its own schedule.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMediaServerRefresh;

#[async_trait]
impl LocationMediaServerRefresh for NoMediaServerRefresh {
    async fn refresh_media_servers(&self, _request: MediaServerRefreshRequest) -> AppResult<()> {
        Ok(())
    }
}

/// The destination folders a finished operation actually placed content in.
///
/// Sorted and de-duplicated (two titles can land in one folder after a merge),
/// and collapsed to parents past [`MAX_TARGETED_FOLDERS`].
pub fn refresh_folders(checkpoints: &[TitleCheckpoint]) -> Vec<String> {
    let folders = checkpoints
        .iter()
        .filter(|checkpoint| placed_content(checkpoint))
        .filter_map(|checkpoint| checkpoint.placement.destination_folder_path.as_deref())
        .map(str::trim)
        .filter(|folder| !folder.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

    if folders.len() <= MAX_TARGETED_FOLDERS {
        return folders.into_iter().collect();
    }

    // Collapsing is only worth doing when it actually reduces the set; a folder
    // sitting directly at the filesystem root has no usable parent, and in that
    // case the original list is still the more honest request.
    let parents = folders
        .iter()
        .filter_map(|folder| parent_folder(folder))
        .collect::<BTreeSet<_>>();
    if parents.is_empty() || parents.len() >= folders.len() {
        return folders.into_iter().collect();
    }
    tracing::debug!(
        folders = folders.len(),
        parents = parents.len(),
        "collapsing a wide location operation's media-server refresh to the parent folders"
    );
    parents.into_iter().collect()
}

/// Whether this title put bytes at its destination.
fn placed_content(checkpoint: &TitleCheckpoint) -> bool {
    if !matches!(
        checkpoint.state,
        TitleCheckpointState::Completed | TitleCheckpointState::CompletedWithWarnings
    ) {
        return false;
    }
    if checkpoint.files_total <= 0 {
        return false;
    }
    // FR-076: a catalog-only reassignment moves a row, not a file. Nothing on
    // any media server changed, so nothing is asked to re-read.
    !matches!(
        checkpoint.classification,
        Some(TitleLocationClass::CatalogOnly) | Some(TitleLocationClass::NoOp)
    )
}

fn parent_folder(folder: &str) -> Option<String> {
    let parent = Path::new(folder).parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    let parent = parent.to_string_lossy().to_string();
    (parent != folder).then_some(parent)
}

/// FR-088's trigger: called once, after a run of `operation_id` has settled.
///
/// Does nothing for a run that stopped short of a terminal state — its
/// checkpoints are durable and the terminal run picks them up — and nothing for
/// a terminal run that placed no content, which is what makes a cancel that
/// stopped before its first title silent.
///
/// Never returns an error, and never fails an operation: every failure path here
/// is logged and dropped (see the module docs).
pub async fn notify_media_servers_for_operation(
    store: &dyn LocationOperationRepository,
    refresh: &dyn LocationMediaServerRefresh,
    operation_id: &str,
    state: LocationOperationState,
) {
    if !state.is_terminal() {
        return;
    }

    let checkpoints = match store.list_location_title_checkpoints(operation_id).await {
        Ok(checkpoints) => checkpoints,
        Err(error) => {
            tracing::warn!(
                operation_id,
                error = %error,
                "could not read a finished location operation's checkpoints to refresh media servers"
            );
            return;
        }
    };

    let folders = refresh_folders(&checkpoints);
    if folders.is_empty() {
        tracing::debug!(
            operation_id,
            state = state.as_str(),
            "a finished location operation placed no content; media servers have nothing to re-read"
        );
        return;
    }

    if let Err(error) = refresh
        .refresh_media_servers(MediaServerRefreshRequest {
            operation_id: operation_id.to_string(),
            folders,
        })
        .await
    {
        tracing::warn!(
            operation_id,
            error = %error,
            "could not notify media servers about a finished location operation; they re-read on their own schedule"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::location::model::TitleCheckpointPlacement;

    fn checkpoint(
        title_id: &str,
        state: TitleCheckpointState,
        destination_folder_path: Option<&str>,
    ) -> TitleCheckpoint {
        TitleCheckpoint {
            operation_id: "op-1".into(),
            title_id: title_id.into(),
            sequence: 1,
            state,
            classification: Some(TitleLocationClass::RootMove),
            placement: TitleCheckpointPlacement {
                destination_folder_path: destination_folder_path.map(str::to_string),
                ..TitleCheckpointPlacement::default()
            },
            files_total: 2,
            files_verified: 2,
            bytes_total: 20,
            bytes_verified: 20,
            detail: None,
            started_at: None,
            updated_at: chrono::Utc::now(),
            completed_at: None,
        }
    }

    #[test]
    fn completed_titles_contribute_their_destination_folder() {
        let checkpoints = vec![
            checkpoint(
                "t-1",
                TitleCheckpointState::Completed,
                Some("/media/tv/Some Show"),
            ),
            checkpoint(
                "t-2",
                TitleCheckpointState::CompletedWithWarnings,
                Some("/media/tv/Other Show"),
            ),
        ];
        assert_eq!(
            refresh_folders(&checkpoints),
            vec![
                "/media/tv/Other Show".to_string(),
                "/media/tv/Some Show".to_string(),
            ]
        );
    }

    #[test]
    fn unsettled_and_unsuccessful_titles_contribute_nothing() {
        for state in [
            TitleCheckpointState::Pending,
            TitleCheckpointState::Moving,
            TitleCheckpointState::Verifying,
            TitleCheckpointState::Reconciling,
            TitleCheckpointState::CleaningUp,
            TitleCheckpointState::Skipped,
            TitleCheckpointState::Blocked,
            TitleCheckpointState::Failed,
        ] {
            let checkpoints = vec![checkpoint("t-1", state, Some("/media/tv/Some Show"))];
            assert!(
                refresh_folders(&checkpoints).is_empty(),
                "{} must not ask a media server to re-read anything",
                state.as_str()
            );
        }
    }

    #[test]
    fn catalog_only_and_no_op_titles_contribute_nothing() {
        for class in [TitleLocationClass::CatalogOnly, TitleLocationClass::NoOp] {
            let mut settled = checkpoint(
                "t-1",
                TitleCheckpointState::Completed,
                Some("/media/tv/Some Show"),
            );
            settled.classification = Some(class);
            assert!(
                refresh_folders(&[settled]).is_empty(),
                "{} changes the catalog, not the filesystem",
                class.as_str()
            );
        }
    }

    #[test]
    fn a_title_that_placed_no_files_contributes_nothing() {
        let mut settled = checkpoint(
            "t-1",
            TitleCheckpointState::Completed,
            Some("/media/tv/Some Show"),
        );
        settled.files_total = 0;
        settled.files_verified = 0;
        assert!(refresh_folders(&[settled]).is_empty());
    }

    #[test]
    fn a_merge_contributes_the_surviving_titles_folder_once() {
        let mut source = checkpoint(
            "t-1",
            TitleCheckpointState::Completed,
            Some("/media/tv/Survivor"),
        );
        source.placement.merged_into_title_id = Some("t-9".into());
        let survivor = checkpoint(
            "t-2",
            TitleCheckpointState::Completed,
            Some("/media/tv/Survivor"),
        );
        assert_eq!(
            refresh_folders(&[source, survivor]),
            vec!["/media/tv/Survivor".to_string()]
        );
    }

    #[test]
    fn a_missing_destination_folder_is_skipped() {
        let checkpoints = vec![
            checkpoint("t-1", TitleCheckpointState::Completed, None),
            checkpoint("t-2", TitleCheckpointState::Completed, Some("   ")),
        ];
        assert!(refresh_folders(&checkpoints).is_empty());
    }

    #[test]
    fn a_root_wide_operation_collapses_to_the_parent_folders() {
        let checkpoints = (0..MAX_TARGETED_FOLDERS + 5)
            .map(|index| {
                checkpoint(
                    &format!("t-{index}"),
                    TitleCheckpointState::Completed,
                    Some(&format!("/media/tv/Show {index}")),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            refresh_folders(&checkpoints),
            vec!["/media/tv".to_string()],
            "a root change must not become one request per title"
        );
    }

    #[test]
    fn collapsing_is_skipped_when_it_would_not_reduce_the_request() {
        let checkpoints = (0..MAX_TARGETED_FOLDERS + 5)
            .map(|index| {
                checkpoint(
                    &format!("t-{index}"),
                    TitleCheckpointState::Completed,
                    Some(&format!("/media/lib{index}/Show")),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(refresh_folders(&checkpoints).len(), MAX_TARGETED_FOLDERS + 5);
    }
}
