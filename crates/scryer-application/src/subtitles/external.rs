use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use scryer_domain::{ExternalSubtitleSourceKind, SUBTITLE_EXTENSIONS, SubtitleDownload};
use tokio::fs;

use super::external_probe::{ExternalSubtitleProbeCacheEntry, resolve_external_subtitle};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::{AppError, AppResult, AppUseCase};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredExternalSubtitle {
    file_path: String,
    language: String,
    forced: bool,
    hearing_impaired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExternalSubtitleCandidate {
    file_path: String,
    extension: String,
    language: Option<String>,
    forced: bool,
    hearing_impaired: bool,
}

#[derive(Default)]
pub(crate) struct ExternalSubtitleDirectoryCache {
    subtitle_paths_by_parent: BTreeMap<PathBuf, Vec<PathBuf>>,
}

impl ExternalSubtitleDirectoryCache {
    async fn subtitle_paths_for_parent(&mut self, parent: &Path) -> AppResult<Vec<PathBuf>> {
        if let Some(paths) = self.subtitle_paths_by_parent.get(parent) {
            return Ok(paths.clone());
        }

        let paths = list_external_subtitle_paths(parent).await?;
        self.subtitle_paths_by_parent
            .insert(parent.to_path_buf(), paths.clone());
        Ok(paths)
    }
}

pub(crate) async fn reconcile_external_subtitles_for_media_file(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    episode_id: Option<&str>,
    video_path: &Path,
) -> AppResult<bool> {
    let mut cache = ExternalSubtitleDirectoryCache::default();
    reconcile_external_subtitles_for_media_file_with_cache(
        app,
        title_id,
        media_file_id,
        episode_id,
        video_path,
        &mut cache,
    )
    .await
}

pub(crate) async fn reconcile_external_subtitles_for_media_file_with_cache(
    app: &AppUseCase,
    title_id: &str,
    media_file_id: &str,
    episode_id: Option<&str>,
    video_path: &Path,
    directory_cache: &mut ExternalSubtitleDirectoryCache,
) -> AppResult<bool> {
    let existing = app
        .services
        .workflow
        .subtitle_downloads
        .list_for_media_file(media_file_id)
        .await?;
    let existing_probe_cache = app
        .services
        .workflow
        .subtitle_downloads
        .list_probe_cache_for_media_file(media_file_id)
        .await?;

    let existing_probe_cache_by_path = existing_probe_cache
        .into_iter()
        .map(|entry| (entry.file_path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let discovered_candidates =
        discover_external_subtitles_for_video_with_cache(video_path, directory_cache).await?;
    let downloaded_paths = existing
        .iter()
        .filter(|record| record.source_kind == ExternalSubtitleSourceKind::Downloaded)
        .map(|record| record.file_path.clone())
        .collect::<HashSet<_>>();

    let mut desired_discovered = BTreeMap::new();
    let mut desired_probe_cache = BTreeMap::<String, ExternalSubtitleProbeCacheEntry>::new();
    let mut failed_probe_paths = HashSet::new();
    for candidate in discovered_candidates {
        if downloaded_paths.contains(&candidate.file_path) {
            continue;
        }

        let subtitle_path = stored_path_to_path_buf(&candidate.file_path);
        match resolve_external_subtitle(
            media_file_id,
            subtitle_path.as_path(),
            &candidate.extension,
            candidate.language.as_deref(),
            candidate.forced,
            candidate.hearing_impaired,
            existing_probe_cache_by_path.get(&candidate.file_path),
        )
        .await
        {
            Ok(resolved) => {
                desired_probe_cache.insert(candidate.file_path.clone(), resolved.cache_entry);
                if let Some(language) = resolved.language {
                    desired_discovered.insert(
                        candidate.file_path.clone(),
                        DiscoveredExternalSubtitle {
                            file_path: candidate.file_path.clone(),
                            language,
                            forced: candidate.forced,
                            hearing_impaired: resolved.hearing_impaired,
                        },
                    );
                } else {
                    tracing::debug!(
                        path = %subtitle_path.display(),
                        "skipping external subtitle sidecar without a resolved language"
                    );
                }
            }
            Err(error) => {
                failed_probe_paths.insert(candidate.file_path.clone());
                tracing::debug!(
                    path = %subtitle_path.display(),
                    error = %error,
                    "failed to fingerprint external subtitle sidecar"
                );
            }
        }
    }

    let mut changed = false;
    let mut existing_discovered_by_path = BTreeMap::new();
    for record in &existing {
        let exists = fs::try_exists(stored_path_to_path_buf(&record.file_path))
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        if !exists {
            app.services
                .workflow
                .subtitle_downloads
                .delete(&record.id)
                .await?;
            changed = true;
            continue;
        }

        if record.source_kind == ExternalSubtitleSourceKind::Discovered {
            if should_preserve_existing_discovered_record(
                &record.file_path,
                &desired_discovered,
                &failed_probe_paths,
            ) {
                existing_discovered_by_path.insert(record.file_path.clone(), record.clone());
            } else {
                app.services
                    .workflow
                    .subtitle_downloads
                    .delete(&record.id)
                    .await?;
                changed = true;
            }
        }
    }

    for discovered in desired_discovered.into_values() {
        if let Some(existing_record) = existing_discovered_by_path.get(&discovered.file_path) {
            let updated = build_discovered_external_subtitle_record(
                existing_record.id.clone(),
                media_file_id,
                title_id,
                episode_id,
                &discovered,
                &existing_record.downloaded_at,
            );
            if subtitle_records_differ(existing_record, &updated) {
                app.services
                    .workflow
                    .subtitle_downloads
                    .insert(&updated)
                    .await?;
                changed = true;
            }
        } else {
            let inserted = build_discovered_external_subtitle_record(
                scryer_domain::Id::new().0,
                media_file_id,
                title_id,
                episode_id,
                &discovered,
                &Utc::now().to_rfc3339(),
            );
            app.services
                .workflow
                .subtitle_downloads
                .insert(&inserted)
                .await?;
            changed = true;
        }
    }

    for cache_entry in desired_probe_cache.values() {
        if existing_probe_cache_by_path.get(&cache_entry.file_path) != Some(cache_entry) {
            app.services
                .workflow
                .subtitle_downloads
                .upsert_probe_cache_entry(cache_entry)
                .await?;
            changed = true;
        }
    }

    for existing_cache_entry in existing_probe_cache_by_path.values() {
        if !should_preserve_existing_probe_cache_entry(
            &existing_cache_entry.file_path,
            &desired_probe_cache,
            &failed_probe_paths,
        ) {
            app.services
                .workflow
                .subtitle_downloads
                .delete_probe_cache_entry(media_file_id, &existing_cache_entry.file_path)
                .await?;
            changed = true;
        }
    }

    Ok(changed)
}

fn should_preserve_existing_discovered_record(
    file_path: &str,
    desired_discovered: &BTreeMap<String, DiscoveredExternalSubtitle>,
    failed_probe_paths: &HashSet<String>,
) -> bool {
    desired_discovered.contains_key(file_path) || failed_probe_paths.contains(file_path)
}

fn should_preserve_existing_probe_cache_entry(
    file_path: &str,
    desired_probe_cache: &BTreeMap<String, ExternalSubtitleProbeCacheEntry>,
    failed_probe_paths: &HashSet<String>,
) -> bool {
    desired_probe_cache.contains_key(file_path) || failed_probe_paths.contains(file_path)
}

fn build_discovered_external_subtitle_record(
    id: String,
    media_file_id: &str,
    title_id: &str,
    episode_id: Option<&str>,
    discovered: &DiscoveredExternalSubtitle,
    downloaded_at: &str,
) -> SubtitleDownload {
    SubtitleDownload {
        id,
        media_file_id: media_file_id.to_string(),
        title_id: title_id.to_string(),
        episode_id: episode_id.map(str::to_string),
        source_kind: ExternalSubtitleSourceKind::Discovered,
        language: discovered.language.clone(),
        provider: None,
        provider_file_id: None,
        file_path: discovered.file_path.clone(),
        score: None,
        hearing_impaired: discovered.hearing_impaired,
        forced: discovered.forced,
        ai_translated: false,
        machine_translated: false,
        uploader: None,
        release_info: None,
        synced: false,
        downloaded_at: downloaded_at.to_string(),
    }
}

fn subtitle_records_differ(left: &SubtitleDownload, right: &SubtitleDownload) -> bool {
    left.media_file_id != right.media_file_id
        || left.title_id != right.title_id
        || left.episode_id != right.episode_id
        || left.source_kind != right.source_kind
        || left.language != right.language
        || left.provider != right.provider
        || left.provider_file_id != right.provider_file_id
        || left.file_path != right.file_path
        || left.score != right.score
        || left.hearing_impaired != right.hearing_impaired
        || left.forced != right.forced
        || left.ai_translated != right.ai_translated
        || left.machine_translated != right.machine_translated
        || left.uploader != right.uploader
        || left.release_info != right.release_info
        || left.synced != right.synced
}

#[cfg(test)]
async fn discover_external_subtitles_for_video(
    video_path: &Path,
) -> AppResult<Vec<ExternalSubtitleCandidate>> {
    let mut cache = ExternalSubtitleDirectoryCache::default();
    discover_external_subtitles_for_video_with_cache(video_path, &mut cache).await
}

async fn discover_external_subtitles_for_video_with_cache(
    video_path: &Path,
    directory_cache: &mut ExternalSubtitleDirectoryCache,
) -> AppResult<Vec<ExternalSubtitleCandidate>> {
    let Some(parent) = video_path.parent() else {
        return Ok(Vec::new());
    };
    let Some(video_stem) = video_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
    else {
        return Ok(Vec::new());
    };

    let paths = directory_cache.subtitle_paths_for_parent(parent).await?;
    let mut discovered = Vec::new();

    for path in paths {
        if let Some(subtitle) = parse_discovered_external_subtitle(&video_stem, path.as_path()) {
            discovered.push(subtitle);
        }
    }

    discovered.sort_by(|left, right| left.file_path.cmp(&right.file_path));
    Ok(discovered)
}

async fn list_external_subtitle_paths(parent: &Path) -> AppResult<Vec<PathBuf>> {
    let mut entries = fs::read_dir(parent)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    let mut paths = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?
    {
        let path = entry.path();
        if path_has_subtitle_extension(&path) {
            paths.push(path);
        }
    }

    paths.sort();
    Ok(paths)
}

fn path_has_subtitle_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| SUBTITLE_EXTENSIONS.contains(&ext.as_str()))
}

fn parse_discovered_external_subtitle(
    video_stem: &str,
    subtitle_path: &Path,
) -> Option<ExternalSubtitleCandidate> {
    let subtitle_stem = subtitle_path.file_stem()?.to_string_lossy();
    let extension = subtitle_path
        .extension()?
        .to_string_lossy()
        .to_ascii_lowercase();
    let suffix = if subtitle_stem == video_stem {
        ""
    } else {
        subtitle_stem.strip_prefix(&format!("{video_stem}."))?
    };

    let (language, forced, hearing_impaired) = parse_sidecar_suffix_tokens(suffix);
    Some(ExternalSubtitleCandidate {
        file_path: path_to_stored_string(subtitle_path),
        extension,
        language,
        forced,
        hearing_impaired,
    })
}

fn parse_sidecar_suffix_tokens(suffix: &str) -> (Option<String>, bool, bool) {
    let mut language = None;
    let mut forced = false;
    let mut hearing_impaired = false;

    for token in suffix.split('.').filter(|token| !token.trim().is_empty()) {
        let normalized = token.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "forced" | "foreign") {
            forced = true;
            continue;
        }
        if matches!(
            normalized.as_str(),
            "hi" | "cc" | "sdh" | "hoh" | "hearingimpaired" | "hearing-impaired"
        ) {
            hearing_impaired = true;
            continue;
        }
        if language.is_none() {
            language = crate::media::language::normalize_detected_subtitle_language_code(token);
        }
    }

    (language, forced, hearing_impaired)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "runtime-media-analysis")]
    use std::time::Duration;
    use std::{
        collections::{BTreeMap, HashSet},
        fs,
        path::Path,
        sync::Arc,
    };

    use async_trait::async_trait;
    use chrono::Utc;
    use tokio::sync::Mutex;

    #[cfg(unix)]
    use super::parse_discovered_external_subtitle;
    use super::{
        DiscoveredExternalSubtitle, ExternalSubtitleDirectoryCache,
        discover_external_subtitles_for_video, discover_external_subtitles_for_video_with_cache,
        parse_sidecar_suffix_tokens, reconcile_external_subtitles_for_media_file,
        should_preserve_existing_discovered_record, should_preserve_existing_probe_cache_entry,
    };
    use crate::{
        AppResult, AppServices, AppUseCase, FacetRegistry, IndexerConfig, IndexerConfigRepository,
        IndexerConfigUpdate, JwtAuthConfig, SubtitleDownloadRepository,
        null_repositories::test_nulls::{
            NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
            NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
            NullTitleRepository, NullUserRepository,
        },
        subtitles::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry},
    };
    use scryer_domain::{ExternalSubtitleSourceKind, SubtitleBlocklistEntry, SubtitleDownload};

    #[derive(Default)]
    struct TestSubtitleDownloadRepository {
        downloads: Mutex<Vec<SubtitleDownload>>,
        probe_cache: Mutex<Vec<ExternalSubtitleProbeCacheEntry>>,
    }

    struct TestIndexerConfigRepository;

    impl TestSubtitleDownloadRepository {
        async fn downloads_for_media_file(&self, media_file_id: &str) -> Vec<SubtitleDownload> {
            self.downloads
                .lock()
                .await
                .iter()
                .filter(|download| download.media_file_id == media_file_id)
                .cloned()
                .collect()
        }

        async fn probe_cache_for_media_file(
            &self,
            media_file_id: &str,
        ) -> Vec<ExternalSubtitleProbeCacheEntry> {
            self.probe_cache
                .lock()
                .await
                .iter()
                .filter(|entry| entry.media_file_id == media_file_id)
                .cloned()
                .collect()
        }

        async fn seed_download(&self, download: SubtitleDownload) {
            self.downloads.lock().await.push(download);
        }
    }

    #[async_trait]
    impl SubtitleDownloadRepository for TestSubtitleDownloadRepository {
        async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
            Ok(self
                .downloads
                .lock()
                .await
                .iter()
                .filter(|download| download.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
            Ok(self
                .downloads
                .lock()
                .await
                .iter()
                .find(|download| download.id == id)
                .cloned())
        }

        async fn list_for_media_file(
            &self,
            media_file_id: &str,
        ) -> AppResult<Vec<SubtitleDownload>> {
            Ok(self.downloads_for_media_file(media_file_id).await)
        }

        async fn list_probe_cache_for_media_file(
            &self,
            media_file_id: &str,
        ) -> AppResult<Vec<ExternalSubtitleProbeCacheEntry>> {
            Ok(self.probe_cache_for_media_file(media_file_id).await)
        }

        async fn list_blocklist_for_media_file(
            &self,
            _media_file_id: &str,
        ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
            Ok(Vec::new())
        }

        async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
            let mut downloads = self.downloads.lock().await;
            if let Some(position) = downloads
                .iter()
                .position(|existing| existing.id == download.id)
            {
                downloads[position] = download.clone();
            } else {
                downloads.push(download.clone());
            }
            Ok(())
        }

        async fn upsert_probe_cache_entry(
            &self,
            entry: &ExternalSubtitleProbeCacheEntry,
        ) -> AppResult<()> {
            let mut probe_cache = self.probe_cache.lock().await;
            if let Some(position) = probe_cache.iter().position(|existing| {
                existing.media_file_id == entry.media_file_id
                    && existing.file_path == entry.file_path
            }) {
                probe_cache[position] = entry.clone();
            } else {
                probe_cache.push(entry.clone());
            }
            Ok(())
        }

        async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
            let mut downloads = self.downloads.lock().await;
            if let Some(download) = downloads.iter_mut().find(|download| download.id == id) {
                download.synced = synced;
            }
            Ok(())
        }

        async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
            let mut downloads = self.downloads.lock().await;
            let Some(position) = downloads.iter().position(|download| download.id == id) else {
                return Ok(None);
            };
            Ok(Some(downloads.remove(position)))
        }

        async fn delete_probe_cache_entry(
            &self,
            media_file_id: &str,
            file_path: &str,
        ) -> AppResult<()> {
            let mut probe_cache = self.probe_cache.lock().await;
            probe_cache.retain(|entry| {
                !(entry.media_file_id == media_file_id && entry.file_path == file_path)
            });
            Ok(())
        }

        async fn is_blocklisted(
            &self,
            _media_file_id: &str,
            _provider: &str,
            _provider_file_id: &str,
        ) -> AppResult<bool> {
            Ok(false)
        }

        async fn blocklist(
            &self,
            _media_file_id: &str,
            _provider: &str,
            _provider_file_id: &str,
            _language: &str,
            _reason: Option<&str>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(&self, _update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(crate::AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    fn build_test_app(subtitle_repo: Arc<TestSubtitleDownloadRepository>) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_subtitle_downloads(subtitle_repo)
        .build_partial_for_tests();

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn downloaded_subtitle(
        id: &str,
        media_file_id: &str,
        title_id: &str,
        file_path: &str,
    ) -> SubtitleDownload {
        SubtitleDownload {
            id: id.to_string(),
            media_file_id: media_file_id.to_string(),
            title_id: title_id.to_string(),
            episode_id: None,
            source_kind: ExternalSubtitleSourceKind::Downloaded,
            language: "eng".to_string(),
            provider: Some("opensubtitles".to_string()),
            provider_file_id: Some("provider-file-1".to_string()),
            file_path: file_path.to_string(),
            score: Some(100),
            hearing_impaired: false,
            forced: false,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            release_info: None,
            synced: false,
            downloaded_at: Utc::now().to_rfc3339(),
        }
    }

    async fn reconcile_fixture(app: &AppUseCase, video_path: &Path, media_file_id: &str) -> bool {
        reconcile_external_subtitles_for_media_file(app, "title-1", media_file_id, None, video_path)
            .await
            .expect("reconcile external subtitles")
    }

    #[test]
    fn parses_language_and_common_flags_from_sidecar_suffix() {
        assert_eq!(
            parse_sidecar_suffix_tokens("eng.forced.hi"),
            (Some("eng".to_string()), true, true)
        );
        assert_eq!(
            parse_sidecar_suffix_tokens("jpn.sdh"),
            (Some("jpn".to_string()), false, true)
        );
        assert_eq!(
            parse_sidecar_suffix_tokens("commentary.eng"),
            (Some("eng".to_string()), false, false)
        );
        assert_eq!(
            parse_sidecar_suffix_tokens("commentary"),
            (None, false, false)
        );
    }

    #[test]
    fn preserves_existing_rows_and_cache_when_probe_failed_for_same_path() {
        let mut desired_discovered = BTreeMap::new();
        desired_discovered.insert(
            "/tmp/kept.srt".to_string(),
            DiscoveredExternalSubtitle {
                file_path: "/tmp/kept.srt".to_string(),
                language: "eng".to_string(),
                forced: false,
                hearing_impaired: false,
            },
        );
        let mut desired_probe_cache = BTreeMap::new();
        desired_probe_cache.insert(
            "/tmp/kept.srt".to_string(),
            ExternalSubtitleProbeCacheEntry {
                media_file_id: "media-1".to_string(),
                file_path: "/tmp/kept.srt".to_string(),
                size_bytes: 100,
                modified_at: Some("2026-04-30T00:00:00Z".to_string()),
                language: Some("eng".to_string()),
                hearing_impaired: Some(false),
                detection_source_language: ExternalSubtitleDetectionSource::Filename,
                detection_source_hi: ExternalSubtitleDetectionSource::Unknown,
                probe_version: 2,
                updated_at: "2026-04-30T00:00:01Z".to_string(),
            },
        );

        let failed = HashSet::from(["/tmp/failed.srt".to_string()]);

        assert!(should_preserve_existing_discovered_record(
            "/tmp/kept.srt",
            &desired_discovered,
            &failed,
        ));
        assert!(should_preserve_existing_discovered_record(
            "/tmp/failed.srt",
            &desired_discovered,
            &failed,
        ));
        assert!(!should_preserve_existing_discovered_record(
            "/tmp/delete.srt",
            &desired_discovered,
            &failed,
        ));

        assert!(should_preserve_existing_probe_cache_entry(
            "/tmp/kept.srt",
            &desired_probe_cache,
            &failed,
        ));
        assert!(should_preserve_existing_probe_cache_entry(
            "/tmp/failed.srt",
            &desired_probe_cache,
            &failed,
        ));
        assert!(!should_preserve_existing_probe_cache_entry(
            "/tmp/delete.srt",
            &desired_probe_cache,
            &failed,
        ));
    }

    #[tokio::test]
    async fn discovers_same_stem_sidecars_only() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Show.S01E01.mkv");
        let english = tempdir.path().join("Example.Show.S01E01.eng.srt");
        let forced = tempdir.path().join("Example.Show.S01E01.jpn.forced.ass");
        let unrelated = tempdir.path().join("Other.Show.eng.srt");

        fs::write(&video, b"video").expect("video");
        fs::write(&english, b"subtitle").expect("subtitle");
        fs::write(&forced, b"subtitle").expect("subtitle");
        fs::write(&unrelated, b"subtitle").expect("subtitle");

        let discovered = discover_external_subtitles_for_video(&video)
            .await
            .expect("discover subtitles");

        assert_eq!(discovered.len(), 2);
        assert_eq!(discovered[0].language.as_deref(), Some("eng"));
        assert_eq!(discovered[0].file_path, english.to_string_lossy());
        assert_eq!(discovered[1].language.as_deref(), Some("jpn"));
        assert!(discovered[1].forced);
    }

    #[tokio::test]
    async fn cached_discovery_reuses_parent_listing_for_multiple_videos() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let first_video = tempdir.path().join("Example.Show.S01E01.mkv");
        let second_video = tempdir.path().join("Example.Show.S01E02.mkv");
        let first_subtitle = tempdir.path().join("Example.Show.S01E01.eng.srt");
        let second_subtitle = tempdir.path().join("Example.Show.S01E02.jpn.ass");

        fs::write(&first_video, b"video").expect("first video");
        fs::write(&second_video, b"video").expect("second video");
        fs::write(&first_subtitle, b"subtitle").expect("first subtitle");
        fs::write(&second_subtitle, b"subtitle").expect("second subtitle");

        let mut cache = ExternalSubtitleDirectoryCache::default();
        let first = discover_external_subtitles_for_video_with_cache(&first_video, &mut cache)
            .await
            .expect("first discovery");
        let second = discover_external_subtitles_for_video_with_cache(&second_video, &mut cache)
            .await
            .expect("second discovery");

        assert_eq!(cache.subtitle_paths_by_parent.len(), 1);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].file_path, first_subtitle.to_string_lossy());
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].file_path, second_subtitle.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn parses_same_stem_sidecars_for_non_utf8_video_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let video_stem = OsStr::from_bytes(b"Example\xFF.Show")
            .to_string_lossy()
            .into_owned();
        let english_path = Path::new(OsStr::from_bytes(b"/tmp/Example\xFF.Show.eng.srt"));
        let unrelated = Path::new(OsStr::from_bytes(b"/tmp/Other\xFF.Show.eng.srt"));

        let english = parse_discovered_external_subtitle(&video_stem, english_path)
            .expect("matching subtitle sidecar");
        assert_eq!(english.language.as_deref(), Some("eng"));
        assert_eq!(
            english.file_path,
            crate::stored_paths::path_to_stored_string(english_path)
        );
        assert!(parse_discovered_external_subtitle(&video_stem, unrelated).is_none());
    }

    #[tokio::test]
    async fn reconcile_prefers_explicit_filename_language_over_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.eng.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nGracias por venir con nosotros esta noche.\n\n2\n00:00:03,000 --> 00:00:04,000\nTodavia tenemos mucho trabajo por delante.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-1").await);

        let downloads = repo.downloads_for_media_file("media-1").await;
        let cache = repo.probe_cache_for_media_file("media-1").await;

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].language, "eng");
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache[0].detection_source_language,
            ExternalSubtitleDetectionSource::Filename
        );
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn reconcile_uses_content_language_for_languageless_sidecar() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, because this conversation matters to everyone in the room.\n\n2\n00:00:03,000 --> 00:00:04,000\nWe still have plenty of time to solve this together if we keep listening carefully.\n\n3\n00:00:05,000 --> 00:00:06,000\nThe plan is simple, practical, and written in plain English for the whole team.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-2").await);

        let downloads = repo.downloads_for_media_file("media-2").await;
        let cache = repo.probe_cache_for_media_file("media-2").await;

        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].language, "eng");
        assert_eq!(
            cache[0].detection_source_language,
            ExternalSubtitleDetectionSource::Content
        );
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn reconcile_uses_srt_hi_probe_when_filename_has_no_hi_token() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.eng.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\n[door opens]\n\n2\n00:00:03,000 --> 00:00:04,000\n[phone rings]\n\n3\n00:00:05,000 --> 00:00:06,000\n♪ dramatic music builds ♪\n\n4\n00:00:07,000 --> 00:00:08,000\nNARRATOR: We should go now before the storm reaches the harbor.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-3").await);

        let downloads = repo.downloads_for_media_file("media-3").await;
        let cache = repo.probe_cache_for_media_file("media-3").await;

        assert!(downloads[0].hearing_impaired);
        assert_eq!(
            cache[0].detection_source_hi,
            ExternalSubtitleDetectionSource::Content
        );
    }

    #[tokio::test]
    async fn unchanged_sidecar_reuses_cache_without_changes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, because this conversation matters to everyone in the room.\n\n2\n00:00:03,000 --> 00:00:04,000\nWe still have plenty of time to solve this together if we keep listening carefully.\n\n3\n00:00:05,000 --> 00:00:06,000\nThe plan is simple, practical, and written in plain English for the whole team.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-4").await);
        let first_cache = repo.probe_cache_for_media_file("media-4").await;

        assert!(!reconcile_fixture(&app, &video, "media-4").await);
        let second_cache = repo.probe_cache_for_media_file("media-4").await;

        assert_eq!(first_cache, second_cache);
    }

    #[cfg(feature = "runtime-media-analysis")]
    #[tokio::test]
    async fn edited_sidecar_reprobes_when_file_changes() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, because this matters.\n\n2\n00:00:03,000 --> 00:00:04,000\nWe still have plenty of time to solve this together.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-5").await);
        let first_cache = repo.probe_cache_for_media_file("media-5").await;

        // Edit the sidecar and move its mtime forward explicitly, rather than
        // sleeping so the filesystem clock ticks past the first write.
        let first_modified = std::fs::metadata(&subtitle)
            .and_then(|metadata| metadata.modified())
            .expect("subtitle mtime");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\n今夜ここに残ってくれて本当にありがとう。まだやるべきことがたくさんあるので、最後まで一緒に確認しましょう。\n\n2\n00:00:03,000 --> 00:00:04,000\n誰も代わりに解決できないから、私たちが落ち着いて話し合いながら最後までやり切るしかありません。\n\n3\n00:00:05,000 --> 00:00:06,000\nこの計画は複雑ではありません。必要なのは、状況をよく見て丁寧に進めることだけです。\n",
        )
        .expect("subtitle");
        std::fs::File::options()
            .write(true)
            .open(&subtitle)
            .and_then(|file| file.set_modified(first_modified + Duration::from_secs(2)))
            .expect("advance subtitle mtime");

        assert!(reconcile_fixture(&app, &video, "media-5").await);

        let downloads = repo.downloads_for_media_file("media-5").await;
        let second_cache = repo.probe_cache_for_media_file("media-5").await;

        assert_eq!(downloads[0].language, "jpn");
        assert_ne!(first_cache[0].updated_at, second_cache[0].updated_at);
    }

    #[tokio::test]
    async fn sidecar_removal_deletes_subtitle_row_and_cache_row() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, because this matters.\n\n2\n00:00:03,000 --> 00:00:04,000\nWe still have plenty of time to solve this together.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-6").await);
        fs::remove_file(&subtitle).expect("remove subtitle");

        assert!(reconcile_fixture(&app, &video, "media-6").await);
        assert!(repo.downloads_for_media_file("media-6").await.is_empty());
        assert!(repo.probe_cache_for_media_file("media-6").await.is_empty());
    }

    #[tokio::test]
    async fn undecidable_sidecar_is_skipped_but_cache_is_retained() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nla\n\n2\n00:00:03,000 --> 00:00:04,000\nla\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        let app = build_test_app(repo.clone());

        assert!(reconcile_fixture(&app, &video, "media-7").await);

        let downloads = repo.downloads_for_media_file("media-7").await;
        let cache = repo.probe_cache_for_media_file("media-7").await;

        assert!(downloads.is_empty());
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].language, None);
    }

    #[tokio::test]
    async fn downloaded_subtitle_rows_are_untouched_by_probe_cache_logic() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let video = tempdir.path().join("Example.Movie.mkv");
        let subtitle = tempdir.path().join("Example.Movie.eng.srt");
        fs::write(&video, b"video").expect("video");
        fs::write(
            &subtitle,
            "1\n00:00:01,000 --> 00:00:02,000\nThank you for staying with us tonight, because this matters.\n",
        )
        .expect("subtitle");

        let repo = Arc::new(TestSubtitleDownloadRepository::default());
        repo.seed_download(downloaded_subtitle(
            "downloaded-1",
            "media-8",
            "title-1",
            &subtitle.to_string_lossy(),
        ))
        .await;
        let app = build_test_app(repo.clone());

        assert!(!reconcile_fixture(&app, &video, "media-8").await);

        let downloads = repo.downloads_for_media_file("media-8").await;
        let cache = repo.probe_cache_for_media_file("media-8").await;

        assert_eq!(downloads.len(), 1);
        assert_eq!(
            downloads[0].source_kind,
            ExternalSubtitleSourceKind::Downloaded
        );
        assert!(cache.is_empty());
    }
}
