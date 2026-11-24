use super::*;

fn scope_row(
    id: &str,
    title_id: &str,
    episode_id: Option<&str>,
    collection_id: Option<&str>,
    series_movie_link_id: Option<&str>,
    media_type: &str,
    updated_at: &str,
) -> AcquisitionScopeState {
    AcquisitionScopeState {
        id: id.to_string(),
        title_id: title_id.to_string(),
        title_name: None,
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: episode_id.map(str::to_string),
        collection_id: collection_id.map(str::to_string),
        series_movie_link_id: series_movie_link_id.map(str::to_string),
        season_number: None,
        episode_number: None,
        media_type: media_type.to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: updated_at.to_string(),
        updated_at: updated_at.to_string(),
    }
}

/// A title with more episode rows than one list page, all ahead of its older
/// title, collection and series-movie rows. Resolving each scope key must
/// reach the existing row by its own key instead of creating a second one.
#[tokio::test]
async fn resolving_a_scope_key_finds_old_rows_behind_many_newer_episode_rows() {
    let scopes = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        Arc::new(StubDownloadClient::default()),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        Arc::new(TrackingPendingReleaseRepo::default()),
        scopes.clone(),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Lanternfold Reach".into(),
                facet: MediaFacet::Series,
                monitored: false,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("520".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Lanternfold Reach: The Movie",
            Some(2026),
            None,
            Some("lanternfold-reach"),
        ))
        .await
        .expect("create series movie link");

    {
        let mut store = scopes.store.lock().await;
        store.clear();
        // Every episode row carries its season's id, as episode scopes do.
        for n in 0..520_usize {
            store.push(scope_row(
                &format!("wanted-episode-{n:03}"),
                &title.id,
                Some(&format!("episode-{n:03}")),
                Some(&season.id),
                None,
                "episode",
                &format!("2026-03-01T00:{:02}:{:02}Z", n / 60, n % 60),
            ));
        }
        store.push(scope_row(
            "wanted-collection",
            &title.id,
            None,
            Some(&season.id),
            None,
            "episode",
            "2020-01-01T00:00:00Z",
        ));
        store.push(scope_row(
            "wanted-link",
            &title.id,
            None,
            None,
            Some(&link.id),
            "series_movie",
            "2020-01-01T00:00:00Z",
        ));
        store.push(scope_row(
            "wanted-title",
            &title.id,
            None,
            None,
            None,
            "movie",
            "2020-01-01T00:00:00Z",
        ));
    }
    let rows_before = scopes.store.lock().await.len();

    for (identifier, expected_id) in [
        (format!("collection:{}", season.id), "wanted-collection"),
        (format!("series_movie:{}", link.id), "wanted-link"),
        (format!("title:{}", title.id), "wanted-title"),
    ] {
        let resolved = app
            .resolve_or_create_wanted_state_row(&identifier)
            .await
            .expect("the scope key should resolve")
            .expect("the scope key should name a row");
        assert_eq!(resolved.id, expected_id, "{identifier}");
    }
    assert_eq!(
        scopes.store.lock().await.len(),
        rows_before,
        "no scope row was created"
    );
}
