use super::*;

#[tokio::test]
async fn completing_a_scope_clears_the_grab_only_when_a_file_landed() {
    let db = std::env::temp_dir().join(format!(
        "scryer_complete_wanted_item_for_title_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now().to_rfc3339();

    let title = make_test_title("title-series", None);
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, status,
          grabbed_release, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-episode")
    .bind("title-series")
    .bind("movie")
    .bind("wanted")
    .bind("Existing Release")
    .bind(&now)
    .bind(&now)
    .execute(services.pool())
    .await
    .expect("wanted item should insert");

    // A passive completion — a scan noticing a file, or a manual close — leaves
    // any in-flight grab alone: nothing landed through it.
    let completed = workflow
        .complete_acquisition_scope_for_title(
            "title-series",
            None,
            Some("2026-04-20T00:00:00Z"),
            false,
        )
        .await
        .expect("completion should succeed");
    assert!(completed);

    let row = sqlx::query(
        "SELECT status, last_search_at, grabbed_release
         FROM wanted_items
         WHERE id = ?",
    )
    .bind("wanted-episode")
    .fetch_one(services.pool())
    .await
    .expect("wanted item should load");

    assert_eq!(row.get::<String, _>("status"), "completed");
    assert_eq!(
        row.get::<Option<String>, _>("last_search_at"),
        Some("2026-04-20T00:00:00Z".to_string())
    );
    assert_eq!(
        row.get::<Option<String>, _>("grabbed_release"),
        Some("Existing Release".to_string()),
        "a passive completion must not erase an in-flight grab"
    );

    // A landed import clears it. That cleared grab is the signal the search and
    // convergence paths read, now that no score is stored on the row.
    sqlx::query("UPDATE wanted_items SET status = ?, grabbed_release = ? WHERE id = ?")
        .bind("wanted")
        .bind("Stale Grabbed Release")
        .bind("wanted-episode")
        .execute(services.pool())
        .await
        .expect("wanted item should reset");

    workflow
        .complete_acquisition_scope_for_title(
            "title-series",
            None,
            Some("2026-04-20T01:00:00Z"),
            true,
        )
        .await
        .expect("landed completion should succeed");

    let row = sqlx::query("SELECT grabbed_release FROM wanted_items WHERE id = ?")
        .bind("wanted-episode")
        .fetch_one(services.pool())
        .await
        .expect("wanted item should load after landed completion");

    assert_eq!(
        row.get::<Option<String>, _>("grabbed_release"),
        None,
        "a landed import clears the in-flight grab"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_wanted_items_filters_on_latest_decision_code() {
    let (services, db) = temp_services("scryer_wanted_latest_decision").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();

    let title = make_test_title("title-latest-decision", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let other_title = make_test_title("title-latest-decision-other", None);
    TitleRepository::create(&catalog, other_title.clone())
        .await
        .expect("other title should insert");

    let wanted_mismatch = AcquisitionScopeState {
        id: "wanted-mismatch".to_string(),
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let wanted_quality_blocked = AcquisitionScopeState {
        id: "wanted-quality-blocked".to_string(),
        title_id: other_title.id.clone(),
        title_name: Some(other_title.name.clone()),
        ..wanted_mismatch.clone()
    };

    workflow
        .upsert_acquisition_scope_state(&wanted_mismatch)
        .await
        .expect("first wanted item should insert");
    workflow
        .upsert_acquisition_scope_state(&wanted_quality_blocked)
        .await
        .expect("second wanted item should insert");

    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-1".to_string(),
            wanted_item_id: wanted_mismatch.id.clone(),
            title_id: title.id.clone(),
            release_title: "Mismatch Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("mismatch decision should insert");
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-2".to_string(),
            wanted_item_id: wanted_quality_blocked.id.clone(),
            title_id: other_title.id.clone(),
            release_title: "Old Mismatch Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: (now - chrono::Duration::minutes(2)).to_rfc3339(),
        })
        .await
        .expect("older mismatch decision should insert");
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-3".to_string(),
            wanted_item_id: wanted_quality_blocked.id.clone(),
            title_id: other_title.id.clone(),
            release_title: "New Blocked Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "quality_blocked".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("latest blocked decision should insert");

    let items = workflow
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            latest_decision_codes: vec!["title_mismatch".into()],
            limit: 50,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("filtered wanted items should load");
    let count = workflow
        .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
            latest_decision_codes: vec!["title_mismatch".into()],
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("filtered wanted count should load");

    assert_eq!(items.len(), 1);
    assert_eq!(count, 1);
    assert_eq!(items[0].id, wanted_mismatch.id);
    assert!(items[0].mismatch_recovery_eligible);
    let latest_decision = items[0]
        .latest_release_decision
        .as_ref()
        .expect("latest decision should be hydrated");
    assert_eq!(latest_decision.decision_code, "title_mismatch");
    assert_eq!(latest_decision.release_title, "Mismatch Release");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn release_decision_explanations_are_compressed_and_hydrated_across_read_paths() {
    let (services, db) = temp_services("scryer_release_decision_explanation").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();
    let title = make_test_title("title-explanation", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let wanted = AcquisitionScopeState {
        id: "wanted-explanation".to_string(),
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    workflow
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("wanted item should insert");

    let explanation = serde_json::json!({
        "candidate": {
            "source": "synthetic-indexer",
            "source_kind": "torrent",
            "guid": "synthetic-guid",
            "download_url_present": true,
            "link_present": true,
            "external_id_conflicts": null,
        },
        "auto_decision": {
            "eligible": false,
            "code": "episode_mismatch",
            "summary": "Synthetic episode mismatch decision",
        },
        "quality_profile_decision": {
            "allowed": true,
            "block_codes": [],
            "release_score": 1050,
            "preference_score": 50,
            "scoring_log": [
                {"code": "quality_tier", "delta": 1000},
                {"code": "preferred_protocol", "delta": 50},
            ],
        },
        "parsed": {
            "raw_title": "Synthetic.Show.S03E07.WEBDL-1080p.x265-EXAMPLE",
            "normalized_title": "synthetic show",
            "normalized_title_variants": ["synthetic show"],
            "year": null,
            "quality": "WEBDL-1080p",
            "source": "Web",
            "release_group": "EXAMPLE",
            "disposition": "Parsed",
            "parse_family": "Episode",
            "parse_confidence": 0.98,
            "is_ambiguous": false,
            "parse_hints": ["synthetic exact identity"],
        },
    });
    let explanation_json = explanation.to_string();
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-explanation".to_string(),
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Synthetic Show S03E07".to_string(),
            release_url: Some("https://example.invalid/release".to_string()),
            release_size_bytes: Some(1_000_000),
            decision_code: "episode_mismatch".to_string(),
            candidate_score: 1050,
            current_score: Some(1000),
            score_delta: Some(50),
            explanation_json: Some(explanation_json.clone()),
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("decision should insert");

    let stored = sqlx::query(
        "SELECT typeof(explanation_json) AS storage_type, explanation_json
           FROM release_decisions
          WHERE id = ?",
    )
    .bind("decision-explanation")
    .fetch_one(services.pool())
    .await
    .expect("stored explanation should load");
    assert_eq!(stored.get::<String, _>("storage_type"), "blob");
    let encoded = stored.get::<Vec<u8>, _>("explanation_json");
    assert_eq!(encoded.first(), Some(&1));
    assert!(encoded.len() < explanation_json.len());

    let decisions = workflow
        .list_release_decisions_for_title(&title.id, 10, 0)
        .await
        .expect("decisions should list");
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            decisions[0]
                .explanation_json
                .as_deref()
                .expect("listed explanation should decode")
        )
        .expect("listed explanation should remain JSON"),
        explanation
    );

    let wanted_items = workflow
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_id: Some(title.id.clone()),
            limit: 10,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("wanted item should list");
    let latest = wanted_items[0]
        .latest_release_decision
        .as_ref()
        .expect("latest decision should hydrate");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            latest
                .explanation_json
                .as_deref()
                .expect("latest explanation should decode")
        )
        .expect("latest explanation should remain JSON"),
        explanation
    );

    sqlx::query("UPDATE release_decisions SET explanation_json = x'0200' WHERE id = ?")
        .bind("decision-explanation")
        .execute(services.pool())
        .await
        .expect("explanation should corrupt for the read-path test");
    let corrupt = workflow
        .list_release_decisions_for_title(&title.id, 10, 0)
        .await
        .expect("corrupt explanation must not fail the decision list");
    assert_eq!(corrupt.len(), 1);
    assert_eq!(corrupt[0].explanation_json, None);
    let corrupt_wanted_items = workflow
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_id: Some(title.id.clone()),
            limit: 10,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("corrupt explanation must not fail the latest-decision projection");
    let corrupt_latest = corrupt_wanted_items[0]
        .latest_release_decision
        .as_ref()
        .expect("corruption must preserve the latest decision");
    assert_eq!(corrupt_latest.id, "decision-explanation");
    assert_eq!(corrupt_latest.explanation_json, None);

    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-invalid-explanation".to_string(),
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Invalid Explanation".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "episode_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: Some("not-json".to_string()),
            created_at: (now + chrono::Duration::seconds(1)).to_rfc3339(),
        })
        .await
        .expect("invalid explanation should not suppress its decision row");
    let invalid_stored = sqlx::query("SELECT explanation_json FROM release_decisions WHERE id = ?")
        .bind("decision-invalid-explanation")
        .fetch_one(services.pool())
        .await
        .expect("invalid-explanation decision should remain stored");
    assert_eq!(
        invalid_stored.get::<Option<Vec<u8>>, _>("explanation_json"),
        None
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_matches_aliases_slug_and_typos_with_direct_priority() {
    let (services, db) = temp_services("scryer_catalog_title_search").await;
    let catalog = title_store(&services);

    let mut direct_title = make_test_title("title-search-direct", None);
    direct_title.name = "Lanternhouse Rock! Earth".to_string();
    direct_title.slug = Some("lanternhouse-rock-earth".to_string());
    direct_title.aliases = vec!["Lantern House Rock".to_string()];
    direct_title.tagged_aliases = vec![TaggedAlias {
        name: "Lanternhouse Planet Earth".to_string(),
        language: "eng".to_string(),
    }];
    TitleRepository::create(&catalog, direct_title.clone())
        .await
        .expect("direct title should insert");

    let mut typo_title = make_test_title("title-search-typo", None);
    typo_title.name = "Lanternhouze Rock Earth".to_string();
    TitleRepository::create(&catalog, typo_title.clone())
        .await
        .expect("typo title should insert");

    let alias_hits = TitleRepository::list(&catalog, None, Some("lantern house rock".to_string()))
        .await
        .expect("alias search should load");
    assert_eq!(
        alias_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );

    let slug_hits =
        TitleRepository::list(&catalog, None, Some("lanternhouse rock earth".to_string()))
            .await
            .expect("slug search should load");
    assert_eq!(
        slug_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );

    let typo_hits =
        TitleRepository::list(&catalog, None, Some("lanterhouse rock earth".to_string()))
            .await
            .expect("typo search should load");
    assert_eq!(
        typo_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );
    assert!(typo_hits.iter().any(|title| title.id == typo_title.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_short_typo_does_not_return_loose_spellfix_neighbors() {
    let (services, db) = temp_services("scryer_catalog_title_search_short_typo").await;
    let catalog = title_store(&services);

    let mut aokumo = make_test_title("title-search-aokumo", None);
    aokumo.name = "Aokumo".to_string();
    aokumo.facet = MediaFacet::Anime;
    aokumo.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    TitleRepository::create(&catalog, aokumo.clone())
        .await
        .expect("close typo target should insert");

    let mut nagami = make_test_title("title-search-nagami", None);
    nagami.name = "Nagami 1/2 (2024)".to_string();
    nagami.facet = MediaFacet::Anime;
    nagami.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    TitleRepository::create(&catalog, nagami.clone())
        .await
        .expect("loose neighbor should insert");

    let mut azure_crate = make_test_title("title-search-azure-crate", None);
    azure_crate.name = "Azure Crate".to_string();
    azure_crate.facet = MediaFacet::Anime;
    azure_crate.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    TitleRepository::create(&catalog, azure_crate.clone())
        .await
        .expect("loose neighbor should insert");

    let mut her_quiet_sky = make_test_title("title-search-her-quiet-sky", None);
    her_quiet_sky.name = "Her Quiet Sky".to_string();
    TitleRepository::create(&catalog, her_quiet_sky.clone())
        .await
        .expect("movie loose neighbor should insert");

    let hits = TitleRepository::list(&catalog, None, Some("akumo".to_string()))
        .await
        .expect("short typo search should load");
    let hit_ids = hits
        .into_iter()
        .map(|title| title.id)
        .collect::<HashSet<_>>();

    assert!(hit_ids.contains(&aokumo.id));
    assert!(!hit_ids.contains(&nagami.id));
    assert!(!hit_ids.contains(&azure_crate.id));
    assert!(!hit_ids.contains(&her_quiet_sky.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_returns_valid_single_substitution_typo_for_frielen() {
    let (services, db) = temp_services("scryer_catalog_title_search_frielen_typo").await;
    let catalog = title_store(&services);

    let mut frielen = make_test_title("title-search-frielen", None);
    frielen.name = "Silver Horizon: Beyond Harbor's End".to_string();
    frielen.facet = MediaFacet::Anime;
    frielen.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    frielen.aliases = vec!["Sora no Vale".to_string(), "Frielen".to_string()];
    TitleRepository::create(&catalog, frielen.clone())
        .await
        .expect("frielen should insert");

    let mut friend = make_test_title("title-search-friend", None);
    friend.name = "Friend".to_string();
    TitleRepository::create(&catalog, friend.clone())
        .await
        .expect("friend should insert");

    let mut signal_run = make_test_title("title-search-signal-run", None);
    signal_run.name = "Signal Run".to_string();
    TitleRepository::create(&catalog, signal_run.clone())
        .await
        .expect("signal_run should insert");

    let hits = TitleRepository::list(&catalog, None, Some("friefen".to_string()))
        .await
        .expect("frielen typo search should load");

    assert_eq!(
        hits.first().map(|title| title.id.as_str()),
        Some(frielen.id.as_str())
    );
    assert!(!hits.iter().any(|title| title.id == friend.id));
    assert!(!hits.iter().any(|title| title.id == signal_run.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_projection_refreshes_after_hydrated_metadata_update_and_delete() {
    let (services, db) = temp_services("scryer_title_search_projection_refresh").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-projection-refresh", None);
    title.name = "Example Show".to_string();
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let missing_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("pre-update search should load");
    assert!(missing_hits.is_empty());

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            slug: Some("earth-defenders".to_string()),
            aliases: vec!["Earth's Defenders".to_string()],
            tagged_aliases: vec![TaggedAlias {
                name: "Earth Defenders".to_string(),
                language: "eng".to_string(),
            }],
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        },
    )
    .await
    .expect("hydrated metadata should update");

    let alias_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("alias search should load");
    assert_eq!(
        alias_hits
            .first()
            .map(|match_title| match_title.id.as_str()),
        Some(title.id.as_str())
    );

    TitleRepository::delete(&catalog, &title.id)
        .await
        .expect("title should delete");

    let deleted_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("post-delete search should load");
    assert!(deleted_hits.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_wanted_items_filters_with_fuzzy_title_search() {
    let (services, db) = temp_services("scryer_wanted_title_search").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();

    let mut title = make_test_title("title-search-match", None);
    title.name = "Lanternhouse Rock! Earth".to_string();
    title.aliases = vec!["Lantern House Rock".to_string()];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("matching title should insert");
    let mut other_title = make_test_title("title-search-other", None);
    other_title.name = "Different Show".to_string();
    TitleRepository::create(&catalog, other_title.clone())
        .await
        .expect("other title should insert");

    let wanted_match = AcquisitionScopeState {
        id: "wanted-search-match".to_string(),
        title_id: title.id.clone(),
        title_name: Some("Lanternhouse Rock! Earth".to_string()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "episode".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let wanted_other = AcquisitionScopeState {
        id: "wanted-search-other".to_string(),
        title_id: other_title.id.clone(),
        title_name: Some("Different Show".to_string()),
        ..wanted_match.clone()
    };

    workflow
        .upsert_acquisition_scope_state(&wanted_match)
        .await
        .expect("matching wanted item should insert");
    workflow
        .upsert_acquisition_scope_state(&wanted_other)
        .await
        .expect("other wanted item should insert");

    let items = workflow
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_search: Some("lanterhouse erth".into()),
            limit: 50,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("filtered wanted items should load");
    let count = workflow
        .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_search: Some("lanterhouse erth".into()),
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("filtered wanted count should load");

    assert_eq!(items.len(), 1);
    assert_eq!(count, 1);
    assert_eq!(items[0].id, wanted_match.id);

    let short_items = workflow
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_search: Some("roc".into()),
            limit: 50,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("short filtered wanted items should load");
    let short_count = workflow
        .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_search: Some("roc".into()),
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("short filtered wanted count should load");

    assert_eq!(short_items.len(), 1);
    assert_eq!(short_count, 1);
    assert_eq!(short_items[0].id, wanted_match.id);

    let short_title_hits = TitleRepository::list(&catalog, None, Some("roc".to_string()))
        .await
        .expect("short title list search should load");
    assert_eq!(short_title_hits.len(), 1);
    assert_eq!(short_title_hits[0].id, title.id);

    let _ = std::fs::remove_file(db);
}
