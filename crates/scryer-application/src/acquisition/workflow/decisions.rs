#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadRouteKey {
    pub source_kind: Option<DownloadSourceKind>,
    pub indexer_id: Option<String>,
}

impl DownloadRouteKey {
    pub(crate) fn for_candidate(candidate: &IndexerSearchResult) -> Option<Self> {
        Some(Self {
            source_kind: candidate.source_kind,
            indexer_id: candidate.indexer_id.clone(),
        })
    }
}

fn annotated_auto_decision_code(candidate: &IndexerSearchResult) -> ReleaseAutoDecisionCode {
    candidate
        .auto_decision_code
        .as_deref()
        .and_then(ReleaseAutoDecisionCode::parse)
        .unwrap_or_else(|| {
            warn!(
                release_title = candidate.title.as_str(),
                "candidate missing auto decision annotation; defaulting to quality_blocked"
            );
            ReleaseAutoDecisionCode::QualityBlocked
        })
}
fn effective_auto_decision_code_for_route(
    candidate: &IndexerSearchResult,
    failed_routes: &[DownloadRouteKey],
    db_blocklist: &crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
) -> ReleaseAutoDecisionCode {
    if crate::app_usecase_discovery::is_release_blocklisted(
        candidate.indexer_id.as_deref(),
        &candidate.title,
        candidate.info_hash(),
        db_blocklist,
    ) {
        return ReleaseAutoDecisionCode::DbBlocklisted;
    }

    if DownloadRouteKey::for_candidate(candidate)
        .is_some_and(|route| failed_routes.contains(&route))
    {
        return ReleaseAutoDecisionCode::DownloadClientUnavailable;
    }

    annotated_auto_decision_code(candidate)
}
/// Record what the gate compared, and against what.
///
/// `incumbent_bar` is the bar the admission gate actually used — the canonical
/// score of the primary file in the way — not the scope ledger's remembered
/// number. The ledger's `current_score` was frequently the score of a release
/// that never landed, so a decision row could claim a comparison that never
/// happened, which is precisely what made the original defect so hard to see.
///
/// `None` is honest, not missing: decisions recorded before the gate runs (a
/// quality-blocked candidate, a pack considered and skipped) genuinely had no
/// bar to compare against.
pub(crate) async fn record_release_decision(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    candidate: &IndexerSearchResult,
    decision_code: ReleaseAutoDecisionCode,
    incumbent_bar: Option<i32>,
    now: &DateTime<Utc>,
) {
    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    let mut decision_candidate = candidate.clone();
    annotate_auto_decision(&mut decision_candidate, decision_code);
    let decision_record = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: item.id.clone(),
        title_id: title.id.clone(),
        release_title: decision_candidate.title.clone(),
        release_url: decision_candidate
            .canonical_download_source()
            .map(|(source, _)| source),
        release_size_bytes: decision_candidate.size_bytes,
        decision_code: decision_code.as_str().to_string(),
        candidate_score,
        current_score: incumbent_bar,
        score_delta: incumbent_bar.map(|bar| candidate_score - bar),
        explanation_json: serialize_decision_explanation(&decision_candidate),
        created_at: now.to_rfc3339(),
    };

    let _ = app
        .services
        .workflow
        .acquisition_scope_states
        .insert_release_decision(&decision_record)
        .await;
}

/// Persist the same decision ledger entry for a release that was previously
/// parked. Its current score is freshly derived by the pending-grab path, while
/// its source details are the immutable release facts saved with the row.
#[expect(
    clippy::too_many_arguments,
    reason = "pending decision persistence carries the saved release and freshly-derived verdict"
)]
pub(crate) async fn record_pending_release_decision(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    pending: &PendingRelease,
    candidate_score: i32,
    decision_code: ReleaseAutoDecisionCode,
    incumbent_bar: Option<i32>,
    now: &DateTime<Utc>,
) {
    let decision_record = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: item.id.clone(),
        title_id: title.id.clone(),
        release_title: pending.release_title.clone(),
        release_url: pending.release_url.clone(),
        release_size_bytes: pending.release_size_bytes,
        decision_code: decision_code.as_str().to_string(),
        candidate_score,
        current_score: incumbent_bar,
        score_delta: incumbent_bar.map(|bar| candidate_score - bar),
        explanation_json: pending.scoring_log_json.clone(),
        created_at: now.to_rfc3339(),
    };

    let _ = app
        .services
        .workflow
        .acquisition_scope_states
        .insert_release_decision(&decision_record)
        .await;
}
impl AppUseCase {
    /// One page of release decisions plus the total row count for the scope.
    pub async fn list_release_decisions_page(
        &self,
        actor: &User,
        query: ReleaseDecisionsQuery,
    ) -> AppResult<(Vec<ReleaseDecision>, i64)> {
        if let Some(wid) = query.wanted_item_id.as_deref() {
            let wanted = self
                .services
                .workflow
                .acquisition_scope_states
                .get_acquisition_scope_state_by_id(wid)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("wanted item {wid}")))?;
            let library_id = if let Some(library_id) = wanted.library_id.as_deref() {
                library_id.to_string()
            } else {
                self.services
                    .catalog
                    .titles
                    .get_by_id(&wanted.title_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("title {}", wanted.title_id)))?
                    .library_id
            };
            self.require_library_permission(
                actor,
                &library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
            let items = self
                .services
                .workflow
                .acquisition_scope_states
                .list_release_decisions_for_acquisition_scope_state(wid, query.limit, query.offset)
                .await?;
            let total = self
                .services
                .workflow
                .acquisition_scope_states
                .count_release_decisions_for_acquisition_scope_state(wid)
                .await?;
            return Ok((items, total));
        }
        if let Some(tid) = query.title_id.as_deref() {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(tid)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {tid}")))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
            let items = self
                .services
                .workflow
                .acquisition_scope_states
                .list_release_decisions_for_title(tid, query.limit, query.offset)
                .await?;
            let total = self
                .services
                .workflow
                .acquisition_scope_states
                .count_release_decisions_for_title(tid)
                .await?;
            return Ok((items, total));
        }
        Ok((vec![], 0))
    }
}
