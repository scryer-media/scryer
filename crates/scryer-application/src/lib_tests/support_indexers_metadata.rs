use super::*;

#[derive(Default)]
pub(super) struct MockIndexerClient;

#[async_trait]
impl IndexerClient for MockIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        if let Some(tvdb) = ids.get("tvdb_id") {
            tracing::info!(tvdb_id = %tvdb, category = ?category, "mock nzbgeek search");
        }
        if let Some(imdb) = ids.get("imdb_id") {
            tracing::info!(imdb_id = %imdb, category = ?category, "mock nzbgeek search");
        }
        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes: Vec::new(),
            results: vec![IndexerSearchResult {
                indexer_id: None,
                source: "nzbgeek".into(),
                title: format!("match for {query}"),
                link: None,
                download_url: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra: Default::default(),
                response_attributes: Default::default(),
                guid: None,
                info_url: None,
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
                coverage_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

pub(super) struct MockIndexerPluginProvider {
    pub(super) client: Arc<dyn IndexerClient>,
    pub(super) management_client: Option<Arc<dyn IndexerManagementClient>>,
}

impl IndexerPluginProvider for MockIndexerPluginProvider {
    fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        Some(Arc::clone(&self.client))
    }

    fn management_client_for_provider(
        &self,
        _config: &IndexerConfig,
    ) -> Option<Arc<dyn IndexerManagementClient>> {
        self.management_client.clone()
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec![
            "nzbgeek".to_string(),
            "prowlarr".to_string(),
            "torrent_rss".to_string(),
        ]
    }

    fn management_capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        scryer_domain::IndexerManagementCapabilities {
            supports_managed_children_sync: provider_type.eq_ignore_ascii_case("prowlarr"),
            ..Default::default()
        }
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        let protocols = match provider_type {
            "nzbgeek" => vec![scryer_domain::IndexerProtocolCapability::Usenet],
            "torrent_rss" => vec![scryer_domain::IndexerProtocolCapability::Torrent],
            _ => vec![scryer_domain::IndexerProtocolCapability::Unknown],
        };
        scryer_domain::IndexerProviderCapabilities {
            protocols,
            ..Default::default()
        }
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        vec![]
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let connection_key = match provider_type {
            "torrent_rss" => "feed_url",
            _ => "base_url",
        };
        let mut fields = vec![scryer_domain::ConfigFieldDef {
            key: connection_key.to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_domain::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role: Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
            ..Default::default()
        }];
        if provider_type != "torrent_rss" {
            fields.push(scryer_domain::ConfigFieldDef {
                key: "api_key".to_string(),
                label: "API Key".to_string(),
                field_type: scryer_domain::ConfigFieldType::Password,
                required: true,
                default_value: None,
                value_source: scryer_domain::ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
                ..Default::default()
            });
        }
        fields
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedIndexerSearch {
    pub(super) query: String,
    pub(super) season: Option<u32>,
    pub(super) episode: Option<u32>,
}

#[derive(Default, Clone)]
pub(super) struct TrackingIndexerClient {
    pub(super) searches: Arc<Mutex<Vec<RecordedIndexerSearch>>>,
    pub(super) season_pack_titles: Vec<String>,
    pub(super) title_pack_titles: Vec<String>,
    pub(super) fail_scoped_queries: bool,
    /// Answer every query with no results at all (see [`returning_no_results`]).
    pub(super) empty_results: bool,
    pub(super) report_routed_indexers_fired: bool,
    /// Overrides the default 1970 publication date, so a delay profile can
    /// actually hold the results this stand-in returns.
    pub(super) published_at: Option<String>,
    /// Stamp each result with the first routed indexer id, the way a real
    /// provider response is attributed. Off by default: most tests do not care,
    /// and coverage-scoped filters key on this field.
    pub(super) stamp_indexer_ids: bool,
}

impl TrackingIndexerClient {
    pub(super) fn with_season_pack_titles(
        mut self,
        titles: impl IntoIterator<Item = String>,
    ) -> Self {
        self.season_pack_titles = titles.into_iter().collect();
        self
    }

    pub(super) fn with_title_pack_titles(
        mut self,
        titles: impl IntoIterator<Item = String>,
    ) -> Self {
        self.title_pack_titles = titles.into_iter().collect();
        self
    }

    /// Answer every query with a genuine zero-hit response. Coverage is still
    /// recorded (the query ran), so this is the shape that leaves a scope both
    /// converged and still wanted.
    pub(super) fn returning_no_results(mut self) -> Self {
        self.empty_results = true;
        self
    }

    pub(super) fn failing_scoped_queries(mut self) -> Self {
        self.fail_scoped_queries = true;
        self
    }

    pub(super) fn reporting_routed_indexers_fired(mut self) -> Self {
        self.report_routed_indexers_fired = true;
        self
    }

    pub(super) fn with_published_at(mut self, published_at: impl Into<String>) -> Self {
        self.published_at = Some(published_at.into());
        self
    }

    pub(super) fn stamping_indexer_ids(mut self) -> Self {
        self.stamp_indexer_ids = true;
        self
    }
}

#[async_trait]
impl IndexerClient for TrackingIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        self.searches.lock().await.push(RecordedIndexerSearch {
            query: query.clone(),
            season,
            episode,
        });
        let mut routed_indexer_ids = indexer_routing
            .iter()
            .flat_map(|plan| plan.entries.iter())
            .filter(|(_, entry)| entry.enabled)
            .map(|(indexer_id, _)| indexer_id.clone())
            .collect::<Vec<_>>();
        routed_indexer_ids.sort();
        let stamped_indexer_id = self
            .stamp_indexer_ids
            .then(|| routed_indexer_ids.first().cloned())
            .flatten();
        let indexer_outcomes = if self.report_routed_indexers_fired {
            indexer_routing
                .into_iter()
                .flat_map(|plan| plan.entries)
                .filter(|(_, entry)| entry.enabled)
                .map(|(indexer_id, _)| crate::IndexerQueryOutcome {
                    indexer_id,
                    outcome: crate::IndexerSearchOutcome::Complete {
                        empty: self.empty_results,
                    },
                })
                .collect()
        } else {
            Vec::new()
        };
        if self.fail_scoped_queries && (season.is_some() || episode.is_some()) {
            return Err(AppError::Repository(
                "tracking indexer scoped-query failure".to_string(),
            ));
        }

        let release_title = match (season, episode) {
            (Some(season), Some(episode)) => {
                format!("{query}.S{season:02}E{episode:02}.1080p.WEB-DL")
            }
            (Some(season), None) => format!("{query}.S{season:02}.1080p.WEB-DL"),
            (None, _) => format!("{query}.2024.1080p.WEB-DL"),
        };
        let release_titles = if self.empty_results {
            Vec::new()
        } else if season.is_none() && episode.is_none() && !self.title_pack_titles.is_empty() {
            self.title_pack_titles.clone()
        } else if season.is_some() && episode.is_none() && !self.season_pack_titles.is_empty() {
            self.season_pack_titles.clone()
        } else {
            vec![release_title]
        };

        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes,
            results: release_titles
                .into_iter()
                .map(|release_title| {
                    let release_slug = release_title.replace([' ', '/'], ".");
                    IndexerSearchResult {
                        indexer_id: stamped_indexer_id.clone(),
                        source: "nzbgeek".into(),
                        title: release_title.clone(),
                        link: Some(format!("https://example.invalid/info/{release_slug}")),
                        download_url: Some(format!(
                            "https://example.invalid/download/{release_slug}.nzb"
                        )),
                        source_kind: Some(DownloadSourceKind::NzbUrl),
                        size_bytes: None,
                        published_at: Some(
                            self.published_at
                                .clone()
                                .unwrap_or_else(|| "1970-01-01T00:00:00Z".into()),
                        ),
                        thumbs_up: None,
                        thumbs_down: None,
                        indexer_languages: None,
                        indexer_subtitles: None,
                        indexer_grabs: None,
                        password_hint: None,
                        parsed_release_metadata: Some(crate::parse_release_metadata(
                            &release_title,
                        )),
                        quality_profile_decision: None,
                        extra: Default::default(),
                        response_attributes: Default::default(),
                        guid: Some(format!("guid-{release_slug}")),
                        info_url: Some(format!("https://example.invalid/info/{release_slug}")),
                        provenance: None,
                        auto_eligible: None,
                        auto_decision_code: None,
                        auto_decision_summary: None,
                        candidate_token: None,
                        queue_scope: None,
                        coverage_scope: None,
                    }
                })
                .collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
pub(super) struct FixedReleaseIndexerClient {
    pub(super) release_title: String,
    pub(super) indexer_languages: Option<Vec<String>>,
    /// Indexer ids this stand-in reports as having fired. Empty by default — set via [`with_fired_indexers`] when a test
    /// drives the real coverage chokepoint and needs specific indexers recorded.
    pub(super) fired_indexer_ids: Vec<String>,
    /// Enabled indexer ids requested by each search call. This observes the
    /// routing plan separately from the stand-in's configured response.
    requested_indexer_id_sets: Arc<Mutex<Vec<Vec<String>>>>,
    /// When set, every fired indexer reports `Fired { empty: true }` and the
    /// response carries no results — a genuine zero-hit response.
    pub(super) empty_response: bool,
    /// Reported seeder count, written onto `extra["seeders"]` — exactly where
    /// the indexer adapter writes it and where `seeders_from_extra` reads it.
    /// Deliberately independent of the release's source kind: the capture path
    /// under test reads the map, not the protocol.
    pub(super) seeders: Option<i64>,
    pub(super) published_at: String,
}

impl FixedReleaseIndexerClient {
    pub(super) fn new(release_title: impl Into<String>) -> Self {
        Self {
            release_title: release_title.into(),
            indexer_languages: None,
            fired_indexer_ids: Vec::new(),
            requested_indexer_id_sets: Arc::new(Mutex::new(Vec::new())),
            empty_response: false,
            seeders: None,
            published_at: "1970-01-01T00:00:00Z".to_string(),
        }
    }

    pub(super) fn with_seeders(mut self, seeders: i64) -> Self {
        self.seeders = Some(seeders);
        self
    }

    pub(super) fn with_published_at(mut self, published_at: impl Into<String>) -> Self {
        self.published_at = published_at.into();
        self
    }

    pub(super) fn with_fired_indexers(
        mut self,
        indexer_ids: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.fired_indexer_ids = indexer_ids.into_iter().map(Into::into).collect();
        self
    }

    pub(super) fn with_empty_response(mut self) -> Self {
        self.empty_response = true;
        self
    }

    pub(super) async fn requested_indexer_id_sets(&self) -> Vec<Vec<String>> {
        self.requested_indexer_id_sets.lock().await.clone()
    }
}

#[async_trait]
impl IndexerClient for FixedReleaseIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let mut requested_indexer_ids = indexer_routing
            .into_iter()
            .flat_map(|plan| plan.entries)
            .filter(|(_, entry)| entry.enabled)
            .map(|(indexer_id, _)| indexer_id)
            .collect::<Vec<_>>();
        requested_indexer_ids.sort();
        self.requested_indexer_id_sets
            .lock()
            .await
            .push(requested_indexer_ids);
        let indexer_outcomes = self
            .fired_indexer_ids
            .iter()
            .map(|id| crate::IndexerQueryOutcome {
                indexer_id: id.clone(),
                outcome: crate::IndexerSearchOutcome::Complete {
                    empty: self.empty_response,
                },
            })
            .collect();
        if self.empty_response {
            return Ok(IndexerSearchResponse {
                completion: crate::IndexerSearchCompletion::Complete,

                indexer_outcomes,
                results: Vec::new(),
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
        }
        let mut extra = std::collections::HashMap::new();
        if let Some(seeders) = self.seeders {
            extra.insert("seeders".to_string(), serde_json::json!(seeders));
        }
        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes,
            results: vec![IndexerSearchResult {
                // A real search names the indexer that served the result; the
                // blocklist attributes a failure to it.
                indexer_id: Some("indexer-a".to_string()),
                source: "nzbgeek".into(),
                title: self.release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some("https://example.invalid/download.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some(self.published_at.clone()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: self.indexer_languages.clone(),
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
                quality_profile_decision: None,
                extra,
                response_attributes: Default::default(),
                guid: Some("guid-fixed-release".to_string()),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
                coverage_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
pub(super) struct SharedUrlMovieIndexerClient {
    pub(super) download_url: String,
}

impl SharedUrlMovieIndexerClient {
    pub(super) fn new(download_url: impl Into<String>) -> Self {
        Self {
            download_url: download_url.into(),
        }
    }
}

#[async_trait]
impl IndexerClient for SharedUrlMovieIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let query = query.trim();
        let release_title = if query.contains("Deferred Movie") {
            "Deferred.Movie.2024.1080p.WEB-DL-GRP".to_string()
        } else if query.contains("Rejected Movie") {
            "Rejected.Movie.2024.1080p.WEB-DL-GRP".to_string()
        } else {
            let release_stem = query
                .split_whitespace()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(".");
            format!("{release_stem}.2024.1080p.WEB-DL-GRP")
        };

        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes: Vec::new(),
            results: vec![IndexerSearchResult {
                indexer_id: None,
                source: "nzbgeek".into(),
                title: release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some(self.download_url.clone()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                response_attributes: Default::default(),
                guid: Some(format!("guid-{release_title}")),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
                coverage_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedSearchCall {
    pub(super) query: String,
    pub(super) ids: std::collections::HashMap<String, String>,
    pub(super) category: Option<String>,
    pub(super) facet: Option<String>,
    pub(super) id_search_facet: Option<String>,
    pub(super) newznab_categories: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedStructuredQueryCall {
    pub(super) query: String,
    pub(super) season: Option<u32>,
    pub(super) episode: Option<u32>,
    pub(super) absolute_episode: Option<u32>,
}

#[derive(Clone)]
pub(super) struct RecordingCategoriesIndexerClient {
    pub(super) release_title: String,
    pub(super) calls: Arc<Mutex<Vec<RecordedSearchCall>>>,
}

impl RecordingCategoriesIndexerClient {
    pub(super) fn new(release_title: impl Into<String>) -> Self {
        Self {
            release_title: release_title.into(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingStructuredQueryIndexerClient {
    pub(super) calls: Arc<Mutex<Vec<RecordedStructuredQueryCall>>>,
}

#[async_trait]
impl IndexerClient for RecordingCategoriesIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        self.calls.lock().await.push(RecordedSearchCall {
            query,
            ids,
            category,
            facet,
            id_search_facet,
            newznab_categories,
        });

        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes: Vec::new(),
            results: vec![IndexerSearchResult {
                indexer_id: None,
                source: "nzbgeek".into(),
                title: self.release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some("https://example.invalid/download.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                response_attributes: Default::default(),
                guid: Some("guid-recording-release".to_string()),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
                coverage_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[async_trait]
impl IndexerClient for RecordingStructuredQueryIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        self.calls.lock().await.push(RecordedStructuredQueryCall {
            query,
            season,
            episode,
            absolute_episode,
        });

        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes: Vec::new(),
            results: vec![],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
pub(super) struct MultiReleaseIndexerClient {
    pub(super) release_titles: Vec<String>,
    info_hash_hint: Option<String>,
}

impl MultiReleaseIndexerClient {
    pub(super) fn new(release_titles: Vec<&str>) -> Self {
        Self {
            release_titles: release_titles.into_iter().map(str::to_string).collect(),
            info_hash_hint: None,
        }
    }

    pub(super) fn with_info_hash_hint(mut self, info_hash_hint: impl Into<String>) -> Self {
        self.info_hash_hint = Some(info_hash_hint.into());
        self
    }
}

#[async_trait]
impl IndexerClient for MultiReleaseIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes: Vec::new(),
            results: self
                .release_titles
                .iter()
                .enumerate()
                .map(|(index, release_title)| IndexerSearchResult {
                    indexer_id: None,
                    source: "nzbgeek".into(),
                    title: release_title.clone(),
                    link: Some(format!("https://example.invalid/info/{index}")),
                    download_url: Some(format!("https://example.invalid/download/{index}.nzb")),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    size_bytes: None,
                    published_at: Some("1970-01-01T00:00:00Z".into()),
                    thumbs_up: None,
                    thumbs_down: None,
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint: None,
                    parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
                    quality_profile_decision: None,
                    extra: self
                        .info_hash_hint
                        .as_ref()
                        .map(|hash| {
                            std::collections::HashMap::from([(
                                "info_hash".to_string(),
                                serde_json::Value::String(hash.clone()),
                            )])
                        })
                        .unwrap_or_default(),
                    response_attributes: Default::default(),
                    guid: Some(format!("guid-multi-release-{index}")),
                    info_url: Some(format!("https://example.invalid/info/{index}")),
                    provenance: None,
                    auto_eligible: None,
                    auto_decision_code: None,
                    auto_decision_summary: None,
                    candidate_token: None,
                    queue_scope: None,
                    coverage_scope: None,
                })
                .collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

pub(super) struct MockMetadataGateway {
    pub(super) movies: HashMap<i64, MovieMetadata>,
}

#[async_trait]
impl MetadataGateway for MockMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        self.movies
            .get(&tvdb_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let movies = movie_tvdb_ids
            .iter()
            .filter_map(|tvdb_id| {
                self.movies
                    .get(tvdb_id)
                    .cloned()
                    .map(|movie| (*tvdb_id, movie))
            })
            .collect();
        Ok(BulkMetadataResult {
            movies,
            series: HashMap::new(),
        })
    }

    async fn get_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        _language: &str,
    ) -> AppResult<MovieTitleBulkResult> {
        let mut result = MovieTitleBulkResult::default();
        for (ref_index, movie_ref) in refs.iter().enumerate() {
            let movie = self.movies.values().find(|movie| {
                movie_ref
                    .smg_id
                    .is_some_and(|smg_id| movie.smg_id == Some(smg_id))
                    || movie_ref
                        .tvdb_id
                        .is_some_and(|tvdb_id| movie.tvdb_id == Some(tvdb_id))
                    || movie_ref
                        .tmdb_id
                        .is_some_and(|tmdb_id| movie.tmdb_id == Some(tmdb_id))
                    || movie_ref
                        .imdb_id
                        .as_deref()
                        .is_some_and(|imdb_id| movie.imdb_id == imdb_id)
            });
            if let Some(movie) = movie {
                result.by_ref_index.insert(ref_index, movie.clone());
            } else {
                result.missing_ref_indexes.push(ref_index);
            }
        }
        Ok(result)
    }
}
