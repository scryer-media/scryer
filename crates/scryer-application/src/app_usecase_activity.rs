use super::*;

impl AppUseCase {
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

        let score = if input.requested_mode == "manual" {
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
                input.title_id, input.requested_mode
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
                kind,
                message,
                severity,
                channels,
            )
            .await
    }
}
