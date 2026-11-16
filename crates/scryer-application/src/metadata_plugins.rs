//! The metadata-provider plugin port.
//!
//! A metadata provider is a WASM plugin that answers for titles the metadata
//! gateway cannot, such as a fan edit SMG has no record of under its IMDb id.
//! The host asks it only after SMG came back empty, so a provider whose
//! upstream counts requests is spent on genuine gaps. The application talks to
//! the plugins through these two traits; `scryer-plugins` implements them.
//!
//! Each call answers in two layers, as list providers do: the outer
//! `AppResult` fails when the host could not complete the exchange, and the
//! inner `PluginResult` carries a failure the provider classified itself.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use scryer_plugin_sdk::{
    MetadataMediaKind, MetadataPluginHealthResponse, MetadataPluginLookupResponse,
    MetadataPluginSearchResponse, MetadataPluginTitle, PluginDescriptor, PluginResult,
};

use crate::{AppResult, RuntimePluginLoad};

/// One installed metadata provider with its installation settings bound.
#[async_trait]
pub trait MetadataProviderClient: Send + Sync {
    fn descriptor(&self) -> &PluginDescriptor;

    /// One title by its IMDb id (`tt` and digits).
    async fn lookup(
        &self,
        kind: MetadataMediaKind,
        imdb_id: &str,
    ) -> AppResult<PluginResult<MetadataPluginLookupResponse>>;

    /// Titles by name, for a provider whose `search` capability is on.
    async fn search(
        &self,
        kind: MetadataMediaKind,
        query: &str,
        year: Option<i32>,
        limit: u32,
    ) -> AppResult<PluginResult<MetadataPluginSearchResponse>>;

    async fn health(&self) -> AppResult<PluginResult<MetadataPluginHealthResponse>>;
}

/// The installed metadata providers.
pub trait MetadataPluginProvider: Send + Sync {
    /// A client per installed provider, sorted by provider type.
    fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>>;

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let _ = plugin;
        Err("this provider does not support runtime-load upsert".to_string())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let _ = provider_type;
        Err("this provider does not support runtime-load removal".to_string())
    }

    fn reload_runtime_plugins(&self, runtime_plugins: &[RuntimePluginLoad]) -> Result<(), String> {
        let _ = runtime_plugins;
        Err("this provider does not support runtime-load reload".to_string())
    }
}

/// The provider an assembly without metadata plugins uses.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullMetadataPluginProvider;

impl MetadataPluginProvider for NullMetadataPluginProvider {
    fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
        Vec::new()
    }

    fn reload_runtime_plugins(&self, _runtime_plugins: &[RuntimePluginLoad]) -> Result<(), String> {
        Ok(())
    }
}

/// A plugin's answer for one title, with the provider that gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginMetadataMatch {
    /// The provider type, such as `omdb`.
    pub provider_type: String,
    /// The provider's display name, such as `OMDb`.
    pub provider_name: String,
    pub title: MetadataPluginTitle,
}

/// How long a provider's definitive "no title under this id" is trusted before
/// the id is asked again.
const UNKNOWN_ID_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Most remembered "no title under this id" answers across all providers.
const UNKNOWN_ID_CAPACITY: usize = 4096;

/// The host's memory of which plugin calls would be wasted.
///
/// It remembers only two definitive facts: a provider answered that it has no
/// title under an id, and a provider refused with a `retry_after`. An error, a
/// timeout or a refusal without `retry_after` is never remembered, so the next
/// lookup asks again. Both facts expire; the unknown-id memo is bounded and
/// sheds the entry closest to expiry when full.
struct LookupGuard {
    now: Arc<dyn Fn() -> Instant + Send + Sync>,
    state: Mutex<LookupGuardState>,
}

#[derive(Default)]
struct LookupGuardState {
    /// `(provider type, normalized IMDb id)` to the instant the answer expires.
    unknown: HashMap<(String, String), Instant>,
    /// Provider type to the instant its `retry_after` passes.
    retry_after: HashMap<String, Instant>,
}

impl LookupGuard {
    fn new(now: Arc<dyn Fn() -> Instant + Send + Sync>) -> Self {
        Self {
            now,
            state: Mutex::new(LookupGuardState::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, LookupGuardState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether `provider` may be asked for `imdb_id` now.
    fn may_ask(&self, provider: &str, imdb_id: &str) -> bool {
        let now = (self.now)();
        let mut state = self.state();
        if let Some(until) = state.retry_after.get(provider).copied() {
            if now < until {
                return false;
            }
            state.retry_after.remove(provider);
        }
        let key = (provider.to_string(), imdb_id.to_string());
        match state.unknown.get(&key).copied() {
            Some(expires) if now < expires => false,
            Some(_) => {
                state.unknown.remove(&key);
                true
            }
            None => true,
        }
    }

    fn remember_unknown(&self, provider: &str, imdb_id: &str) {
        let now = (self.now)();
        let mut state = self.state();
        if state.unknown.len() >= UNKNOWN_ID_CAPACITY {
            state.unknown.retain(|_, expires| now < *expires);
        }
        if state.unknown.len() >= UNKNOWN_ID_CAPACITY
            && let Some(oldest) = state
                .unknown
                .iter()
                .min_by_key(|(_, expires)| **expires)
                .map(|(key, _)| key.clone())
        {
            state.unknown.remove(&oldest);
        }
        state.unknown.insert(
            (provider.to_string(), imdb_id.to_string()),
            now + UNKNOWN_ID_TTL,
        );
    }

    fn hold_until_retry_after(&self, provider: &str, retry_after_seconds: i64) {
        let Ok(seconds) = u64::try_from(retry_after_seconds) else {
            return;
        };
        if seconds == 0 {
            return;
        }
        let until = (self.now)() + Duration::from_secs(seconds);
        let mut state = self.state();
        let entry = state
            .retry_after
            .entry(provider.to_string())
            .or_insert(until);
        *entry = (*entry).max(until);
    }
}

/// Ask each provider in `clients` for `imdb_id` and return the first answer.
/// Providers that fail are logged and skipped; the scan or search that asked
/// carries on as if they had no answer. `guard` skips providers whose answer
/// is already known to be empty or that asked not to be called yet.
async fn lookup_by_imdb_id(
    clients: &[Arc<dyn MetadataProviderClient>],
    guard: &LookupGuard,
    imdb_id: &str,
) -> Option<PluginMetadataMatch> {
    let imdb_id = scryer_plugin_sdk::normalize_imdb_id(imdb_id)?;
    for client in clients {
        let descriptor = client.descriptor();
        let Some(metadata) = descriptor.metadata_provider() else {
            continue;
        };
        if !metadata.coverage.is_empty() && !metadata.coverage.contains(&MetadataMediaKind::Movie) {
            continue;
        }
        let provider_type = descriptor.provider_type();
        if !guard.may_ask(provider_type, &imdb_id) {
            continue;
        }
        match client.lookup(MetadataMediaKind::Movie, &imdb_id).await {
            Ok(PluginResult::Ok(response)) => {
                if let Some(title) = response
                    .title
                    .filter(|title| title.imdb_id.eq_ignore_ascii_case(&imdb_id))
                {
                    return Some(PluginMetadataMatch {
                        provider_type: provider_type.to_string(),
                        provider_name: metadata.display_name.clone(),
                        title,
                    });
                }
                guard.remember_unknown(provider_type, &imdb_id);
            }
            Ok(PluginResult::Err(error)) => {
                if let Some(seconds) = error.retry_after_seconds {
                    guard.hold_until_retry_after(provider_type, seconds);
                }
                tracing::warn!(
                    provider = provider_type,
                    imdb_id = imdb_id.as_str(),
                    error = ?error,
                    "metadata provider plugin refused a lookup"
                );
            }
            Err(error) => tracing::warn!(
                provider = provider_type,
                imdb_id = imdb_id.as_str(),
                error = %error,
                "metadata provider plugin lookup failed"
            ),
        }
    }
    None
}

// ── Gateway fallback ───────────────────────────────────────────────────────

use std::collections::HashMap;

use crate::library_scan::*;
use scryer_domain::ExternalId;

/// The movie kind the gateway's title surface uses.
const MOVIE_KIND: &str = "movie";

/// The metadata gateway with metadata-provider plugins behind it.
///
/// Every call goes to SMG first. Only where SMG has no answer for a movie's
/// IMDb id does the wrapper ask the installed plugins:
///
/// - an IMDb-anchored scan lookup SMG returned nothing for gets the plugin's
///   title as its one auto-match-safe result, so the scan creates the title
///   exactly as it would from SMG;
/// - a Search & Match query that is an IMDb id SMG does not know gets the
///   plugin's title added after SMG's own results;
/// - a refresh of a movie SMG has no id for and cannot resolve is answered
///   from the plugin, so such titles keep their metadata instead of failing
///   every refresh. A movie SMG identified by its own id is never answered
///   from a plugin.
///
/// Each call asks the plugins once per distinct IMDb id, however many queries
/// or refs carry it.
///
/// A provider's definitive "no title under this id" is remembered for a while
/// and a provider's `retry_after` is honoured, so a scan that meets the same
/// unknown id again does not spend another plugin call on it.
pub struct PluginFallbackMetadataGateway {
    inner: Arc<dyn MetadataGateway>,
    plugins: Arc<dyn MetadataPluginProvider>,
    guard: LookupGuard,
}

impl PluginFallbackMetadataGateway {
    pub fn new(inner: Arc<dyn MetadataGateway>, plugins: Arc<dyn MetadataPluginProvider>) -> Self {
        Self::with_clock(inner, plugins, Arc::new(Instant::now))
    }

    fn with_clock(
        inner: Arc<dyn MetadataGateway>,
        plugins: Arc<dyn MetadataPluginProvider>,
        now: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self {
            inner,
            plugins,
            guard: LookupGuard::new(now),
        }
    }

    /// Whether any metadata provider is installed. Without one, every call
    /// goes straight to the gateway.
    fn has_plugins(&self) -> bool {
        !self.plugins.clients().is_empty()
    }

    async fn plugin_title(&self, imdb_id: &str) -> Option<PluginMetadataMatch> {
        let clients = self.plugins.clients();
        lookup_by_imdb_id(&clients, &self.guard, imdb_id).await
    }

    /// The plugins' titles for each distinct normalized IMDb id among
    /// `imdb_ids`, keyed by that id. Every id is asked once, so a batch whose
    /// queries share an id spends one lookup on it.
    async fn plugin_titles<'a>(
        &self,
        imdb_ids: impl IntoIterator<Item = &'a str>,
    ) -> HashMap<String, PluginMetadataMatch> {
        let mut wanted = imdb_ids
            .into_iter()
            .filter_map(scryer_plugin_sdk::normalize_imdb_id)
            .collect::<Vec<_>>();
        wanted.sort();
        wanted.dedup();
        let mut found = HashMap::with_capacity(wanted.len());
        for imdb_id in wanted {
            if let Some(title) = self.plugin_title(&imdb_id).await {
                found.insert(imdb_id, title);
            }
        }
        found
    }

    /// Fill refs SMG left missing from the plugins, where the ref carries an
    /// IMDb id and no SMG id. A ref SMG identified by its own id but did not
    /// return stays missing: SMG's record is the authority for that title.
    async fn fill_missing_movies(
        &self,
        refs: &[MovieTitleRef],
        mut result: MovieTitleBulkResult,
    ) -> MovieTitleBulkResult {
        let missing = std::mem::take(&mut result.missing_ref_indexes);
        let plugin_imdb_id = |index: usize| {
            refs.get(index)
                .filter(|movie| movie.smg_id.is_none())
                .and_then(|movie| movie.imdb_id.as_deref())
                .and_then(scryer_plugin_sdk::normalize_imdb_id)
        };
        let ids = missing
            .iter()
            .filter_map(|index| plugin_imdb_id(*index))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            result.missing_ref_indexes = missing;
            return result;
        }
        let found = self.plugin_titles(ids.iter().map(String::as_str)).await;
        let mut still_missing = Vec::new();
        for index in missing {
            match plugin_imdb_id(index).and_then(|imdb_id| found.get(&imdb_id)) {
                Some(title) => {
                    result
                        .by_ref_index
                        .insert(index, movie_metadata_from_plugin(title));
                }
                None => still_missing.push(index),
            }
        }
        result.missing_ref_indexes = still_missing;
        result
    }
}

fn imdb_external_id(imdb_id: &str) -> ExternalId {
    ExternalId::new("imdb", imdb_id)
}

/// A scan search hit for a plugin's title: safe to auto-match, because it was
/// found by the exact IMDb id the folder or file named.
pub(crate) fn search_item_from_plugin(found: &PluginMetadataMatch) -> MetadataSearchItem {
    MetadataSearchItem {
        tvdb_id: String::new(),
        smg_id: None,
        primary_source: Some(found.provider_type.clone()),
        external_ids: vec![imdb_external_id(&found.title.imdb_id)],
        name: found.title.title.clone(),
        year: found.title.year,
        auto_match_safe: true,
        auto_match_signals: vec![format!("{}:imdb_id", found.provider_type)],
    }
}

/// A Search & Match result for a plugin's title.
pub(crate) fn rich_item_from_plugin(found: &PluginMetadataMatch) -> RichMetadataSearchItem {
    let title = &found.title;
    RichMetadataSearchItem {
        tvdb_id: String::new(),
        smg_id: None,
        primary_source: Some(found.provider_type.clone()),
        external_ids: vec![imdb_external_id(&title.imdb_id)],
        name: title.title.clone(),
        imdb_id: Some(title.imdb_id.clone()),
        slug: None,
        type_hint: Some(MOVIE_KIND.to_string()),
        year: title.year,
        status: Some("released".to_string()),
        overview: title.overview.clone(),
        popularity: None,
        poster_url: title.poster_url.clone(),
        language: title.original_language.clone(),
        runtime_minutes: title.runtime_minutes,
        sort_title: Some(title.title.clone()),
    }
}

/// Title metadata for a refresh, from a plugin's title.
pub(crate) fn movie_metadata_from_plugin(found: &PluginMetadataMatch) -> MovieMetadata {
    let title = &found.title;
    let ratings = match title.imdb_rating {
        Some(rating) => crate::TitleRatingSummary {
            rating: Some(rating),
            rating_sources: vec!["imdb".to_string()],
            external_ratings: vec![crate::TitleExternalRating {
                source: "imdb".to_string(),
                value: Some(rating),
                score: Some(rating * 10.0),
                normalized: rating / 10.0,
                votes: title.imdb_votes.and_then(|votes| i32::try_from(votes).ok()),
                url: format!("https://www.imdb.com/title/{}/", title.imdb_id),
            }],
        },
        None => crate::TitleRatingSummary::default(),
    };
    MovieMetadata {
        external_ids: vec![imdb_external_id(&title.imdb_id)],
        primary_source: found.provider_type.clone(),
        name: title.title.clone(),
        slug: String::new(),
        year: title.year,
        content_status: "released".to_string(),
        overview: title.overview.clone().unwrap_or_default(),
        poster_url: title.poster_url.clone().unwrap_or_default(),
        language: title.original_language.clone().unwrap_or_default(),
        original_language: title.original_language.clone(),
        runtime_minutes: title.runtime_minutes.unwrap_or_default(),
        sort_title: title.title.clone(),
        imdb_id: title.imdb_id.clone(),
        tmdb_release_date: title.release_date.clone(),
        ratings,
        genres: title.genres.clone(),
        ..MovieMetadata::default()
    }
}

#[async_trait]
impl MetadataGateway for PluginFallbackMetadataGateway {
    async fn canonical_tag_vocabulary(
        &self,
        known_version: Option<&str>,
    ) -> AppResult<crate::lists::vocabulary::VocabularyReply> {
        self.inner.canonical_tag_vocabulary(known_version).await
    }

    async fn search_tvdb(
        &self,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        self.inner.search_tvdb(query, type_hint, year).await
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        self.inner.search_tvdb_batch(queries, language).await
    }

    async fn search_tvdb_rich(
        &self,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        self.inner
            .search_tvdb_rich(query, type_hint, limit, language, year)
            .await
    }

    async fn search_tvdb_multi(
        &self,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.inner.search_tvdb_multi(query, limit, language).await
    }

    async fn search_titles_multi(
        &self,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.inner.search_titles_multi(query, limit, language).await
    }

    async fn get_movie(&self, tvdb_id: i64, language: &str) -> AppResult<MovieMetadata> {
        self.inner.get_movie(tvdb_id, language).await
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkMetadataResult> {
        self.inner
            .get_metadata_bulk(movie_tvdb_ids, series_tvdb_ids, language)
            .await
    }

    async fn get_series_titles(
        &self,
        refs: &[SeriesTitleRef],
        language: &str,
        include_episodes: bool,
        include_episode_orders: bool,
    ) -> AppResult<SeriesTitleBulkResult> {
        self.inner
            .get_series_titles(refs, language, include_episodes, include_episode_orders)
            .await
    }

    async fn resolve_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        self.inner.resolve_movie_titles(refs, create_missing).await
    }

    async fn resolve_titles(
        &self,
        refs: &[TitleExternalRef],
        kind: &str,
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        self.inner.resolve_titles(refs, kind, create_missing).await
    }

    async fn list_movie_targets(
        &self,
        ids: &[i64],
    ) -> AppResult<Vec<crate::lists::gateway::ListMovieTarget>> {
        self.inner.list_movie_targets(ids).await
    }

    async fn list_chart_catalog(
        &self,
        language: &str,
    ) -> AppResult<Vec<crate::lists::gateway::ListChartCatalogEntry>> {
        self.inner.list_chart_catalog(language).await
    }

    async fn list_chart_items(
        &self,
        provider: &str,
        chart_key: &str,
        scope: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<Vec<crate::lists::gateway::ListChartItem>> {
        self.inner
            .list_chart_items(provider, chart_key, scope, limit, language)
            .await
    }

    async fn title_recommendations(
        &self,
        input: &TitleRecommendationsInput,
    ) -> AppResult<DiscoveryRelatedResult> {
        self.inner.title_recommendations(input).await
    }

    async fn discover_public_feed(
        &self,
        input: &DiscoveryPublicFeedInput,
    ) -> AppResult<DiscoveryDashboardResult> {
        self.inner.discover_public_feed(input).await
    }

    async fn collection_completions(
        &self,
        input: &DiscoveryCollectionCompletionInput,
    ) -> AppResult<DiscoveryCollectionCompletionResult> {
        self.inner.collection_completions(input).await
    }

    async fn submit_discovery_context_snapshot(
        &self,
        input: &DiscoveryContextSnapshotSubmitInput,
    ) -> AppResult<DiscoveryContextSnapshotSubmitResult> {
        self.inner.submit_discovery_context_snapshot(input).await
    }

    async fn discovery_context_snapshot_status(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotStatusResult> {
        self.inner
            .discovery_context_snapshot_status(request_id)
            .await
    }

    async fn discovery_context_snapshot_page(
        &self,
        request_id: &str,
        page: i32,
    ) -> AppResult<DiscoveryContextSnapshotPageResult> {
        self.inner
            .discovery_context_snapshot_page(request_id, page)
            .await
    }

    async fn discovery_context_changes(
        &self,
        input: &DiscoveryContextChangesInput,
    ) -> AppResult<DiscoveryContextChangesResult> {
        self.inner.discovery_context_changes(input).await
    }

    async fn acknowledge_discovery_context_snapshot(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotAckResult> {
        self.inner
            .acknowledge_discovery_context_snapshot(request_id)
            .await
    }

    async fn get_artwork_urls_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkArtworkUrlResult> {
        self.inner
            .get_artwork_urls_bulk(movie_tvdb_ids, series_tvdb_ids, language)
            .await
    }

    async fn library_graph(
        &self,
        input: &LibraryGraphGatewayInput,
    ) -> AppResult<LibraryGraphGatewayResult> {
        self.inner.library_graph(input).await
    }

    async fn search_titles_batch(
        &self,
        queries: &[MetadataSearchQuery],
        kind: &str,
        language: &str,
        create_missing: bool,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        if !kind.eq_ignore_ascii_case(MOVIE_KIND) || !self.has_plugins() {
            return self
                .inner
                .search_titles_batch(queries, kind, language, create_missing)
                .await;
        }
        let mut results = self
            .inner
            .search_titles_batch(queries, kind, language, create_missing)
            .await?;
        let unanswered = queries
            .iter()
            .filter(|query| !results.get(*query).is_some_and(|items| !items.is_empty()))
            .filter_map(|query| {
                query
                    .imdb_id
                    .as_deref()
                    .and_then(scryer_plugin_sdk::normalize_imdb_id)
                    .map(|imdb_id| (query, imdb_id))
            })
            .collect::<Vec<_>>();
        if unanswered.is_empty() {
            return Ok(results);
        }
        let found = self
            .plugin_titles(unanswered.iter().map(|(_, imdb_id)| imdb_id.as_str()))
            .await;
        for (query, imdb_id) in unanswered {
            if let Some(title) = found.get(&imdb_id) {
                results.insert(query.clone(), vec![search_item_from_plugin(title)]);
            }
        }
        Ok(results)
    }

    async fn search_titles(
        &self,
        query: &str,
        kind: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        let imdb_id = scryer_plugin_sdk::normalize_imdb_id(query)
            .filter(|_| kind.eq_ignore_ascii_case(MOVIE_KIND) && self.has_plugins());
        let results = self
            .inner
            .search_titles(query, kind, limit, language, year)
            .await?;
        let Some(imdb_id) = imdb_id else {
            return Ok(results);
        };
        // SMG knows the id: its answer stands.
        if results.iter().any(|item| {
            item.imdb_id
                .as_deref()
                .and_then(scryer_plugin_sdk::normalize_imdb_id)
                .as_deref()
                == Some(imdb_id.as_str())
        }) {
            return Ok(results);
        }
        // SMG has no title under the id: add the plugin's after SMG's own
        // results, which stay available to choose from.
        let mut results = results;
        if let Some(found) = self.plugin_title(&imdb_id).await {
            results.push(rich_item_from_plugin(&found));
        }
        Ok(results)
    }

    async fn get_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        language: &str,
    ) -> AppResult<MovieTitleBulkResult> {
        if !self.has_plugins() {
            return self.inner.get_movie_titles(refs, language).await;
        }
        let result = self.inner.get_movie_titles(refs, language).await?;
        Ok(self.fill_missing_movies(refs, result).await)
    }

    async fn get_movie_titles_for_release_market(
        &self,
        refs: &[MovieTitleRef],
        language: &str,
        release_market: &str,
    ) -> AppResult<MovieTitleBulkResult> {
        if !self.has_plugins() {
            return self
                .inner
                .get_movie_titles_for_release_market(refs, language, release_market)
                .await;
        }
        let result = self
            .inner
            .get_movie_titles_for_release_market(refs, language, release_market)
            .await?;
        Ok(self.fill_missing_movies(refs, result).await)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use scryer_plugin_sdk::{
        MetadataProviderCapabilities, MetadataProviderDescriptor, ProviderDescriptor,
    };

    use super::*;
    use crate::MovieMetadata;

    const KNOWN: &str = "tt0000042";
    const FAN_EDIT: &str = "tt90000001";
    /// An id the plugin answers with a host-level failure.
    const BROKEN: &str = "tt90000002";
    /// An id the plugin refuses with a `retry_after`.
    const THROTTLED: &str = "tt90000003";
    const RETRY_AFTER_SECONDS: i64 = 60;

    fn plugin_title(imdb_id: &str) -> MetadataPluginTitle {
        MetadataPluginTitle {
            imdb_id: imdb_id.to_string(),
            title: "The Fan Edit".to_string(),
            year: Some(2021),
            overview: Some("Recut.".to_string()),
            runtime_minutes: Some(201),
            imdb_rating: Some(8.2),
            imdb_votes: Some(1234),
            ..MetadataPluginTitle::default()
        }
    }

    /// A plugin that knows `FAN_EDIT` and counts every lookup.
    struct FakeClient {
        descriptor: PluginDescriptor,
        lookups: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl MetadataProviderClient for FakeClient {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.descriptor
        }

        async fn lookup(
            &self,
            _kind: MetadataMediaKind,
            imdb_id: &str,
        ) -> AppResult<PluginResult<MetadataPluginLookupResponse>> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            match imdb_id {
                BROKEN => Err(crate::AppError::Repository(
                    "fixture plugin timed out".to_string(),
                )),
                THROTTLED => Ok(PluginResult::Err(scryer_plugin_sdk::PluginError {
                    code: scryer_plugin_sdk::PluginErrorCode::RateLimited,
                    public_message: "fixture plugin is out of requests".to_string(),
                    debug_message: None,
                    retry_after_seconds: Some(RETRY_AFTER_SECONDS),
                    details: None,
                })),
                _ => Ok(PluginResult::Ok(MetadataPluginLookupResponse {
                    title: (imdb_id == FAN_EDIT).then(|| plugin_title(imdb_id)),
                })),
            }
        }

        async fn search(
            &self,
            _kind: MetadataMediaKind,
            _query: &str,
            _year: Option<i32>,
            _limit: u32,
        ) -> AppResult<PluginResult<MetadataPluginSearchResponse>> {
            panic!("title search is not used")
        }

        async fn health(&self) -> AppResult<PluginResult<MetadataPluginHealthResponse>> {
            panic!("health is not used")
        }
    }

    struct FakeProvider {
        lookups: Arc<AtomicUsize>,
    }

    impl MetadataPluginProvider for FakeProvider {
        fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
            vec![Arc::new(FakeClient {
                descriptor: PluginDescriptor {
                    id: "fixture-metadata".to_string(),
                    name: "Fixture Metadata".to_string(),
                    version: "0.1.0".to_string(),
                    sdk_version: "0.1.0".to_string(),
                    sdk_constraint: String::new(),
                    socket_permissions: Vec::new(),
                    settings: Vec::new(),
                    provider: ProviderDescriptor::MetadataProvider(MetadataProviderDescriptor {
                        provider_type: "fixture-metadata".to_string(),
                        provider_aliases: Vec::new(),
                        display_name: "Fixture Metadata".to_string(),
                        summary: None,
                        coverage: vec![MetadataMediaKind::Movie],
                        capabilities: MetadataProviderCapabilities::default(),
                        config_fields: Vec::new(),
                        default_base_url: None,
                        allowed_hosts: Vec::new(),
                    }),
                },
                lookups: self.lookups.clone(),
            })]
        }
    }

    /// SMG that knows only `KNOWN`.
    #[derive(Default)]
    struct FakeSmg {
        title_searches: Mutex<Vec<String>>,
    }

    fn smg_item(imdb_id: &str) -> MetadataSearchItem {
        MetadataSearchItem {
            tvdb_id: "1".to_string(),
            smg_id: Some(1),
            primary_source: Some("tmdb".to_string()),
            external_ids: vec![imdb_external_id(imdb_id)],
            name: "Known".to_string(),
            year: Some(1994),
            auto_match_safe: true,
            auto_match_signals: Vec::new(),
        }
    }

    fn smg_rich(imdb_id: &str, name: &str) -> RichMetadataSearchItem {
        RichMetadataSearchItem {
            tvdb_id: "1".to_string(),
            smg_id: Some(1),
            primary_source: Some("tmdb".to_string()),
            external_ids: Vec::new(),
            name: name.to_string(),
            imdb_id: Some(imdb_id.to_string()),
            slug: None,
            type_hint: Some(MOVIE_KIND.to_string()),
            year: None,
            status: None,
            overview: None,
            popularity: None,
            poster_url: None,
            language: None,
            runtime_minutes: None,
            sort_title: None,
        }
    }

    #[async_trait]
    impl MetadataGateway for FakeSmg {
        async fn search_tvdb(
            &self,
            _query: &str,
            _type_hint: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<MetadataSearchItem>> {
            unreachable!()
        }

        async fn search_tvdb_batch(
            &self,
            _queries: &[MetadataSearchQuery],
            _language: &str,
        ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
            unreachable!()
        }

        async fn search_tvdb_rich(
            &self,
            _query: &str,
            _type_hint: &str,
            _limit: i32,
            _language: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<RichMetadataSearchItem>> {
            unreachable!()
        }

        async fn search_tvdb_multi(
            &self,
            _query: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<MultiMetadataSearchResult> {
            unreachable!()
        }

        async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
            unreachable!()
        }

        async fn get_metadata_bulk(
            &self,
            _movie_tvdb_ids: &[i64],
            _series_tvdb_ids: &[i64],
            _language: &str,
        ) -> AppResult<BulkMetadataResult> {
            unreachable!()
        }

        async fn search_titles_batch(
            &self,
            queries: &[MetadataSearchQuery],
            _kind: &str,
            _language: &str,
            _create_missing: bool,
        ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
            Ok(queries
                .iter()
                .map(|query| {
                    let items = match query.imdb_id.as_deref() {
                        Some(KNOWN) => vec![smg_item(KNOWN)],
                        _ => Vec::new(),
                    };
                    (query.clone(), items)
                })
                .collect())
        }

        async fn search_titles(
            &self,
            query: &str,
            _kind: &str,
            _limit: i32,
            _language: &str,
            _year: Option<i32>,
        ) -> AppResult<Vec<RichMetadataSearchItem>> {
            self.title_searches.lock().unwrap().push(query.to_string());
            Ok(match query {
                KNOWN => vec![smg_rich(KNOWN, "Known")],
                // SMG's fuzzy search returns noise for an id it doesn't know.
                _ => vec![smg_rich("tt0000001", "Noise")],
            })
        }

        async fn library_graph(
            &self,
            _input: &LibraryGraphGatewayInput,
        ) -> AppResult<LibraryGraphGatewayResult> {
            Ok(LibraryGraphGatewayResult {
                generated_at: "smg".to_string(),
                ..LibraryGraphGatewayResult::default()
            })
        }

        async fn get_movie_titles(
            &self,
            refs: &[MovieTitleRef],
            _language: &str,
        ) -> AppResult<MovieTitleBulkResult> {
            let mut result = MovieTitleBulkResult::default();
            for (index, movie) in refs.iter().enumerate() {
                if movie.imdb_id.as_deref() == Some(KNOWN) {
                    result.by_ref_index.insert(
                        index,
                        MovieMetadata {
                            name: "Known".to_string(),
                            ..MovieMetadata::default()
                        },
                    );
                } else {
                    result.missing_ref_indexes.push(index);
                }
            }
            Ok(result)
        }
    }

    fn gateway() -> (PluginFallbackMetadataGateway, Arc<AtomicUsize>) {
        let lookups = Arc::new(AtomicUsize::new(0));
        let gateway = PluginFallbackMetadataGateway::new(
            Arc::new(FakeSmg::default()),
            Arc::new(FakeProvider {
                lookups: lookups.clone(),
            }),
        );
        (gateway, lookups)
    }

    /// A gateway whose clock only moves when the test moves it.
    fn gateway_with_clock() -> (
        PluginFallbackMetadataGateway,
        Arc<AtomicUsize>,
        Arc<Mutex<Instant>>,
    ) {
        let lookups = Arc::new(AtomicUsize::new(0));
        let clock = Arc::new(Mutex::new(Instant::now()));
        let now = {
            let clock = clock.clone();
            Arc::new(move || *clock.lock().unwrap()) as Arc<dyn Fn() -> Instant + Send + Sync>
        };
        let gateway = PluginFallbackMetadataGateway::with_clock(
            Arc::new(FakeSmg::default()),
            Arc::new(FakeProvider {
                lookups: lookups.clone(),
            }),
            now,
        );
        (gateway, lookups, clock)
    }

    fn advance(clock: &Mutex<Instant>, by: Duration) {
        let mut now = clock.lock().unwrap();
        *now += by;
    }

    async fn scan_lookup(gateway: &PluginFallbackMetadataGateway, imdb_id: &str) -> bool {
        let query = query(Some(imdb_id));
        let results = gateway
            .search_titles_batch(std::slice::from_ref(&query), MOVIE_KIND, "eng", true)
            .await
            .unwrap();
        results.get(&query).is_some_and(|items| !items.is_empty())
    }

    #[tokio::test]
    async fn an_unknown_id_is_not_asked_again_until_its_answer_expires() {
        let (gateway, lookups, clock) = gateway_with_clock();
        let unknown = "tt90000009";
        assert!(!scan_lookup(&gateway, unknown).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 1);

        // Another scan, a by-ref refresh and a Search & Match query for the
        // same id inside the window make no plugin call.
        assert!(!scan_lookup(&gateway, unknown).await);
        gateway
            .get_movie_titles(
                &[MovieTitleRef {
                    imdb_id: Some(unknown.to_string()),
                    ..MovieTitleRef::default()
                }],
                "eng",
            )
            .await
            .unwrap();
        gateway
            .search_titles(unknown, MOVIE_KIND, 20, "eng", None)
            .await
            .unwrap();
        assert_eq!(lookups.load(Ordering::SeqCst), 1);

        // A known id is still asked.
        assert!(scan_lookup(&gateway, FAN_EDIT).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 2);

        advance(&clock, UNKNOWN_ID_TTL);
        assert!(!scan_lookup(&gateway, unknown).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_failed_lookup_is_not_remembered() {
        let (gateway, lookups, _clock) = gateway_with_clock();
        assert!(!scan_lookup(&gateway, BROKEN).await);
        assert!(!scan_lookup(&gateway, BROKEN).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refusal_with_retry_after_holds_the_plugin_until_it_passes() {
        let (gateway, lookups, clock) = gateway_with_clock();
        assert!(!scan_lookup(&gateway, THROTTLED).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 1);

        // The hold covers the provider, not just the id it refused.
        assert!(!scan_lookup(&gateway, THROTTLED).await);
        assert!(!scan_lookup(&gateway, FAN_EDIT).await);
        advance(
            &clock,
            Duration::from_secs(RETRY_AFTER_SECONDS as u64) - Duration::from_millis(1),
        );
        assert!(!scan_lookup(&gateway, FAN_EDIT).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 1);

        advance(&clock, Duration::from_millis(1));
        assert!(scan_lookup(&gateway, FAN_EDIT).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 2);
        // A refusal is not an answer: the refused id is asked again too.
        assert!(!scan_lookup(&gateway, THROTTLED).await);
        assert_eq!(lookups.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn the_unknown_id_memo_stays_bounded() {
        let clock = Arc::new(Mutex::new(Instant::now()));
        let guard = LookupGuard::new({
            let clock = clock.clone();
            Arc::new(move || *clock.lock().unwrap())
        });
        for index in 0..UNKNOWN_ID_CAPACITY + 10 {
            guard.remember_unknown("fixture-metadata", &format!("tt{index:08}"));
            advance(&clock, Duration::from_millis(1));
        }
        assert_eq!(guard.state().unknown.len(), UNKNOWN_ID_CAPACITY);
        // The entries closest to expiry were shed first.
        assert!(guard.may_ask("fixture-metadata", "tt00000000"));
        assert!(!guard.may_ask(
            "fixture-metadata",
            &format!("tt{:08}", UNKNOWN_ID_CAPACITY + 9)
        ));
    }

    /// A provider set with no metadata plugin installed.
    struct EmptyProvider;

    impl MetadataPluginProvider for EmptyProvider {
        fn clients(&self) -> Vec<Arc<dyn MetadataProviderClient>> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn without_plugins_every_call_is_the_gateway_s_answer() {
        let gateway = PluginFallbackMetadataGateway::new(
            Arc::new(FakeSmg::default()),
            Arc::new(EmptyProvider),
        );
        let fan_edit = query(Some(FAN_EDIT));
        let results = gateway
            .search_titles_batch(std::slice::from_ref(&fan_edit), MOVIE_KIND, "eng", true)
            .await
            .unwrap();
        assert!(results[&fan_edit].is_empty());
        let rich = gateway
            .search_titles(FAN_EDIT, MOVIE_KIND, 20, "eng", None)
            .await
            .unwrap();
        assert_eq!(rich.len(), 1);
        assert_eq!(rich[0].name, "Noise");
        let refs = [MovieTitleRef {
            imdb_id: Some(FAN_EDIT.to_string()),
            ..MovieTitleRef::default()
        }];
        let result = gateway.get_movie_titles(&refs, "eng").await.unwrap();
        assert_eq!(result.missing_ref_indexes, vec![0]);
    }

    fn query(imdb_id: Option<&str>) -> MetadataSearchQuery {
        MetadataSearchQuery {
            query: "Some Film".to_string(),
            type_hint: MOVIE_KIND.to_string(),
            year: None,
            imdb_id: imdb_id.map(str::to_string),
            tmdb_id: None,
            tvdb_id: None,
        }
    }

    #[tokio::test]
    async fn scan_lookup_falls_back_to_the_plugin_only_where_smg_is_empty() {
        let (gateway, lookups) = gateway();
        let known = query(Some(KNOWN));
        let fan_edit = query(Some(FAN_EDIT));
        let unanchored = query(None);
        let results = gateway
            .search_titles_batch(
                &[known.clone(), fan_edit.clone(), unanchored.clone()],
                MOVIE_KIND,
                "eng",
                true,
            )
            .await
            .unwrap();

        assert_eq!(results[&known][0].smg_id, Some(1));
        let hit = &results[&fan_edit];
        assert_eq!(hit.len(), 1);
        assert!(hit[0].auto_match_safe);
        assert_eq!(hit[0].primary_source.as_deref(), Some("fixture-metadata"));
        assert_eq!(hit[0].external_ids, vec![imdb_external_id(FAN_EDIT)]);
        assert!(results[&unanchored].is_empty());
        // Only the IMDb-anchored query SMG missed reached the plugin.
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn series_scans_never_reach_the_plugin() {
        let (gateway, lookups) = gateway();
        gateway
            .search_titles_batch(&[query(Some(FAN_EDIT))], "series", "eng", true)
            .await
            .unwrap();
        assert_eq!(lookups.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_imdb_id_search_smg_does_not_know_adds_the_plugin_title_after_smg() {
        let (gateway, lookups) = gateway();
        let results = gateway
            .search_titles(FAN_EDIT, MOVIE_KIND, 20, "eng", None)
            .await
            .unwrap();
        // SMG's own results stay, so one can still be chosen and applied.
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "Noise");
        assert_eq!(results[1].imdb_id.as_deref(), Some(FAN_EDIT));
        assert_eq!(results[1].name, "The Fan Edit");
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_library_graph_is_smg_s_answer_passed_through() {
        let (gateway, lookups) = gateway();
        let graph = gateway
            .library_graph(&LibraryGraphGatewayInput {
                subjects: Vec::new(),
                axis: LibraryGraphAxis::Semantic,
                neighbors_per_node: 5,
                min_cosine: 0.5,
            })
            .await
            .unwrap();
        assert_eq!(graph.generated_at, "smg");
        assert_eq!(lookups.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn queries_sharing_an_imdb_id_spend_one_lookup() {
        let (gateway, lookups) = gateway();
        // One file yields a query per search candidate and year variant, all
        // anchored on the same id; the id is written in two spellings too.
        let queries = (0..6)
            .map(|variant| MetadataSearchQuery {
                query: format!("Candidate {variant}"),
                year: (variant % 2 == 0).then_some(2021),
                imdb_id: Some(if variant == 5 {
                    FAN_EDIT.to_uppercase()
                } else {
                    FAN_EDIT.to_string()
                }),
                ..query(None)
            })
            .collect::<Vec<_>>();
        let results = gateway
            .search_titles_batch(&queries, MOVIE_KIND, "eng", true)
            .await
            .unwrap();
        for query in &queries {
            assert_eq!(results[query][0].name, "The Fan Edit");
        }
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn searches_smg_answers_never_reach_the_plugin() {
        let (gateway, lookups) = gateway();
        let known = gateway
            .search_titles(KNOWN, MOVIE_KIND, 20, "eng", None)
            .await
            .unwrap();
        assert_eq!(known[0].name, "Known");
        let by_name = gateway
            .search_titles("The Fan Edit", MOVIE_KIND, 20, "eng", None)
            .await
            .unwrap();
        assert_eq!(by_name[0].name, "Noise");
        assert_eq!(lookups.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refresh_fills_only_the_movies_smg_could_not_resolve() {
        let (gateway, lookups) = gateway();
        let refs = [
            MovieTitleRef {
                imdb_id: Some(KNOWN.to_string()),
                ..MovieTitleRef::default()
            },
            MovieTitleRef {
                imdb_id: Some(FAN_EDIT.to_string()),
                ..MovieTitleRef::default()
            },
            MovieTitleRef {
                tmdb_id: Some(42),
                ..MovieTitleRef::default()
            },
        ];
        let result = gateway.get_movie_titles(&refs, "eng").await.unwrap();

        assert_eq!(result.by_ref_index[&0].name, "Known");
        let filled = &result.by_ref_index[&1];
        assert_eq!(filled.name, "The Fan Edit");
        assert_eq!(filled.primary_source, "fixture-metadata");
        assert_eq!(filled.imdb_id, FAN_EDIT);
        assert_eq!(filled.runtime_minutes, 201);
        assert_eq!(filled.ratings.external_ratings[0].score, Some(82.0));
        assert_eq!(result.missing_ref_indexes, vec![2]);
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_never_answers_a_title_smg_identified_from_a_plugin() {
        let (gateway, lookups) = gateway();
        let refs = [
            // SMG knows this title by its own id but did not return it.
            MovieTitleRef {
                smg_id: Some(7),
                imdb_id: Some(FAN_EDIT.to_string()),
                ..MovieTitleRef::default()
            },
            MovieTitleRef {
                imdb_id: Some(FAN_EDIT.to_string()),
                ..MovieTitleRef::default()
            },
            MovieTitleRef {
                imdb_id: Some(FAN_EDIT.to_string()),
                ..MovieTitleRef::default()
            },
        ];
        let result = gateway.get_movie_titles(&refs, "eng").await.unwrap();

        assert_eq!(result.missing_ref_indexes, vec![0]);
        assert_eq!(result.by_ref_index[&1].name, "The Fan Edit");
        assert_eq!(result.by_ref_index[&2].name, "The Fan Edit");
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
    }
}
