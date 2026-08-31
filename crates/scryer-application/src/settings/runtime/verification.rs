impl AppUseCase {
    /// Effective verification depth (FR-042). Absent or unparseable settings fall
    /// back to the `full` default rather than silently weakening verification.
    pub(crate) async fn resolve_verification_depth(&self) -> VerificationDepth {
        match self
            .read_setting_string_value_for_scope(SETTINGS_SCOPE_MEDIA, VERIFICATION_DEPTH_KEY, None)
            .await
        {
            Ok(Some(raw)) => match VerificationDepth::from_setting(&raw) {
                Ok(depth) => depth,
                Err(message) => {
                    warn!(
                        setting = VERIFICATION_DEPTH_KEY,
                        value = raw.as_str(),
                        "{message}; falling back to full verification"
                    );
                    VerificationDepth::default()
                }
            },
            Ok(None) => VerificationDepth::default(),
            Err(error) => {
                warn!(
                    setting = VERIFICATION_DEPTH_KEY,
                    "failed to read verification depth setting: {error}; falling back to full verification"
                );
                VerificationDepth::default()
            }
        }
    }
}
impl AppUseCase {
    async fn load_verification_settings(&self) -> AppResult<VerificationSettings> {
        Ok(VerificationSettings {
            depth: self.resolve_verification_depth().await,
        })
    }
}
impl AppUseCase {
    pub async fn get_verification_settings(&self, actor: &User) -> AppResult<VerificationSettings> {
        if !self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?
            && !self
                .has_any_granted_library_permission(
                    actor,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to view verification settings".to_string(),
            ));
        }

        self.load_verification_settings().await
    }
}
impl AppUseCase {
    pub async fn update_verification_settings(
        &self,
        actor: &User,
        input: UpdateVerificationSettings,
    ) -> AppResult<VerificationSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.upsert_media_setting_json(
            VERIFICATION_DEPTH_KEY,
            &input.depth.as_str(),
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "verification_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(vec![VERIFICATION_DEPTH_KEY.to_string()]);

        self.load_verification_settings().await
    }
}
