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
fn candidate_quality_allowed(candidate: &IndexerSearchResult) -> bool {
    candidate
        .quality_profile_decision
        .as_ref()
        .is_some_and(|decision| decision.allowed)
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
) -> ReleaseDecisionOutcome {
    let decision_record =
        release_decision_record(item, title, candidate, decision_code, incumbent_bar, now);
    let written_id = app
        .services
        .workflow
        .acquisition_scope_states
        .insert_release_decision(&decision_record)
        .await;
    ReleaseDecisionOutcome::of(&decision_record.id, written_id.ok().as_deref())
}

/// `record_release_decision` for each candidate in order, written together,
/// each with its own incumbent bar. Outcomes come back in candidate order.
pub(crate) async fn record_release_decisions(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    candidates: &[(&IndexerSearchResult, ReleaseAutoDecisionCode, Option<i32>)],
    now: &DateTime<Utc>,
) -> Vec<ReleaseDecisionOutcome> {
    let explained = candidates
        .iter()
        .map(|&(candidate, decision_code, incumbent_bar)| ExplainedReleaseDecision {
            candidate,
            decision_code,
            incumbent_bar,
            explanation: release_decision_explanation_for(candidate, decision_code),
        })
        .collect();
    record_explained_release_decisions(app, item, title, explained, now).await
}

/// One decision to record, with the explanation its row stores already
/// built: `release_decision_explanation_for(candidate, decision_code)`, or
/// text proven equal to it.
pub(crate) struct ExplainedReleaseDecision<'a> {
    pub(crate) candidate: &'a IndexerSearchResult,
    pub(crate) decision_code: ReleaseAutoDecisionCode,
    pub(crate) incumbent_bar: Option<i32>,
    pub(crate) explanation: crate::ReleaseDecisionExplanation,
}

/// [`record_release_decisions`] for decisions whose explanations the caller
/// has already built, so a caller that also needs the text builds it once.
pub(crate) async fn record_explained_release_decisions(
    app: &AppUseCase,
    item: &AcquisitionScopeState,
    title: &Title,
    decisions: Vec<ExplainedReleaseDecision<'_>>,
    now: &DateTime<Utc>,
) -> Vec<ReleaseDecisionOutcome> {
    let records = decisions
        .into_iter()
        .map(|decision| explained_release_decision_record(item, title, decision, now))
        .collect::<Vec<_>>();
    let record_ids = records
        .iter()
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    let written = app
        .services
        .workflow
        .acquisition_scope_states
        .insert_release_decisions(records)
        .await;
    record_ids
        .iter()
        .zip(written)
        .map(|(record_id, written_id)| {
            ReleaseDecisionOutcome::of(record_id, written_id.ok().as_deref())
        })
        .collect()
}

fn release_decision_record(
    item: &AcquisitionScopeState,
    title: &Title,
    candidate: &IndexerSearchResult,
    decision_code: ReleaseAutoDecisionCode,
    incumbent_bar: Option<i32>,
    now: &DateTime<Utc>,
) -> ReleaseDecision {
    explained_release_decision_record(
        item,
        title,
        ExplainedReleaseDecision {
            candidate,
            decision_code,
            incumbent_bar,
            explanation: release_decision_explanation_for(candidate, decision_code),
        },
        now,
    )
}

/// The ledger row for one decision. Nothing the row reads from the candidate
/// depends on the auto-decision annotation, so the candidate is read as it is
/// and the annotation lives only in the explanation.
fn explained_release_decision_record(
    item: &AcquisitionScopeState,
    title: &Title,
    decision: ExplainedReleaseDecision<'_>,
    now: &DateTime<Utc>,
) -> ReleaseDecision {
    let ExplainedReleaseDecision {
        candidate,
        decision_code,
        incumbent_bar,
        explanation,
    } = decision;
    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: item.id.clone(),
        title_id: title.id.clone(),
        release_title: candidate.title.clone(),
        release_url: candidate.canonical_download_source().map(|(source, _)| source),
        release_size_bytes: candidate.size_bytes,
        decision_code: decision_code.as_str().to_string(),
        candidate_score,
        current_score: incumbent_bar,
        score_delta: incumbent_bar.map(|bar| candidate_score - bar),
        explanation_json: Some(explanation),
        created_at: now.to_rfc3339(),
    }
}

/// What the ledger did with a decision, and therefore whether this cycle
/// learned anything.
///
/// `insert_release_decision` has been an upsert on the decision's identity
/// since 5dcc7b375: the same scope reaching the same verdict about the same
/// release ages the row it already has forward instead of appending. The id it
/// returns says which happened — the record's own id for a fresh row, the
/// surviving row's id when one was already on file — and that is the only
/// "have we said this before?" signal anything needs. It is deliberately not a
/// second table: a rejection that repeats is not a new fact, and the row that
/// proves it is the one the decisions view already shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseDecisionOutcome {
    /// This scope had not reached this verdict about this release before.
    Recorded,
    /// The identical verdict was already on file and was aged forward.
    Unchanged,
    /// The ledger write failed, so nothing is known about what came before.
    /// Treated as new everywhere, which keeps the pre-dedupe behaviour when
    /// the store cannot answer.
    Unknown,
}

impl ReleaseDecisionOutcome {
    pub(crate) fn of(record_id: &str, written_id: Option<&str>) -> Self {
        match written_id {
            Some(written_id) if written_id == record_id => Self::Recorded,
            Some(_) => Self::Unchanged,
            None => Self::Unknown,
        }
    }

    /// Whether this verdict is worth telling the activity feed about: a first
    /// sighting, a changed verdict, or a write we could not interpret.
    pub(crate) fn is_new_signal(self) -> bool {
        !matches!(self, Self::Unchanged)
    }
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
        explanation_json: pending
            .scoring_log_json
            .clone()
            .map(crate::ReleaseDecisionExplanation::untrusted),
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
                .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::acquisition::WANTED_ITEM_NOT_FOUND, format!("wanted item {wid}")))?;
            let library_id = if let Some(library_id) = wanted.library_id.as_deref() {
                library_id.to_string()
            } else {
                self.services
                    .catalog
                    .titles
                    .get_by_id(&wanted.title_id)
                    .await?
                    .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::catalog::TITLE_NOT_FOUND, format!("title {}", wanted.title_id)))?
                    .library_id
            };
            self.require_privileged_library_access(actor, &library_id)
                .await?;
            let items = self
                .services
                .workflow
                .acquisition_scope_states
                .list_release_decisions_for_acquisition_scope_state(wid, query.limit, query.offset)
                .await?;
            let total = if query.skip_total {
                0
            } else {
                self.services
                    .workflow
                    .acquisition_scope_states
                    .count_release_decisions_for_acquisition_scope_state(wid)
                    .await?
            };
            return Ok((items, total));
        }
        if let Some(tid) = query.title_id.as_deref() {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(tid)
                .await?
                .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::catalog::TITLE_NOT_FOUND, format!("title {tid}")))?;
            self.require_privileged_library_access(actor, &title.library_id)
                .await?;
            let items = self
                .services
                .workflow
                .acquisition_scope_states
                .list_release_decisions_for_title(tid, query.limit, query.offset)
                .await?;
            let total = if query.skip_total {
                0
            } else {
                self.services
                    .workflow
                    .acquisition_scope_states
                    .count_release_decisions_for_title(tid)
                    .await?
            };
            return Ok((items, total));
        }
        Ok((vec![], 0))
    }
}
