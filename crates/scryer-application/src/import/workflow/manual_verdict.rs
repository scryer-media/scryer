/// Most files one manual-import verdict request may name. Far above any
/// download a person maps by hand.
pub const MANUAL_IMPORT_VERDICT_MAX_ROWS: usize = 2_000;

/// Most episodes one file of a verdict request may name.
pub const MANUAL_IMPORT_VERDICT_MAX_EPISODES_PER_ROW: usize = 500;

/// How a manual-import file compares with the files already holding the
/// episodes it is mapped to, estimated from its name and size.
///
/// Display only: the import itself decides again on the landed file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualImportVerdict {
    /// The file would beat what the episodes hold.
    Upgrade,
    /// The file ties what the episodes hold.
    Equal,
    /// What the episodes hold is better than the file.
    Downgrade,
    /// None of the episodes holds a file.
    NoCurrentFile,
}

/// One file of a verdict request, with the episodes the member mapped it to.
#[derive(Clone, Debug)]
pub struct ManualImportVerdictRequest {
    pub candidate_id: String,
    pub episode_ids: Vec<String>,
}

/// The best file already holding a mapped scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualImportCurrentFile {
    pub quality: Option<String>,
    pub release_group: Option<String>,
    pub score: i32,
}

/// The estimate for one requested file.
///
/// `verdict` is `None` when the file cannot be estimated: a selection built
/// before sizes were kept, a candidate the selection does not hold, a mapping
/// that names no episode of the title, or a comparison that is not about
/// quality (a profile that forbids upgrades, a broader file in the way).
#[derive(Clone, Debug)]
pub struct ManualImportVerdictRow {
    pub candidate_id: String,
    /// The requested episodes of the title the estimate covers.
    pub episode_ids: Vec<String>,
    pub verdict: Option<ManualImportVerdict>,
    pub estimated_score: Option<i32>,
    pub current_file: Option<ManualImportCurrentFile>,
}

impl ManualImportVerdictRow {
    fn unknown(candidate_id: String, episode_ids: Vec<String>) -> Self {
        Self {
            candidate_id,
            episode_ids,
            verdict: None,
            estimated_score: None,
            current_file: None,
        }
    }
}

/// Estimate, for each requested file of a manual-import selection, whether
/// importing it to the episodes it is mapped to would be an upgrade.
///
/// Read-only. A multi-episode mapping is judged as one span, the way import
/// judges it, so its verdict is the worst across the episodes it takes.
/// Every incumbent is read in one statement for the whole request; nothing
/// touches the file system.
pub async fn manual_import_verdicts(
    app: &AppUseCase,
    actor: &User,
    selection_id: &str,
    requests: Vec<ManualImportVerdictRequest>,
) -> AppResult<Vec<ManualImportVerdictRow>> {
    if requests.len() > MANUAL_IMPORT_VERDICT_MAX_ROWS
        || requests
            .iter()
            .any(|request| request.episode_ids.len() > MANUAL_IMPORT_VERDICT_MAX_EPISODES_PER_ROW)
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::import::MANUAL_IMPORT_VERDICT_REQUEST_TOO_LARGE,
            format!(
                "a manual import verdict request may name at most {MANUAL_IMPORT_VERDICT_MAX_ROWS} files and {MANUAL_IMPORT_VERDICT_MAX_EPISODES_PER_ROW} episodes per file"
            ),
        ));
    }
    let selection = app
        .services
        .workflow
        .imports
        .get_manual_import_selection(selection_id, &actor.id)
        .await?
        .ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::integration::MANUAL_IMPORT_SELECTION_UNAVAILABLE,
                "manual import selection is unavailable; reopen the import dialog".to_string(),
            )
        })?;
    let title = app
        .services
        .catalog
        .titles
        .get_by_id(&selection.title_id)
        .await?
        .ok_or_else(|| {
            AppError::validation_refused(
                crate::error_reasons::integration::MANUAL_IMPORT_SELECTION_TITLE_UNAVAILABLE,
                "manual import selection title is unavailable".to_string(),
            )
        })?;
    app.require_library_permission(
        actor,
        &title.library_id,
        scryer_domain::LibraryPermission::ResolveImports,
    )
    .await?;

    let release_evidence = selection
        .release_evidence_json
        .as_deref()
        .and_then(|snapshot| serde_json::from_str::<ReleaseEvidence>(snapshot).ok());
    let (Some(release_evidence), false) = (release_evidence, title.facet == MediaFacet::Movie)
    else {
        return Ok(requests
            .into_iter()
            .map(|request| ManualImportVerdictRow::unknown(request.candidate_id, Vec::new()))
            .collect());
    };

    let reads = crate::acquisition::title_reads::TitleCatalogReads::new(&title.id);
    let episodes = reads.episodes(app).await?;
    let known_episodes = episodes
        .iter()
        .map(|episode| episode.id.as_str())
        .collect::<HashSet<_>>();
    let candidates = selection
        .candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate))
        .collect::<HashMap<_, _>>();

    // Only episodes of this title count; anything else is dropped, not read.
    let spans = requests
        .iter()
        .map(|request| {
            let mut span = Vec::new();
            for id in &request.episode_ids {
                if known_episodes.contains(id.as_str()) && !span.contains(id) {
                    span.push(id.clone());
                }
            }
            span
        })
        .collect::<Vec<_>>();
    let estimable = requests
        .iter()
        .zip(&spans)
        .map(|(request, span)| {
            !span.is_empty()
                && candidates
                    .get(request.candidate_id.as_str())
                    .is_some_and(|candidate| candidate.size_bytes.is_some())
        })
        .collect::<Vec<_>>();
    let scored_spans = spans
        .iter()
        .zip(&estimable)
        .filter(|(_, estimable)| **estimable)
        .map(|(span, _)| span.clone())
        .collect::<Vec<_>>();
    if scored_spans.is_empty() {
        return Ok(requests
            .into_iter()
            .zip(spans)
            .map(|(request, span)| ManualImportVerdictRow::unknown(request.candidate_id, span))
            .collect());
    }

    // A profile that does not resolve blocks the import itself, which says
    // why; the preview just has nothing to estimate against.
    let profile = match resolve_import_quality_profile(app, &title).await {
        Ok(profile) => profile,
        Err(
            AppError::Validation(_)
            | AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. },
        ) => {
            return Ok(requests
                .into_iter()
                .zip(spans)
                .map(|(request, span)| ManualImportVerdictRow::unknown(request.candidate_id, span))
                .collect());
        }
        Err(error) => return Err(error),
    };
    let context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    reads
        .prime_live_media_files_for_episode_sets(app, &scored_spans)
        .await?;
    let other_video_files = selection.candidates.len() > 1;

    let mut rows = Vec::with_capacity(requests.len());
    for (index, (request, span)) in requests.into_iter().zip(spans).enumerate() {
        if !estimable[index] {
            rows.push(ManualImportVerdictRow::unknown(request.candidate_id, span));
            continue;
        }
        let candidate = candidates[request.candidate_id.as_str()];
        let size_bytes = candidate.size_bytes.unwrap_or_default();
        let parsed = build_augmented_episode_import_metadata_for_title(
            &stored_path_to_path_buf(&candidate.canonical_path),
            &release_evidence,
            &title,
            other_video_files,
        );
        let size_basis = crate::acquisition_coverage::episode_span_size_basis(
            &episodes,
            &span,
            context.default_runtime_minutes(),
        );
        let facts = crate::quality::canonical_context::score_announced_candidate(
            &parsed, size_bytes, size_basis, &context,
        );
        let scope = if span.len() == 1 {
            crate::SubmissionScope::Episode {
                episode_id: span[0].clone(),
            }
        } else {
            crate::SubmissionScope::EpisodeSet {
                episode_ids: span.clone(),
            }
        };
        let subject = app
            .admission_subject_for_scope_with_reads(
                &title,
                &scope,
                &context,
                size_basis.total_runtime_minutes,
                crate::quality::canonical_context::SubjectIntent::Import,
                &reads,
            )
            .await;
        let current_file = match subject.best_incumbent_record(&facts.pipeline) {
            Some(best) => {
                let files = reads
                    .live_media_files_for_episode_ids(app, &span)
                    .await
                    .unwrap_or_default();
                Some(ManualImportCurrentFile {
                    quality: files
                        .iter()
                        .find(|file| file.media_file.id == best.file_id)
                        .and_then(|file| file.media_file.quality_label.clone()),
                    release_group: best.release_group.clone(),
                    score: best.score,
                })
            }
            None => None,
        };
        let verdict = manual_import_verdict_for(&subject, facts);
        rows.push(ManualImportVerdictRow {
            candidate_id: request.candidate_id,
            episode_ids: span,
            verdict,
            estimated_score: Some(facts.score),
            current_file,
        });
    }
    Ok(rows)
}

/// The verdict import's own not-a-downgrade comparison gives this file.
fn manual_import_verdict_for(
    subject: &crate::admission::AdmissionSubject,
    candidate: crate::admission::CandidateFacts<'_>,
) -> Option<ManualImportVerdict> {
    if subject.is_unoccupied() {
        return Some(ManualImportVerdict::NoCurrentFile);
    }
    let verdict = crate::admission::evaluate_admission(
        subject,
        candidate,
        &crate::admission::AdmissionPolicy::not_a_downgrade(),
    );
    match &verdict {
        crate::admission::AdmissionVerdict::Reject(rejection) => {
            crate::import::decide::lost_library_comparison(&rejection.reason)
                .then_some(ManualImportVerdict::Downgrade)
        }
        crate::admission::AdmissionVerdict::Admit {
            previous_best_score,
            ..
        } => Some(match verdict.decided_by(subject, candidate) {
            Some(crate::decision_pipeline::ReleaseDecidingStep::Gate(_)) => {
                ManualImportVerdict::Upgrade
            }
            _ if candidate.score > *previous_best_score => ManualImportVerdict::Upgrade,
            _ => ManualImportVerdict::Equal,
        }),
    }
}
