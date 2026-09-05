use super::*;

#[cfg(windows)]
fn current_media_source_signature_scheme() -> &'static str {
    "windows_last_write_100ns_v1"
}

#[cfg(unix)]
fn current_media_source_signature_scheme() -> &'static str {
    "unix_mtime_nsec_v1"
}

#[cfg(all(not(unix), not(windows)))]
fn current_media_source_signature_scheme() -> &'static str {
    "system_time_nsec_v1"
}

#[derive(Clone, Default)]
struct NotifyingLibraryScanner {
    library_files: Arc<Mutex<Vec<LibraryFile>>>,
    directory_scan_calls: Arc<AtomicUsize>,
    directory_scan_started: Arc<Notify>,
    block_directory_scans: Arc<AtomicUsize>,
    release_directory_scans: Arc<Notify>,
}

impl NotifyingLibraryScanner {
    async fn set_library_files(&self, files: Vec<LibraryFile>) {
        *self.library_files.lock().await = files;
    }

    fn block_directory_scans(&self) {
        self.block_directory_scans.store(1, Ordering::SeqCst);
    }

    fn release_directory_scans(&self) {
        self.block_directory_scans.store(0, Ordering::SeqCst);
        self.release_directory_scans.notify_waiters();
    }

    async fn wait_for_directory_scan(&self) {
        if self.directory_scan_calls.load(Ordering::SeqCst) > 0 {
            return;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.directory_scan_started.notified();
                if self.directory_scan_calls.load(Ordering::SeqCst) > 0 {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for title directory scan");
    }
}

#[async_trait]
impl LibraryScanner for NotifyingLibraryScanner {
    async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
        Ok(self.library_files.lock().await.clone())
    }

    async fn scan_directory(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
        self.directory_scan_calls.fetch_add(1, Ordering::SeqCst);
        self.directory_scan_started.notify_waiters();
        loop {
            let notified = self.release_directory_scans.notified();
            if self.block_directory_scans.load(Ordering::SeqCst) == 0 {
                break;
            }
            notified.await;
        }
        Ok(self.library_files.lock().await.clone())
    }

    async fn scan_library_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.library_files.lock().await.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        Ok(rx)
    }

    async fn scan_directory_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.scan_directory(_root).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        Ok(rx)
    }
}

struct HydratingMovieSearchGateway {
    search_item: MetadataSearchItem,
    movie: MovieMetadata,
}

#[async_trait]
impl MetadataGateway for HydratingMovieSearchGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(vec![self.search_item.clone()])
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, vec![self.search_item.clone()]))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        if self.movie.tvdb_id == Some(tvdb_id) {
            Ok(self.movie.clone())
        } else {
            Err(AppError::NotFound(format!("movie {tvdb_id}")))
        }
    }

    async fn get_series(&self, tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(format!("series {tvdb_id}")))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult {
            movies: movie_tvdb_ids
                .iter()
                .filter(|tvdb_id| Some(**tvdb_id) == self.movie.tvdb_id)
                .map(|tvdb_id| (*tvdb_id, self.movie.clone()))
                .collect(),
            series: HashMap::new(),
        })
    }
}

#[derive(Clone, Default)]
struct CountingRecommendationMetadataGateway {
    movies: HashMap<i64, MovieMetadata>,
    title_recommendation_calls: Arc<AtomicUsize>,
    recommendation_release: Option<Arc<Notify>>,
}

#[async_trait]
impl MetadataGateway for CountingRecommendationMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        self.movies
            .get(&tvdb_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let movies = movie_tvdb_ids
            .iter()
            .filter_map(|tvdb_id| {
                self.movies
                    .get(tvdb_id)
                    .cloned()
                    .map(|movie| (*tvdb_id, movie))
            })
            .collect();
        Ok(BulkMetadataResult {
            movies,
            series: HashMap::new(),
        })
    }

    async fn title_recommendations(
        &self,
        _input: &TitleRecommendationsInput,
    ) -> AppResult<DiscoveryRelatedResult> {
        self.title_recommendation_calls
            .fetch_add(1, Ordering::SeqCst);
        if let Some(release) = &self.recommendation_release {
            release.notified().await;
        }
        Ok(DiscoveryRelatedResult {
            subject_key: "tvdb:movie:1".to_string(),
            query: String::new(),
            generated_at: "2026-01-01T00:00:00Z".to_string(),
            results: Vec::new(),
        })
    }
}

#[derive(Clone, Default)]
struct PerDirectoryBlockingLibraryScanner {
    library_files: Arc<Mutex<Vec<LibraryFile>>>,
    directory_files: Arc<Mutex<std::collections::HashMap<String, Vec<LibraryFile>>>>,
    scanned_directories: Arc<Mutex<Vec<String>>>,
    blocked_directories: Arc<Mutex<std::collections::HashSet<String>>>,
    blocked_scan_calls: Arc<AtomicUsize>,
    blocked_scan_started: Arc<Notify>,
    release_blocked_scans: Arc<Notify>,
}

impl PerDirectoryBlockingLibraryScanner {
    async fn set_library_files(&self, files: Vec<LibraryFile>) {
        *self.library_files.lock().await = files;
    }

    async fn set_directory_files(&self, root: &std::path::Path, files: Vec<LibraryFile>) {
        self.directory_files
            .lock()
            .await
            .insert(root.to_string_lossy().to_string(), files);
    }

    async fn scanned_directories(&self) -> Vec<String> {
        self.scanned_directories.lock().await.clone()
    }

    async fn block_directory(&self, root: &std::path::Path) {
        self.blocked_directories
            .lock()
            .await
            .insert(root.to_string_lossy().to_string());
    }

    async fn release_blocked_directory_scans(&self) {
        self.blocked_directories.lock().await.clear();
        self.release_blocked_scans.notify_waiters();
    }

    async fn release_blocked_directory_scan(&self, root: &std::path::Path) {
        self.blocked_directories
            .lock()
            .await
            .remove::<str>(root.to_string_lossy().as_ref());
        self.release_blocked_scans.notify_waiters();
    }

    async fn wait_for_blocked_directory_scan(&self) {
        if self.blocked_scan_calls.load(Ordering::SeqCst) > 0 {
            return;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.blocked_scan_started.notified();
                if self.blocked_scan_calls.load(Ordering::SeqCst) > 0 {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for blocked title directory scan");
    }

    async fn wait_for_blocked_directory_scan_count(&self, expected: usize) {
        if self.blocked_scan_calls.load(Ordering::SeqCst) >= expected {
            return;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.blocked_scan_started.notified();
                if self.blocked_scan_calls.load(Ordering::SeqCst) >= expected {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for concurrent blocked title directory scans");
    }
}

#[async_trait]
impl LibraryScanner for PerDirectoryBlockingLibraryScanner {
    async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
        Ok(self.library_files.lock().await.clone())
    }

    async fn scan_directory(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        loop {
            let should_block = self.blocked_directories.lock().await.contains(root);
            if !should_block {
                break;
            }
            self.blocked_scan_calls.fetch_add(1, Ordering::SeqCst);
            self.blocked_scan_started.notify_waiters();
            self.release_blocked_scans.notified().await;
        }
        Ok(self
            .directory_files
            .lock()
            .await
            .get(root)
            .cloned()
            .unwrap_or_default())
    }

    // The shallow evidence listing never blocks: it models the real
    // scanner's single readdir, while the recursive walk above can be held.
    async fn scan_directory_children(&self, root: &str) -> AppResult<Vec<LibraryFile>> {
        self.scanned_directories.lock().await.push(root.to_string());
        let root_path = std::path::Path::new(root).to_path_buf();
        Ok(self
            .directory_files
            .lock()
            .await
            .get(root)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|file| {
                std::path::Path::new(&file.path)
                    .parent()
                    .is_some_and(|parent| parent == root_path.as_path())
            })
            .collect())
    }

    async fn scan_library_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.library_files.lock().await.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        Ok(rx)
    }

    async fn scan_directory_batched(
        &self,
        root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.scan_directory(root).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        Ok(rx)
    }
}

#[derive(Clone, Default)]
struct BlockingMediaAnalyzer {
    analyze_calls: Arc<AtomicUsize>,
    active_calls: Arc<AtomicUsize>,
    max_active_calls: Arc<AtomicUsize>,
    analyze_started: Arc<Notify>,
    block_analysis: Arc<AtomicUsize>,
    release_analysis: Arc<Notify>,
}

impl BlockingMediaAnalyzer {
    fn block(&self) {
        self.block_analysis.store(1, Ordering::SeqCst);
    }

    fn release(&self) {
        self.block_analysis.store(0, Ordering::SeqCst);
        self.release_analysis.notify_waiters();
    }

    async fn wait_for_analysis(&self) {
        if self.analyze_calls.load(Ordering::SeqCst) > 0 {
            return;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.analyze_started.notified();
                if self.analyze_calls.load(Ordering::SeqCst) > 0 {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for media analysis");
    }

    async fn wait_for_active_analysis(&self, expected: usize) {
        if self.active_calls.load(Ordering::SeqCst) >= expected {
            return;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.analyze_started.notified();
                if self.active_calls.load(Ordering::SeqCst) >= expected {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for active media analysis");
    }

    fn max_active_calls(&self) -> usize {
        self.max_active_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl MediaAnalyzer for BlockingMediaAnalyzer {
    async fn analyze_file(&self, _path: std::path::PathBuf) -> AppResult<MediaAnalysisOutcome> {
        self.analyze_calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let mut observed = self.max_active_calls.load(Ordering::SeqCst);
        while active > observed {
            match self.max_active_calls.compare_exchange(
                observed,
                active,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(current) => observed = current,
            }
        }
        self.analyze_started.notify_waiters();
        loop {
            let notified = self.release_analysis.notified();
            if self.block_analysis.load(Ordering::SeqCst) == 0 {
                break;
            }
            notified.await;
        }
        self.active_calls.fetch_sub(1, Ordering::SeqCst);
        Ok(MediaAnalysisOutcome::Invalid(
            "blocked test analyzer".to_string(),
        ))
    }
}

#[derive(Clone, Default)]
struct CountingValidMediaAnalyzer {
    analyze_calls: Arc<AtomicUsize>,
}

impl CountingValidMediaAnalyzer {
    fn analyze_calls(&self) -> usize {
        self.analyze_calls.load(Ordering::SeqCst)
    }
}

fn test_valid_media_analysis() -> MediaFileAnalysis {
    MediaFileAnalysis {
        video_codec: None,
        video_width: Some(1920),
        video_height: Some(1080),
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        dovi_profile: None,
        dovi_bl_compat_id: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: None,
        audio_profile: None,
        audio_channels: None,
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: Some(60),
        num_chapters: None,
        container_format: Some("Matroska".to_string()),
    }
}

#[async_trait]
impl MediaAnalyzer for CountingValidMediaAnalyzer {
    async fn analyze_file(&self, _path: std::path::PathBuf) -> AppResult<MediaAnalysisOutcome> {
        self.analyze_calls.fetch_add(1, Ordering::SeqCst);
        Ok(MediaAnalysisOutcome::Valid(Box::new(
            test_valid_media_analysis(),
        )))
    }
}

type MetadataSearchBatch = (Vec<MetadataSearchQuery>, String, bool);

#[derive(Clone, Default)]
struct RecordingExactIdMetadataGateway {
    batch_queries: Arc<Mutex<Vec<Vec<MetadataSearchQuery>>>>,
    title_batch_queries: Arc<Mutex<Vec<MetadataSearchBatch>>>,
    title_id_movies_enabled: bool,
    rich_external_ids: bool,
    raw_title_id_error: bool,
    detail_calls: Arc<AtomicUsize>,
}

impl RecordingExactIdMetadataGateway {
    fn with_title_id_movies() -> Self {
        Self {
            title_id_movies_enabled: true,
            ..Default::default()
        }
    }

    /// SMG answers every facet with the full identity set, and every facet keeps
    /// it: a scan-created series or anime title carries the same identity set a
    /// movie does.
    fn with_rich_external_ids(mut self) -> Self {
        self.rich_external_ids = true;
        self
    }

    /// An SMG that predates the title-id surface answers `searchTitlesBatch`
    /// with a raw GraphQL validation error, not with the mapped capability
    /// message the client produces once its probe recognises one.
    fn with_raw_unknown_field_error(mut self) -> Self {
        self.raw_title_id_error = true;
        self
    }

    fn rich_external_ids(&self) -> Vec<ExternalId> {
        if !self.rich_external_ids {
            return vec![];
        }
        vec![
            ExternalId {
                source: "smg".to_string(),
                value: "5555".to_string(),
            },
            ExternalId {
                source: "tmdb".to_string(),
                value: "6666".to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt0055555".to_string(),
            },
        ]
    }

    async fn batch_queries(&self) -> Vec<Vec<MetadataSearchQuery>> {
        self.batch_queries.lock().await.clone()
    }

    async fn title_batch_queries(&self) -> Vec<(Vec<MetadataSearchQuery>, String, bool)> {
        self.title_batch_queries.lock().await.clone()
    }

    fn detail_calls(&self) -> usize {
        self.detail_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl MetadataGateway for RecordingExactIdMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        self.batch_queries.lock().await.push(queries.to_vec());
        Ok(queries
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, query)| {
                let tvdb_id = query
                    .tvdb_id
                    .clone()
                    .or_else(|| query.tmdb_id.clone())
                    .unwrap_or_else(|| format!("900{index:03}"));
                (
                    query,
                    vec![MetadataSearchItem {
                        name: format!("Deliberately Different Identity Title {tvdb_id}"),
                        tvdb_id,
                        smg_id: self.rich_external_ids.then_some(5_555),
                        primary_source: None,
                        external_ids: self.rich_external_ids(),
                        year: Some(1901),
                        auto_match_safe: true,
                        auto_match_signals: vec!["identity".to_string()],
                    }],
                )
            })
            .collect())
    }

    async fn search_titles_batch(
        &self,
        queries: &[MetadataSearchQuery],
        kind: &str,
        _language: &str,
        create_missing: bool,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        self.title_batch_queries.lock().await.push((
            queries.to_vec(),
            kind.to_string(),
            create_missing,
        ));
        if !self.title_id_movies_enabled {
            return Err(AppError::Repository(if self.raw_title_id_error {
                "Cannot query field \"searchTitlesBatch\" on type \"Query\".".to_string()
            } else {
                "metadata gateway does not support title-id queries".to_string()
            }));
        }

        assert_eq!(kind, "movie");
        Ok(queries
            .iter()
            .cloned()
            .map(|query| {
                let tmdb_id = query.tmdb_id.clone().expect("tmdb identity hint");
                (
                    query,
                    vec![MetadataSearchItem {
                        name: "TMDB Primary Match".to_string(),
                        tvdb_id: if self.rich_external_ids {
                            "444444".to_string()
                        } else {
                            String::new()
                        },
                        smg_id: Some(7_777),
                        primary_source: Some("tmdb".to_string()),
                        external_ids: vec![
                            ExternalId {
                                source: "tmdb".to_string(),
                                value: tmdb_id,
                            },
                            ExternalId {
                                source: "imdb".to_string(),
                                value: "tt0077777".to_string(),
                            },
                        ],
                        year: Some(2020),
                        auto_match_safe: true,
                        auto_match_signals: vec!["external_id:tmdb".to_string()],
                    }],
                )
            })
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        self.detail_calls.fetch_add(1, Ordering::SeqCst);
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        self.detail_calls.fetch_add(1, Ordering::SeqCst);
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult::default())
    }
}

#[derive(Clone, Default)]
struct BlockingBulkHydrationMetadataGateway {
    bulk_calls: Arc<AtomicUsize>,
    bulk_started: Arc<Notify>,
    released_through: Arc<AtomicUsize>,
    release_notify: Arc<Notify>,
}

impl BlockingBulkHydrationMetadataGateway {
    async fn wait_for_bulk_calls(&self, expected_calls: usize) {
        if self.bulk_calls.load(Ordering::SeqCst) >= expected_calls {
            return;
        }
        timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.bulk_started.notified();
                if self.bulk_calls.load(Ordering::SeqCst) >= expected_calls {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("timed out waiting for bulk hydration");
    }

    fn release_through(&self, call_number: usize) {
        self.released_through.store(call_number, Ordering::SeqCst);
        self.release_notify.notify_waiters();
    }

    fn release_all(&self) {
        self.release_through(usize::MAX);
    }
}

#[async_trait]
impl MetadataGateway for BlockingBulkHydrationMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, Vec::new()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let call_number = self.bulk_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.bulk_started.notify_waiters();
        while self.released_through.load(Ordering::SeqCst) < call_number {
            self.release_notify.notified().await;
        }
        Ok(BulkMetadataResult {
            movies: movie_tvdb_ids
                .iter()
                .map(|tvdb_id| {
                    (
                        *tvdb_id,
                        make_movie_metadata(*tvdb_id, "Hydrated Test Title"),
                    )
                })
                .collect(),
            series: HashMap::new(),
        })
    }
}

#[tokio::test]
async fn manual_title_create_without_hydration_does_not_fetch_poster() {
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::default());
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway.clone());

    let created = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Fixture 1234".to_string(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "1234".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    assert_eq!(metadata_gateway.detail_calls(), 0);
    assert_eq!(created.title.poster_url, None);
}

#[tokio::test]
async fn movie_full_scan_persists_and_reconciles_unmatched_items() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let first_folder = tempdir.path().join("Unknown One (2020)");
    std::fs::create_dir(&first_folder).expect("create first movie folder");
    let first_path = first_folder.join("Unknown.One.2020.1080p.WEB-DL.mkv");
    std::fs::write(&first_path, b"movie").expect("write first movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        library_scanner.clone(),
        unmatched_items.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let first_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("first movie scan");
    assert_eq!(first_summary.scanned, 1);
    assert_eq!(first_summary.unmatched, 1);

    let first_items = unmatched_items.items().await;
    assert_eq!(first_items.len(), 1);
    assert_eq!(first_items[0].facet, MediaFacet::Movie);
    assert_eq!(first_items[0].item_path, first_folder.to_string_lossy());
    let first_session_id = first_items[0].scan_session_id.clone();

    let second_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("second movie scan");
    assert_eq!(second_summary.unmatched, 1);

    let second_items = unmatched_items.items().await;
    assert_eq!(second_items.len(), 1);
    assert_ne!(second_items[0].scan_session_id, first_session_id);

    std::fs::remove_dir_all(&first_folder).expect("remove first movie folder");
    let second_folder = tempdir.path().join("Unknown Two (2021)");
    std::fs::create_dir(&second_folder).expect("create second movie folder");
    let second_path = second_folder.join("Unknown.Two.2021.2160p.BluRay.mkv");
    std::fs::write(&second_path, b"movie").expect("write second movie file");
    let third_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("third movie scan");
    assert_eq!(third_summary.scanned, 1);
    assert_eq!(third_summary.unmatched, 1);

    let third_items = unmatched_items.items().await;
    assert_eq!(third_items.len(), 1);
    assert_eq!(third_items[0].item_path, second_folder.to_string_lossy());
}

#[tokio::test]
async fn title_scan_returns_error_when_one_off_hydration_fails() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let (app, user, _) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile movie root");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Hydration Missing".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".into(),
                    value: "900001".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");

    let error = app
        .scan_title_library(&user, &title.id)
        .await
        .expect_err("one-off hydration failure should propagate");

    assert!(
        error
            .to_string()
            .contains("bulk metadata response missing title"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn title_scan_returns_error_when_one_off_walk_fails() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile movie root");
    let title = create_movie_title_with_folder(&app, &user, "Broken Root", tempdir.path()).await;
    {
        let mut store = titles.store.lock().await;
        let stored = store
            .iter_mut()
            .find(|candidate| candidate.id == title.id)
            .expect("stored title");
        stored.root_folder_id = "missing-root".to_string();
    }

    let error = app
        .scan_title_library(&user, &title.id)
        .await
        .expect_err("one-off walk failure should propagate");

    assert!(
        error.to_string().contains("missing-root"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn movie_title_scan_removes_missing_tracked_movie_file() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Ironclad (1997) - 2160p.mkv");
    std::fs::write(&movie_path, b"movie").expect("write movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items);
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Ironclad".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                year: Some(1997),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, tempdir.path().to_string_lossy().as_ref())
        .await
        .expect("set movie folder path");

    let movie_path_string = movie_path.to_string_lossy().to_string();
    app.services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("2160p".to_string()),
            ordered_path: Some(movie_path_string.clone()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: title.monitored,
            created_at: Utc::now(),
        })
        .await
        .expect("seed movie collection");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: movie_path_string,
            size_bytes: 5,
            quality_label: Some("2160p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed movie media file");

    std::fs::remove_file(&movie_path).expect("remove movie file externally");

    let summary = app
        .scan_title_library(&user, &title.id)
        .await
        .expect("movie title scan should succeed");

    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );
    assert!(
        app.services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .expect("list collections")
            .is_empty()
    );
}

#[tokio::test]
async fn movie_title_scan_multiple_files_picks_initial_primary_and_marks_rest_additional() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Primary Choice (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    // Both files sit inside the built-in default (1080p) profile's tiers so
    // the pick exercises relative scoring rather than quality gating.
    let small_path = title_dir.join("Primary.Choice.2026.720p.WEB-DL.mkv");
    let large_path = title_dir.join("Primary.Choice.2026.1080p.WEB-DL.mkv");
    std::fs::write(&small_path, vec![0_u8; 128]).expect("write smaller movie file");
    std::fs::write(&large_path, vec![0_u8; 512]).expect("write larger movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[small_path.as_path(), large_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Primary Choice", title_dir.as_path()).await;

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 2);
    assert_eq!(
        files.iter().filter(|file| file.role.is_primary()).count(),
        1
    );
    assert_eq!(
        media_file_role_for_path(&files, large_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, small_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_title_scan_backfills_stale_mtime_signature_then_skips_current_signature() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Signature Backfill (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let movie_path = title_dir.join("Signature.Backfill.2026.1080p.WEB-DL.mkv");
    std::fs::write(&movie_path, b"stable media bytes").expect("write movie file");

    let (base_app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        Vec::new(),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let analyzer = Arc::new(CountingValidMediaAnalyzer::default());
    let app = base_app.with_test_overrides(|builder| builder.with_media_analyzer(analyzer.clone()));
    let title =
        create_movie_title_with_folder(&app, &user, "Signature Backfill", title_dir.as_path())
            .await;
    let file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: movie_path.to_string_lossy().to_string(),
            size_bytes: 18,
            role: MediaFileRole::Primary,
            source_signature_scheme: Some(current_media_source_signature_scheme().to_string()),
            source_signature_value: Some("stale".to_string()),
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed legacy media file");
    app.services
        .library
        .media_files
        .update_media_file_analysis(&file_id, test_valid_media_analysis())
        .await
        .expect("mark seeded file analyzed");

    app.scan_title_library_with_discovered_files(
        &user,
        title.clone(),
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("scan stale signature file");
    assert_eq!(
        analyzer.analyze_calls(),
        1,
        "stale source signature should force one MediaInfo re-analysis"
    );
    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after backfill");
    let file = files
        .iter()
        .find(|file| file.file_path == movie_path.to_string_lossy())
        .expect("media file exists after backfill");
    assert_eq!(
        file.source_signature_scheme.as_deref(),
        Some(current_media_source_signature_scheme())
    );
    assert!(
        file.source_signature_value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        "backfilled signature value should be stored"
    );

    app.scan_title_library_with_discovered_files(
        &user,
        title,
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("scan current signature file");
    assert_eq!(
        analyzer.analyze_calls(),
        1,
        "current mtime source signature should skip MediaInfo"
    );
}

#[tokio::test]
async fn movie_title_scan_backfills_missing_source_signature_without_media_info() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Signature Missing (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let movie_path = title_dir.join("Signature.Missing.2026.1080p.WEB-DL.mkv");
    std::fs::write(&movie_path, b"stable media bytes").expect("write movie file");

    let (base_app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        Vec::new(),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let analyzer = Arc::new(CountingValidMediaAnalyzer::default());
    let app = base_app.with_test_overrides(|builder| builder.with_media_analyzer(analyzer.clone()));
    let title =
        create_movie_title_with_folder(&app, &user, "Signature Missing", title_dir.as_path()).await;
    let file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: movie_path.to_string_lossy().to_string(),
            size_bytes: 18,
            role: MediaFileRole::Primary,
            source_signature_scheme: None,
            source_signature_value: None,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed media file without source signature");
    app.services
        .library
        .media_files
        .update_media_file_analysis(&file_id, test_valid_media_analysis())
        .await
        .expect("mark seeded file analyzed");

    app.scan_title_library_with_discovered_files(
        &user,
        title.clone(),
        vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )],
    )
    .await
    .expect("scan missing signature file");

    assert_eq!(
        analyzer.analyze_calls(),
        0,
        "missing source signature should be backfilled without rerunning MediaInfo"
    );
    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after backfill");
    let file = files
        .iter()
        .find(|file| file.file_path == movie_path.to_string_lossy())
        .expect("media file exists after backfill");
    assert_eq!(
        file.source_signature_scheme.as_deref(),
        Some(current_media_source_signature_scheme())
    );
    assert!(
        file.source_signature_value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        "backfilled signature value should be stored"
    );
}

#[tokio::test]
async fn movie_library_scan_does_not_promote_additional_file_but_title_scan_does() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Additional Only (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let additional_path = title_dir.join("Additional.Only.2026.1080p.WEB-DL.mkv");
    std::fs::write(&additional_path, vec![0_u8; 256]).expect("write additional movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[additional_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Additional Only", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: additional_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Additional,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed additional file");

    app.scan_library(&user, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after library scan");
    assert_eq!(
        media_file_role_for_path(&files, additional_path.as_path()),
        MediaFileRole::Additional
    );

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after title scan");
    assert_eq!(
        media_file_role_for_path(&files, additional_path.as_path()),
        MediaFileRole::Primary
    );
}

#[tokio::test]
async fn series_title_scan_imports_episode_file_as_primary() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Fresh Show (2026)");
    std::fs::create_dir(&title_dir).expect("create series folder");
    let episode_path = title_dir.join("Fresh Show - 1x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&episode_path, vec![0_u8; 128]).expect("write episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(build_test_library_files(&[episode_path.as_path()]))
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fresh Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_dir.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan series title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(
        media_file_role_for_path(&files, episode_path.as_path()),
        MediaFileRole::Primary
    );
}

#[tokio::test]
async fn series_library_scan_marks_duplicate_episode_files_as_additional() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Fresh Show (2026)");
    std::fs::create_dir(&title_dir).expect("create series folder");
    let small_episode_path = title_dir.join("Fresh Show - 1x01 - Pilot WEBDL-720p.mkv");
    let large_episode_path = title_dir.join("Fresh Show - 1x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&small_episode_path, vec![0_u8; 128]).expect("write smaller episode file");
    std::fs::write(&large_episode_path, vec![0_u8; 512]).expect("write larger episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(build_test_library_files(&[
            small_episode_path.as_path(),
            large_episode_path.as_path(),
        ]))
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fresh Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_dir.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    app.scan_library(&user, MediaFacet::Series)
        .await
        .expect("scan series library");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 2);
    assert_eq!(
        files.iter().filter(|file| file.role.is_primary()).count(),
        1
    );
    assert_eq!(
        media_file_role_for_path(&files, large_episode_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, small_episode_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn series_library_scan_does_not_promote_additional_file_but_title_scan_does() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Additional Show (2026)");
    std::fs::create_dir(&title_dir).expect("create series folder");
    let episode_path = title_dir.join("Additional Show - 1x01 - Pilot WEBDL-1080p.mkv");
    std::fs::write(&episode_path, vec![0_u8; 256]).expect("write additional episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(build_test_library_files(&[episode_path.as_path()]))
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_dir.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    let episode = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");
    let file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: episode_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Additional,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed additional episode file");
    app.services
        .library
        .media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link additional file to episode");

    app.scan_library(&user, MediaFacet::Series)
        .await
        .expect("scan series library");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after library scan");
    assert_eq!(
        media_file_role_for_path(&files, episode_path.as_path()),
        MediaFileRole::Additional
    );

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan series title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files after title scan");
    assert_eq!(
        media_file_role_for_path(&files, episode_path.as_path()),
        MediaFileRole::Primary
    );
}

#[tokio::test]
async fn movie_title_scan_preserves_existing_primary_even_when_other_file_scores_better() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Stable Primary (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let primary_path = title_dir.join("Stable.Primary.2026.720p.WEB-DL.mkv");
    let additional_path = title_dir.join("Stable.Primary.2026.2160p.BluRay.mkv");
    std::fs::write(&primary_path, vec![0_u8; 128]).expect("write primary movie file");
    std::fs::write(&additional_path, vec![0_u8; 1024]).expect("write additional movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[primary_path.as_path(), additional_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Stable Primary", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: primary_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            quality_label: Some("720p".to_string()),
            acquisition_score: Some(1),
            ..Default::default()
        })
        .await
        .expect("seed primary file");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: additional_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            role: MediaFileRole::Additional,
            quality_label: Some("2160p".to_string()),
            acquisition_score: Some(100_000),
            ..Default::default()
        })
        .await
        .expect("seed additional file");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_file_role_for_path(&files, primary_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, additional_path.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_title_scan_preserves_user_selected_primary() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("User Primary (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let old_path = title_dir.join("User.Primary.2026.720p.WEB-DL.mkv");
    let selected_path = title_dir.join("User.Primary.2026.1080p.WEB-DL.mkv");
    std::fs::write(&old_path, vec![0_u8; 128]).expect("write old primary movie file");
    std::fs::write(&selected_path, vec![0_u8; 256]).expect("write selected movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[old_path.as_path(), selected_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "User Primary", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: old_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed old primary");
    let selected_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: selected_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("seed selected file");
    app.set_primary_movie_file(&user, &title.id, &selected_id)
        .await
        .expect("select primary file");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_file_role_for_path(&files, selected_path.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, old_path.as_path()),
        MediaFileRole::Additional
    );
}

/// Seeds two files that both claim the primary role and scans the title.
/// `first` is inserted before `second` so age would pick `first` under the
/// old oldest-wins repair; callers assert which one the ladder keeps.
async fn scan_movie_title_with_two_primaries(
    first: &str,
    first_size: usize,
    second: &str,
    second_size: usize,
) -> (std::path::PathBuf, std::path::PathBuf, Vec<TitleMediaFile>) {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let title_dir = tempdir.path().join("Primary Repair (2026)");
    std::fs::create_dir(&title_dir).expect("create movie folder");
    let first_path = title_dir.join(first);
    let second_path = title_dir.join(second);
    std::fs::write(&first_path, vec![0_u8; first_size]).expect("write first primary movie file");
    std::fs::write(&second_path, vec![0_u8; second_size]).expect("write second primary movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[first_path.as_path(), second_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Primary Repair", title_dir.as_path()).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: first_path.to_string_lossy().to_string(),
            size_bytes: first_size as i64,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed first primary");
    sleep(Duration::from_millis(2)).await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: second_path.to_string_lossy().to_string(),
            size_bytes: second_size as i64,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed second primary");

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    (first_path, second_path, files)
}

/// Two primaries are repaired on the gate's ladder, not on age: the newer
/// 1080p file outranks the older 720p file in the built-in profile and keeps
/// the role. Oldest-wins used to demote a freshly imported upgrade behind the
/// file it replaced, after which every gate read the scope through the worse
/// file and admitted the same upgrade again on every sync.
#[tokio::test]
async fn movie_title_scan_repairs_multiple_primaries_by_keeping_the_best_ranked_file() {
    let (older_720p, newer_1080p, files) = scan_movie_title_with_two_primaries(
        "Primary.Repair.2026.720p.WEB-DL.mkv",
        128,
        "Primary.Repair.2026.1080p.WEB-DL.mkv",
        1024,
    )
    .await;
    assert_eq!(
        media_file_role_for_path(&files, newer_1080p.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, older_720p.as_path()),
        MediaFileRole::Additional
    );
}

/// The ladder is the profile's, so a resolution the profile does not allow
/// ranks below every allowed one: a 2160p file loses the role to a 1080p file
/// under the built-in 1080p profile however large or new it is.
#[tokio::test]
async fn movie_title_scan_primary_repair_ranks_on_the_profile_ladder_not_resolution() {
    let (older_1080p, newer_2160p, files) = scan_movie_title_with_two_primaries(
        "Primary.Repair.2026.1080p.WEB-DL.mkv",
        128,
        "Primary.Repair.2026.2160p.WEB-DL.mkv",
        1024,
    )
    .await;
    assert_eq!(
        media_file_role_for_path(&files, older_1080p.as_path()),
        MediaFileRole::Primary
    );
    assert_eq!(
        media_file_role_for_path(&files, newer_2160p.as_path()),
        MediaFileRole::Additional
    );
}

#[tokio::test]
async fn movie_title_scan_cleans_out_of_canonical_folder_pollution() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let canonical_dir = tempdir.path().join("Polluted Movie (2026)");
    let duplicate_dir = tempdir.path().join("Polluted Movie Copy (2026)");
    std::fs::create_dir(&canonical_dir).expect("create canonical movie folder");
    std::fs::create_dir(&duplicate_dir).expect("create duplicate movie folder");
    let canonical_path = canonical_dir.join("Polluted.Movie.2026.1080p.WEB-DL.mkv");
    let duplicate_path = duplicate_dir.join("Polluted.Movie.2026.720p.WEB-DL.mkv");
    std::fs::write(&canonical_path, vec![0_u8; 256]).expect("write canonical movie file");
    std::fs::write(&duplicate_path, vec![0_u8; 128]).expect("write duplicate movie file");

    let (app, user, _) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[canonical_path.as_path()]),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Polluted Movie", canonical_dir.as_path())
            .await;
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: canonical_path.to_string_lossy().to_string(),
            size_bytes: 256,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed canonical media file");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: duplicate_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed duplicate media file");
    for (path, id) in [
        (&canonical_path, "canonical"),
        (&duplicate_path, "duplicate"),
    ] {
        app.services
            .catalog
            .shows
            .create_collection(Collection {
                id: format!("collection-{id}"),
                title_id: title.id.clone(),
                collection_type: CollectionType::Movie,
                collection_index: id.to_string(),
                label: None,
                ordered_path: Some(path.to_string_lossy().to_string()),
                narrative_order: None,
                first_episode_number: None,
                last_episode_number: None,
                monitored: title.monitored,
                created_at: Utc::now(),
            })
            .await
            .expect("seed movie collection");
    }

    app.scan_title_library(&user, &title.id)
        .await
        .expect("scan movie title");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_path, canonical_path.to_string_lossy());
    assert_eq!(files[0].role, MediaFileRole::Primary);

    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(canonical_path.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn movie_full_scan_records_duplicate_same_title_sibling_folder_as_ownership_conflict() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let first_dir = tempdir.path().join("Duplicate Title (2026)");
    let second_dir = tempdir.path().join("Duplicate Title Copy (2026)");
    std::fs::create_dir(&first_dir).expect("create first movie folder");
    std::fs::create_dir(&second_dir).expect("create second movie folder");
    let first_path = first_dir.join("Duplicate.Title.2026.1080p.WEB-DL.mkv");
    let second_path = second_dir.join("Duplicate.Title.2026.720p.WEB-DL.mkv");
    std::fs::write(&first_path, vec![0_u8; 256]).expect("write first movie file");
    std::fs::write(&second_path, vec![0_u8; 128]).expect("write second movie file");

    let (app, user, unmatched_items) = bootstrap_movie_scan_app(
        tempdir.path(),
        build_test_library_files(&[first_path.as_path(), second_path.as_path()]),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "112233".to_string(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Duplicate Title".to_string(),
                year: Some(2026),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        }),
    )
    .await;
    let title =
        create_movie_title_with_folder(&app, &user, "Duplicate Title", first_dir.as_path()).await;

    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.unmatched, 1);
    let unmatched = unmatched_items.items().await;
    assert_eq!(unmatched.len(), 1);
    assert_eq!(
        unmatched[0].reason_code,
        crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER
    );
    assert_eq!(unmatched[0].title_id.as_ref(), Some(&title.id));
    assert_eq!(unmatched[0].item_path, second_path.to_string_lossy());
    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].id, title.id);
    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_path, first_path.to_string_lossy());
    assert!(first_path.exists());
    assert!(second_path.exists());
}

#[cfg(not(windows))]
#[tokio::test]
async fn series_full_scan_records_case_distinct_folder_as_ownership_conflict() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let owned_folder = tempdir.path().join("CASE SPLIT FIXTURE");
    let candidate_folder = tempdir.path().join("Case Split Fixture");
    std::fs::create_dir(&candidate_folder).expect("create case-distinct series folder");
    let episode_path = candidate_folder.join("Case Split Fixture - S01E01.mkv");
    std::fs::write(&episode_path, vec![0_u8; 128]).expect("write episode file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Case Split Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, owned_folder.to_string_lossy().as_ref())
        .await
        .expect("set owned series folder");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: episode_path.to_string_lossy().to_string(),
            size_bytes: 128,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed catalog row in losing folder");

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("scan series library");

    assert_eq!(summary.unmatched, 1);
    let unmatched = unmatched_items.items().await;
    assert_eq!(unmatched.len(), 1);
    assert_eq!(
        unmatched[0].reason_code,
        crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER
    );
    assert_eq!(unmatched[0].title_id.as_ref(), Some(&title.id));
    assert_eq!(unmatched[0].item_path, candidate_folder.to_string_lossy());
    assert!(episode_path.exists());
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );
}

#[tokio::test]
async fn movie_full_scan_external_id_nfo_without_gateway_match_persists_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_folder = tempdir.path().join("Broken Movie (2020)");
    std::fs::create_dir(&movie_folder).expect("create movie folder");
    let movie_path = movie_folder.join("Broken.Movie.2020.mkv");
    let nfo_path = movie_folder.join("movie.nfo");
    std::fs::write(&movie_path, b"movie").expect("write movie file");
    std::fs::write(
        &nfo_path,
        r#"<movie><title>Broken Movie</title><tvdbid>123456</tvdbid></movie>"#,
    )
    .expect("write nfo");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![LibraryFile {
            path: movie_path.to_string_lossy().to_string(),
            display_name: "Broken.Movie.2020".to_string(),
            nfo_path: Some(nfo_path.to_string_lossy().to_string()),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, _titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");
    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "no_metadata_search_results");
    assert_eq!(items[0].error_message, None);
    assert_eq!(items[0].item_path, movie_path.to_string_lossy());
}

#[tokio::test]
async fn movie_full_scan_title_create_failure_from_search_persists_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_folder = tempdir.path().join("Matched Movie (2020)");
    std::fs::create_dir(&movie_folder).expect("create movie folder");
    let movie_path = movie_folder.join("Matched.Movie.2020.mkv");
    std::fs::write(&movie_path, b"movie").expect("write movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "123456".to_string(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Matched Movie".to_string(),
                year: Some(2020),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");
    titles
        .fail_create_or_get_existing("forced movie title creation failure from search")
        .await;

    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "title_create_from_search_failed");
    assert_eq!(
        items[0].error_message.as_deref(),
        Some("repository: forced movie title creation failure from search")
    );
    assert_eq!(items[0].item_path, movie_path.to_string_lossy());
}

#[tokio::test]
async fn series_full_scan_persists_unmatched_folders() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(tempdir.path().join("Unknown Show (2020)"))
        .expect("create unknown show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("series scan");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Series);
    assert_eq!(items[0].display_name, "Unknown Show (2020)");
    assert_eq!(
        items[0].scan_root,
        tempdir.path().to_string_lossy().to_string()
    );
    assert_eq!(
        items[0].item_path,
        tempdir
            .path()
            .join("Unknown Show (2020)")
            .to_string_lossy()
            .to_string()
    );
}

#[tokio::test]
async fn movie_full_scan_scans_all_configured_roots_in_one_session() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create movie root one");
    std::fs::create_dir_all(&root_two).expect("create movie root two");
    let movie_one = root_one.join("Unknown One (2020)");
    let movie_two = root_two.join("Unknown Two (2021)");
    std::fs::create_dir(&movie_one).expect("create movie one folder");
    std::fs::create_dir(&movie_two).expect("create movie two folder");
    std::fs::write(movie_one.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie one");
    std::fs::write(movie_two.join("Unknown.Two.2021.mkv"), b"movie-two").expect("seed movie two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-multi-root-full-scan";
    let summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Movie,
            Some(session_id.to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("movie full scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 2);
    assert_eq!(projected.status, LibraryScanStatus::Completed);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_one.to_string_lossy())
    );
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_two.to_string_lossy())
    );
}

#[tokio::test]
async fn series_full_scan_scans_all_configured_roots_in_one_session() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("series-a");
    let root_two = tempdir.path().join("series-b");
    std::fs::create_dir_all(root_one.join("Unknown Show One (2020)"))
        .expect("create first show folder");
    std::fs::create_dir_all(root_two.join("Unknown Show Two (2021)"))
        .expect("create second show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store series roots");

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("series full scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_one.to_string_lossy())
    );
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_two.to_string_lossy())
    );
}

#[tokio::test]
async fn series_full_scan_batches_sonarr_identity_hints_at_gateway_cap() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let mut scan_hints = LibraryScanHintSet::new();

    for index in 0..22 {
        let folder = series_root.join(format!("Arr Show {index:02} (1999)"));
        std::fs::create_dir_all(&folder).expect("create hinted show folder");
        let folder_path = folder.to_string_lossy().to_string();
        scan_hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportSonarr,
            facet: LibraryScanHintFacet::Series,
            path_key: crate::library_scan_folder_leaf_key(&folder_path)
                .expect("folder leaf path key"),
            full_path_key: crate::library_scan_folder_full_path_key(&folder_path),
            ids: vec![
                ExternalIdHint::normalized(
                    ExternalIdProvider::Tvdb,
                    &(900_000 + index).to_string(),
                )
                .expect("normalized tvdb id"),
            ],
        });
    }

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&series_root, true)]),
    )
    .await
    .expect("store series root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            Some(scan_hints),
        )
        .await
        .expect("trigger hinted series scan");

    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        projected.summary.as_ref().map(|summary| summary.matched),
        Some(22)
    );
    assert_eq!(metadata_gateway.detail_calls(), 0);

    let batches = metadata_gateway.batch_queries().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 22);
    for query in &batches[0] {
        assert_eq!(query.query, "");
        assert_eq!(query.type_hint, "series");
        assert_eq!(query.year, None);
        assert!(query.tvdb_id.is_some());
        assert_eq!(query.imdb_id, None);
        assert_eq!(query.tmdb_id, None);
    }
}

#[tokio::test]
async fn series_full_scan_keeps_hinted_batch_intact_across_unhinted_folder() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let mut scan_hints = LibraryScanHintSet::new();

    for index in 0..22 {
        let folder_name = if index < 11 {
            format!("A Hinted Show {index:02} (1999)")
        } else {
            format!("Z Hinted Show {index:02} (1999)")
        };
        let folder = series_root.join(folder_name);
        std::fs::create_dir_all(&folder).expect("create hinted show folder");
        let folder_path = folder.to_string_lossy().to_string();
        scan_hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportSonarr,
            facet: LibraryScanHintFacet::Series,
            path_key: crate::library_scan_folder_leaf_key(&folder_path)
                .expect("folder leaf path key"),
            full_path_key: crate::library_scan_folder_full_path_key(&folder_path),
            ids: vec![
                ExternalIdHint::normalized(
                    ExternalIdProvider::Tvdb,
                    &(910_000 + index).to_string(),
                )
                .expect("normalized tvdb id"),
            ],
        });
    }
    std::fs::create_dir_all(series_root.join("M Unhinted Show (1999)"))
        .expect("create unhinted show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&series_root, true)]),
    )
    .await
    .expect("store series root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            Some(scan_hints),
        )
        .await
        .expect("trigger mixed hinted series scan");

    wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
        matches!(
            session.status,
            LibraryScanStatus::Completed | LibraryScanStatus::Warning
        )
    })
    .await;

    // Hinted and unhinted candidates share one match queue: the exact-ID
    // lookups and the fuzzy lookup travel in the same searchTvdbBatch call
    // instead of separate hinted-only batches.
    let batches = metadata_gateway.batch_queries().await;
    assert_eq!(batches.len(), 1);
    let exact_queries = batches[0]
        .iter()
        .filter(|query| {
            query.query.is_empty()
                && query.type_hint == "series"
                && query.tvdb_id.is_some()
                && query.imdb_id.is_none()
                && query.tmdb_id.is_none()
        })
        .count();
    assert_eq!(exact_queries, 22);
    assert!(
        batches[0]
            .iter()
            .any(|query| !query.query.trim().is_empty() && query.tvdb_id.is_none())
    );
}

#[tokio::test]
async fn movie_full_scan_batches_radarr_identity_hints_at_gateway_cap() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let mut scan_hints = LibraryScanHintSet::new();
    let mut library_files = Vec::new();

    for index in 0..22 {
        let folder = movie_root.join(format!("Arr Movie {index:02} (1999)"));
        std::fs::create_dir_all(&folder).expect("create hinted movie folder");
        let movie_file = folder.join(format!("Arr Movie {index:02}.mkv"));
        std::fs::write(&movie_file, b"movie").expect("write hinted movie file");
        let file_path = movie_file.to_string_lossy().to_string();
        library_files.push(build_test_library_file(&file_path));
        scan_hints.push(LibraryScanHint {
            source: LibraryScanHintSource::ExternalImportRadarr,
            facet: LibraryScanHintFacet::Movie,
            path_key: crate::library_scan_file_leaf_key(&file_path).expect("file leaf path key"),
            full_path_key: crate::library_scan_file_full_path_key(&file_path),
            ids: vec![
                ExternalIdHint::normalized(
                    ExternalIdProvider::Tmdb,
                    &(800_000 + index).to_string(),
                )
                .expect("normalized tmdb id"),
            ],
        });
    }

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner.set_library_files(library_files).await;
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            Some(scan_hints),
        )
        .await
        .expect("trigger hinted movie scan");

    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        projected.summary.as_ref().map(|summary| summary.matched),
        Some(22)
    );
    assert_eq!(metadata_gateway.detail_calls(), 0);

    // Movie discovery yields folders, so each candidate runs async NFO/dir
    // probing before it lands in the pending match queue. Under full-suite
    // load the arrival spread of the 22 candidates can exceed the 50ms
    // wall-clock flush interval (LIBRARY_SCAN_MATCH_FLUSH_INTERVAL), so the
    // match batcher may flush them across more than one gateway call. That
    // split is a scheduling artifact, not a batching regression -- every
    // candidate still fires its id-anchored exact lookup. Assert on the
    // flattened union of all batches instead of requiring a single coalesced
    // call, which is what made this test flaky.
    let batches = metadata_gateway.batch_queries().await;
    let queries: Vec<_> = batches.iter().flatten().collect();
    assert_eq!(queries.len(), 22);
    for query in &queries {
        assert_eq!(query.query, "");
        assert_eq!(query.type_hint, "movie");
        assert_eq!(query.year, None);
        assert!(query.tmdb_id.is_some());
        assert_eq!(query.imdb_id, None);
        assert_eq!(query.tvdb_id, None);
    }
    // Each candidate carries a distinct tmdb id, so the union must contain
    // exactly the 22 expected anchored lookups with none dropped or duplicated
    // across the split.
    let distinct_tmdb_ids: std::collections::HashSet<_> = queries
        .iter()
        .filter_map(|query| query.tmdb_id.clone())
        .collect();
    assert_eq!(distinct_tmdb_ids.len(), 22);

    let title_batches = metadata_gateway.title_batch_queries().await;
    assert!(!title_batches.is_empty());
    assert!(
        title_batches
            .iter()
            .all(|(_, kind, create_missing)| { kind == "movie" && *create_missing })
    );
    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 22);
    assert!(titles.iter().all(|title| {
        title
            .external_ids
            .iter()
            .any(|id| id.source.eq_ignore_ascii_case("tvdb") && !id.value.trim().is_empty())
    }));
}

#[tokio::test]
async fn movie_full_scan_creates_a_tmdb_primary_title_from_a_radarr_hint() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let movie_folder = movie_root.join("TMDB Primary Match (2020)");
    std::fs::create_dir_all(&movie_folder).expect("create movie folder");
    let movie_file = movie_folder.join("TMDB.Primary.Match.2020.mkv");
    std::fs::write(&movie_file, b"movie").expect("write movie file");
    let movie_path = movie_file.to_string_lossy().to_string();

    let mut scan_hints = LibraryScanHintSet::new();
    scan_hints.push(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportRadarr,
        facet: LibraryScanHintFacet::Movie,
        path_key: crate::library_scan_file_leaf_key(&movie_path).expect("file leaf path key"),
        full_path_key: crate::library_scan_file_full_path_key(&movie_path),
        ids: vec![
            ExternalIdHint::normalized(ExternalIdProvider::Tmdb, "7777")
                .expect("normalized tmdb id"),
        ],
    });

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::with_title_id_movies());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(&movie_path)])
        .await;
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            Some(scan_hints),
        )
        .await
        .expect("trigger hinted movie scan");
    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        projected.summary.as_ref().map(|summary| summary.matched),
        Some(1)
    );

    let title_batches = metadata_gateway.title_batch_queries().await;
    assert_eq!(title_batches.len(), 1);
    assert_eq!(title_batches[0].1, "movie");
    assert!(
        title_batches[0].2,
        "external-id matches create missing titles"
    );
    assert_eq!(title_batches[0].0.len(), 1);
    assert_eq!(title_batches[0].0[0].query, "");
    assert_eq!(title_batches[0].0[0].tmdb_id.as_deref(), Some("7777"));
    assert!(metadata_gateway.batch_queries().await.is_empty());

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 1);
    assert!(titles[0].metadata_fetched_at.is_none());
    assert!(
        titles[0]
            .external_ids
            .iter()
            .any(|id| { id.source.eq_ignore_ascii_case("smg") && id.value == "7777" })
    );
    assert!(
        titles[0]
            .external_ids
            .iter()
            .any(|id| { id.source.eq_ignore_ascii_case("tmdb") && id.value == "7777" })
    );
    assert!(
        titles[0]
            .external_ids
            .iter()
            .any(|id| { id.source.eq_ignore_ascii_case("imdb") && id.value == "tt0077777" })
    );
    assert!(
        !titles[0]
            .external_ids
            .iter()
            .any(|id| id.source.eq_ignore_ascii_case("tvdb")),
        "TMDB-primary matches must not synthesize a TVDB id"
    );
}

/// The batched search returns SMG's full identity set for every facet, and a
/// scan-created series keeps all of it -- indexer search subjects, RSS candidate
/// indexes, notification payloads and the `externalIds` readback all read these
/// ids without checking the facet, so throwing the non-TVDB ids away left a
/// series unidentifiable everywhere downstream.
#[tokio::test]
async fn series_full_scan_keeps_every_identity_from_a_rich_gateway_match() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let folder = series_root.join("Rich Identity Show (1999)");
    std::fs::create_dir_all(&folder).expect("create hinted show folder");
    let folder_path = folder.to_string_lossy().to_string();

    let mut scan_hints = LibraryScanHintSet::new();
    scan_hints.push(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportSonarr,
        facet: LibraryScanHintFacet::Series,
        path_key: crate::library_scan_folder_leaf_key(&folder_path).expect("folder leaf path key"),
        full_path_key: crate::library_scan_folder_full_path_key(&folder_path),
        ids: vec![
            ExternalIdHint::normalized(ExternalIdProvider::Tvdb, "900001")
                .expect("normalized tvdb id"),
        ],
    });

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway =
        Arc::new(RecordingExactIdMetadataGateway::default().with_rich_external_ids());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&series_root, true)]),
    )
    .await
    .expect("store series root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            Some(scan_hints),
        )
        .await
        .expect("trigger hinted series scan");
    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        projected.summary.as_ref().map(|summary| summary.matched),
        Some(1)
    );

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Series), None, None)
        .await
        .expect("list series titles");
    assert_eq!(titles.len(), 1);
    let mut external_ids = titles[0]
        .external_ids
        .iter()
        .map(|id| (id.source.to_ascii_lowercase(), id.value.clone()))
        .collect::<Vec<_>>();
    external_ids.sort();
    assert_eq!(
        external_ids,
        vec![
            ("imdb".to_string(), "tt0055555".to_string()),
            ("smg".to_string(), "5555".to_string()),
            ("tmdb".to_string(), "6666".to_string()),
            ("tvdb".to_string(), "900001".to_string()),
        ],
        "a scan-created series keeps every identity the gateway returned"
    );
}

/// The movie side of the same match keeps every identity SMG returned.
#[tokio::test]
async fn movie_full_scan_keeps_every_identity_from_a_rich_gateway_match() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let movie_folder = movie_root.join("TMDB Primary Match (2020)");
    std::fs::create_dir_all(&movie_folder).expect("create movie folder");
    let movie_file = movie_folder.join("TMDB.Primary.Match.2020.mkv");
    std::fs::write(&movie_file, b"movie").expect("write movie file");
    let movie_path = movie_file.to_string_lossy().to_string();

    let mut scan_hints = LibraryScanHintSet::new();
    scan_hints.push(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportRadarr,
        facet: LibraryScanHintFacet::Movie,
        path_key: crate::library_scan_file_leaf_key(&movie_path).expect("file leaf path key"),
        full_path_key: crate::library_scan_file_full_path_key(&movie_path),
        ids: vec![
            ExternalIdHint::normalized(ExternalIdProvider::Tmdb, "7777")
                .expect("normalized tmdb id"),
        ],
    });

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway =
        Arc::new(RecordingExactIdMetadataGateway::with_title_id_movies().with_rich_external_ids());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(&movie_path)])
        .await;
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            Some(scan_hints),
        )
        .await
        .expect("trigger hinted movie scan");
    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        projected.summary.as_ref().map(|summary| summary.matched),
        Some(1)
    );

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 1);
    let mut external_ids = titles[0]
        .external_ids
        .iter()
        .map(|id| (id.source.to_ascii_lowercase(), id.value.clone()))
        .collect::<Vec<_>>();
    external_ids.sort();
    assert_eq!(
        external_ids,
        vec![
            ("imdb".to_string(), "tt0077777".to_string()),
            ("smg".to_string(), "7777".to_string()),
            ("tmdb".to_string(), "7777".to_string()),
            ("tvdb".to_string(), "444444".to_string()),
        ],
        "a scan-created movie keeps every identity the gateway returned"
    );
}

/// An SMG old enough to lack `searchTitlesBatch` rejects it with a raw GraphQL
/// validation error naming that field. The scan must read that as a capability
/// signal and fall back to the legacy batched search -- not fail the whole
/// batch and leave the library unmatched.
#[tokio::test]
async fn movie_full_scan_falls_back_on_a_raw_unknown_field_error() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let movie_folder = movie_root.join("Legacy Fallback Movie (1999)");
    std::fs::create_dir_all(&movie_folder).expect("create movie folder");
    let movie_file = movie_folder.join("Legacy.Fallback.Movie.1999.mkv");
    std::fs::write(&movie_file, b"movie").expect("write movie file");
    let movie_path = movie_file.to_string_lossy().to_string();

    let mut scan_hints = LibraryScanHintSet::new();
    scan_hints.push(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportRadarr,
        facet: LibraryScanHintFacet::Movie,
        path_key: crate::library_scan_file_leaf_key(&movie_path).expect("file leaf path key"),
        full_path_key: crate::library_scan_file_full_path_key(&movie_path),
        ids: vec![
            ExternalIdHint::normalized(ExternalIdProvider::Tmdb, "800000")
                .expect("normalized tmdb id"),
        ],
    });

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway =
        Arc::new(RecordingExactIdMetadataGateway::default().with_raw_unknown_field_error());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(&movie_path)])
        .await;
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id_with_hints(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            Some(scan_hints),
        )
        .await
        .expect("trigger hinted movie scan");
    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        projected.summary.as_ref().map(|summary| summary.matched),
        Some(1)
    );

    assert!(
        !metadata_gateway.title_batch_queries().await.is_empty(),
        "the scan tries the title-id surface first"
    );
    let legacy_queries = metadata_gateway.batch_queries().await;
    assert_eq!(legacy_queries.iter().flatten().count(), 1);
    assert_eq!(
        legacy_queries[0][0].tmdb_id.as_deref(),
        Some("800000"),
        "the raw validation error falls back to the legacy batched search"
    );

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list movie titles");
    assert_eq!(titles.len(), 1);
    assert!(
        titles[0]
            .external_ids
            .iter()
            .any(|id| id.source.eq_ignore_ascii_case("tvdb") && id.value == "800000")
    );
}

#[tokio::test]
async fn movie_full_scan_batches_unhinted_fuzzy_candidates_at_gateway_cap() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let mut library_files = Vec::new();
    let titles = [
        "Amber Harbor",
        "Blue Orchard",
        "Copper Lantern",
        "Distant Meadow",
        "Emerald Signal",
        "Frosted Avenue",
        "Golden Trellis",
        "Hidden Valley",
        "Ivory Station",
        "Jade Horizon",
        "Kindred Bridge",
        "Lunar Garden",
        "Marble Echo",
        "Northern Arcade",
        "Opal Junction",
        "Pacific Ember",
        "Quiet Meridian",
        "River Anthem",
        "Silver Plateau",
        "Timber Bloom",
        "Umber Skyline",
        "Velvet Harbor",
    ];

    for title in titles {
        let folder = movie_root.join(format!("Fuzzy Movie {title} (1999)"));
        std::fs::create_dir_all(&folder).expect("create fuzzy movie folder");
        let movie_file = folder.join(format!("Fuzzy Movie {title}.mkv"));
        std::fs::write(&movie_file, b"movie").expect("write fuzzy movie file");
        library_files.push(build_test_library_file(
            movie_file.to_string_lossy().as_ref(),
        ));
    }

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner.set_library_files(library_files).await;
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )
        .await
        .expect("trigger unhinted movie scan");

    wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
        matches!(
            session.status,
            LibraryScanStatus::Completed | LibraryScanStatus::Warning
        )
    })
    .await;

    // Same flush-timer split as the hinted sibling above: movie folders run
    // async NFO/dir probing before entering the pending match queue, so under
    // load the 22 fuzzy candidates can be flushed across more than one gateway
    // call. Assert on the flattened union rather than a single coalesced call.
    let batches = metadata_gateway.batch_queries().await;
    let queries: Vec<_> = batches.iter().flatten().collect();
    assert_eq!(queries.len(), 22);
    assert!(queries.iter().all(|query| {
        !query.query.trim().is_empty()
            && query.type_hint == "movie"
            && query.imdb_id.is_none()
            && query.tmdb_id.is_none()
            && query.tvdb_id.is_none()
    }));
    // Each fuzzy candidate comes from a distinctly named folder, so the union
    // must hold exactly 22 distinct title queries with none lost across the
    // split.
    let distinct_titles: std::collections::HashSet<_> =
        queries.iter().map(|query| query.query.trim()).collect();
    assert_eq!(distinct_titles.len(), 22);

    let title_batches = metadata_gateway.title_batch_queries().await;
    assert!(!title_batches.is_empty());
    assert!(
        title_batches
            .iter()
            .all(|(_, kind, create_missing)| { kind == "movie" && !create_missing })
    );
}

#[tokio::test]
async fn movie_full_scan_marks_title_match_total_known_before_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    std::fs::create_dir_all(&movie_root).expect("create movie root");
    let movie_folder = movie_root.join("Unknown One (2020)");
    std::fs::create_dir(&movie_folder).expect("create movie folder");
    let movie_path = movie_folder.join("Unknown.One.2020.mkv");
    std::fs::write(&movie_path, b"movie").expect("seed movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            movie_root.to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let session_id = "movie-title-match-known-before-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Movie,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1 && session.title_match_total_known
    })
    .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert_eq!(projected.title_match_progress.completed, 0);
    assert!(projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join movie full scan task")
        .expect("movie full scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn movie_full_scan_completes_title_match_while_inventory_walk_is_blocked() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let movie_folder = movie_root.join("Blocked Movie (2024)");
    std::fs::create_dir_all(&movie_folder).expect("create movie folder");
    let movie_file = movie_folder.join("Blocked.Movie.2024.mkv");
    std::fs::write(&movie_file, b"movie").expect("write movie file");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    scanner
        .set_directory_files(
            &movie_folder,
            vec![build_test_library_file(
                movie_file.to_string_lossy().as_ref(),
            )],
        )
        .await;
    scanner.block_directory(&movie_folder).await;

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "445566".to_string(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Blocked Movie".to_string(),
                year: Some(2024),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        }),
    );
    let app = app.with_test_overrides(|builder| builder.with_library_scanner(scanner.clone()));
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )
        .await
        .expect("trigger blocked-inventory movie scan");

    // The candidate's recursive inventory walk is held; title evidence and
    // SMG matching must complete anyway.
    scanner.wait_for_blocked_directory_scan().await;
    let projected =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            session.title_match_total_known && session.title_match_progress.completed >= 1
        })
        .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert!(
        !projected.file_total_known,
        "media total must stay unknown while a matched candidate's inventory is pending"
    );
    assert!(projected.summary.is_none());

    scanner.release_blocked_directory_scans().await;

    let completed =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    let summary = completed.summary.expect("completed scan summary");
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.unmatched, 0);
}

#[tokio::test]
async fn movie_full_scan_marks_exact_media_total_before_blocked_analysis_finishes() {
    const ITEM_COUNT: usize = 90;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let scan_root = tempdir.path().join("scan-root");
    std::fs::create_dir_all(&scan_root).expect("create scan root");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    let (base_app, user, _) =
        bootstrap_movie_scan_app(&scan_root, Vec::new(), Arc::new(EmptySearchMetadataGateway))
            .await;
    let blocking_analyzer = Arc::new(BlockingMediaAnalyzer::default());
    blocking_analyzer.block();
    let app = base_app.with_test_overrides(|builder| {
        builder
            .with_library_scanner(scanner.clone())
            .with_media_analyzer(blocking_analyzer.clone())
    });

    for index in 0..ITEM_COUNT {
        let item_label = format!("scan-item-{index:03}");
        let item_folder = scan_root.join(&item_label);
        std::fs::create_dir_all(&item_folder).expect("create scan item folder");
        let item_file = item_folder.join(format!("{item_label}.mkv"));
        std::fs::write(&item_file, b"scan").expect("write scan item file");
        scanner
            .set_directory_files(
                &item_folder,
                vec![build_test_library_file(
                    item_file.to_string_lossy().as_ref(),
                )],
            )
            .await;
        create_movie_title_with_folder(&app, &user, &item_label, item_folder.as_path()).await;
    }

    let session_id = "scan-exact-total-before-analysis-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Movie,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    blocking_analyzer
        .wait_for_active_analysis(crate::GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY)
        .await;
    assert_eq!(
        blocking_analyzer.max_active_calls(),
        crate::GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY
    );
    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.file_total_known && session.file_progress.total == ITEM_COUNT
    })
    .await;
    assert_eq!(projected.file_progress.total, ITEM_COUNT);
    assert_eq!(projected.file_progress.completed, 0);
    assert_eq!(projected.file_progress.failed, 0);
    assert!(projected.title_match_total_known);
    assert!(!projected.status.is_terminal());

    blocking_analyzer.release();
    let summary = handle
        .await
        .expect("join movie full scan task")
        .expect("movie full scan should complete");
    assert_eq!(summary.scanned, ITEM_COUNT);
}

#[tokio::test]
async fn movie_full_scan_streams_completed_final_hydration_chunks_into_analysis() {
    const ITEM_COUNT: usize = 25;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let scan_root = tempdir.path().join("scan-root");
    std::fs::create_dir_all(&scan_root).expect("create scan root");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    let metadata_gateway = Arc::new(BlockingBulkHydrationMetadataGateway::default());
    let (base_app, user, _) =
        bootstrap_movie_scan_app(&scan_root, Vec::new(), metadata_gateway.clone()).await;
    let blocking_analyzer = Arc::new(BlockingMediaAnalyzer::default());
    blocking_analyzer.block();
    let app = base_app.with_test_overrides(|builder| {
        builder
            .with_library_scanner(scanner.clone())
            .with_media_analyzer(blocking_analyzer.clone())
    });

    for index in 0..ITEM_COUNT {
        let item_label = format!("scan-item-{index:03}");
        let item_folder = scan_root.join(&item_label);
        std::fs::create_dir_all(&item_folder).expect("create scan item folder");
        let item_file = item_folder.join(format!("{item_label}.mkv"));
        std::fs::write(&item_file, b"scan").expect("write scan item file");
        scanner
            .set_directory_files(
                &item_folder,
                vec![build_test_library_file(
                    item_file.to_string_lossy().as_ref(),
                )],
            )
            .await;

        let mut title = make_due_hydration_title(
            &format!("scan-title-{index:03}"),
            MediaFacet::Movie,
            90_000 + index as i64,
        );
        title.name = item_label;
        title.folder_path = Some(item_folder.to_string_lossy().to_string());
        let title = app
            .services
            .catalog
            .titles
            .create(title)
            .await
            .expect("seed due-hydration movie title");
        app.services
            .catalog
            .titles
            .set_folder_path(&title.id, item_folder.to_string_lossy().as_ref())
            .await
            .expect("set movie folder path");
    }

    let session_id = "scan-final-hydration-streams-to-analysis";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Movie,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_bulk_calls(2).await;
    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.file_total_known && session.file_progress.total == ITEM_COUNT
    })
    .await;
    assert_eq!(projected.file_progress.completed, 0);
    assert_eq!(projected.file_progress.failed, 0);
    assert!(!projected.status.is_terminal());

    metadata_gateway.release_through(1);
    blocking_analyzer.wait_for_analysis().await;
    assert!(
        blocking_analyzer.max_active_calls() > 0,
        "first completed hydration chunk should stream into media analysis before later chunks finish"
    );

    metadata_gateway.release_all();
    blocking_analyzer.release();
    let summary = handle
        .await
        .expect("join movie full scan task")
        .expect("movie full scan should complete");
    assert_eq!(summary.scanned, ITEM_COUNT);
    assert_eq!(summary.matched, ITEM_COUNT);
}

#[tokio::test]
async fn movie_full_scan_of_empty_library_completes_with_deterministic_zero_totals() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    std::fs::create_dir_all(&movie_root).expect("create empty movie root");

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(RecordingExactIdMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&movie_root, true)]),
    )
    .await
    .expect("store movie root");

    let session = app
        .trigger_library_scan_by_id(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )
        .await
        .expect("trigger empty movie scan");

    let completed =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;

    assert!(completed.title_match_total_known);
    assert_eq!(completed.title_match_progress.total, 0);
    assert_eq!(completed.file_progress.total, 0);
    let summary = completed.summary.expect("empty scan summary");
    assert_eq!(summary.scanned, 0);
    assert!(
        metadata_gateway.batch_queries().await.is_empty(),
        "an empty library must not issue SMG lookups"
    );
}

#[tokio::test]
async fn movie_full_scan_by_id_uses_selected_library_scope() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let default_root = tempdir.path().join("default-movies");
    let selected_root = tempdir.path().join("selected-movies");
    let default_title_dir = default_root.join("Default Movie (2025)");
    let selected_title_dir = selected_root.join("Selected Movie (2026)");
    std::fs::create_dir_all(&default_title_dir).expect("create default movie folder");
    std::fs::create_dir_all(&selected_title_dir).expect("create selected movie folder");
    let default_file = default_title_dir.join("Default.Movie.2025.mkv");
    let selected_file = selected_title_dir.join("Selected.Movie.2026.mkv");
    std::fs::write(&default_file, b"default movie").expect("write default movie");
    std::fs::write(&selected_file, b"selected movie").expect("write selected movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            default_root.to_string_lossy().as_ref(),
        )
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (base_app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items,
        Arc::new(HydratingMovieSearchGateway {
            search_item: MetadataSearchItem {
                tvdb_id: "778899".to_string(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "Selected Movie".to_string(),
                year: Some(2026),
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into()],
            },
            movie: make_movie_metadata(778899, "Selected Movie"),
        }),
    );
    base_app
        .reconcile_default_library_roots()
        .await
        .expect("reconcile default movie root");
    let selected_library = base_app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Selected Movies".to_string(),
            vec![LibraryRootDraft {
                path: selected_root.to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create selected movie library");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    scanner
        .set_directory_files(
            &default_title_dir,
            vec![build_test_library_file(
                default_file.to_string_lossy().as_ref(),
            )],
        )
        .await;
    scanner
        .set_directory_files(
            &selected_title_dir,
            vec![build_test_library_file(
                selected_file.to_string_lossy().as_ref(),
            )],
        )
        .await;
    let app = base_app.with_test_overrides(|builder| {
        builder
            .with_library_scanner(scanner.clone())
            .with_media_analyzer(Arc::new(CountingValidMediaAnalyzer::default()))
    });

    let session = app
        .trigger_library_scan_by_id(&user, &selected_library.id)
        .await
        .expect("trigger selected movie library scan");
    assert_eq!(
        session.library_id.as_deref(),
        Some(selected_library.id.as_str())
    );
    let completed =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;

    assert_eq!(
        scanner.scanned_directories().await,
        vec![selected_title_dir.to_string_lossy().to_string()]
    );
    assert_eq!(
        completed.library_id.as_deref(),
        Some(selected_library.id.as_str())
    );
    assert_eq!(completed.found_titles, 1);
    assert_eq!(completed.title_match_progress.total, 1);
    assert_eq!(completed.file_progress.total, 1);
    let summary = completed.summary.expect("selected library scan summary");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);

    let stored_titles = titles.store.lock().await.clone();
    assert_eq!(stored_titles.len(), 1);
    let selected_title = &stored_titles[0];
    assert_eq!(selected_title.name, "Selected Movie");
    assert_eq!(selected_title.library_id, selected_library.id);
    assert_eq!(selected_title.root_folder_id, selected_library.roots[0].id);
    assert_eq!(
        selected_title.folder_path.as_deref(),
        Some(selected_title_dir.to_string_lossy().as_ref())
    );
    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&selected_title.id)
        .await
        .expect("list selected movie media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(media_files[0].file_path, selected_file.to_string_lossy());
    assert!(stored_titles.iter().all(|title| {
        title.library_id != scryer_domain::default_library_id_for_facet(&MediaFacet::Movie)
    }));
}

#[tokio::test]
async fn movie_full_scan_adopts_empty_folder_match_with_zero_files() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let empty_folder = movie_root.join("scan-item-empty");
    std::fs::create_dir_all(&empty_folder).expect("create empty movie folder");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    scanner.set_directory_files(&empty_folder, Vec::new()).await;
    let (base_app, user, _) = bootstrap_movie_scan_app(
        &movie_root,
        Vec::new(),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "667788".to_string(),
                smg_id: None,
                primary_source: None,
                external_ids: vec![],
                name: "scan-item-empty".to_string(),
                year: None,
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into()],
            }],
        }),
    )
    .await;
    let app = base_app.with_test_overrides(|builder| builder.with_library_scanner(scanner.clone()));

    let session = app
        .trigger_library_scan_by_id(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )
        .await
        .expect("trigger movie scan with empty folder");
    let completed =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    let summary = completed.summary.expect("scan summary");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);
    assert!(completed.file_total_known);
    assert_eq!(completed.file_progress.total, 0);
    assert_eq!(completed.file_progress.completed, 0);
    assert_eq!(completed.file_progress.failed, 0);

    let pending = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("pending imports");
    assert_eq!(pending.total, 0);
    let ignored = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored imports");
    assert_eq!(ignored.total, 0);

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts");
    assert_eq!(counts.movie, 0);

    let movie_file = empty_folder.join("scan-item-empty.mkv");
    std::fs::write(&movie_file, b"movie").expect("write movie file");
    scanner
        .set_directory_files(
            &empty_folder,
            vec![build_test_library_file(
                movie_file.to_string_lossy().as_ref(),
            )],
        )
        .await;
    let second_session = app
        .trigger_library_scan_by_id(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )
        .await
        .expect("trigger movie scan after adding media file");
    let second_completed = wait_for_projected_library_scan_session_matching(
        &app,
        &second_session.session_id,
        |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        },
    )
    .await;
    assert_eq!(
        second_completed
            .summary
            .as_ref()
            .map(|summary| summary.matched),
        Some(1)
    );
    let ignored_after_success = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored imports after successful match");
    assert_eq!(ignored_after_success.total, 0);
}

#[tokio::test]
async fn movie_full_scan_records_empty_folder_without_safe_match_as_pending_import() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    let empty_folder = movie_root.join("scan-item-unmatched (2024)");
    std::fs::create_dir_all(&empty_folder).expect("create empty movie folder");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    scanner.set_directory_files(&empty_folder, Vec::new()).await;
    let (base_app, user, _) = bootstrap_movie_scan_app(
        &movie_root,
        Vec::new(),
        Arc::new(EmptySearchMetadataGateway),
    )
    .await;
    let app = base_app.with_test_overrides(|builder| builder.with_library_scanner(scanner.clone()));

    let session = app
        .trigger_library_scan_by_id(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        )
        .await
        .expect("trigger empty unmatched movie scan");
    let completed =
        wait_for_projected_library_scan_session_matching(&app, &session.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    let summary = completed.summary.expect("scan summary");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 0);
    assert_eq!(summary.unmatched, 1);
    assert_eq!(summary.skipped, 0);
    assert!(completed.file_total_known);
    assert_eq!(completed.file_progress.total, 0);

    let pending = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("pending imports");
    assert_eq!(pending.total, 1);
    assert_eq!(
        pending.items[0].path,
        empty_folder.to_string_lossy().to_string()
    );
    assert_eq!(pending.items[0].status, PendingImportStatus::Pending);

    let ignored = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored imports");
    assert_eq!(ignored.total, 0);
}

#[tokio::test]
async fn title_scan_records_unreadable_file_skip_as_ignored_pending_import() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let title_folder = series_root.join("scan-title");
    std::fs::create_dir_all(&title_folder).expect("create title folder");
    let missing_file = title_folder.join("scan-title - S01E01.mkv");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "scan-title".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2026),
                ..NewTitle::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, title_folder.to_string_lossy().as_ref())
        .await
        .expect("set title folder");
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");

    let summary = app
        .scan_title_library_with_discovered_files(
            &user,
            title.clone(),
            vec![build_test_library_file(
                missing_file.to_string_lossy().as_ref(),
            )],
        )
        .await
        .expect("title scan should complete with skipped unreadable file");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.skipped, 1);

    let pending = app
        .pending_imports(
            &user,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("pending imports");
    assert_eq!(pending.total, 0);
    let ignored = app
        .pending_imports(
            &user,
            MediaFacet::Series,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored imports");
    assert_eq!(ignored.total, 1);
    assert_eq!(ignored.items[0].status, PendingImportStatus::Ignored);
    assert_eq!(
        ignored.items[0].title_id.as_deref(),
        Some(title.id.as_str())
    );
    assert_eq!(
        ignored.items[0].path,
        missing_file.to_string_lossy().to_string()
    );
    assert_eq!(
        ignored.items[0].reason,
        crate::library_scan_unmatched::LIBRARY_SCAN_SKIPPED_FILE_METADATA_UNREADABLE
    );
}

#[tokio::test]
async fn series_full_scan_marks_title_match_total_known_before_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let session_id = "series-title-match-known-before-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1 && session.title_match_total_known
    })
    .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert_eq!(projected.title_match_progress.completed, 0);
    assert!(projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join series full scan task")
        .expect("series full scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn series_full_scan_starts_immediate_title_walk_before_blocked_metadata_lookup_finishes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let known_folder = series_root.join("Known Show (2020)");
    let unknown_folder = series_root.join("Unknown Show (2021)");
    let known_season = known_folder.join("Season 01");
    std::fs::create_dir_all(&known_season).expect("create known show season");
    std::fs::create_dir_all(&unknown_folder).expect("create unknown show folder");
    let known_file = known_season.join("Known Show - S01E01.mkv");
    std::fs::write(&known_file, b"episode").expect("write known episode");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let notifying_scanner = Arc::new(NotifyingLibraryScanner::default());
    notifying_scanner
        .set_library_files(build_test_library_files(&[known_file.as_path()]))
        .await;
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    let app =
        app.with_test_overrides(|builder| builder.with_library_scanner(notifying_scanner.clone()));
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");
    app.add_title(
        &user,
        NewTitle {
            name: "Known Show".into(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            year: Some(2020),
            ..Default::default()
        },
    )
    .await
    .expect("create existing series title");

    let session_id = "series-immediate-walk-before-metadata";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;
    notifying_scanner.wait_for_directory_scan().await;
    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join series full scan task")
        .expect("series full scan should complete");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn series_full_scan_marks_media_total_known_after_enumeration_before_analysis_finishes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let known_folder = series_root.join("Known Show (2020)");
    let known_season = known_folder.join("Season 01");
    std::fs::create_dir_all(&known_season).expect("create known show season");
    let known_file = known_season.join("Known Show - S01E01.mkv");
    std::fs::write(&known_file, b"episode").expect("write known episode");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;
    let notifying_scanner = Arc::new(NotifyingLibraryScanner::default());
    notifying_scanner
        .set_library_files(build_test_library_files(&[known_file.as_path()]))
        .await;
    let blocking_analyzer = Arc::new(BlockingMediaAnalyzer::default());
    blocking_analyzer.block();

    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(EmptySearchMetadataGateway),
    );
    let app = app.with_test_overrides(|builder| {
        builder
            .with_library_scanner(notifying_scanner.clone())
            .with_media_analyzer(blocking_analyzer.clone())
    });
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Known Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                year: Some(2020),
                ..Default::default()
            },
        )
        .await
        .expect("create existing series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, known_folder.to_string_lossy().as_ref())
        .await
        .expect("set series folder path");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2020-01-01".to_string()),
            duration_seconds: Some(420),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    let session_id = "series-media-known-before-analysis-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    notifying_scanner.wait_for_directory_scan().await;
    blocking_analyzer.wait_for_analysis().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.file_total_known && session.file_progress.total == 1
    })
    .await;
    assert_eq!(projected.file_progress.total, 1);
    assert_eq!(projected.file_progress.completed, 0);
    assert_eq!(projected.file_progress.failed, 0);
    assert!(!projected.status.is_terminal());

    blocking_analyzer.release();

    let summary = handle
        .await
        .expect("join series full scan task")
        .expect("series full scan should complete");
    assert_eq!(summary.scanned, 1);
}

#[tokio::test]
async fn series_full_scan_starts_media_analysis_while_later_enumeration_is_pending() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let fast_folder = series_root.join("Fast Show (2020)");
    let fast_season = fast_folder.join("Season 01");
    let slow_folder = series_root.join("Slow Show (2020)");
    let slow_season = slow_folder.join("Season 01");
    std::fs::create_dir_all(&fast_season).expect("create fast show season");
    std::fs::create_dir_all(&slow_season).expect("create slow show season");
    let fast_file = fast_season.join("Fast Show - S01E01.mkv");
    let slow_file = slow_season.join("Slow Show - S01E01.mkv");
    std::fs::write(&fast_file, b"episode").expect("write fast episode");
    std::fs::write(&slow_file, b"episode").expect("write slow episode");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;
    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    scanner
        .set_library_files(build_test_library_files(&[
            fast_file.as_path(),
            slow_file.as_path(),
        ]))
        .await;
    scanner
        .set_directory_files(
            &fast_folder,
            build_test_library_files(&[fast_file.as_path()]),
        )
        .await;
    scanner
        .set_directory_files(
            &slow_folder,
            build_test_library_files(&[slow_file.as_path()]),
        )
        .await;
    scanner.block_directory(&slow_folder).await;
    let blocking_analyzer = Arc::new(BlockingMediaAnalyzer::default());
    blocking_analyzer.block();

    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(EmptySearchMetadataGateway),
    );
    let app = app.with_test_overrides(|builder| {
        builder
            .with_library_scanner(scanner.clone())
            .with_media_analyzer(blocking_analyzer.clone())
    });
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    for (name, folder) in [
        ("Fast Show", fast_folder.as_path()),
        ("Slow Show", slow_folder.as_path()),
    ] {
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: name.into(),
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    year: Some(2020),
                    ..Default::default()
                },
            )
            .await
            .expect("create existing series title");
        app.services
            .catalog
            .titles
            .set_folder_path(&title.id, folder.to_string_lossy().as_ref())
            .await
            .expect("set series folder path");
        let season = app
            .services
            .catalog
            .shows
            .create_collection(Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type: CollectionType::Season,
                collection_index: "1".to_string(),
                label: Some("Season 1".to_string()),
                ordered_path: None,
                narrative_order: Some("1".to_string()),
                first_episode_number: Some("1".to_string()),
                last_episode_number: Some("1".to_string()),
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create season");
        app.services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some("1".to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some("S01E01".to_string()),
                title: Some("Pilot".to_string()),
                air_date: Some("2020-01-01".to_string()),
                duration_seconds: Some(420),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");
    }

    let session_id = "series-analysis-before-enumeration-drain";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    scanner.wait_for_blocked_directory_scan().await;
    blocking_analyzer.wait_for_analysis().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        !session.file_total_known && session.file_progress.total == 1
    })
    .await;
    assert_eq!(projected.file_progress.total, 1);
    assert_eq!(projected.file_progress.completed, 0);
    assert!(!projected.status.is_terminal());

    scanner.release_blocked_directory_scans().await;
    blocking_analyzer.release();

    let summary = handle
        .await
        .expect("join series full scan task")
        .expect("series full scan should complete");
    assert_eq!(summary.scanned, 2);
}

#[tokio::test]
async fn multi_root_full_scan_waits_for_final_root_to_mark_title_match_total_known() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("series-a");
    let root_two = tempdir.path().join("series-b");
    std::fs::create_dir_all(root_one.join("Unknown Show One (2020)"))
        .expect("create first series folder");
    std::fs::create_dir_all(root_two.join("Unknown Show Two (2021)"))
        .expect("create second series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::blocking_calls(&[1, 2]));
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store series roots");

    let session_id = "series-multi-root-title-match-known";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search_calls(1).await;

    let first_root_projected =
        wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
            session.found_titles == 1
        })
        .await;
    assert!(!first_root_projected.title_match_total_known);

    metadata_gateway.release_through(1);
    metadata_gateway.wait_for_batch_search_calls(2).await;

    let final_root_projected =
        wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
            session.found_titles == 2 && session.title_match_total_known
        })
        .await;
    assert_eq!(final_root_projected.title_match_progress.total, 2);
    assert!(final_root_projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join multi-root full scan task")
        .expect("multi-root full scan should complete");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);
}

#[tokio::test]
async fn additive_scan_keeps_title_match_total_unknown_until_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        None,
        Some(vec![LibraryRootDraft {
            path: series_root.to_string_lossy().to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("store series library roots");

    let session_id = "series-additive-title-match-stays-unknown";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .background_library_refresh_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                session_id,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1
    })
    .await;
    assert!(!projected.title_match_total_known);

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join additive scan task")
        .expect("additive scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn movie_full_scan_skips_invalid_roots_and_finishes_warning() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let valid_root = tempdir.path().join("movies-valid");
    let invalid_root = tempdir.path().join("movies-missing");
    std::fs::create_dir_all(&valid_root).expect("create valid movie root");
    let movie_folder = valid_root.join("Unknown One (2020)");
    std::fs::create_dir(&movie_folder).expect("create movie folder");
    std::fs::write(movie_folder.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&valid_root, true),
            build_root_folder_entry(&invalid_root, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-invalid-root-warning";
    let summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Movie,
            Some(session_id.to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("movie full scan with invalid root");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert_eq!(summary.skipped, 1);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 1);
    assert_eq!(projected.status, LibraryScanStatus::Warning);
}

#[tokio::test]
async fn background_refresh_movies_scans_all_configured_roots() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create movie root one");
    std::fs::create_dir_all(&root_two).expect("create movie root two");
    let movie_one = root_one.join("Unknown One (2020)");
    let movie_two = root_two.join("Unknown Two (2021)");
    std::fs::create_dir(&movie_one).expect("create movie one folder");
    std::fs::create_dir(&movie_two).expect("create movie two folder");
    std::fs::write(movie_one.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie one");
    std::fs::write(movie_two.join("Unknown.Two.2021.mkv"), b"movie-two").expect("seed movie two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        None,
        Some(vec![
            LibraryRootDraft {
                path: root_one.to_string_lossy().to_string(),
                is_default: true,
            },
            LibraryRootDraft {
                path: root_two.to_string_lossy().to_string(),
                is_default: false,
            },
        ]),
        None,
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-multi-root-refresh";
    let summary = app
        .background_library_refresh_with_tracking(&user, MediaFacet::Movie, session_id)
        .await
        .expect("movie background refresh");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 2);
}

#[tokio::test]
async fn cancel_full_library_scan_with_in_flight_title_walk_drains_executor_permits() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    let known_folder = series_root.join("Known Show (2020)");
    let known_season = known_folder.join("Season 01");
    std::fs::create_dir_all(&known_season).expect("create known show season");
    let known_file = known_season.join("Known Show - S01E01.mkv");
    std::fs::write(&known_file, b"episode").expect("write known episode");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;
    let notifying_scanner = Arc::new(NotifyingLibraryScanner::default());
    notifying_scanner
        .set_library_files(build_test_library_files(&[known_file.as_path()]))
        .await;
    notifying_scanner.block_directory_scans();
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(EmptySearchMetadataGateway),
    );
    let app =
        app.with_test_overrides(|builder| builder.with_library_scanner(notifying_scanner.clone()));
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");
    app.add_title(
        &user,
        NewTitle {
            name: "Known Show".into(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            year: Some(2020),
            ..Default::default()
        },
    )
    .await
    .expect("create existing series title");

    let session_id = "cancel-in-flight-title-walk";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    notifying_scanner.wait_for_directory_scan().await;
    let cancel_result = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect("cancel full library scan");
    assert!(cancel_result.accepted);

    let summary = timeout(Duration::from_secs(5), handle)
        .await
        .expect("scan task should drain after cancellation")
        .expect("join canceled scan task")
        .expect("canceled scan task should not error");
    assert_eq!(summary.matched, 1);
    notifying_scanner.release_directory_scans();

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected canceled session")
            .expect("canceled session snapshot");
    assert_eq!(projected.status, LibraryScanStatus::Canceled);
    assert_eq!(
        app.runtime
            .library
            .library_scan_title_walk_limit
            .available_permits(),
        LIBRARY_SCAN_GLOBAL_TITLE_WALK_CONCURRENCY
    );
    assert_eq!(
        app.runtime
            .library
            .library_scan_analysis_limit
            .available_permits(),
        GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY
    );
}

#[tokio::test]
async fn distinct_movie_libraries_scan_concurrently_and_cancel_independently() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let default_root = tempdir.path().join("default-movies");
    let first_root = tempdir.path().join("first-movies");
    let second_root = tempdir.path().join("second-movies");
    let first_title_dir = first_root.join("First Unknown Movie (2025)");
    let second_title_dir = second_root.join("Second Unknown Movie (2026)");
    std::fs::create_dir_all(&default_root).expect("create default movie root");
    std::fs::create_dir_all(&first_title_dir).expect("create first movie folder");
    std::fs::create_dir_all(&second_title_dir).expect("create second movie folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            default_root.to_string_lossy().as_ref(),
        )
        .await;
    let (base_app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        Arc::new(EmptySearchMetadataGateway),
    );
    base_app
        .reconcile_default_library_roots()
        .await
        .expect("reconcile default movie root");
    let first_library = base_app
        .create_library(
            &user,
            MediaFacet::Movie,
            "First Movies".to_string(),
            vec![LibraryRootDraft {
                path: first_root.to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create first movie library");
    let second_library = base_app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Second Movies".to_string(),
            vec![LibraryRootDraft {
                path: second_root.to_string_lossy().to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create second movie library");

    let scanner = Arc::new(PerDirectoryBlockingLibraryScanner::default());
    scanner
        .set_directory_files(&first_title_dir, Vec::new())
        .await;
    scanner
        .set_directory_files(&second_title_dir, Vec::new())
        .await;
    scanner.block_directory(&first_title_dir).await;
    scanner.block_directory(&second_title_dir).await;
    let app = base_app.with_test_overrides(|builder| builder.with_library_scanner(scanner.clone()));

    let first = app
        .trigger_library_scan_by_id(&user, &first_library.id)
        .await
        .expect("start first movie library scan");
    let second = app
        .trigger_library_scan_by_id(&user, &second_library.id)
        .await
        .expect("start second movie library scan");
    scanner.wait_for_blocked_directory_scan_count(2).await;

    let mut active_library_ids = app
        .active_library_scan_sessions()
        .await
        .into_iter()
        .filter_map(|session| session.library_id)
        .collect::<Vec<_>>();
    active_library_ids.sort();
    let mut expected_library_ids = vec![first_library.id.clone(), second_library.id.clone()];
    expected_library_ids.sort();
    assert_eq!(active_library_ids, expected_library_ids);
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    assert!(
        !app.runtime
            .library
            .library_scan_tracker
            .has_conflicting_session(&MediaFacet::Movie, Some(&default_library_id))
            .await,
        "custom library scans must not block the default-library job scope"
    );
    app.trigger_library_scan_by_id(&user, &first_library.id)
        .await
        .expect_err("duplicate scan for first library should be rejected");

    app.cancel_library_scan(&user, &first.session_id)
        .await
        .expect("request first scan cancellation");
    scanner
        .release_blocked_directory_scan(&first_title_dir)
        .await;
    let canceled =
        wait_for_projected_library_scan_session_matching(&app, &first.session_id, |session| {
            session.status == LibraryScanStatus::Canceled
        })
        .await;
    assert_eq!(
        canceled.library_id.as_deref(),
        Some(first_library.id.as_str())
    );

    let active = app.active_library_scan_sessions().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, second.session_id);
    let restarted = app
        .trigger_library_scan_by_id(&user, &first_library.id)
        .await
        .expect("restart first library while second remains active");
    let restarted =
        wait_for_projected_library_scan_session_matching(&app, &restarted.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        restarted.library_id.as_deref(),
        Some(first_library.id.as_str())
    );

    scanner
        .release_blocked_directory_scan(&second_title_dir)
        .await;
    let second =
        wait_for_projected_library_scan_session_matching(&app, &second.session_id, |session| {
            matches!(
                session.status,
                LibraryScanStatus::Completed | LibraryScanStatus::Warning
            )
        })
        .await;
    assert_eq!(
        second.library_id.as_deref(),
        Some(second_library.id.as_str())
    );
}

#[tokio::test]
async fn cancel_full_library_scan_marks_session_canceled_and_allows_restart() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

    let session_id = "cancel-full-library-scan";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let cancel_result = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect("cancel full library scan");
    assert!(cancel_result.accepted);
    assert_eq!(cancel_result.session_id, session_id);

    metadata_gateway.release();

    handle
        .await
        .expect("join canceled scan task")
        .expect("canceled scan task should not error");

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected canceled session")
            .expect("canceled session snapshot");
    assert_eq!(projected.status, LibraryScanStatus::Canceled);
    assert_eq!(projected.found_titles, 1);
    assert!(
        app.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .get(session_id)
            .is_none(),
        "cancellation token should be cleared after terminal cancel",
    );

    let retry_summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Series,
            Some("cancel-full-library-scan-retry".to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("retry full scan after cancel");
    assert_eq!(retry_summary.unmatched, 1);
}

#[tokio::test]
async fn cancel_library_scan_rejects_additive_sessions() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );
    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        None,
        Some(vec![LibraryRootDraft {
            path: series_root.to_string_lossy().to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("store series library roots");

    let session_id = "cancel-additive-library-scan";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .background_library_refresh_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                session_id,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let error = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect_err("additive scan should not be cancelable");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("only full library scans")),
        "unexpected cancel error: {error:?}"
    );

    metadata_gateway.release();

    handle
        .await
        .expect("join additive scan task")
        .expect("background refresh should complete");
}

#[tokio::test]
async fn ensure_library_scan_cancellation_token_reuses_existing_token() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, _user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let first = app
        .ensure_library_scan_cancellation_token("reused-library-scan-token", LibraryScanMode::Full)
        .await
        .expect("first full-scan cancel token");
    let second = app
        .ensure_library_scan_cancellation_token("reused-library-scan-token", LibraryScanMode::Full)
        .await
        .expect("second full-scan cancel token");

    first.cancel();

    assert!(
        second.is_cancelled(),
        "subsequent ensure should reuse the existing cancellation token",
    );
    assert_eq!(
        app.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .len(),
        1,
        "reusing a cancellation token should not create duplicate map entries",
    );
}

#[tokio::test]
async fn pending_import_counts_and_items_are_facet_scoped() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    let known_movie_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Known Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..NewTitle::default()
            },
        )
        .await
        .expect("seed known movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&known_movie_title.title.id, "/movies/Known Movie")
        .await
        .expect("set known movie folder");
    let known_series_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Known Show".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                ..NewTitle::default()
            },
        )
        .await
        .expect("seed known series title");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed movie item");
    let mut matched_movie = build_test_unmatched_item(
        "movie-matched-1",
        MediaFacet::Movie,
        "/movies",
        "/movies/Known.Movie.2020.mkv",
        "Known Movie",
        "Known Movie",
        Some(2020),
    );
    matched_movie.title_id = Some(known_movie_title.title.id.clone());
    unmatched_items
        .upsert_library_scan_unmatched_item(&matched_movie)
        .await
        .expect("seed matched movie item");
    let mut ownership_conflict = build_test_unmatched_item(
        "movie-ownership-conflict-1",
        MediaFacet::Movie,
        "/movies",
        "/movies/Known Movie Copy/Known.Movie.2020.mkv",
        "Known Movie",
        "Known Movie",
        Some(2020),
    );
    ownership_conflict.title_id = Some(known_movie_title.title.id.clone());
    ownership_conflict.reason_code =
        crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER.to_string();
    unmatched_items
        .upsert_library_scan_unmatched_item(&ownership_conflict)
        .await
        .expect("seed ownership conflict");
    let mut series_item = build_test_unmatched_item(
        "series-1",
        MediaFacet::Series,
        "/series",
        "/series/Unknown Show (2020)",
        "Unknown Show (2020)",
        "Unknown Show",
        Some(2020),
    );
    series_item.title_id = Some(known_series_title.title.id.clone());
    unmatched_items
        .upsert_library_scan_unmatched_item(&series_item)
        .await
        .expect("seed series item");
    let mut ignored_movie = build_test_unmatched_item(
        "movie-ignored-1",
        MediaFacet::Movie,
        "/movies",
        "/movies/Ignored.Movie.2020.mkv",
        "Ignored Movie",
        "Ignored Movie",
        Some(2020),
    );
    ignored_movie.status = PendingImportStatus::Ignored;
    unmatched_items
        .upsert_library_scan_unmatched_item(&ignored_movie)
        .await
        .expect("seed ignored movie item");

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts");
    assert_eq!(counts.movie, 2);
    assert_eq!(counts.series, 1);
    assert_eq!(counts.anime, 0);

    let movie_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("movie pending imports");
    assert_eq!(movie_items.total, 2);
    assert_eq!(movie_items.items.len(), 2);
    let unknown_movie = movie_items
        .items
        .iter()
        .find(|item| item.id == "movie-1")
        .expect("unknown movie pending import");
    assert_eq!(unknown_movie.display_name, "Unknown Movie");
    assert_eq!(unknown_movie.path, "/movies/Unknown.Movie.2020.mkv");
    assert_eq!(unknown_movie.folder_path, None);
    let ownership_conflict = movie_items
        .items
        .iter()
        .find(|item| item.id == "movie-ownership-conflict-1")
        .expect("ownership conflict pending import");
    assert_eq!(
        ownership_conflict.title_id.as_deref(),
        Some(known_movie_title.title.id.as_str())
    );

    let ignored_movie_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored movie imports");
    assert_eq!(ignored_movie_items.total, 1);
    assert_eq!(ignored_movie_items.items.len(), 1);
    assert_eq!(ignored_movie_items.items[0].display_name, "Ignored Movie");
    assert_eq!(
        ignored_movie_items.items[0].status,
        PendingImportStatus::Ignored
    );

    let series_items = app
        .pending_imports(
            &user,
            MediaFacet::Series,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("series pending imports");
    assert_eq!(series_items.total, 1);
    assert_eq!(series_items.items.len(), 1);
    assert_eq!(
        series_items.items[0].folder_path.as_deref(),
        Some("/series/Unknown Show (2020)")
    );
    assert_eq!(
        series_items.items[0].title_id.as_deref(),
        Some(known_series_title.title.id.as_str())
    );
    assert_eq!(
        series_items.items[0].title_name.as_deref(),
        Some(known_series_title.title.name.as_str())
    );
    assert_eq!(
        series_items.items[0].title_slug,
        known_series_title.title.slug
    );
}

#[tokio::test]
async fn ignore_pending_import_moves_item_out_of_pending_counts() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-ignore-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Needs.Ignore.2020.mkv",
            "Needs Ignore",
            "Needs Ignore",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let result = app
        .ignore_pending_import(&user, "movie-ignore-1")
        .await
        .expect("ignore pending import");
    assert_eq!(result.status, PendingImportStatus::Ignored);

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts after ignore");
    assert_eq!(counts.movie, 0);

    let pending_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("pending movie imports after ignore");
    assert_eq!(pending_items.total, 0);

    let ignored_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            None,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored movie imports after ignore");
    assert_eq!(ignored_items.total, 1);
    assert_eq!(ignored_items.items[0].id, "movie-ignore-1");
    assert_eq!(ignored_items.items[0].status, PendingImportStatus::Ignored);
}

#[tokio::test]
async fn update_media_settings_removing_root_clears_pending_imports_for_removed_root() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create root one");
    std::fs::create_dir_all(&root_two).expect("create root two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("seed movie roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-root-one",
            MediaFacet::Movie,
            root_one.to_string_lossy().as_ref(),
            root_one
                .join("Unknown.One.2020.mkv")
                .to_string_lossy()
                .as_ref(),
            "Unknown One",
            "Unknown One",
            Some(2020),
        ))
        .await
        .expect("seed first pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-root-two",
            MediaFacet::Movie,
            root_two.to_string_lossy().as_ref(),
            root_two
                .join("Unknown.Two.2021.mkv")
                .to_string_lossy()
                .as_ref(),
            "Unknown Two",
            "Unknown Two",
            Some(2021),
        ))
        .await
        .expect("seed second pending import");

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root_one, true)]),
    )
    .await
    .expect("remove second movie root");

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].scan_root, root_one.to_string_lossy());
}

#[tokio::test]
async fn update_media_settings_root_folders_sync_default_library_roots() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("save movie roots");

    let library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("lookup should succeed")
        .expect("default movie library");
    assert_eq!(
        library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            (root_one.to_string_lossy().to_string(), true),
            (root_two.to_string_lossy().to_string(), false),
        ]
    );
}

#[tokio::test]
async fn reconcile_default_library_roots_backfills_legacy_root_folders_when_bootstrap() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let legacy_roots = vec![
        RootFolderEntry {
            path: "/mnt/anime-main".to_string(),
            is_default: true,
        },
        RootFolderEntry {
            path: "/mnt/anime-archive".to_string(),
            is_default: false,
        },
    ];
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "anime.root_folders",
            &serde_json::to_string(&legacy_roots).expect("serialize legacy roots"),
        )
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings.clone(),
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let anime_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&anime_library_id)
        .await
        .expect("library lookup")
        .expect("default anime library");
    app.services
        .catalog
        .libraries
        .update(
            &anime_library_id,
            anime_library.name.clone(),
            anime_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/data/anime".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed bootstrap root");

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile roots");

    let media_settings = app
        .get_media_settings(&user, MediaFacet::Anime)
        .await
        .expect("anime settings");
    assert_eq!(media_settings.library_path, "/mnt/anime-main");
    assert_eq!(media_settings.root_folders, legacy_roots);
}

#[tokio::test]
async fn reconcile_default_library_roots_keeps_non_bootstrap_canonical_roots() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/legacy/movies")
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings.clone(),
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let movie_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&movie_library_id)
        .await
        .expect("library lookup")
        .expect("default movie library");
    app.services
        .catalog
        .libraries
        .update(
            &movie_library_id,
            movie_library.name.clone(),
            movie_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/canonical/movies".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed canonical root");

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile roots");

    let paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(paths.movie_path, "/canonical/movies");
    assert_eq!(
        app.read_setting_string_value_for_scope_explicit(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            None,
        )
        .await
        .expect("read mirror"),
        Some("/canonical/movies".to_string())
    );
}

#[tokio::test]
async fn reconcile_default_library_roots_repairs_missing_default_libraries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::empty()),
    );

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile missing defaults");

    let libraries = app
        .services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("list repaired libraries");
    assert_eq!(libraries.len(), 3);

    let library_paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(library_paths.movie_path, "/data/movies");
    assert_eq!(library_paths.series_path, "/data/series");
    assert_eq!(library_paths.anime_path, "/data/anime");

    for (facet, expected_path) in [
        (MediaFacet::Movie, "/data/movies"),
        (MediaFacet::Series, "/data/series"),
        (MediaFacet::Anime, "/data/anime"),
    ] {
        let library = app
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await
            .expect("lookup repaired library")
            .expect("default library should be recreated");
        assert_eq!(
            crate::settings::runtime::root_folder_entries_from_library_roots(&library.roots),
            vec![RootFolderEntry {
                path: expected_path.to_string(),
                is_default: true,
            }]
        );
    }
}

#[tokio::test]
async fn update_library_paths_repairs_missing_default_libraries_before_save() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::empty()),
    );

    let updated = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: "/wizard-movies".to_string(),
                series_path: "/wizard-series".to_string(),
                anime_path: Some("/wizard-anime".to_string()),
            },
        )
        .await
        .expect("update repaired library paths");

    assert_eq!(updated.movie_path, "/wizard-movies");
    assert_eq!(updated.series_path, "/wizard-series");
    assert_eq!(updated.anime_path, "/wizard-anime");

    for (facet, expected_path) in [
        (MediaFacet::Movie, "/wizard-movies"),
        (MediaFacet::Series, "/wizard-series"),
        (MediaFacet::Anime, "/wizard-anime"),
    ] {
        let root_folders = app
            .root_folders_for_facet(&facet)
            .await
            .expect("repaired root folders");
        assert_eq!(
            root_folders,
            vec![RootFolderEntry {
                path: expected_path.to_string(),
                is_default: true,
            }]
        );
    }
}

#[tokio::test]
async fn find_or_create_default_user_dedupes_duplicate_default_library_grants() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let duplicate_movie_library = mock_default_library(MediaFacet::Movie);
    let libraries = vec![
        duplicate_movie_library.clone(),
        duplicate_movie_library,
        mock_default_library(MediaFacet::Series),
        mock_default_library(MediaFacet::Anime),
    ];
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::with_libraries(libraries)),
    );

    let admin = app
        .find_or_create_default_user()
        .await
        .expect("create default admin");
    assert_eq!(admin.username, user.username);

    let grants = app
        .services
        .catalog
        .libraries
        .permission_masks_for_user(&admin.id)
        .await
        .expect("load grants");
    let unique_library_ids = grants
        .iter()
        .map(|grant| grant.library_id.clone())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(grants.len(), 3);
    assert_eq!(unique_library_ids.len(), 3);
}

#[tokio::test]
async fn find_or_create_default_user_creates_passwordless_default_actor() {
    let (app, _) = bootstrap();

    let admin = app
        .find_or_create_default_user()
        .await
        .expect("create default admin actor");

    assert_eq!(admin.username, "admin");
    assert!(admin.password_hash.is_none());
    assert!(
        !app.existing_default_admin_uses_bootstrap_password()
            .await
            .expect("check default admin password")
    );
}

#[tokio::test]
async fn existing_default_admin_uses_bootstrap_password_detects_admin_password() {
    let (app, _) = bootstrap();
    let mut admin = User::new_admin("admin");
    admin.password_hash = Some(app.hash_password("admin").expect("hash admin password"));
    app.services
        .identity
        .users
        .create(admin)
        .await
        .expect("seed default admin");

    assert!(
        app.existing_default_admin_uses_bootstrap_password()
            .await
            .expect("check default admin password")
    );
}

#[tokio::test]
async fn usable_admin_login_accepts_non_default_full_admin() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let mut owner = User::new_admin("owner");
    owner.password_hash = Some(
        app.hash_password("correct horse battery staple")
            .expect("hash owner password"),
    );
    let owner = users.create(owner).await.expect("seed owner");
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(
            &owner.id,
            scryer_domain::UserAuthorization::full_admin().app,
        )
        .await
        .expect("grant full admin permissions");

    assert!(
        app.usable_admin_login_exists()
            .await
            .expect("check usable admin login")
    );
}

#[tokio::test]
async fn usable_admin_login_rejects_passwordless_default_admin_only() {
    let (app, _) = bootstrap();
    app.find_or_create_default_user()
        .await
        .expect("create passwordless default admin");

    assert!(
        !app.usable_admin_login_exists()
            .await
            .expect("check usable admin login")
    );
}

#[tokio::test]
async fn usable_admin_login_rejects_malformed_password_hash() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let mut owner = User::new_admin("owner");
    owner.password_hash = Some("v2$not-a-phc-hash".to_string());
    let owner = users.create(owner).await.expect("seed owner");
    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(
            &owner.id,
            scryer_domain::UserAuthorization::full_admin().app,
        )
        .await
        .expect("grant full admin permissions");

    assert!(
        !app.usable_admin_login_exists()
            .await
            .expect("check usable admin login")
    );
}

#[tokio::test]
async fn recover_reserved_admin_access_creates_recovery_admin() {
    let (app, _) = bootstrap();
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            MFA_REQUIRE_CONFIG_STEP_UP_KEY,
            None,
            "true".to_string(),
            "test",
            None,
        )
        .await
        .expect("seed config step-up setting");

    let recovery_admin = app
        .recover_reserved_admin_access("new recovery password")
        .await
        .expect("recover reserved admin access");
    assert_eq!(recovery_admin.username, "recovery-admin");

    let stored_recovery_admin = app
        .services
        .identity
        .users
        .get_by_username("recovery-admin")
        .await
        .expect("load recovery admin")
        .expect("recovery admin created during recovery");
    assert_eq!(stored_recovery_admin.id, recovery_admin.id);
    let password_hash = recovery_admin
        .password_hash
        .as_deref()
        .expect("recovery admin password hash");
    assert!(
        app.validate_password("new recovery password", password_hash)
            .expect("validate recovery admin password")
    );
    assert!(matches!(
        app.authenticate_credentials("recovery-admin", "new recovery password")
            .await,
        Err(AppError::Unauthorized(_))
    ));
    app.set_recovery_admin_login_enabled(true);
    assert_eq!(
        app.authenticate_credentials("recovery-admin", "new recovery password")
            .await
            .expect("authenticate recovery admin while recovery is enabled")
            .id,
        recovery_admin.id
    );
    assert!(
        app.services
            .identity
            .totp
            .get_credential_for_user(&recovery_admin.id)
            .await
            .expect("load recovery admin TOTP")
            .is_none()
    );
    assert!(
        app.services
            .identity
            .webauthn
            .list_credentials_for_user(&recovery_admin.id)
            .await
            .expect("load recovery admin passkeys")
            .is_empty()
    );

    let authorization = app
        .load_user_authorization(&recovery_admin)
        .await
        .expect("load recovery admin authorization");
    assert!(
        authorization
            .app
            .contains(scryer_domain::UserAuthorization::full_admin().app)
    );
    assert!(
        !app.security_settings()
            .await
            .expect("load security settings")
            .mfa_require_config_step_up
    );
}

#[tokio::test]
async fn recover_reserved_admin_access_reenables_environment_managed_identity() {
    let (app, _) = bootstrap();
    let recovery_admin = app
        .recover_reserved_admin_access("initial recovery password")
        .await
        .expect("create recovery admin");
    app.services
        .identity
        .users
        .update_login_status_and_rotate_session(
            &recovery_admin.id,
            scryer_domain::UserLoginStatus::Disabled,
            "disabled-session",
        )
        .await
        .expect("seed disabled recovery admin");

    let repaired = app
        .recover_reserved_admin_access("replacement recovery password")
        .await
        .expect("repair disabled recovery admin");

    assert_eq!(
        repaired.login_status(),
        scryer_domain::UserLoginStatus::Enabled
    );
    app.set_recovery_admin_login_enabled(true);
    assert_eq!(
        app.authenticate_credentials("recovery-admin", "replacement recovery password")
            .await
            .expect("authenticate repaired recovery admin")
            .id,
        recovery_admin.id
    );
}

#[tokio::test]
async fn update_default_library_roots_updates_all_facet_root_read_paths() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let expected_roots = vec![
        RootFolderEntry {
            path: "/library/movies-main".to_string(),
            is_default: true,
        },
        RootFolderEntry {
            path: "/library/movies-archive".to_string(),
            is_default: false,
        },
    ];

    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(
            expected_roots
                .iter()
                .map(|root| LibraryRootDraft {
                    path: root.path.clone(),
                    is_default: root.is_default,
                })
                .collect(),
        ),
        None,
    )
    .await
    .expect("update canonical roots");

    let media_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(media_settings.library_path, "/library/movies-main");
    assert_eq!(media_settings.root_folders, expected_roots);

    let library_paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(library_paths.movie_path, "/library/movies-main");

    let root_folders = app
        .root_folders_for_facet(&MediaFacet::Movie)
        .await
        .expect("root folders");
    assert_eq!(root_folders, media_settings.root_folders);
}

#[tokio::test]
async fn title_root_resolution_uses_owning_library_roots() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/library/default-movies".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("default movie library roots should update");
    let kids_library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids Movies".to_string(),
            vec![LibraryRootDraft {
                path: "/library/kids-movies".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("custom library should be created");
    let mut title = make_due_hydration_title("custom-library-title", MediaFacet::Movie, 42);
    title.library_id = kids_library.id.clone();
    title.root_folder_id = kids_library.roots[0].id.clone();

    let import_paths = crate::import_workflow::resolve_import_paths(&app, &title)
        .await
        .expect("import paths should resolve");
    assert_eq!(import_paths.media_root, "/library/kids-movies");

    let recycle_root = crate::recycle_bin::media_root_for_title(&app, &title).await;
    assert_eq!(recycle_root.as_deref(), Some("/library/kids-movies"));
}

#[tokio::test]
async fn update_default_library_rejects_empty_roots_without_persisting_them() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/library/movies-main".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("initial default roots should update");

    let error = app
        .update_library(&user, &movie_library_id, None, Some(Vec::new()), None)
        .await
        .expect_err("empty default roots should be rejected");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("libraries require at least one root folder")),
        "unexpected error: {error:?}"
    );

    let library = app
        .services
        .catalog
        .libraries
        .get_by_id(&movie_library_id)
        .await
        .expect("library lookup should succeed")
        .expect("movie library should exist");
    assert_eq!(library.roots.len(), 1);
    assert_eq!(library.roots[0].path, "/library/movies-main");
}

#[tokio::test]
async fn update_library_removing_root_clears_pending_imports_for_removed_root() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/movies-old".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("initial default roots should update");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root-canonical",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");

    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/movies-new".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("canonical roots should update");

    let items = unmatched_items.items().await;
    assert!(items.is_empty());
}

#[tokio::test]
async fn update_library_paths_removing_root_clears_pending_imports_for_removed_root() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "series-root",
            MediaFacet::Series,
            "/series",
            "/series/Unknown Show (2020)",
            "Unknown Show",
            "Unknown Show",
            Some(2020),
        ))
        .await
        .expect("seed kept pending import");

    app.update_library_paths(
        &user,
        UpdateLibraryPaths {
            movie_path: "/movies-new".to_string(),
            series_path: "/series".to_string(),
            anime_path: Some("/anime".to_string()),
        },
    )
    .await
    .expect("update library paths");

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Series);
    assert_eq!(items[0].scan_root, "/series");
}

#[tokio::test]
async fn update_library_paths_allows_partial_wizard_paths() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime-old")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings.clone(),
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items,
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

    let updated = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: "".to_string(),
                series_path: "/series-new".to_string(),
                anime_path: None,
            },
        )
        .await
        .expect("update partial library paths");

    assert_eq!(updated.movie_path, "/movies-old");
    assert_eq!(updated.series_path, "/series-new");
    assert_eq!(updated.anime_path, "/anime-old");
}

#[tokio::test]
async fn save_external_import_library_paths_removing_root_clears_pending_imports_for_removed_root()
{
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root-external",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "anime-root-external",
            MediaFacet::Anime,
            "/anime",
            "/anime/Unknown Anime",
            "Unknown Anime",
            "Unknown Anime",
            Some(2021),
        ))
        .await
        .expect("seed kept pending import");

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec!["/movies-new".to_string()],
                series_paths: vec![],
                anime_paths: vec![],
            },
        )
        .await
        .expect("save external import paths");

    assert!(saved);
    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Anime);
    assert_eq!(items[0].scan_root, "/anime");
}

#[tokio::test]
async fn save_external_import_library_paths_persists_multiple_root_folders_per_facet() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec![
                    "/movies-primary".to_string(),
                    "/movies-secondary".to_string(),
                ],
                series_paths: vec!["/series-main".to_string(), "/series-archive".to_string()],
                anime_paths: vec!["/anime".to_string()],
            },
        )
        .await
        .expect("save external import paths");

    assert!(saved);

    let movie_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(movie_settings.library_path, "/movies-primary");
    assert_eq!(
        movie_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/movies-primary".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/movies-secondary".to_string(),
                is_default: false,
            },
        ]
    );

    let series_settings = app
        .get_media_settings(&user, MediaFacet::Series)
        .await
        .expect("series settings");
    assert_eq!(series_settings.library_path, "/series-main");
    assert_eq!(
        series_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/series-main".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/series-archive".to_string(),
                is_default: false,
            },
        ]
    );

    let movie_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("lookup should succeed")
        .expect("default movie library");
    assert_eq!(
        movie_library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            ("/movies-primary".to_string(), true),
            ("/movies-secondary".to_string(), false),
        ]
    );

    let series_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Series)
        .await
        .expect("lookup should succeed")
        .expect("default series library");
    assert_eq!(
        series_library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            ("/series-main".to_string(), true),
            ("/series-archive".to_string(), false),
        ]
    );
}

#[tokio::test]
async fn save_external_import_library_paths_accepts_custom_selected_paths() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec![
                    "/custom/movies".to_string(),
                    "/custom/movies-archive".to_string(),
                ],
                series_paths: vec!["/custom/series".to_string()],
                anime_paths: vec!["/custom/anime".to_string()],
            },
        )
        .await
        .expect("save custom external import paths");

    assert!(saved);

    let movie_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(movie_settings.library_path, "/custom/movies");
    assert_eq!(
        movie_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/custom/movies".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/custom/movies-archive".to_string(),
                is_default: false,
            },
        ]
    );

    let series_settings = app
        .get_media_settings(&user, MediaFacet::Series)
        .await
        .expect("series settings");
    assert_eq!(series_settings.library_path, "/custom/series");
    assert_eq!(
        series_settings.root_folders,
        vec![RootFolderEntry {
            path: "/custom/series".to_string(),
            is_default: true,
        }]
    );

    let anime_settings = app
        .get_media_settings(&user, MediaFacet::Anime)
        .await
        .expect("anime settings");
    assert_eq!(anime_settings.library_path, "/custom/anime");
    assert_eq!(
        anime_settings.root_folders,
        vec![RootFolderEntry {
            path: "/custom/anime".to_string(),
            is_default: true,
        }]
    );
}

fn pending_import_title_request(
    facet: MediaFacet,
    name: &str,
    tvdb_id: Option<&str>,
    year: Option<i32>,
) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet,
        monitored: true,
        tags: vec!["should-be-cleared".to_string()],
        external_ids: tvdb_id
            .map(|value| {
                vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: value.to_string(),
                }]
            })
            .unwrap_or_default(),
        root_folder_id: Some("should-be-cleared".to_string()),
        min_availability: Some("should-be-cleared".to_string()),
        poster_url: None,
        year,
        overview: Some("Matched overview".to_string()),
        sort_title: Some(name.to_string()),
        slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
        runtime_minutes: Some(101),
        language: Some("eng".to_string()),
        content_status: Some("Released".to_string()),
    }
}

struct PendingImportSearchMetadataGateway {
    results: Vec<RichMetadataSearchItem>,
}

#[async_trait]
impl MetadataGateway for PendingImportSearchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(self.results.clone())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }
}

fn pending_import_search_result(tvdb_id: &str, name: &str) -> RichMetadataSearchItem {
    RichMetadataSearchItem {
        tvdb_id: tvdb_id.to_string(),
        smg_id: None,
        primary_source: None,
        external_ids: vec![],
        name: name.to_string(),
        imdb_id: None,
        slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
        type_hint: Some("movie".to_string()),
        year: Some(2020),
        status: Some("Released".to_string()),
        overview: None,
        popularity: None,
        poster_url: None,
        language: Some("eng".to_string()),
        runtime_minutes: Some(101),
        sort_title: Some(name.to_string()),
    }
}

#[tokio::test]
async fn pending_import_title_search_filters_same_library_titles_only() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(PendingImportSearchMetadataGateway {
            results: vec![
                pending_import_search_result("123456", "Other Library Movie"),
                pending_import_search_result("333333", "Existing Movie"),
                pending_import_search_result("222222", "New Movie"),
            ],
        }),
    );

    let mut existing_request = pending_import_title_request(
        MediaFacet::Movie,
        "Existing Movie",
        Some("333333"),
        Some(2020),
    );
    existing_request.root_folder_id = None;
    existing_request.min_availability = None;
    let existing_title = app
        .create_title_without_hydration(&user, existing_request)
        .await
        .expect("seed existing title");
    let mut other_library_title = existing_title.title.clone();
    other_library_title.id = "other-library-movie-title".to_string();
    other_library_title.library_id = "other-movie-library".to_string();
    other_library_title.name = "Other Library Movie".to_string();
    other_library_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    {
        let mut store = titles.store.lock().await;
        store.push(other_library_title);
        for index in 0..100 {
            let mut noise_title = existing_title.title.clone();
            noise_title.id = format!("noise-movie-title-{index}");
            noise_title.name = format!("Noise Movie {index}");
            noise_title.external_ids = vec![ExternalId {
                source: "tvdb".to_string(),
                value: format!("9{index:05}"),
            }];
            store.push(noise_title);
        }
    }

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-search-filter-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Existing Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let results = app
        .pending_import_title_search(
            &user,
            "movie-search-filter-1",
            "Movie",
            8,
            "eng",
            Some(2020),
        )
        .await
        .expect("search pending import titles");

    let result_ids = results
        .iter()
        .map(|result| result.tvdb_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(result_ids, vec!["123456", "222222"]);
    assert_eq!(
        titles.external_id_batch_lookup_calls.load(Ordering::SeqCst),
        1,
        "pending-import title search should batch same-library exclusion"
    );
}

#[tokio::test]
async fn resolve_pending_import_creates_unmonitored_movie_title_and_keeps_item_bound() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Unknown.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    target_key: None,
                    smg_id: None,
                    primary_source: "tvdb".into(),
                    tvdb_id: Some(123_456),
                    name: "Matched Movie".into(),
                    slug: "matched-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Matched overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    background_url: None,
                    language: "eng".into(),
                    original_language: Some("eng".into()),
                    runtime_minutes: 101,
                    sort_title: "Matched Movie".into(),
                    imdb_id: "tt0123456".into(),
                    tmdb_id: None,
                    popularity: None,
                    anidb_id: None,
                    canonical_tags: vec![],
                    studio: "Test Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                    ratings: Default::default(),
                    credits: Vec::new(),
                    ..Default::default()
                },
            )]),
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Matched Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let mut request = pending_import_title_request(
        MediaFacet::Movie,
        "Matched Movie",
        Some("123456"),
        Some(2020),
    );
    request.external_ids = vec![ExternalId {
        source: "tmdb".to_string(),
        value: "5001".to_string(),
    }];
    let result = app
        .resolve_pending_import(&user, "movie-resolve-1", request)
        .await
        .expect("resolve pending import");

    assert!(result.created);
    assert!(!result.title.monitored);
    assert_eq!(result.title.name, "Matched Movie");
    assert!(result.title.tags.is_empty());
    assert_ne!(result.title.root_folder_id, "should-be-cleared");
    assert!(result.title.min_availability.is_none());
    assert!(result.library_scan.is_none());
    assert!(matches!(
        result.metadata_hydration_state,
        AddTitleHydrationState::Pending
    ));
    assert!(unmatched_items.items().await.is_empty());
}

#[tokio::test]
async fn resolve_ignored_pending_import_creates_unmonitored_movie_title_and_clears_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Ignored.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    target_key: None,
                    smg_id: None,
                    primary_source: "tvdb".into(),
                    tvdb_id: Some(123_456),
                    name: "Matched Movie".into(),
                    slug: "matched-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Matched overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    background_url: None,
                    language: "eng".into(),
                    original_language: Some("eng".into()),
                    runtime_minutes: 101,
                    sort_title: "Matched Movie".into(),
                    imdb_id: "tt0123456".into(),
                    tmdb_id: None,
                    popularity: None,
                    anidb_id: None,
                    canonical_tags: vec![],
                    studio: "Test Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                    ratings: Default::default(),
                    credits: Vec::new(),
                    ..Default::default()
                },
            )]),
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let mut ignored_item = build_test_unmatched_item(
        "movie-resolve-ignored-1",
        MediaFacet::Movie,
        tempdir.path().to_string_lossy().as_ref(),
        movie_path.to_string_lossy().as_ref(),
        "Ignored Movie",
        "Matched Movie",
        Some(2020),
    );
    ignored_item.status = PendingImportStatus::Ignored;

    unmatched_items
        .upsert_library_scan_unmatched_item(&ignored_item)
        .await
        .expect("seed ignored import");

    let result = app
        .resolve_pending_import(
            &user,
            "movie-resolve-ignored-1",
            pending_import_title_request(
                MediaFacet::Movie,
                "Matched Movie",
                Some("123456"),
                Some(2020),
            ),
        )
        .await
        .expect("resolve ignored pending import");

    assert!(result.created);
    assert_eq!(result.title.name, "Matched Movie");
    assert!(unmatched_items.items().await.is_empty());
}

#[tokio::test]
async fn resolve_pending_import_failure_keeps_pending_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Unknown.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-failure-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Matched Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let error = app
        .resolve_pending_import(
            &user,
            "movie-resolve-failure-1",
            pending_import_title_request(MediaFacet::Movie, "Matched Movie", None, Some(2020)),
        )
        .await
        .expect_err("resolution should fail without tvdb id");
    assert!(!error.to_string().trim().is_empty());
    assert_eq!(unmatched_items.items().await.len(), 1);
    assert!(
        app.list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn hydrate_titles_bulk_updates_title_name_for_selected_metadata_language() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, "jpn")
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    target_key: None,
                    smg_id: None,
                    primary_source: "tvdb".into(),
                    tvdb_id: Some(123_456),
                    name: "サンドライン".into(),
                    slug: "sandline".into(),
                    year: Some(2021),
                    content_status: "Released".into(),
                    overview: "日本語概要".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    background_url: None,
                    language: "jpn".into(),
                    original_language: Some("jpn".into()),
                    runtime_minutes: 155,
                    sort_title: "サンドライン".into(),
                    imdb_id: "tt1160419".into(),
                    tmdb_id: None,
                    popularity: None,
                    anidb_id: None,
                    canonical_tags: vec![],
                    studio: "Legendary".into(),
                    tmdb_release_date: Some("2021-10-22".into()),
                    ratings: Default::default(),
                    credits: Vec::new(),
                    ..Default::default()
                },
            )]),
        }),
    );

    let created = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Glass Harbor".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: None,
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("seed untranslated title");
    let created_title = created.title;

    let mut outcome = app
        .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title: created_title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::Interactive,
        }])
        .await
        .expect("hydrate title");

    let hydrated = outcome
        .hydrated_titles
        .remove(&created_title.id)
        .expect("hydrated title should be returned");
    assert_eq!(hydrated.name, "サンドライン");
    assert_eq!(hydrated.metadata_language.as_deref(), Some("jpn"));
    assert_eq!(hydrated.overview.as_deref(), Some("日本語概要"));

    let persisted = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list titles");
    assert_eq!(persisted[0].name, "サンドライン");
    assert_eq!(persisted[0].metadata_language.as_deref(), Some("jpn"));
}

#[tokio::test]
async fn hydrate_titles_bulk_persists_movie_tmdb_external_id() {
    let mut movie = make_movie_metadata(91_501, "Hydrated Movie");
    movie.imdb_id = "tt9150100".to_string();
    movie.tmdb_id = Some(815_010);
    movie.anidb_id = Some(715_010);
    let metadata_gateway = Arc::new(MockMetadataGateway {
        // This test gateway remains TVDB-keyed; TMDB-primary rows are not in this map.
        movies: HashMap::from([(movie.tvdb_id.unwrap_or(0), movie)]),
    });
    let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);
    let title = make_due_hydration_title("movie-tmdb-hydration", MediaFacet::Movie, 91_501);

    TitleRepository::create(&*titles, title.clone())
        .await
        .expect("seed due movie title");

    let mut outcome = app
        .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title: title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::Interactive,
        }])
        .await
        .expect("hydrate title");

    let hydrated = outcome
        .hydrated_titles
        .remove(&title.id)
        .expect("hydrated title should be returned");
    assert!(
        hydrated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tmdb" && external_id.value == "815010" })
    );

    let persisted = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load hydrated title")
        .expect("hydrated title should exist");
    assert!(
        persisted
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tmdb" && external_id.value == "815010" })
    );
}

#[tokio::test]
async fn background_hydration_completes_without_inline_recommendation_refresh() {
    let recommendation_calls = Arc::new(AtomicUsize::new(0));
    let recommendation_release = Arc::new(Notify::new());
    let metadata_gateway = Arc::new(CountingRecommendationMetadataGateway {
        movies: HashMap::from([(91_601, make_movie_metadata(91_601, "Hydrated Movie"))]),
        title_recommendation_calls: Arc::clone(&recommendation_calls),
        recommendation_release: Some(Arc::clone(&recommendation_release)),
    });
    let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);
    let title = make_due_hydration_title("movie-background-hydration", MediaFacet::Movie, 91_601);
    TitleRepository::create(&*titles, title.clone())
        .await
        .expect("seed due movie title");
    let mut outcome = timeout(
        Duration::from_secs(2),
        app.hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title: title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::BackgroundDue,
        }]),
    )
    .await
    .expect("background hydration should not wait for recommendation refresh")
    .expect("hydrate title");

    recommendation_release.notify_one();

    assert!(
        outcome.hydrated_titles.remove(&title.id).is_some(),
        "metadata persistence should complete hydration"
    );
    assert!(
        recommendation_calls.load(Ordering::SeqCst) <= 1,
        "background hydration may queue one async recommendation refresh"
    );
}

#[tokio::test]
async fn interactive_hydration_queues_recommendations_off_the_hydration_path() {
    let recommendation_calls = Arc::new(AtomicUsize::new(0));
    let recommendation_release = Arc::new(Notify::new());
    let metadata_gateway = Arc::new(CountingRecommendationMetadataGateway {
        movies: HashMap::from([(91_602, make_movie_metadata(91_602, "Hydrated Movie"))]),
        title_recommendation_calls: Arc::clone(&recommendation_calls),
        recommendation_release: Some(Arc::clone(&recommendation_release)),
    });
    let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);
    let title = make_due_hydration_title("movie-interactive-hydration", MediaFacet::Movie, 91_602);
    TitleRepository::create(&*titles, title.clone())
        .await
        .expect("seed due movie title");

    timeout(
        Duration::from_secs(2),
        app.hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title,
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::Interactive,
        }]),
    )
    .await
    .expect("interactive hydration should not wait for recommendation refresh")
    .expect("hydrate title");

    timeout(Duration::from_secs(2), async {
        while recommendation_calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("queued recommendation refresh should reach the metadata gateway");
    assert_eq!(
        recommendation_calls.load(Ordering::SeqCst),
        1,
        "interactive hydration should queue one recommendation refresh"
    );
    recommendation_release.notify_one();
}

#[tokio::test]
async fn resolve_pending_import_rejects_existing_title_in_same_library() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Missing.Movie.2020.mkv");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner.set_library_files(vec![]).await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

    let existing_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Existing Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: Some(2020),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("seed existing title");
    let existing_title = existing_title.title;
    app.services
        .catalog
        .titles
        .set_folder_path(&existing_title.id, "/existing/movies/Existing Movie")
        .await
        .expect("set original folder path");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-existing-failure-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Existing Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let error = app
        .resolve_pending_import(
            &user,
            "movie-resolve-existing-failure-1",
            pending_import_title_request(
                MediaFacet::Movie,
                "Existing Movie",
                Some("123456"),
                Some(2020),
            ),
        )
        .await
        .expect_err("resolution should fail when title already exists");
    assert!(
        error
            .to_string()
            .contains("title already exists in this library")
    );
    assert_eq!(unmatched_items.items().await.len(), 1);

    let refreshed_title = app
        .services
        .catalog
        .titles
        .get_by_id(&existing_title.id)
        .await
        .expect("load existing title")
        .expect("existing title should still exist");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some("/existing/movies/Existing Movie")
    );
}

#[tokio::test]
async fn ownership_conflict_pending_import_cannot_be_bound_or_adopted() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    let title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Case Split Fixture".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                ..NewTitle::default()
            },
        )
        .await
        .expect("seed existing title")
        .title;
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, "/series/CASE SPLIT FIXTURE")
        .await
        .expect("set owned folder");
    let mut conflict = build_test_unmatched_item(
        "series-ownership-conflict-1",
        MediaFacet::Series,
        "/series",
        "/series/Case Split Fixture/Season 01/Case Split Fixture - S01E01.mkv",
        "Case Split Fixture",
        "Case Split Fixture",
        None,
    );
    conflict.title_id = Some(title.id.clone());
    conflict.reason_code =
        crate::library_scan_unmatched::LIBRARY_SCAN_TITLE_ALREADY_OWNS_ANOTHER_FOLDER.to_string();
    unmatched_items
        .upsert_library_scan_unmatched_item(&conflict)
        .await
        .expect("seed ownership conflict");

    let resolve_error = app
        .resolve_pending_import(
            &user,
            &conflict.id,
            pending_import_title_request(
                MediaFacet::Series,
                "Case Split Fixture",
                Some("123"),
                None,
            ),
        )
        .await
        .expect_err("ownership conflict must not be adopted");
    let preview_error = app
        .preview_title_bound_pending_import(&user, &conflict.id)
        .await
        .expect_err("ownership conflict must not be previewed for binding");
    let bind_error = app
        .bind_title_bound_pending_import(&user, &conflict.id, None, &[])
        .await
        .expect_err("ownership conflict must not be bound");

    for error in [resolve_error, preview_error, bind_error] {
        assert!(
            error
                .to_string()
                .contains("folder ownership conflicts cannot be bound or adopted")
        );
    }
    assert_eq!(unmatched_items.items().await, vec![conflict]);
}
