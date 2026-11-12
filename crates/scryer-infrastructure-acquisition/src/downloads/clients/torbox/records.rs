//! The client's durable record of the jobs it sent to TorBox.
//!
//! One JSON document per client configuration, kept in the HTTP engine's
//! state directory under the download directory, next to the engine's own
//! queue. It holds what Scryer needs to find a job's TorBox item again after a
//! restart and to remove exactly that item later. It never holds the API
//! token or a download link.
//!
//! The document is replaced atomically: written to a staging file, synced,
//! renamed into place, and the directory synced, so a crash leaves either the
//! previous document or the new one.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::api::TorboxItemKind;
use crate::downloads::clients::http::owned::{STATE_DIR_NAME, safe_file_stem, sync_directory};

/// The document format this build writes and reads.
pub(crate) const RECORD_FORMAT_VERSION: u32 = 1;

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TorboxJobPhase {
    /// The intent is recorded and the create was sent, but TorBox's answer
    /// has not been recorded: the item may or may not exist.
    Submitting,
    /// TorBox holds the item and is fetching it.
    Remote,
    /// TorBox has the whole item and the engine fetches its files. The
    /// engine's job is the truth for this phase from here on.
    Fetching,
    /// The job ended before its files could be fetched.
    Failed,
}

/// One job, from the create to its removal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TorboxJobRecord {
    /// The client item id minted at submit. Every phase reports the job
    /// under this id.
    pub id: String,
    /// The grab's download id, in its wire form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    pub title_id: String,
    pub title_name: String,
    pub facet: String,
    pub release_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub kind: TorboxItemKind,
    /// The TorBox item id, once TorBox's answer named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torbox_id: Option<i64>,
    /// Lowercase hex v1 info hash, for torrents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    /// The name sent with the create, which identifies a usenet item when
    /// TorBox's answer was lost.
    pub submitted_name: String,
    /// Items of the account that already matched this job before its create
    /// was sent. A lost answer is never matched to one of them: they are
    /// not this job's to adopt or delete.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preexisting_ids: Vec<i64>,
    /// Whether TorBox reports the torrent as private, once it has said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<bool>,
    /// The job was removed while its create was still in flight. The item
    /// that create made is deleted once its id is known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    pub phase: TorboxJobPhase,
    /// The job folder chosen for the fetch, recorded before the engine job
    /// is added so a restart can find that job again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_job_id: Option<String>,
    /// Resolved seeding goals of the grab, reported back with the queue item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_goal_seconds: Option<i64>,
    /// The grab's seeding profile never lets the torrent be removed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub never_remove: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the poller first saw TorBox holding the whole item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
}

impl TorboxJobRecord {
    /// Whether Scryer asked for this torrent to seed to a goal.
    pub(crate) fn has_seed_goal(&self) -> bool {
        self.seed_goal_ratio.is_some() || self.seed_goal_seconds.is_some()
    }

    /// Whether TorBox is asked to seed: a numeric goal, or a profile that
    /// never removes the torrent. Only a numeric goal is watched: with
    /// never-remove alone, nothing TorBox reports changes the outcome.
    pub(crate) fn asks_to_seed(&self) -> bool {
        self.has_seed_goal() || self.never_remove
    }

    /// TorBox holds the whole item: the fetch started, or the job failed
    /// after TorBox finished (its files were refused), so TorBox may be
    /// seeding it either way.
    pub(crate) fn torbox_finished(&self) -> bool {
        match self.phase {
            TorboxJobPhase::Fetching => true,
            TorboxJobPhase::Failed => self.remote_finished_at.is_some(),
            TorboxJobPhase::Submitting | TorboxJobPhase::Remote => false,
        }
    }

    /// A torrent whose TorBox seeding decides when it may be removed, so
    /// it is polled while it seeds.
    pub(crate) fn watches_seeding(&self) -> bool {
        self.kind == super::api::TorboxItemKind::Torrent
            && self.torbox_finished()
            && self.torbox_id.is_some()
            && self.has_seed_goal()
    }

    /// A finished torrent with no seed goal whose privacy TorBox has not
    /// reported: until it has, the torrent is held, since a private one
    /// must keep seeding.
    pub(crate) fn awaits_privacy(&self) -> bool {
        self.kind == super::api::TorboxItemKind::Torrent
            && self.torbox_finished()
            && self.torbox_id.is_some()
            && !self.has_seed_goal()
            && self.private.is_none()
    }

    pub(crate) fn fail(&mut self, reason: &'static str, message: String, now: DateTime<Utc>) {
        self.phase = TorboxJobPhase::Failed;
        self.error = Some(message);
        self.error_reason = Some(reason.to_string());
        self.updated_at = now;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct TorboxRecordDocument {
    pub version: u32,
    pub jobs: Vec<TorboxJobRecord>,
}

/// Reads and atomically replaces one client's record document.
#[derive(Clone, Debug)]
pub(crate) struct TorboxRecordStore {
    path: PathBuf,
}

impl TorboxRecordStore {
    pub(crate) fn for_client(download_dir: &Path, client_id: &str) -> Self {
        Self {
            path: download_dir
                .join(STATE_DIR_NAME)
                .join(format!("torbox-{}.json", safe_file_stem(client_id))),
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The stored records, or none when no document was ever written.
    pub(crate) async fn load(&self) -> AppResult<Vec<TorboxJobRecord>> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "the TorBox client could not read its records at {}: {error}",
                    self.path.display()
                )));
            }
        };
        let document: TorboxRecordDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Repository(format!(
                "the TorBox client's records at {} are unreadable ({error}); they were left untouched",
                self.path.display()
            ))
        })?;
        if document.version > RECORD_FORMAT_VERSION {
            return Err(AppError::Repository(format!(
                "the TorBox client's records at {} were written by a newer version (format {})",
                self.path.display(),
                document.version
            )));
        }
        Ok(document.jobs)
    }

    pub(crate) async fn save(&self, jobs: &[TorboxJobRecord]) -> AppResult<()> {
        #[derive(Serialize)]
        struct DocumentRef<'a> {
            version: u32,
            jobs: &'a [TorboxJobRecord],
        }
        let parent = self.path.parent().ok_or_else(|| {
            AppError::Repository("the TorBox client's record path has no parent".into())
        })?;
        let bytes = serde_json::to_vec(&DocumentRef {
            version: RECORD_FORMAT_VERSION,
            jobs,
        })
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let staging = self.path.with_extension("json.next");
        let write = async {
            tokio::fs::create_dir_all(parent).await?;
            let mut file = tokio::fs::File::create(&staging).await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&staging, &self.path).await?;
            sync_directory(parent).await
        };
        write.await.map_err(|error| {
            AppError::Repository(format!(
                "the TorBox client could not record its jobs at {}: {error}",
                self.path.display()
            ))
        })
    }
}
