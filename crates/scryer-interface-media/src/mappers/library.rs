use super::acquisition::{extract_tag_bool, extract_tag_string};
use super::runtime::monitor_type_value_from_normalized;
use super::*;

fn import_facet_from_payload(payload: &Value) -> Option<MediaFacetValue> {
    let parameters = payload.get("parameters")?.as_array()?;
    for parameter in parameters {
        let (key, value) = match parameter {
            Value::Array(values) => (
                values.first().and_then(Value::as_str),
                values.get(1).and_then(Value::as_str),
            ),
            Value::Object(_) => (
                parameter.get("key").and_then(Value::as_str),
                parameter.get("value").and_then(Value::as_str),
            ),
            _ => (None, None),
        };
        let Some(key) = key else {
            continue;
        };
        if key != "*scryer_facet" {
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        return match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(MediaFacetValue::Movie),
            "series" => Some(MediaFacetValue::Series),
            "anime" => Some(MediaFacetValue::Anime),
            _ => None,
        };
    }
    None
}

fn path_basename(path: &str) -> Option<String> {
    let path = stored_path_to_path_buf(path.trim());
    let display = path.to_string_lossy();
    let trimmed = display.trim().trim_end_matches(std::path::MAIN_SEPARATOR);
    if trimmed.is_empty() {
        return None;
    }
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn looks_like_weaver_job_id(title: &str, source_ref: &str) -> bool {
    let trimmed = title.trim();
    !trimmed.is_empty()
        && (trimmed == source_ref
            || (trimmed.len() >= 4 && trimmed.chars().all(|ch| ch.is_ascii_digit())))
}

fn import_source_title_from_payload(
    payload: &Value,
    source_system: &str,
    source_ref: &str,
    source_path: Option<&str>,
) -> Option<String> {
    let payload_title = payload
        .get("source_title")
        .and_then(Value::as_str)
        .or_else(|| payload.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string);

    let fallback_path_title = source_path.and_then(path_basename).or_else(|| {
        payload
            .get("dest_dir")
            .and_then(Value::as_str)
            .and_then(path_basename)
    });

    if source_system.eq_ignore_ascii_case("weaver")
        && payload_title
            .as_deref()
            .is_some_and(|title| looks_like_weaver_job_id(title, source_ref))
    {
        return fallback_path_title.or(payload_title);
    }

    payload_title.or(fallback_path_title)
}

pub fn from_title(app: &AppUseCase, title: Title) -> TitlePayload {
    let quality_profile_id = extract_tag_string(&title.tags, "scryer:quality-profile:");
    let monitor_type = extract_tag_string(&title.tags, "scryer:monitor-type:")
        .as_deref()
        .and_then(MonitorTypeValue::from_tag_value);
    let use_season_folders = if title.facet == MediaFacet::Movie {
        None
    } else {
        extract_tag_string(&title.tags, "scryer:season-folder:")
            .map(|value| !value.eq_ignore_ascii_case("disabled"))
    };
    let monitor_specials = extract_tag_bool(&title.tags, "scryer:monitor-specials:");
    let inter_season_movies = extract_tag_bool(&title.tags, "scryer:inter-season-movies:");
    let filler_policy = extract_tag_string(&title.tags, "scryer:filler-policy:")
        .as_deref()
        .and_then(FillerPolicyValue::from_app_str);
    let recap_policy = extract_tag_string(&title.tags, "scryer:recap-policy:")
        .as_deref()
        .and_then(RecapPolicyValue::from_app_str);
    let poster_upstream = title.poster_source_url.as_deref().or(title
        .poster_url
        .as_deref()
        .filter(|url| url.starts_with("http")));
    let background_upstream = title.background_source_url.as_deref().or(title
        .background_url
        .as_deref()
        .filter(|url| url.starts_with("http")));
    let poster_url = app.media_image_url(
        poster_upstream,
        Some("title"),
        Some(&title.id),
        ImageProxyKind::Poster,
        "w250",
    );
    let background_url = app.media_image_url(
        background_upstream,
        Some("title"),
        Some(&title.id),
        ImageProxyKind::Fanart,
        "w1280",
    );

    TitlePayload {
        id: title.id.into(),
        library_id: title.library_id.into(),
        name: title.name,
        facet: MediaFacetValue::from_domain(title.facet),
        monitored: title.monitored,
        tags: title.tags,
        external_ids: title
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                value: id.value,
            })
            .collect(),
        created_at: title.created_at,
        year: title.year,
        overview: title.overview,
        poster_url: poster_url.clone(),
        poster_source_url: poster_url,
        background_url: background_url.clone(),
        background_source_url: background_url,
        sort_title: title.sort_title,
        slug: title.slug,
        imdb_id: title.imdb_id,
        runtime_minutes: title.runtime_minutes,
        popularity: title.popularity,
        canonical_tags: title
            .canonical_tags
            .into_iter()
            .map(|tag| CanonicalMediaTagPayload {
                key: tag.key,
                category: tag.category,
                name: tag.name,
                confidence: tag.confidence,
                sources: tag.sources,
                source_tag_keys: tag.source_tag_keys,
                is_adult: tag.is_adult,
                is_spoiler: tag.is_spoiler,
            })
            .collect(),
        content_status: title.content_status,
        language: title.language,
        first_aired: parse_date(title.first_aired),
        network: title.network,
        studio: title.studio,
        country: title.country,
        aliases: title.aliases,
        metadata_language: title.metadata_language,
        metadata_fetched_at: title.metadata_fetched_at,
        quality_profile_id: quality_profile_id.map(Into::into),
        root_folder_id: title.root_folder_id.into(),
        monitor_type,
        use_season_folders,
        monitor_specials,
        inter_season_movies,
        filler_policy,
        recap_policy,
    }
}

pub fn from_title_rating_summary(ratings: TitleRatingSummary) -> TitleRatingPayload {
    TitleRatingPayload {
        rating: ratings.rating,
        rating_sources: ratings.rating_sources,
        external_ratings: ratings
            .external_ratings
            .into_iter()
            .map(|rating| TitleExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
    }
}

/// Map one cached credit onto its GraphQL payload.
///
/// The provider's portrait URL is never handed to clients directly: it goes
/// through the same opaque `/images/media/{token}/{variant}` route posters use,
/// registered against the owning title. `w185` is the only sized portrait the
/// proxy serves (`original` is the other), so it is the card default; clients
/// re-variant with `selectMediaImageVariantUrl`. A credit with no upstream image
/// resolves to null rather than a token that could only 404.
pub fn from_title_credit(
    app: &AppUseCase,
    title_id: &str,
    credit: TitleCredit,
) -> TitleCreditPayload {
    from_credit(app, "title", title_id, credit)
}

pub fn from_movie_entity_credit(
    app: &AppUseCase,
    movie_entity_id: &str,
    credit: TitleCredit,
) -> TitleCreditPayload {
    from_credit(app, "movie", movie_entity_id, credit)
}

fn from_credit(
    app: &AppUseCase,
    owner_type: &str,
    owner_id: &str,
    credit: TitleCredit,
) -> TitleCreditPayload {
    let person_image_url = Some(credit.person_image_url.trim())
        .filter(|url| !url.is_empty())
        .and_then(|url| {
            app.media_image_url(
                Some(url),
                Some(owner_type),
                Some(owner_id),
                ImageProxyKind::Person,
                "w185",
            )
        });
    TitleCreditPayload {
        kind: credit.kind,
        person_name: credit.person_name,
        person_original_name: credit.person_original_name,
        person_image_url,
        character: credit.character_name,
        language: credit.language,
        billing_order: credit.billing_order,
        episode_count: credit.episode_count,
    }
}

/// Project one media request.
///
/// `policy` carries the decision trace and the lease claim, which live in other
/// tables and are therefore read by the caller in one batch rather than one
/// query per row (see `AppUseCase::media_request_policy_facts`). Passing `None`
/// projects the request with no lease and no decision — exactly what a request
/// that was never evaluated looks like — so a call site that has no use for the
/// policy detail pays nothing for it.
pub fn from_media_request(
    app: &AppUseCase,
    request: MediaRequest,
    policy: Option<&scryer_application::request_rules::MediaRequestPolicyFacts>,
) -> MediaRequestPayload {
    let owner_id = request.id.to_string();
    let poster_url = app.media_image_url(
        request.poster_url.as_deref(),
        Some("media_request"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    // Unlike the poster, no placeholder: requests submitted before background
    // art was captured must fall back to the poster on the card.
    let background_url = request
        .background_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .and_then(|url| {
            app.media_image_url(
                Some(url),
                Some("media_request"),
                Some(&owner_id),
                ImageProxyKind::Fanart,
                "w1280",
            )
        });
    let policy_projection = super::request_rules::project_media_request_policy(&request, policy);
    let metadata = super::request_rules::from_media_request_metadata(
        &scryer_application::MediaRequestMetadataSnapshotExt::metadata_snapshot(&request),
    );
    let requested_lease_days = request.requested_lease_days;
    let approved_lease_days = request.approved_lease_days;
    let policy_tags = request.policy_tags.clone();
    let rating_summary = request.rating_summary;
    MediaRequestPayload {
        id: request.id.into(),
        library_id: request.library_id.into(),
        facet: MediaFacetValue::from_domain(request.facet),
        status: MediaRequestStatusValue::from_domain(request.status),
        identity_fingerprint: request.identity_fingerprint,
        title: request.title,
        sort_title: request.sort_title,
        slug: request.slug,
        poster_url,
        background_url,
        year: request.year,
        overview: request.overview,
        runtime_minutes: request.runtime_minutes,
        language: request.language,
        content_status: request.content_status,
        rating: rating_summary.rating,
        rating_sources: rating_summary.rating_sources,
        external_ratings: rating_summary
            .external_ratings
            .into_iter()
            .map(|rating| MediaRequestExternalRatingPayload {
                source: rating.source,
                value: rating.value,
                score: rating.score,
                normalized: rating.normalized,
                votes: rating.votes,
                url: rating.url,
            })
            .collect(),
        requested_quality_profile_id: request.requested_quality_profile_id.map(Into::into),
        requested_quality_profile_name: request.requested_quality_profile_name,
        requested_monitor_type: request
            .requested_monitor_type
            .as_deref()
            .and_then(monitor_type_value_from_normalized),
        requested_monitor_selection: request
            .requested_monitor_selection
            .as_ref()
            .map(monitor_selection_payload),
        resolved_by_user_id: request.resolved_by_user_id.map(Into::into),
        resolved_at: request.resolved_at,
        created_title_id: request.created_title_id.map(Into::into),
        approved_quality_profile_id: request.approved_quality_profile_id.map(Into::into),
        approved_quality_profile_name: request.approved_quality_profile_name,
        external_ids: request
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                value: id.value,
            })
            .collect(),
        requesters: request
            .requesters
            .into_iter()
            .map(|requester| MediaRequestRequesterPayload {
                user_id: requester.user_id.into(),
                username: requester.username,
                avatar_url: requester.avatar_url,
                requested_at: requester.requested_at,
            })
            .collect(),
        created_by_user_id: request.created_by_user_id.into(),
        created_at: request.created_at,
        updated_at: request.updated_at,
        requested_lease_days: requested_lease_days
            .map(|days| i32::try_from(days).unwrap_or(i32::MAX)),
        approved_lease_days: approved_lease_days
            .map(|days| i32::try_from(days).unwrap_or(i32::MAX)),
        lease: policy_projection.lease,
        decision: policy_projection.decision,
        policy_tags,
        metadata,
    }
}

pub fn from_library(library: Library) -> LibraryPayload {
    let is_bootstrap_default_root_set =
        scryer_application::is_bootstrap_default_library_root_set(&library);
    LibraryPayload {
        id: library.id.into(),
        facet: MediaFacetValue::from_domain(library.facet),
        name: library.name,
        slug: library.slug,
        is_default: library.is_default,
        is_bootstrap_default_root_set,
        roots: library
            .roots
            .into_iter()
            .map(|root| LibraryRootPayload {
                id: root.id.into(),
                path: root.path,
                is_default: root.is_default,
            })
            .collect(),
    }
}

pub fn from_library_scan_summary(summary: LibraryScanSummary) -> LibraryScanSummaryPayload {
    LibraryScanSummaryPayload {
        scanned: summary.scanned as i32,
        matched: summary.matched as i32,
        imported: summary.imported as i32,
        skipped: summary.skipped as i32,
        unmatched: summary.unmatched as i32,
    }
}

pub fn from_pending_import_counts(counts: PendingImportCounts) -> PendingImportCountsPayload {
    PendingImportCountsPayload {
        movie: counts.movie as i32,
        series: counts.series as i32,
        anime: counts.anime as i32,
    }
}

fn from_activity_window_counts(counts: ActivityWindowCounts) -> ActivityWindowCountsPayload {
    ActivityWindowCountsPayload {
        grabbed: counts.grabbed as i32,
        upgraded: counts.upgraded as i32,
        imported: counts.imported as i32,
        import_failed: counts.import_failed as i32,
        download_failed: counts.download_failed as i32,
    }
}

pub fn from_dashboard_activity_stats(
    stats: DashboardActivityStats,
) -> DashboardActivityStatsPayload {
    DashboardActivityStatsPayload {
        current: from_activity_window_counts(stats.current),
        previous: from_activity_window_counts(stats.previous),
    }
}

pub fn from_storage_root_usage(usage: StorageRootUsage) -> StorageRootUsagePayload {
    StorageRootUsagePayload {
        path: usage.path,
        library_id: usage.library_id.into(),
        library_name: usage.library_name,
        facet: MediaFacetValue::from_domain(usage.facet),
        used_bytes: usage.used_bytes.map(Long::from),
        total_bytes: usage.total_bytes.map(Long::from),
    }
}

pub fn from_media_request_counts(counts: MediaRequestCounts) -> MediaRequestCountsPayload {
    MediaRequestCountsPayload {
        movie: counts.movie as i32,
        series: counts.series as i32,
        anime: counts.anime as i32,
    }
}

fn from_pending_import_search_attempt(
    attempt: PendingImportSearchAttempt,
) -> PendingImportSearchAttemptPayload {
    PendingImportSearchAttemptPayload {
        query: attempt.query,
        result_count: attempt.result_count as i32,
        top_results: attempt.top_results,
        summary: attempt.summary,
    }
}

/// Map one pending import onto its payload.
///
/// Fallible for the same reason [`from_title_history_record`] is: `created_at`
/// is stored as text, and an unparseable timestamp is surfaced as a validation
/// error rather than papered over with a sentinel date.
pub fn from_pending_import_item(
    item: PendingImportItem,
) -> scryer_application::AppResult<PendingImportItemPayload> {
    Ok(PendingImportItemPayload {
        id: item.id.into(),
        library_id: item.library_id.into(),
        facet: MediaFacetValue::from_domain(item.facet),
        status: PendingImportStatusValue::from_application(item.status),
        title_id: item.title_id.map(Into::into),
        title_name: item.title_name,
        title_slug: item.title_slug,
        display_name: item.display_name,
        path: item.path,
        folder_path: item.folder_path,
        query: item.query,
        year_hint: item.year_hint,
        reason: item.reason,
        reason_class: PendingImportReasonClassValue::from_application(item.reason_class),
        search_attempts: item
            .search_attempts
            .into_iter()
            .map(from_pending_import_search_attempt)
            .collect(),
        size_bytes: item.size_bytes.map(Long::from),
        created_at: parse_datetime(&item.created_at, "pending import created_at")
            .map_err(scryer_application::AppError::Validation)?,
    })
}

pub fn from_pending_import_connection(
    connection: PendingImportConnection,
    offset: i64,
) -> scryer_application::AppResult<PendingImportConnectionPayload> {
    let items = connection
        .items
        .into_iter()
        .map(from_pending_import_item)
        .collect::<scryer_application::AppResult<Vec<_>>>()?;
    let has_more = offset.saturating_add(items.len() as i64) < connection.total;
    Ok(PendingImportConnectionPayload {
        items,
        total_count: connection.total as i32,
        has_more,
    })
}

pub fn from_resolve_pending_import_result(
    app: &AppUseCase,
    result: ResolvePendingImportResult,
) -> ResolvePendingImportPayload {
    ResolvePendingImportPayload {
        title: from_title(app, result.title),
        created: result.created,
        library_scan: result.library_scan.map(from_library_scan_summary),
        metadata_hydration_state: AddTitleHydrationStateValue::from_application(
            result.metadata_hydration_state,
        ),
    }
}

pub fn from_ignore_pending_import_result(
    result: IgnorePendingImportResult,
) -> IgnorePendingImportPayload {
    IgnorePendingImportPayload {
        id: async_graphql::ID::from(result.id),
        status: PendingImportStatusValue::from_application(result.status),
    }
}

pub fn from_cancel_library_scan_result(
    result: scryer_application::CancelLibraryScanResult,
) -> CancelLibraryScanPayload {
    CancelLibraryScanPayload {
        session_id: async_graphql::ID::from(result.session_id),
        accepted: result.accepted,
    }
}

pub fn from_library_scan_phase_progress(
    progress: scryer_application::LibraryScanPhaseProgress,
) -> LibraryScanPhaseProgressPayload {
    LibraryScanPhaseProgressPayload {
        total: progress.total as i32,
        completed: progress.completed as i32,
        failed: progress.failed as i32,
    }
}

pub fn from_library_scan_session(
    session: scryer_application::LibraryScanSession,
) -> LibraryScanProgressPayload {
    LibraryScanProgressPayload {
        session_id: session.session_id.into(),
        facet: MediaFacetValue::from_domain(session.facet),
        library_id: session.library_id.map(Into::into),
        mode: LibraryScanModeValue::from_application(session.mode),
        status: LibraryScanStatusValue::from_application(session.status),
        started_at: session.started_at,
        updated_at: session.updated_at,
        found_titles: session.found_titles as i32,
        title_match_total_known: session.title_match_total_known,
        title_match_progress: from_library_scan_phase_progress(session.title_match_progress),
        hydration_total_known: session.metadata_total_known,
        hydration_progress: from_library_scan_phase_progress(session.metadata_progress),
        media_analysis_total_known: session.file_total_known,
        media_analysis_progress: from_library_scan_phase_progress(session.file_progress),
        summary: session.summary.map(from_library_scan_summary),
    }
}

pub fn from_job_definition(definition: JobDefinition) -> JobDefinitionPayload {
    JobDefinitionPayload {
        key: JobKeyValue::from_application(definition.key),
        display_name: definition.display_name,
        description: definition.description,
        category: JobCategoryValue::from_application(definition.category),
        section: JobSectionValue::from_application(definition.section),
        manual_trigger_allowed: definition.manual_trigger_allowed,
        uses_library_scan_progress: definition.uses_library_scan_progress,
        schedule: JobScheduleInfoPayload {
            kind: JobScheduleKindValue::from_application(definition.schedule.kind),
            description: definition.schedule.description,
            interval_seconds: definition
                .schedule
                .interval_seconds
                .map(|value| value as i32),
            initial_delay_seconds: definition
                .schedule
                .initial_delay_seconds
                .map(|value| value as i32),
            next_run_at: definition.schedule.next_run_at,
        },
    }
}

pub fn from_job_run(run: JobRun) -> JobRunPayload {
    JobRunPayload {
        id: run.id.into(),
        job_key: JobKeyValue::from_application(run.job_key),
        display_name: run.display_name,
        category: JobCategoryValue::from_application(run.category),
        section: JobSectionValue::from_application(run.section),
        status: JobRunStatusValue::from_application(run.status),
        trigger_source: JobTriggerSourceValue::from_application(run.trigger_source),
        started_at: run.started_at,
        completed_at: run.completed_at,
        summary_json: run.summary_json.map(json_string_to_value),
        summary_text: run.summary_text,
        error_text: run.error_text,
        progress_json: run.progress_json.map(json_string_to_value),
        library_scan_progress: run.library_scan_progress.map(from_library_scan_session),
    }
}

pub fn from_media_rename_plan(plan: RenamePlan) -> MediaRenamePlanPayload {
    MediaRenamePlanPayload {
        facet: MediaFacetValue::from_domain(plan.facet),
        title_id: plan.title_id.map(Into::into),
        template: plan.template,
        collision_policy: RenameCollisionPolicyValue::from_app_str(plan.collision_policy.as_str())
            .unwrap_or(RenameCollisionPolicyValue::Skip),
        missing_metadata_policy: RenameMissingMetadataPolicyValue::from_app_str(
            plan.missing_metadata_policy.as_str(),
        )
        .unwrap_or(RenameMissingMetadataPolicyValue::FallbackTitle),
        fingerprint: plan.fingerprint,
        total: plan.total as i32,
        renamable: plan.renamable as i32,
        noop: plan.noop as i32,
        conflicts: plan.conflicts as i32,
        errors: plan.errors as i32,
        items: plan
            .items
            .into_iter()
            .map(from_media_rename_plan_item)
            .collect(),
    }
}

fn from_media_rename_plan_item(item: RenamePlanItem) -> MediaRenamePlanItemPayload {
    MediaRenamePlanItemPayload {
        collection_id: item.collection_id.map(Into::into),
        series_movie_link_ids: item
            .series_movie_link_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        normalized_filename: item.normalized_filename,
        collision: item.collision,
        reason_code: item.reason_code,
        write_action: item.write_action.as_str().to_string(),
        source_size_bytes: item.source_size_bytes.map(Long::from_u64_saturating),
        source_mtime_unix_ms: item.source_mtime_unix_ms.map(Long::from),
    }
}

pub fn from_media_rename_apply(result: RenameApplyResult) -> MediaRenameApplyPayload {
    MediaRenameApplyPayload {
        plan_fingerprint: result.plan_fingerprint,
        total: result.total as i32,
        applied: result.applied as i32,
        skipped: result.skipped as i32,
        failed: result.failed as i32,
        items: result
            .items
            .into_iter()
            .map(from_media_rename_apply_item)
            .collect(),
    }
}

fn from_media_rename_apply_item(item: RenameApplyItemResult) -> MediaRenameApplyItemPayload {
    MediaRenameApplyItemPayload {
        collection_id: item.collection_id.map(Into::into),
        series_movie_link_ids: item
            .series_movie_link_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        final_path: item.final_path,
        write_action: item.write_action.as_str().to_string(),
        status: item.status.as_str().to_string(),
        reason_code: item.reason_code,
        error_message: item.error_message,
    }
}

pub fn from_collection(collection: Collection) -> CollectionPayload {
    CollectionPayload {
        id: collection.id.into(),
        title_id: collection.title_id.into(),
        collection_type: collection.collection_type.into(),
        collection_index: collection.collection_index,
        label: collection.label,
        ordered_path: collection.ordered_path,
        narrative_order: collection.narrative_order,
        first_episode_number: collection.first_episode_number,
        last_episode_number: collection.last_episode_number,
        monitored: collection.monitored,
        created_at: collection.created_at,
    }
}

pub fn from_movie_entity(
    app: &AppUseCase,
    permission_title_id: String,
    movie: scryer_domain::MovieEntity,
) -> MovieEntityPayload {
    let owner_id = movie.id.to_string();
    let ratings = from_title_rating_summary(movie.ratings.unwrap_or_default());
    let poster_url = app.media_image_url(
        movie.poster_url.as_deref(),
        Some("movie"),
        Some(&owner_id),
        ImageProxyKind::Poster,
        "w250",
    );
    MovieEntityPayload {
        permission_title_id: permission_title_id.into(),
        id: movie.id.into(),
        title: movie.title,
        slug: movie.slug,
        year: movie.year,
        overview: movie.overview,
        poster_url,
        runtime_minutes: movie.runtime_minutes,
        content_status: movie.content_status,
        imdb_id: movie.imdb_id,
        tvdb_id: movie.tvdb_id,
        tmdb_id: movie.tmdb_id,
        mal_id: movie.mal_id,
        anidb_id: movie.anidb_id,
        ratings,
    }
}

pub fn from_series_movie_link(
    app: &AppUseCase,
    link: scryer_domain::SeriesMovieLink,
) -> SeriesMovieLinkPayload {
    let series_title_id = link.series_title_id.clone();
    SeriesMovieLinkPayload {
        id: link.id.into(),
        movie: from_movie_entity(app, series_title_id, link.movie),
        narrative_order: link.narrative_order,
        after_season: link.after_season,
        before_season: link.before_season,
        linked_episode_id: link.linked_episode_id.map(Into::into),
        continuity_status: link.continuity_status,
        movie_form: link.movie_form,
        signal_summary: link.signal_summary,
        monitoring_override: link.monitoring_override,
        metadata_active: link.metadata_active,
        monitored: link.monitored,
        tags: link.tags,
    }
}

pub fn from_episode(app: &AppUseCase, episode: Episode) -> EpisodePayload {
    let image_url = app.media_image_url(
        episode.image_url.as_deref(),
        Some("episode"),
        Some(&episode.id),
        ImageProxyKind::EpisodeStill,
        "w300",
    );
    EpisodePayload {
        id: episode.id.into(),
        title_id: episode.title_id.into(),
        collection_id: episode.collection_id.map(Into::into),
        episode_type: episode.episode_type.into(),
        episode_number: episode.episode_number,
        season_number: episode.season_number,
        episode_label: episode.episode_label,
        title: episode.title,
        overview: episode.overview,
        air_date: parse_date(episode.air_date),
        duration_seconds: episode.duration_seconds,
        has_multi_audio: episode.has_multi_audio,
        has_subtitle: episode.has_subtitle,
        is_filler: episode.is_filler,
        is_recap: episode.is_recap,
        absolute_number: episode.absolute_number,
        image_url,
        monitored: episode.monitored,
        created_at: episode.created_at,
    }
}

pub fn from_episode_media_availability(
    availability: EpisodeMediaAvailability,
) -> EpisodeMediaAvailabilityPayload {
    let state = match availability.state {
        EpisodeMediaAvailabilityState::Available => EpisodeMediaAvailabilityStateValue::Available,
        EpisodeMediaAvailabilityState::PendingScan => {
            EpisodeMediaAvailabilityStateValue::PendingScan
        }
        EpisodeMediaAvailabilityState::ScanFailed => EpisodeMediaAvailabilityStateValue::ScanFailed,
        EpisodeMediaAvailabilityState::Missing => EpisodeMediaAvailabilityStateValue::Missing,
        EpisodeMediaAvailabilityState::Unmonitored => {
            EpisodeMediaAvailabilityStateValue::Unmonitored
        }
    };
    EpisodeMediaAvailabilityPayload {
        state,
        primary_quality_label: availability.primary_quality_label,
    }
}

pub fn fallback_episode_media_availability(monitored: bool) -> EpisodeMediaAvailabilityPayload {
    EpisodeMediaAvailabilityPayload {
        state: if monitored {
            EpisodeMediaAvailabilityStateValue::Missing
        } else {
            EpisodeMediaAvailabilityStateValue::Unmonitored
        },
        primary_quality_label: None,
    }
}

pub fn from_calendar_episode(
    app: &AppUseCase,
    ep: CalendarEpisode,
    availability: Option<EpisodeMediaAvailability>,
    playback_links: Vec<MediaServerPlaybackLinkPayload>,
) -> CalendarEpisodePayload {
    let media_availability = availability
        .map(from_episode_media_availability)
        .unwrap_or_else(|| fallback_episode_media_availability(ep.monitored));
    let is_movie = ep.title_facet == "movie";
    let image_url = app.media_image_url(
        ep.image_url.as_deref(),
        Some(if is_movie { "title" } else { "episode" }),
        Some(if is_movie { &ep.title_id } else { &ep.id }),
        if is_movie {
            ImageProxyKind::Poster
        } else {
            ImageProxyKind::EpisodeStill
        },
        if is_movie { "w250" } else { "w300" },
    );
    CalendarEpisodePayload {
        id: ep.id.into(),
        title_id: ep.title_id.into(),
        library_id: ep.library_id.into(),
        library_name: ep.library_name,
        library_slug: ep.library_slug,
        title_name: ep.title_name,
        title_slug: ep.title_slug,
        title_facet: ep.title_facet,
        season_number: ep.season_number,
        episode_number: ep.episode_number,
        episode_title: ep.episode_title,
        overview: ep.overview,
        image_url,
        air_date: parse_date(ep.air_date),
        monitored: ep.monitored,
        media_availability,
        playback_links,
    }
}

pub fn from_title_media_file(file: scryer_application::TitleMediaFile) -> TitleMediaFilePayload {
    TitleMediaFilePayload {
        id: file.id.into(),
        title_id: file.title_id.into(),
        episode_id: file.episode_id.map(Into::into),
        series_movie_link_ids: file
            .series_movie_link_ids
            .into_iter()
            .map(Into::into)
            .collect(),
        file_path: file.file_path,
        size_bytes: Long::from(file.size_bytes),
        role: file.role.as_str().to_string(),
        quality_label: file.quality_label,
        scan_status: file.scan_status,
        created_at: parse_required_datetime(&file.created_at, "title media file created_at"),
        video_codec: file.video_codec.map(|codec| codec.to_string()),
        video_width: file.video_width,
        video_height: file.video_height,
        video_bitrate_kbps: file.video_bitrate_kbps,
        video_bit_depth: file.video_bit_depth,
        video_hdr_format: file.video_hdr_format,
        video_frame_rate: file.video_frame_rate,
        video_profile: file.video_profile,
        audio_codec: file.audio_codec,
        audio_channels: file.audio_channels,
        audio_bitrate_kbps: file.audio_bitrate_kbps,
        audio_languages: file.audio_languages,
        audio_streams: file
            .audio_streams
            .into_iter()
            .map(|s| crate::types::AudioStreamDetailPayload {
                codec: s.codec,
                channels: s.channels,
                language: s.language,
                bitrate_kbps: s.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: file.subtitle_languages,
        subtitle_codecs: file.subtitle_codecs,
        subtitle_streams: file
            .subtitle_streams
            .into_iter()
            .map(|s| crate::types::SubtitleStreamDetailPayload {
                codec: s.codec,
                language: s.language,
                name: s.name,
                forced: s.forced,
                default: s.default,
            })
            .collect(),
        has_multiaudio: file.has_multiaudio,
        duration_seconds: file.duration_seconds,
        num_chapters: file.num_chapters,
        container_format: file.container_format,
        scene_name: file.scene_name,
        release_group: file.release_group,
        source_type: file.source_type,
        resolution: file.resolution,
        video_codec_parsed: file.video_codec_parsed.map(|codec| codec.to_string()),
        audio_codec_parsed: file.audio_codec_parsed,
        acquisition_score: file.acquisition_score,
        scoring_log: file.scoring_log,
        indexer_source: file.indexer_source,
        grabbed_release_title: file.grabbed_release_title,
        grabbed_at: parse_optional_datetime(file.grabbed_at, "title media file grabbed_at"),
        edition: file.edition,
        original_file_path: file.original_file_path,
        release_hash: file.release_hash,
    }
}

pub fn from_import_record(record: scryer_domain::ImportRecord) -> ImportRecordPayload {
    // Deserialize result_json to extract structured fields
    let (error_message, decision, skip_reason, title_id, source_path, dest_path) =
        if let Some(ref result_json) = record.result_json {
            if let Ok(result) = serde_json::from_str::<scryer_domain::ImportResult>(result_json) {
                (
                    result.error_message,
                    Some(ImportDecisionValue::from_domain(result.decision)),
                    result.skip_reason.map(ImportSkipReasonValue::from_domain),
                    result.title_id,
                    Some(result.source_path),
                    result.dest_path,
                )
            } else {
                (None, None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None, None)
        };

    let payload = serde_json::from_str::<serde_json::Value>(&record.payload_json).ok();
    let source_title = payload.as_ref().and_then(|payload| {
        import_source_title_from_payload(
            payload,
            &record.source_system,
            &record.source_ref,
            source_path.as_deref(),
        )
    });
    let facet = payload.as_ref().and_then(import_facet_from_payload);

    ImportRecordPayload {
        id: record.id.into(),
        source_system: record.source_system,
        source_ref: record.source_ref,
        source_title,
        facet,
        import_type: ImportTypeValue::from_domain(record.import_type),
        status: ImportStatusValue::from_domain(record.status),
        error_message,
        decision,
        skip_reason,
        title_id: title_id.map(Into::into),
        source_path,
        dest_path,
        started_at: parse_optional_datetime(record.started_at, "import history started_at"),
        finished_at: parse_optional_datetime(record.finished_at, "import history finished_at"),
        created_at: parse_required_datetime(&record.created_at, "import history created_at"),
    }
}

use scryer_application::location::folder_match::{
    ChangeTitleFolderPreview, ChangeTitleFolderResult, DisplacedTitleRepair, FolderMatchOutcome,
    FolderMatchOwnership, FolderMatchResolution, FolderMatchTitleRef,
};

/// Stored paths carry an internal escape form for names the platform cannot
/// spell in UTF-8; the API always hands back the real path.
fn display_path(path: &str) -> String {
    scryer_application::stored_paths::stored_path_to_display_string(path)
}

pub fn folder_match_resolution_into_application(
    value: FolderMatchResolutionValue,
) -> FolderMatchResolution {
    match value {
        FolderMatchResolutionValue::Assign => FolderMatchResolution::Assign,
        FolderMatchResolutionValue::Swap => FolderMatchResolution::Swap,
        FolderMatchResolutionValue::TakeOver => FolderMatchResolution::TakeOver,
    }
}

fn from_folder_match_resolution(resolution: FolderMatchResolution) -> FolderMatchResolutionValue {
    match resolution {
        FolderMatchResolution::Assign => FolderMatchResolutionValue::Assign,
        FolderMatchResolution::Swap => FolderMatchResolutionValue::Swap,
        FolderMatchResolution::TakeOver => FolderMatchResolutionValue::TakeOver,
    }
}

fn from_folder_match_ownership(ownership: FolderMatchOwnership) -> FolderMatchOwnershipValue {
    match ownership {
        FolderMatchOwnership::Unowned => FolderMatchOwnershipValue::Unowned,
        FolderMatchOwnership::OwnedByThisTitle => FolderMatchOwnershipValue::OwnedByThisTitle,
        FolderMatchOwnership::OwnedByAnotherTitle => FolderMatchOwnershipValue::OwnedByAnotherTitle,
    }
}

fn from_folder_match_outcome(outcome: FolderMatchOutcome) -> FolderMatchOutcomeValue {
    match outcome {
        FolderMatchOutcome::AlreadyOwned => FolderMatchOutcomeValue::AlreadyOwned,
        FolderMatchOutcome::Assigned => FolderMatchOutcomeValue::Assigned,
        FolderMatchOutcome::Swapped => FolderMatchOutcomeValue::Swapped,
        FolderMatchOutcome::TakenOver => FolderMatchOutcomeValue::TakenOver,
    }
}

fn from_folder_match_title_ref(title: FolderMatchTitleRef) -> FolderMatchTitleRefPayload {
    FolderMatchTitleRefPayload {
        id: ID::from(title.title_id),
        name: title.title_name,
        // Stored paths are an internal encoding; the API always shows the real one.
        folder_path: title.folder_path.as_deref().map(display_path),
    }
}

fn from_displaced_title_repair(displaced: DisplacedTitleRepair) -> DisplacedTitleRepairPayload {
    DisplacedTitleRepairPayload {
        id: ID::from(displaced.title_id),
        name: displaced.title_name,
        previous_folder_path: display_path(&displaced.previous_folder_path),
        repair_reason_code: displaced.repair_reason_code,
    }
}

pub fn from_change_title_folder_preview(
    preview: ChangeTitleFolderPreview,
) -> ChangeTitleFolderPreviewPayload {
    ChangeTitleFolderPreviewPayload {
        facet: MediaFacetValue::from_domain(preview.facet),
        title: from_folder_match_title_ref(preview.title),
        library_id: ID::from(preview.library_id),
        library_name: preview.library_name,
        current_root_id: preview.current_root_id.map(ID::from),
        current_root_path: preview.current_root_path.as_deref().map(display_path),
        selected_folder_path: display_path(&preview.selected_folder_path),
        selected_root_id: ID::from(preview.selected_root_id),
        selected_root_path: display_path(&preview.selected_root_path),
        ownership: from_folder_match_ownership(preview.ownership),
        current_owner: preview.current_owner.map(from_folder_match_title_ref),
        current_folder_tracked_media_count: preview.current_folder_tracked_media_count as i32,
        selected_folder_tracked_media_count: preview.selected_folder_tracked_media_count as i32,
        files_will_move: preview.files_will_move,
        no_op: preview.no_op,
        available_resolutions: preview
            .available_resolutions
            .into_iter()
            .map(from_folder_match_resolution)
            .collect(),
    }
}

/// Domain selection -> GraphQL payload.
pub fn monitor_selection_payload(
    selection: &scryer_domain::MonitorSelection,
) -> MonitorSelectionPayload {
    MonitorSelectionPayload {
        season_numbers: selection.seasons.clone(),
        series_movies: selection
            .series_movies
            .iter()
            .map(|movie| MonitorSelectionMoviePayload {
                name: movie.name.clone(),
                external_ids: movie
                    .external_ids
                    .iter()
                    .map(|external_id| ExternalIdPayload {
                        source: external_id.source.clone(),
                        value: external_id.value.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn from_change_title_folder_result(
    result: ChangeTitleFolderResult,
) -> ChangeTitleFolderPayload {
    ChangeTitleFolderPayload {
        outcome: from_folder_match_outcome(result.outcome),
        title: from_folder_match_title_ref(result.title),
        previous_folder_path: result.previous_folder_path.as_deref().map(display_path),
        detached_media_file_count: result.detached_media_file_count as i32,
        scan: result.scan.map(from_library_scan_summary),
        swapped_title: result.swapped_title.map(from_folder_match_title_ref),
        swapped_title_scan: result.swapped_title_scan.map(from_library_scan_summary),
        displaced_title: result.displaced_title.map(from_displaced_title_repair),
    }
}

/// GraphQL input -> domain selection. Normalization (dedupe, dropping movies
/// with no usable identifier) happens in the application layer.
pub fn monitor_selection_from_input(
    input: MonitorSelectionInput,
) -> scryer_domain::MonitorSelection {
    scryer_domain::MonitorSelection {
        seasons: input.season_numbers,
        series_movies: input
            .series_movies
            .unwrap_or_default()
            .into_iter()
            .map(|movie| scryer_domain::MonitorSelectionMovie {
                name: movie.name,
                external_ids: movie
                    .external_ids
                    .into_iter()
                    .map(|external_id| scryer_domain::ExternalId {
                        source: external_id.source,
                        value: external_id.value,
                    })
                    .collect(),
            })
            .collect(),
    }
}
