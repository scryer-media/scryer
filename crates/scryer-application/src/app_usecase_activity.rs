use super::*;

impl AppUseCase {
    /// Canonical reactive bus event for title-list/detail refresh. Flows that
    /// change title-visible UI state should emit this instead of open-coding
    /// scan- or workflow-specific refresh signals.
    pub(crate) async fn emit_title_updated_activity(
        &self,
        actor_user_id: Option<String>,
        title: &Title,
    ) {
        if let Err(error) = self
            .services
            .record_activity_event(
                actor_user_id,
                Some(title.id.clone()),
                Some(title.facet.as_str().to_string()),
                ActivityKind::TitleUpdated,
                format!("title updated for {}", title.name),
                ActivitySeverity::Info,
                vec![ActivityChannel::WebUi],
            )
            .await
        {
            tracing::warn!(
                title_id = %title.id,
                error = %error,
                "failed to record title updated activity event"
            );
        }
    }

    pub async fn evaluate_policy(
        &self,
        actor: &User,
        input: PolicyInput,
    ) -> AppResult<PolicyOutput> {
        require(actor, &Entitlement::ViewHistory)?;

        let mut reason_codes = vec!["default_policy_evaluation".to_string()];
        if input.has_existing_file {
            reason_codes.push("existing_file_present".to_string());
        }

        let score = if input.requested_mode == scryer_domain::RequestedMode::Manual {
            100.0
        } else {
            80.0
        };

        Ok(PolicyOutput {
            decision: true,
            score,
            reason_codes,
            explanation: format!(
                "policy evaluation for title {} in {} mode",
                input.title_id,
                input.requested_mode.as_str()
            ),
            scoring_log: vec![],
        })
    }

    pub async fn recent_events(
        &self,
        actor: &User,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<HistoryEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        self.services
            .events
            .list(title_id, limit.max(0), offset.max(0))
            .await
    }

    pub async fn recent_activity(
        &self,
        actor: &User,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ActivityEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        Ok(self
            .services
            .activity_stream
            .list(limit.max(0), offset.max(0))
            .await)
    }

    pub fn subscribe_activity_events(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<ActivityEvent>> {
        require(actor, &Entitlement::ViewHistory)?;
        Ok(self.services.activity_event_broadcast.subscribe())
    }

    pub fn subscribe_import_history(&self, actor: &User) -> AppResult<broadcast::Receiver<()>> {
        require(actor, &Entitlement::ViewHistory)?;
        Ok(self.services.import_history_broadcast.subscribe())
    }

    pub async fn active_library_scans(&self, actor: &User) -> AppResult<Vec<LibraryScanSession>> {
        require(actor, &Entitlement::ViewCatalog)?;
        Ok(self.services.library_scan_tracker.list_active().await)
    }

    pub fn subscribe_library_scan_progress(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<LibraryScanSession>> {
        require(actor, &Entitlement::ViewCatalog)?;
        Ok(self.services.library_scan_tracker.subscribe())
    }

    pub fn subscribe_settings_changed(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<String>>> {
        require(actor, &Entitlement::ViewCatalog)?;
        Ok(self.services.settings_changed_broadcast.subscribe())
    }

    pub async fn record_activity_event(
        &self,
        actor: &User,
        title_id: Option<String>,
        kind: ActivityKind,
        message: String,
        severity: ActivitySeverity,
        channels: Vec<ActivityChannel>,
    ) -> AppResult<()> {
        self.services
            .record_activity_event(
                Some(actor.id.clone()),
                title_id,
                None,
                kind,
                message,
                severity,
                channels,
            )
            .await
    }

    pub async fn list_title_history(
        &self,
        actor: &User,
        filter: &TitleHistoryFilter,
    ) -> AppResult<TitleHistoryPage> {
        require(actor, &Entitlement::ViewHistory)?;
        self.services.title_history.list_history(filter).await
    }

    pub async fn list_title_history_for_title(
        &self,
        actor: &User,
        title_id: &str,
        event_types: Option<&[TitleHistoryEventType]>,
        limit: usize,
        offset: usize,
    ) -> AppResult<TitleHistoryPage> {
        require(actor, &Entitlement::ViewHistory)?;
        self.services
            .title_history
            .list_for_title(title_id, event_types, limit, offset)
            .await
    }

    pub async fn list_title_history_for_episode(
        &self,
        actor: &User,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        require(actor, &Entitlement::ViewHistory)?;
        self.services
            .title_history
            .list_for_episode(episode_id, limit)
            .await
    }
}
