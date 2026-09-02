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

pub fn from_search_result(result: IndexerSearchResult) -> IndexerSearchResultPayload {
    let seeders = result
        .extra
        .get("seeders")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let peers = result
        .extra
        .get("peers")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let info_hash = result
        .extra
        .get("info_hash")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    let freeleech = result.extra.get("freeleech").and_then(|v| v.as_bool());
    let download_volume_factor = result
        .extra
        .get("downloadvolumefactor")
        .and_then(|v| v.as_f64());

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
        published_at: parse_optional_datetime(result.published_at, "indexer search published_at"),
        thumbs_up: result.thumbs_up,
        thumbs_down: result.thumbs_down,
        grabs: result.indexer_grabs.map(|grabs| grabs as i32),
        parsed_release: result.parsed_release_metadata.map(from_parsed_release),
        quality_profile_decision: result
            .quality_profile_decision
            .map(from_quality_profile_decision),
        seeders,
        peers,
        info_hash,
        freeleech,
        download_volume_factor,
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
        release_group: result.release_group,
        quality: result.quality,
        source: result.source.map(|source| source.to_string()),
        video_codec: result.video_codec.map(|codec| codec.to_string()),
        video_encoding: result.video_encoding,
        audio: result.audio.map(|codec| codec.to_string()),
        is_dual_audio: result.is_dual_audio,
        is_atmos: result.is_atmos,
        is_dolby_vision: result.is_dolby_vision,
        detected_hdr: result.detected_hdr,
        is_proper_upload: result.is_proper_upload,
        is_remux: result.is_remux,
        is_bd_disk: result.is_bd_disk,
        is_ai_enhanced: result.is_ai_enhanced,
        parse_confidence: result.parse_confidence,
        parse_hints: result.parse_hints,
        episode: result.episode.map(from_parsed_episode),
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
    DownloadQueueItemPayload {
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
            scryer_application::AppError::Validation(format!(
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
            .map(from_release_decision)
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
            .map_err(scryer_application::AppError::Validation)?,
        updated_at: parse_datetime(&item.updated_at, "wanted item updated_at")
            .map_err(scryer_application::AppError::Validation)?,
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
            scryer_application::AppError::Validation(format!(
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
            .map(from_release_decision)
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
    Ok(ReleaseDecisionPayload {
        id: decision.id.into(),
        wanted_item_id: decision.wanted_item_id.into(),
        title_id: decision.title_id.into(),
        release_title: decision.release_title,
        release_url: decision.release_url,
        release_size_bytes: decision.release_size_bytes.map(Long::from),
        decision_code: decision.decision_code,
        candidate_score: decision.candidate_score,
        current_score: decision.current_score,
        score_delta: decision.score_delta,
        explanation_json: decision.explanation_json.map(json_string_to_value),
        created_at: parse_datetime(&decision.created_at, "release decision created_at")
            .map_err(scryer_application::AppError::Validation)?,
    })
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
    Ok(TitleAcquisitionDiagnosticsPayload {
        recent_decisions: value
            .recent_decisions
            .into_iter()
            .map(from_release_decision)
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
