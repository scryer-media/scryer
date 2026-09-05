async fn remap_completed_download_for_client(app: &AppUseCase, completed: &mut CompletedDownload) {
    let client_id = completed.client_id.trim();
    if client_id.is_empty() {
        return;
    }

    let config = match app
        .services
        .integrations
        .download_client_configs
        .get_by_id(client_id)
        .await
    {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(
                client_id,
                error = %error,
                "import: failed to load download client config for remote path mapping"
            );
            return;
        }
    };

    match parse_download_client_remote_path_mappings(&config.config_json) {
        Ok(mappings) => apply_remote_path_mappings_to_completed_download(completed, &mappings),
        Err(error) => {
            tracing::warn!(
                client_id,
                error = %error,
                "import: failed to parse remote path mappings"
            );
        }
    }
}

#[derive(Clone, Debug)]
struct CompletedDownloadSubmissionMatch {
    submission: DownloadSubmission,
    identity: Option<DownloadSubmissionIdentity>,
}

#[derive(Clone, Debug)]
enum CompletedDownloadSubmissionResolution {
    Matched(Box<CompletedDownloadSubmissionMatch>),
    DownloaderObservation,
    MissingDownloadId {
        identity: DownloadSubmissionIdentity,
    },
    AmbiguousDownloadId {
        download_id: String,
        matches: usize,
    },
}

/// The only release-name evidence that may enter import matching or scoring.
/// Downloader display labels, categories, parameters, and destination folders
/// intentionally have no representation here.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum ReleaseEvidence {
    ScryerSubmission {
        title_id: String,
        facet: String,
        /// The indexer release title persisted at grab time
        /// (`download_submissions.source_title`). `None` when the grab was
        /// recorded without one (an `addTitle{sourceHint}` API grab, or a
        /// legacy row); the Scryer identity still stands and only the name
        /// degrades to `observed_release_name`, then the source video's stem.
        #[serde(default)]
        source_title: Option<String>,
        /// The client-reported release name observed at completion. Only the
        /// fallback name when `source_title` is missing; never the identity.
        #[serde(default)]
        observed_release_name: Option<String>,
        /// The size the indexer announced for this grab
        /// (`download_submissions.release_size_bytes`); the import scores the
        /// size term on it when the landed file is within the overhead band.
        #[serde(default)]
        release_size_bytes: Option<i64>,
        purpose: crate::DownloadSubmissionPurpose,
        scope: SubmissionScope,
    },
    DownloaderObservation {
        #[serde(default)]
        release_name: Option<String>,
    },
}

/// The persisted indexer release title of a Scryer grab, or `None` when the
/// submission row carries no usable one (NULL/blank).
fn submission_source_title(submission: &DownloadSubmission) -> Option<String> {
    submission
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn completed_observed_release_name(completed: &CompletedDownload) -> Option<String> {
    completed
        .release_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn source_video_stem(source_video: Option<&Path>) -> Option<String> {
    source_video.and_then(|path| {
        path.file_stem()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

impl ReleaseEvidence {
    /// Durable Scryer-grab evidence. A submission without a persisted release
    /// title keeps its identity (title/facet/scope/purpose) and degrades only
    /// the name to what the client reported at completion; it never fails, so
    /// the download stays importable (automatic, manual, and retry).
    fn from_submission(submission: &DownloadSubmission, completed: &CompletedDownload) -> Self {
        let source_title = submission_source_title(submission);
        let observed_release_name = completed_observed_release_name(completed);
        if source_title.is_none() {
            tracing::warn!(
                client_id = ?submission.download_client_id,
                client_type = %submission.download_client_type,
                download_client_item_id = %submission.download_client_item_id,
                title_id = %submission.title_id,
                observed_release_name = ?observed_release_name,
                "import: Scryer submission has no persisted release title; using the client-reported release name for parsing"
            );
        }
        Self::ScryerSubmission {
            title_id: submission.title_id.clone(),
            facet: submission.facet.clone(),
            source_title,
            observed_release_name,
            release_size_bytes: submission.release_size_bytes,
            purpose: submission.purpose,
            scope: submission.scope.clone(),
        }
    }

    fn from_completed_observation(completed: &CompletedDownload) -> Self {
        Self::DownloaderObservation {
            release_name: completed_observed_release_name(completed),
        }
    }

    pub(crate) fn title_id(&self) -> Option<&str> {
        match self {
            Self::ScryerSubmission { title_id, .. } => Some(title_id),
            Self::DownloaderObservation { .. } => None,
        }
    }

    pub(crate) fn facet(&self) -> Option<&str> {
        match self {
            Self::ScryerSubmission { facet, .. } => Some(facet),
            Self::DownloaderObservation { .. } => None,
        }
    }

    pub(crate) fn scope(&self) -> Option<&SubmissionScope> {
        match self {
            Self::ScryerSubmission { scope, .. } => Some(scope),
            Self::DownloaderObservation { .. } => None,
        }
    }

    /// The size the indexer announced for a Scryer grab; `None` for adopted
    /// downloads and grabs recorded without one.
    pub(crate) fn announced_size_bytes(&self) -> Option<i64> {
        match self {
            Self::ScryerSubmission {
                release_size_bytes, ..
            } => *release_size_bytes,
            Self::DownloaderObservation { .. } => None,
        }
    }

    pub(crate) fn purpose(&self) -> crate::DownloadSubmissionPurpose {
        match self {
            Self::ScryerSubmission { purpose, .. } => *purpose,
            Self::DownloaderObservation { .. } => crate::DownloadSubmissionPurpose::Standard,
        }
    }

    /// Whether a completed download was selected by an operator. Client-only
    /// observations have no such durable intent and remain automatic.
    pub(crate) fn import_origin(&self) -> crate::import_decide::ImportOrigin {
        crate::import_decide::ImportOrigin::from_submission_purpose(self.purpose())
    }

    /// The release name to parse and score: the persisted indexer title for a
    /// Scryer grab, else the client-reported release name observed at
    /// completion, else the source video's file stem.
    pub(crate) fn release_title<'a>(&'a self, source_video: Option<&'a Path>) -> Option<String> {
        match self {
            Self::ScryerSubmission {
                source_title,
                observed_release_name,
                ..
            } => source_title
                .clone()
                .or_else(|| observed_release_name.clone())
                .or_else(|| source_video_stem(source_video)),
            Self::DownloaderObservation { release_name } => release_name
                .clone()
                .or_else(|| source_video_stem(source_video)),
        }
    }
}

fn release_evidence_for_resolution(
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
) -> ReleaseEvidence {
    match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched)
            if submission_has_scryer_origin(&matched.submission) =>
        {
            ReleaseEvidence::from_submission(&matched.submission, completed)
        }
        CompletedDownloadSubmissionResolution::Matched(_)
        | CompletedDownloadSubmissionResolution::DownloaderObservation
        | CompletedDownloadSubmissionResolution::MissingDownloadId { .. }
        | CompletedDownloadSubmissionResolution::AmbiguousDownloadId { .. } => {
            ReleaseEvidence::from_completed_observation(completed)
        }
    }
}

pub(crate) async fn resolve_release_evidence_for_completed_download(
    app: &AppUseCase,
    completed: &CompletedDownload,
    item: Option<&DownloadQueueItem>,
) -> AppResult<ReleaseEvidence> {
    let resolution = resolve_completed_download_submission(app, completed, item).await?;
    Ok(release_evidence_for_resolution(completed, &resolution))
}

const DOWNLOAD_SUBMISSION_VISIBILITY_GRACE_SECONDS: i64 = 15;

fn download_submission_persistence_may_be_in_flight(
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
    now: DateTime<Utc>,
) -> bool {
    if !matches!(
        resolution,
        CompletedDownloadSubmissionResolution::MissingDownloadId { .. }
    ) {
        return false;
    }

    let Some(completed_at) = completed.completed_at else {
        return false;
    };
    let age = now.signed_duration_since(completed_at);
    age >= chrono::Duration::zero()
        && age < chrono::Duration::seconds(DOWNLOAD_SUBMISSION_VISIBILITY_GRACE_SECONDS)
}

fn completed_download_observed_identity(
    completed: &CompletedDownload,
) -> DownloadSubmissionIdentity {
    crate::observed_download_identity(crate::ObservedDownloadIdentityInput {
        download_id: completed.download_id.as_deref(),
        parameters: &completed.parameters,
        info_hash_hint: None,
    })
}

fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
}

fn submission_source_identity(submission: &DownloadSubmission) -> ClientJobLocator {
    ClientJobLocator::from_submission(submission)
}

async fn resolve_completed_download_submission(
    app: &AppUseCase,
    completed: &CompletedDownload,
    item: Option<&DownloadQueueItem>,
) -> AppResult<CompletedDownloadSubmissionResolution> {
    let observed_identity = completed_download_observed_identity(completed);
    if let Some(download_id) = observed_identity.download_id.as_deref().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }) {
        let download_id_submissions = app
            .services
            .workflow
            .download_submissions
            .list_by_download_id(
                Some(completed.client_id.as_str()),
                &completed.client_type,
                download_id,
            )
            .await?;

        if download_id_submissions.is_empty() {
            return Ok(CompletedDownloadSubmissionResolution::MissingDownloadId {
                identity: observed_identity,
            });
        }
        if let Some(submission) =
            crate::download_identity::coalesce_download_submissions_by_release_attempt(
                &download_id_submissions,
            )
        {
            return matched_completed_download_submission(app, submission, &observed_identity)
                .await;
        }
        return Ok(CompletedDownloadSubmissionResolution::AmbiguousDownloadId {
            download_id: download_id.to_string(),
            matches: download_id_submissions.len(),
        });
    }

    if !download_submission_identity_is_empty(&observed_identity) {
        return Ok(CompletedDownloadSubmissionResolution::MissingDownloadId {
            identity: observed_identity,
        });
    }

    let mut source_identities = vec![completed_download_identity(completed)];
    if let Some(item) = item {
        source_identities.push(ClientJobLocator::new(
            Some(item.client_id.as_str()),
            &item.client_type,
            &item.download_client_item_id,
        ));
    }
    for source_identity in source_identities {
        if let Some(submission) = app
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&source_identity)
            .await?
        {
            return matched_completed_download_submission(app, submission, &observed_identity)
                .await;
        }
    }

    Ok(CompletedDownloadSubmissionResolution::DownloaderObservation)
}

pub(crate) async fn recent_download_submission_persistence_is_pending(
    app: &AppUseCase,
    completed: &CompletedDownload,
) -> AppResult<bool> {
    let resolution = resolve_completed_download_submission(app, completed, None).await?;
    Ok(download_submission_persistence_may_be_in_flight(
        completed,
        &resolution,
        Utc::now(),
    ))
}

async fn submission_identity_for_submission(
    app: &AppUseCase,
    submission: &DownloadSubmission,
) -> AppResult<DownloadSubmissionIdentity> {
    Ok(app
        .services
        .workflow
        .download_submissions
        .get_submission_identity(&submission_source_identity(submission))
        .await?
        .unwrap_or_default())
}

async fn matched_completed_download_submission(
    app: &AppUseCase,
    submission: DownloadSubmission,
    observed_identity: &DownloadSubmissionIdentity,
) -> AppResult<CompletedDownloadSubmissionResolution> {
    let stored_identity = submission_identity_for_submission(app, &submission).await?;
    let identity = if download_submission_identity_is_empty(&stored_identity) {
        (!download_submission_identity_is_empty(observed_identity))
            .then_some(observed_identity.clone())
    } else {
        Some(stored_identity)
    };
    Ok(CompletedDownloadSubmissionResolution::Matched(Box::new(
        CompletedDownloadSubmissionMatch {
            submission,
            identity,
        },
    )))
}

pub(crate) enum ResolvedCompletedDownloadOriginForImport {
    Ready {
        completed: Box<CompletedDownload>,
        release_evidence: ReleaseEvidence,
    },
    NoScryerOrigin,
}

pub(crate) async fn resolve_completed_download_origin_for_import(
    app: &AppUseCase,
    completed: &CompletedDownload,
    item: Option<&DownloadQueueItem>,
) -> AppResult<ResolvedCompletedDownloadOriginForImport> {
    let provenance = resolve_import_provenance(
        app,
        completed.clone(),
        ImportProvenanceRequest {
            identity_policy: CompletedImportIdentityPolicy::RequireSubmission,
            queue_item: item,
            requested_target_title_id: None,
            release_evidence_override: None,
            // The tracked path decides its import target from the LIVE row
            // only; a lost row is replayed by the import request itself.
            persisted: None,
            tolerate_lookup_failure: false,
        },
    )
    .await?;
    Ok(if provenance.release_evidence.title_id().is_some() {
        ResolvedCompletedDownloadOriginForImport::Ready {
            completed: Box::new(provenance.completed),
            release_evidence: provenance.release_evidence,
        }
    } else {
        ResolvedCompletedDownloadOriginForImport::NoScryerOrigin
    })
}

fn completed_download_import_identity_for_resolution(
    completed: &CompletedDownload,
    resolution: &CompletedDownloadSubmissionResolution,
) -> Option<DownloadSubmissionIdentity> {
    let observed_identity = completed_download_observed_identity(completed);
    if !download_submission_identity_is_empty(&observed_identity) {
        return Some(observed_identity);
    }

    match resolution {
        CompletedDownloadSubmissionResolution::Matched(matched) => matched.identity.clone(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ImportPathSettings {
    pub(crate) media_root: String,
    pub(crate) rename_enabled: bool,
    pub(crate) rename_template: String,
    pub(crate) folder_template: String,
    pub(crate) season_folder_template: String,
    pub(crate) specials_folder_template: String,
}
async fn ensure_import_title_folder_available(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    crate::folder_ownership::ensure_folder_available_to_title(app, title, folder_path).await
}

async fn persist_title_folder_path_if_missing(
    app: &AppUseCase,
    title: &Title,
    folder_path: &Path,
) -> AppResult<()> {
    let mut title = title.clone();
    crate::folder_ownership::claim_title_folder_if_missing(app, &mut title, folder_path).await
}
pub(crate) fn preserved_import_filename(source_path: &Path) -> String {
    let raw = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled");
    let sanitized = sanitize_filesystem_component(raw);
    if sanitized.is_empty() {
        "untitled".to_string()
    } else {
        sanitized
    }
}
#[cfg(test)]
fn sanitized_title_folder_component(raw: &str) -> String {
    let sanitized = sanitize_filesystem_component(raw);
    if sanitized.is_empty() {
        "untitled".to_string()
    } else {
        sanitized
    }
}
/// Recursively find all video files under `dir`, optionally filtering out samples.
///
/// `dir` is usually a directory, but download clients may report a completed
/// file path directly. Classify the source before walking it so a regular file
/// never reaches `read_dir` (Windows reports that mistake as OS error 267).
pub(crate) fn find_video_files(dir: &Path, filter_samples: bool) -> AppResult<Vec<PathBuf>> {
    fn single_file(path: &Path, filter_samples: bool) -> Vec<PathBuf> {
        (is_video_file(path) && (!filter_samples || !is_sample_file(path)))
            .then_some(path.to_path_buf())
            .into_iter()
            .collect()
    }

    let metadata = std::fs::metadata(dir).map_err(|error| AppError::ImportSourceInspection {
        path: dir.display().to_string(),
        message: error.to_string(),
    })?;
    if metadata.is_file() {
        tracing::info!(
            path = %dir.display(),
            "download path is a video file, not a directory"
        );
        return Ok(single_file(dir, filter_samples));
    }
    if !metadata.is_dir() {
        return Err(AppError::UnsupportedImportSource {
            path: dir.display().to_string(),
        });
    }

    let walked = match crate::filesystem_walk::FilesystemWalker::new()
        .skip_unreadable_subdirectories()
        .walk(dir)
    {
        Ok(walked) => walked,
        Err(walk_error) => match std::fs::metadata(dir) {
            Ok(metadata) if metadata.is_file() => return Ok(single_file(dir, filter_samples)),
            Ok(metadata) if metadata.is_dir() => {
                return Err(AppError::ImportSourceInspection {
                    path: dir.display().to_string(),
                    message: walk_error.to_string(),
                });
            }
            Ok(_) => {
                return Err(AppError::ImportSourceChanged {
                    path: dir.display().to_string(),
                    message: "source changed to an unsupported filesystem object".to_string(),
                });
            }
            Err(error) => {
                return Err(AppError::ImportSourceChanged {
                    path: dir.display().to_string(),
                    message: error.to_string(),
                });
            }
        },
    };

    Ok(walked
        .into_iter()
        .flat_map(|entry| entry.files.into_iter())
        .filter(|path| is_video_file(path))
        .filter(|path| !filter_samples || !is_sample_file(path))
        .collect())
}

/// The release-name claims a completed download makes about its own identity,
/// most authoritative first: the client-reported release name when the client
/// exposes one, else the stems of its video files — non-sample files largest
/// first (sample-only downloads fall back to every video, largest first). Never
/// the downloader display label or the destination folder. Shared by the
/// completion-time re-resolution and the identity proof so they cannot drift.
pub(crate) fn completed_download_release_claims(completed: &CompletedDownload) -> Vec<String> {
    if let Some(release_name) = completed
        .release_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return vec![release_name.to_string()];
    }
    let dest_dir = Path::new(&completed.dest_dir);
    let mut video_files = find_video_files(dest_dir, true).unwrap_or_default();
    if video_files.is_empty() {
        video_files = find_video_files(dest_dir, false).unwrap_or_default();
    }
    video_files.sort_by_cached_key(|file| {
        std::cmp::Reverse(
            std::fs::metadata(file)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        )
    });
    let mut claims = Vec::new();
    for stem in video_files.iter().filter_map(|file| {
        file.file_stem()
            .and_then(|name| name.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }) {
        if !claims.contains(&stem) {
            claims.push(stem);
        }
    }
    claims
}

pub(crate) fn pick_largest_file(files: &[PathBuf]) -> AppResult<PathBuf> {
    files
        .iter()
        .max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
        .cloned()
        .ok_or_else(|| AppError::Repository("no files to pick from".to_string()))
}
fn parsed_release_from_file_stem(path: &Path) -> ParsedReleaseMetadata {
    let fallback = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or(fallback);
    normalize_release_title_signal(parse_release_metadata(stem.as_str()))
}
fn parsed_release_from_folder_name(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| parse_release_metadata(value.as_str()))
        .map(normalize_release_title_signal)
}
fn parsed_release_from_parent_folder(path: &Path) -> Option<ParsedReleaseMetadata> {
    path.parent().and_then(parsed_release_from_folder_name)
}

fn parsed_usable_release_from_parent_folder(path: &Path) -> Option<ParsedReleaseMetadata> {
    parsed_release_from_parent_folder(path).filter(has_usable_release_title_signal)
}

fn parsed_usable_release_from_file_stem(path: &Path) -> Option<ParsedReleaseMetadata> {
    let fallback = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or(fallback);
    parse_usable_release_title(stem.as_str())
}

/// A video file an import is about to process, plus the name parsing should
/// read for it.
///
/// `physical` is the only path that ever reaches the filesystem: every fs,
/// artifact, move and cleanup call takes [`ImportVideoFile::path`]. A
/// `logical_name` is set only when srrdb recovered the original scene filename
/// for an obfuscated member, and it exists purely so title matching, episode
/// identity, pack planning and the movie fallback have something to parse. The
/// file on disk is never renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportVideoFile {
    pub(crate) physical: PathBuf,
    pub(crate) logical_name: Option<String>,
}

impl ImportVideoFile {
    /// A file with no recovered name: parsing reads the physical name, which is
    /// what every import did before filename recovery existed.
    pub(crate) fn physical(physical: PathBuf) -> Self {
        Self {
            physical,
            logical_name: None,
        }
    }

    /// The path on disk. Every filesystem, artifact, move and cleanup call uses
    /// this and nothing else.
    pub(crate) fn path(&self) -> &Path {
        &self.physical
    }

    /// The file name parsing should read: the recovered original name when
    /// there is one, else the physical file name.
    pub(crate) fn parse_name(&self) -> Cow<'_, str> {
        match self.logical_name.as_deref() {
            Some(logical_name) => Cow::Borrowed(logical_name),
            None => self.physical.file_name().map_or_else(
                || Cow::Borrowed(""),
                |name| match name.to_str() {
                    Some(name) => Cow::Borrowed(name),
                    None => Cow::Owned(name.to_string_lossy().into_owned()),
                },
            ),
        }
    }

    /// A parse-only path: the physical parent carrying [`Self::parse_name`].
    ///
    /// This exists so the existing `&Path` stem and parent-folder helpers keep
    /// working unchanged. It must never be handed to the filesystem; use
    /// [`Self::path`] for that.
    pub(crate) fn parse_path(&self) -> Cow<'_, Path> {
        if self.logical_name.is_none() {
            return Cow::Borrowed(self.physical.as_path());
        }
        let parse_name = self.parse_name();
        let parse_name: &str = &parse_name;
        Cow::Owned(match self.physical.parent() {
            Some(parent) => parent.join(parse_name),
            None => PathBuf::from(parse_name),
        })
    }
}

/// srrdb filename recovery for one `run_import` call.
///
/// Everything about it is per-import and in memory: the admin setting is read
/// at most once, results are memoized by physical path so the titleless probe
/// and the final file list never look the same file up twice, and the first
/// outage-class failure trips the breaker for the rest of this import. A retry
/// starts from scratch.
#[derive(Default)]
struct SrrdbFilenameRecovery {
    /// `None` until the admin setting has been read.
    enabled: Option<bool>,
    /// Physical path to the recovered name, or `None` for a file that was
    /// looked up and produced nothing. Absent means never looked up.
    resolved: HashMap<PathBuf, Option<String>>,
    /// Set by the first timeout, transport failure, 429 or 5xx.
    tripped: bool,
}

impl SrrdbFilenameRecovery {
    /// Whether this file has to be hashed and looked up.
    ///
    /// Two things must both hold. The file must carry no usable title signal
    /// in its own stem: a member that already names itself needs no help, and
    /// asking would be a third-party request for nothing. And the caller must
    /// say this file needs a name *of its own* — `needs_own_name` — because a
    /// well-named parent folder or release name identifies the title and
    /// season but never says which member is which episode. The caller owns
    /// that judgement; see the two `enrich` call sites in
    /// `resolve_completed_import_target`.
    fn is_candidate(path: &Path, needs_own_name: bool) -> bool {
        if !needs_own_name {
            return false;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("strm"))
        {
            return false;
        }
        parsed_usable_release_from_file_stem(path).is_none()
    }

    /// Pair each physical video file with the name parsing should read.
    ///
    /// `needs_own_name` is the caller's judgement that these files have to
    /// identify themselves individually; see [`Self::is_candidate`]. Names
    /// already memoized by an earlier pass are reapplied either way, so a file
    /// recovered by the titleless probe keeps its name here for free.
    ///
    /// Every failure of any kind yields a file with no recovered name, which is
    /// byte for byte what this import would have done with the feature off.
    async fn enrich(
        &mut self,
        app: &AppUseCase,
        completed: &CompletedDownload,
        video_files: Vec<PathBuf>,
        needs_own_name: bool,
    ) -> Vec<ImportVideoFile> {
        let mut files: Vec<ImportVideoFile> = video_files
            .into_iter()
            .map(ImportVideoFile::physical)
            .collect();

        // The port is absent in every assembly that does not wire the
        // production adapter, which is indistinguishable from the switch being
        // off: no setting read, no hashing, no request.
        let Some(lookup) = app
            .services
            .integrations
            .srrdb_filename_lookup
            .available()
            .cloned()
        else {
            return files;
        };

        let enabled = match self.enabled {
            Some(enabled) => enabled,
            None => {
                let enabled = app.srrdb_filename_recovery_enabled().await.unwrap_or(false);
                self.enabled = Some(enabled);
                enabled
            }
        };
        if !srrdb_lookup_applies(enabled, &completed.client_type) {
            return files;
        }

        for file in &mut files {
            if let Some(cached) = self.resolved.get(&file.physical) {
                file.logical_name = cached.clone();
                continue;
            }
            if !Self::is_candidate(&file.physical, needs_own_name) {
                continue;
            }
            if self.tripped {
                tracing::debug!(
                    file = %file.physical.display(),
                    "srrdb filename recovery skipped: the lookup is unavailable for this import"
                );
                continue;
            }

            let hash_path = file.physical.clone();
            let hashed = tokio::task::spawn_blocking(move || crc32_iso_hdlc_of_file(&hash_path))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let (crc, size_bytes) = match hashed {
                Ok(hashed) => hashed,
                Err(error) => {
                    tracing::debug!(
                        file = %file.physical.display(),
                        error = %error,
                        "srrdb filename recovery skipped: could not checksum the file"
                    );
                    self.resolved.insert(file.physical.clone(), None);
                    continue;
                }
            };
            let crc32_hex = format!("{crc:08X}");

            match lookup.recover_filename(&crc32_hex, size_bytes).await {
                Err(_) => {
                    self.tripped = true;
                    tracing::debug!(
                        file = %file.physical.display(),
                        crc32_hex,
                        "srrdb filename recovery unavailable; skipping the rest of this import"
                    );
                }
                Ok(None) => {
                    tracing::debug!(
                        file = %file.physical.display(),
                        crc32_hex,
                        size_bytes,
                        "srrdb had no unambiguous original filename for this file"
                    );
                    self.resolved.insert(file.physical.clone(), None);
                }
                Ok(Some(recovered)) => {
                    tracing::info!(
                        physical_name = %file.physical.display(),
                        logical_name = %recovered,
                        crc32_hex,
                        size_bytes,
                        "recovered the original filename for an obfuscated import file"
                    );
                    self.resolved
                        .insert(file.physical.clone(), Some(recovered.clone()));
                    file.logical_name = Some(recovered);
                }
            }
        }

        files
    }
}

/// The largest file by on-disk size, which is the release claim when nothing
/// else names the download.
fn pick_largest_import_video_file(files: &[ImportVideoFile]) -> AppResult<ImportVideoFile> {
    files
        .iter()
        .max_by_key(|file| {
            std::fs::metadata(file.path())
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        })
        .cloned()
        .ok_or_else(|| AppError::Repository("no files to pick from".to_string()))
}

fn title_evidence_candidates_from_video_files(
    video_files: &[ImportVideoFile],
) -> Vec<ParsedReleaseMetadata> {
    let mut candidates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for video_file in video_files {
        let video_file = video_file.parse_path();
        let video_file = video_file.as_ref();
        let candidate = parsed_usable_release_from_file_stem(video_file)
            .or_else(|| parsed_usable_release_from_parent_folder(video_file));

        if let Some(candidate) = candidate {
            let key = candidate.raw_title.to_ascii_uppercase();
            if seen.insert(key) {
                candidates.push(candidate);
            }
        }
    }

    candidates
}
