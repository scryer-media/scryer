#![recursion_limit = "256"]

mod common;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use scryer_application::{
    AppError, AppResult, InsertMediaFileInput, MediaFileAnalysis, MediaFileRepository,
    PendingRelease, ReleaseDecision, ShowRepository, TitleEpisodeProgressSummary, TitleMediaFile,
    TitleMediaSizeSummary, TitleRepository, WantedItem,
};
use scryer_domain::{Collection, CollectionType, Episode, ExternalId, Id, MediaFacet, Title};
use scryer_infrastructure::{FileSystemLibraryRenamer, SettingDefinitionSeed, SqliteServices};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use common::{TestContext, load_fixture};

/// Execute a GraphQL operation directly against the schema, without going
/// through the HTTP test server.  This gives full control over what data
/// (e.g. `User`) is attached to the request.
async fn schema_exec(ctx: &TestContext, query: &str, user: Option<scryer_domain::User>) -> Value {
    let mut req = async_graphql::Request::new(query);
    if let Some(u) = user {
        req = req.data(u);
    }
    let resp = ctx.schema.execute(req).await;
    serde_json::to_value(&resp).expect("serialize gql response")
}

/// Helper to execute a GraphQL query and return the parsed JSON body.
async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

/// Assert no GraphQL errors in response body.
fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

async fn set_rename_collision_policy(ctx: &TestContext, scope: &str, policy: &str) {
    let body = gql(
        ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            renameCollisionPolicy
          }
        }
        "#,
        json!({
            "input": {
                "scope": scope,
                "renameCollisionPolicy": policy
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["updateMediaSettings"]["renameCollisionPolicy"],
        policy
    );
}

struct FailingMediaFileRepo {
    inner: SqliteServices,
    fail_file_id: String,
}

#[async_trait]
impl MediaFileRepository for FailingMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        <SqliteServices as MediaFileRepository>::insert_media_file(&self.inner, input).await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        <SqliteServices as MediaFileRepository>::link_file_to_episode(
            &self.inner,
            file_id,
            episode_id,
        )
        .await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        <SqliteServices as MediaFileRepository>::list_media_files_for_title(&self.inner, title_id)
            .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        <SqliteServices as MediaFileRepository>::list_title_media_size_summaries(
            &self.inner,
            title_ids,
        )
        .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        <SqliteServices as MediaFileRepository>::list_title_episode_progress_summaries(
            &self.inner,
            title_ids,
        )
        .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        <SqliteServices as MediaFileRepository>::update_media_file_analysis(
            &self.inner,
            file_id,
            analysis,
        )
        .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        <SqliteServices as MediaFileRepository>::update_media_file_source_signature(
            &self.inner,
            file_id,
            size_bytes,
            source_signature_scheme,
            source_signature_value,
        )
        .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        if file_id == self.fail_file_id {
            return Err(AppError::Repository(format!(
                "injected media file path failure for {file_id} -> {file_path}"
            )));
        }

        <SqliteServices as MediaFileRepository>::update_media_file_path(
            &self.inner,
            file_id,
            file_path,
        )
        .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        <SqliteServices as MediaFileRepository>::mark_scan_failed(&self.inner, file_id, error).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        <SqliteServices as MediaFileRepository>::get_media_file_by_id(&self.inner, file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        <SqliteServices as MediaFileRepository>::get_media_file_by_path(&self.inner, file_path)
            .await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        <SqliteServices as MediaFileRepository>::delete_media_file(&self.inner, file_id).await
    }
}

struct FailingShowRepo {
    inner: SqliteServices,
    fail_collection_id: String,
    fail_path: String,
}

#[async_trait]
impl ShowRepository for FailingShowRepo {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        <SqliteServices as ShowRepository>::list_collections_for_title(&self.inner, title_id).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        <SqliteServices as ShowRepository>::get_collection_by_id(&self.inner, collection_id).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        <SqliteServices as ShowRepository>::get_collection_by_ordered_path(
            &self.inner,
            ordered_path,
        )
        .await
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        <SqliteServices as ShowRepository>::create_collection(&self.inner, collection).await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        collection_type: Option<CollectionType>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection> {
        if collection_id == self.fail_collection_id
            && ordered_path.as_deref() == Some(self.fail_path.as_str())
        {
            return Err(AppError::Repository(format!(
                "injected collection path failure for {collection_id}"
            )));
        }

        <SqliteServices as ShowRepository>::update_collection(
            &self.inner,
            collection_id,
            collection_type,
            collection_index,
            label,
            ordered_path,
            first_episode_number,
            last_episode_number,
            monitored,
        )
        .await
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        <SqliteServices as ShowRepository>::update_interstitial_season_episode(
            &self.inner,
            collection_id,
            season_episode,
        )
        .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        <SqliteServices as ShowRepository>::set_collection_episodes_monitored(
            &self.inner,
            collection_id,
            monitored,
        )
        .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        <SqliteServices as ShowRepository>::delete_collection(&self.inner, collection_id).await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        <SqliteServices as ShowRepository>::delete_collections_for_title(&self.inner, title_id)
            .await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        <SqliteServices as ShowRepository>::list_episodes_for_collection(&self.inner, collection_id)
            .await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        <SqliteServices as ShowRepository>::list_episodes_for_title(&self.inner, title_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        <SqliteServices as ShowRepository>::get_episode_by_id(&self.inner, episode_id).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        <SqliteServices as ShowRepository>::create_episode(&self.inner, episode).await
    }

    async fn update_episode(
        &self,
        episode_id: &str,
        episode_type: Option<scryer_domain::EpisodeType>,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: Option<bool>,
        has_subtitle: Option<bool>,
        monitored: Option<bool>,
        collection_id: Option<String>,
        overview: Option<String>,
        tvdb_id: Option<String>,
    ) -> AppResult<Episode> {
        <SqliteServices as ShowRepository>::update_episode(
            &self.inner,
            episode_id,
            episode_type,
            episode_number,
            season_number,
            episode_label,
            title,
            air_date,
            duration_seconds,
            has_multi_audio,
            has_subtitle,
            monitored,
            collection_id,
            overview,
            tvdb_id,
        )
        .await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        <SqliteServices as ShowRepository>::delete_episode(&self.inner, episode_id).await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        <SqliteServices as ShowRepository>::delete_episodes_for_title(&self.inner, title_id).await
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        <SqliteServices as ShowRepository>::find_episode_by_title_and_numbers(
            &self.inner,
            title_id,
            season_number,
            episode_number,
        )
        .await
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        <SqliteServices as ShowRepository>::find_episode_by_title_and_absolute_number(
            &self.inner,
            title_id,
            absolute_number,
        )
        .await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_inclusive: &str,
        end_inclusive: &str,
    ) -> AppResult<Vec<scryer_domain::CalendarEpisode>> {
        <SqliteServices as ShowRepository>::list_episodes_in_date_range(
            &self.inner,
            start_inclusive,
            end_inclusive,
        )
        .await
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::PrimaryCollectionSummary>> {
        <SqliteServices as ShowRepository>::list_primary_collection_summaries(
            &self.inner,
            title_ids,
        )
        .await
    }
}

/// Helper to add a title and return the title ID.
async fn add_test_title(ctx: &TestContext, name: &str, facet: &str) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) { addTitle(input: $input) { title { id name } } }"#,
        json!({
            "input": {
                "name": name,
                "facet": facet,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "999" }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn seed_typed_settings_definitions(ctx: &TestContext) {
    ctx.db
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_api_key".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_username".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.opensubtitles_password".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: true,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.languages".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.auto_download_on_import".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.minimum_score_series".into(),
                data_type: "number".into(),
                default_value_json: "240".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.minimum_score_movie".into(),
                data_type: "number".into(),
                default_value_json: "70".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.search_interval_hours".into(),
                data_type: "number".into(),
                default_value_json: "6".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.include_ai_translated".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.include_machine_translated".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_threshold_series".into(),
                data_type: "number".into(),
                default_value_json: "90".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_threshold_movie".into(),
                data_type: "number".into(),
                default_value_json: "70".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "subtitles".into(),
                scope: "system".into(),
                key_name: "subtitles.sync_max_offset_seconds".into(),
                data_type: "number".into(),
                default_value_json: "60".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.enabled".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.upgrade_cooldown_hours".into(),
                data_type: "number".into(),
                default_value_json: "24".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.same_tier_min_delta".into(),
                data_type: "number".into(),
                default_value_json: "120".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.cross_tier_min_delta".into(),
                data_type: "number".into(),
                default_value_json: "30".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.forced_upgrade_delta_bypass".into(),
                data_type: "number".into(),
                default_value_json: "400".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.poll_interval_seconds".into(),
                data_type: "number".into(),
                default_value_json: "60".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.sync_interval_seconds".into(),
                data_type: "number".into(),
                default_value_json: "3600".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.batch_size".into(),
                data_type: "number".into(),
                default_value_json: "50".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "acquisition".into(),
                scope: "system".into(),
                key_name: "acquisition.delay_profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profiles".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "quality.profile_id".into(),
                data_type: "string".into(),
                default_value_json: "\"4k\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "download_client.routing".into(),
                data_type: "json".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "indexer.routing".into(),
                data_type: "json".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/movies\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/series\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/anime\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.root_folders".into(),
                data_type: "json".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"{title} ({year}) - {quality}.{ext}\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.series.global".into(),
                data_type: "string".into(),
                default_value_json:
                    "\"{title} - S{season:2}E{episode:2} - {quality}.{ext}\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.template.anime.global".into(),
                data_type: "string".into(),
                default_value_json:
                    "\"{title} - S{season_order:2}E{episode:2} ({absolute_episode}) - {quality}.{ext}\""
                        .into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy.global".into(),
                data_type: "string".into(),
                default_value_json: "\"skip\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.collision_policy.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"skip\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy.global".into(),
                data_type: "string".into(),
                default_value_json: "\"fallback_title\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "rename.missing_metadata_policy.movie.global".into(),
                data_type: "string".into(),
                default_value_json: "\"fallback_title\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.filler_policy".into(),
                data_type: "string".into(),
                default_value_json: "\"download_all\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.recap_policy".into(),
                data_type: "string".into(),
                default_value_json: "\"download_all\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.monitor_specials".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.inter_season_movies".into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "anime.monitor_filler_movies".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.movie".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.series".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "nfo.write_on_import.anime".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "plexmatch.write_on_import.series".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "system".into(),
                key_name: "plexmatch.write_on_import.anime".into(),
                data_type: "boolean".into(),
                default_value_json: "false".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "service".into(),
                scope: "system".into(),
                key_name: "tls.cert_path".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "service".into(),
                scope: "system".into(),
                key_name: "tls.key_path".into(),
                data_type: "string".into(),
                default_value_json: "null".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("settings definitions should seed");
}

async fn mount_smg_mocks(ctx: &TestContext, fixture_path: &str) {
    let fixture = load_fixture(fixture_path);
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;
}

async fn create_series_scan_title(
    ctx: &TestContext,
    media_root: &std::path::Path,
    name: &str,
    extra_tags: Vec<String>,
) -> (Title, Collection) {
    let mut tags = vec![format!("scryer:root-folder:{}", media_root.display())];
    tags.extend(extra_tags);

    let title = Title {
        id: Id::new().0,
        name: name.to_string(),
        facet: MediaFacet::Series,
        monitored: true,
        tags,
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: Some(24),
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };
    let title = ctx.db.create(title).await.expect("create series title");

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("10".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .db
        .create_collection(collection)
        .await
        .expect("create season collection");

    (title, collection)
}

async fn create_catalog_title(
    ctx: &TestContext,
    name: &str,
    facet: MediaFacet,
    external_ids: Vec<ExternalId>,
    tags: Vec<String>,
    monitored: bool,
) -> Title {
    let title = Title {
        id: Id::new().0,
        name: name.to_string(),
        facet,
        monitored,
        tags,
        external_ids,
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: Some("Original overview".to_string()),
        poster_url: Some("https://example.com/old-poster.jpg".to_string()),
        poster_source_url: None,
        banner_url: Some("https://example.com/old-banner.jpg".to_string()),
        banner_source_url: None,
        background_url: Some("https://example.com/old-background.jpg".to_string()),
        background_source_url: None,
        sort_title: Some(name.to_string()),
        slug: Some("old-slug".to_string()),
        imdb_id: Some("tt0000001".to_string()),
        runtime_minutes: Some(100),
        genres: vec!["Drama".to_string()],
        content_status: Some("ended".to_string()),
        language: Some("eng".to_string()),
        first_aired: Some("2020-01-01".to_string()),
        network: Some("Old Network".to_string()),
        studio: Some("Old Studio".to_string()),
        country: Some("usa".to_string()),
        aliases: vec!["Legacy Alias".to_string()],
        tagged_aliases: vec![],
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(Utc::now()),
        min_availability: None,
        digital_release_date: Some("2020-01-01".to_string()),
        folder_path: None,
    };

    ctx.db.create(title).await.expect("create title")
}

#[derive(Debug, PartialEq, Eq)]
struct SeriesMonitoringSummary {
    title_monitored: bool,
    collection_monitored: bool,
    episode_monitored: bool,
    wanted_count: i64,
}

async fn create_series_monitoring_fixture(
    ctx: &TestContext,
    name: &str,
    tvdb_id: &str,
) -> (Title, Collection, Episode) {
    let title = create_catalog_title(
        ctx,
        name,
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .db
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: Some("Pilot episode".to_string()),
            tvdb_id: Some(format!("{tvdb_id}01")),
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    (title, collection, episode)
}

async fn series_monitoring_summary(
    ctx: &TestContext,
    title_id: &str,
    collection_id: &str,
    episode_id: &str,
) -> SeriesMonitoringSummary {
    let title = ctx
        .db
        .get_by_id(title_id)
        .await
        .expect("load title")
        .expect("title");
    let collection = ctx
        .db
        .get_collection_by_id(collection_id)
        .await
        .expect("load collection")
        .expect("collection");
    let episode = ctx
        .db
        .get_episode_by_id(episode_id)
        .await
        .expect("load episode")
        .expect("episode");
    let wanted_count = ctx
        .db
        .count_wanted_items(None, None, Some(title_id))
        .await
        .expect("count wanted items");

    SeriesMonitoringSummary {
        title_monitored: title.monitored,
        collection_monitored: collection.monitored,
        episode_monitored: episode.monitored,
        wanted_count,
    }
}

async fn activity_kinds_for_title(ctx: &TestContext, title_id: &str) -> Vec<String> {
    let body = gql(&ctx, "{ activityEvents { kind titleId } }", json!({})).await;
    assert_no_errors(&body);

    body["data"]["activityEvents"]
        .as_array()
        .expect("activity events array")
        .iter()
        .filter(|event| event["titleId"] == title_id)
        .filter_map(|event| event["kind"].as_str())
        .map(str::to_string)
        .collect()
}

async fn create_series_scan_episode(
    ctx: &TestContext,
    title: &Title,
    collection: &Collection,
    season_number: &str,
    episode_number: &str,
    label: &str,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some(episode_number.to_string()),
        season_number: Some(season_number.to_string()),
        episode_label: Some(label.to_string()),
        title: Some(format!("Episode {episode_number}")),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.db
        .create_episode(episode)
        .await
        .expect("create episode")
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_uses_media_file_rows() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Rename Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "91001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .db
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Arrival".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("12".to_string()),
            overview: None,
            tvdb_id: Some("9100103".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Rename Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("[SubsPlease] Rename Preview Show - 03 (1080p).mkv");
    std::fs::write(&file_path, b"anime-preview").expect("write preview file");

    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            noop
            conflicts
            errors
            items {
              collectionId
              mediaFileId
              currentPath
              proposedPath
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(plan["noop"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(0));
    assert_eq!(plan["errors"].as_i64(), Some(0));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], Value::Null);
    assert_eq!(item["mediaFileId"], json!(file_id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            season_dir
                .join("Rename Preview Show - S01E03 (012) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
    assert_eq!(item["reasonCode"], "rename_move");
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_interstitial_uses_season_zero_numbering() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Festival Saga",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "92001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_zero_dir = media_root.path().join("Festival Saga").join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let file_path = season_zero_dir.join("Festival.Saga.Movie.Special.1080p.mkv");
    std::fs::write(&file_path, b"anime-interstitial").expect("write interstitial file");

    let interstitial = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "9200103".to_string(),
                name: "Festival Film".to_string(),
                slug: "festival-film".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Festival special".to_string(),
                poster_url: "https://example.com/festival-film.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 95,
                sort_title: "Festival Film".to_string(),
                imdb_id: "tt9200103".to_string(),
                genres: vec!["Fantasy".to_string()],
                studio: "Scryer Films".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                association_confidence: None,
                continuity_status: None,
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial file");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              collectionId
              mediaFileId
              currentPath
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], json!(interstitial.id));
    assert_eq!(item["mediaFileId"], json!(file_id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            season_zero_dir
                .join("Festival Saga - S00E03 (3) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
}

#[tokio::test]
async fn apply_media_rename_for_anime_updates_media_files_and_only_interstitial_collections() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app.services.library_renamer = std::sync::Arc::new(FileSystemLibraryRenamer::new());
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Anime Apply Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "93001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .db
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("9300101".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root.path().join("Anime Apply Show").join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let regular_file_path = season_dir.join("Anime.Apply.Show.Episode.One.1080p.mkv");
    std::fs::write(&regular_file_path, b"anime-apply-episode").expect("write regular file");

    let regular_file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: regular_file_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert regular file");
    ctx.db
        .link_file_to_episode(&regular_file_id, &episode.id)
        .await
        .expect("link regular file");

    let season_zero_dir = media_root.path().join("Anime Apply Show").join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let interstitial_file_path = season_zero_dir.join("Anime.Apply.Show.Movie.Special.1080p.mkv");
    std::fs::write(&interstitial_file_path, b"anime-apply-interstitial")
        .expect("write interstitial file");

    let interstitial_collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: Some(interstitial_file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "9300103".to_string(),
                name: "Pilot Movie".to_string(),
                slug: "pilot-movie".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Pilot side story".to_string(),
                poster_url: "https://example.com/pilot-movie.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 90,
                sort_title: "Pilot Movie".to_string(),
                imdb_id: "tt9300103".to_string(),
                genres: vec!["Adventure".to_string()],
                studio: "Scryer Films".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                association_confidence: None,
                continuity_status: None,
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let interstitial_file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: interstitial_file_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 2);

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 2);
    assert_eq!(result.failed, 0);

    let expected_regular_path = season_dir
        .join("Anime Apply Show - S01E01 (001) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let expected_interstitial_path = season_zero_dir
        .join("Anime Apply Show - S00E03 (3) - 1080p.mkv")
        .to_string_lossy()
        .to_string();

    let updated_regular_file = ctx
        .db
        .get_media_file_by_id(&regular_file_id)
        .await
        .expect("load updated regular media file")
        .expect("regular media file");
    let updated_interstitial_file = ctx
        .db
        .get_media_file_by_id(&interstitial_file_id)
        .await
        .expect("load updated interstitial media file")
        .expect("interstitial media file");
    let refreshed_season_collection = ctx
        .db
        .get_collection_by_id(&season_collection.id)
        .await
        .expect("load season collection")
        .expect("season collection");
    let refreshed_interstitial_collection = ctx
        .db
        .get_collection_by_id(&interstitial_collection.id)
        .await
        .expect("load interstitial collection")
        .expect("interstitial collection");

    assert_eq!(updated_regular_file.file_path, expected_regular_path);
    assert_eq!(
        updated_interstitial_file.file_path,
        expected_interstitial_path
    );
    assert_eq!(refreshed_season_collection.ordered_path, None);
    assert_eq!(
        refreshed_interstitial_collection.ordered_path,
        Some(expected_interstitial_path.clone())
    );
    assert!(std::path::Path::new(&expected_regular_path).exists());
    assert!(std::path::Path::new(&expected_interstitial_path).exists());
    assert!(!regular_file_path.exists());
    assert!(!interstitial_file_path.exists());
}

#[tokio::test]
async fn graphql_media_rename_preview_for_movies_stays_collection_based() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Regression Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Regression Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let file_path = movie_dir.join("Regression.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"movie-rename-preview").expect("write movie file");

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(file_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              collectionId
              mediaFileId
              currentPath
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "movie",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));

    let item = &plan["items"][0];
    assert_eq!(item["collectionId"], json!(collection.id));
    assert_eq!(item["mediaFileId"], json!(file_id));
    assert_eq!(
        item["currentPath"],
        json!(file_path.to_string_lossy().to_string())
    );
    assert_eq!(
        item["proposedPath"],
        json!(
            movie_dir
                .join("Regression Movie (2024) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
    assert_eq!(item["writeAction"], "move");
}

#[tokio::test]
async fn apply_media_rename_for_movies_updates_collection_and_media_file_paths() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app.services.library_renamer = std::sync::Arc::new(FileSystemLibraryRenamer::new());
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Movie Apply Sync (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "94002".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Movie Apply Sync (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let source_path = movie_dir.join("Movie.Apply.Sync.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"movie-apply-sync").expect("write movie file");

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");
    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 8192,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert movie media file");

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Movie)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert_eq!(
        preview.items[0].media_file_id.as_deref(),
        Some(file_id.as_str())
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Movie, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 1);
    assert_eq!(result.failed, 0);

    let expected_path = movie_dir
        .join("Movie Apply Sync (2024) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let updated_collection = ctx
        .db
        .get_collection_by_id(&collection.id)
        .await
        .expect("load movie collection")
        .expect("movie collection");
    let updated_file = ctx
        .db
        .get_media_file_by_id(&file_id)
        .await
        .expect("load movie media file")
        .expect("movie media file");

    assert_eq!(
        updated_collection.ordered_path.as_deref(),
        Some(expected_path.as_str())
    );
    assert_eq!(updated_file.file_path, expected_path);
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_tracked_destination_returns_error_not_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "anime", "replace_if_better").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Tracked Collision Anime",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "95001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("3".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .db
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: Some("Arrival".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("12".to_string()),
            overview: None,
            tvdb_id: Some("9500103".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Tracked Collision Anime")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let source_path = season_dir.join("[SubsPlease] Tracked Collision Anime - 03 (1080p).mkv");
    std::fs::write(&source_path, b"tracked-collision-source").expect("write source file");
    let destination_path = season_dir.join("Tracked Collision Anime - S01E03 (012) - 1080p.mkv");

    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert source media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let owning_title = create_catalog_title(
        &ctx,
        "Tracked Collision Owner",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "95002".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    ctx.db
        .insert_media_file(&InsertMediaFileInput {
            title_id: owning_title.id,
            file_path: destination_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert tracked destination");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing_tracked");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_movies_tracked_destination_returns_error_not_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "movie", "replace_if_better").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Tracked Collision Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "96001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Tracked Collision Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let source_path = movie_dir.join("Tracked.Collision.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"tracked-movie-source").expect("write movie source");
    let destination_path = movie_dir.join("Tracked Collision Movie (2024) - 1080p.mkv");

    ctx.db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    let owning_title = create_catalog_title(
        &ctx,
        "Tracked Collision Owner Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "96002".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    ctx.db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: owning_title.id,
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(destination_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create tracked destination collection");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "movie",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing_tracked");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_anime_multi_episode_file_uses_episode_range() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Range Preview Show",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "97002".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");
    let episode_one =
        create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    let episode_two =
        create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let season_dir = media_root
        .path()
        .join("Range Preview Show")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Range.Preview.Show.S01E01-E02.1080p.mkv");
    std::fs::write(&file_path, b"anime-range-preview").expect("write preview file");

    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4096,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode_one.id)
        .await
        .expect("link first episode");
    ctx.db
        .link_file_to_episode(&file_id, &episode_two.id)
        .await
        .expect("link second episode");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            items {
              mediaFileId
              proposedPath
              writeAction
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "anime",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["mediaFileId"], json!(file_id));
    assert_eq!(plan["items"][0]["writeAction"], "move");
    assert_eq!(
        plan["items"][0]["proposedPath"],
        json!(
            season_dir
                .join("Range Preview Show - S01E01-02 (01-02) - 1080p.mkv")
                .to_string_lossy()
                .to_string()
        )
    );
}

#[tokio::test]
async fn graphql_media_rename_preview_for_untracked_existing_target_does_not_emit_replace() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    set_rename_collision_policy(&ctx, "movie", "replace_if_better").await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Untracked Collision Movie (2024)",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "97001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let movie_dir = media_root.path().join("Untracked Collision Movie (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let source_path = movie_dir.join("Untracked.Collision.Movie.2024.1080p.WEB-DL.mkv");
    std::fs::write(&source_path, b"untracked-movie-source").expect("write movie source");
    let destination_path = movie_dir.join("Untracked Collision Movie (2024) - 1080p.mkv");
    std::fs::write(&destination_path, b"untracked-movie-destination")
        .expect("write untracked destination");

    ctx.db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("1080p".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    let body = gql(
        &ctx,
        r#"
        query($input: MediaRenamePreviewInput!) {
          mediaRenamePreview(input: $input) {
            total
            renamable
            conflicts
            errors
            items {
              writeAction
              reasonCode
            }
          }
        }
        "#,
        json!({
            "input": {
                "facet": "movie",
                "titleId": title.id,
                "dryRun": true
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let plan = &body["data"]["mediaRenamePreview"];
    assert_eq!(plan["total"].as_i64(), Some(1));
    assert_eq!(plan["renamable"].as_i64(), Some(0));
    assert_eq!(plan["conflicts"].as_i64(), Some(1));
    assert_eq!(plan["errors"].as_i64(), Some(1));
    assert_eq!(plan["items"][0]["writeAction"], "error");
    assert_eq!(plan["items"][0]["reasonCode"], "collision_existing");
    assert!(
        plan["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["writeAction"] != "replace")
    );
}

#[tokio::test]
async fn apply_media_rename_for_anime_rolls_back_when_media_file_update_fails() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app.services.library_renamer = std::sync::Arc::new(FileSystemLibraryRenamer::new());
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Anime Media Rollback",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "98001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let episode = ctx
        .db
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("1".to_string()),
            overview: None,
            tvdb_id: Some("9800101".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create episode");

    let season_dir = media_root
        .path()
        .join("Anime Media Rollback")
        .join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let source_path = season_dir.join("Anime.Media.Rollback.Episode.One.1080p.mkv");
    std::fs::write(&source_path, b"anime-media-rollback").expect("write source file");

    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    ctx.app.services.media_files = std::sync::Arc::new(FailingMediaFileRepo {
        inner: ctx.db.clone(),
        fail_file_id: file_id.clone(),
    });

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert!(
        preview
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 1);
    assert!(
        result
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let expected_path = season_dir
        .join("Anime Media Rollback - S01E01 (001) - 1080p.mkv")
        .to_string_lossy()
        .to_string();
    let item = &result.items[0];
    assert_eq!(item.status.as_str(), "failed");
    assert_eq!(item.reason_code, "db_update_failed");
    assert_eq!(
        item.final_path.as_deref(),
        Some(source_path.to_string_lossy().as_ref())
    );
    assert!(
        item.error_message
            .as_deref()
            .is_some_and(|message| message.contains("rollback succeeded"))
    );

    let stored = ctx
        .db
        .get_media_file_by_id(&file_id)
        .await
        .expect("load media file")
        .expect("media file present");
    assert_eq!(stored.file_path, source_path.to_string_lossy().to_string());
    assert!(source_path.exists());
    assert!(!std::path::Path::new(&expected_path).exists());
}

#[tokio::test]
async fn apply_media_rename_for_anime_interstitial_rolls_back_when_collection_update_fails() {
    let mut ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app.services.library_renamer = std::sync::Arc::new(FileSystemLibraryRenamer::new());
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Anime Interstitial Rollback",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "99001".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let season_zero_dir = media_root
        .path()
        .join("Anime Interstitial Rollback")
        .join("Season 00");
    std::fs::create_dir_all(&season_zero_dir).expect("create season zero dir");
    let source_path = season_zero_dir.join("Anime.Interstitial.Rollback.Movie.Special.1080p.mkv");
    std::fs::write(&source_path, b"anime-interstitial-rollback").expect("write source file");
    let expected_path = season_zero_dir
        .join("Anime Interstitial Rollback - S00E03 (3) - 1080p.mkv")
        .to_string_lossy()
        .to_string();

    let interstitial = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: Some(source_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "9900103".to_string(),
                name: "Rollback Movie".to_string(),
                slug: "rollback-movie".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Rollback special".to_string(),
                poster_url: "https://example.com/rollback-movie.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 88,
                sort_title: "Rollback Movie".to_string(),
                imdb_id: "tt9900103".to_string(),
                genres: vec!["Adventure".to_string()],
                studio: "Scryer Films".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                association_confidence: None,
                continuity_status: None,
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: Some("S00E03".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: source_path.to_string_lossy().to_string(),
            size_bytes: 2048,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert interstitial media file");

    ctx.app.services.shows = std::sync::Arc::new(FailingShowRepo {
        inner: ctx.db.clone(),
        fail_collection_id: interstitial.id.clone(),
        fail_path: expected_path.clone(),
    });

    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    let preview = ctx
        .app
        .preview_rename_for_title(&actor, &title.id, MediaFacet::Anime)
        .await
        .expect("preview rename plan");
    assert_eq!(preview.renamable, 1);
    assert!(
        preview
            .items
            .iter()
            .all(|item| item.write_action != scryer_application::RenameWriteAction::Replace)
    );

    let result = ctx
        .app
        .apply_rename_for_title(&actor, &title.id, MediaFacet::Anime, &preview.fingerprint)
        .await
        .expect("apply rename");
    assert_eq!(result.applied, 0);
    assert_eq!(result.failed, 1);

    let item = &result.items[0];
    assert_eq!(item.status.as_str(), "failed");
    assert_eq!(item.reason_code, "db_update_failed");
    assert_eq!(
        item.final_path.as_deref(),
        Some(source_path.to_string_lossy().as_ref())
    );
    assert!(
        item.error_message
            .as_deref()
            .is_some_and(|message| message.contains("rollback succeeded"))
    );

    let stored_file = ctx
        .db
        .get_media_file_by_id(&file_id)
        .await
        .expect("load interstitial media file")
        .expect("interstitial media file");
    let stored_collection = ctx
        .db
        .get_collection_by_id(&interstitial.id)
        .await
        .expect("load interstitial collection")
        .expect("interstitial collection");
    assert_eq!(
        stored_file.file_path,
        source_path.to_string_lossy().to_string()
    );
    assert_eq!(
        stored_collection.ordered_path,
        Some(source_path.to_string_lossy().to_string())
    );
    assert!(source_path.exists());
    assert!(!std::path::Path::new(&expected_path).exists());
}

// ---------------------------------------------------------------------------
// Basic connectivity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_get_returns_non_500() {
    let ctx = TestContext::new().await;
    let resp = ctx
        .http_client()
        .get(format!("{}/graphql", ctx.app_url))
        .send()
        .await
        .unwrap();
    // GET on a POST-only endpoint — should not crash
    assert_ne!(resp.status().as_u16(), 500);
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_introspection_query_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { queryType { name } } }", json!({})).await;
    assert_eq!(body["data"]["__schema"]["queryType"]["name"], "QueryRoot");
}

#[tokio::test]
async fn graphql_introspection_mutation_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { mutationType { name } } }", json!({})).await;
    assert_eq!(
        body["data"]["__schema"]["mutationType"]["name"],
        "MutationRoot"
    );
}

#[tokio::test]
async fn graphql_introspection_query_root_uses_semantic_search_and_browse_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ __type(name: "QueryRoot") { fields { name } } }"#,
        json!({}),
    )
    .await;
    let fields = body["data"]["__type"]["fields"]
        .as_array()
        .expect("should have fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();

    assert!(names.contains(&"searchReleases"));
    assert!(!names.contains(&"searchIndexers"));
    assert!(!names.contains(&"searchIndexersEpisode"));
    assert!(!names.contains(&"searchIndexersForTitle"));
    assert!(!names.contains(&"searchIndexersForEpisode"));
    assert!(!names.contains(&"titleCollections"));
    assert!(!names.contains(&"collectionEpisodes"));
    assert!(!names.contains(&"titleMediaFiles"));
    assert!(names.contains(&"wantedItem"));
    assert!(names.contains(&"pendingRelease"));
    assert!(names.contains(&"downloadHistory"));
}

#[tokio::test]
async fn graphql_introspection_exposes_typed_settings_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields { name }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields { name }
          }
          subtitleSettings: __type(name: "SubtitleSettingsPayload") {
            fields { name }
          }
          acquisitionSettings: __type(name: "AcquisitionSettingsPayload") {
            fields { name }
          }
          mediaSettings: __type(name: "MediaSettingsPayload") {
            fields { name }
          }
          libraryPaths: __type(name: "LibraryPathsPayload") {
            fields { name }
          }
          serviceSettings: __type(name: "ServiceSettingsPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let query_fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let query_names: Vec<&str> = query_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(query_names.contains(&"subtitleSettings"));
    assert!(query_names.contains(&"acquisitionSettings"));
    assert!(query_names.contains(&"mediaSettings"));
    assert!(query_names.contains(&"libraryPaths"));
    assert!(query_names.contains(&"serviceSettings"));
    assert!(query_names.contains(&"qualityProfileSettings"));
    assert!(query_names.contains(&"downloadClientRouting"));
    assert!(query_names.contains(&"indexerRouting"));
    assert!(!query_names.contains(&"adminSettings"));

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation_names: Vec<&str> = mutation_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(mutation_names.contains(&"updateSubtitleSettings"));
    assert!(mutation_names.contains(&"updateAcquisitionSettings"));
    assert!(mutation_names.contains(&"updateMediaSettings"));
    assert!(mutation_names.contains(&"updateLibraryPaths"));
    assert!(mutation_names.contains(&"updateServiceSettings"));
    assert!(mutation_names.contains(&"saveQualityProfileSettings"));
    assert!(mutation_names.contains(&"updateDownloadClientRouting"));
    assert!(mutation_names.contains(&"updateIndexerRouting"));
    assert!(!mutation_names.contains(&"saveAdminSettings"));

    let subtitle_fields = body["data"]["subtitleSettings"]["fields"]
        .as_array()
        .expect("SubtitleSettingsPayload should expose fields");
    let subtitle_names: Vec<&str> = subtitle_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(subtitle_names.contains(&"openSubtitlesUsername"));
    assert!(subtitle_names.contains(&"hasOpenSubtitlesPassword"));
    assert!(subtitle_names.contains(&"languages"));

    let acquisition_fields = body["data"]["acquisitionSettings"]["fields"]
        .as_array()
        .expect("AcquisitionSettingsPayload should expose fields");
    let acquisition_names: Vec<&str> = acquisition_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(acquisition_names.contains(&"pollIntervalSeconds"));
    assert!(acquisition_names.contains(&"batchSize"));

    let media_fields = body["data"]["mediaSettings"]["fields"]
        .as_array()
        .expect("MediaSettingsPayload should expose fields");
    let media_names: Vec<&str> = media_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(media_names.contains(&"libraryPath"));
    assert!(media_names.contains(&"rootFolders"));
    assert!(media_names.contains(&"renameTemplate"));

    let library_fields = body["data"]["libraryPaths"]["fields"]
        .as_array()
        .expect("LibraryPathsPayload should expose fields");
    let library_names: Vec<&str> = library_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(library_names.contains(&"moviePath"));
    assert!(library_names.contains(&"seriesPath"));
    assert!(library_names.contains(&"animePath"));

    let service_fields = body["data"]["serviceSettings"]["fields"]
        .as_array()
        .expect("ServiceSettingsPayload should expose fields");
    let service_names: Vec<&str> = service_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(service_names.contains(&"tlsCertPath"));
    assert!(service_names.contains(&"tlsKeyPath"));
}

#[tokio::test]
async fn graphql_typed_media_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            libraryPath
            rootFolders { path isDefault }
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
            fillerPolicy
            recapPolicy
            monitorSpecials
            interSeasonMovies
            monitorFillerMovies
            nfoWriteOnImport
            plexmatchWriteOnImport
          }
        }
        "#,
        json!({
          "input": {
            "scope": "anime",
            "rootFolders": [
              { "path": "/library/anime-main", "isDefault": true },
              { "path": "/library/anime-archive", "isDefault": false }
            ],
            "renameTemplate": "{title} [{quality}].{ext}",
            "renameCollisionPolicy": "replace_if_better",
            "renameMissingMetadataPolicy": "skip",
            "fillerPolicy": "skip_filler",
            "recapPolicy": "skip_recap",
            "monitorSpecials": true,
            "interSeasonMovies": false,
            "monitorFillerMovies": true,
            "nfoWriteOnImport": true,
            "plexmatchWriteOnImport": true
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let updated = &update["data"]["updateMediaSettings"];
    assert_eq!(updated["scope"], "anime");
    assert_eq!(updated["libraryPath"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["path"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["isDefault"], true);
    assert_eq!(updated["renameTemplate"], "{title} [{quality}].{ext}");
    assert_eq!(updated["renameCollisionPolicy"], "replace_if_better");
    assert_eq!(updated["renameMissingMetadataPolicy"], "skip");
    assert_eq!(updated["fillerPolicy"], "skip_filler");
    assert_eq!(updated["recapPolicy"], "skip_recap");
    assert_eq!(updated["monitorSpecials"], true);
    assert_eq!(updated["interSeasonMovies"], false);
    assert_eq!(updated["monitorFillerMovies"], true);
    assert_eq!(updated["nfoWriteOnImport"], true);
    assert_eq!(updated["plexmatchWriteOnImport"], true);

    let read = gql(
        &ctx,
        r#"
        query MediaSettings($scope: ContentScopeValue!) {
          mediaSettings(scope: $scope) {
            scope
            libraryPath
            rootFolders { path isDefault }
            renameTemplate
            renameCollisionPolicy
            renameMissingMetadataPolicy
            fillerPolicy
            recapPolicy
            monitorSpecials
            interSeasonMovies
            monitorFillerMovies
            nfoWriteOnImport
            plexmatchWriteOnImport
          }
        }
        "#,
        json!({ "scope": "anime" }),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["mediaSettings"];
    assert_eq!(settings["scope"], "anime");
    assert_eq!(settings["libraryPath"], "/library/anime-main");
    assert_eq!(settings["rootFolders"][1]["path"], "/library/anime-archive");
    assert_eq!(settings["renameTemplate"], "{title} [{quality}].{ext}");
    assert_eq!(settings["renameCollisionPolicy"], "replace_if_better");
    assert_eq!(settings["renameMissingMetadataPolicy"], "skip");
    assert_eq!(settings["fillerPolicy"], "skip_filler");
    assert_eq!(settings["recapPolicy"], "skip_recap");
    assert_eq!(settings["monitorSpecials"], true);
    assert_eq!(settings["interSeasonMovies"], false);
    assert_eq!(settings["monitorFillerMovies"], true);
    assert_eq!(settings["nfoWriteOnImport"], true);
    assert_eq!(settings["plexmatchWriteOnImport"], true);
}

#[tokio::test]
async fn graphql_typed_library_paths_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/mnt/storage/movies",
            "seriesPath": "/mnt/storage/series",
            "animePath": "/mnt/storage/anime"
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateLibraryPaths"]["moviePath"],
        "/mnt/storage/movies"
    );

    let read = gql(
        &ctx,
        r#"
        query LibraryPaths {
          libraryPaths {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["libraryPaths"]["moviePath"],
        "/mnt/storage/movies"
    );
    assert_eq!(
        read["data"]["libraryPaths"]["seriesPath"],
        "/mnt/storage/series"
    );
    assert_eq!(
        read["data"]["libraryPaths"]["animePath"],
        "/mnt/storage/anime"
    );
}

#[tokio::test]
async fn graphql_typed_service_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateServiceSettings($input: UpdateServiceSettingsInput!) {
          updateServiceSettings(input: $input) {
            tlsCertPath
            tlsKeyPath
          }
        }
        "#,
        json!({
          "input": {
            "tlsCertPath": "/etc/scryer/tls.crt",
            "tlsKeyPath": "/etc/scryer/tls.key"
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateServiceSettings"]["tlsCertPath"],
        "/etc/scryer/tls.crt"
    );

    let read = gql(
        &ctx,
        r#"
        query ServiceSettings {
          serviceSettings {
            tlsCertPath
            tlsKeyPath
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["serviceSettings"]["tlsCertPath"],
        "/etc/scryer/tls.crt"
    );
    assert_eq!(
        read["data"]["serviceSettings"]["tlsKeyPath"],
        "/etc/scryer/tls.key"
    );
}

#[tokio::test]
async fn graphql_typed_subtitle_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
          updateSubtitleSettings(input: $input) {
            enabled
            openSubtitlesUsername
            hasOpenSubtitlesPassword
            languages { code hearingImpaired forced }
            autoDownloadOnImport
            minimumScoreSeries
            minimumScoreMovie
            searchIntervalHours
            includeAiTranslated
            includeMachineTranslated
            syncEnabled
            syncThresholdSeries
            syncThresholdMovie
            syncMaxOffsetSeconds
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "openSubtitlesUsername": "subtitle-user",
            "openSubtitlesPassword": "secret-pass",
            "languages": [
              { "code": "eng", "hearingImpaired": true, "forced": false },
              { "code": "spa", "hearingImpaired": false, "forced": true }
            ],
            "autoDownloadOnImport": true,
            "minimumScoreSeries": 255,
            "minimumScoreMovie": 85,
            "searchIntervalHours": 12,
            "includeAiTranslated": true,
            "includeMachineTranslated": false,
            "syncEnabled": true,
            "syncThresholdSeries": 91,
            "syncThresholdMovie": 74,
            "syncMaxOffsetSeconds": 48
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateSubtitleSettings"]["openSubtitlesUsername"],
        "subtitle-user"
    );
    assert_eq!(
        update["data"]["updateSubtitleSettings"]["hasOpenSubtitlesPassword"],
        true
    );

    let read = gql(
        &ctx,
        r#"
        query SubtitleSettings {
          subtitleSettings {
            enabled
            openSubtitlesUsername
            hasOpenSubtitlesPassword
            languages { code hearingImpaired forced }
            autoDownloadOnImport
            minimumScoreSeries
            minimumScoreMovie
            searchIntervalHours
            includeAiTranslated
            includeMachineTranslated
            syncEnabled
            syncThresholdSeries
            syncThresholdMovie
            syncMaxOffsetSeconds
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["subtitleSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["openSubtitlesUsername"], "subtitle-user");
    assert_eq!(settings["hasOpenSubtitlesPassword"], true);
    assert_eq!(settings["autoDownloadOnImport"], true);
    assert_eq!(settings["minimumScoreSeries"], 255);
    assert_eq!(settings["minimumScoreMovie"], 85);
    assert_eq!(settings["searchIntervalHours"], 12);
    assert_eq!(settings["includeAiTranslated"], true);
    assert_eq!(settings["includeMachineTranslated"], false);
    assert_eq!(settings["syncEnabled"], true);
    assert_eq!(settings["syncThresholdSeries"], 91);
    assert_eq!(settings["syncThresholdMovie"], 74);
    assert_eq!(settings["syncMaxOffsetSeconds"], 48);
    assert_eq!(settings["languages"][0]["code"], "eng");
    assert_eq!(settings["languages"][0]["hearingImpaired"], true);
    assert_eq!(settings["languages"][1]["code"], "spa");
    assert_eq!(settings["languages"][1]["forced"], true);
}

#[tokio::test]
async fn graphql_typed_acquisition_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
          updateAcquisitionSettings(input: $input) {
            enabled
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
            pollIntervalSeconds
            syncIntervalSeconds
            batchSize
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "upgradeCooldownHours": 18,
            "sameTierMinDelta": 140,
            "crossTierMinDelta": 35,
            "forcedUpgradeDeltaBypass": 420,
            "pollIntervalSeconds": 45,
            "syncIntervalSeconds": 1800,
            "batchSize": 25
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query AcquisitionSettings {
          acquisitionSettings {
            enabled
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
            pollIntervalSeconds
            syncIntervalSeconds
            batchSize
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["acquisitionSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["upgradeCooldownHours"], 18);
    assert_eq!(settings["sameTierMinDelta"], 140);
    assert_eq!(settings["crossTierMinDelta"], 35);
    assert_eq!(settings["forcedUpgradeDeltaBypass"], 420);
    assert_eq!(settings["pollIntervalSeconds"], 45);
    assert_eq!(settings["syncIntervalSeconds"], 1800);
    assert_eq!(settings["batchSize"], 25);
}

#[tokio::test]
async fn graphql_delay_profiles_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let upsert = gql(
        &ctx,
        r#"
        mutation UpsertDelayProfile($input: DelayProfileInput!) {
          upsertDelayProfile(input: $input) {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({
          "input": {
            "id": "balanced-delay",
            "name": "Balanced Delay",
            "usenetDelayMinutes": 30,
            "torrentDelayMinutes": 90,
            "preferredProtocol": "usenet",
            "minAgeMinutes": 15,
            "bypassScoreThreshold": 320,
            "appliesToFacets": ["movie", "tv"],
            "tags": ["4k", "hdr"],
            "priority": 5,
            "enabled": true
          }
        }),
    )
    .await;
    assert_no_errors(&upsert);
    assert_eq!(upsert["data"]["upsertDelayProfile"]["id"], "balanced-delay");
    assert_eq!(
        upsert["data"]["upsertDelayProfile"]["appliesToFacets"][1],
        "tv"
    );

    let read = gql(
        &ctx,
        r#"
        query DelayProfiles {
          delayProfiles {
            id
            name
            usenetDelayMinutes
            torrentDelayMinutes
            preferredProtocol
            minAgeMinutes
            bypassScoreThreshold
            appliesToFacets
            tags
            priority
            enabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    let profile = &read["data"]["delayProfiles"][0];
    assert_eq!(profile["id"], "balanced-delay");
    assert_eq!(profile["name"], "Balanced Delay");
    assert_eq!(profile["usenetDelayMinutes"], 30);
    assert_eq!(profile["torrentDelayMinutes"], 90);
    assert_eq!(profile["preferredProtocol"], "usenet");
    assert_eq!(profile["minAgeMinutes"], 15);
    assert_eq!(profile["bypassScoreThreshold"], 320);
    assert_eq!(profile["appliesToFacets"][0], "movie");
    assert_eq!(profile["appliesToFacets"][1], "tv");
    assert_eq!(profile["tags"][0], "4k");
    assert_eq!(profile["priority"], 5);
    assert_eq!(profile["enabled"], true);

    let delete = gql(
        &ctx,
        r#"
        mutation DeleteDelayProfile($input: DeleteDelayProfileInput!) {
          deleteDelayProfile(input: $input) {
            id
          }
        }
        "#,
        json!({
          "input": { "id": "balanced-delay" }
        }),
    )
    .await;
    assert_no_errors(&delete);
    assert_eq!(delete["data"]["deleteDelayProfile"]["id"], "balanced-delay");
}

#[tokio::test]
async fn graphql_quality_profile_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            globalProfileId
            profiles {
              id
              name
              criteria {
                qualityTiers
                requiredAudioLanguages
                scoringPersona
              }
            }
            categorySelections {
              scope
              overrideProfileId
              effectiveProfileId
              inheritsGlobal
            }
          }
        }
        "#,
        json!({
          "input": {
            "profiles": [
              {
                "id": "custom-audio",
                "name": "Custom Audio",
                "criteria": {
                  "qualityTiers": ["2160P", "1080P"],
                  "archivalQuality": "2160P",
                  "allowUnknownQuality": false,
                  "sourceAllowlist": [],
                  "sourceBlocklist": [],
                  "videoCodecAllowlist": [],
                  "videoCodecBlocklist": [],
                  "audioCodecAllowlist": [],
                  "audioCodecBlocklist": [],
                  "atmosPreferred": true,
                  "dolbyVisionAllowed": true,
                  "detectedHdrAllowed": true,
                  "preferRemux": false,
                  "allowBdDisk": true,
                  "allowUpgrades": true,
                  "preferDualAudio": true,
                  "requiredAudioLanguages": ["jpn", "eng"],
                  "scoringPersona": "Audiophile",
                  "scoringOverrides": {},
                  "cutoffTier": null,
                  "minScoreToGrab": null,
                  "facetPersonaOverrides": [
                    { "scope": "anime", "persona": "Compatible" }
                  ]
                }
              }
            ],
            "globalProfileId": "custom-audio",
            "categorySelections": [
              {
                "scope": "movie",
                "profileId": "custom-audio",
                "inheritGlobal": false
              },
              {
                "scope": "series",
                "profileId": null,
                "inheritGlobal": true
              }
            ],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["saveQualityProfileSettings"]["globalProfileId"],
        "custom-audio"
    );
    assert_eq!(
        update["data"]["saveQualityProfileSettings"]["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["id"] == "custom-audio")
            .unwrap()["criteria"]["requiredAudioLanguages"][0],
        "jpn"
    );

    let read = gql(
        &ctx,
        r#"
        query QualityProfileSettings {
          qualityProfileSettings {
            globalProfileId
            profiles {
              id
              criteria {
                requiredAudioLanguages
                scoringPersona
                facetPersonaOverrides {
                  scope
                  persona
                }
              }
            }
            categorySelections {
              scope
              overrideProfileId
              effectiveProfileId
              inheritsGlobal
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["qualityProfileSettings"];
    assert_eq!(settings["globalProfileId"], "custom-audio");
    let movie_selection = settings["categorySelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|selection| selection["scope"] == "movie")
        .unwrap();
    assert_eq!(movie_selection["overrideProfileId"], "custom-audio");
    assert_eq!(movie_selection["inheritsGlobal"], false);
}

#[tokio::test]
async fn graphql_typed_routing_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update_download = gql(
        &ctx,
        r#"
        mutation UpdateDownloadClientRouting($input: UpdateDownloadClientRoutingInput!) {
          updateDownloadClientRouting(input: $input) {
            clientId
            enabled
            category
            recentQueuePriority
            olderQueuePriority
            removeCompleted
            removeFailed
          }
        }
        "#,
        json!({
          "input": {
            "scope": "movie",
            "entries": [
              {
                "clientId": "client-a",
                "enabled": true,
                "category": "movies",
                "recentQueuePriority": "high",
                "olderQueuePriority": "low",
                "removeCompleted": true,
                "removeFailed": false
              }
            ]
          }
        }),
    )
    .await;
    assert_no_errors(&update_download);
    assert_eq!(
        update_download["data"]["updateDownloadClientRouting"][0]["clientId"],
        "client-a"
    );

    let update_indexer = gql(
        &ctx,
        r#"
        mutation UpdateIndexerRouting($input: UpdateIndexerRoutingInput!) {
          updateIndexerRouting(input: $input) {
            indexerId
            enabled
            categories
            priority
          }
        }
        "#,
        json!({
          "input": {
            "scope": "anime",
            "entries": [
              {
                "indexerId": "indexer-a",
                "enabled": true,
                "categories": ["5070", "2000"],
                "priority": 3
              }
            ]
          }
        }),
    )
    .await;
    assert_no_errors(&update_indexer);
    assert_eq!(
        update_indexer["data"]["updateIndexerRouting"][0]["indexerId"],
        "indexer-a"
    );

    let read = gql(
        &ctx,
        r#"
        query TypedRouting {
          downloadClientRouting(scope: movie) {
            clientId
            category
            recentQueuePriority
          }
          indexerRouting(scope: anime) {
            indexerId
            categories
            priority
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["downloadClientRouting"][0]["clientId"],
        "client-a"
    );
    assert_eq!(
        read["data"]["downloadClientRouting"][0]["category"],
        "movies"
    );
    assert_eq!(read["data"]["indexerRouting"][0]["indexerId"], "indexer-a");
    assert_eq!(read["data"]["indexerRouting"][0]["priority"], 3);
}

#[tokio::test]
async fn graphql_introspection_lists_title_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ __type(name: "TitlePayload") { fields { name } } }"#,
        json!({}),
    )
    .await;
    let fields = body["data"]["__type"]["fields"]
        .as_array()
        .expect("should have fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();
    assert!(names.contains(&"id"), "TitlePayload should have id field");
    assert!(
        names.contains(&"name"),
        "TitlePayload should have name field"
    );
    assert!(
        names.contains(&"facet"),
        "TitlePayload should have facet field"
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_core_graph_relationship_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          title: __type(name: "TitlePayload") { fields { name } }
          collection: __type(name: "CollectionPayload") { fields { name } }
          episode: __type(name: "EpisodePayload") { fields { name } }
          queueItem: __type(name: "DownloadQueueItemPayload") { fields { name } }
          mediaFile: __type(name: "TitleMediaFilePayload") { fields { name } }
          wantedItem: __type(name: "WantedItemPayload") { fields { name } }
          releaseDecision: __type(name: "ReleaseDecisionPayload") { fields { name } }
          pendingRelease: __type(name: "PendingReleasePayload") { fields { name } }
          pendingReleaseStatus: __type(name: "PendingReleaseStatusValue") { enumValues { name } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let title_fields: Vec<&str> = body["data"]["title"]["fields"]
        .as_array()
        .expect("title fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(title_fields.contains(&"downloadQueueItems"));

    let collection_fields: Vec<&str> = body["data"]["collection"]["fields"]
        .as_array()
        .expect("collection fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(collection_fields.contains(&"title"));
    assert!(collection_fields.contains(&"episodes"));

    let episode_fields: Vec<&str> = body["data"]["episode"]["fields"]
        .as_array()
        .expect("episode fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(episode_fields.contains(&"parentTitle"));
    assert!(episode_fields.contains(&"collection"));
    assert!(episode_fields.contains(&"wantedItem"));
    assert!(episode_fields.contains(&"mediaFiles"));

    let queue_item_fields: Vec<&str> = body["data"]["queueItem"]["fields"]
        .as_array()
        .expect("queue item fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(queue_item_fields.contains(&"title"));

    let media_file_fields: Vec<&str> = body["data"]["mediaFile"]["fields"]
        .as_array()
        .expect("media file fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(media_file_fields.contains(&"title"));
    assert!(media_file_fields.contains(&"episode"));

    let wanted_item_fields: Vec<&str> = body["data"]["wantedItem"]["fields"]
        .as_array()
        .expect("wanted item fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(wanted_item_fields.contains(&"title"));
    assert!(wanted_item_fields.contains(&"collection"));
    assert!(wanted_item_fields.contains(&"episode"));
    assert!(wanted_item_fields.contains(&"releaseDecisions"));
    assert!(wanted_item_fields.contains(&"pendingReleases"));

    let release_decision_fields: Vec<&str> = body["data"]["releaseDecision"]["fields"]
        .as_array()
        .expect("release decision fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(release_decision_fields.contains(&"title"));
    assert!(release_decision_fields.contains(&"wantedItem"));

    let pending_release_fields: Vec<&str> = body["data"]["pendingRelease"]["fields"]
        .as_array()
        .expect("pending release fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(pending_release_fields.contains(&"title"));
    assert!(pending_release_fields.contains(&"wantedItem"));

    let pending_release_status_names: Vec<&str> =
        body["data"]["pendingReleaseStatus"]["enumValues"]
            .as_array()
            .expect("pending release status values")
            .iter()
            .filter_map(|value| value["name"].as_str())
            .collect();
    assert_eq!(
        pending_release_status_names,
        vec![
            "waiting",
            "standby",
            "processing",
            "grabbed",
            "superseded",
            "expired",
            "dismissed"
        ]
    );
}

#[tokio::test]
async fn graphql_traverses_core_graph_relationships() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = Title {
        id: Id::new().0,
        name: "Graph Traversal Show".to_string(),
        facet: MediaFacet::Series,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: Some("Traversal coverage".to_string()),
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: Some(24),
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };
    let title = ctx.db.create(title).await.expect("create title");

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .db
        .create_collection(collection)
        .await
        .expect("create collection");

    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Pilot".to_string()),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: Some("Episode overview".to_string()),
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let episode = ctx
        .db
        .create_episode(episode)
        .await
        .expect("create episode");

    let file_path = media_root
        .path()
        .join("Graph.Traversal.Show.S01E01.1080p.WEB-DL.mkv");
    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 4_096,
            quality_label: Some("1080p".to_string()),
            acquisition_score: Some(120),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let wanted_item = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        episode_id: Some(episode.id.clone()),
        collection_id: Some(collection.id.clone()),
        season_number: Some("1".to_string()),
        media_type: "episode".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 1,
        baseline_date: None,
        status: scryer_application::WantedStatus::Wanted,
        grabbed_release: None,
        current_score: Some(120),
        created_at: "2026-03-20T00:00:00Z".to_string(),
        updated_at: "2026-03-20T00:00:00Z".to_string(),
    };
    ctx.db
        .upsert_wanted_item(&wanted_item)
        .await
        .expect("seed wanted item");

    let decision = ReleaseDecision {
        id: Id::new().0,
        wanted_item_id: wanted_item.id.clone(),
        title_id: title.id.clone(),
        release_title: "Graph Traversal Show S01E01 1080p WEB-DL".to_string(),
        release_url: Some("https://example.invalid/release".to_string()),
        release_size_bytes: Some(8_192),
        decision_code: "accepted".to_string(),
        candidate_score: 140,
        current_score: Some(120),
        score_delta: Some(20),
        explanation_json: None,
        created_at: "2026-03-20T00:05:00Z".to_string(),
    };
    ctx.db
        .insert_release_decision(&decision)
        .await
        .expect("seed release decision");

    let pending_release = PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted_item.id.clone(),
        title_id: title.id.clone(),
        release_title: "Graph Traversal Show S01E01 1080p Delay Hold".to_string(),
        release_url: Some("https://example.invalid/pending".to_string()),
        source_kind: None,
        release_size_bytes: Some(16_384),
        release_score: 135,
        scoring_log_json: None,
        indexer_source: Some("test-indexer".to_string()),
        release_guid: Some("pending-guid".to_string()),
        added_at: "2026-03-20T00:06:00Z".to_string(),
        delay_until: "2026-03-20T01:06:00Z".to_string(),
        status: scryer_application::PendingReleaseStatus::Waiting,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
    };
    ctx.db
        .insert_pending_release(&pending_release)
        .await
        .expect("seed pending release");

    let body = gql(
        &ctx,
        r#"
        query CoreGraph($titleId: String!, $collectionId: String!, $episodeId: String!, $wantedItemId: String!, $pendingReleaseId: String!) {
          title(id: $titleId) {
            id
            downloadQueueItems {
              id
            }
            collections {
              id
              title { id }
              episodes {
                id
                parentTitle { id }
                collection { id }
                wantedItem { id }
                mediaFiles {
                  id
                  title { id }
                  episode { id }
                }
              }
            }
            mediaFiles {
              id
              title { id }
              episode {
                id
                parentTitle { id }
              }
            }
            wantedItems {
              id
              title { id }
              collection { id }
              episode { id }
              pendingReleases {
                id
                status
                title { id }
                wantedItem { id }
              }
              releaseDecisions(limit: 10) {
                id
                wantedItem { id }
                title { id }
              }
            }
            releaseDecisions(limit: 10) {
              id
              wantedItem { id }
              title { id }
            }
          }
          collection(id: $collectionId) {
            id
            title { id }
          }
          episode(id: $episodeId) {
            id
            parentTitle { id }
            collection { id }
            wantedItem { id }
            mediaFiles { id }
          }
          wantedItem(id: $wantedItemId) {
            id
            title { id }
            collection { id }
            episode { id }
            pendingReleases {
              id
              status
              title { id }
              wantedItem { id }
            }
            releaseDecisions(limit: 10) { id }
          }
          pendingRelease(id: $pendingReleaseId) {
            id
            status
            title { id }
            wantedItem { id }
          }
        }
        "#,
        json!({
            "titleId": title.id,
            "collectionId": collection.id,
            "episodeId": episode.id,
            "wantedItemId": wanted_item.id,
            "pendingReleaseId": pending_release.id,
        }),
    )
    .await;
    assert_no_errors(&body);

    let title_data = &body["data"]["title"];
    assert_eq!(title_data["downloadQueueItems"], json!([]));
    assert_eq!(title_data["collections"][0]["title"]["id"], title.id);
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["parentTitle"]["id"],
        title.id
    );
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["collection"]["id"],
        collection.id
    );
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["wantedItem"]["id"],
        wanted_item.id
    );
    assert_eq!(
        title_data["collections"][0]["episodes"][0]["mediaFiles"][0]["id"],
        file_id
    );
    assert_eq!(title_data["mediaFiles"][0]["title"]["id"], title.id);
    assert_eq!(title_data["mediaFiles"][0]["episode"]["id"], episode.id);
    assert_eq!(title_data["wantedItems"][0]["title"]["id"], title.id);
    assert_eq!(
        title_data["wantedItems"][0]["collection"]["id"],
        collection.id
    );
    assert_eq!(title_data["wantedItems"][0]["episode"]["id"], episode.id);
    assert_eq!(
        title_data["wantedItems"][0]["pendingReleases"][0]["id"],
        pending_release.id
    );
    assert_eq!(
        title_data["wantedItems"][0]["pendingReleases"][0]["status"],
        "waiting"
    );
    assert_eq!(
        title_data["wantedItems"][0]["releaseDecisions"][0]["id"],
        decision.id
    );
    assert_eq!(
        title_data["releaseDecisions"][0]["wantedItem"]["id"],
        wanted_item.id
    );

    assert_eq!(body["data"]["collection"]["title"]["id"], title.id);
    assert_eq!(body["data"]["episode"]["parentTitle"]["id"], title.id);
    assert_eq!(body["data"]["episode"]["collection"]["id"], collection.id);
    assert_eq!(body["data"]["episode"]["wantedItem"]["id"], wanted_item.id);
    assert_eq!(body["data"]["episode"]["mediaFiles"][0]["id"], file_id);
    assert_eq!(body["data"]["wantedItem"]["title"]["id"], title.id);
    assert_eq!(
        body["data"]["wantedItem"]["collection"]["id"],
        collection.id
    );
    assert_eq!(body["data"]["wantedItem"]["episode"]["id"], episode.id);
    assert_eq!(
        body["data"]["wantedItem"]["pendingReleases"][0]["id"],
        pending_release.id
    );
    assert_eq!(
        body["data"]["wantedItem"]["releaseDecisions"][0]["id"],
        decision.id
    );
    assert_eq!(body["data"]["pendingRelease"]["id"], pending_release.id);
    assert_eq!(body["data"]["pendingRelease"]["status"], "waiting");
    assert_eq!(body["data"]["pendingRelease"]["title"]["id"], title.id);
    assert_eq!(
        body["data"]["pendingRelease"]["wantedItem"]["id"],
        wanted_item.id
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_queue_and_source_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queueItem: __type(name: "DownloadQueueItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          queueState: __type(name: "DownloadQueueStateValue") {
            enumValues { name }
          }
          sourceKind: __type(name: "DownloadSourceKindValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["queueItem"]["fields"]
        .as_array()
        .expect("DownloadQueueItemPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("state")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("state")["type"]["ofType"]["name"],
        "DownloadQueueStateValue"
    );
    assert_eq!(field("importStatus")["type"]["name"], "ImportStatusValue");
    assert_eq!(
        field("trackedState")["type"]["name"],
        "TrackedDownloadStateValue"
    );
    assert_eq!(
        field("trackedStatus")["type"]["name"],
        "TrackedDownloadStatusValue"
    );
    assert_eq!(
        field("trackedMatchType")["type"]["name"],
        "TitleMatchTypeValue"
    );

    let queue_states = body["data"]["queueState"]["enumValues"]
        .as_array()
        .expect("DownloadQueueStateValue should expose enum values");
    let queue_state_names: Vec<&str> = queue_states
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(queue_state_names.contains(&"import_pending"));
    assert!(!queue_state_names.contains(&"importpending"));

    let source_kinds = body["data"]["sourceKind"]["enumValues"]
        .as_array()
        .expect("DownloadSourceKindValue should expose enum values");
    let source_kind_names: Vec<&str> = source_kinds
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        source_kind_names,
        vec!["nzbFile", "nzbUrl", "torrentFile", "magnetUri"]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_queue_action_payloads() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          actionPayload: __type(name: "DownloadQueueActionPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          actionKind: __type(name: "DownloadQueueActionKindValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation_field = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };

    for field_name in [
        "queueManualImport",
        "ignoreTrackedDownload",
        "markTrackedDownloadFailed",
        "retryTrackedDownloadImport",
        "assignTrackedDownloadTitle",
        "pauseDownload",
        "resumeDownload",
        "deleteDownload",
    ] {
        assert_eq!(mutation_field(field_name)["type"]["kind"], "NON_NULL");
        assert_eq!(
            mutation_field(field_name)["type"]["ofType"]["name"],
            "DownloadQueueActionPayload"
        );
    }

    let action_fields = body["data"]["actionPayload"]["fields"]
        .as_array()
        .expect("DownloadQueueActionPayload should expose fields");
    let action_field = |name: &str| {
        action_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("action payload field should exist")
    };

    assert_eq!(action_field("kind")["type"]["kind"], "NON_NULL");
    assert_eq!(
        action_field("kind")["type"]["ofType"]["name"],
        "DownloadQueueActionKindValue"
    );
    assert_eq!(
        action_field("downloadClientItemId")["type"]["kind"],
        "NON_NULL"
    );
    assert_eq!(action_field("removed")["type"]["kind"], "NON_NULL");
    assert_eq!(
        action_field("queueItem")["type"]["name"],
        "DownloadQueueItemPayload"
    );
    assert_eq!(action_field("importId")["type"]["name"], "String");

    let action_kind_names: Vec<&str> = body["data"]["actionKind"]["enumValues"]
        .as_array()
        .expect("DownloadQueueActionKindValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(action_kind_names.contains(&"queued_manual_import"));
    assert!(action_kind_names.contains(&"assigned_tracked_download_title"));
    assert!(action_kind_names.contains(&"deleted"));
}

#[tokio::test]
async fn graphql_introspection_exposes_wanted_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          wantedItem: __type(name: "WantedItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          wantedStatus: __type(name: "WantedStatusValue") {
            enumValues { name }
          }
          wantedMediaType: __type(name: "WantedMediaTypeValue") {
            enumValues { name }
          }
          wantedSearchPhase: __type(name: "WantedSearchPhaseValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["wantedItem"]["fields"]
        .as_array()
        .expect("WantedItemPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("mediaType")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("mediaType")["type"]["ofType"]["name"],
        "WantedMediaTypeValue"
    );
    assert_eq!(field("searchPhase")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("searchPhase")["type"]["ofType"]["name"],
        "WantedSearchPhaseValue"
    );
    assert_eq!(field("status")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("status")["type"]["ofType"]["name"],
        "WantedStatusValue"
    );

    let status_names: Vec<&str> = body["data"]["wantedStatus"]["enumValues"]
        .as_array()
        .expect("WantedStatusValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        status_names,
        vec!["wanted", "grabbed", "paused", "completed"]
    );

    let media_type_names: Vec<&str> = body["data"]["wantedMediaType"]["enumValues"]
        .as_array()
        .expect("WantedMediaTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        media_type_names,
        vec!["movie", "episode", "interstitial_movie"]
    );

    let search_phase_names: Vec<&str> = body["data"]["wantedSearchPhase"]["enumValues"]
        .as_array()
        .expect("WantedSearchPhaseValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        search_phase_names,
        vec![
            "pre_air",
            "pre_release",
            "primary",
            "secondary",
            "long_tail"
        ]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_import_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          importRecord: __type(name: "ImportRecordPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          importResult: __type(name: "ImportResultPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          importStatus: __type(name: "ImportStatusValue") {
            enumValues { name }
          }
          importType: __type(name: "ImportTypeValue") {
            enumValues { name }
          }
          importDecision: __type(name: "ImportDecisionValue") {
            enumValues { name }
          }
          importSkipReason: __type(name: "ImportSkipReasonValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let record_fields = body["data"]["importRecord"]["fields"]
        .as_array()
        .expect("ImportRecordPayload should expose fields");
    let record_field = |name: &str| {
        record_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(record_field("importType")["type"]["kind"], "NON_NULL");
    assert_eq!(
        record_field("importType")["type"]["ofType"]["name"],
        "ImportTypeValue"
    );
    assert_eq!(record_field("status")["type"]["kind"], "NON_NULL");
    assert_eq!(
        record_field("status")["type"]["ofType"]["name"],
        "ImportStatusValue"
    );
    assert_eq!(
        record_field("decision")["type"]["name"],
        "ImportDecisionValue"
    );
    assert_eq!(
        record_field("skipReason")["type"]["name"],
        "ImportSkipReasonValue"
    );

    let result_fields = body["data"]["importResult"]["fields"]
        .as_array()
        .expect("ImportResultPayload should expose fields");
    let result_field = |name: &str| {
        result_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(result_field("decision")["type"]["kind"], "NON_NULL");
    assert_eq!(
        result_field("decision")["type"]["ofType"]["name"],
        "ImportDecisionValue"
    );
    assert_eq!(
        result_field("skipReason")["type"]["name"],
        "ImportSkipReasonValue"
    );

    let import_status_names: Vec<&str> = body["data"]["importStatus"]["enumValues"]
        .as_array()
        .expect("ImportStatusValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        import_status_names,
        vec![
            "pending",
            "running",
            "processing",
            "completed",
            "failed",
            "skipped"
        ]
    );

    let import_type_names: Vec<&str> = body["data"]["importType"]["enumValues"]
        .as_array()
        .expect("ImportTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(import_type_names.contains(&"tv_download"));
    assert!(import_type_names.contains(&"rename_io_failed"));

    let import_decision_names: Vec<&str> = body["data"]["importDecision"]["enumValues"]
        .as_array()
        .expect("ImportDecisionValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        import_decision_names,
        vec![
            "imported",
            "rejected",
            "skipped",
            "conflict",
            "unmatched",
            "failed"
        ]
    );

    let import_skip_reason_names: Vec<&str> = body["data"]["importSkipReason"]["enumValues"]
        .as_array()
        .expect("ImportSkipReasonValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(import_skip_reason_names.contains(&"password_required"));
    assert!(import_skip_reason_names.contains(&"post_download_rule_blocked"));
}

#[tokio::test]
async fn graphql_introspection_exposes_activity_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          activityEvent: __type(name: "ActivityEventPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          activityKind: __type(name: "ActivityKindValue") {
            enumValues { name }
          }
          activitySeverity: __type(name: "ActivitySeverityValue") {
            enumValues { name }
          }
          activityChannel: __type(name: "ActivityChannelValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["activityEvent"]["fields"]
        .as_array()
        .expect("ActivityEventPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("kind")["type"]["kind"], "NON_NULL");
    assert_eq!(field("kind")["type"]["ofType"]["name"], "ActivityKindValue");
    assert_eq!(field("severity")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("severity")["type"]["ofType"]["name"],
        "ActivitySeverityValue"
    );
    assert_eq!(field("channels")["type"]["kind"], "NON_NULL");
    assert_eq!(field("channels")["type"]["ofType"]["kind"], "LIST");
    assert_eq!(
        field("channels")["type"]["ofType"]["ofType"]["kind"],
        "NON_NULL"
    );
    assert_eq!(
        field("channels")["type"]["ofType"]["ofType"]["ofType"]["name"],
        "ActivityChannelValue"
    );

    let activity_kind_names: Vec<&str> = body["data"]["activityKind"]["enumValues"]
        .as_array()
        .expect("ActivityKindValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(activity_kind_names.contains(&"metadata_hydration_completed"));
    assert!(activity_kind_names.contains(&"import_rejected"));

    let activity_severity_names: Vec<&str> = body["data"]["activitySeverity"]["enumValues"]
        .as_array()
        .expect("ActivitySeverityValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        activity_severity_names,
        vec!["info", "success", "warning", "error"]
    );

    let activity_channel_names: Vec<&str> = body["data"]["activityChannel"]["enumValues"]
        .as_array()
        .expect("ActivityChannelValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(activity_channel_names, vec!["web_ui", "toast"]);
}

// ---------------------------------------------------------------------------
// Title CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_list_titles_starts_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ titles { id } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn graphql_add_title_movie() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Movie", "movie").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_tv() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Series", "tv").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_anime() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Anime", "anime").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_with_structured_options() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title {
                    id
                    qualityProfileId
                    rootFolderPath
                    monitorType
                    useSeasonFolders
                    monitorSpecials
                    interSeasonMovies
                    fillerPolicy
                    recapPolicy
                }
            }
        }"#,
        json!({
            "input": {
                "name": "Configured Anime",
                "facet": "anime",
                "monitored": true,
                "tags": ["favorite"],
                "options": {
                    "qualityProfileId": "anime-hd",
                    "rootFolderPath": "/library/anime",
                    "monitorType": "futureEpisodes",
                    "useSeasonFolders": false,
                    "monitorSpecials": true,
                    "interSeasonMovies": false,
                    "fillerPolicy": "skip_filler",
                    "recapPolicy": "skip_recap"
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let title = &body["data"]["addTitle"]["title"];
    assert_eq!(title["qualityProfileId"], "anime-hd");
    assert_eq!(title["rootFolderPath"], "/library/anime");
    assert_eq!(title["monitorType"], "futureEpisodes");
    assert_eq!(title["useSeasonFolders"], false);
    assert_eq!(title["monitorSpecials"], true);
    assert_eq!(title["interSeasonMovies"], false);
    assert_eq!(title["fillerPolicy"], "skip_filler");
    assert_eq!(title["recapPolicy"], "skip_recap");
}

#[tokio::test]
async fn graphql_add_title_then_list() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Listed Movie", "movie").await;

    let body = gql(&ctx, "{ titles { id name facet } }", json!({})).await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["name"], "Listed Movie");
    assert_eq!(titles[0]["facet"], "movie");
}

#[tokio::test]
async fn graphql_add_multiple_titles() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie One", "movie").await;
    add_test_title(&ctx, "Series One", "tv").await;
    add_test_title(&ctx, "Anime One", "anime").await;

    let body = gql(&ctx, "{ titles { id facet } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn graphql_titles_are_sorted_by_display_name() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "zeta movie", "movie").await;
    add_test_title(&ctx, "Alpha Movie", "movie").await;
    add_test_title(&ctx, "beta movie", "movie").await;

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { name } }"#,
        json!({ "facet": "movie" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().unwrap();
    let names: Vec<&str> = titles
        .iter()
        .map(|title| title["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Alpha Movie", "beta movie", "zeta movie"]);
}

#[tokio::test]
async fn graphql_titles_expose_episode_progress_excluding_specials() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Episode Progress Show",
        MediaFacet::Series,
        vec![],
        vec![],
        false,
    )
    .await;

    let season_collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let specials_collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");

    let regular_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "1", "S01E01").await;
    let regular_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "2", "S01E02").await;
    let _regular_episode_3 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "3", "S01E03").await;
    let special_episode_1 =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "1", "S00E01").await;
    let _special_episode_2 =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "2", "S00E02").await;

    for (index, episode) in [regular_episode_1, regular_episode_2, special_episode_1]
        .into_iter()
        .enumerate()
    {
        let file_path = media_root
            .path()
            .join(format!("Episode.Progress.Show.file-{index}.mkv"));
        let file_id = ctx
            .db
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path.to_string_lossy().to_string(),
                size_bytes: 4_096 + index as i64,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.db
            .link_file_to_episode(&file_id, &episode.id)
            .await
            .expect("link file to episode");
    }

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { id name episodesOwned episodesTotal } }"#,
        json!({ "facet": "tv" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("series title should be listed");

    assert_eq!(listed_title["name"], "Episode Progress Show");
    assert_eq!(listed_title["episodesOwned"], 2);
    assert_eq!(listed_title["episodesTotal"], 3);
}

#[tokio::test]
async fn graphql_get_title_by_id() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Specific Movie", "movie").await;

    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id name monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["title"]["name"], "Specific Movie");
    assert_eq!(body["data"]["title"]["monitored"], true);
}

#[tokio::test]
async fn graphql_get_title_not_found() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id name } }"#,
        json!({ "id": "nonexistent-id" }),
    )
    .await;
    assert!(
        body["data"]["title"].is_null(),
        "should return null for nonexistent title"
    );
}

#[tokio::test]
async fn graphql_set_title_monitored() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Monitor Test", "movie").await;

    // Disable monitoring
    let body = gql(
        &ctx,
        r#"mutation($input: SetTitleMonitoredInput!) {
            setTitleMonitored(input: $input) { id monitored }
        }"#,
        json!({ "input": { "titleId": id, "monitored": false } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["setTitleMonitored"]["monitored"], false);

    // Verify via query
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_eq!(body["data"]["title"]["monitored"], false);
}

#[tokio::test]
async fn graphql_update_collection_monitored_matches_set_collection_monitored_side_effects() {
    let ctx = TestContext::new().await;
    let (set_title, set_collection, set_episode) =
        create_series_monitoring_fixture(&ctx, "Set Collection Flow", "61001").await;
    let (update_title, update_collection, update_episode) =
        create_series_monitoring_fixture(&ctx, "Update Collection Flow", "61002").await;

    let set_enable = gql(
        &ctx,
        r#"mutation($input: SetCollectionMonitoredInput!) {
            setCollectionMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": set_collection.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&set_enable);

    let update_enable = gql(
        &ctx,
        r#"mutation($input: UpdateCollectionInput!) {
            updateCollection(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": update_collection.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&update_enable);

    let set_enabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_enabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_enabled_summary, update_enabled_summary);
    assert_eq!(
        set_enabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: true,
            episode_monitored: true,
            wanted_count: 1,
        }
    );

    let set_disable = gql(
        &ctx,
        r#"mutation($input: SetCollectionMonitoredInput!) {
            setCollectionMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": set_collection.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&set_disable);

    let update_disable = gql(
        &ctx,
        r#"mutation($input: UpdateCollectionInput!) {
            updateCollection(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "collectionId": update_collection.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&update_disable);

    let set_disabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_disabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_disabled_summary, update_disabled_summary);
    assert_eq!(
        set_disabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: false,
            episode_monitored: false,
            wanted_count: 0,
        }
    );
}

#[tokio::test]
async fn graphql_update_episode_monitored_matches_set_episode_monitored_side_effects() {
    let ctx = TestContext::new().await;
    let (set_title, set_collection, set_episode) =
        create_series_monitoring_fixture(&ctx, "Set Episode Flow", "62001").await;
    let (update_title, update_collection, update_episode) =
        create_series_monitoring_fixture(&ctx, "Update Episode Flow", "62002").await;

    let set_enable = gql(
        &ctx,
        r#"mutation($input: SetEpisodeMonitoredInput!) {
            setEpisodeMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": set_episode.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&set_enable);

    let update_enable = gql(
        &ctx,
        r#"mutation($input: UpdateEpisodeInput!) {
            updateEpisode(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": update_episode.id,
                "monitored": true
            }
        }),
    )
    .await;
    assert_no_errors(&update_enable);

    let set_enabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_enabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_enabled_summary, update_enabled_summary);
    assert_eq!(
        set_enabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: true,
            episode_monitored: true,
            wanted_count: 1,
        }
    );

    let set_disable = gql(
        &ctx,
        r#"mutation($input: SetEpisodeMonitoredInput!) {
            setEpisodeMonitored(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": set_episode.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&set_disable);

    let update_disable = gql(
        &ctx,
        r#"mutation($input: UpdateEpisodeInput!) {
            updateEpisode(input: $input) { id monitored }
        }"#,
        json!({
            "input": {
                "episodeId": update_episode.id,
                "monitored": false
            }
        }),
    )
    .await;
    assert_no_errors(&update_disable);

    let set_disabled_summary =
        series_monitoring_summary(&ctx, &set_title.id, &set_collection.id, &set_episode.id).await;
    let update_disabled_summary = series_monitoring_summary(
        &ctx,
        &update_title.id,
        &update_collection.id,
        &update_episode.id,
    )
    .await;
    assert_eq!(set_disabled_summary, update_disabled_summary);
    assert_eq!(
        set_disabled_summary,
        SeriesMonitoringSummary {
            title_monitored: true,
            collection_monitored: true,
            episode_monitored: false,
            wanted_count: 0,
        }
    );
}

#[tokio::test]
async fn graphql_update_title_structured_options_merge_with_existing_tags() {
    let ctx = TestContext::new().await;
    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id }
            }
        }"#,
        json!({
            "input": {
                "name": "Option Update Anime",
                "facet": "anime",
                "monitored": true,
                "tags": ["favorite"]
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title_id = add_body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("title id")
        .to_string();

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                id
                tags
                qualityProfileId
                rootFolderPath
                useSeasonFolders
                fillerPolicy
                recapPolicy
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "qualityProfileId": "anime-4k",
                    "rootFolderPath": "/custom/anime",
                    "useSeasonFolders": false,
                    "fillerPolicy": "skip_filler",
                    "recapPolicy": ""
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let updated = &body["data"]["updateTitle"];
    assert_eq!(updated["qualityProfileId"], "anime-4k");
    assert_eq!(updated["rootFolderPath"], "/custom/anime");
    assert_eq!(updated["useSeasonFolders"], false);
    assert_eq!(updated["fillerPolicy"], "skip_filler");
    assert!(updated["recapPolicy"].is_null());

    let tags = updated["tags"].as_array().expect("tags array");
    let tag_values: Vec<&str> = tags.iter().filter_map(|tag| tag.as_str()).collect();
    assert!(tag_values.contains(&"favorite"));
    assert!(tag_values.contains(&"scryer:quality-profile:anime-4k"));
    assert!(tag_values.contains(&"scryer:root-folder:/custom/anime"));
    assert!(tag_values.contains(&"scryer:season-folder:disabled"));
    assert!(tag_values.contains(&"scryer:filler-policy:skip_filler"));
    assert!(
        !tag_values
            .iter()
            .any(|tag| tag.starts_with("scryer:recap-policy:"))
    );
}

#[tokio::test]
async fn graphql_trigger_title_wanted_search() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Search Monitored Test", "movie").await;

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            triggerTitleWantedSearch(input: $input)
        }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["triggerTitleWantedSearch"], 1);

    let body = gql(
        &ctx,
        r#"query($titleId: String) {
            wantedItems(titleId: $titleId) {
                total
                items { titleId mediaType status }
            }
        }"#,
        json!({ "titleId": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["wantedItems"]["total"], 1);
    assert_eq!(body["data"]["wantedItems"]["items"][0]["titleId"], id);
    assert_eq!(
        body["data"]["wantedItems"]["items"][0]["mediaType"],
        "movie"
    );
    assert_eq!(body["data"]["wantedItems"]["items"][0]["status"], "wanted");
}

#[tokio::test]
async fn graphql_scan_title_library() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Scan Show", vec![]).await;
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Scan.Show.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                    scanStatus
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(files[0]["scanStatus"], "scan_failed");

    let persisted_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = media_root.path().join(&title.name);
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.to_string_lossy().as_ref())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .all(|tag| tag != "scryer:season-folder:disabled")
    );
}

#[tokio::test]
async fn graphql_scan_title_library_matches_daily_episodes_by_air_date() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Daily Show", vec![]).await;
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Daily Episode".to_string()),
        air_date: Some("2024-03-15".to_string()),
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let episode = ctx
        .db
        .create_episode(episode)
        .await
        .expect("create episode");

    let season_dir = media_root.path().join(&title.name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Daily.Show.2024.03.15.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let body = gql(
        &ctx,
        r#"mutation($input: TitleIdInput!) {
            scanTitleLibrary(input: $input) {
                scanned
                matched
                imported
                skipped
                unmatched
            }
        }"#,
        json!({ "input": { "titleId": title.id.clone() } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["scanTitleLibrary"]["scanned"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["matched"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["imported"], 1);
    assert_eq!(body["data"]["scanTitleLibrary"]["skipped"], 0);
    assert_eq!(body["data"]["scanTitleLibrary"]["unmatched"], 0);

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            title(id: $id) {
                mediaFiles {
                    episodeId
                    filePath
                }
            }
        }"#,
        json!({ "id": title.id.clone() }),
    )
    .await;
    assert_no_errors(&body);
    let files = body["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["episodeId"], episode.id);
    assert_eq!(
        files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
}

#[tokio::test]
async fn graphql_scan_title_library_disables_season_folders_for_flat_layout() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) =
        create_series_scan_title(&ctx, media_root.path(), "Flat Show", vec![]).await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let title_dir = media_root.path().join(&title.name);
    std::fs::create_dir_all(&title_dir).expect("create title dir");
    std::fs::write(
        title_dir.join("Flat.Show.S01E01.1080p.WEB-DL.mkv"),
        b"not-a-real-video",
    )
    .expect("write fake video");

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_title_library(&admin, &title.id)
        .await
        .expect("scan title library");

    let persisted_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = title_dir.to_string_lossy().to_string();
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.as_str())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .any(|tag| tag == "scryer:season-folder:disabled")
    );
}

#[tokio::test]
async fn graphql_scan_title_library_preserves_existing_layout_when_ambiguous() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let (title, collection) = create_series_scan_title(
        &ctx,
        media_root.path(),
        "Mixed Show",
        vec!["scryer:season-folder:disabled".to_string()],
    )
    .await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;
    create_series_scan_episode(&ctx, &title, &collection, "1", "2", "S01E02").await;

    let title_dir = media_root.path().join(&title.name);
    let season_dir = title_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(title_dir.join("Mixed.Show.S01E01.1080p.WEB-DL.mkv"), b"one")
        .expect("write flat file");
    std::fs::write(
        season_dir.join("Mixed.Show.S01E02.1080p.WEB-DL.mkv"),
        b"two",
    )
    .expect("write season file");

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_title_library(&admin, &title.id)
        .await
        .expect("scan title library");

    let persisted_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let expected_folder_path = title_dir.to_string_lossy().to_string();
    assert_eq!(
        persisted_title.folder_path.as_deref(),
        Some(expected_folder_path.as_str())
    );
    assert!(
        persisted_title
            .tags
            .iter()
            .any(|tag| tag == "scryer:season-folder:disabled")
    );
    assert_eq!(
        persisted_title
            .tags
            .iter()
            .filter(|tag| tag.starts_with("scryer:season-folder:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn library_series_scan_hydrates_without_creating_wanted_for_unmonitored_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "s0": {
                "series": {
                    "tvdb_id": 345678,
                    "name": "Test Show Name",
                    "sort_name": "Test Show Name",
                    "slug": "test-show-name",
                    "status": "Continuing",
                    "year": 2023,
                    "first_aired": "2023-09-15",
                    "overview": "A compelling drama about software testing.",
                    "network": "Test Network",
                    "runtime_minutes": 45,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/345678/posters/test.jpg",
                    "country": "usa",
                    "genres": ["Drama", "Thriller"],
                    "aliases": ["Testing Show", "QA Chronicles"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 1000001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2000001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Pilot",
                            "aired": "2023-09-15",
                            "runtime_minutes": 60,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "The team assembles.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let token = tokio_util::sync::CancellationToken::new();
    let hydration_token = token.clone();
    let hydration_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_hydration_loop(hydration_app, hydration_token).await;
    });
    let post_hydration_scan_token = token.clone();
    let post_hydration_scan_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_post_hydration_title_scan_workers(
            post_hydration_scan_app,
            post_hydration_scan_token,
        )
        .await;
    });

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Test Show Name");
    std::fs::create_dir_all(&show_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Test Show Name</title><tvdbid>345678</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    let mut hydrated_title = None;
    for _ in 0..20 {
        let titles = ctx
            .db
            .list(Some(MediaFacet::Series), None)
            .await
            .expect("list titles");
        assert_eq!(titles.len(), 1);
        if titles[0].metadata_fetched_at.is_some() {
            hydrated_title = Some(titles[0].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    token.cancel();

    let hydrated_title = hydrated_title.expect("title should hydrate");
    assert!(!hydrated_title.monitored);

    let (wanted_items, total) = ctx
        .app
        .list_wanted_items(None, None, Some(&hydrated_title.id), 10, 0)
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
    assert_eq!(total, 0);
}

#[tokio::test]
async fn library_anime_scan_hydrates_and_relinks_files_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = json!({
        "data": {
            "s0": {
                "series": {
                    "tvdb_id": 456789,
                    "name": "Hydrated Anime Title",
                    "sort_name": "Hydrated Anime Title",
                    "slug": "hydrated-anime-title",
                    "status": "Ended",
                    "year": 2021,
                    "first_aired": "2021-01-10",
                    "overview": "An anime hydration fixture.",
                    "network": "Tokyo MX",
                    "runtime_minutes": 24,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/456789/posters/test.jpg",
                    "country": "jpn",
                    "genres": ["Animation"],
                    "aliases": ["Hydrated Anime Alias"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": [
                        {
                            "tvdb_id": 1001001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2001001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Episode 1",
                            "aired": "2021-01-10",
                            "runtime_minutes": 24,
                            "is_filler": false,
                            "is_recap": false,
                            "overview": "Episode 1 overview.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let token = tokio_util::sync::CancellationToken::new();
    let hydration_token = token.clone();
    let hydration_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_hydration_loop(hydration_app, hydration_token).await;
    });
    let post_hydration_scan_token = token.clone();
    let post_hydration_scan_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_post_hydration_title_scan_workers(
            post_hydration_scan_app,
            post_hydration_scan_token,
        )
        .await;
    });

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Anime Scan [SubsPlease]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Anime Scan</title><tvdbid>456789</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Anime.Scan.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let token = tokio_util::sync::CancellationToken::new();
    let post_hydration_scan_token = token.clone();
    let post_hydration_scan_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_post_hydration_title_scan_workers(
            post_hydration_scan_app,
            post_hydration_scan_token,
        )
        .await;
    });

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);

    let mut hydrated_title = None;
    let mut linked_files = Vec::new();
    for _ in 0..40 {
        let titles = ctx
            .db
            .list(Some(MediaFacet::Anime), None)
            .await
            .expect("list anime titles");
        assert_eq!(titles.len(), 1);
        let files = ctx
            .db
            .list_media_files_for_title(&titles[0].id)
            .await
            .expect("list media files");
        if titles[0].metadata_fetched_at.is_some() && !files.is_empty() {
            hydrated_title = Some(titles[0].clone());
            linked_files = files;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    token.cancel();

    let hydrated_title = hydrated_title.expect("anime title should hydrate and relink files");
    assert_eq!(hydrated_title.name, "Anime Scan");
    assert!(hydrated_title.metadata_fetched_at.is_some());
    assert_eq!(
        hydrated_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert!(
        linked_files[0].episode_id.is_some(),
        "linked file should target a hydrated episode"
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_anime_scan_relinks_existing_hydrated_titles_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = create_catalog_title(
        &ctx,
        "Existing Anime",
        MediaFacet::Anime,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "567890".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: false,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .db
        .create_collection(collection)
        .await
        .expect("create collection");
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Existing Anime [BD]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Existing Anime</title><tvdbid>567890</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Existing.Anime.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": "/tmp/series-unused",
            "animePath": media_root.path().display().to_string()
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("scan anime library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 1);

    let mut linked_files = Vec::new();
    for _ in 0..10 {
        linked_files = ctx
            .db
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        if !linked_files.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let refreshed_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Anime");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(
        linked_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_series_scan_relinks_existing_hydrated_titles_from_discovered_folder_path() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = create_catalog_title(
        &ctx,
        "Existing Series",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "345678".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: false,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .db
        .create_collection(collection)
        .await
        .expect("create collection");
    let episode = create_series_scan_episode(&ctx, &title, &collection, "1", "1", "S01E01").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Existing Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Existing Series</title><tvdbid>345678</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Existing.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let token = tokio_util::sync::CancellationToken::new();
    let post_hydration_scan_token = token.clone();
    let post_hydration_scan_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_post_hydration_title_scan_workers(
            post_hydration_scan_app,
            post_hydration_scan_token,
        )
        .await;
    });

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan series library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 1);

    let mut linked_files = Vec::new();
    for _ in 0..100 {
        linked_files = ctx
            .db
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files");
        if !linked_files.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let refreshed_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Series");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );

    assert_eq!(linked_files.len(), 1);
    assert_eq!(
        linked_files[0].file_path,
        file_path.to_string_lossy().to_string()
    );
    assert_eq!(
        linked_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
    assert_eq!(linked_files[0].scan_status, "scan_failed");
}

#[tokio::test]
async fn library_series_scan_existing_unhydrated_title_without_episodes_completes_session() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = ctx
        .db
        .create(Title {
            id: Id::new().0,
            name: "Pending Series".to_string(),
            facet: MediaFacet::Series,
            monitored: false,
            tags: vec![],
            external_ids: vec![ExternalId {
                source: "tvdb".to_string(),
                value: "345679".to_string(),
            }],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2024),
            overview: Some("Pending hydration title".to_string()),
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: Some("Pending Series".to_string()),
            slug: Some("pending-series".to_string()),
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        })
        .await
        .expect("create pending title");

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Pending Series [WEB-DL]");
    let season_dir = show_dir.join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Pending Series</title><tvdbid>345679</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");
    let file_path = season_dir.join("Pending.Series.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan series library");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 1);

    let refreshed_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        ctx.app
            .services
            .library_scan_tracker
            .list_active()
            .await
            .is_empty(),
        "scan session should complete when an existing unhydrated title is skipped",
    );
    assert!(
        ctx.db
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );
}

#[tokio::test]
async fn library_series_scan_creates_unmonitored_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let show_dir = media_root.path().join("Bluey");
    std::fs::create_dir_all(&show_dir).expect("create show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        r#"<tvshow><title>Bluey</title><tvdbid>81189</tvdbid></tvshow>"#,
    )
    .expect("write tvshow.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);

    let titles = ctx
        .db
        .list(Some(MediaFacet::Series), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].name, "Bluey");
    assert!(!titles[0].monitored);
}

#[tokio::test]
async fn library_movie_scan_refreshes_existing_title_from_disk_without_renaming() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let title = create_catalog_title(
        &ctx,
        "Existing Movie",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        vec![],
        false,
    )
    .await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let movie_dir = media_root.path().join("Existing Movie [2160p]");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Existing.Movie.2024.2160p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Existing Movie</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let refreshed_title = ctx
        .db
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    assert_eq!(refreshed_title.name, "Existing Movie");

    let collections = ctx
        .db
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(movie_path.to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn library_movie_scan_creates_unmonitored_title_and_collection() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let fixture = load_fixture("smg/get_movie.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let token = tokio_util::sync::CancellationToken::new();
    let hydration_token = token.clone();
    let hydration_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_hydration_loop(hydration_app, hydration_token).await;
    });
    let post_hydration_scan_token = token.clone();
    let post_hydration_scan_app = ctx.app.clone();
    tokio::spawn(async move {
        scryer_application::start_background_post_hydration_title_scan_workers(
            post_hydration_scan_app,
            post_hydration_scan_token,
        )
        .await;
    });

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let movie_dir = media_root.path().join("Test Movie Title (2024)");
    std::fs::create_dir_all(&movie_dir).expect("create movie dir");
    let movie_path = movie_dir.join("Test.Movie.Title.2024.1080p.WEB-DL.mkv");
    let movie_file = std::fs::File::create(&movie_path).expect("create movie file");
    movie_file
        .set_len(60 * 1024 * 1024)
        .expect("set movie file size");
    std::fs::write(
        movie_dir.join("movie.nfo"),
        r#"<movie><title>Test Movie Title</title><tvdbid>123456</tvdbid><year>2024</year></movie>"#,
    )
    .expect("write movie.nfo");

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.matched, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let mut hydrated_title = None;
    for _ in 0..20 {
        let titles = ctx
            .db
            .list(Some(MediaFacet::Movie), None)
            .await
            .expect("list titles");
        assert_eq!(titles.len(), 1);
        if titles[0].metadata_fetched_at.is_some() {
            hydrated_title = Some(titles[0].clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    token.cancel();

    let hydrated_title = hydrated_title.expect("movie title should hydrate");
    assert_eq!(hydrated_title.name, "Test Movie Title");
    assert!(!hydrated_title.monitored);

    let collections = ctx
        .db
        .list_collections_for_title(&hydrated_title.id)
        .await
        .expect("list collections");
    assert_eq!(collections.len(), 1);
    assert!(!collections[0].monitored);
    assert_eq!(
        collections[0].ordered_path.as_deref(),
        Some(movie_path.to_string_lossy().as_ref())
    );

    let (wanted_items, total) = ctx
        .app
        .list_wanted_items(None, None, Some(&hydrated_title.id), 10, 0)
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
    assert_eq!(total, 0);
}

#[tokio::test]
async fn library_series_scan_handles_more_than_one_batch_of_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    for index in 0..300 {
        let folder = media_root.path().join(format!("Show {index:04}"));
        std::fs::create_dir_all(&folder).expect("create show dir");
        std::fs::write(
            folder.join("tvshow.nfo"),
            format!(
                "<tvshow><title>Show {index:04}</title><tvdbid>{}</tvdbid></tvshow>",
                900_000 + index
            ),
        )
        .expect("write tvshow.nfo");
    }

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": "/tmp/movies-unused",
            "seriesPath": media_root.path().display().to_string(),
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Series)
        .await
        .expect("scan library");

    assert_eq!(summary.scanned, 300);
    assert_eq!(summary.imported, 300);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let titles = ctx
        .db
        .list(Some(MediaFacet::Series), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 300);
    assert!(titles.iter().all(|title| !title.monitored));
}

#[tokio::test]
async fn library_movie_scan_handles_more_than_one_batch_of_titles() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    for index in 0..300 {
        let display_name = format!("Movie.Title.{index:04}.2024");
        let video_path = media_root.path().join(format!("{display_name}.mkv"));
        std::fs::write(&video_path, b"video").expect("write movie");
        std::fs::write(
            video_path.with_extension("nfo"),
            format!(
                "<movie><title>Movie {index:04}</title><tvdbid>{}</tvdbid><year>2024</year></movie>",
                800_000 + index
            ),
        )
        .expect("write movie nfo");
    }

    let update = gql(
        &ctx,
        r#"
        mutation UpdateLibraryPaths($input: UpdateLibraryPathsInput!) {
          updateLibraryPaths(input: $input) {
            moviePath
            seriesPath
            animePath
          }
        }
        "#,
        json!({
          "input": {
            "moviePath": media_root.path().display().to_string(),
            "seriesPath": "/tmp/series-unused",
            "animePath": "/tmp/anime-unused"
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let summary = ctx
        .app
        .scan_library(&admin, MediaFacet::Movie)
        .await
        .expect("scan movie library");

    assert_eq!(summary.scanned, 300);
    assert_eq!(summary.matched, 300);
    assert_eq!(summary.imported, 300);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.unmatched, 0);

    let titles = ctx
        .db
        .list(Some(MediaFacet::Movie), None)
        .await
        .expect("list titles");
    assert_eq!(titles.len(), 300);
    assert!(titles.iter().all(|title| !title.monitored));
}

#[tokio::test]
async fn graphql_delete_title() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "To Delete", "movie").await;

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"], true);

    // Verify deleted
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id } }"#,
        json!({ "id": id }),
    )
    .await;
    assert!(body["data"]["title"].is_null(), "title should be gone");
}

#[tokio::test]
async fn graphql_delete_title_cleans_title_workflow_state() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Delete With Cleanup", "movie").await;

    ctx.db
        .upsert_wanted_item(&WantedItem {
            id: Id::new().0,
            title_id: id.clone(),
            title_name: Some("Delete With Cleanup".to_string()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: "auto".to_string(),
            next_search_at: None,
            last_search_at: None,
            search_count: 0,
            baseline_date: None,
            status: scryer_application::WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            created_at: "2026-03-12T00:00:00Z".to_string(),
            updated_at: "2026-03-12T00:00:00Z".to_string(),
        })
        .await
        .expect("seed wanted item");
    ctx.db
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: "wanted-delete".to_string(),
            title_id: id.clone(),
            release_title: "Delete With Cleanup 2026".to_string(),
            release_url: Some("https://example.invalid/release.nzb".to_string()),
            source_kind: None,
            release_size_bytes: Some(1_024),
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("test-indexer".to_string()),
            release_guid: Some("guid-delete".to_string()),
            added_at: "2026-03-12T00:00:00Z".to_string(),
            delay_until: "2026-03-13T00:00:00Z".to_string(),
            status: scryer_application::PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
        })
        .await
        .expect("seed pending release");
    ctx.db
        .record_download_submission(
            id.clone(),
            "movie".to_string(),
            "sabnzbd".to_string(),
            "queue-delete".to_string(),
            Some("Delete With Cleanup".to_string()),
            None,
        )
        .await
        .expect("seed download submission");

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"], true);

    assert!(
        ctx.db
            .list_wanted_items(None, None, Some(&id), 10, 0)
            .await
            .expect("wanted items")
            .is_empty()
    );
    assert!(
        ctx.db
            .list_waiting_pending_releases()
            .await
            .expect("pending releases")
            .iter()
            .all(|entry| entry.title_id != id)
    );
    assert!(
        ctx.db
            .list_download_submissions_for_title(&id)
            .await
            .expect("download submissions")
            .is_empty()
    );
}

#[tokio::test]
async fn graphql_filter_titles_by_facet() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie A", "movie").await;
    add_test_title(&ctx, "Series A", "tv").await;

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { name facet } }"#,
        json!({ "facet": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["facet"], "movie");
}

#[tokio::test]
async fn graphql_series_titles_expose_tv_facet() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Series A", "tv").await;

    let body = gql(&ctx, "{ titles { name facet } }", json!({})).await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"].as_array().unwrap();
    let title = titles
        .iter()
        .find(|title| title["name"] == "Series A")
        .expect("series title should be present");
    assert_eq!(title["facet"], "tv");
}

// ---------------------------------------------------------------------------
// User management
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_me_query() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ me { id username } }", json!({})).await;
    assert_no_errors(&body);
    // auth-disabled mode creates an "admin" user
    assert_eq!(body["data"]["me"]["username"], "admin");
}

#[tokio::test]
async fn graphql_users_query() {
    let ctx = TestContext::new().await;
    // Trigger default admin user creation first
    gql(&ctx, "{ me { id } }", json!({})).await;

    let body = gql(&ctx, "{ users { id username } }", json!({})).await;
    assert_no_errors(&body);
    let users = body["data"]["users"].as_array().unwrap();
    assert!(!users.is_empty(), "should have at least one user");
}

#[tokio::test]
async fn graphql_create_user() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "testuser", "password": "testpass123", "entitlements": [] } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["createUser"]["username"], "testuser");
}

#[tokio::test]
async fn graphql_dev_auto_login() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { devAutoLogin { token user { username } } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["devAutoLogin"]["token"].is_string(),
        "should return token"
    );
    assert_eq!(body["data"]["devAutoLogin"]["user"]["username"], "admin");
}

// ---------------------------------------------------------------------------
// Download queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_download_queue_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadQueue { id titleName } }", json!({})).await;
    assert_no_errors(&body);
    let queue = body["data"]["downloadQueue"].as_array().unwrap();
    assert!(queue.is_empty(), "queue should start empty");
}

#[tokio::test]
async fn graphql_invalid_nzb_xml_queue_failure_is_blocklisted() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Broken NZB Movie", "movie").await;
    let source_hint = format!("{}/invalid.nzb", ctx.nzbget_server.uri());

    Mock::given(method("GET"))
        .and(path("/invalid.nzb"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not xml"))
        .mount(&ctx.nzbget_server)
        .await;

    let queue_body = gql(
        &ctx,
        r#"
        mutation($input: QueueDownloadInput!) {
          queueExistingTitleDownload(input: $input) {
            jobId
          }
        }
        "#,
        json!({
            "input": {
                "titleId": title_id,
                "release": {
                    "sourceHint": source_hint,
                    "sourceKind": "nzbFile",
                    "sourceTitle": "Broken.NZB.Movie.2024"
                }
            }
        }),
    )
    .await;

    assert!(
        queue_body.get("errors").is_some(),
        "expected queue mutation to fail for invalid nzb xml: {queue_body}"
    );
    let error_message = queue_body["errors"][0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(
        error_message.contains("did not look like xml")
            || error_message.contains("root element must be <nzb>")
            || error_message.contains("not valid xml"),
        "expected invalid-xml error message, got: {error_message}"
    );

    let blocklist_body = gql(
        &ctx,
        r#"
        query($titleId: String!) {
          titleReleaseBlocklist(titleId: $titleId) {
            sourceHint
            sourceTitle
            errorMessage
          }
        }
        "#,
        json!({ "titleId": title_id }),
    )
    .await;

    assert_no_errors(&blocklist_body);
    let entries = blocklist_body["data"]["titleReleaseBlocklist"]
        .as_array()
        .expect("blocklist entries array");
    assert!(
        entries.iter().any(|entry| {
            entry["sourceHint"].as_str() == Some(source_hint.as_str())
                && entry["sourceTitle"].as_str() == Some("Broken.NZB.Movie.2024")
                && entry["errorMessage"].as_str().is_some_and(|message| {
                    message.contains("did not look like xml")
                        || message.contains("root element must be <nzb>")
                        || message.contains("not valid xml")
                })
        }),
        "expected invalid nzb release to appear in titleReleaseBlocklist: {blocklist_body}"
    );
}

#[tokio::test]
async fn graphql_download_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ downloadHistory(limit: 50, offset: 0) { items { id titleName } hasMore } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let items = body["data"]["downloadHistory"]["items"].as_array().unwrap();
    assert!(items.is_empty(), "history should start empty");
    assert_eq!(body["data"]["downloadHistory"]["hasMore"], json!(false));
}

#[tokio::test]
async fn graphql_run_housekeeping_reports_pruned_staged_nzb_artifacts() {
    let ctx = TestContext::new().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");
    let staged = ctx
        .staged_nzb_store
        .stage_nzb_bytes_for_test(nzb_xml.as_bytes())
        .await
        .expect("staged artifact should insert");
    ctx.staged_nzb_store
        .set_staged_nzb_updated_at(&staged, Utc::now() - Duration::hours(2))
        .await
        .expect("staged artifact timestamp should update");

    let body = gql(
        &ctx,
        "mutation { runHousekeeping { stagedNzbArtifactsPruned } }",
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["runHousekeeping"]["stagedNzbArtifactsPruned"],
        1
    );
    assert_eq!(
        ctx.staged_nzb_store.count_staged_artifacts().await.unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------
// System health
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_system_health() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ systemHealth { serviceReady totalTitles } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["systemHealth"]["serviceReady"].is_boolean(),
        "should return serviceReady boolean"
    );
}

// ---------------------------------------------------------------------------
// Activity / events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_activity_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ activityEvents { id kind severity } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["activityEvents"].is_array());
}

#[tokio::test]
async fn graphql_title_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleEvents { id eventType sourceTitle quality occurredAt } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["titleEvents"].is_array());
}

#[tokio::test]
async fn graphql_title_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleHistory(filter: { limit: 10 }) { records { id eventType sourceTitle } totalCount } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titleHistory"]["totalCount"], 0);
    assert!(body["data"]["titleHistory"]["records"].is_array());
}

// ---------------------------------------------------------------------------
// Metadata queries (via SMG mock)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_search_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/search_tvdb_rich.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($query: String!, $type: String!) {
            searchMetadata(query: $query, type: $type) {
                tvdbId name year type overview posterUrl
            }
        }"#,
        json!({ "query": "Test Movie", "type": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    let results = body["data"]["searchMetadata"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["name"], "Test Movie Title");
}

#[tokio::test]
async fn graphql_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/get_movie.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($tvdbId: Int!) {
            metadataMovie(tvdbId: $tvdbId) {
                name year runtimeMinutes genres overview
            }
        }"#,
        json!({ "tvdbId": 123456 }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["metadataMovie"]["name"], "Test Movie Title");
    assert_eq!(body["data"]["metadataMovie"]["year"], 2024);
    assert_eq!(body["data"]["metadataMovie"]["runtimeMinutes"], 142);
}

#[tokio::test]
async fn graphql_metadata_series() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/get_series.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($id: String!) {
            metadataSeries(id: $id) {
                name year seasons { number label } episodes { name seasonNumber }
            }
        }"#,
        json!({ "id": "345678" }),
    )
    .await;
    assert_no_errors(&body);
    let series = &body["data"]["metadataSeries"];
    assert_eq!(series["name"], "Test Show Name");
    assert_eq!(series["seasons"].as_array().unwrap().len(), 2);
    assert_eq!(series["episodes"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn graphql_fix_title_match_movie_updates_identity_and_history() {
    let ctx = TestContext::new().await;
    mount_smg_mocks(&ctx, "smg/get_movie.json").await;

    let title = create_catalog_title(
        &ctx,
        "Broken Movie Match",
        MediaFacet::Movie,
        vec![
            ExternalId {
                source: "tvdb".to_string(),
                value: "999".to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt0000999".to_string(),
            },
            ExternalId {
                source: "tmdb".to_string(),
                value: "4444".to_string(),
            },
        ],
        vec!["scryer:quality-profile:4k".to_string()],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            hydrated
            warnings
            libraryScan { scanned }
            title {
              id
              name
              slug
              imdbId
              metadataFetchedAt
              tags
              externalIds { source value }
            }
          }
        }
        "#,
        json!({ "input": { "titleId": title.id, "tvdbId": "123456" } }),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["fixTitleMatch"];
    assert_eq!(payload["hydrated"], true);
    assert_eq!(payload["warnings"], json!([]));
    assert!(payload["libraryScan"].is_null());
    assert_eq!(payload["title"]["name"], "Broken Movie Match");
    assert_eq!(payload["title"]["slug"], "test-movie-title");
    assert_eq!(payload["title"]["imdbId"], "tt1234567");
    assert!(payload["title"]["metadataFetchedAt"].is_string());

    let tags = payload["title"]["tags"].as_array().expect("tags array");
    assert!(tags.contains(&json!("scryer:quality-profile:4k")));

    let external_ids = payload["title"]["externalIds"]
        .as_array()
        .expect("external ids array");
    assert!(
        external_ids
            .iter()
            .any(|value| { value["source"] == "tvdb" && value["value"] == "123456" })
    );
    assert!(
        external_ids
            .iter()
            .any(|value| { value["source"] == "imdb" && value["value"] == "tt1234567" })
    );
    assert!(
        !external_ids
            .iter()
            .any(|value| { value["source"] == "tvdb" && value["value"] == "999" })
    );
    assert!(!external_ids.iter().any(|value| value["source"] == "tmdb"));

    let events = gql(
        &ctx,
        r#"
        query TitleEvents($titleId: String!) {
          titleEvents(titleId: $titleId, limit: 10) {
            eventType
            dataJson
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&events);
    let rematch_events = events["data"]["titleEvents"]
        .as_array()
        .expect("title events array");
    let rematch_event = rematch_events
        .iter()
        .find(|event| event["eventType"] == "rematched")
        .expect("rematched history event");
    let data_json = rematch_event["dataJson"]
        .as_str()
        .expect("rematch data json");
    let data_value: Value = serde_json::from_str(data_json).expect("parse rematch data");
    assert_eq!(data_value["old_tvdb_id"], "999");
    assert_eq!(data_value["new_tvdb_id"], "123456");
    assert_eq!(data_value["source"], "manual");

    let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_started")
    );
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_completed")
    );
}

#[tokio::test]
async fn graphql_fix_title_match_series_rebuilds_and_relinks_library() {
    let ctx = TestContext::new().await;
    mount_smg_mocks(&ctx, "smg/get_series.json").await;

    let media_root = tempfile::tempdir().expect("media root tempdir");
    let title_name = "Broken Series Match";
    let title = create_catalog_title(
        &ctx,
        title_name,
        MediaFacet::Series,
        vec![
            ExternalId {
                source: "tvdb".to_string(),
                value: "999".to_string(),
            },
            ExternalId {
                source: "mal".to_string(),
                value: "5555".to_string(),
            },
        ],
        vec![
            format!("scryer:root-folder:{}", media_root.path().display()),
            "scryer:season-folder:enabled".to_string(),
            "scryer:anime-status:finished".to_string(),
        ],
        true,
    )
    .await;

    let old_collection = ctx
        .db
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "99".to_string(),
            label: Some("Legacy Season".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create old collection");

    let old_episode = ctx
        .db
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(old_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("99".to_string()),
            episode_label: Some("S99E01".to_string()),
            title: Some("Legacy Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: Some("Legacy episode".to_string()),
            tvdb_id: Some("9999001".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create old episode");

    let season_dir = media_root.path().join(title_name).join("Season 01");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    let file_path = season_dir.join("Broken.Series.Match.S01E01.1080p.WEB-DL.mkv");
    std::fs::write(&file_path, b"not-a-real-video").expect("write fake video");
    let file_id = ctx
        .db
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: file_path.to_string_lossy().to_string(),
            size_bytes: 1024,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.db
        .link_file_to_episode(&file_id, &old_episode.id)
        .await
        .expect("link file to legacy episode");

    let body = gql(
        &ctx,
        r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            hydrated
            warnings
            libraryScan {
              scanned
              matched
              imported
              skipped
              unmatched
            }
            title {
              id
              name
              tags
              externalIds { source value }
              collections {
                id
                collectionIndex
                episodes {
                  id
                  seasonNumber
                  episodeNumber
                  title
                }
              }
              mediaFiles {
                episodeId
                filePath
              }
            }
          }
        }
        "#,
        json!({ "input": { "titleId": title.id, "tvdbId": "345678" } }),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["fixTitleMatch"];
    assert_eq!(payload["hydrated"], true);
    assert_eq!(payload["warnings"], json!([]));
    assert_eq!(payload["title"]["name"], title_name);
    assert_eq!(payload["libraryScan"]["scanned"], 1);
    assert_eq!(payload["libraryScan"]["unmatched"], 0);

    let tags = payload["title"]["tags"].as_array().expect("tags array");
    assert!(tags.contains(&json!(format!(
        "scryer:root-folder:{}",
        media_root.path().display()
    ))));
    assert!(tags.contains(&json!("scryer:season-folder:enabled")));
    assert!(!tags.contains(&json!("scryer:anime-status:finished")));

    let external_ids = payload["title"]["externalIds"]
        .as_array()
        .expect("external ids array");
    assert!(
        external_ids
            .iter()
            .any(|value| { value["source"] == "tvdb" && value["value"] == "345678" })
    );
    assert!(!external_ids.iter().any(|value| value["source"] == "mal"));

    let collections = payload["title"]["collections"]
        .as_array()
        .expect("collections array");
    assert_eq!(collections.len(), 2);
    assert!(
        !collections
            .iter()
            .any(|collection| collection["id"] == old_collection.id)
    );
    let rebuilt_episode_count: usize = collections
        .iter()
        .map(|collection| {
            collection["episodes"]
                .as_array()
                .expect("episodes array")
                .len()
        })
        .sum();
    assert_eq!(rebuilt_episode_count, 3);

    let media_files = payload["title"]["mediaFiles"]
        .as_array()
        .expect("media files array");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0]["filePath"],
        file_path.to_string_lossy().to_string()
    );
    let relinked_episode_id = media_files[0]["episodeId"]
        .as_str()
        .expect("media file should relink to rebuilt episode");
    assert_ne!(relinked_episode_id, old_episode.id);

    let events = gql(
        &ctx,
        r#"
        query TitleEvents($titleId: String!) {
          titleEvents(titleId: $titleId, limit: 10) {
            eventType
            dataJson
          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&events);
    let rematch_events = events["data"]["titleEvents"]
        .as_array()
        .expect("title events array");
    let rematch_event = rematch_events
        .iter()
        .find(|event| event["eventType"] == "rematched")
        .expect("rematched history event");
    let data_json = rematch_event["dataJson"]
        .as_str()
        .expect("rematch data json");
    let data_value: Value = serde_json::from_str(data_json).expect("parse rematch data");
    assert_eq!(data_value["old_tvdb_id"], "999");
    assert_eq!(data_value["new_tvdb_id"], "345678");

    let activity_kinds = activity_kinds_for_title(&ctx, &title.id).await;
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_started")
    );
    assert!(
        activity_kinds
            .iter()
            .any(|kind| kind == "metadata_hydration_completed")
    );
}

#[tokio::test]
async fn graphql_fix_title_match_rejects_duplicate_target_tvdb_id() {
    let ctx = TestContext::new().await;
    let existing = create_catalog_title(
        &ctx,
        "Existing Correct Match",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let broken = create_catalog_title(
        &ctx,
        "Broken Match",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "999".to_string(),
        }],
        vec![],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"
        mutation FixTitleMatch($input: FixTitleMatchInput!) {
          fixTitleMatch(input: $input) {
            title { id }
          }
        }
        "#,
        json!({ "input": { "titleId": broken.id, "tvdbId": "123456" } }),
    )
    .await;

    assert!(
        body.get("errors").is_some(),
        "expected graphql errors: {body}"
    );
    let message = body["errors"][0]["message"]
        .as_str()
        .expect("graphql error message");
    assert!(message.contains("tvdb id 123456 is already assigned to title"));
    assert!(message.contains(&existing.name));
}

// ---------------------------------------------------------------------------
// Configuration (indexers + download clients)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_indexers_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ indexers { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["indexers"].is_array());
}

#[tokio::test]
async fn graphql_download_client_configs_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadClientConfigs { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["downloadClientConfigs"].is_array());
}

// ---------------------------------------------------------------------------
// Wanted items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_wanted_items_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($status: WantedStatusValue, $mediaType: WantedMediaTypeValue) {
            wantedItems(status: $status, mediaType: $mediaType) {
                items { id }
                total
            }
        }"#,
        json!({ "status": "wanted", "mediaType": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["wantedItems"]["total"], 0,
        "should have no wanted items initially"
    );
}

// ---------------------------------------------------------------------------
// Rule sets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_rule_sets_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ ruleSets { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Import history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_import_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ importHistory { id sourceTitle status } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["importHistory"].is_array());
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_calendar_episodes() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($start: String!, $end: String!) {
            calendarEpisodes(startDate: $start, endDate: $end) {
                episodeTitle seasonNumber episodeNumber
            }
        }"#,
        json!({ "start": "2024-01-01", "end": "2024-12-31" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["calendarEpisodes"].is_array());
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_unknown_field_returns_error() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ nonExistentField }", json!({})).await;
    assert!(
        body.get("errors").is_some(),
        "unknown field should return errors"
    );
}

#[tokio::test]
async fn graphql_invalid_mutation_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { addTitle(input: { name: "" }) { title { id } } }"#,
        json!({}),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "invalid input should return errors"
    );
}

#[tokio::test]
async fn graphql_batch_request_not_supported_via_single() {
    let ctx = TestContext::new().await;
    // Verify single requests work (batch is handled at the middleware level)
    let body = gql(&ctx, "{ titles { id } }", json!({})).await;
    assert_no_errors(&body);
}

// ---------------------------------------------------------------------------
// Authentication flow
// ---------------------------------------------------------------------------

/// The login mutation is available without a pre-existing session.
/// After providing valid credentials, the server returns a non-empty JWT.
///
/// Note: the migration-seeded "admin" user has a NULL password_hash (it is
/// intended for dev-mode auto-login, not credential-based login).  We
/// therefore create a fresh user with an explicit password to exercise the
/// full login path.
#[tokio::test]
async fn login_with_valid_credentials_returns_token() {
    let ctx = TestContext::new().await;

    // Need an actor to create the test user — admin has all entitlements.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "logintest".to_string(),
            "s3cr3t!".to_string(),
            vec![],
        )
        .await
        .unwrap();

    let body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "logintest", password: "s3cr3t!" }) { token expiresAt user { username } } }"#,
        None,
    )
    .await;

    assert!(
        body["errors"].is_null(),
        "login should not return errors: {body}"
    );
    let token = body["data"]["login"]["token"].as_str().unwrap();
    assert!(!token.is_empty(), "JWT token should not be empty");
    assert_eq!(body["data"]["login"]["user"]["username"], "logintest");
}

/// Providing the wrong password must produce a GraphQL error — never a token.
#[tokio::test]
async fn login_with_wrong_password_returns_error() {
    let ctx = TestContext::new().await;

    // Create a user with a known password so we can test wrong-password rejection.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "wrongpasstest".to_string(),
            "correct_horse".to_string(),
            vec![],
        )
        .await
        .unwrap();

    let body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "wrongpasstest", password: "wrong_password" }) { token } }"#,
        None,
    )
    .await;

    assert!(
        !body["errors"].is_null()
            && body["errors"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
        "wrong password should return a GraphQL error: {body}"
    );
    // Verify the error indicates bad credentials, not a server error.
    let error_msg = body["errors"][0]["message"].as_str().unwrap_or("");
    assert!(
        error_msg.to_ascii_lowercase().contains("credentials")
            || error_msg.to_ascii_lowercase().contains("invalid"),
        "error should indicate bad credentials: {error_msg}"
    );
}

/// Most queries require a user in the request context.  Executing one via the
/// schema directly (without injecting a User) must return an authentication
/// error rather than leaking data.
#[tokio::test]
async fn unauthenticated_request_returns_error() {
    let ctx = TestContext::new().await;

    // `titles` calls actor_from_ctx — must fail without a user in context.
    let body = schema_exec(&ctx, "{ titles { id } }", None).await;

    let errors = body["errors"].as_array().expect("should have errors");
    assert!(
        !errors.is_empty(),
        "unauthenticated request should return errors"
    );
    let messages: Vec<&str> = errors
        .iter()
        .filter_map(|e| e["message"].as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.to_ascii_lowercase().contains("auth")),
        "error message should mention authentication: {messages:?}"
    );
}

/// After obtaining a JWT via the login mutation, the caller can authenticate
/// that token to retrieve the User and use it on a protected query.
#[tokio::test]
async fn authenticated_request_with_valid_token_succeeds() {
    let ctx = TestContext::new().await;

    // Create a user with an explicit password and ViewCatalog so the
    // protected `titles` query can succeed.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .create_user(
            &admin,
            "authtest".to_string(),
            "s3cr3t!".to_string(),
            vec![scryer_domain::Entitlement::ViewCatalog],
        )
        .await
        .unwrap();

    // Step 1: log in and capture the token.
    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "authtest", password: "s3cr3t!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"]
        .as_str()
        .expect("token should be a string")
        .to_string();

    // Step 2: validate the token to recover the User.
    let user = ctx
        .app
        .authenticate_token(&token)
        .await
        .expect("token should be valid");

    // Step 3: execute a protected query with the authenticated user attached.
    let body = schema_exec(&ctx, "{ titles { id } }", Some(user)).await;
    assert!(
        body["errors"].is_null(),
        "authenticated query should not error: {body}"
    );
    assert!(body["data"]["titles"].is_array());
}

#[tokio::test]
async fn token_is_revoked_after_set_user_entitlements_until_relogin() {
    let ctx = TestContext::new().await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    let create_body = schema_exec(
        &ctx,
        r#"mutation {
            createUser(input: {
                username: "entrevoketest",
                password: "s3cr3t!",
                entitlements: ["view_catalog"]
            }) {
                id
                username
            }
        }"#,
        Some(admin.clone()),
    )
    .await;
    assert!(
        create_body["errors"].is_null(),
        "createUser should succeed: {create_body}"
    );
    let user_id = create_body["data"]["createUser"]["id"]
        .as_str()
        .expect("created user id")
        .to_string();

    let login_before = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "entrevoketest", password: "s3cr3t!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_before["errors"].is_null(),
        "initial login should succeed: {login_before}"
    );
    let old_token = login_before["data"]["login"]["token"]
        .as_str()
        .expect("token should be a string")
        .to_string();

    let update_body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
                setUserEntitlements(input: {{
                    userId: "{user_id}",
                    entitlements: ["view_catalog", "manage_title"]
                }}) {{
                    id
                    entitlements
                }}
            }}"#
        ),
        Some(admin),
    )
    .await;
    assert!(
        update_body["errors"].is_null(),
        "setUserEntitlements should succeed: {update_body}"
    );

    let old_result = ctx.app.authenticate_token(&old_token).await;
    assert!(
        old_result.is_err(),
        "token issued before entitlement change should be rejected"
    );

    let login_after = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "entrevoketest", password: "s3cr3t!" }) { token } }"#,
        None,
    )
    .await;
    assert!(
        login_after["errors"].is_null(),
        "re-login should succeed after entitlement change: {login_after}"
    );
    let new_token = login_after["data"]["login"]["token"]
        .as_str()
        .expect("refreshed token should be a string")
        .to_string();

    let decoded = ctx
        .app
        .authenticate_token(&new_token)
        .await
        .expect("refreshed token should authenticate");
    assert!(
        decoded
            .entitlements
            .contains(&scryer_domain::Entitlement::ManageTitle),
        "re-issued token should carry updated entitlements"
    );
}

/// A token issued for a different issuer (or an arbitrary tampered token)
/// must be rejected by `authenticate_token` — not by a GraphQL error but as
/// a hard application-level failure.
#[tokio::test]
async fn tampered_token_is_rejected_by_authenticate_token() {
    let ctx = TestContext::new().await;

    // Craft a syntactically valid-looking but unsigned JWT (three base64 parts).
    let fake_token = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJoYWNrZXIifQ.invalidsig";

    let result = ctx.app.authenticate_token(fake_token).await;
    assert!(
        result.is_err(),
        "tampered/unsigned token must not be accepted"
    );
}

/// Creating a user with `createUser` and then logging in as that user must
/// succeed end-to-end — confirming that the password is stored and validated
/// consistently.
#[tokio::test]
async fn newly_created_user_can_login() {
    let ctx = TestContext::new().await;

    // The admin user must exist before we can create another user
    // (createUser requires ManageConfig entitlement).
    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    // Create a new user as admin.
    let create_body = schema_exec(
        &ctx,
        r#"mutation { createUser(input: { username: "newuser", password: "s3cr3t!", entitlements: [] }) { id username } }"#,
        Some(admin),
    )
    .await;
    assert!(
        create_body["errors"].is_null(),
        "createUser should succeed: {create_body}"
    );
    assert_eq!(create_body["data"]["createUser"]["username"], "newuser");

    // Log in as the newly created user.
    let login_body = schema_exec(
        &ctx,
        r#"mutation { login(input: { username: "newuser", password: "s3cr3t!" }) { token user { username } } }"#,
        None,
    )
    .await;
    assert!(
        login_body["errors"].is_null(),
        "new user login should succeed: {login_body}"
    );
    let token = login_body["data"]["login"]["token"].as_str().unwrap();
    assert!(!token.is_empty());
    assert_eq!(login_body["data"]["login"]["user"]["username"], "newuser");
}
