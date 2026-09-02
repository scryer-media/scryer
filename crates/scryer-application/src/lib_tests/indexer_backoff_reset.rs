use super::*;

/// Records which indexers the search client was told to forget backoff for.
#[derive(Default)]
struct BackoffRecordingIndexerClient {
    resets: Mutex<Vec<String>>,
}

#[async_trait]
impl IndexerClient for BackoffRecordingIndexerClient {
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
            results: Vec::new(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }

    async fn reset_indexer_backoff(&self, indexer_id: &str) {
        self.resets.lock().await.push(indexer_id.to_string());
    }
}

fn credentials(api_key: &str) -> String {
    serde_json::json!({
        "base_url": "https://example.invalid",
        "api_key": api_key
    })
    .to_string()
}

/// An indexer that failed enough to be backed off stays skipped until the
/// backoff expires. Saving it with new credentials is the operator saying
/// "try again", so the validated save clears both the persisted backoff and
/// the search client's in-memory copy. A save that changes nothing about
/// reachability leaves the backoff alone.
#[tokio::test]
async fn saving_validated_indexer_changes_clears_its_backoff() {
    let mut config = synthetic_direct_nab_indexer_config("idx", "nzbgeek");
    config.config_json = Some(credentials("stale"));
    config.disabled_until = Some(Utc::now() + chrono::Duration::hours(1));
    let client = Arc::new(BackoffRecordingIndexerClient::default());
    let (app, admin) = bootstrap_with_search_settings_indexer_and_configs(
        Arc::new(StoredSettingsRepo::default()),
        client.clone(),
        vec![config],
    );

    let renamed = app
        .update_indexer_config(
            &admin,
            IndexerConfigUpdate {
                id: "idx".to_string(),
                name: Some("Renamed".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("a rename saves without validation");
    assert!(
        renamed.disabled_until.is_some(),
        "a rename is not a reason to retry a backed-off indexer"
    );
    assert!(client.resets.lock().await.is_empty());

    let saved = app
        .update_indexer_config(
            &admin,
            IndexerConfigUpdate {
                id: "idx".to_string(),
                config_json: Some(credentials("fresh")),
                ..Default::default()
            },
        )
        .await
        .expect("new credentials validate and save");
    assert_eq!(saved.disabled_until, None, "the returned config reflects the clear");
    let persisted = app
        .get_indexer_config(&admin, "idx")
        .await
        .expect("lookup")
        .expect("indexer exists");
    assert_eq!(persisted.disabled_until, None, "the persisted backoff is cleared");
    assert_eq!(
        *client.resets.lock().await,
        vec!["idx".to_string()],
        "the search client forgets its in-memory backoff for that indexer"
    );
}
