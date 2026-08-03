use super::*;
use super::{check::*, execute::*, lookup::*, path_state::*, result_state::*, verification::*};

mod category_gate;
mod lookup_identity;
mod notifications;
mod path_state;
mod result_state;
mod snapshot_resolution;
mod verification;

use crate::null_repositories::test_nulls::{
    NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
    NullReleaseAttemptRepository, NullUserRepository,
};
use crate::{
    ActivityKind, AppError, AppResult, AppServices, AppUseCase, CollectionUpdate,
    CreateTitleOutcome, DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, DomainEventRepository, DownloadClient,
    DownloadClientAddRequest, DownloadClientConfigRepository, DownloadGrabResult,
    DownloadSourceIdentity, DownloadSubmission, DownloadSubmissionIdentity,
    DownloadSubmissionRepository, EpisodeUpdate, FacetRegistry, ImportArtifact,
    ImportArtifactRepository, IndexerConfigRepository, JwtAuthConfig, PendingTitleHydration,
    QualityProfile, QualityProfileRepository, SETTINGS_SCOPE_SYSTEM, ScopedExternalId,
    SeriesMovieExternalIdLookupMatch, SettingsRepository, ShowRepository, SubmissionScope,
    TitleExternalIdLookup, TitleMetadataUpdate, TitleRepository,
};
use async_trait::async_trait;
use chrono::Utc;
use scryer_domain::{
    CalendarEpisode, Collection, CollectionType, DomainEvent, DomainEventFilter,
    DownloadClientConfig, DownloadQueueItem, DownloadQueueState, Episode, EpisodeType, Id,
    MediaFacet, NewDomainEvent, Title, TitleHistoryEventType, TitleMatchType, TrackedDownloadState,
    TrackedDownloadStatus, User,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex;

type ScopedRecentCompletedCalls = Vec<(Vec<String>, Vec<String>)>;

#[derive(Default)]
struct TestTitleRepo {
    titles: Arc<Mutex<Vec<Title>>>,
}

#[async_trait]
impl TitleRepository for TestTitleRepo {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let titles = self.titles.lock().await.clone();
        Ok(titles
            .into_iter()
            .filter(|title| {
                facet
                    .as_ref()
                    .is_none_or(|expected| &title.facet == expected)
            })
            .filter(|title| {
                query.as_ref().is_none_or(|value| {
                    title
                        .name
                        .to_ascii_lowercase()
                        .contains(&value.to_ascii_lowercase())
                })
            })
            .collect())
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        let requested: Vec<&str> = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();
        let titles = self.titles.lock().await;
        let mut matches = Vec::new();
        let mut seen = HashSet::new();
        for value in requested {
            if let Some(title) = titles.iter().find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            }) && seen.insert(title.id.clone())
            {
                matches.push(title.clone());
            }
        }
        Ok(matches)
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let titles = self.titles.lock().await;
        Ok(titles.iter().find(|title| title.id == id).cloned())
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() {
            return Ok(None);
        }

        let titles = self.titles.lock().await;
        let matches = titles
            .iter()
            .filter(|title| {
                title.facet == facet
                    && title.slug.as_deref().is_some_and(|candidate| {
                        candidate.trim().eq_ignore_ascii_case(normalized_slug)
                    })
            })
            .cloned()
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(AppError::Validation(
                "multiple titles found for slug lookup".into(),
            )),
        }
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let titles = self.titles.lock().await;
        Ok(titles
            .iter()
            .find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            })
            .cloned())
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let titles = self.titles.lock().await;
        Ok(titles
            .iter()
            .find(|title| {
                title.facet == facet
                    && title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
            })
            .cloned())
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        Ok(CreateTitleOutcome {
            title: self.create(title).await?,
            reused_existing: false,
        })
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        self.titles.lock().await.push(title.clone());
        Ok(title)
    }

    async fn list_titles_due_for_hydration(
        &self,
        _: usize,
        _: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        Ok(vec![])
    }

    async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        _: &str,
        _: &str,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update_metadata(
        &self,
        _: &str,
        _: Option<String>,
        _: Option<MediaFacet>,
        _: Option<Vec<String>>,
        _: Option<String>,
    ) -> AppResult<Title> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn update_title_hydrated_metadata(
        &self,
        _: &str,
        _: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn replace_match_state(
        &self,
        _: &str,
        _: Vec<scryer_domain::ExternalId>,
        _: Vec<String>,
    ) -> AppResult<Title> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
struct TestShowRepo {
    collections: Arc<Mutex<Vec<Collection>>>,
    episodes: Arc<Mutex<Vec<Episode>>>,
    series_movie_links: Arc<Mutex<Vec<scryer_domain::SeriesMovieLink>>>,
}

#[async_trait]
impl ShowRepository for TestShowRepo {
    async fn list_series_movie_links_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SeriesMovieLink>> {
        Ok(self
            .series_movie_links
            .lock()
            .await
            .iter()
            .filter(|link| link.series_title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_series_movie_external_id_lookup_matches(
        &self,
        _: &[String],
        _: &[TitleExternalIdLookup],
    ) -> AppResult<Vec<SeriesMovieExternalIdLookupMatch>> {
        Ok(vec![])
    }

    async fn get_series_movie_link_by_id(
        &self,
        _: &str,
    ) -> AppResult<Option<scryer_domain::SeriesMovieLink>> {
        Ok(None)
    }

    async fn find_series_movie_link_by_legacy_collection_id(
        &self,
        _: &str,
    ) -> AppResult<Option<scryer_domain::SeriesMovieLink>> {
        Ok(None)
    }

    async fn upsert_series_movie_link(
        &self,
        link: scryer_domain::SeriesMovieLink,
    ) -> AppResult<scryer_domain::SeriesMovieLink> {
        let mut links = self.series_movie_links.lock().await;
        links.retain(|existing| existing.id != link.id);
        links.push(link.clone());
        Ok(link)
    }

    async fn delete_stale_series_movie_links(&self, _: &str, _: &[String]) -> AppResult<()> {
        Ok(())
    }

    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .filter(|collection| collection.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_collection_external_ids(&self, _: &str) -> AppResult<Vec<ScopedExternalId>> {
        Ok(vec![])
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<Collection>>> {
        let collections = self.collections.lock().await;
        let wanted = title_ids.iter().cloned().collect::<HashSet<_>>();
        let mut grouped = std::collections::HashMap::<String, Vec<Collection>>::new();
        for collection in collections.iter() {
            if wanted.contains(&collection.title_id) {
                grouped
                    .entry(collection.title_id.clone())
                    .or_default()
                    .push(collection.clone());
            }
        }
        Ok(grouped)
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .cloned())
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .find(|collection| collection.ordered_path.as_deref() == Some(ordered_path))
            .cloned())
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        self.collections.lock().await.push(collection.clone());
        Ok(collection)
    }

    async fn update_collection(&self, _: &str, _: CollectionUpdate) -> AppResult<Collection> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn set_collection_episodes_monitored(&self, _: &str, _: bool) -> AppResult<()> {
        Ok(())
    }

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let wanted = collection_ids.iter().cloned().collect::<HashSet<_>>();
        let mut collections = self.collections.lock().await;
        for collection in collections.iter_mut() {
            if wanted.contains(&collection.id) {
                collection.monitored = monitored;
            }
        }
        Ok(())
    }

    async fn delete_collection(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn delete_collections_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .filter(|episode| episode.collection_id.as_deref() == Some(collection_id))
            .cloned()
            .collect())
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .filter(|episode| episode.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_episode_external_ids(&self, _: &str) -> AppResult<Vec<ScopedExternalId>> {
        Ok(vec![])
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|episode| episode.id == episode_id)
            .cloned())
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        self.episodes.lock().await.push(episode.clone());
        Ok(episode)
    }

    async fn update_episode(&self, _: &str, _: EpisodeUpdate) -> AppResult<Episode> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let wanted = episode_ids.iter().cloned().collect::<HashSet<_>>();
        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if wanted.contains(&episode.id) {
                episode.monitored = monitored;
            }
        }
        Ok(())
    }

    async fn delete_episode(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn delete_episodes_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|episode| {
                episode.title_id == title_id
                    && episode.season_number.as_deref() == Some(season_number)
                    && episode.episode_number.as_deref() == Some(episode_number)
            })
            .cloned())
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|episode| {
                episode.title_id == title_id
                    && episode.absolute_number.as_deref() == Some(absolute_number)
            })
            .cloned())
    }

    async fn list_primary_collection_summaries(
        &self,
        _: &[String],
    ) -> AppResult<Vec<crate::PrimaryCollectionSummary>> {
        Ok(vec![])
    }

    async fn list_episodes_in_date_range(
        &self,
        _: &str,
        _: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        Ok(vec![])
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        _: &str,
        _: Vec<ScopedExternalId>,
        _: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
struct TestImportArtifactRepo {
    artifacts: Arc<Mutex<Vec<ImportArtifact>>>,
}

#[async_trait]
impl ImportArtifactRepository for TestImportArtifactRepo {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()> {
        self.artifacts.lock().await.push(artifact);
        Ok(())
    }

    async fn list_by_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Vec<ImportArtifact>> {
        let artifacts = self.artifacts.lock().await;
        Ok(artifacts
            .iter()
            .filter(|artifact| artifact.source_identity() == *identity)
            .cloned()
            .collect())
    }

    async fn count_by_result_for_source_identity(
        &self,
        identity: &DownloadSourceIdentity,
        result: &str,
    ) -> AppResult<u64> {
        let artifacts = self.artifacts.lock().await;
        Ok(artifacts
            .iter()
            .filter(|artifact| artifact.source_identity() == *identity && artifact.result == result)
            .count() as u64)
    }
}

#[derive(Default)]
struct TestDownloadSubmissionRepo {
    rows: Arc<Mutex<Vec<(DownloadSubmission, DownloadSubmissionIdentity)>>>,
    tracked_states: Arc<Mutex<Vec<(DownloadSourceIdentity, String)>>>,
    identity_tracked_states: Arc<Mutex<Vec<(String, String)>>>,
}

fn test_download_identity_state_key(
    identity: &DownloadSubmissionIdentity,
    source_identity: Option<&DownloadSourceIdentity>,
) -> Option<String> {
    let download_id = identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if download_id.starts_with("scryer-download:")
        || (matches!(download_id.len(), 40 | 64)
            && download_id.chars().all(|ch| ch.is_ascii_hexdigit()))
    {
        return Some(format!("download:{download_id}"));
    }

    let source_identity = source_identity?;
    let client_type = source_identity.client_type.trim();
    if client_type.is_empty() {
        return None;
    }
    Some(format!(
        "client:{}:{}:download:{}",
        source_identity
            .client_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default(),
        client_type.to_ascii_lowercase(),
        download_id
    ))
}

#[async_trait]
impl DownloadSubmissionRepository for TestDownloadSubmissionRepo {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .push((submission, DownloadSubmissionIdentity::default()));
        Ok(())
    }

    async fn record_submission_with_identity(
        &self,
        submission: DownloadSubmission,
        submission_identity: DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .push((submission, submission_identity));
        Ok(())
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .find(|(submission, _)| {
                DownloadSourceIdentity::from_submission(submission) == *identity
            })
            .map(|(submission, _)| submission.clone()))
    }

    async fn list_by_download_id(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(submission, identity)| {
                submission.download_client_id.as_deref().unwrap_or("") == client_id.unwrap_or("")
                    && submission
                        .download_client_type
                        .eq_ignore_ascii_case(client_type)
                    && identity.download_id.as_deref() == Some(download_id)
            })
            .map(|(submission, _)| submission.clone())
            .collect())
    }

    async fn get_submission_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmissionIdentity>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .find(|(submission, _)| {
                DownloadSourceIdentity::from_submission(submission) == *identity
            })
            .map(|(_, submission_identity)| submission_identity.clone()))
    }

    async fn record_identity_tracked_state(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
        tracked_state: &str,
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<()> {
        let Some(key) = test_download_identity_state_key(identity, source_identity) else {
            return Ok(());
        };
        let mut states = self.identity_tracked_states.lock().await;
        if let Some((_, state)) = states.iter_mut().find(|(stored_key, _)| stored_key == &key) {
            *state = tracked_state.to_string();
        } else {
            states.push((key, tracked_state.to_string()));
        }
        Ok(())
    }

    async fn get_identity_tracked_state(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(key) = test_download_identity_state_key(identity, source_identity) else {
            return Ok(None);
        };
        Ok(self
            .identity_tracked_states
            .lock()
            .await
            .iter()
            .find(|(stored_key, _)| stored_key == &key)
            .map(|(_, state)| state.clone()))
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(submission, _)| {
                let identity = DownloadSourceIdentity::from_submission(submission);
                client_items.contains(&identity)
            })
            .map(|(submission, _)| submission.clone())
            .collect())
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|(submission, _)| submission.title_id == title_id)
            .map(|(submission, _)| submission.clone())
            .collect())
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
        purpose: crate::DownloadSubmissionPurpose,
        scope: &crate::SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .find(|(submission, _)| {
                submission.title_id == title_id
                    && submission.request_signature.as_deref() == Some(request_signature)
                    && submission.purpose == purpose
                    && &submission.scope == scope
            })
            .map(|(submission, _)| submission.clone()))
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.rows
            .lock()
            .await
            .retain(|(submission, _)| submission.title_id != title_id);
        Ok(())
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        self.rows.lock().await.retain(|(submission, _)| {
            DownloadSourceIdentity::from_submission(submission) != *identity
        });
        Ok(())
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        let mut states = self.tracked_states.lock().await;
        if let Some((_, state)) = states
            .iter_mut()
            .find(|(stored_identity, _)| stored_identity == identity)
        {
            *state = tracked_state.to_string();
        } else {
            states.push((identity.clone(), tracked_state.to_string()));
        }
        Ok(())
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        Ok(self
            .tracked_states
            .lock()
            .await
            .iter()
            .find(|(stored_identity, _)| stored_identity == identity)
            .map(|(_, state)| state.clone()))
    }
}

struct TestDownloadClientConfigRepo {
    configs: Vec<DownloadClientConfig>,
}

#[async_trait]
impl DownloadClientConfigRepository for TestDownloadClientConfigRepo {
    async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        Ok(self.configs.clone())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        Ok(self.configs.iter().find(|config| config.id == id).cloned())
    }

    async fn create(&self, _config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn update(
        &self,
        _update: crate::DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn delete(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn reorder(&self, _ordered_ids: Vec<String>) -> AppResult<()> {
        Err(AppError::Repository("not needed in test".into()))
    }
}

#[derive(Default)]
struct TestIndexerConfigRepo;

#[async_trait]
impl IndexerConfigRepository for TestIndexerConfigRepo {
    async fn list(&self, _: Option<String>) -> AppResult<Vec<scryer_domain::IndexerConfig>> {
        Ok(vec![])
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::IndexerConfig>> {
        Ok(None)
    }

    async fn create(
        &self,
        _: scryer_domain::IndexerConfig,
    ) -> AppResult<scryer_domain::IndexerConfig> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn touch_last_error(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update(
        &self,
        _: crate::IndexerConfigUpdate,
    ) -> AppResult<scryer_domain::IndexerConfig> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
struct TestQualityProfileRepo;

#[async_trait]
impl QualityProfileRepository for TestQualityProfileRepo {
    async fn list_quality_profiles(
        &self,
        _: &str,
        _: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        Ok(vec![])
    }

    async fn replace_quality_profiles(
        &self,
        _: &str,
        _: Option<String>,
        _: Vec<QualityProfile>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
struct TestDomainEventRepo {
    events: Arc<Mutex<Vec<DomainEvent>>>,
    subscriber_offsets: Arc<Mutex<std::collections::HashMap<String, i64>>>,
}

#[derive(Default)]
struct TestDownloadClient {
    completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
    completed_download_calls: Arc<AtomicUsize>,
    recent_completed_download_calls: Arc<AtomicUsize>,
    scoped_recent_completed_calls: Arc<Mutex<ScopedRecentCompletedCalls>>,
}

#[async_trait]
impl DownloadClient for TestDownloadClient {
    async fn submit_download(&self, _: &DownloadClientAddRequest) -> AppResult<DownloadGrabResult> {
        Err(AppError::Repository("not needed in test".into()))
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        self.completed_download_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.completed_downloads.lock().await.clone())
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        self.recent_completed_download_calls
            .fetch_add(1, Ordering::SeqCst);
        Ok(self
            .completed_downloads
            .lock()
            .await
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_recent_completed_downloads_for_client_scope(
        &self,
        limit: usize,
        client_ids: &[String],
        client_types: &[String],
        excluded_client_types: &[&str],
    ) -> AppResult<Vec<CompletedDownload>> {
        self.scoped_recent_completed_calls
            .lock()
            .await
            .push((client_ids.to_vec(), client_types.to_vec()));
        let mut items = self.list_recent_completed_downloads(limit).await?;
        items.retain(|item| {
            let item_type = item.client_type.trim();
            if excluded_client_types
                .iter()
                .any(|client_type| item_type.eq_ignore_ascii_case(client_type.trim()))
            {
                return false;
            }

            let has_scope = !client_ids.is_empty() || !client_types.is_empty();
            if !has_scope {
                return true;
            }

            let item_client_id = item.client_id.trim();
            let id_matches = !item_client_id.is_empty()
                && client_ids
                    .iter()
                    .any(|client_id| item_client_id == client_id.trim());
            let type_matches = !item_type.is_empty()
                && client_types
                    .iter()
                    .any(|client_type| item_type.eq_ignore_ascii_case(client_type.trim()));

            if !client_ids.is_empty() {
                id_matches && (client_types.is_empty() || type_matches)
            } else {
                type_matches
            }
        });
        Ok(items)
    }
}

#[async_trait]
impl DomainEventRepository for TestDomainEventRepo {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let mut events = self.events.lock().await;
        let sequence = events
            .last()
            .map(|existing| existing.sequence + 1)
            .unwrap_or(1);
        let stored = DomainEvent {
            sequence,
            event_id: event.event_id,
            occurred_at: event.occurred_at,
            actor_kind: event.actor_kind,
            actor_user_id: event.actor_user_id,
            actor_display_name: event.actor_display_name,
            title_id: event.title_id,
            facet: event.facet,
            correlation_id: event.correlation_id,
            causation_id: event.causation_id,
            schema_version: event.schema_version,
            stream: event.stream,
            payload: event.payload,
        };
        events.push(stored.clone());
        Ok(stored)
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        let mut stored = Vec::with_capacity(events.len());
        for event in events {
            stored.push(self.append(event).await?);
        }
        Ok(stored)
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        let events = self.events.lock().await;
        let limit = if filter.limit == 0 {
            usize::MAX
        } else {
            filter.limit
        };
        let iter: Box<dyn Iterator<Item = &DomainEvent>> =
            if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
                Box::new(events.iter())
            } else {
                Box::new(events.iter().rev())
            };
        Ok(iter
            .filter(|event| {
                filter
                    .after_sequence
                    .is_none_or(|after| event.sequence > after)
                    && filter
                        .before_sequence
                        .is_none_or(|before| event.sequence < before)
                    && filter
                        .title_id
                        .as_ref()
                        .is_none_or(|title_id| event.title_id.as_deref() == Some(title_id.as_str()))
                    && filter
                        .facet
                        .as_ref()
                        .is_none_or(|facet| event.facet.as_ref() == Some(facet))
                    && filter.event_types.as_ref().is_none_or(|event_types| {
                        event_types
                            .iter()
                            .any(|event_type| &event.payload.event_type() == event_type)
                    })
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .rev()
            .filter_map(crate::event_views::title_history_record_from_domain_event)
            .filter(|record| {
                event_types.is_none_or(|values| values.contains(&record.event_type))
                    && title_ids.is_none_or(|values| values.contains(&record.title_id))
                    && download_id.is_none_or(|value| record.download_id.as_deref() == Some(value))
            })
            .count() as i64)
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let page_size = if limit == 0 { usize::MAX } else { limit };
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .rev()
            .filter(|event| {
                crate::event_views::title_history_record_from_domain_event(event).is_some_and(
                    |record| {
                        event_types.is_none_or(|values| values.contains(&record.event_type))
                            && title_ids.is_none_or(|values| values.contains(&record.title_id))
                            && download_id
                                .is_none_or(|value| record.download_id.as_deref() == Some(value))
                    },
                )
            })
            .skip(offset)
            .take(page_size)
            .cloned()
            .collect())
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn delete_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        let offsets = self.subscriber_offsets.lock().await;
        Ok(*offsets.get(subscriber).unwrap_or(&0))
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        let mut offsets = self.subscriber_offsets.lock().await;
        offsets.insert(subscriber.to_string(), sequence);
        Ok(())
    }
}

type TestSettingsKey = (String, String, Option<String>);
type TestSettingsValues = Arc<Mutex<HashMap<TestSettingsKey, String>>>;

#[derive(Default)]
struct TestSettingsRepo {
    values: TestSettingsValues,
    failing_read_key: Option<String>,
}

impl TestSettingsRepo {
    fn failing_reads_for(key_name: &str) -> Self {
        Self {
            values: Arc::new(Mutex::new(HashMap::new())),
            failing_read_key: Some(key_name.to_string()),
        }
    }

    async fn set_scoped_json(&self, scope: &str, key_name: &str, scope_id: &str, value_json: &str) {
        self.values.lock().await.insert(
            (
                scope.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ),
            value_json.to_string(),
        );
    }
}

#[async_trait]
impl SettingsRepository for TestSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        if self.failing_read_key.as_deref() == Some(key_name) {
            return Err(AppError::Repository(format!(
                "settings read deliberately failed for {key_name}"
            )));
        }
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

fn build_app(
    titles: Vec<Title>,
    collections: Vec<Collection>,
    episodes: Vec<Episode>,
    artifacts: Vec<ImportArtifact>,
) -> AppUseCase {
    build_app_with_download_client(
        titles,
        collections,
        episodes,
        artifacts,
        Arc::new(NullDownloadClient),
    )
}

fn build_app_with_download_client(
    titles: Vec<Title>,
    collections: Vec<Collection>,
    episodes: Vec<Episode>,
    artifacts: Vec<ImportArtifact>,
    download_client: Arc<dyn DownloadClient>,
) -> AppUseCase {
    build_app_with_download_client_and_configs(
        titles,
        collections,
        episodes,
        artifacts,
        download_client,
        Arc::new(NullDownloadClientConfigRepository),
    )
}

fn build_app_with_download_client_and_configs(
    titles: Vec<Title>,
    collections: Vec<Collection>,
    episodes: Vec<Episode>,
    artifacts: Vec<ImportArtifact>,
    download_client: Arc<dyn DownloadClient>,
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
) -> AppUseCase {
    build_app_with_download_client_configs_and_submissions(
        titles,
        collections,
        episodes,
        artifacts,
        download_client,
        download_client_configs,
        Arc::new(crate::null_repositories::NullDownloadSubmissionRepository),
    )
}

fn build_app_with_download_client_configs_and_submissions(
    titles: Vec<Title>,
    collections: Vec<Collection>,
    episodes: Vec<Episode>,
    artifacts: Vec<ImportArtifact>,
    download_client: Arc<dyn DownloadClient>,
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    download_submissions: Arc<dyn DownloadSubmissionRepository>,
) -> AppUseCase {
    build_app_with_download_client_configs_submissions_and_settings(
        titles,
        collections,
        episodes,
        artifacts,
        TestAppRepositories {
            download_client,
            download_client_configs,
            download_submissions,
            settings: Arc::new(crate::null_repositories::NullSettingsRepository),
        },
    )
}

struct TestAppRepositories {
    download_client: Arc<dyn DownloadClient>,
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    download_submissions: Arc<dyn DownloadSubmissionRepository>,
    settings: Arc<dyn SettingsRepository>,
}

fn build_app_with_download_client_configs_submissions_and_settings(
    titles: Vec<Title>,
    collections: Vec<Collection>,
    episodes: Vec<Episode>,
    artifacts: Vec<ImportArtifact>,
    repositories: TestAppRepositories,
) -> AppUseCase {
    let services = AppServices::builder(
        Arc::new(TestTitleRepo {
            titles: Arc::new(Mutex::new(titles)),
        }),
        Arc::new(TestShowRepo {
            collections: Arc::new(Mutex::new(collections)),
            episodes: Arc::new(Mutex::new(episodes)),
            series_movie_links: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(NullUserRepository),
        Arc::new(TestIndexerConfigRepo),
        Arc::new(NullIndexerClient),
        repositories.download_client,
        repositories.download_client_configs,
        Arc::new(NullReleaseAttemptRepository),
        repositories.settings,
        Arc::new(TestQualityProfileRepo),
        String::new(),
    )
    .with_domain_events(Arc::new(TestDomainEventRepo::default()))
    .with_import_artifacts(Arc::new(TestImportArtifactRepo {
        artifacts: Arc::new(Mutex::new(artifacts)),
    }))
    .with_download_submissions(repositories.download_submissions)
    .build_partial_for_tests();

    AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(FacetRegistry::new()),
    )
}

fn build_title(id: &str, name: &str, facet: MediaFacet) -> Title {
    Title {
        id: id.to_string(),
        name: name.to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
        facet,
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
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
    }
}

fn build_collection(id: &str, title_id: &str, season: &str) -> Collection {
    Collection {
        id: id.to_string(),
        title_id: title_id.to_string(),
        collection_type: CollectionType::Season,
        collection_index: season.to_string(),
        label: Some(format!("Season {season}")),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: None,
        last_episode_number: None,
        monitored: true,
        created_at: Utc::now(),
    }
}

fn build_episode(
    id: &str,
    title_id: &str,
    collection_id: &str,
    season_number: &str,
    episode_number: &str,
    absolute_number: Option<&str>,
) -> Episode {
    build_episode_with_details(
        id,
        title_id,
        collection_id,
        EpisodeType::Standard,
        season_number,
        episode_number,
        None,
        absolute_number,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture helper mirrors the episode fields under test"
)]
fn build_episode_with_details(
    id: &str,
    title_id: &str,
    collection_id: &str,
    episode_type: EpisodeType,
    season_number: &str,
    episode_number: &str,
    air_date: Option<&str>,
    absolute_number: Option<&str>,
) -> Episode {
    Episode {
        id: id.to_string(),
        title_id: title_id.to_string(),
        collection_id: Some(collection_id.to_string()),
        episode_type,
        episode_number: Some(episode_number.to_string()),
        season_number: Some(season_number.to_string()),
        episode_label: None,
        title: None,
        air_date: air_date.map(str::to_string),
        duration_seconds: None,
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: absolute_number.map(str::to_string),
        overview: None,
        tvdb_id: None,
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    }
}

fn build_artifact(
    source_ref: &str,
    episode_id: &str,
    normalized_file_name: &str,
) -> ImportArtifact {
    build_artifact_with_result(
        source_ref,
        Some(episode_id),
        normalized_file_name,
        "imported",
    )
}

fn build_artifact_with_result(
    source_ref: &str,
    episode_id: Option<&str>,
    normalized_file_name: &str,
    result: &str,
) -> ImportArtifact {
    ImportArtifact {
        id: Id::new().0,
        source_client_id: Some("client-1".to_string()),
        source_system: "nzbget".to_string(),
        source_ref: source_ref.to_string(),
        import_id: None,
        relative_path: None,
        normalized_file_name: normalized_file_name.to_string(),
        media_kind: "episode".to_string(),
        title_id: Some("title-1".to_string()),
        episode_id: episode_id.map(str::to_string),
        season_number: Some(1),
        episode_number: None,
        result: result.to_string(),
        reason_code: None,
        imported_media_file_id: None,
        created_at: Utc::now(),
    }
}

fn build_tracked_download(title_id: &str, facet: &str, release_title: &str) -> TrackedDownload {
    TrackedDownload {
        id: format!("nzbget:{release_title}"),
        client_id: "client-1".to_string(),
        client_type: "nzbget".to_string(),
        client_item: DownloadQueueItem {
            id: Id::new().0,
            title_id: Some(title_id.to_string()),
            episode_id: None,
            title_name: release_title.to_string(),
            facet: Some(facet.to_string()),
            category: None,
            client_id: "client-1".to_string(),
            client_name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "dl-1".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: vec![],
            tracked_match_type: None,
        },
        state: TrackedDownloadState::Downloading,
        status: TrackedDownloadStatus::Ok,
        status_messages: vec![],
        title_id: Some(title_id.to_string()),
        facet: Some(facet.to_string()),
        source_title: Some(release_title.to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: false,
    }
}

fn build_completed_download(
    name: &str,
    dest_dir: &str,
    category: Option<&str>,
) -> CompletedDownload {
    CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "client-1".to_string(),
        download_client_item_id: "dl-1".to_string(),
        download_id: None,
        name: name.to_string(),
        dest_dir: dest_dir.to_string(),
        category: category.map(str::to_string),
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    }
}

fn test_download_client_with_completed(completed: CompletedDownload) -> Arc<TestDownloadClient> {
    Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![completed])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    })
}

fn build_foreign_completed_tracked_download(
    category: Option<&str>,
    match_type: TitleMatchType,
    is_scryer_origin: bool,
) -> TrackedDownload {
    let mut td = build_tracked_download("title-1", "movie", "Paper.Lantern.2012.1080p.WEB-DL");
    td.client_item.is_scryer_origin = is_scryer_origin;
    td.client_item.category = category.map(str::to_string);
    td.match_type = match_type;
    td
}

async fn run_category_gate_check(
    settings: Arc<TestSettingsRepo>,
    completed_category: Option<&str>,
    queue_category: Option<&str>,
    match_type: TitleMatchType,
    is_scryer_origin: bool,
) -> TrackedDownload {
    if matches!(
        settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
                Some("movie".to_string()),
            )
            .await,
        Ok(None)
    ) {
        set_scoped_default_category(&settings, "movie", "movie").await;
    }

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let completed = build_completed_download(
        "Paper.Lantern.2012.1080p.WEB-DL",
        temp_dir.path().to_string_lossy().as_ref(),
        completed_category,
    );
    let title = build_title("title-1", "Paper Lantern", MediaFacet::Movie);
    let download_client = test_download_client_with_completed(completed);
    let app = build_app_with_download_client_configs_submissions_and_settings(
        vec![title],
        vec![],
        vec![],
        vec![],
        TestAppRepositories {
            download_client,
            download_client_configs: Arc::new(NullDownloadClientConfigRepository),
            download_submissions: Arc::new(
                crate::null_repositories::NullDownloadSubmissionRepository,
            ),
            settings,
        },
    );
    let mut td =
        build_foreign_completed_tracked_download(queue_category, match_type, is_scryer_origin);

    check(&app, &mut td).await;
    td
}

async fn set_scoped_routing(settings: &TestSettingsRepo, scope_id: &str, routing_json: &str) {
    settings
        .set_scoped_json(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            scope_id,
            routing_json,
        )
        .await;
}

async fn set_scoped_default_category(settings: &TestSettingsRepo, scope_id: &str, category: &str) {
    settings
        .set_scoped_json(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
            scope_id,
            &serde_json::json!(category).to_string(),
        )
        .await;
}
