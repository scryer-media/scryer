impl AppUseCase {
    pub async fn title_acquisition_diagnostics(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<TitleAcquisitionDiagnostics> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;

        let recent_decisions = self
            .services
            .workflow
            .acquisition_scope_states
            .list_release_decisions_for_title(title_id, 25, 0)
            .await?;
        let wanted_items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;
        let pending_releases = self
            .services
            .workflow
            .pending_releases
            .list_pending_releases_for_title(title_id)
            .await?;

        let mut decision_counts = HashMap::<String, i64>::new();
        for decision in &recent_decisions {
            *decision_counts
                .entry(decision.decision_code.clone())
                .or_insert(0) += 1;
        }
        let mut wanted_status_counts = HashMap::<String, i64>::new();
        for item in &wanted_items {
            *wanted_status_counts
                .entry(item.status.as_str().to_string())
                .or_insert(0) += 1;
        }
        let mut pending_release_counts = HashMap::<String, i64>::new();
        for release in &pending_releases {
            *pending_release_counts
                .entry(release.status.as_str().to_string())
                .or_insert(0) += 1;
        }

        let mismatch_recovery_eligible_count = wanted_items
            .iter()
            .filter(|item| {
                item.status == AcquisitionScopeStatus::Wanted && item.mismatch_recovery_eligible
            })
            .count() as i64;

        let mut decision_counts = decision_counts
            .into_iter()
            .map(|(code, count)| DecisionCodeCount { code, count })
            .collect::<Vec<_>>();
        decision_counts.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.code.cmp(&right.code))
        });

        let mut wanted_status_counts = wanted_status_counts
            .into_iter()
            .map(|(status, count)| WantedStatusCount { status, count })
            .collect::<Vec<_>>();
        wanted_status_counts.sort_by(|left, right| left.status.cmp(&right.status));

        let mut pending_release_counts = pending_release_counts
            .into_iter()
            .map(|(status, count)| PendingReleaseStatusCount { status, count })
            .collect::<Vec<_>>();
        pending_release_counts.sort_by(|left, right| left.status.cmp(&right.status));

        let latest_wanted_search_at = wanted_items
            .iter()
            .filter_map(|item| item.last_search_at.clone())
            .max();

        Ok(TitleAcquisitionDiagnostics {
            latest_decision_at: recent_decisions
                .first()
                .map(|decision| decision.created_at.clone()),
            latest_wanted_search_at,
            recent_decisions,
            decision_counts,
            wanted_status_counts,
            pending_release_counts,
            mismatch_recovery_eligible_count,
        })
    }
}
