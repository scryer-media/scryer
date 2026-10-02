use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{AppError, AppResult, AppUseCase, stored_paths::path_to_stored_string};

const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 16;
const MAX_SUBTITLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SUBTITLE_FILES: usize = 512;
const MAX_TOTAL_SUBTITLE_BYTES: u64 = 128 * 1024 * 1024;
const SUBTITLE_DISK_RESERVE_BYTES: u64 = 10 * 1024 * 1024;
const SUBTITLE_IO_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct IoControl {
    cancellation: tokio_util::sync::CancellationToken,
    import_cancellation: Option<crate::ImportCancellation>,
    deadline: Instant,
}

impl Default for IoControl {
    fn default() -> Self {
        Self {
            cancellation: tokio_util::sync::CancellationToken::new(),
            import_cancellation: None,
            deadline: Instant::now() + SUBTITLE_IO_TIMEOUT,
        }
    }
}

impl IoControl {
    async fn cancelled(&self) {
        tokio::select! {
            _ = self.cancellation.cancelled() => {},
            _ = async {
                match &self.import_cancellation {
                    Some(cancellation) => cancellation.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            } => {},
        }
    }

    fn check(&self) -> AppResult<()> {
        if self.cancellation.is_cancelled()
            || self
                .import_cancellation
                .as_ref()
                .is_some_and(|cancellation| cancellation.is_cancelled())
        {
            return Err(AppError::Canceled(
                "scene subtitle operation cancelled; sources preserved".into(),
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(AppError::Repository(
                "scene subtitle operation deadline exceeded; sources preserved".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct DeliveryContext {
    import_id: String,
    library_id: String,
    facet: scryer_domain::MediaFacet,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SceneSubtitlePlan {
    files: BTreeMap<PathBuf, Vec<(PathBuf, String)>>,
    staging_root: Option<PathBuf>,
    context: Option<DeliveryContext>,
    total_bytes: u64,
}

impl SceneSubtitlePlan {
    pub(super) async fn discover_for_import(
        app: &AppUseCase,
        title: &scryer_domain::Title,
        import_id: &str,
        videos: &[(PathBuf, PathBuf)],
        roots: &[PathBuf],
    ) -> AppResult<Self> {
        let Some(root) = roots
            .iter()
            .find(|root| crate::archive_extractor::is_owned_archive_workspace(root))
        else {
            return Ok(Self::default());
        };
        let stream = app
            .runtime
            .imports
            .active_streams
            .register(
                import_id,
                &title.library_id,
                title.facet.clone(),
                root,
                root,
            )
            .await;
        let control = IoControl {
            import_cancellation: Some(stream.cancellation_token()),
            ..Default::default()
        };
        let videos = videos.to_vec();
        let roots = roots.to_vec();
        let result = bounded_blocking(app, control, move |control| {
            Self::discover_with_control(&videos, &roots, &control)
        })
        .await;
        stream.finish().await;
        let mut plan = result?;
        plan.context = Some(DeliveryContext {
            import_id: import_id.to_string(),
            library_id: title.library_id.clone(),
            facet: title.facet.clone(),
        });
        Ok(plan)
    }

    // The complete video set is required even when the operator selects only
    // one member of a pack: selection must not make generic subtitles unambiguous.
    #[cfg(all(test, feature = "runtime-archives"))]
    pub(super) fn discover(videos: &[(PathBuf, PathBuf)], roots: &[PathBuf]) -> AppResult<Self> {
        Self::discover_with_control(videos, roots, &IoControl::default())
    }

    fn discover_with_control(
        videos: &[(PathBuf, PathBuf)],
        roots: &[PathBuf],
        control: &IoControl,
    ) -> AppResult<Self> {
        let Some(staging_root) = roots
            .iter()
            .find(|root| crate::archive_extractor::is_owned_archive_workspace(root))
        else {
            return Ok(Self::default());
        };
        let mut paths = BTreeSet::new();
        let mut count = 0;
        let mut replaced = BTreeSet::new();
        for root in roots {
            control.check()?;
            if !crate::archive_extractor::is_owned_archive_workspace(root) {
                continue;
            }
            replaced.extend(crate::archive_extractor::replaced_archive_sources(root)?);
            collect_subtitles(root, 0, &mut count, &mut paths, control)?;
        }
        let mut plan = Self {
            staging_root: Some(staging_root.clone()),
            ..Default::default()
        };
        if paths.len() > MAX_SUBTITLE_FILES {
            return Err(AppError::Validation(
                "scene subtitle file count limit exceeded; workspace preserved".into(),
            ));
        }
        for path in paths {
            control.check()?;
            let size = open_owned_subtitle(&path, staging_root)?
                .metadata()
                .map_err(|error| AppError::Repository(error.to_string()))?
                .len();
            if size > MAX_SUBTITLE_BYTES
                || plan.total_bytes.saturating_add(size) > MAX_TOTAL_SUBTITLE_BYTES
            {
                return Err(AppError::Validation(
                    "scene subtitle aggregate size limit exceeded; workspace preserved".into(),
                ));
            }
            plan.total_bytes += size;
            if path
                .canonicalize()
                .is_ok_and(|canonical| replaced.contains(&canonical))
            {
                continue;
            }
            let matches = videos
                .iter()
                .filter_map(|(physical, alias)| {
                    subtitle_suffix(&path, physical, alias, videos.len() == 1)
                        .map(|suffix| (physical, suffix))
                })
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [(source, suffix)] => plan
                    .files
                    .entry((*source).clone())
                    .or_default()
                    .push((path, suffix.clone())),
                [] => {
                    tracing::warn!(subtitle = %path.display(), "subtitle skipped because no imported video matches")
                }
                _ => {
                    tracing::warn!(subtitle = %path.display(), "subtitle skipped because multiple imported videos match")
                }
            }
        }
        Ok(plan)
    }

    #[cfg(all(test, feature = "runtime-archives"))]
    pub(super) fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub(super) async fn deliver(
        &self,
        app: &AppUseCase,
        title_id: &str,
        source: &Path,
        destination: &Path,
    ) -> AppResult<()> {
        if !self.files.contains_key(source) {
            return Ok(());
        }
        let context = self.context.as_ref().ok_or_else(|| {
            AppError::Validation("scene subtitle delivery context is unavailable".into())
        })?;
        let stream = app
            .runtime
            .imports
            .active_streams
            .register(
                &context.import_id,
                &context.library_id,
                context.facet.clone(),
                source,
                destination,
            )
            .await;
        let control = IoControl {
            import_cancellation: Some(stream.cancellation_token()),
            ..Default::default()
        };
        let plan = self.clone();
        let source = source.to_path_buf();
        let copy_destination = destination.to_path_buf();
        let copied = bounded_blocking(app, control, move |control| {
            plan.copy_to_with_control(&source, &copy_destination, &control)
        })
        .await;
        stream.finish().await;
        if !copied? {
            return Ok(());
        }
        if let Some(media_file) = app
            .services
            .library
            .media_files
            .get_media_file_by_path(&path_to_stored_string(destination))
            .await?
        {
            crate::subtitles::reconcile_external_subtitles_for_media_file(
                app,
                title_id,
                &media_file.id,
                media_file.episode_id.as_deref(),
                destination,
            )
            .await?;
        }
        Ok(())
    }

    #[cfg(all(test, feature = "runtime-archives"))]
    fn copy_to(&self, source: &Path, destination: &Path) -> AppResult<bool> {
        self.copy_to_with_control(source, destination, &IoControl::default())
    }

    fn copy_to_with_control(
        &self,
        source: &Path,
        destination: &Path,
        control: &IoControl,
    ) -> AppResult<bool> {
        control.check()?;
        let Some(files) = self.files.get(source) else {
            return Ok(false);
        };
        let stem = destination
            .file_stem()
            .ok_or_else(|| AppError::Validation("imported video has no filename stem".into()))?;
        let parent = destination
            .parent()
            .ok_or_else(|| AppError::Validation("imported video has no parent directory".into()))?;
        validate_disk_allowance(
            crate::filesystem_space_raw(parent)
                .map_err(|error| AppError::Repository(error.to_string()))?
                .available_bytes,
            self.total_bytes,
        )?;
        let mut targets = BTreeSet::new();
        for (_, suffix) in files {
            let mut name = stem.to_os_string();
            name.push(suffix);
            if !targets.insert(parent.join(name)) {
                return Err(AppError::Validation(
                    "multiple scene subtitles map to the same destination; sources preserved"
                        .into(),
                ));
            }
        }
        for (subtitle, suffix) in files {
            control.check()?;
            let mut name = stem.to_os_string();
            name.push(suffix);
            copy_subtitle_with_control(
                subtitle,
                &parent.join(name),
                self.staging_root.as_deref().ok_or_else(|| {
                    AppError::Validation("scene subtitle staging workspace is unavailable".into())
                })?,
                control,
            )?;
        }
        Ok(true)
    }

    pub(super) fn retain_sources(&mut self, sources: &[PathBuf]) {
        let sources = sources.iter().collect::<BTreeSet<_>>();
        self.files.retain(|source, _| sources.contains(source));
    }

    pub(super) fn has_pending_sources<'a>(
        &self,
        delivered: impl Iterator<Item = &'a Path>,
    ) -> bool {
        let delivered = delivered.collect::<BTreeSet<_>>();
        let pending = self
            .files
            .keys()
            .filter(|source| !delivered.contains(source.as_path()))
            .collect::<Vec<_>>();
        if !pending.is_empty() {
            tracing::warn!(
                count = pending.len(),
                "scene subtitles have no completed current import destination; workspace preserved"
            );
        }
        !pending.is_empty()
    }
}

async fn bounded_blocking<T: Send + 'static>(
    app: &AppUseCase,
    control: IoControl,
    operation: impl FnOnce(IoControl) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let permit = tokio::select! {
        biased;
        _ = control.cancelled() => return Err(AppError::Canceled("scene subtitle operation cancelled".into())),
        result = tokio::time::timeout(SUBTITLE_IO_TIMEOUT, app.runtime.imports.execution_coordinator.acquire_archive_extraction()) => result.map_err(|_| AppError::Repository("scene subtitle worker queue deadline exceeded".into()))?,
    };
    let worker_control = control.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        worker_control.check()?;
        operation(worker_control)
    });
    let result = tokio::select! {
        biased;
        _ = control.cancelled() => Err(AppError::Canceled("scene subtitle operation cancelled; workspace preserved".into())),
        result = tokio::time::timeout(SUBTITLE_IO_TIMEOUT, &mut worker) => match result {
            Ok(joined) => joined.map_err(|error| AppError::Repository(format!("scene subtitle worker failed: {error}")))?,
            Err(_) => { control.cancellation.cancel(); Err(AppError::Repository("scene subtitle worker deadline exceeded; workspace preserved".into())) }
        },
    };
    // A blocking syscall cannot be interrupted. The worker retains its bounded
    // permit and checks cancellation before publishing any final sidecar.
    result
}

fn validate_disk_allowance(available: u64, required: u64) -> AppResult<()> {
    if available < required.saturating_add(SUBTITLE_DISK_RESERVE_BYTES) {
        return Err(AppError::Validation(
            "insufficient disk allowance for scene subtitles; workspace preserved".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn open_owned_subtitle(source: &Path, workspace: &Path) -> AppResult<std::fs::File> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    };
    let relative = source.strip_prefix(workspace).map_err(|_| {
        AppError::Validation("scene subtitle lies outside its owned workspace".into())
    })?;
    let mut directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(workspace)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    if !crate::archive_extractor::is_owned_archive_workspace_handle(workspace, &directory) {
        return Err(AppError::Validation(
            "scene subtitle workspace identity changed".into(),
        ));
    }
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return Err(AppError::Validation(
                "invalid scene subtitle path component".into(),
            ));
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| AppError::Validation("invalid scene subtitle path".into()))?;
        let final_component = index + 1 == components.len();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if final_component {
                0
            } else {
                libc::O_DIRECTORY
            };
        // Each component is opened relative to the held directory handle, so
        // replacing a path with a symlink cannot redirect a subtitle read.
        let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            return Err(AppError::Repository(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
        if final_component {
            if !file
                .metadata()
                .map_err(|error| AppError::Repository(error.to_string()))?
                .is_file()
            {
                return Err(AppError::Validation(
                    "scene subtitle is not a regular file".into(),
                ));
            }
            return Ok(file);
        }
        directory = file;
    }
    Err(AppError::Validation(
        "scene subtitle path has no filename".into(),
    ))
}

#[cfg(windows)]
fn open_owned_subtitle(source: &Path, workspace: &Path) -> AppResult<std::fs::File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let mut directory_options = std::fs::OpenOptions::new();
    directory_options
        .read(true)
        .share_mode(3)
        .custom_flags(0x0220_0000);
    let directory = directory_options
        .open(workspace)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    if !crate::archive_extractor::is_owned_archive_workspace_handle(workspace, &directory) {
        return Err(AppError::Validation(
            "scene subtitle workspace identity changed".into(),
        ));
    }
    let mut file_options = std::fs::OpenOptions::new();
    file_options
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000);
    let file = file_options
        .open(source)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let root_path = final_windows_handle_path(&directory)?
        .trim_end_matches('\\')
        .to_lowercase();
    let final_path = final_windows_handle_path(&file)?.to_lowercase();
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || !final_path.starts_with(&format!("{root_path}\\"))
    {
        return Err(AppError::Validation(
            "scene subtitle handle lies outside its owned workspace or is a reparse point".into(),
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn final_windows_handle_path(file: &std::fs::File) -> AppResult<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buffer = vec![0u16; 32_768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if count == 0 || count as usize >= buffer.len() {
        return Err(AppError::Repository(
            "unable to resolve scene subtitle handle path".into(),
        ));
    }
    String::from_utf16(&buffer[..count as usize])
        .map_err(|_| AppError::Validation("scene subtitle handle path is invalid UTF-16".into()))
}

#[cfg(not(any(unix, windows)))]
fn open_owned_subtitle(source: &Path, workspace: &Path) -> AppResult<std::fs::File> {
    let relative = source.strip_prefix(workspace).map_err(|_| {
        AppError::Validation("scene subtitle lies outside its owned workspace".into())
    })?;
    if !crate::archive_extractor::is_owned_archive_workspace(workspace) {
        return Err(AppError::Validation(
            "scene subtitle workspace identity changed".into(),
        ));
    }
    let mut path = workspace.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(AppError::Validation(
                "invalid scene subtitle path component".into(),
            ));
        };
        path.push(name);
        if std::fs::symlink_metadata(&path)
            .map_err(|error| AppError::Repository(error.to_string()))?
            .file_type()
            .is_symlink()
        {
            return Err(AppError::Validation(
                "scene subtitle path contains a symlink".into(),
            ));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(source)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    if !file
        .metadata()
        .map_err(|error| AppError::Repository(error.to_string()))?
        .is_file()
    {
        return Err(AppError::Validation(
            "scene subtitle is not a regular file".into(),
        ));
    }
    Ok(file)
}

fn collect_subtitles(
    root: &Path,
    depth: usize,
    count: &mut usize,
    paths: &mut BTreeSet<PathBuf>,
    control: &IoControl,
) -> AppResult<()> {
    control.check()?;
    if depth > MAX_DEPTH {
        return Err(AppError::Validation(
            "scene subtitle directory nesting limit exceeded".into(),
        ));
    }
    for entry in std::fs::read_dir(root).map_err(|error| AppError::Repository(error.to_string()))? {
        control.check()?;
        let entry = entry.map_err(|error| AppError::Repository(error.to_string()))?;
        *count += 1;
        if *count > MAX_ENTRIES {
            return Err(AppError::Validation(
                "scene subtitle directory entry limit exceeded".into(),
            ));
        }
        let kind = entry
            .file_type()
            .map_err(|error| AppError::Repository(error.to_string()))?;
        // Directory entries are inspected without following symlinks. A
        // sidecar outside the authorized download or workspace is never read.
        if kind.is_dir() {
            collect_subtitles(&entry.path(), depth + 1, count, paths, control)?;
        } else if kind.is_file() && scryer_domain::is_subtitle_file(&entry.path()) {
            paths.insert(entry.path());
        }
    }
    Ok(())
}

fn subtitle_suffix(
    subtitle: &Path,
    physical: &Path,
    alias: &Path,
    single_video: bool,
) -> Option<String> {
    let subtitle_stem = subtitle.file_stem()?.to_str()?;
    let extension = subtitle.extension()?.to_str()?.to_ascii_lowercase();
    for video in [physical, alias] {
        let stem = video.file_stem()?.to_str()?;
        if subtitle_stem.eq_ignore_ascii_case(stem) {
            return Some(format!(".{extension}"));
        }
        if subtitle_stem.len() > stem.len()
            && subtitle_stem
                .get(..stem.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(stem))
            && subtitle_stem.as_bytes().get(stem.len()) == Some(&b'.')
        {
            return Some(format!("{}.{extension}", &subtitle_stem[stem.len()..]));
        }
        if subtitle.ancestors().skip(1).any(|parent| {
            parent
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(stem))
        }) && let Some(suffix) = language_suffix(subtitle_stem)
        {
            return Some(format!(".{suffix}.{extension}"));
        }
    }
    single_video
        .then(|| language_suffix(subtitle_stem))
        .flatten()
        .map(|suffix| format!(".{suffix}.{extension}"))
}

fn language_suffix(stem: &str) -> Option<String> {
    let stem = stem.trim_start_matches(|character: char| {
        character.is_ascii_digit() || matches!(character, '_' | '-' | ' ')
    });
    let mut tokens = stem
        .split(['.', '_', ' '])
        .filter(|token| !token.is_empty());
    let language =
        crate::media::language::normalize_detected_subtitle_language_code(tokens.next()?)?;
    let mut suffix = language;
    for token in tokens {
        let token = token.to_ascii_lowercase();
        if !matches!(
            token.as_str(),
            "forced" | "foreign" | "sdh" | "hi" | "cc" | "hoh"
        ) {
            return None;
        }
        suffix.push('.');
        suffix.push_str(&token);
    }
    Some(suffix)
}

#[cfg(all(test, feature = "runtime-archives"))]
fn copy_subtitle_without_replacement(
    source: &Path,
    destination: &Path,
    staging_root: &Path,
) -> AppResult<()> {
    copy_subtitle_with_control(source, destination, staging_root, &IoControl::default())
}

fn copy_subtitle_with_control(
    source: &Path,
    destination: &Path,
    staging_root: &Path,
    control: &IoControl,
) -> AppResult<()> {
    copy_subtitle_with_writer(
        source,
        destination,
        staging_root,
        |file, bytes| {
            for chunk in bytes.chunks(16 * 1024) {
                control
                    .check()
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                file.write_all(chunk)?;
            }
            file.sync_all()
        },
        control,
    )
}

fn copy_subtitle_with_writer(
    source: &Path,
    destination: &Path,
    staging_root: &Path,
    writer: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
    control: &IoControl,
) -> AppResult<()> {
    control.check()?;
    if !crate::archive_extractor::is_owned_archive_workspace(staging_root) {
        return Err(AppError::Validation(
            "scene subtitle staging requires an owned archive workspace".into(),
        ));
    }
    let mut source_file = open_owned_subtitle(source, staging_root)?;
    let metadata = source_file
        .metadata()
        .map_err(|error| AppError::Repository(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_SUBTITLE_BYTES {
        return Err(AppError::Validation(
            "scene subtitle is not a bounded regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        control.check()?;
        let count = source_file
            .read(&mut buffer)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() as u64 > MAX_SUBTITLE_BYTES {
            break;
        }
    }
    if bytes.len() as u64 > MAX_SUBTITLE_BYTES {
        return Err(AppError::Validation(
            "scene subtitle size limit exceeded".into(),
        ));
    }
    let mut staging = tempfile::NamedTempFile::new_in(staging_root)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    staging
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|error| AppError::Repository(error.to_string()))?;
    writer(staging.as_file_mut(), &bytes).map_err(|error| {
        AppError::Repository(format!(
            "scene subtitle staging failed; source preserved: {error}"
        ))
    })?;
    control.check()?;
    match staging.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if existing_subtitle_matches(destination, &bytes, control)? {
                Ok(())
            } else {
                Err(AppError::Validation("scene subtitle destination already exists with different contents; source preserved".into()))
            }
        }
        Err(error) => Err(AppError::Repository(format!(
            "scene subtitle delivery failed; source preserved: {}",
            error.error
        ))),
    }
}

fn existing_subtitle_matches(
    destination: &Path,
    expected: &[u8],
    control: &IoControl,
) -> AppResult<bool> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let mut file = options
        .open(destination)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::Repository(error.to_string()))?;
    if !metadata.is_file() || metadata.len() != expected.len() as u64 {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    let mut offset = 0;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        control.check()?;
        let count = file
            .read(&mut buffer)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if count == 0 {
            return Ok(offset == expected.len());
        }
        if count > expected.len().saturating_sub(offset)
            || expected[offset..offset + count] != buffer[..count]
        {
            return Ok(false);
        }
        offset += count;
    }
}

#[cfg(all(test, feature = "runtime-archives"))]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn video(path: &Path) -> (PathBuf, PathBuf) {
        (path.to_path_buf(), path.to_path_buf())
    }

    fn workspace(root: &Path) -> PathBuf {
        crate::archive_extractor::create_test_archive_workspace(root)
    }

    #[test]
    fn loose_video_receives_subtitle_only_archive_output() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(fixture.path(), "download/release.mkv", b"video");
        let subtitle = write(&workspace, "nested-1/Subs/1_English.srt", b"subtitle");
        let destination = write(fixture.path(), "library/renamed.mkv", b"video");
        let plan = SceneSubtitlePlan::discover(
            &[video(&source)],
            &[fixture.path().join("download"), workspace],
        )
        .unwrap();
        assert!(plan.copy_to(&source, &destination).unwrap());
        assert_eq!(
            std::fs::read(destination.with_file_name("renamed.eng.srt")).unwrap(),
            b"subtitle"
        );
        assert!(subtitle.is_file());
    }

    #[test]
    fn nested_archive_sidecars_follow_recovered_video_name_and_keep_flags() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(&workspace, "out/opaque.mkv", b"video");
        write(
            &workspace,
            "nested-1/Scene.Release.en.forced.srt",
            b"subtitle",
        );
        let destination = write(fixture.path(), "library/renamed.mkv", b"video");
        let plan = SceneSubtitlePlan::discover(
            &[(source.clone(), source.with_file_name("Scene.Release.mkv"))],
            &[workspace],
        )
        .unwrap();
        plan.copy_to(&source, &destination).unwrap();
        assert_eq!(
            std::fs::read(destination.with_file_name("renamed.en.forced.srt")).unwrap(),
            b"subtitle"
        );
    }

    #[test]
    fn pack_generic_subtitles_are_skipped_but_video_scoped_pairs_are_delivered() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let first = write(fixture.path(), "download/Show.S01E01.mkv", b"first");
        let second = write(fixture.path(), "download/Show.S01E02.mkv", b"second");
        write(&workspace, "nested-1/Subs/English.srt", b"ambiguous");
        let index = write(
            &workspace,
            "nested-1/Subs/Show.S01E02/English.idx",
            b"index",
        );
        let sub = write(
            &workspace,
            "nested-1/Subs/Show.S01E02/English.sub",
            b"binary subtitles",
        );
        let destination = write(fixture.path(), "library/Episode 2.mkv", b"second");
        let plan = SceneSubtitlePlan::discover(
            &[video(&first), video(&second)],
            &[fixture.path().join("download"), workspace],
        )
        .unwrap();
        assert!(!plan.copy_to(&first, &destination).unwrap());
        plan.copy_to(&second, &destination).unwrap();
        assert_eq!(
            std::fs::read(destination.with_file_name("Episode 2.eng.idx")).unwrap(),
            b"index"
        );
        assert_eq!(
            std::fs::read(destination.with_file_name("Episode 2.eng.sub")).unwrap(),
            b"binary subtitles"
        );
        assert!(index.is_file() && sub.is_file());
        assert!(!destination.with_file_name("Episode 2.eng.srt").exists());
    }

    #[test]
    fn conflicting_destination_preserves_both_files_and_identical_retry_is_safe() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(&workspace, "out/source.srt", b"new subtitle");
        let destination = write(fixture.path(), "destination.srt", b"existing subtitle");
        assert!(copy_subtitle_without_replacement(&source, &destination, &workspace).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing subtitle");
        assert_eq!(std::fs::read(&source).unwrap(), b"new subtitle");
        let identical = write(fixture.path(), "identical.srt", b"new subtitle");
        copy_subtitle_without_replacement(&source, &identical, &workspace).unwrap();
    }

    #[test]
    fn duplicate_language_targets_fail_before_creating_any_sidecar() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(fixture.path(), "download/release.mkv", b"video");
        write(&workspace, "out/Subs/English.srt", b"first");
        write(&workspace, "out/Subs/en.srt", b"second");
        let destination = write(fixture.path(), "library/renamed.mkv", b"video");
        let plan = SceneSubtitlePlan::discover(
            &[video(&source)],
            &[fixture.path().join("download"), workspace],
        )
        .unwrap();
        assert!(plan.copy_to(&source, &destination).is_err());
        assert!(!destination.with_file_name("renamed.eng.srt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_and_copy_refuse_symlink_sidecars() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(fixture.path(), "download/release.mkv", b"video");
        let outside = write(fixture.path(), "outside/release.en.srt", b"outside");
        std::os::unix::fs::symlink(&outside, workspace.join("out/release.en.srt")).unwrap();
        std::os::unix::fs::symlink(outside.parent().unwrap(), workspace.join("out/Subs")).unwrap();
        let plan = SceneSubtitlePlan::discover(
            &[video(&source)],
            &[fixture.path().join("download"), workspace.clone()],
        )
        .unwrap();
        assert!(plan.is_empty());
        assert!(
            copy_subtitle_without_replacement(
                &workspace.join("out/release.en.srt"),
                &fixture.path().join("copied.srt"),
                &workspace
            )
            .is_err()
        );
        assert!(!fixture.path().join("copied.srt").exists());
    }

    #[test]
    fn failed_staging_write_leaves_no_partial_final_and_can_retry() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(&workspace, "out/source.srt", b"complete subtitle");
        let destination = fixture.path().join("final.srt");
        let entries_before = std::fs::read_dir(&workspace).unwrap().count();
        let result = copy_subtitle_with_writer(
            &source,
            &destination,
            &workspace,
            |file, bytes| {
                file.write_all(&bytes[..4])?;
                Err(std::io::Error::other("synthetic write failure"))
            },
            &IoControl::default(),
        );
        assert!(result.is_err());
        assert!(!destination.exists());
        assert_eq!(
            std::fs::read_dir(&workspace).unwrap().count(),
            entries_before
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"complete subtitle");
        copy_subtitle_without_replacement(&source, &destination, &workspace).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete subtitle");
    }

    #[test]
    fn repaired_subtitle_replaces_only_candidate_selection_and_preserves_damaged_original() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(&workspace, "out/release.mkv", b"repaired video");
        let damaged = write(
            fixture.path(),
            "download/release.eng.srt",
            b"damaged subtitle",
        );
        write(&workspace, "out/release.eng.srt", b"repaired subtitle");
        write(
            &workspace,
            ".source-replacements-out.json",
            &serde_json::to_vec(&vec![damaged.canonicalize().unwrap()]).unwrap(),
        );
        let destination = write(fixture.path(), "library/renamed.mkv", b"repaired video");
        let plan = SceneSubtitlePlan::discover(
            &[video(&source)],
            &[fixture.path().join("download"), workspace],
        )
        .unwrap();
        plan.copy_to(&source, &destination).unwrap();
        assert_eq!(
            std::fs::read(destination.with_file_name("renamed.eng.srt")).unwrap(),
            b"repaired subtitle"
        );
        assert_eq!(std::fs::read(damaged).unwrap(), b"damaged subtitle");
    }

    #[test]
    fn scene_subtitle_count_and_aggregate_size_are_bounded_before_delivery() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(fixture.path(), "release.mkv", b"video");
        for index in 0..=MAX_SUBTITLE_FILES {
            write(&workspace, &format!("out/release.eng.{index}.srt"), b"s");
        }
        assert!(
            SceneSubtitlePlan::discover(&[video(&source)], &[workspace])
                .unwrap_err()
                .to_string()
                .contains("file count limit")
        );

        let size_fixture = tempfile::tempdir().unwrap();
        let workspace =
            crate::archive_extractor::create_test_archive_workspace(size_fixture.path());
        for (index, size) in [MAX_SUBTITLE_BYTES, MAX_SUBTITLE_BYTES, 1]
            .into_iter()
            .enumerate()
        {
            let path = write(&workspace, &format!("out/release.eng.{index}.srt"), b"");
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_len(size)
                .unwrap();
        }
        assert!(
            SceneSubtitlePlan::discover(&[video(&source)], &[workspace])
                .unwrap_err()
                .to_string()
                .contains("aggregate size limit")
        );
    }

    #[test]
    fn pending_source_mapping_prevents_workspace_release() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(fixture.path(), "release.mkv", b"video");
        write(&workspace, "out/release.eng.srt", b"subtitle");
        let plan = SceneSubtitlePlan::discover(&[video(&source)], &[workspace]).unwrap();
        assert!(plan.has_pending_sources(std::iter::empty()));
        assert!(plan.has_pending_sources(std::iter::once(Path::new("catalog/old-original.mkv"))));
        assert!(!plan.has_pending_sources(std::iter::once(source.as_path())));
    }

    #[test]
    fn cancelled_and_expired_scene_subtitle_writes_preserve_sources_without_publishing() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(&workspace, "out/source.srt", b"subtitle");
        let destination = fixture.path().join("final.srt");
        let cancelled = IoControl::default();
        cancelled.cancellation.cancel();
        assert!(matches!(
            copy_subtitle_with_control(&source, &destination, &workspace, &cancelled),
            Err(AppError::Canceled(_))
        ));
        let expired = IoControl {
            deadline: Instant::now(),
            ..Default::default()
        };
        assert!(copy_subtitle_with_control(&source, &destination, &workspace, &expired).is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source).unwrap(), b"subtitle");
        assert!(validate_disk_allowance(SUBTITLE_DISK_RESERVE_BYTES - 1, 0).is_err());
        validate_disk_allowance(
            SUBTITLE_DISK_RESERVE_BYTES + MAX_TOTAL_SUBTITLE_BYTES,
            MAX_TOTAL_SUBTITLE_BYTES,
        )
        .unwrap();
    }

    #[test]
    fn mutable_download_root_subtitles_are_not_part_of_archive_delivery() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = workspace(fixture.path());
        let source = write(fixture.path(), "download/release.mkv", b"video");
        let outside = write(
            fixture.path(),
            "download/release.eng.srt",
            b"loose subtitle",
        );
        let plan = SceneSubtitlePlan::discover(
            &[video(&source)],
            &[fixture.path().join("download"), workspace],
        )
        .unwrap();
        assert!(plan.is_empty());
        assert_eq!(std::fs::read(outside).unwrap(), b"loose subtitle");
    }
}
