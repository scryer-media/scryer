//! Jobs whose files an owner plans itself, each fetched from a link the
//! owner resolves on demand.
//!
//! A debrid-style service hands back a remote item as a list of files with
//! relative paths, and a download link has to be asked for per file. Such
//! links may only be started for a while, so they are never stored: the job
//! records the owner's opaque key per file, and the engine asks the owner's
//! [`LinkSource`] for a link each time a file starts or resumes, and again
//! when the server refuses the link in use.
//!
//! Every file lands at `<download_dir>/<job folder>/<relative path>`. The
//! folder and every path component are checked to be plain names, so no
//! file can be written outside the job folder.
//!
//! One file is fetched over one connection at a time, and a job's files are
//! fetched one after another; a resolved link belongs to the transfer of the
//! one file it was resolved for and is dropped when that transfer ends. A
//! link is therefore never used by more than one connection at once
//! ([`CONNECTIONS_PER_LINK`]), within any per-link cap a service sets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, http_release_refusal};
use scryer_plugin_sdk::{
    HttpResourceRole, HttpTransport, PluginHttpResource, PluginHttpValidators, PluginHttpVerify,
};

use super::state::{HttpFileRole, HttpJob, HttpJobFile, HttpJobState};

/// Connections the engine opens for one resolved link at a time. Files are
/// never split into ranged parts fetched in parallel.
pub(crate) const CONNECTIONS_PER_LINK: usize = 1;
/// Files one planned job may hold.
pub(crate) const MAX_LINKED_JOB_FILES: usize = 1000;
/// Longest link key the queue records.
pub(crate) const MAX_LINK_KEY_LEN: usize = 1024;
/// Longest single path component, in bytes.
const MAX_COMPONENT_LEN: usize = 255;
/// Longest relative path, in bytes.
const MAX_RELATIVE_PATH_LEN: usize = 1024;
/// Names Windows reserves for devices, whatever the extension.
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Why the engine is asking for a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkPurpose {
    /// The file has no bytes yet.
    Start,
    /// The file continues from bytes an earlier transfer wrote.
    Resume,
    /// The server refused the link in use.
    Refused,
}

/// One request for a link.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LinkRequest<'a> {
    pub job_id: &'a str,
    /// The key the owner planned the file with.
    pub key: &'a str,
    pub purpose: LinkPurpose,
}

/// A URL resolved for one transfer. It is kept in memory only, and its
/// debug form names the host alone, so it cannot reach a log by accident.
pub(crate) struct ResolvedLink {
    url: reqwest::Url,
}

impl ResolvedLink {
    pub(crate) fn new(url: reqwest::Url) -> Self {
        Self { url }
    }

    pub(crate) fn url(&self) -> &reqwest::Url {
        &self.url
    }
}

impl std::fmt::Debug for ResolvedLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedLink")
            .field("host", &self.url.host_str().unwrap_or_default())
            .finish_non_exhaustive()
    }
}

/// What a link source answered.
#[derive(Debug)]
pub(crate) enum LinkOutcome {
    Ready(ResolvedLink),
    /// No link can be had yet; ask again after `wait`. The engine waits and
    /// asks again instead of failing the file.
    RetryAfter {
        wait: Duration,
        message: String,
    },
    /// No link will be had; the file fails with this reason.
    Refused {
        reason: &'static str,
        message: String,
    },
}

/// Resolves a planned file's key to a download link. Injected into the
/// engine by the owner of the jobs that use it.
#[async_trait]
pub(crate) trait LinkSource: Send + Sync {
    async fn resolve(&self, request: &LinkRequest<'_>) -> LinkOutcome;
}

/// One file of a planned job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkedFile {
    pub role: HttpFileRole,
    /// `/`-separated, relative to the job folder.
    pub relative_path: String,
    /// Opaque to the engine; handed back to the link source.
    pub link_key: String,
    /// The file's exact size, when the service states it. A finished file
    /// of another size fails.
    pub size_bytes: Option<u64>,
    pub language: Option<String>,
}

/// A job an owner plans itself, for [`super::engine::HttpEngine::add_linked_job`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinkedJobRequest {
    pub download_id: Option<String>,
    pub title_id: String,
    pub title_name: String,
    pub facet: String,
    pub release_name: String,
    pub category: Option<String>,
    /// One plain folder name under the download directory.
    pub job_folder: String,
    pub files: Vec<LinkedFile>,
    pub expected_size_bytes: Option<i64>,
    /// Checked against every finished media file.
    pub expected_duration_seconds: Option<u64>,
    pub verify: PluginHttpVerify,
    /// A subtitle that cannot be fetched fails the job instead of being
    /// skipped.
    pub subtitle_required: bool,
}

fn path_refusal(message: String) -> AppError {
    AppError::validation_refused(http_release_refusal::FILE_PATH_INVALID, message)
}

/// A single plain path component: no separator, no `.` or `..`, nothing a
/// filesystem would read as a drive, a device or another name.
fn check_component(component: &str, whole: &str) -> AppResult<()> {
    let refuse = |why: &str| path_refusal(format!("the file path {whole:?} {why}"));
    if component.is_empty() {
        return Err(refuse("has an empty component"));
    }
    if component == "." || component == ".." {
        return Err(refuse("may not name `.` or `..`"));
    }
    if component.len() > MAX_COMPONENT_LEN {
        return Err(refuse("has a component longer than 255 bytes"));
    }
    if component
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
    {
        return Err(refuse(
            "holds a separator, a drive marker or a control character",
        ));
    }
    // Windows drops trailing dots and spaces, so `.. ` would mean `..`.
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(refuse("has a component ending in a dot or a space"));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_lowercase();
    if RESERVED_DEVICE_NAMES.contains(&stem.as_str()) {
        return Err(refuse("names a reserved device"));
    }
    Ok(())
}

/// The job folder: one plain component that is not hidden, so it can never
/// be the downloader's own state directory.
pub(crate) fn validate_job_folder(raw: &str) -> AppResult<&str> {
    check_component(raw, raw)?;
    if raw.starts_with('.') {
        return Err(path_refusal(format!(
            "the job folder {raw:?} may not start with a dot"
        )));
    }
    Ok(raw)
}

/// A relative path's components, refused unless every one is plain.
pub(crate) fn validate_relative_path(raw: &str) -> AppResult<Vec<&str>> {
    if raw.is_empty() {
        return Err(path_refusal("a file path is empty".into()));
    }
    if raw.len() > MAX_RELATIVE_PATH_LEN {
        return Err(path_refusal(format!(
            "a file path is longer than {MAX_RELATIVE_PATH_LEN} bytes"
        )));
    }
    if raw.starts_with('/') || Path::new(raw).is_absolute() {
        return Err(path_refusal(format!("the file path {raw:?} is absolute")));
    }
    let components: Vec<&str> = raw.split('/').collect();
    for component in &components {
        check_component(component, raw)?;
    }
    Ok(components)
}

/// Where each file lands, refused when a path is not plain, two files share
/// a name (compared without case, as some filesystems do), or a file sits
/// where another file needs a folder.
pub(crate) fn plan_linked_paths(
    output_dir: &Path,
    files: &[LinkedFile],
) -> AppResult<Vec<PathBuf>> {
    let mut file_names: HashSet<String> = HashSet::with_capacity(files.len());
    let mut folder_names: HashSet<String> = HashSet::new();
    let mut planned = Vec::with_capacity(files.len());
    for file in files {
        let components = validate_relative_path(&file.relative_path)?;
        let folded = components.join("/").to_lowercase();
        let mut prefix = String::new();
        for component in &components[..components.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(&component.to_lowercase());
            if file_names.contains(&prefix) {
                return Err(path_refusal(format!(
                    "the file path {:?} runs through another file of the job",
                    file.relative_path
                )));
            }
            folder_names.insert(prefix.clone());
        }
        if folder_names.contains(&folded) || !file_names.insert(folded) {
            return Err(path_refusal(format!(
                "the file path {:?} collides with another file of the job",
                file.relative_path
            )));
        }
        planned.push(
            components
                .iter()
                .fold(output_dir.to_path_buf(), |path, component| {
                    path.join(component)
                }),
        );
    }
    Ok(planned)
}

/// The resource a planned file is recorded with: no URL of its own, since
/// its link is resolved for each transfer.
pub(crate) fn linked_resource(role: HttpFileRole, size_bytes: Option<u64>) -> PluginHttpResource {
    PluginHttpResource {
        transport: HttpTransport::DirectFile,
        role: match role {
            HttpFileRole::Media => HttpResourceRole::Muxed,
            HttpFileRole::Subtitle => HttpResourceRole::Subtitle,
        },
        url: String::new(),
        expires_at: None,
        headers: Vec::new(),
        auth_ref: None,
        size_bytes: size_bytes.and_then(|size| i64::try_from(size).ok()),
        size_estimate: None,
        validators: PluginHttpValidators::default(),
        container: None,
        video: None,
        audio: None,
        subtitle: None,
        separate_tracks: false,
        range_support: Default::default(),
    }
}

/// The queued job for a planned request, or the refusal that keeps it out.
pub(crate) fn plan_linked_job(
    request: LinkedJobRequest,
    download_dir: &Path,
    id: String,
    now: DateTime<Utc>,
) -> AppResult<HttpJob> {
    let folder = validate_job_folder(&request.job_folder)?;
    if request.files.len() > MAX_LINKED_JOB_FILES {
        return Err(AppError::validation_refused(
            http_release_refusal::TOO_MANY_FILES,
            format!(
                "the download names {} files; at most {MAX_LINKED_JOB_FILES} are accepted",
                request.files.len()
            ),
        ));
    }
    if !request
        .files
        .iter()
        .any(|file| file.role == HttpFileRole::Media)
    {
        return Err(AppError::validation_refused(
            http_release_refusal::NO_MEDIA_RESOURCE,
            "the download names no media file",
        ));
    }
    if let Some(file) = request
        .files
        .iter()
        .find(|file| file.link_key.trim().is_empty() || file.link_key.len() > MAX_LINK_KEY_LEN)
    {
        return Err(AppError::validation_refused(
            http_release_refusal::LINK_KEY_INVALID,
            format!(
                "the file {:?} has an empty link key or one longer than {MAX_LINK_KEY_LEN} bytes",
                file.relative_path
            ),
        ));
    }
    let output_dir = download_dir.join(folder);
    let final_paths = plan_linked_paths(&output_dir, &request.files)?;
    let files = request
        .files
        .into_iter()
        .zip(final_paths)
        .map(|(file, final_path)| HttpJobFile {
            role: file.role,
            resource: linked_resource(file.role, file.size_bytes),
            final_path,
            partial_path: None,
            publication: None,
            attempt: 1,
            bytes_done: 0,
            total_bytes: None,
            validator: None,
            complete: false,
            language: file.language,
            forbidden_answers: 0,
            skipped_error: None,
            skipped_reason: None,
            link_key: Some(file.link_key),
        })
        .collect();
    Ok(HttpJob {
        id,
        download_id: request.download_id,
        title_id: request.title_id,
        title_name: request.title_name,
        facet: request.facet,
        release_name: request.release_name,
        category: request
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        output_basename: folder.to_string(),
        output_dir,
        files,
        state: HttpJobState::Queued,
        error: None,
        error_reason: None,
        expected_size_bytes: request.expected_size_bytes,
        expected_duration_seconds: request.expected_duration_seconds,
        verify: request.verify,
        queued_at: now,
        updated_at: now,
        completed_at: None,
        media_fallbacks: Vec::new(),
        candidate_index: 0,
        forbidden_requests: 0,
        refused_variants: Vec::new(),
        geo_restricted: false,
        subtitle_required: request.subtitle_required,
        progress_revision: 0,
    })
}
