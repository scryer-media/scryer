//! The per-title route gate for downloader observations
//! (`completed_download_allows_automatic_import`): a category Scryer knows
//! (admitted) must also be the category the title's *effective* route would
//! submit with — library-scoped routing shadows facet routing, a disabled or
//! missing route means no automatic import — or the download waits for Manual
//! Import.

use super::*;
use crate::{
    DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
    SETTINGS_SCOPE_SYSTEM,
};
use std::collections::HashMap;

type RoutingSettingKey = (String, String, Option<String>);
type RoutingSettingValues = Arc<Mutex<HashMap<RoutingSettingKey, String>>>;

/// Scoped settings values keyed by (scope, key, scope_id).
#[derive(Default)]
pub(super) struct RoutingSettingsRepo {
    values: RoutingSettingValues,
}

impl RoutingSettingsRepo {
    async fn set_scoped_json(&self, key_name: &str, scope_id: &str, value_json: &str) {
        self.values.lock().await.insert(
            (
                SETTINGS_SCOPE_SYSTEM.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ),
            value_json.to_string(),
        );
    }

    pub(super) async fn set_routing(&self, scope_id: &str, routing_json: &str) {
        self.set_scoped_json(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, scope_id, routing_json)
            .await;
    }

    async fn set_default_category(&self, scope_id: &str, category: &str) {
        self.set_scoped_json(
            DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
            scope_id,
            &serde_json::json!(category).to_string(),
        )
        .await;
    }
}

#[async_trait]
impl SettingsRepository for RoutingSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), scope_id))
            .cloned())
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.get_setting_json(scope, key_name, scope_id).await
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        _source: &str,
        _updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), scope_id),
            value_json,
        );
        Ok(())
    }

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.values
            .lock()
            .await
            .remove(&(scope.to_string(), key_name.to_string(), scope_id));
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let mut values = self.values.lock().await;
        let before = values.len();
        values.retain(|(_, _, stored_scope_id), _| stored_scope_id.as_deref() != Some(scope_id));
        Ok((before - values.len()) as u32)
    }
}

/// Runs the completed check for a parse-matched observation of "Paper Lantern"
/// (movie, default movie library, client `client-1`) whose completed history
/// entry carries `completed_category`, against the given routing settings.
async fn run_route_gate_check(
    settings: Arc<RoutingSettingsRepo>,
    completed_category: Option<&str>,
) -> TrackedDownload {
    // The facet default category is what admission knows when nothing else
    // is configured; the route itself may still disagree with it.
    if settings
        .get_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
            Some("movie".to_string()),
        )
        .await
        .expect("settings read")
        .is_none()
    {
        settings.set_default_category("movie", "movie").await;
    }

    let temp_dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        temp_dir.path().join("Paper.Lantern.2012.1080p.WEB-DL.mkv"),
        b"video",
    )
    .expect("write fixture video");
    let mut completed = build_completed_download(
        "downloader display label",
        temp_dir.path().to_string_lossy().as_ref(),
        completed_category,
    );
    completed.release_name = Some("Paper.Lantern.2012.1080p.WEB-DL".to_string());
    let app = build_app_with_download_client_configs_submissions_and_settings(
        vec![build_title("title-1", "Paper Lantern", MediaFacet::Movie)],
        vec![],
        vec![],
        vec![],
        TestAppRepositories {
            download_client: test_download_client_with_completed(completed),
            download_client_configs: Arc::new(TestDownloadClientConfigRepo {
                configs: vec![DownloadClientConfig {
                    id: "client-1".to_string(),
                    name: "Test NZBGet".to_string(),
                    client_type: "nzbget".to_string(),
                    config_json: "{}".to_string(),
                    is_enabled: true,
                    status: scryer_domain::DownloadClientStatus::Healthy,
                    last_error: None,
                    last_seen_at: None,
                    client_priority: 0,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    proxy_config_id: None,
                }],
            }),
            download_submissions: Arc::new(
                crate::null_repositories::NullDownloadSubmissionRepository,
            ),
            settings,
        },
    );
    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p.WEB-DL");
    td.client_item.category = completed_category.map(str::to_string);
    td.client_item.is_scryer_origin = false;
    td.match_type = TitleMatchType::TitleParse;

    check(&app, &mut td).await;
    td
}

fn assert_blocked_for_route_mismatch(td: &TrackedDownload, case: &str) {
    assert_eq!(td.state, TrackedDownloadState::ImportBlocked, "{case}");
    assert!(
        td.status_messages
            .iter()
            .any(|message| message.contains("does not match this title's active route")),
        "{case}: {:?}",
        td.status_messages
    );
}

#[tokio::test]
async fn route_gate_honors_facet_routing_category() {
    let settings = Arc::new(RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;

    let td = run_route_gate_check(settings.clone(), Some("Facet Movies")).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);

    // Known to Scryer (the facet default) but not this title's route.
    let td = run_route_gate_check(settings, Some("movie")).await;
    assert_blocked_for_route_mismatch(&td, "facet default when the route says otherwise");
}

#[tokio::test]
async fn route_gate_library_routing_shadows_facet_routing() {
    let settings = Arc::new(RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;
    settings
        .set_routing(
            "movie_default_library",
            r#"{"client-1":{"enabled":true,"category":"Library Movies"}}"#,
        )
        .await;

    let td = run_route_gate_check(settings.clone(), Some("Facet Movies")).await;
    assert_blocked_for_route_mismatch(&td, "shadowed facet category");
    let td = run_route_gate_check(settings, Some("Library Movies")).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn route_gate_empty_library_category_falls_back_to_the_facet_default() {
    let settings = Arc::new(RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;
    settings
        .set_routing(
            "movie_default_library",
            r#"{"client-1":{"enabled":true,"category":""}}"#,
        )
        .await;

    // The library route exists (enabled) but names no category: the effective
    // category is the facet DEFAULT (`movie`), not the shadowed facet route.
    let td = run_route_gate_check(settings.clone(), Some("Facet Movies")).await;
    assert_blocked_for_route_mismatch(&td, "shadowed facet route with empty library category");
    let td = run_route_gate_check(settings, Some("movie")).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn route_gate_blocks_when_the_client_is_missing_from_the_library_route() {
    let settings = Arc::new(RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"category":"movie"}}"#,
        )
        .await;
    settings
        .set_routing(
            "movie_default_library",
            r#"{"other-client":{"enabled":true,"category":"movie"}}"#,
        )
        .await;

    // A library-scoped routing object shadows facet routing completely; a
    // client it omits is disabled for the title.
    let td = run_route_gate_check(settings, Some("movie")).await;
    assert_blocked_for_route_mismatch(&td, "client omitted from the library route");
}

#[tokio::test]
async fn route_gate_missing_client_in_facet_route_uses_the_facet_default() {
    let settings = Arc::new(RoutingSettingsRepo::default());
    settings
        .set_routing(
            "movie",
            r#"{"other-client":{"enabled":true,"category":"other"}}"#,
        )
        .await;

    // No library route and no facet entry for this client: the effective
    // category is the facet default, so `movie` still routes.
    let td = run_route_gate_check(settings, Some("movie")).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}

#[tokio::test]
async fn route_gate_blocks_disabled_routes_at_either_scope() {
    for scope_id in ["movie_default_library", "movie"] {
        let settings = Arc::new(RoutingSettingsRepo::default());
        settings
            .set_routing(
                scope_id,
                r#"{"client-1":{"enabled":false,"category":"movie"}}"#,
            )
            .await;

        let td = run_route_gate_check(settings, Some("movie")).await;
        assert_blocked_for_route_mismatch(&td, &format!("disabled route at {scope_id}"));
    }
}

#[tokio::test]
async fn route_gate_ignores_an_invalid_library_route_and_uses_the_facet_route() {
    let settings = Arc::new(RoutingSettingsRepo::default());
    settings
        .set_routing("movie_default_library", "not-json")
        .await;
    settings
        .set_routing(
            "movie",
            r#"{"client-1":{"enabled":true,"category":"Facet Movies"}}"#,
        )
        .await;

    let td = run_route_gate_check(settings, Some("Facet Movies")).await;
    assert_eq!(td.state, TrackedDownloadState::ImportPending);
}
