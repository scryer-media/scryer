use super::*;

#[tokio::test]
async fn graphql_trusted_proxies_requires_system_settings_permission() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let viewer = ctx
        .app
        .create_user(
            &admin,
            "proxy-viewer".into(),
            "fixture-password".into(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .unwrap();
    let response = schema_exec(&ctx, r#"mutation { updateServiceSettings(input: {trustedProxyIps:["127.0.0.1"]}) { trustedProxyIps } }"#, Some(viewer)).await;
    assert_graphql_field_denied(&response, "updateServiceSettings");
    assert!(
        ctx.app
            .trusted_proxy_runtime()
            .snapshot()
            .addresses
            .is_empty()
    );
}

#[tokio::test]
async fn graphql_trusted_proxies_override_reset_and_reload() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    ctx.app
        .initialize_trusted_proxy_policy("127.0.0.1")
        .await
        .unwrap();
    let runtime = ctx.app.trusted_proxy_runtime();
    let before = runtime.snapshot();
    let mutation = r#"mutation Update($input: UpdateServiceSettingsInput!) {
        updateServiceSettings(input: $input) { tlsCertPath trustedProxyIps trustedProxyOverride trustedProxySource }
    }"#;
    let saved = gql(&ctx, mutation, json!({"input":{"trustedProxyIps":["192.0.2.0/24", "::1"], "tlsCertPath":"/etc/scryer/current.crt"}})).await;
    assert_no_errors(&saved);
    assert_eq!(
        saved["data"]["updateServiceSettings"]["trustedProxySource"],
        "settings"
    );
    assert!(runtime.snapshot().matches("192.0.2.1".parse().unwrap()));
    assert!(!runtime.snapshot().matches("127.0.0.1".parse().unwrap()));
    assert!(before.matches("127.0.0.1".parse().unwrap()));

    let invalid = gql(
        &ctx,
        mutation,
        json!({"input":{"trustedProxyIps":["192.0.2.1/99"]}}),
    )
    .await;
    assert!(
        invalid["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty())
    );
    assert!(runtime.snapshot().matches("192.0.2.1".parse().unwrap()));
    // Older clients can still save TLS paths without replacing the proxy policy.
    let legacy = gql(
        &ctx,
        mutation,
        json!({"input":{"tlsCertPath":"/etc/scryer/new.crt", "tlsKeyPath":"/etc/scryer/new.key"}}),
    )
    .await;
    assert_no_errors(&legacy);
    assert!(runtime.snapshot().matches("192.0.2.1".parse().unwrap()));

    let empty = gql(&ctx, mutation, json!({"input":{"trustedProxyIps":[]}})).await;
    assert_no_errors(&empty);
    assert_eq!(
        empty["data"]["updateServiceSettings"]["tlsCertPath"],
        "/etc/scryer/new.crt"
    );
    assert_eq!(
        empty["data"]["updateServiceSettings"]["trustedProxyOverride"],
        json!([])
    );
    ctx.app
        .initialize_trusted_proxy_policy("127.0.0.1")
        .await
        .unwrap();
    assert!(!runtime.snapshot().matches("127.0.0.1".parse().unwrap()));

    let reset = gql(
        &ctx,
        mutation,
        json!({"input":{"resetTrustedProxyIps":true}}),
    )
    .await;
    assert_no_errors(&reset);
    assert_eq!(
        reset["data"]["updateServiceSettings"]["trustedProxyOverride"],
        json!(null)
    );
    assert!(runtime.snapshot().matches("127.0.0.1".parse().unwrap()));
    ctx.app
        .initialize_trusted_proxy_policy("::1")
        .await
        .unwrap();
    assert!(runtime.snapshot().matches("::1".parse().unwrap()));
}

#[tokio::test]
async fn graphql_media_settings_rejects_invalid_folder_template_tokens() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            folderTemplate
          }
        }
        "#,
        json!({
          "input": {
            "scope": "MOVIE",
            "folderTemplate": "{quality}"
          }
        }),
    )
    .await;

    let errors = body["errors"]
        .as_array()
        .expect("invalid folder template should return graphql errors");
    assert!(!errors.is_empty());
    let message = errors[0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unsupported folder template token"));
}

#[tokio::test]
async fn graphql_media_settings_rejects_invalid_season_folder_templates() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            seasonFolderTemplate
          }
        }
        "#,
        json!({
          "input": {
            "scope": "SERIES",
            "seasonFolderTemplate": "Season"
          }
        }),
    )
    .await;

    let errors = body["errors"]
        .as_array()
        .expect("invalid season folder template should return graphql errors");
    assert!(!errors.is_empty());
    let message = errors[0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("must include {season}"));
}

#[tokio::test]
async fn graphql_media_settings_rejects_blank_episode_folder_templates() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    for input in [
        json!({
          "scope": "SERIES",
          "seasonFolderTemplate": "   "
        }),
        json!({
          "scope": "ANIME",
          "specialsFolderTemplate": "\n"
        }),
    ] {
        let body = gql(
            &ctx,
            r#"
            mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
              updateMediaSettings(input: $input) { scope }
            }
            "#,
            json!({ "input": input }),
        )
        .await;

        let errors = body["errors"]
            .as_array()
            .expect("blank episode folder template should return graphql errors");
        assert!(!errors.is_empty());
        assert!(
            errors[0]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("template is required")
        );
    }
}

#[tokio::test]
async fn graphql_media_settings_rejects_invalid_rename_template_tokens() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let body = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) {
            scope
            renameTemplate
          }
        }
        "#,
        json!({
          "input": {
            "scope": "MOVIE",
            "renameTemplate": "{title|truncate:0}.{ext}"
          }
        }),
    )
    .await;

    let errors = body["errors"]
        .as_array()
        .expect("invalid rename template should return graphql errors");
    assert!(!errors.is_empty());
    let message = errors[0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("unsupported rename template token"));
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
            requiredAudioLanguages
            folderTemplate
            seasonFolderTemplate
            specialsFolderTemplate
            renameEnabled
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
            "scope": "ANIME",
            "rootFolders": [
              { "path": "/library/anime-main", "isDefault": true },
              { "path": "/library/anime-archive", "isDefault": false }
            ],
            "requiredAudioLanguages": ["eng", "jpn"],
            "folderTemplate": "{title|truncate:64|space:_} ({year})",
            "seasonFolderTemplate": "{title|space:.}.S{season:2}",
            "specialsFolderTemplate": "{title} Specials",
            "renameEnabled": false,
            "renameTemplate": "{title|truncate:64|space:_} [{quality}].{ext}",
            "renameCollisionPolicy": "REPLACE_IF_BETTER",
            "renameMissingMetadataPolicy": "SKIP",
            "fillerPolicy": "SKIP_FILLER",
            "recapPolicy": "SKIP_RECAP",
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
    assert_eq!(updated["scope"], "ANIME");
    assert_eq!(updated["libraryPath"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["path"], "/library/anime-main");
    assert_eq!(updated["rootFolders"][0]["isDefault"], true);
    assert_eq!(updated["requiredAudioLanguages"][0], "eng");
    assert_eq!(updated["requiredAudioLanguages"][1], "jpn");
    assert_eq!(
        updated["folderTemplate"],
        "{title|truncate:64|space:_} ({year})"
    );
    assert_eq!(
        updated["seasonFolderTemplate"],
        "{title|space:.}.S{season:2}"
    );
    assert_eq!(updated["specialsFolderTemplate"], "{title} Specials");
    assert_eq!(updated["renameEnabled"], false);
    assert_eq!(
        updated["renameTemplate"],
        "{title|truncate:64|space:_} [{quality}].{ext}"
    );
    assert_eq!(updated["renameCollisionPolicy"], "REPLACE_IF_BETTER");
    assert_eq!(updated["renameMissingMetadataPolicy"], "SKIP");
    assert_eq!(updated["fillerPolicy"], "SKIP_FILLER");
    assert_eq!(updated["recapPolicy"], "SKIP_RECAP");
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
            requiredAudioLanguages
            folderTemplate
            seasonFolderTemplate
            specialsFolderTemplate
            renameEnabled
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
        json!({ "scope": "ANIME" }),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["mediaSettings"];
    assert_eq!(settings["scope"], "ANIME");
    assert_eq!(settings["libraryPath"], "/library/anime-main");
    assert_eq!(settings["rootFolders"][1]["path"], "/library/anime-archive");
    assert_eq!(settings["requiredAudioLanguages"][0], "eng");
    assert_eq!(settings["requiredAudioLanguages"][1], "jpn");
    assert_eq!(
        settings["folderTemplate"],
        "{title|truncate:64|space:_} ({year})"
    );
    assert_eq!(
        settings["seasonFolderTemplate"],
        "{title|space:.}.S{season:2}"
    );
    assert_eq!(settings["specialsFolderTemplate"], "{title} Specials");
    assert_eq!(settings["renameEnabled"], false);
    assert_eq!(
        settings["renameTemplate"],
        "{title|truncate:64|space:_} [{quality}].{ext}"
    );
    assert_eq!(settings["renameCollisionPolicy"], "REPLACE_IF_BETTER");
    assert_eq!(settings["renameMissingMetadataPolicy"], "SKIP");
    assert_eq!(settings["fillerPolicy"], "SKIP_FILLER");
    assert_eq!(settings["recapPolicy"], "SKIP_RECAP");
    assert_eq!(settings["monitorSpecials"], true);
    assert_eq!(settings["interSeasonMovies"], false);
    assert_eq!(settings["monitorFillerMovies"], true);
    assert_eq!(settings["nfoWriteOnImport"], true);
    assert_eq!(settings["plexmatchWriteOnImport"], true);
}

#[tokio::test]
async fn graphql_episode_folder_templates_remain_facet_scoped() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    for (scope, season_template, specials_template) in [
        ("SERIES", "Series S{season:2}", "Series Specials"),
        ("ANIME", "Anime S{season:3}", "Anime Specials"),
    ] {
        let update = gql(
            &ctx,
            r#"
            mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
              updateMediaSettings(input: $input) {
                scope
                seasonFolderTemplate
                specialsFolderTemplate
              }
            }
            "#,
            json!({
              "input": {
                "scope": scope,
                "seasonFolderTemplate": season_template,
                "specialsFolderTemplate": specials_template
              }
            }),
        )
        .await;
        assert_no_errors(&update);
    }

    for (scope, season_template, specials_template) in [
        ("SERIES", "Series S{season:2}", "Series Specials"),
        ("ANIME", "Anime S{season:3}", "Anime Specials"),
    ] {
        let read = gql(
            &ctx,
            r#"
            query MediaSettings($scope: ContentScopeValue!) {
              mediaSettings(scope: $scope) {
                seasonFolderTemplate
                specialsFolderTemplate
              }
            }
            "#,
            json!({ "scope": scope }),
        )
        .await;
        assert_no_errors(&read);
        assert_eq!(
            read["data"]["mediaSettings"]["seasonFolderTemplate"],
            season_template
        );
        assert_eq!(
            read["data"]["mediaSettings"]["specialsFolderTemplate"],
            specials_template
        );
    }
}

#[tokio::test]
async fn graphql_movie_settings_expose_null_and_reject_season_folder_templates() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let read = gql(
        &ctx,
        r#"
        query MediaSettings($scope: ContentScopeValue!) {
          mediaSettings(scope: $scope) {
            seasonFolderTemplate
            specialsFolderTemplate
          }
        }
        "#,
        json!({ "scope": "MOVIE" }),
    )
    .await;
    assert_no_errors(&read);
    assert!(read["data"]["mediaSettings"]["seasonFolderTemplate"].is_null());
    assert!(read["data"]["mediaSettings"]["specialsFolderTemplate"].is_null());

    let update = gql(
        &ctx,
        r#"
        mutation UpdateMediaSettings($input: UpdateMediaSettingsInput!) {
          updateMediaSettings(input: $input) { scope }
        }
        "#,
        json!({
          "input": {
            "scope": "MOVIE",
            "seasonFolderTemplate": "Season {season}"
          }
        }),
    )
    .await;
    let errors = update["errors"]
        .as_array()
        .expect("movie season folder updates should return graphql errors");
    assert!(!errors.is_empty());
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("only supported for series and anime")
    );
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
    ctx.settings_store
        .upsert_setting_value(
            "system",
            "subtitles.opensubtitles_api_key",
            None,
            json!("smg-managed-key").to_string(),
            "test",
            None,
        )
        .await
        .expect("subtitle api key should seed");
    let update = gql(
        &ctx,
        r#"
        mutation UpdateSubtitleSettings($input: UpdateSubtitleSettingsInput!) {
          updateSubtitleSettings(input: $input) {
            enabled
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
            "languages": [
              { "code": "eng", "hearingImpaired": true, "forced": false },
              { "code": "spa", "hearingImpaired": false, "forced": true }
            ],
            "autoDownloadOnImport": true,
            "minimumScoreSeries": 95,
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

    let read = gql(
        &ctx,
        r#"
        query SubtitleSettings {
          subtitleSettings {
            enabled
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
    assert_eq!(settings["autoDownloadOnImport"], true);
    assert_eq!(settings["minimumScoreSeries"], 95);
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
async fn graphql_typed_subtitle_settings_invalid_scores_fall_back_to_defaults() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    for (key, value) in [
        ("subtitles.minimum_score_series", json!(101)),
        ("subtitles.minimum_score_movie", json!(-1)),
    ] {
        ctx.settings_store
            .upsert_setting_value("system", key, None, value.to_string(), "test", None)
            .await
            .expect("subtitle score should seed");
    }

    let read = gql(
        &ctx,
        r#"
        query SubtitleSettings {
          subtitleSettings {
            minimumScoreSeries
            minimumScoreMovie
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["subtitleSettings"]["minimumScoreSeries"], 90);
    assert_eq!(read["data"]["subtitleSettings"]["minimumScoreMovie"], 70);
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
            sameTierMinDelta
            pollIntervalSeconds
            walkIntervalSeconds
            longTailBackfillMaxScopesPerCycle
            longTailReconvergeDays
            defaultMovieAvailability
            movieReleaseMarket
            movieAvailabilityDelayDays
          }
        }
        "#,
        json!({
          "input": {
            "enabled": true,
            "sameTierMinDelta": 140,
            "pollIntervalSeconds": 45,
            "walkIntervalSeconds": 240,
            "longTailBackfillMaxScopesPerCycle": 750,
            "longTailReconvergeDays": 30,
            "defaultMovieAvailability": "released",
            "movieReleaseMarket": "US",
            "movieAvailabilityDelayDays": -2
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
            sameTierMinDelta
            pollIntervalSeconds
            walkIntervalSeconds
            longTailBackfillMaxScopesPerCycle
            longTailReconvergeDays
            defaultMovieAvailability
            movieReleaseMarket
            movieAvailabilityDelayDays
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["acquisitionSettings"];
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["sameTierMinDelta"], 140);
    assert_eq!(settings["pollIntervalSeconds"], 45);
    assert_eq!(settings["walkIntervalSeconds"], 240);
    assert_eq!(settings["longTailBackfillMaxScopesPerCycle"], 750);
    assert_eq!(settings["longTailReconvergeDays"], 30);
    assert_eq!(settings["defaultMovieAvailability"], "released");
    assert_eq!(settings["movieReleaseMarket"], "US");
    assert_eq!(settings["movieAvailabilityDelayDays"], -2);
}

/// The walk interval defaults to five minutes, and a client that predates it
/// (so never sends it) keeps whatever is stored instead of resetting it.
#[tokio::test]
async fn graphql_acquisition_walk_interval_defaults_and_survives_older_clients() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let read_query = r#"
        query AcquisitionSettings {
          acquisitionSettings {
            pollIntervalSeconds
            walkIntervalSeconds
            defaultMovieAvailability
            movieReleaseMarket
            movieAvailabilityDelayDays
          }
        }
        "#;
    let defaults = gql(&ctx, read_query, json!({})).await;
    assert_no_errors(&defaults);
    assert_eq!(
        defaults["data"]["acquisitionSettings"]["pollIntervalSeconds"],
        60
    );
    assert_eq!(
        defaults["data"]["acquisitionSettings"]["walkIntervalSeconds"],
        300
    );
    assert_eq!(
        defaults["data"]["acquisitionSettings"]["defaultMovieAvailability"],
        "announced"
    );
    assert_eq!(
        defaults["data"]["acquisitionSettings"]["movieReleaseMarket"],
        "US"
    );
    assert_eq!(
        defaults["data"]["acquisitionSettings"]["movieAvailabilityDelayDays"],
        0
    );

    let mutation = r#"
        mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
          updateAcquisitionSettings(input: $input) {
            walkIntervalSeconds
            defaultMovieAvailability
            movieReleaseMarket
            movieAvailabilityDelayDays
          }
        }
        "#;
    let set = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "enabled": true,
            "sameTierMinDelta": 120,
            "pollIntervalSeconds": 30,
            "walkIntervalSeconds": 90,
            "longTailBackfillMaxScopesPerCycle": 500,
            "longTailReconvergeDays": 0,
            "defaultMovieAvailability": "in_cinemas",
            "movieReleaseMarket": "US",
            "movieAvailabilityDelayDays": 3
          }
        }),
    )
    .await;
    assert_no_errors(&set);
    assert_eq!(
        set["data"]["updateAcquisitionSettings"]["walkIntervalSeconds"],
        90
    );
    assert_eq!(
        set["data"]["updateAcquisitionSettings"]["defaultMovieAvailability"],
        "in_cinemas"
    );
    assert_eq!(
        set["data"]["updateAcquisitionSettings"]["movieReleaseMarket"],
        "US"
    );
    assert_eq!(
        set["data"]["updateAcquisitionSettings"]["movieAvailabilityDelayDays"],
        3
    );

    let older_client = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "enabled": true,
            "sameTierMinDelta": 120,
            "pollIntervalSeconds": 30,
            "longTailBackfillMaxScopesPerCycle": 500,
            "longTailReconvergeDays": 0
          }
        }),
    )
    .await;
    assert_no_errors(&older_client);
    assert_eq!(
        older_client["data"]["updateAcquisitionSettings"]["walkIntervalSeconds"],
        90
    );
    assert_eq!(
        older_client["data"]["updateAcquisitionSettings"]["defaultMovieAvailability"],
        "in_cinemas"
    );
    assert_eq!(
        older_client["data"]["updateAcquisitionSettings"]["movieReleaseMarket"],
        "US"
    );
    assert_eq!(
        older_client["data"]["updateAcquisitionSettings"]["movieAvailabilityDelayDays"],
        3
    );

    let rejected = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "enabled": true,
            "sameTierMinDelta": 120,
            "pollIntervalSeconds": 30,
            "walkIntervalSeconds": 0,
            "longTailBackfillMaxScopesPerCycle": 500,
            "longTailReconvergeDays": 0
          }
        }),
    )
    .await;
    assert!(
        rejected["errors"].is_array(),
        "a walk interval below one second is rejected: {rejected}"
    );
}

/// Clients built before the dead upgrade knobs were retired still send them.
/// The fields stay in the schema as deprecated no-ops: accepted, never stored,
/// and always reported as 0.
#[tokio::test]
async fn graphql_retired_acquisition_fields_are_accepted_and_ignored() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let update = gql(
        &ctx,
        r#"
        mutation UpdateAcquisitionSettings($input: UpdateAcquisitionSettingsInput!) {
          updateAcquisitionSettings(input: $input) {
            upgradeCooldownHours
            sameTierMinDelta
            crossTierMinDelta
            forcedUpgradeDeltaBypass
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
            "longTailBackfillMaxScopesPerCycle": 750,
            "longTailReconvergeDays": 30
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let settings = &update["data"]["updateAcquisitionSettings"];
    assert_eq!(settings["sameTierMinDelta"], 140);
    assert_eq!(settings["upgradeCooldownHours"], 0);
    assert_eq!(settings["crossTierMinDelta"], 0);
    assert_eq!(settings["forcedUpgradeDeltaBypass"], 0);
}

#[tokio::test]
async fn graphql_typed_general_settings_defaults() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
            srrdbFilenameRecoveryEnabled
            keepHistoryForever
            historyRetentionDays
            imageCacheMaxSizeMb
            effectiveImageCacheMaxSizeBytes
            effectiveImageCacheMaxSizeMb
            imageCacheMaxSizeEnvOverrideActive
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["generalSettings"]["experimentalFeaturesEnabled"],
        false
    );
    assert_eq!(
        read["data"]["generalSettings"]["personalizedDiscoveryEnabled"],
        true
    );
    assert_eq!(
        read["data"]["generalSettings"]["srrdbFilenameRecoveryEnabled"],
        false
    );
    assert_eq!(read["data"]["generalSettings"]["keepHistoryForever"], false);
    assert_eq!(read["data"]["generalSettings"]["historyRetentionDays"], 180);
    assert_eq!(read["data"]["generalSettings"]["imageCacheMaxSizeMb"], 256);
    if read["data"]["generalSettings"]["imageCacheMaxSizeEnvOverrideActive"] == false {
        assert_eq!(
            read["data"]["generalSettings"]["effectiveImageCacheMaxSizeBytes"],
            256 * 1024 * 1024
        );
        assert_eq!(
            read["data"]["generalSettings"]["effectiveImageCacheMaxSizeMb"],
            256.0
        );
    }
    assert_eq!(read["data"]["generalSettings"]["pluginHttpCaBundlePem"], "");
    assert_eq!(
        read["data"]["generalSettings"]["pluginHttpTrustedCertificates"],
        json!([])
    );
}

#[tokio::test]
async fn graphql_typed_general_settings_srrdb_filename_recovery_round_trips() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let defaults = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            srrdbFilenameRecoveryEnabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&defaults);
    assert_eq!(
        defaults["data"]["generalSettings"]["srrdbFilenameRecoveryEnabled"],
        false
    );

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            srrdbFilenameRecoveryEnabled
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "srrdbFilenameRecoveryEnabled": true
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateGeneralSettings"]["srrdbFilenameRecoveryEnabled"],
        true
    );
    // Untouched fields keep their stored values through a partial update.
    assert_eq!(
        update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        180
    );

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            srrdbFilenameRecoveryEnabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["generalSettings"]["srrdbFilenameRecoveryEnabled"],
        true
    );

    let restore = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            srrdbFilenameRecoveryEnabled
          }
        }
        "#,
        json!({
          "input": {
            "srrdbFilenameRecoveryEnabled": false
          }
        }),
    )
    .await;
    assert_no_errors(&restore);
    assert_eq!(
        restore["data"]["updateGeneralSettings"]["srrdbFilenameRecoveryEnabled"],
        false
    );

    let final_read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            srrdbFilenameRecoveryEnabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&final_read);
    assert_eq!(
        final_read["data"]["generalSettings"]["srrdbFilenameRecoveryEnabled"],
        false
    );
}

#[tokio::test]
async fn graphql_typed_general_settings_instance_feature_switches_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "experimentalFeaturesEnabled": true,
            "personalizedDiscoveryEnabled": false
          }
        }),
    )
    .await;
    assert_no_errors(&update);
    assert_eq!(
        update["data"]["updateGeneralSettings"]["experimentalFeaturesEnabled"],
        true
    );
    assert_eq!(
        update["data"]["updateGeneralSettings"]["personalizedDiscoveryEnabled"],
        false
    );
    // Untouched fields keep their stored values through a partial update.
    assert_eq!(
        update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        180
    );

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(
        read["data"]["generalSettings"]["experimentalFeaturesEnabled"],
        true
    );
    assert_eq!(
        read["data"]["generalSettings"]["personalizedDiscoveryEnabled"],
        false
    );

    let restore = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
          }
        }
        "#,
        json!({
          "input": {
            "experimentalFeaturesEnabled": false,
            "personalizedDiscoveryEnabled": true
          }
        }),
    )
    .await;
    assert_no_errors(&restore);
    assert_eq!(
        restore["data"]["updateGeneralSettings"]["experimentalFeaturesEnabled"],
        false
    );
    assert_eq!(
        restore["data"]["updateGeneralSettings"]["personalizedDiscoveryEnabled"],
        true
    );
}

#[tokio::test]
async fn graphql_instance_features_readable_without_manage_system_settings() {
    const INSTANCE_FEATURES_QUERY: &str = r#"
        query InstanceFeatures {
          instanceFeatures {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
          }
        }
        "#;

    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    let viewer = ctx
        .app
        .create_user(
            &admin,
            "instance_features_viewer".to_string(),
            "viewer-pass1".to_string(),
            AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create user without manage system settings");
    let viewer_token = ctx
        .app
        .issue_access_token(&viewer)
        .await
        .expect("issue viewer token");

    let denied = gql_with_token(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            experimentalFeaturesEnabled
          }
        }
        "#,
        json!({}),
        &viewer_token,
    )
    .await;
    assert_graphql_field_denied(&denied, "generalSettings");

    let defaults = gql_with_token(&ctx, INSTANCE_FEATURES_QUERY, json!({}), &viewer_token).await;
    assert_no_errors(&defaults);
    assert_eq!(
        defaults["data"]["instanceFeatures"]["experimentalFeaturesEnabled"],
        false
    );
    assert_eq!(
        defaults["data"]["instanceFeatures"]["personalizedDiscoveryEnabled"],
        true
    );

    let update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
          }
        }
        "#,
        json!({
          "input": {
            "experimentalFeaturesEnabled": true,
            "personalizedDiscoveryEnabled": false
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    let after = gql_with_token(&ctx, INSTANCE_FEATURES_QUERY, json!({}), &viewer_token).await;
    assert_no_errors(&after);
    assert_eq!(
        after["data"]["instanceFeatures"]["experimentalFeaturesEnabled"],
        true
    );
    assert_eq!(
        after["data"]["instanceFeatures"]["personalizedDiscoveryEnabled"],
        false
    );
}

#[tokio::test]
async fn graphql_instance_features_requires_a_session() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    // Form login refuses to turn on until a full administrator can actually
    // log in, so give the default admin a password first.
    let admin = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.app
        .set_initial_own_password(&admin, "admin-pass1".to_string())
        .await
        .expect("set initial default admin password");

    let enable_form_login = gql(
        &ctx,
        r#"
        mutation UpdateSecuritySettings {
          updateSecuritySettings(input: {
            formLoginEnabled: true
            passwordMinLength: 8
            skipLoginForLocalIps: false
            mfaRequireConfigStepUp: false
            mfaRequirePasswordLogin: false
            totpRequireJellyfinLogin: false
            totpRequireEmbyLogin: false
          }) {
            effectiveFormLoginEnabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&enable_form_login);

    let anonymous = gql(
        &ctx,
        r#"
        query InstanceFeatures {
          instanceFeatures {
            experimentalFeaturesEnabled
            personalizedDiscoveryEnabled
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_graphql_field_denied(&anonymous, "instanceFeatures");
}

#[tokio::test]
async fn graphql_typed_general_settings_round_trip_and_forever_preserves_days() {
    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let first_update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
            imageCacheMaxSizeMb
            effectiveImageCacheMaxSizeBytes
            effectiveImageCacheMaxSizeMb
            imageCacheMaxSizeEnvOverrideActive
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": false,
            "historyRetentionDays": 45,
            "imageCacheMaxSizeMb": 384,
            "pluginHttpCaBundlePem": TEST_PLUGIN_HTTP_CA_CERT_PEM
          }
        }),
    )
    .await;
    assert_no_errors(&first_update);
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        45
    );
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["imageCacheMaxSizeMb"],
        384
    );
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["pluginHttpCaBundlePem"],
        TEST_PLUGIN_HTTP_CA_CERT_PEM
    );
    assert_eq!(
        first_update["data"]["updateGeneralSettings"]["pluginHttpTrustedCertificates"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );

    let forever_update = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
            imageCacheMaxSizeMb
            effectiveImageCacheMaxSizeBytes
            effectiveImageCacheMaxSizeMb
            imageCacheMaxSizeEnvOverrideActive
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({
          "input": {
            "keepHistoryForever": true
          }
        }),
    )
    .await;
    assert_no_errors(&forever_update);
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["keepHistoryForever"],
        true
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["historyRetentionDays"],
        45
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["imageCacheMaxSizeMb"],
        384
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["pluginHttpCaBundlePem"],
        TEST_PLUGIN_HTTP_CA_CERT_PEM
    );
    assert_eq!(
        forever_update["data"]["updateGeneralSettings"]["pluginHttpTrustedCertificates"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );

    let read = gql(
        &ctx,
        r#"
        query GeneralSettings {
          generalSettings {
            keepHistoryForever
            historyRetentionDays
            imageCacheMaxSizeMb
            effectiveImageCacheMaxSizeBytes
            effectiveImageCacheMaxSizeMb
            imageCacheMaxSizeEnvOverrideActive
            pluginHttpCaBundlePem
            pluginHttpTrustedCertificates {
              fingerprintSha256
              pem
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["generalSettings"]["keepHistoryForever"], true);
    assert_eq!(read["data"]["generalSettings"]["historyRetentionDays"], 45);
    assert_eq!(read["data"]["generalSettings"]["imageCacheMaxSizeMb"], 384);
    assert_eq!(
        read["data"]["generalSettings"]["pluginHttpCaBundlePem"],
        TEST_PLUGIN_HTTP_CA_CERT_PEM
    );
    assert_eq!(
        read["data"]["generalSettings"]["pluginHttpTrustedCertificates"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn graphql_typed_general_settings_rejects_invalid_days() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let body = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            keepHistoryForever
            historyRetentionDays
          }
        }
        "#,
        json!({
          "input": {
            "historyRetentionDays": 0
          }
        }),
    )
    .await;

    assert!(
        body["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected validation errors: {body}"
    );
}

#[tokio::test]
async fn graphql_typed_general_settings_rejects_invalid_image_cache_size() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let body = gql(
        &ctx,
        r#"
        mutation UpdateGeneralSettings($input: UpdateGeneralSettingsInput!) {
          updateGeneralSettings(input: $input) {
            imageCacheMaxSizeMb
          }
        }
        "#,
        json!({
          "input": {
            "imageCacheMaxSizeMb": 0
          }
        }),
    )
    .await;

    assert!(
        body["errors"].as_array().is_some_and(|errors| {
            errors.iter().any(|error| {
                error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("at least 1 MiB"))
            })
        }),
        "expected image cache validation error: {body}"
    );
}
