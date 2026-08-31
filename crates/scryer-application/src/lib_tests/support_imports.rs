use super::*;

#[derive(Default)]
pub(super) struct MockUserRepo {
    pub(super) store: Arc<Mutex<Vec<User>>>,
    pub(super) auth_session_versions: Arc<Mutex<HashMap<String, String>>>,
    pub(super) get_by_id_calls: Arc<AtomicUsize>,
    pub(super) list_all_calls: Arc<AtomicUsize>,
}

impl MockUserRepo {
    pub(super) fn get_by_id_call_count(&self) -> usize {
        self.get_by_id_calls.load(Ordering::SeqCst)
    }

    pub(super) fn list_all_call_count(&self) -> usize {
        self.list_all_calls.load(Ordering::SeqCst)
    }
}

#[derive(Default, Clone)]
pub(super) struct CopyingFileImporter;

pub(super) fn test_import_source_snapshot(
    path: &Path,
) -> AppResult<scryer_domain::ImportSourceSnapshot> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(path).map_err(|err| {
        AppError::Repository(format!(
            "failed to stat import source {}: {err}",
            path.display()
        ))
    })?;
    let bytes = std::fs::read(path).map_err(|err| {
        AppError::Repository(format!(
            "failed to read import source {}: {err}",
            path.display()
        ))
    })?;

    Ok(scryer_domain::ImportSourceSnapshot {
        identity: scryer_domain::ImportSourceIdentity {
            file: scryer_domain::ImportFileIdentity {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                #[cfg(unix)]
                dev: metadata.dev(),
                #[cfg(unix)]
                ino: metadata.ino(),
            },
            kind: scryer_domain::ImportSourceIdentityKind::Regular,
        },
        proof: scryer_domain::ImportContentProof {
            size_bytes: metadata.len(),
            sample_bytes: bytes.len() as u64,
            sample_blake3: blake3::hash(&bytes).to_hex().to_string(),
        },
    })
}

#[async_trait]
impl FileImporter for CopyingFileImporter {
    async fn snapshot_import_source(
        &self,
        source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot> {
        test_import_source_snapshot(source)
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        if let Some(expected_source) = expected_source {
            let actual = test_import_source_snapshot(source)?;
            if &actual != expected_source {
                return Err(AppError::Repository(format!(
                    "import source changed after validation: {}",
                    source.display()
                )));
            }
        }

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                AppError::Repository(format!(
                    "failed to create import destination {}: {err}",
                    parent.display()
                ))
            })?;
        }

        let strategy = match mode {
            scryer_domain::ImportMode::HardlinkOrCopy => {
                if tokio::fs::hard_link(source, dest).await.is_ok() {
                    scryer_domain::ImportStrategy::HardLink
                } else {
                    tokio::fs::copy(source, dest).await.map_err(|err| {
                        AppError::Repository(format!(
                            "failed to copy import source {} to {}: {err}",
                            source.display(),
                            dest.display()
                        ))
                    })?;
                    scryer_domain::ImportStrategy::Copy
                }
            }
            scryer_domain::ImportMode::Move => {
                tokio::fs::rename(source, dest).await.map_err(|err| {
                    AppError::Repository(format!(
                        "failed to move import source {} to {}: {err}",
                        source.display(),
                        dest.display()
                    ))
                })?;
                scryer_domain::ImportStrategy::Move
            }
        };
        let size_bytes = std::fs::metadata(dest)
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        Ok(scryer_domain::ImportFileResult {
            strategy,
            source_path: source.to_path_buf(),
            dest_path: dest.to_path_buf(),
            size_bytes,
            destination_disposition: scryer_domain::ImportDestinationDisposition::Created,
            source_cleanup: None,
        })
    }

    async fn remove_import_source_after_verified_import(
        &self,
        _guard: scryer_domain::ImportSourceCleanupGuard,
        _final_dest_path: &Path,
    ) -> AppResult<()> {
        Ok(())
    }
}

/// [`CopyingFileImporter`] that also reports transfer progress the way the
/// real importer does (copying 0 → total, then finalizing), so tests can
/// observe what an import path writes onto its import record.
pub(super) struct ProgressReportingFileImporter;

#[async_trait]
impl FileImporter for ProgressReportingFileImporter {
    async fn snapshot_import_source(
        &self,
        source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot> {
        CopyingFileImporter.snapshot_import_source(source).await
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        CopyingFileImporter
            .import_file(source, dest, mode, expected_source)
            .await
    }

    async fn import_file_with_progress_and_permissions(
        &self,
        source: &Path,
        dest: &Path,
        mode: scryer_domain::ImportMode,
        expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
        progress: Option<crate::ImportFileTransferProgressSender>,
        _permissions: &crate::ImportFilePermissions,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        let total_bytes = std::fs::metadata(source)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if let Some(progress) = progress.as_ref() {
            for (phase, bytes) in [
                (scryer_domain::ImportTransferPhase::Copying, 0),
                (scryer_domain::ImportTransferPhase::Copying, total_bytes),
                (scryer_domain::ImportTransferPhase::Finalizing, total_bytes),
            ] {
                let _ = progress.send(crate::ImportFileTransferProgress {
                    phase,
                    bytes,
                    total_bytes,
                });
            }
        }
        self.import_file(source, dest, mode, expected_source).await
    }

    async fn remove_import_source_after_verified_import(
        &self,
        _guard: scryer_domain::ImportSourceCleanupGuard,
        _final_dest_path: &Path,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default, Clone)]
pub(super) struct MockMediaFileRepo {
    pub(super) store: Arc<Mutex<Vec<TitleMediaFile>>>,
    /// Optional bridge for the background acquisition cursor: when set, the
    /// derived missing-target sweep reads the seeded acquisition-state rows so a
    /// mock-backed store still yields targets for `run_background_acquisition_cycle_once`.
    /// Left `None` for stores that manage their own media files directly.
    pub(super) missing_scope_source: Option<Arc<super::TrackingAcquisitionScopeStateRepo>>,
    /// The catalog the seeded scopes belong to — used to resolve each scope's
    /// real facet (movie/series/anime) for the derived target, since the thinned
    /// state row does not carry it.
    pub(super) missing_scope_titles: Option<Arc<super::MockTitleRepo>>,
}

impl MockMediaFileRepo {
    /// Wire the seeded wanted-state store (and its catalog) as the missing-target
    /// source so the convergence cursor sees each monitored, fileless scope as a
    /// target with its correct facet.
    pub(super) fn with_missing_scope_source(
        source: Arc<super::TrackingAcquisitionScopeStateRepo>,
        titles: Arc<super::MockTitleRepo>,
    ) -> Self {
        Self {
            store: Arc::new(Mutex::new(Vec::new())),
            missing_scope_source: Some(source),
            missing_scope_titles: Some(titles),
        }
    }
}

fn mock_media_file(id: String, input: &InsertMediaFileInput) -> TitleMediaFile {
    TitleMediaFile {
        id,
        title_id: input.title_id.clone(),
        episode_id: None,
        series_movie_link_ids: Vec::new(),
        role: input.role,
        file_path: input.file_path.clone(),
        size_bytes: input.size_bytes,
        announced_size_bytes: input.announced_size_bytes,
        source_signature_scheme: input.source_signature_scheme.clone(),
        source_signature_value: input.source_signature_value.clone(),
        quality_label: input.quality_label.clone(),
        scan_status: "pending".to_string(),
        created_at: Utc::now().to_rfc3339(),
        video_codec: None,
        video_width: None,
        video_height: None,
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
        duration_seconds: None,
        num_chapters: None,
        container_format: None,
        scene_name: input.scene_name.clone(),
        release_group: input.release_group.clone(),
        source_type: input.source_type.clone(),
        resolution: input.resolution.clone(),
        video_codec_parsed: input.video_codec_parsed,
        audio_codec_parsed: input.audio_codec_parsed.clone(),
        audio_channels_parsed: input.audio_channels_parsed.clone(),
        acquisition_score: input.acquisition_score,
        scoring_log: input.scoring_log.clone(),
        indexer_source: input.indexer_source.clone(),
        grabbed_release_title: input.grabbed_release_title.clone(),
        grabbed_at: input.grabbed_at.clone(),
        edition: input.edition.clone(),
        original_file_path: input.original_file_path.clone(),
        release_hash: input.release_hash.clone(),
    }
}

#[async_trait]
impl MediaFileRepository for MockMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        let id = Id::new().0;
        self.store
            .lock()
            .await
            .push(mock_media_file(id.clone(), input));
        Ok(id)
    }

    async fn claim_import_destination(
        &self,
        input: &InsertMediaFileInput,
        associations: &MediaFileAssociations,
    ) -> AppResult<crate::ClaimedMediaFile> {
        let mut files = self.store.lock().await;
        if let Some(existing) = files
            .iter_mut()
            .find(|file| file.file_path == input.file_path)
        {
            let episode_matches = existing
                .episode_id
                .as_ref()
                .is_none_or(|id| associations.episode_ids.contains(id));
            let links_match = existing
                .series_movie_link_ids
                .iter()
                .all(|id| associations.series_movie_link_ids.contains(id));
            let has_associations =
                existing.episode_id.is_some() || !existing.series_movie_link_ids.is_empty();
            let provenance_matches =
                has_associations || existing.original_file_path == input.original_file_path;
            if existing.title_id != input.title_id
                || !episode_matches
                || !links_match
                || !provenance_matches
            {
                return Err(AppError::ManualReconciliationRequired(format!(
                    "destination {} belongs to another import target",
                    input.file_path
                )));
            }
            if existing.episode_id.is_none() {
                existing.episode_id = associations.episode_ids.first().cloned();
            }
            for link_id in &associations.series_movie_link_ids {
                if !existing.series_movie_link_ids.contains(link_id) {
                    existing.series_movie_link_ids.push(link_id.clone());
                }
            }
            return Ok(crate::ClaimedMediaFile {
                media_file_id: existing.id.clone(),
                disposition: crate::MediaFileCatalogDisposition::Reused,
            });
        }

        let id = Id::new().0;
        let mut file = mock_media_file(id.clone(), input);
        file.episode_id = associations.episode_ids.first().cloned();
        file.series_movie_link_ids = associations.series_movie_link_ids.clone();
        files.push(file);
        Ok(crate::ClaimedMediaFile {
            media_file_id: id,
            disposition: crate::MediaFileCatalogDisposition::Created,
        })
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.episode_id = Some(episode_id.to_string());
        Ok(())
    }

    async fn link_file_to_series_movie(
        &self,
        file_id: &str,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        if !entry
            .series_movie_link_ids
            .iter()
            .any(|existing| existing == series_movie_link_id)
        {
            entry
                .series_movie_link_ids
                .push(series_movie_link_id.to_string());
        }
        Ok(())
    }

    async fn list_missing_scope_candidates(&self) -> AppResult<MissingScopeCandidates> {
        // Without a real library store, derive the missing-scope
        // sweep from the seeded acquisition-state rows so the convergence cursor
        // sees each monitored, fileless `wanted` scope as a target. Synthetic
        // recency inputs (past air date, current add date) keep the scope inside
        // its availability window and in the hot lane, matching a freshly wanted
        // scope.
        let Some(source) = self.missing_scope_source.as_ref() else {
            return Ok(MissingScopeCandidates::default());
        };
        let now = Utc::now();
        let past_air = (now - chrono::Duration::days(1)).to_rfc3339();
        let created = now.to_rfc3339();
        let mut candidates = MissingScopeCandidates::default();
        for item in source.store.lock().await.iter() {
            if item.status != AcquisitionScopeStatus::Wanted {
                continue;
            }
            // Resolve the scope's real facet from the catalog (the thinned state
            // row does not carry it); fall back to the row's facet, then to the
            // media-type shape.
            let facet = match self.missing_scope_titles.as_ref() {
                Some(titles) => titles
                    .get_by_id(&item.title_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|title| title.facet.as_str().to_string()),
                None => None,
            }
            .or_else(|| item.title_facet.clone())
            .unwrap_or_else(|| {
                if item.episode_id.is_some() {
                    scryer_domain::MediaFacet::Series.as_str().to_string()
                } else {
                    scryer_domain::MediaFacet::Movie.as_str().to_string()
                }
            });
            let library_id = item.library_id.clone().unwrap_or_default();
            if let Some(link_id) = item.series_movie_link_id.clone() {
                candidates
                    .series_movie_links
                    .push(MissingSeriesMovieLinkCandidate {
                        series_movie_link_id: link_id,
                        title_id: item.title_id.clone(),
                        library_id,
                        title_facet: facet,
                        continuity_status: None,
                        movie_digital_release_date: None,
                        link_created_at: created.clone(),
                    });
            } else if let Some(episode_id) = item.episode_id.clone() {
                candidates.episodes.push(MissingEpisodeCandidate {
                    episode_id,
                    title_id: item.title_id.clone(),
                    library_id,
                    title_facet: facet,
                    collection_id: item.collection_id.clone(),
                    season_number: item.season_number.clone(),
                    episode_number: item.episode_number.clone(),
                    air_date: Some(past_air.clone()),
                    title_created_at: created.clone(),
                });
            } else {
                candidates.titles.push(MissingTitleCandidate {
                    title_id: item.title_id.clone(),
                    library_id,
                    title_facet: facet,
                    min_availability: None,
                    first_aired: None,
                    digital_release_date: None,
                    created_at: created.clone(),
                });
            }
        }
        // Deterministic season/episode order for the derived episode set, so
        // per-cycle season-pack grouping and grab ordering are stable in tests.
        candidates.episodes.sort_by(|left, right| {
            let key = |candidate: &MissingEpisodeCandidate| {
                (
                    candidate
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse::<i64>().ok())
                        .unwrap_or(i64::MAX),
                    candidate
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<i64>().ok())
                        .unwrap_or(i64::MAX),
                    candidate.episode_id.clone(),
                )
            };
            key(left).cmp(&key(right))
        });
        Ok(candidates)
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        let requested = episode_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        // The real store joins the file-episode table and aggregates, so a file
        // spanning two episodes comes back **once** with both ids. Modelling it
        // as one row per link here would let a test pass against a span the
        // product never sees.
        let mut spans: Vec<(TitleMediaFile, Vec<String>)> = Vec::new();
        for entry in self.store.lock().await.iter() {
            if entry.title_id != title_id {
                continue;
            }
            let Some(episode_id) = entry.episode_id.clone() else {
                continue;
            };
            match spans.iter_mut().find(|(file, _)| file.id == entry.id) {
                Some((_, episode_ids)) => {
                    if !episode_ids.contains(&episode_id) {
                        episode_ids.push(episode_id);
                    }
                }
                None => spans.push((entry.clone(), vec![episode_id])),
            }
        }
        Ok(spans
            .into_iter()
            .filter(|(_, episode_ids)| {
                episode_ids
                    .iter()
                    .any(|episode_id| requested.contains(episode_id.as_str()))
            })
            .map(|(media_file, episode_ids)| {
                let title_role = media_file.role;
                let primary_episode_ids = if media_file.role.is_primary() {
                    episode_ids.clone()
                } else {
                    Vec::new()
                };
                EpisodeScopedMediaFile {
                    media_file,
                    title_role,
                    episode_ids,
                    primary_episode_ids,
                }
            })
            .collect())
    }

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<String>> {
        Ok(vec![])
    }

    async fn list_title_media_size_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        Ok(Vec::new())
    }

    async fn collection_media_size_bytes(
        &self,
        _title_id: &str,
        _ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        Ok(None)
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        let rank = |value: &str| match value.trim().to_ascii_uppercase().as_str() {
            "4320P" => 0,
            "2160P" => 1,
            "1440P" => 2,
            "1080P" => 3,
            "1080I" => 4,
            "720P" => 5,
            "480P" => 6,
            "360P" => 7,
            _ => 999,
        };

        let store = self.store.lock().await;
        let mut out = Vec::new();
        for title_id in title_ids {
            let mut selected: Option<(i32, String)> = None;
            for entry in store.iter().filter(|entry| &entry.title_id == title_id) {
                let Some(label) = entry.quality_label.as_ref() else {
                    continue;
                };
                let normalized = label.trim().to_ascii_uppercase();
                if normalized.is_empty() {
                    continue;
                }
                let candidate = (rank(&normalized), normalized);
                if selected
                    .as_ref()
                    .is_none_or(|current| candidate.0 > current.0)
                {
                    selected = Some(candidate);
                }
            }
            if let Some((_, quality_tier)) = selected {
                out.push(TitleQualitySummary {
                    title_id: title_id.clone(),
                    quality_tier,
                });
            }
        }

        Ok(out)
    }

    async fn list_title_movie_media_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        Ok(Vec::new())
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        let store = self.store.lock().await;
        let mut out = Vec::new();
        for title_id in title_ids {
            for entry in store.iter().filter(|entry| &entry.title_id == title_id) {
                let Some(label) = entry.quality_label.as_ref() else {
                    continue;
                };
                let normalized = label.trim().to_ascii_uppercase();
                if normalized.is_empty() {
                    continue;
                }
                out.push(CutoffUnmetQualitySummary {
                    title_id: title_id.clone(),
                    episode_id: entry.episode_id.clone(),
                    season_number: None,
                    episode_number: None,
                    quality_tier: normalized,
                });
            }
        }

        Ok(out)
    }

    async fn list_title_episode_progress_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        Ok(Vec::new())
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        Ok(Vec::new())
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.scan_status = "scanned".to_string();
        entry.video_codec = analysis.video_codec;
        entry.video_width = analysis.video_width;
        entry.video_height = analysis.video_height;
        entry.video_bitrate_kbps = analysis.video_bitrate_kbps;
        entry.video_bit_depth = analysis.video_bit_depth;
        entry.video_hdr_format = analysis.video_hdr_format;
        entry.video_frame_rate = analysis.video_frame_rate;
        entry.video_profile = analysis.video_profile;
        entry.audio_codec = analysis.audio_codec;
        entry.audio_channels = analysis.audio_channels;
        entry.audio_bitrate_kbps = analysis.audio_bitrate_kbps;
        entry.audio_languages = analysis.audio_languages;
        entry.audio_streams = analysis.audio_streams;
        entry.subtitle_languages = analysis.subtitle_languages;
        entry.subtitle_codecs = analysis.subtitle_codecs;
        entry.subtitle_streams = analysis.subtitle_streams;
        entry.has_multiaudio = analysis.has_multiaudio;
        entry.duration_seconds = analysis.duration_seconds;
        entry.num_chapters = analysis.num_chapters;
        entry.container_format = analysis.container_format;
        Ok(())
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.size_bytes = size_bytes;
        entry.source_signature_scheme = source_signature_scheme;
        entry.source_signature_value = source_signature_value;
        Ok(())
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.file_path = file_path.to_string();
        Ok(())
    }

    async fn set_media_file_roles_for_title(
        &self,
        title_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        let additional_ids = additional_file_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut list = self.store.lock().await;
        let mut updated = 0usize;
        for entry in list.iter_mut().filter(|entry| entry.title_id == title_id) {
            if entry.id == primary_file_id {
                entry.role = crate::MediaFileRole::Primary;
                updated += 1;
            } else if additional_ids.contains(entry.id.as_str()) {
                entry.role = crate::MediaFileRole::Additional;
                updated += 1;
            }
        }
        if updated != additional_ids.len() + 1 {
            return Err(AppError::NotFound(format!(
                "media files for title {title_id}"
            )));
        }
        Ok(())
    }

    async fn set_media_file_roles_for_episode(
        &self,
        title_id: &str,
        episode_id: &str,
        primary_file_id: &str,
        additional_file_ids: &[String],
    ) -> AppResult<()> {
        let additional_ids = additional_file_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut list = self.store.lock().await;
        let mut updated = 0usize;
        for entry in list.iter_mut().filter(|entry| {
            entry.title_id == title_id && entry.episode_id.as_deref() == Some(episode_id)
        }) {
            if entry.id == primary_file_id {
                entry.role = crate::MediaFileRole::Primary;
                updated += 1;
            } else if additional_ids.contains(entry.id.as_str()) {
                entry.role = crate::MediaFileRole::Additional;
                updated += 1;
            }
        }
        if updated != additional_ids.len() + 1 {
            return Err(AppError::NotFound(format!(
                "media files for episode {episode_id}"
            )));
        }
        Ok(())
    }

    async fn mark_scan_failed(&self, file_id: &str, _error: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.scan_status = "failed".to_string();
        Ok(())
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|entry| entry.id == file_id)
            .cloned())
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|entry| entry.file_path == file_path)
            .cloned())
    }
    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let position = list
            .iter()
            .position(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        list.remove(position);
        Ok(())
    }
}

/// Keeps every per-file import artifact so tests can assert what each file in a
/// pack was recorded as (imported / rejected / already present) and why.
#[derive(Default, Clone)]
pub(super) struct RecordingImportArtifactRepo {
    pub(super) artifacts: Arc<Mutex<Vec<crate::ImportArtifact>>>,
}

impl RecordingImportArtifactRepo {
    pub(super) async fn artifacts_for_file(&self, file_name: &str) -> Vec<crate::ImportArtifact> {
        let normalized = file_name.to_ascii_lowercase();
        self.artifacts
            .lock()
            .await
            .iter()
            .filter(|artifact| artifact.normalized_file_name == normalized)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl crate::ImportArtifactRepository for RecordingImportArtifactRepo {
    async fn insert_artifact(&self, artifact: crate::ImportArtifact) -> AppResult<()> {
        self.artifacts.lock().await.push(artifact);
        Ok(())
    }

    async fn list_by_source_identity(
        &self,
        identity: &ClientJobLocator,
    ) -> AppResult<Vec<crate::ImportArtifact>> {
        Ok(self
            .artifacts
            .lock()
            .await
            .iter()
            .filter(|artifact| &artifact.source_identity() == identity)
            .cloned()
            .collect())
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &ClientJobLocator,
        result: &str,
    ) -> AppResult<u64> {
        Ok(self
            .list_by_source_identity(identity)
            .await?
            .iter()
            .filter(|artifact| artifact.result == result)
            .count() as u64)
    }
}

#[derive(Default, Clone)]
pub(super) struct TrackingImportRepo {
    pub(super) records: Arc<Mutex<Vec<ImportRecord>>>,
    pub(super) identities: ImportIdentities,
    pub(super) manual_import_selection: Arc<Mutex<Option<crate::ManualImportSelection>>>,
    pub(super) manual_import_selection_consume_calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl ImportRepository for TrackingImportRepo {
    async fn get_manual_import_selection(
        &self,
        selection_id: &str,
        actor_user_id: &str,
    ) -> AppResult<Option<crate::ManualImportSelection>> {
        Ok(self
            .manual_import_selection
            .lock()
            .await
            .clone()
            .filter(|selection| {
                selection.id == selection_id && selection.actor_user_id == actor_user_id
            }))
    }

    async fn consume_manual_import_selection(
        &self,
        selection_id: &str,
        actor_user_id: &str,
        _candidate_ids: &[String],
    ) -> AppResult<Option<crate::ManualImportSelection>> {
        self.manual_import_selection_consume_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self
            .manual_import_selection
            .lock()
            .await
            .clone()
            .filter(|selection| {
                selection.id == selection_id && selection.actor_user_id == actor_user_id
            }))
    }

    async fn queue_import_request(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        self.queue_import_request_with_identity(source_identity, import_type, payload_json, None)
            .await
    }

    async fn queue_import_request_with_identity(
        &self,
        source_identity: ClientJobLocator,
        import_type: String,
        payload_json: String,
        submission_identity: Option<DownloadSubmissionIdentity>,
    ) -> AppResult<String> {
        let download_id = submission_identity.as_ref().and_then(|identity| {
            identity
                .download_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
        if let Some(download_id) = download_id.as_deref() {
            let records = self.records.lock().await;
            if let Some(record) = records.iter().rev().find(|record| {
                record.status.is_active()
                    && record.source_client_id.as_deref().unwrap_or("")
                        == source_identity.client_id_or_empty()
                    && record.source_system == source_identity.client_type
                    && record.download_id.as_deref() == Some(download_id)
            }) {
                return Ok(record.id.clone());
            }
        }
        let id = Id::new().0;
        let now = Utc::now().to_rfc3339();
        self.records.lock().await.push(ImportRecord {
            id: id.clone(),
            source_client_id: source_identity.client_id.clone(),
            source_system: source_identity.client_type,
            source_ref: source_identity.item_id,
            import_type: ImportType::parse(&import_type).unwrap_or(ImportType::ManualImport),
            status: ImportStatus::Pending,
            payload_json,
            result_json: None,
            download_id,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: now.clone(),
            updated_at: now,
        });
        if let Some(submission_identity) = submission_identity {
            self.identities
                .lock()
                .await
                .insert(id.clone(), submission_identity);
        }
        Ok(id)
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .find(|record| record.id == id)
            .cloned())
    }

    async fn update_import_status(
        &self,
        id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let mut records = self.records.lock().await;
        let record = records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("import record {id}")))?;
        record.status = status;
        record.result_json = result_json;
        if record.started_at.is_none() {
            record.started_at = Some(now.clone());
        }
        if status.is_terminal() {
            record.finished_at = Some(now.clone());
            record.import_transfer_phase = None;
        }
        record.updated_at = now;
        Ok(())
    }

    async fn update_import_transfer_progress(
        &self,
        id: &str,
        phase: scryer_domain::ImportTransferPhase,
        bytes: i64,
        total_bytes: i64,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let mut records = self.records.lock().await;
        let record = records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("import record {id}")))?;
        record.import_transfer_phase = Some(phase);
        record.import_transfer_bytes = Some(bytes.max(0));
        record.import_transfer_total_bytes = Some(total_bytes.max(bytes));
        if record.import_transfer_started_at.is_none() {
            record.import_transfer_started_at = Some(now.clone());
        }
        record.import_transfer_updated_at = Some(now.clone());
        record.updated_at = now;
        Ok(())
    }

    async fn recover_stale_processing_imports(&self, _stale_seconds: i64) -> AppResult<u64> {
        Ok(0)
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        _import_type: ImportType,
        _stale_seconds: i64,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .filter(|record| record.status.is_active())
            .cloned()
            .collect())
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .filter(|record| record.import_type == import_type && record.status.is_active())
            .cloned()
            .collect())
    }

    async fn list_imports_for_identities(
        &self,
        identities: &[ClientJobLocator],
    ) -> AppResult<Vec<ImportRecord>> {
        let records = self.records.lock().await;
        Ok(records
            .iter()
            .rev()
            .filter(|record| {
                identities.iter().any(|identity| {
                    record.source_client_id.as_deref().unwrap_or("")
                        == identity.client_id_or_empty()
                        && record.source_system == identity.client_type
                        && record.source_ref == identity.item_id
                })
            })
            .cloned()
            .collect())
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        let mut records = self.records.lock().await.clone();
        records.reverse();
        records.truncate(limit);
        Ok(records)
    }
}

#[async_trait]
impl UserRepository for MockUserRepo {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let users = self.store.lock().await;
        Ok(users.iter().find(|user| user.username == username).cloned())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        self.get_by_id_calls.fetch_add(1, Ordering::SeqCst);
        let users = self.store.lock().await;
        Ok(users.iter().find(|user| user.id == id).cloned())
    }

    async fn create(&self, user: User) -> AppResult<User> {
        self.store.lock().await.push(user.clone());
        Ok(user)
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        self.list_all_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.store.lock().await.clone())
    }

    async fn auth_session_version(&self, user_id: &str) -> AppResult<Option<String>> {
        Ok(self
            .auth_session_versions
            .lock()
            .await
            .get(user_id)
            .cloned())
    }

    async fn update_password_and_invalidate_sessions(
        &self,
        id: &str,
        password_hash: String,
        password_change_required: bool,
        auth_session_version: &str,
    ) -> AppResult<User> {
        let user = {
            let mut users = self.store.lock().await;
            let user = users
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
            user.password_hash = Some(password_hash);
            user.password_change_required = password_change_required;
            user.clone()
        };
        self.auth_session_versions
            .lock()
            .await
            .insert(id.to_string(), auth_session_version.to_string());
        Ok(user)
    }

    async fn complete_required_password_change(
        &self,
        id: &str,
        password_hash: String,
        expected_auth_session_version: &Option<String>,
        auth_session_version: &str,
    ) -> AppResult<User> {
        if self.auth_session_versions.lock().await.get(id).cloned()
            != *expected_auth_session_version
        {
            return Err(AppError::Unauthorized(
                "authentication session was invalidated".into(),
            ));
        }
        let mut users = self.store.lock().await;
        let user = users
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
        if !user.password_change_required {
            return Err(AppError::Unauthorized(
                "password change is no longer required".into(),
            ));
        }
        user.password_hash = Some(password_hash);
        user.password_change_required = false;
        let user = user.clone();
        self.auth_session_versions
            .lock()
            .await
            .insert(id.to_string(), auth_session_version.to_string());
        Ok(user)
    }

    async fn update_login_status_and_rotate_session(
        &self,
        id: &str,
        status: scryer_domain::UserLoginStatus,
        auth_session_version: &str,
    ) -> AppResult<User> {
        let mut users = self.store.lock().await;
        let user = users
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
        user.set_login_status(status);
        let user = user.clone();
        self.auth_session_versions
            .lock()
            .await
            .insert(id.to_string(), auth_session_version.to_string());
        Ok(user)
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let mut users = self.store.lock().await;
        let index = users
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
        users.remove(index);
        self.auth_session_versions.lock().await.remove(id);
        Ok(())
    }
}
