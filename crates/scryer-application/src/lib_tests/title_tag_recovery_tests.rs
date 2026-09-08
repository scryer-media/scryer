use super::*;

#[tokio::test]
async fn bulk_title_tag_ceiling_preflight_leaves_every_title_unchanged() {
    let (app, user) = bootstrap();
    let first = create_tagged_movie(&app, &user, "First").await;
    let full = create_tagged_movie(&app, &user, "Full").await;
    let labels = seed_ceiling_labels(&app, &user).await;
    app.update_title_tags(
        &user,
        &[full.id.clone()],
        &labels[..crate::MAX_USER_TAGS_PER_TITLE],
        &[],
    )
    .await
    .unwrap();
    assert!(
        app.update_title_tags(
            &user,
            &[first.id.clone(), full.id.clone()],
            &labels[crate::MAX_USER_TAGS_PER_TITLE..],
            &[]
        )
        .await
        .is_err()
    );
    assert!(stored_title_tags(&app, &first.id).await.is_empty());
    assert_eq!(
        stored_title_tags(&app, &full.id).await,
        labels[..crate::MAX_USER_TAGS_PER_TITLE]
    );
}

#[tokio::test]
async fn bulk_series_movie_tag_ceiling_preflight_leaves_every_link_unchanged() {
    let (app, user) = bootstrap();
    let (_, first) = create_series_with_movie(&app, &user, "First").await;
    let (_, full) = create_series_with_movie(&app, &user, "Full").await;
    let labels = seed_ceiling_labels(&app, &user).await;
    app.update_series_movie_tags(
        &user,
        &[full.id.clone()],
        &labels[..crate::MAX_USER_TAGS_PER_TITLE],
        &[],
    )
    .await
    .unwrap();
    assert!(
        app.update_series_movie_tags(
            &user,
            &[first.id.clone(), full.id.clone()],
            &labels[crate::MAX_USER_TAGS_PER_TITLE..],
            &[]
        )
        .await
        .is_err()
    );
    assert!(stored_series_movie_tags(&app, &first.id).await.is_empty());
    assert_eq!(
        stored_series_movie_tags(&app, &full.id).await,
        labels[..crate::MAX_USER_TAGS_PER_TITLE]
    );
}

async fn seed_ceiling_labels(app: &AppUseCase, user: &User) -> Vec<String> {
    let labels = (0..=crate::MAX_USER_TAGS_PER_TITLE)
        .map(|index| format!("tag {index}"))
        .collect::<Vec<_>>();
    for label in &labels {
        app.create_title_tag_definition(user, label, None)
            .await
            .unwrap();
    }
    labels
}

struct RenameFailureSettings {
    inner: Arc<dyn SettingsRepository>,
    fail_delay_write: std::sync::atomic::AtomicBool,
    fail_journal_clear: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl SettingsRepository for RenameFailureSettings {
    async fn get_setting_json(
        &self,
        scope: &str,
        key: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.inner.get_setting_json(scope, key, scope_id).await
    }
    async fn upsert_setting_json(
        &self,
        scope: &str,
        key: &str,
        scope_id: Option<String>,
        value: String,
        source: &str,
        actor: Option<String>,
    ) -> AppResult<()> {
        use std::sync::atomic::Ordering;
        if (key == crate::delay_profile::DELAY_PROFILE_CATALOG_KEY
            && self.fail_delay_write.load(Ordering::SeqCst))
            || (key == "title_tags.pending_renames"
                && value == "{}"
                && self.fail_journal_clear.load(Ordering::SeqCst))
        {
            return Err(AppError::Repository(
                "synthetic rename settings write failure".into(),
            ));
        }
        self.inner
            .upsert_setting_json(scope, key, scope_id, value, source, actor)
            .await
    }
    async fn delete_setting_value(
        &self,
        scope: &str,
        key: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.inner.delete_setting_value(scope, key, scope_id).await
    }
    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        self.inner.delete_values_for_scope_id(scope_id).await
    }
}

#[tokio::test]
async fn title_tag_rename_recovers_each_cross_store_failure_on_retry_or_startup() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for startup in [false, true] {
        for failure_stage in 0..3 {
            let mut h = bootstrap_media_request_app();
            let settings = Arc::new(RenameFailureSettings {
                inner: h.app.services.config.settings.clone(),
                fail_delay_write: AtomicBool::new(false),
                fail_journal_clear: AtomicBool::new(false),
            });
            h.app = h
                .app
                .with_test_overrides(|builder| builder.with_settings(settings.clone()));
            let definition = h
                .app
                .create_title_tag_definition(&h.manager, "before", None)
                .await
                .unwrap();
            h.app
                .upsert_delay_profile(&h.manager, tagged_delay_profile(vec!["before".into()]))
                .await
                .unwrap();
            let library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
            h.app
                .submit_media_request(&h.user, media_request_input(library, 4242))
                .await
                .unwrap();
            h.media_requests.requests.lock().await[0].policy_tags = vec!["before".into()];
            match failure_stage {
                0 => settings.fail_delay_write.store(true, Ordering::SeqCst),
                1 => h
                    .media_requests
                    .fail_policy_tag_rewrites
                    .store(true, Ordering::SeqCst),
                _ => settings.fail_journal_clear.store(true, Ordering::SeqCst),
            }
            assert!(
                h.app
                    .update_title_tag_definition(
                        &h.manager,
                        &definition.id,
                        Some("after".into()),
                        None
                    )
                    .await
                    .is_err()
            );
            assert_eq!(
                h.app
                    .services
                    .catalog
                    .titles
                    .get_title_tag_definition(&definition.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .label,
                "after"
            );
            let pending = h
                .app
                .read_setting_json_value::<serde_json::Value>("title_tags.pending_renames", None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(pending[&definition.id]["previous_label"], "before");
            // Reusing the old label cannot race ahead of unfinished rewrites.
            assert!(
                h.app
                    .create_title_tag_definition(&h.manager, "before", None)
                    .await
                    .is_err()
            );
            settings.fail_delay_write.store(false, Ordering::SeqCst);
            settings.fail_journal_clear.store(false, Ordering::SeqCst);
            h.media_requests
                .fail_policy_tag_rewrites
                .store(false, Ordering::SeqCst);
            if startup {
                h.app.resume_pending_title_tag_renames().await.unwrap();
            } else {
                h.app
                    .update_title_tag_definition(
                        &h.manager,
                        &definition.id,
                        Some("after".into()),
                        None,
                    )
                    .await
                    .unwrap();
            }
            assert_eq!(
                h.app.get_delay_profiles(&h.manager).await.unwrap()[0].tags,
                ["after"]
            );
            assert_eq!(
                h.media_requests.requests.lock().await[0].policy_tags,
                ["after"]
            );
            assert_eq!(
                h.app
                    .read_setting_json_value::<serde_json::Value>(
                        "title_tags.pending_renames",
                        None
                    )
                    .await
                    .unwrap(),
                Some(serde_json::json!({}))
            );
            h.app.resume_pending_title_tag_renames().await.unwrap();
            h.app
                .create_title_tag_definition(&h.manager, "before", None)
                .await
                .unwrap();
        }
    }
}
