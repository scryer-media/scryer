use super::*;

pub fn from_delete_preview(preview: DeletePreview) -> DeletePreviewPayload {
    DeletePreviewPayload {
        fingerprint: preview.fingerprint,
        total_file_count: preview.total_file_count,
        media_count: preview.media_count,
        subtitle_count: preview.subtitle_count,
        image_count: preview.image_count,
        other_count: preview.other_count,
        directory_count: preview.directory_count,
        requires_typed_confirmation: preview.requires_typed_confirmation,
        typed_confirmation_prompt: preview.typed_confirmation_prompt,
        target_label: preview.target_label,
        sample_paths: preview.sample_paths,
    }
}

pub fn from_delete_titles_preview(
    preview: scryer_application::DeleteTitlesPreview,
) -> DeleteTitlesPreviewPayload {
    let failed_count = preview
        .items
        .iter()
        .filter(|item| item.error.is_some())
        .count() as i32;
    DeleteTitlesPreviewPayload {
        preview: from_delete_preview(preview.preview),
        items: preview
            .items
            .into_iter()
            .map(|item| DeleteTitlePreviewResultPayload {
                title_id: item.title_id.into(),
                preview: item.preview.map(from_delete_preview),
                error: item.error,
            })
            .collect(),
        failed_count,
    }
}

pub fn from_delete_episode_files_preview(
    preview: scryer_application::DeleteEpisodeFilesPreview,
) -> DeleteEpisodeFilesPreviewPayload {
    let failed_count = preview
        .items
        .iter()
        .filter(|item| item.error.is_some())
        .count() as i32;
    DeleteEpisodeFilesPreviewPayload {
        preview: from_delete_preview(preview.preview),
        items: preview
            .items
            .into_iter()
            .map(|item| DeleteEpisodeFilePreviewResultPayload {
                file_id: item.file_id.into(),
                episode_id: item.episode_id.into(),
                episode_ids: item.episode_ids.into_iter().map(Into::into).collect(),
                preview: item.preview.map(from_delete_preview),
                error: item.error,
            })
            .collect(),
        file_count: preview.file_count,
        failed_count,
    }
}

pub fn from_delete_episode_files_job_accepted(
    accepted: scryer_application::DeleteEpisodeFilesJobAccepted,
) -> DeleteEpisodeFilesPayload {
    DeleteEpisodeFilesPayload {
        job_run: crate::mappers::from_job_run(accepted.job_run),
        accepted_file_ids: accepted
            .accepted_file_ids
            .into_iter()
            .map(Into::into)
            .collect(),
    }
}

pub fn from_search_result(result: IndexerSearchResult) -> IndexerSearchResultPayload {
    let facts = result.release_facts();
    let int = |value: Option<i64>| value.and_then(|value| i32::try_from(value).ok());

    IndexerSearchResultPayload {
        source: result.source,
        indexer_id: result.indexer_id.map(Into::into),
        title: result.title,
        link: result.link,
        download_url: result.download_url,
        source_kind: result
            .source_kind
            .map(DownloadSourceKindValue::from_application),
        size_bytes: result.size_bytes.map(Long::from),
        published_at: result
            .published_at
            .as_deref()
            .and_then(scryer_domain::publish_time::parse_publish_time),
        thumbs_up: result.thumbs_up,
        thumbs_down: result.thumbs_down,
        grabs: result.indexer_grabs.map(|grabs| grabs as i32),
        parsed_release: result.parsed_release_metadata.map(from_parsed_release),
        quality_profile_decision: result
            .quality_profile_decision
            .map(from_quality_profile_decision),
        seeders: int(facts.seeders),
        peers: int(facts.peers),
        leechers: int(facts.leechers),
        info_hash: facts.info_hash,
        freeleech: facts.freeleech,
        download_volume_factor: facts.download_volume_factor,
        upload_volume_factor: facts.upload_volume_factor,
        minimum_seed_ratio: facts.minimum_seed_ratio,
        minimum_seed_time_minutes: int(facts.minimum_seed_time_minutes),
        indexer_flags: facts.indexer_flags,
        languages: facts.languages,
        subtitles: facts.subtitles,
        files: int(facts.files),
        year: facts.year,
        poster_url: facts.poster_url,
        candidate_token: result.candidate_token,
        queue_scope: result.queue_scope.map(from_submission_scope),
        auto_eligible: result.auto_eligible,
        auto_decision_code: result.auto_decision_code,
        auto_decision_summary: result.auto_decision_summary,
    }
}

pub fn from_submission_scope(scope: SubmissionScope) -> QueueDownloadScopePayload {
    match scope {
        SubmissionScope::Episode { episode_id } => {
            QueueDownloadScopePayload::episode(episode_id.into())
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            QueueDownloadScopePayload::EpisodeSet(EpisodeSetScopePayload {
                episode_ids: episode_ids.into_iter().map(Into::into).collect(),
            })
        }
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => QueueDownloadScopePayload::SeriesMovie(SeriesMovieScopePayload {
            series_movie_link_id: series_movie_link_id.into(),
        }),
        SubmissionScope::Collection { collection_id } => {
            QueueDownloadScopePayload::Collection(CollectionScopePayload {
                collection_id: collection_id.into(),
            })
        }
        SubmissionScope::Title => {
            QueueDownloadScopePayload::Title(TitleScopePayload { whole_title: true })
        }
        SubmissionScope::Orphan => {
            QueueDownloadScopePayload::Orphan(OrphanScopePayload { orphaned: true })
        }
    }
}

pub fn from_title_release_blocklist_entry(
    entry: TitleReleaseBlocklistEntry,
) -> TitleReleaseBlocklistEntryPayload {
    TitleReleaseBlocklistEntryPayload {
        id: entry.id.into(),
        release_name: entry.release_name,
        error_message: entry.error_message,
        attempted_at: parse_required_datetime(
            &entry.attempted_at,
            "title release blocklist attempted_at",
        ),
    }
}

pub fn from_quality_profile_decision(
    decision: QualityProfileDecision,
) -> QualityProfileDecisionPayload {
    QualityProfileDecisionPayload {
        audio_language: AudioLanguageRankPayload {
            preferred_rank: i32::from(decision.language.matched.preferred_rank),
            preferred_count: i32::from(decision.language.preferred_count),
            optional_hits: i32::from(decision.language.matched.optional_hits),
            wanted: decision.language.wanted,
        },
        decided_by: decision
            .decided_by
            .map(ReleaseDecidingStepValue::from_application),
        allowed: decision.allowed,
        block_codes: decision.block_codes,
        release_score: decision.release_score,
        preference_score: decision.preference_score,
        scoring_log: decision
            .scoring_log
            .into_iter()
            .map(|e: ScoringEntry| {
                let (source, rule_set_name) = match e.source {
                    ScoringSource::Builtin => ("builtin".to_string(), None),
                    ScoringSource::UserRule { id, name } => (format!("user:{id}"), Some(name)),
                    ScoringSource::SystemRule { id, name } => (format!("system:{id}"), Some(name)),
                };
                ScoringEntryPayload {
                    code: e.code,
                    delta: e.delta,
                    kind: e.kind.as_str().to_string(),
                    source,
                    rule_set_name,
                }
            })
            .collect(),
    }
}

pub fn from_parsed_release(result: ParsedReleaseMetadata) -> ParsedReleasePayload {
    ParsedReleasePayload {
        raw_title: result.raw_title,
        normalized_title: result.normalized_title,
        year: result.year,
        release_group: result.release_group,
        quality: result.quality,
        source: result.source.map(|source| source.to_string()),
        video_codec: result.video_codec.map(|codec| codec.to_string()),
        video_encoding: result.video_encoding,
        stereoscopy: result.stereoscopy.map(from_stereoscopy),
        audio: result.audio.map(|codec| codec.to_string()),
        audio_channels: result.audio_channels,
        audio_languages: result.languages_audio,
        is_dual_audio: result.is_dual_audio,
        is_atmos: result.is_atmos,
        is_dolby_vision: result.is_dolby_vision,
        detected_hdr: result.detected_hdr,
        is_proper_upload: result.is_proper_upload,
        revision: result.revision,
        is_remux: result.is_remux,
        is_bd_disk: result.is_bd_disk,
        is_ai_enhanced: result.is_ai_enhanced,
        parse_confidence: result.parse_confidence,
        parse_hints: result.parse_hints,
        episode: result.episode.map(from_parsed_episode),
    }
}

fn from_stereoscopy(value: scryer_application::ParsedStereoscopy) -> ParsedStereoscopyPayload {
    use scryer_application::{StereoEncoding, StereoLayout, StereoPresentation, StereoSampling};
    ParsedStereoscopyPayload {
        presentation: match value.presentation {
            StereoPresentation::TwoD => StereoPresentationValue::TwoD,
            StereoPresentation::ThreeD => StereoPresentationValue::ThreeD,
            StereoPresentation::Mixed2d3d => StereoPresentationValue::Mixed2d3d,
        },
        layout: value.layout.map(|layout| match layout {
            StereoLayout::SideBySide => StereoLayoutValue::SideBySide,
            StereoLayout::TopBottom => StereoLayoutValue::TopBottom,
            StereoLayout::FrameSequential => StereoLayoutValue::FrameSequential,
            StereoLayout::RowInterleaved => StereoLayoutValue::RowInterleaved,
            StereoLayout::ColumnInterleaved => StereoLayoutValue::ColumnInterleaved,
            StereoLayout::Checkerboard => StereoLayoutValue::Checkerboard,
            StereoLayout::Anaglyph => StereoLayoutValue::Anaglyph,
        }),
        sampling: value.sampling.map(|sampling| match sampling {
            StereoSampling::Half => StereoSamplingValue::Half,
            StereoSampling::Full => StereoSamplingValue::Full,
        }),
        encoding: value.encoding.map(|encoding| match encoding {
            StereoEncoding::Mvc => StereoEncodingValue::Mvc,
        }),
    }
}

pub fn from_parsed_episode(episode: ParsedEpisodeMetadata) -> ParsedEpisodePayload {
    ParsedEpisodePayload {
        season: episode.season.map(|value| value as i32),
        episode_numbers: episode
            .episode_numbers
            .into_iter()
            .map(|value| value as i32)
            .collect(),
    }
}

pub fn from_active_import_stream(
    stream: scryer_application::ActiveImportStream,
) -> ActiveImportStreamPayload {
    let phase = match stream.phase {
        scryer_application::ActiveImportStreamPhase::Queued => ActiveImportStreamPhaseValue::Queued,
        scryer_application::ActiveImportStreamPhase::Extracting => {
            ActiveImportStreamPhaseValue::Extracting
        }
        scryer_application::ActiveImportStreamPhase::Placing => {
            ActiveImportStreamPhaseValue::Placing
        }
        scryer_application::ActiveImportStreamPhase::Copying => {
            ActiveImportStreamPhaseValue::Copying
        }
        scryer_application::ActiveImportStreamPhase::Finalizing => {
            ActiveImportStreamPhaseValue::Finalizing
        }
        scryer_application::ActiveImportStreamPhase::Verifying => {
            ActiveImportStreamPhaseValue::Verifying
        }
    };
    let cancellable = stream.cancellable();
    ActiveImportStreamPayload {
        id: stream.id.into(),
        import_id: stream.import_id.into(),
        library_id: stream.library_id.into(),
        facet: MediaFacetValue::from_domain(stream.facet),
        source_path: stream.source_path,
        destination_path: stream.destination_path,
        phase,
        bytes: Long::from(i64::try_from(stream.bytes).unwrap_or(i64::MAX)),
        total_bytes: Long::from(i64::try_from(stream.total_bytes).unwrap_or(i64::MAX)),
        queued_at: stream.queued_at,
        started_at: stream.started_at,
        updated_at: stream.updated_at,
        cancellable,
        cancellation_requested: stream.cancellation_requested,
    }
}

pub fn from_download_queue_item(item: DownloadQueueItem) -> DownloadQueueItemPayload {
    let display_state = DownloadDisplayStateValue::from_application(
        scryer_application::derive_download_queue_display_state(&item),
    );
    let seeding_state = scryer_application::derive_download_seeding_state(&item)
        .map(DownloadSeedingStateValue::from_application);
    let seeding = item.seeding.clone().unwrap_or_default();
    let actions = scryer_application::derive_download_queue_import_actions(&item);
    DownloadQueueItemPayload {
        password_failure_code: item
            .password_failure()
            .map(DownloadPasswordFailureCodeValue::from_domain),
        import_actions: DownloadImportActionsPayload {
            manual_import_interactive: actions.manual_import_interactive,
            manual_import_direct: actions.manual_import_direct,
            assign_title: actions.assign_title,
            ignore: actions.ignore,
            mark_failed: actions.mark_failed,
            dismiss_import_warning: actions.dismiss_import_warning,
            resume_import_tracking: actions.resume_import_tracking,
        },
        seeding_state,
        seed_ratio: seeding.seed_ratio,
        seed_ratio_goal: seeding.seed_goal_ratio,
        seed_time_seconds: seeding.seed_time_seconds.map(Long::from),
        seed_time_goal_seconds: seeding.seed_goal_seconds.map(Long::from),
        is_private: seeding.is_private,
        id: item.id.into(),
        title_id: item.title_id.map(Into::into),
        episode_id: item.episode_id.map(Into::into),
        title_name: item.title_name,
        facet: item.facet.as_deref().and_then(MediaFacetValue::parse),
        is_scryer_origin: item.is_scryer_origin,
        source_provider: item.source_provider,
        tracked_state: item
            .tracked_state
            .map(TrackedDownloadStateValue::from_domain),
        tracked_status: item
            .tracked_status
            .map(TrackedDownloadStatusValue::from_domain),
        tracked_status_messages: item.tracked_status_messages,
        tracked_match_type: item
            .tracked_match_type
            .map(TitleMatchTypeValue::from_domain),
        client_id: item.client_id.into(),
        client_name: item.client_name,
        client_type: item.client_type,
        state: DownloadQueueStateValue::from_domain(item.state),
        display_state,
        progress_percent: i32::from(item.progress_percent),
        import_transfer_phase: item.import_transfer_phase.map(Into::into),
        import_transfer_bytes: item.import_transfer_bytes.map(Long::from),
        import_transfer_total_bytes: item.import_transfer_total_bytes.map(Long::from),
        import_transfer_started_at: parse_optional_datetime(
            item.import_transfer_started_at,
            "download queue import_transfer_started_at",
        ),
        import_transfer_updated_at: parse_optional_datetime(
            item.import_transfer_updated_at,
            "download queue import_transfer_updated_at",
        ),
        size_bytes: item.size_bytes.map(Long::from),
        remaining_seconds: item
            .remaining_seconds
            .and_then(|value| i32::try_from(value).ok()),
        queued_at: parse_optional_datetime(item.queued_at, "download queue queued_at"),
        last_updated_at: parse_optional_datetime(
            item.last_updated_at,
            "download queue last_updated_at",
        ),
        attention_required: item.attention_required,
        attention_reason: item.attention_reason,
        attention_reason_code: item.attention_reason_code,
        download_client_item_id: item.download_client_item_id,
        download_id: item.download_id,
        import_status: item.import_status.map(ImportStatusValue::from_domain),
        import_error_code: item
            .import_error_code
            .map(ImportErrorCodeValue::from_domain),
        import_error_message: item.import_error_message,
        imported_at: parse_optional_datetime(item.imported_at, "download queue imported_at"),
        delete_status: item
            .delete_status
            .map(DownloadQueueDeleteStatusValue::from_domain),
        delete_error_message: item.delete_error_message,
    }
}

pub(super) fn extract_tag_string(tags: &[String], prefix: &str) -> Option<String> {
    tags.iter().find_map(|tag| {
        tag.strip_prefix(prefix).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    })
}

pub(super) fn extract_tag_bool(tags: &[String], prefix: &str) -> Option<bool> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(prefix))
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
}

pub fn from_wanted_item(
    item: scryer_application::AcquisitionScopeState,
) -> scryer_application::AppResult<WantedItemPayload> {
    from_wanted_item_selecting(item, ExplanationSelection::ALL)
}

/// [`from_wanted_item`] that parses the latest decision's explanation only
/// for the fields `selection` names.
pub fn from_wanted_item_selecting(
    item: scryer_application::AcquisitionScopeState,
    selection: ExplanationSelection,
) -> scryer_application::AppResult<WantedItemPayload> {
    let source_provider = source_provider_from_grabbed_release(item.grabbed_release.as_deref());
    Ok(WantedItemPayload {
        id: item.id.into(),
        title_id: item.title_id.into(),
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: item.title_facet,
        library_id: item.library_id.map(Into::into),
        library_name: item.library_name,
        library_slug: item.library_slug,
        episode_id: item.episode_id.map(Into::into),
        collection_id: item.collection_id.map(Into::into),
        season_number: item.season_number,
        episode_number: item.episode_number,
        media_type: WantedMediaTypeValue::parse(&item.media_type).ok_or_else(|| {
            scryer_application::AppError::Repository(format!(
                "invalid wanted item media_type '{}'",
                item.media_type
            ))
        })?,
        last_search_at: parse_optional_datetime(item.last_search_at, "wanted item last_search_at"),
        status: WantedStatusValue::from_application(item.status),
        grabbed_release: item.grabbed_release,
        source_provider,
        current_score: item.landed_bar,
        latest_release_decision: item
            .latest_release_decision
            .map(|decision| from_release_decision_selecting(decision, selection))
            .transpose()?,
        standby_count: 0,
        mismatch_recovery_eligible: item.mismatch_recovery_eligible,
        // Relation-field state rows (`title.wantedItems`, `episode.wantedItem`) are
        // not the convergence display surface: the derived Missing /
        // Upgrades views carry live convergence; here we present a neutral default
        // so the shared payload stays well-typed off a bare state row.
        convergence_state: ConvergenceStateValue::Queued,
        indexers_covered: 0,
        indexers_routed: 0,
        recency_lane: RecencyLaneValue::Cold,
        created_at: parse_datetime(&item.created_at, "wanted item created_at")
            .map_err(scryer_application::AppError::Repository)?,
        updated_at: parse_datetime(&item.updated_at, "wanted item updated_at")
            .map_err(scryer_application::AppError::Repository)?,
    })
}

/// Map a derived Missing/Upgrades view row onto the shared `WantedItemPayload`
///. The payload `id` is the scope identity: the state-row id when a
/// state row exists, else the convergence scope key — so a derived target with no
/// state row is still addressable (pause/resume and the interactive search job both
/// accept a scope key). Convergence progress and the recency lane come from the
/// batched per-page derivation.
pub fn from_wanted_scope_view(
    view: scryer_application::WantedScopeView,
) -> scryer_application::AppResult<WantedItemPayload> {
    from_wanted_scope_view_selecting(view, ExplanationSelection::ALL)
}

/// [`from_wanted_scope_view`] that parses the latest decision's explanation
/// only for the fields `selection` names.
pub fn from_wanted_scope_view_selecting(
    view: scryer_application::WantedScopeView,
    selection: ExplanationSelection,
) -> scryer_application::AppResult<WantedItemPayload> {
    let id = view
        .state
        .as_ref()
        .map(|state| state.id.clone())
        .unwrap_or_else(|| view.scope_key.clone());
    let state = view.state;
    let grabbed_release = state
        .as_ref()
        .and_then(|state| state.grabbed_release.clone());
    let source_provider = source_provider_from_grabbed_release(grabbed_release.as_deref());
    Ok(WantedItemPayload {
        id: id.into(),
        title_id: view.title_id.into(),
        title_name: view.title_name,
        title_slug: view.title_slug,
        title_facet: Some(view.facet.as_str().to_string()),
        library_id: Some(view.library_id.into()),
        library_name: view.library_name,
        library_slug: view.library_slug,
        episode_id: view.episode_id.map(Into::into),
        collection_id: view.collection_id.map(Into::into),
        season_number: view.season_number,
        episode_number: view.episode_number,
        media_type: WantedMediaTypeValue::parse(&view.media_type).ok_or_else(|| {
            scryer_application::AppError::Repository(format!(
                "invalid wanted view media_type '{}'",
                view.media_type
            ))
        })?,
        last_search_at: state.as_ref().and_then(|state| {
            parse_optional_datetime(state.last_search_at.clone(), "wanted view last_search_at")
        }),
        status: state
            .as_ref()
            .map(|state| WantedStatusValue::from_application(state.status))
            .unwrap_or(WantedStatusValue::Wanted),
        grabbed_release,
        source_provider,
        // From the **view**: a scope with no persisted state row still has a
        // file occupying it, and that file still has a bar.
        current_score: view
            .landed_bar
            .or_else(|| state.as_ref().and_then(|state| state.landed_bar)),
        latest_release_decision: state
            .as_ref()
            .and_then(|state| state.latest_release_decision.clone())
            .map(|decision| from_release_decision_selecting(decision, selection))
            .transpose()?,
        standby_count: view.standby_count,
        mismatch_recovery_eligible: state
            .as_ref()
            .is_some_and(|state| state.mismatch_recovery_eligible),
        convergence_state: convergence_state_value(view.convergence.state),
        indexers_covered: view.convergence.indexers_covered,
        indexers_routed: view.convergence.indexers_routed,
        recency_lane: if view.is_hot {
            RecencyLaneValue::Hot
        } else {
            RecencyLaneValue::Cold
        },
        created_at: state
            .as_ref()
            .and_then(|state| parse_datetime(&state.created_at, "wanted view created_at").ok())
            .unwrap_or_else(chrono::Utc::now),
        updated_at: state
            .as_ref()
            .and_then(|state| parse_datetime(&state.updated_at, "wanted view updated_at").ok())
            .unwrap_or_else(chrono::Utc::now),
    })
}

fn source_provider_from_grabbed_release(grabbed_release: Option<&str>) -> Option<String> {
    let grabbed_release = serde_json::from_str::<Value>(grabbed_release?).ok()?;
    let source_provider = grabbed_release
        .get("source_provider")
        .or_else(|| grabbed_release.get("indexer"))
        .and_then(Value::as_str)?;
    let source_provider = source_provider.trim();
    (!source_provider.is_empty()
        && !source_provider.contains([':', '/', '\\', '?', '#', '@', '|', '=']))
    .then(|| source_provider.to_string())
}

fn convergence_state_value(
    state: scryer_application::WantedConvergenceState,
) -> ConvergenceStateValue {
    match state {
        scryer_application::WantedConvergenceState::Queued => ConvergenceStateValue::Queued,
        scryer_application::WantedConvergenceState::Searching => ConvergenceStateValue::Searching,
        scryer_application::WantedConvergenceState::Converged => ConvergenceStateValue::Converged,
        scryer_application::WantedConvergenceState::Deferred => ConvergenceStateValue::Deferred,
    }
}

pub fn from_release_decision(
    decision: scryer_application::ReleaseDecision,
) -> scryer_application::AppResult<ReleaseDecisionPayload> {
    from_release_decision_selecting(decision, ExplanationSelection::ALL)
}

/// Which of a release decision's explanation-derived fields a query reads.
/// A field left out is returned as null and its explanation is not parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplanationSelection {
    pub explanation_json: bool,
    pub decided_by: bool,
}

impl ExplanationSelection {
    pub const ALL: Self = Self {
        explanation_json: true,
        decided_by: true,
    };

    /// The explanation fields `decision`, the look-ahead at a
    /// `ReleaseDecisionPayload` selection, reads — through fragments and
    /// aliases too.
    pub fn of(decision: async_graphql::Lookahead<'_>) -> Self {
        Self {
            explanation_json: decision.field("explanationJson").exists(),
            decided_by: decision.field("decidedBy").exists(),
        }
    }
}

pub fn from_release_decision_selecting(
    decision: scryer_application::ReleaseDecision,
    selection: ExplanationSelection,
) -> scryer_application::AppResult<ReleaseDecisionPayload> {
    // Parsed once for both fields that read it.
    let explanation = (selection.explanation_json || selection.decided_by)
        .then_some(decision.explanation_json.as_ref())
        .flatten()
        .map(|explanation| serde_json::from_str::<serde_json::Value>(explanation.as_str()).ok());
    let decided_by = if selection.decided_by {
        explanation
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(recorded_deciding_step)
    } else {
        None
    };
    let explanation_json = if selection.explanation_json {
        explanation.map(|value| async_graphql::Json(value.unwrap_or(serde_json::Value::Null)))
    } else {
        None
    };
    Ok(ReleaseDecisionPayload {
        id: decision.id.into(),
        wanted_item_id: decision.wanted_item_id.into(),
        title_id: decision.title_id.into(),
        release_title: decision.release_title,
        // Rows written before the ledger started redacting on write still
        // hold the indexer key.
        release_url: scryer_application::url_redaction::redact_optional_url_credentials(
            decision.release_url,
        ),
        release_size_bytes: decision.release_size_bytes.map(Long::from),
        decision_code: decision.decision_code,
        candidate_score: decision.candidate_score,
        current_score: decision.current_score,
        score_delta: decision.score_delta,
        decided_by,
        explanation_json,
        created_at: parse_datetime(&decision.created_at, "release decision created_at")
            .map_err(scryer_application::AppError::Repository)?,
    })
}

/// The deciding step a release decision's explanation records under
/// `auto_decision.decided_by`; null for decisions recorded before it was.
fn recorded_deciding_step(explanation: &serde_json::Value) -> Option<ReleaseDecidingStepValue> {
    crate::conversions::deciding_step_from_recorded(
        explanation
            .get("auto_decision")?
            .get("decided_by")?
            .as_str(),
    )
}

pub fn from_decision_code_count(
    item: scryer_application::DecisionCodeCount,
) -> DecisionCodeCountPayload {
    DecisionCodeCountPayload {
        code: item.code,
        count: item.count,
    }
}

pub fn from_wanted_status_count(
    item: scryer_application::WantedStatusCount,
) -> WantedStatusCountPayload {
    WantedStatusCountPayload {
        status: scryer_application::AcquisitionScopeStatus::parse(&item.status)
            .map(WantedStatusValue::from_application)
            .unwrap_or(WantedStatusValue::Wanted),
        count: item.count,
    }
}

pub fn from_pending_release_status_count(
    item: scryer_application::PendingReleaseStatusCount,
) -> PendingReleaseStatusCountPayload {
    PendingReleaseStatusCountPayload {
        status: scryer_application::PendingReleaseStatus::parse(&item.status)
            .map(PendingReleaseStatusValue::from_application)
            .unwrap_or(PendingReleaseStatusValue::Waiting),
        count: item.count,
    }
}

pub fn from_title_acquisition_diagnostics(
    value: scryer_application::TitleAcquisitionDiagnostics,
) -> scryer_application::AppResult<TitleAcquisitionDiagnosticsPayload> {
    from_title_acquisition_diagnostics_selecting(value, ExplanationSelection::ALL)
}

/// [`from_title_acquisition_diagnostics`] that parses the recent decisions'
/// explanations only for the fields `selection` names.
pub fn from_title_acquisition_diagnostics_selecting(
    value: scryer_application::TitleAcquisitionDiagnostics,
    selection: ExplanationSelection,
) -> scryer_application::AppResult<TitleAcquisitionDiagnosticsPayload> {
    Ok(TitleAcquisitionDiagnosticsPayload {
        recent_decisions: value
            .recent_decisions
            .into_iter()
            .map(|decision| from_release_decision_selecting(decision, selection))
            .collect::<scryer_application::AppResult<Vec<_>>>()?,
        decision_counts: value
            .decision_counts
            .into_iter()
            .map(from_decision_code_count)
            .collect(),
        wanted_status_counts: value
            .wanted_status_counts
            .into_iter()
            .map(from_wanted_status_count)
            .collect(),
        pending_release_counts: value
            .pending_release_counts
            .into_iter()
            .map(from_pending_release_status_count)
            .collect(),
        mismatch_recovery_eligible_count: value.mismatch_recovery_eligible_count,
        latest_decision_at: parse_optional_datetime(
            value.latest_decision_at,
            "title acquisition latest_decision_at",
        ),
        latest_wanted_search_at: parse_optional_datetime(
            value.latest_wanted_search_at,
            "title acquisition latest_wanted_search_at",
        ),
    })
}

#[cfg(test)]
mod release_decision_tests {
    use super::*;

    fn decision(
        explanation: Option<scryer_application::ReleaseDecisionExplanation>,
    ) -> scryer_application::ReleaseDecision {
        scryer_application::ReleaseDecision {
            id: "decision-synthetic".into(),
            wanted_item_id: "wanted-synthetic".into(),
            title_id: "title-synthetic".into(),
            release_title: "Synthetic.Release.2020.1080p-GRP".into(),
            release_url: None,
            release_size_bytes: Some(1),
            decision_code: "accepted".into(),
            candidate_score: 1,
            current_score: None,
            score_delta: None,
            explanation_json: explanation,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    /// The mapping as it stood before the explanation was parsed once: one
    /// parse per field.
    fn reparsed_fields(
        explanation: Option<&str>,
    ) -> (Option<serde_json::Value>, Option<ReleaseDecidingStepValue>) {
        let decided_by = explanation
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .and_then(|value| {
                crate::conversions::deciding_step_from_recorded(
                    value.get("auto_decision")?.get("decided_by")?.as_str(),
                )
            });
        let explanation_json = explanation.map(|text| json_string_to_value(text.to_string()).0);
        (explanation_json, decided_by)
    }

    #[test]
    fn explanation_fields_map_as_they_did_with_one_parse_per_field() {
        let mut explanations = vec![
            None,
            Some(r#"{"auto_decision":{"decided_by":"quality"}}"#.to_string()),
            Some(r#"{"auto_decision":{"decided_by":"not_a_step"}}"#.to_string()),
            Some(r#"{"auto_decision":{"decided_by":7}}"#.to_string()),
            Some(r#"{"auto_decision":null}"#.to_string()),
            Some(r#"{"candidate":{"guid":"synthetic"},"auto_decision":{"code":"x"}}"#.to_string()),
            Some("not-json".to_string()),
            Some(String::new()),
            Some("[1,2]".to_string()),
        ];
        for step in [
            "score", "Score", "quality", "revision", "language", "protocol",
        ] {
            explanations.push(Some(format!(
                r#"{{"auto_decision":{{"decided_by":"{step}"}},"parsed":null}}"#
            )));
        }
        for text in explanations {
            let explanation = text
                .clone()
                .map(scryer_application::ReleaseDecisionExplanation::untrusted);
            let payload = from_release_decision(decision(explanation.clone())).unwrap();
            let (explanation_json, decided_by) = reparsed_fields(text.as_deref());
            assert_eq!(
                payload.explanation_json.map(|json| json.0),
                explanation_json
            );
            assert!(payload.decided_by == decided_by, "{text:?}");

            let unselected = from_release_decision_selecting(
                decision(explanation),
                ExplanationSelection {
                    explanation_json: false,
                    decided_by: false,
                },
            )
            .unwrap();
            assert!(unselected.explanation_json.is_none());
            assert!(unselected.decided_by.is_none());
        }
    }

    /// A decision-shaped object whose resolver records what
    /// [`ExplanationSelection::of`] reads from its own selection.
    struct ProbeQuery;

    struct ProbeDecision;

    #[async_graphql::Object]
    impl ProbeDecision {
        async fn decision_code(&self) -> &str {
            "accepted"
        }
        async fn explanation_json(&self) -> &str {
            "{}"
        }
        async fn decided_by(&self) -> &str {
            "score"
        }
    }

    type Seen = std::sync::Arc<std::sync::Mutex<Option<ExplanationSelection>>>;

    #[async_graphql::Object]
    impl ProbeQuery {
        async fn decision(&self, ctx: &async_graphql::Context<'_>) -> ProbeDecision {
            *ctx.data_unchecked::<Seen>().lock().unwrap() =
                Some(ExplanationSelection::of(ctx.look_ahead()));
            ProbeDecision
        }
    }

    /// Executes a query that needs no I/O to completion on this thread.
    fn selection_for(query: &str) -> ExplanationSelection {
        let seen = Seen::default();
        let schema = async_graphql::Schema::build(
            ProbeQuery,
            async_graphql::EmptyMutation,
            async_graphql::EmptySubscription,
        )
        .data(Seen::clone(&seen))
        .finish();
        let mut execution = std::pin::pin!(schema.execute(query));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let response = loop {
            if let std::task::Poll::Ready(response) = execution.as_mut().poll(&mut context) {
                break response;
            }
        };
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        seen.lock().unwrap().expect("probe resolver ran")
    }

    #[test]
    fn explanation_selection_reads_exactly_the_selected_explanation_fields() {
        let none = ExplanationSelection {
            explanation_json: false,
            decided_by: false,
        };
        let json_only = ExplanationSelection {
            explanation_json: true,
            decided_by: false,
        };
        let decided_by_only = ExplanationSelection {
            explanation_json: false,
            decided_by: true,
        };
        for (query, expected) in [
            ("{ decision { decisionCode } }", none),
            ("{ decision { decisionCode explanationJson } }", json_only),
            ("{ decision { step: decidedBy } }", decided_by_only),
            (
                "{ decision { ...Step } } fragment Step on ProbeDecision { decidedBy }",
                decided_by_only,
            ),
            (
                "{ decision { ... on ProbeDecision { explanationJson decidedBy } } }",
                ExplanationSelection::ALL,
            ),
        ] {
            assert_eq!(selection_for(query), expected, "{query}");
        }
    }
}

#[cfg(test)]
mod stereo_tests {
    use super::*;

    #[test]
    fn stereo_api_mapping_and_nullable_schema() {
        let parsed =
            scryer_application::parse_release_metadata("Title.2020.1080p.BluRay.3D.HSBS.x264");
        let payload = from_parsed_release(parsed);
        let stereo = payload.stereoscopy.unwrap();
        assert_eq!(stereo.presentation, StereoPresentationValue::ThreeD);
        assert_eq!(stereo.layout, Some(StereoLayoutValue::SideBySide));
        assert_eq!(stereo.sampling, Some(StereoSamplingValue::Half));
        assert_eq!(stereo.encoding, None);
        assert!(
            from_parsed_release(ParsedReleaseMetadata::default())
                .stereoscopy
                .is_none()
        );

        struct Query;
        #[async_graphql::Object]
        impl Query {
            async fn parsed(&self) -> ParsedReleasePayload {
                from_parsed_release(ParsedReleaseMetadata::default())
            }
        }
        let schema = async_graphql::Schema::build(
            Query,
            async_graphql::EmptyMutation,
            async_graphql::EmptySubscription,
        )
        .finish()
        .sdl();
        assert!(schema.contains("stereoscopy: ParsedStereoscopyPayload\n"));
        assert!(schema.contains("layout: StereoLayoutValue\n"));
        assert!(schema.contains("presentation: StereoPresentationValue!"));
        assert!(schema.contains("audioChannels: String\n"));
        assert!(schema.contains("audioLanguages: [String!]!\n"));
    }

    #[test]
    fn audio_channels_and_languages_carry_the_parse() {
        let mut parsed = ParsedReleaseMetadata {
            audio_channels: Some("5.1".to_string()),
            languages_audio: vec!["fra".to_string(), "eng".to_string()],
            ..ParsedReleaseMetadata::default()
        };
        let payload = from_parsed_release(parsed.clone());
        assert_eq!(payload.audio_channels.as_deref(), Some("5.1"));
        assert_eq!(payload.audio_languages, vec!["fra", "eng"]);

        parsed.audio_channels = None;
        parsed.languages_audio.clear();
        let payload = from_parsed_release(parsed);
        assert_eq!(payload.audio_channels, None);
        assert!(payload.audio_languages.is_empty());
    }
}
