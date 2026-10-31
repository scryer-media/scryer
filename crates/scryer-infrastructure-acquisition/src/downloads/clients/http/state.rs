//! The durable queue of the native HTTP downloader.
//!
//! One JSON document per configured client, under the client's own
//! `download_dir/.scryer-http/`. It holds every job the client knows about,
//! each file's destination, the attempt it is on, and the byte offset that
//! has been durably written. A restart reads it back and resumes from there.
//!
//! The document is replaced atomically (write a sibling, flush, rename over
//! the previous document), so a crash mid-write leaves the last complete
//! document in place. A document that does not parse is reported, never
//! overwritten: the client refuses to start rather than forget its jobs.
//!
//! The document changes only when a job does: it is added, removed, changes
//! state, finishes a file or moves to another media candidate. Byte progress
//! goes to a small file per job under `progress/` instead, so a transfer's
//! checkpoints never rewrite the whole queue. A progress file carries the
//! job's progress revision; on load it replaces the document's file fields
//! only when it is newer than the revision the document recorded.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::{PluginHttpResource, PluginHttpVerify};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

/// The directory under `download_dir` that holds the client's own state.
pub(crate) const STATE_DIR_NAME: &str = ".scryer-http";

const STATE_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HttpJobState {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

impl HttpJobState {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HttpFileRole {
    Media,
    Subtitle,
}

/// The representation identity a partial file was written against. A resume
/// is only trusted when the server still answers for the same one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecordedValidator {
    /// A strong entity tag. Weak tags never qualify a resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

impl RecordedValidator {
    /// The value for `If-Range`: the strong entity tag, else the date.
    pub(crate) fn if_range_value(&self) -> Option<&str> {
        self.etag.as_deref().or(self.last_modified.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HttpJobFile {
    pub role: HttpFileRole,
    pub resource: PluginHttpResource,
    /// The finished file. Bytes are written to an attempt-specific `.part`
    /// sibling and renamed here only once complete and verified.
    pub final_path: PathBuf,
    /// The actual partial name, independent of a probed final extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_path: Option<PathBuf>,
    /// Durable identity recorded before publishing the verified bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicationCheckpoint>,
    /// 1-based. A resume that cannot be trusted moves to the next attempt,
    /// which writes a fresh `.part` file; earlier partials are left in place.
    pub attempt: u32,
    /// Bytes of the current attempt that were flushed to disk before this
    /// document recorded them.
    pub bytes_done: u64,
    /// The representation length, when the server stated it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<RecordedValidator>,
    pub complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `403 Forbidden` answers this resource has given. Recorded so the
    /// retry bound holds across pauses and restarts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub forbidden_answers: u32,
    /// Why an optional file was given up on. A skipped file is not tried
    /// again and does not hold the job back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublicationCheckpoint {
    pub attempt: u32,
    pub partial_path: PathBuf,
    pub final_path: PathBuf,
    pub size: u64,
    pub sha256: Vec<u8>,
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A media resource that was given up on, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RefusedVariant {
    /// The server kept answering `403 Forbidden`.
    pub forbidden: bool,
    pub error: String,
}

impl HttpJobFile {
    /// `<final>.part` for the first attempt, `<final>.<n>.part` after it.
    pub(crate) fn part_path(&self) -> PathBuf {
        self.partial_path
            .clone()
            .unwrap_or_else(|| part_path_for(&self.final_path, self.attempt))
    }
}

pub(crate) fn part_path_for(final_path: &Path, attempt: u32) -> PathBuf {
    let file_name = final_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part_name = if attempt <= 1 {
        format!("{file_name}.part")
    } else {
        format!("{file_name}.{attempt}.part")
    };
    final_path.with_file_name(part_name)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HttpJob {
    /// The client item id this client minted for the job.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_id: Option<String>,
    pub title_id: String,
    pub title_name: String,
    pub facet: String,
    pub release_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub output_basename: String,
    pub output_dir: PathBuf,
    pub files: Vec<HttpJobFile>,
    pub state: HttpJobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_size_bytes: Option<i64>,
    /// The running time the indexer stated, checked against the finished
    /// media file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_duration_seconds: Option<u64>,
    /// How the finished media file is checked.
    #[serde(default)]
    pub verify: PluginHttpVerify,
    pub queued_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// The other complete media resources of the release, best first, tried
    /// in order when the active one is refused. Bounded at planning time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_fallbacks: Vec<PluginHttpResource>,
    /// Which media candidate is active: 0 for the first choice, `n` for
    /// `media_fallbacks[n - 1]`.
    #[serde(default)]
    pub candidate_index: u32,
    /// `403 Forbidden` answers across every media candidate of the job.
    #[serde(default)]
    pub forbidden_requests: u32,
    /// The media candidates given up on so far, in the order they were tried.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused_variants: Vec<RefusedVariant>,
    /// The indexer declared the release geo-restricted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub geo_restricted: bool,
    /// The selection required subtitle sidecars, so a subtitle that cannot be
    /// fetched fails the job instead of being skipped.
    #[serde(default, skip_serializing_if = "is_false")]
    pub subtitle_required: bool,
    /// Bumped on every recorded change to the job's files. The progress file
    /// written with a higher revision than the document's is the newer record
    /// of those files.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub progress_revision: u64,
}

impl HttpJob {
    pub(crate) fn media_file(&self) -> Option<&HttpJobFile> {
        self.files
            .iter()
            .find(|file| file.role == HttpFileRole::Media)
    }

    /// Bytes written so far, counting finished files at their full length.
    pub(crate) fn durable_bytes(&self) -> u64 {
        self.files
            .iter()
            .map(|file| {
                if file.complete {
                    file.total_bytes.unwrap_or(file.bytes_done)
                } else {
                    file.bytes_done
                }
            })
            .sum()
    }

    /// The whole job's size when every file's length is known.
    pub(crate) fn known_total_bytes(&self) -> Option<u64> {
        // A skipped file is never delivered, so it is not part of the size.
        let mut delivered = self
            .files
            .iter()
            .filter(|file| file.skipped_error.is_none());
        delivered.try_fold(0u64, |sum, file| {
            let size = file.total_bytes.or_else(|| {
                file.resource
                    .size_bytes
                    .and_then(|size| u64::try_from(size).ok())
            })?;
            Some(sum.saturating_add(size))
        })
    }

    /// Why the job failed once no media candidate is left to try.
    pub(crate) fn exhausted_message(&self) -> String {
        let variants = self.refused_variants.len();
        let Some(last) = self.refused_variants.last() else {
            return "no media resource could be downloaded".to_string();
        };
        if self
            .refused_variants
            .iter()
            .all(|variant| variant.forbidden)
        {
            let mut message = format!(
                "geo-blocked: every variant answered 403 Forbidden ({}, {})",
                counted(variants, "variant"),
                counted(self.forbidden_requests as usize, "request")
            );
            if self.geo_restricted {
                message.push_str("; the release is declared geo-restricted");
            }
            return message;
        }
        if variants == 1 {
            return last.error.clone();
        }
        format!(
            "no variant could be downloaded ({}); the last one failed with: {}",
            counted(variants, "variant"),
            last.error
        )
    }
}

fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct HttpQueueDocument {
    pub version: u32,
    pub jobs: Vec<HttpJob>,
}

/// The document as written: borrowed, so a save never copies the jobs.
#[derive(Serialize)]
struct HttpQueueDocumentRef<'a> {
    version: u32,
    jobs: &'a [HttpJob],
}

/// One job's byte progress, written at the transfer's checkpoint cadence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HttpJobProgress {
    /// The job's progress revision when this was written.
    pub revision: u64,
    /// One entry per job file, in the job's file order.
    pub files: Vec<HttpFileProgress>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct HttpFileProgress {
    pub attempt: u32,
    pub bytes_done: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<RecordedValidator>,
}

impl HttpJobProgress {
    pub(crate) fn of(job: &HttpJob) -> Self {
        Self {
            revision: job.progress_revision,
            files: job
                .files
                .iter()
                .map(|file| HttpFileProgress {
                    attempt: file.attempt,
                    bytes_done: file.bytes_done,
                    total_bytes: file.total_bytes,
                    validator: file.validator.clone(),
                })
                .collect(),
        }
    }

    /// Apply this progress to `job` when it is newer than what the document
    /// recorded and still describes the same files. Only the byte-progress
    /// fields change; whether a file is complete stays the document's word.
    pub(crate) fn apply_if_newer(&self, job: &mut HttpJob) -> bool {
        if self.revision <= job.progress_revision || self.files.len() != job.files.len() {
            return false;
        }
        for (file, progress) in job.files.iter_mut().zip(&self.files) {
            if file.complete {
                continue;
            }
            file.attempt = progress.attempt;
            file.bytes_done = progress.bytes_done;
            file.total_bytes = progress.total_bytes;
            file.validator = progress.validator.clone();
        }
        job.progress_revision = self.revision;
        true
    }
}

/// Reads and atomically replaces one client's queue document.
#[derive(Clone, Debug)]
pub(crate) struct HttpStateStore {
    path: PathBuf,
    progress_dir: PathBuf,
}

/// The directory under the state directory that holds per-job progress.
const PROGRESS_DIR_NAME: &str = "progress";

impl HttpStateStore {
    pub(crate) fn for_client(download_dir: &Path, client_id: &str) -> Self {
        let state_dir = download_dir.join(STATE_DIR_NAME);
        Self {
            path: state_dir.join(format!("queue-{}.json", safe_file_stem(client_id))),
            progress_dir: state_dir.join(PROGRESS_DIR_NAME),
        }
    }

    /// Where a job's byte progress is kept.
    pub(crate) fn progress_path(&self, job_id: &str) -> PathBuf {
        self.progress_dir
            .join(format!("{}.json", safe_file_stem(job_id)))
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The stored document, or an empty one when none was ever written.
    pub(crate) async fn load(&self) -> AppResult<HttpQueueDocument> {
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HttpQueueDocument {
                    version: STATE_FORMAT_VERSION,
                    jobs: Vec::new(),
                });
            }
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "the HTTP downloader could not read its queue at {}: {error}",
                    self.path.display()
                )));
            }
        };
        let document: HttpQueueDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Repository(format!(
                "the HTTP downloader's queue at {} is unreadable ({error}); it was left untouched",
                self.path.display()
            ))
        })?;
        if document.version > STATE_FORMAT_VERSION {
            return Err(AppError::Repository(format!(
                "the HTTP downloader's queue at {} was written by a newer version (format {})",
                self.path.display(),
                document.version
            )));
        }
        Ok(document)
    }

    /// Create the state directory and write the document when none exists,
    /// proving the directory is writable without touching anything else.
    pub(crate) async fn initialize(&self) -> AppResult<()> {
        let parent = self.state_dir()?;
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::ACQUISITION_DOWNLOAD_DIRECTORY_NOT_WRITABLE,
                format!(
                    "the download directory is not writable ({}): {error}",
                    parent.display()
                ),
            )
        })?;
        match tokio::fs::metadata(&self.path).await {
            Ok(_) => {
                // Opening for write without truncation proves the document
                // can be replaced later and changes nothing.
                tokio::fs::OpenOptions::new()
                    .write(true)
                    .open(&self.path)
                    .await
                    .map(|_| ())
                    .map_err(|error| {
                        AppError::validation_refused(
                            crate::error_reasons::ACQUISITION_HTTP_QUEUE_NOT_WRITABLE,
                            format!(
                                "the HTTP downloader's queue at {} is not writable: {error}",
                                self.path.display()
                            ),
                        )
                    })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.save(&HttpQueueDocument {
                    version: STATE_FORMAT_VERSION,
                    jobs: Vec::new(),
                })
                .await
            }
            Err(error) => Err(AppError::validation_refused(
                crate::error_reasons::ACQUISITION_HTTP_QUEUE_UNREADABLE,
                format!(
                    "the HTTP downloader's queue at {} cannot be inspected: {error}",
                    self.path.display()
                ),
            )),
        }
    }

    pub(crate) async fn save(&self, document: &HttpQueueDocument) -> AppResult<()> {
        let parent = self.state_dir()?;
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::Repository(format!(
                "the HTTP downloader could not create {}: {error}",
                parent.display()
            ))
        })?;
        let bytes = serde_json::to_vec(&HttpQueueDocumentRef {
            version: STATE_FORMAT_VERSION,
            jobs: &document.jobs,
        })
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let staging = self.path.with_extension("json.next");
        let write = async {
            let mut file = tokio::fs::File::create(&staging).await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&staging, &self.path).await?;
            // The rename lives in the directory; without this a crash right
            // after it could bring the previous document back.
            sync_directory(parent).await
        };
        write.await.map_err(|error| {
            AppError::Repository(format!(
                "the HTTP downloader could not record its queue at {}: {error}",
                self.path.display()
            ))
        })
    }

    /// Replace one job's progress file. Only the data is synced: losing the
    /// newest progress to a crash only means resuming from an earlier offset
    /// the partial file still holds.
    pub(crate) async fn save_progress(
        &self,
        job_id: &str,
        progress: &HttpJobProgress,
    ) -> AppResult<()> {
        let path = self.progress_path(job_id);
        let bytes = serde_json::to_vec(progress)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let staging = path.with_extension("json.next");
        let write = async {
            tokio::fs::create_dir_all(&self.progress_dir).await?;
            let mut file = tokio::fs::File::create(&staging).await?;
            file.write_all(&bytes).await?;
            file.sync_data().await?;
            drop(file);
            tokio::fs::rename(&staging, &path).await
        };
        write.await.map_err(|error| {
            AppError::Repository(format!(
                "the HTTP downloader could not record progress at {}: {error}",
                path.display()
            ))
        })
    }

    /// A job's progress file, when one was written and can be read. An
    /// unreadable one is ignored: the document's own offsets still hold.
    pub(crate) async fn load_progress(&self, job_id: &str) -> Option<HttpJobProgress> {
        let path = self.progress_path(job_id);
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "HTTP downloader could not read a progress file");
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(progress) => Some(progress),
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "HTTP downloader ignored an unreadable progress file");
                None
            }
        }
    }

    fn state_dir(&self) -> AppResult<&Path> {
        self.path.parent().ok_or_else(|| {
            AppError::Repository("the HTTP downloader state path has no parent".into())
        })
    }
}

/// A file-name stem that cannot leave its directory.
fn safe_file_stem(raw: &str) -> String {
    let safe: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .take(96)
        .collect();
    if safe.is_empty() {
        "default".to_string()
    } else {
        safe
    }
}

#[cfg(unix)]
async fn sync_directory(dir: &Path) -> std::io::Result<()> {
    tokio::fs::File::open(dir).await?.sync_all().await
}

/// Windows cannot open a directory as a file; its renames are not made
/// durable this way.
#[cfg(not(unix))]
async fn sync_directory(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}
