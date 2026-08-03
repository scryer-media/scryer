use super::*;

#[tokio::test]
async fn graphql_introspection_lists_title_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
		          queryRoot: __type(name: "QueryRoot") {
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
		              args {
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
	                    }
	                  }
	                }
	              }
	            }
	          }
          subscriptionRoot: __type(name: "SubscriptionRoot") {
            fields {
              name
              args {
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
          }
	          title: __type(name: "TitlePayload") {
            fields {
              name
              args {
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
                    }
                  }
                }
              }
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
          titleCatalog: __type(name: "TitleCatalogPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name } } } }
          }
          library: __type(name: "LibraryPayload") {
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
          libraryRoot: __type(name: "LibraryRootPayload") {
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
          episode: __type(name: "EpisodePayload") {
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
          collection: __type(name: "CollectionPayload") {
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
                  }
                }
              }
            }
          }
          titleMediaFile: __type(name: "TitleMediaFilePayload") {
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
          movieEntity: __type(name: "MovieEntityPayload") {
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
                  }
                }
              }
            }
          }
          seriesMovieLink: __type(name: "SeriesMovieLinkPayload") {
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
                  }
                }
              }
            }
          }
	          wantedItem: __type(name: "WantedItemPayload") {
	            fields {
	              name
              args {
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
                    }
                  }
                }
              }
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
          releaseDecision: __type(name: "ReleaseDecisionPayload") {
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
                  }
                }
              }
            }
          }
          pendingRelease: __type(name: "PendingReleasePayload") {
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
                  }
                }
              }
            }
          }
          wantedItemsPage: __type(name: "WantedItemsPagePayload") {
            fields { name }
          }
          releaseDecisionsPage: __type(name: "ReleaseDecisionsPagePayload") {
            fields { name }
          }
          pendingReleasesPage: __type(name: "PendingReleasesPayload") {
            fields { name }
          }
	          calendarEpisode: __type(name: "CalendarEpisodePayload") {
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
	          titleHistoryFilter: __type(name: "TitleHistoryFilterInput") {
            inputFields {
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
                  }
                }
              }
            }
          }
	          titleCatalogFilter: __type(name: "TitleCatalogFilterInput") {
            inputFields {
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
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["title"]["fields"]
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

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .expect("type should expose fields")
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
            .clone()
    };
    let assert_non_null_object_field = |type_alias: &str, name: &str, object_name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL");
        assert_eq!(field["type"]["ofType"]["name"], object_name);
    };
    let assert_non_null_id_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL");
        assert_eq!(field["type"]["ofType"]["name"], "ID");
    };
    let assert_nullable_id_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR");
        assert_eq!(field["type"]["name"], "ID");
    };
    let assert_non_null_id_list_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL");
        assert_eq!(field["type"]["ofType"]["kind"], "LIST");
        assert_eq!(field["type"]["ofType"]["ofType"]["kind"], "NON_NULL");
        assert_eq!(field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID");
    };
    for (type_alias, name) in [
        ("title", "id"),
        ("title", "libraryId"),
        ("library", "id"),
        ("libraryRoot", "id"),
        ("episode", "id"),
        ("episode", "titleId"),
        ("collection", "id"),
        ("collection", "titleId"),
        ("titleMediaFile", "id"),
        ("titleMediaFile", "titleId"),
        ("movieEntity", "id"),
        ("seriesMovieLink", "id"),
        ("wantedItem", "id"),
        ("wantedItem", "titleId"),
        ("releaseDecision", "id"),
        ("releaseDecision", "wantedItemId"),
        ("releaseDecision", "titleId"),
        ("pendingRelease", "id"),
        ("pendingRelease", "wantedItemId"),
        ("pendingRelease", "titleId"),
        ("calendarEpisode", "id"),
        ("calendarEpisode", "titleId"),
        ("calendarEpisode", "libraryId"),
    ] {
        assert_non_null_id_field(type_alias, name);
    }
    for (type_alias, name) in [
        ("title", "qualityProfileId"),
        ("titleMediaFile", "episodeId"),
        ("seriesMovieLink", "linkedEpisodeId"),
    ] {
        assert_nullable_id_field(type_alias, name);
    }
    assert_non_null_id_list_field("titleMediaFile", "seriesMovieLinkIds");
    assert_non_null_object_field("title", "wantedItems", "WantedItemsPagePayload");
    assert_non_null_object_field("title", "releaseDecisions", "ReleaseDecisionsPagePayload");
    assert_non_null_object_field(
        "wantedItem",
        "releaseDecisions",
        "ReleaseDecisionsPagePayload",
    );
    assert_non_null_object_field("wantedItem", "pendingReleases", "PendingReleasesPayload");

    let field_arg = |type_alias: &str, field_name: &str, arg_name: &str| {
        let field_value = field(type_alias, field_name);
        field_value["args"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should expose args"))
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name}.{arg_name} should exist"))
            .clone()
    };
    let status_arg = field_arg("title", "wantedItems", "status");
    assert_eq!(status_arg["type"]["kind"], "ENUM");
    assert_eq!(status_arg["type"]["name"], "WantedStatusValue");
    for (type_alias, field_name) in [
        ("title", "wantedItems"),
        ("title", "releaseDecisions"),
        ("wantedItem", "releaseDecisions"),
        ("wantedItem", "pendingReleases"),
    ] {
        for arg_name in ["limit", "offset"] {
            let arg = field_arg(type_alias, field_name, arg_name);
            assert_eq!(
                arg["type"]["kind"], "NON_NULL",
                "{type_alias}.{field_name}.{arg_name}"
            );
            assert_eq!(
                arg["type"]["ofType"]["name"], "Int",
                "{type_alias}.{field_name}.{arg_name}"
            );
        }
    }

    for (type_alias, expected_fields) in [
        // 0.17.0 pagination unification: pages expose {items, totalCount,
        // hasMore}; input echoes (limit/offset) were removed. The bare
        // wantedItemsPage relation stays items-only.
        ("wantedItemsPage", vec!["items"]),
        (
            "releaseDecisionsPage",
            vec!["items", "totalCount", "hasMore"],
        ),
        (
            "pendingReleasesPage",
            vec!["items", "hasMore", "totalCount"],
        ),
    ] {
        let fields: Vec<&str> = body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect();
        assert_eq!(fields, expected_fields, "{type_alias}");
    }

    for (type_alias, name) in [("title", "createdAt"), ("episode", "createdAt")] {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL");
        assert_eq!(field["type"]["ofType"]["name"], "DateTime");
    }
    let calendar_air_date = field("calendarEpisode", "airDate");
    assert_eq!(calendar_air_date["type"]["kind"], "SCALAR");
    assert_eq!(calendar_air_date["type"]["name"], "Date");

    let query_fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let query_field = |field_name: &str| {
        query_fields
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("QueryRoot.{field_name} should exist"))
            .clone()
    };
    let query_arg = |field_name: &str, arg_name: &str| {
        query_fields
            .iter()
            .find(|field| field["name"] == field_name)
            .expect("query field should exist")["args"]
            .as_array()
            .expect("query field should expose args")
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .expect("query arg should exist")
            .clone()
    };
    let query_field_names: Vec<&str> = query_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(
        !query_field_names.contains(&"titlesPage"),
        "QueryRoot.titlesPage should not exist"
    );
    let titles_field = query_field("titles");
    assert_eq!(titles_field["type"]["kind"], "NON_NULL");
    assert_eq!(
        titles_field["type"]["ofType"]["name"],
        "TitleCatalogPayload"
    );
    let title_catalog_fields: Vec<&str> = body["data"]["titleCatalog"]["fields"]
        .as_array()
        .expect("TitleCatalogPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        title_catalog_fields,
        vec![
            "items",
            "hasMore",
            "totalCount",
            "filterCounts",
            "managedBytes",
        ]
    );
    let subscription_fields = body["data"]["subscriptionRoot"]["fields"]
        .as_array()
        .expect("SubscriptionRoot should expose fields");
    let subscription_arg = |field_name: &str, arg_name: &str| {
        subscription_fields
            .iter()
            .find(|field| field["name"] == field_name)
            .expect("subscription field should exist")["args"]
            .as_array()
            .expect("subscription field should expose args")
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .expect("subscription arg should exist")
            .clone()
    };
    for (field_name, arg_name) in [
        ("title", "id"),
        ("wantedItem", "id"),
        ("titleReleaseBlocklist", "titleId"),
        ("titleAcquisitionDiagnostics", "titleId"),
        ("externalSubtitles", "titleId"),
        ("librarySettings", "libraryId"),
        ("linkedAccounts", "userId"),
        ("pendingImportBindingPreview", "pendingImportId"),
        ("postProcessingScriptRuns", "scriptId"),
        ("externalSubtitleBlocklistEntries", "mediaFileId"),
    ] {
        let arg = query_arg(field_name, arg_name);
        if field_name == "linkedAccounts" {
            assert_eq!(arg["type"]["kind"], "SCALAR", "{field_name}.{arg_name}");
            assert_eq!(arg["type"]["name"], "ID", "{field_name}.{arg_name}");
        } else {
            assert_eq!(arg["type"]["kind"], "NON_NULL", "{field_name}.{arg_name}");
            assert_eq!(
                arg["type"]["ofType"]["name"], "ID",
                "{field_name}.{arg_name}"
            );
        }
    }
    for arg_name in ["startDate", "endDate"] {
        let arg = query_arg("calendarEpisodes", arg_name);
        assert_eq!(arg["type"]["kind"], "NON_NULL");
        assert_eq!(arg["type"]["ofType"]["name"], "Date");
    }
    for arg_name in ["limit", "offset"] {
        let arg = query_arg("titles", arg_name);
        assert_eq!(arg["type"]["kind"], "SCALAR");
        assert_eq!(arg["type"]["name"], "Int");
    }
    for (arg_name, expected_name) in [
        ("filter", "TitleCatalogFilterInput"),
        ("sort", "TitleCatalogSortInput"),
    ] {
        let arg = query_arg("titles", arg_name);
        assert_eq!(arg["type"]["kind"], "INPUT_OBJECT");
        assert_eq!(arg["type"]["name"], expected_name);
    }

    // `wantedItems` is the derived view now — the state-row `titleId`
    // filter was dropped (use `titleSearch` or the interactive job's `titleId`).
    let download_queue_title_id = query_arg("downloadQueue", "titleId");
    assert_eq!(download_queue_title_id["type"]["kind"], "SCALAR");
    assert_eq!(download_queue_title_id["type"]["name"], "ID");
    assert!(
        query_field("wantedItems")["args"]
            .as_array()
            .expect("wantedItems args")
            .iter()
            .all(|arg| arg["name"] != "titleId"),
        "wantedItems.titleId was removed in the derived view"
    );
    let subscription_title_id = subscription_arg("downloadQueue", "titleId");
    assert_eq!(subscription_title_id["type"]["kind"], "SCALAR");
    assert_eq!(subscription_title_id["type"]["name"], "ID");
    for field_name in [
        "titles",
        "mediaRequests",
        "myMediaRequests",
        "pendingImports",
        "wantedItems",
        "cutoffUnmetTitlesPage",
        "calendarEpisodes",
    ] {
        let arg = query_arg(field_name, "libraryIds");
        assert_eq!(arg["type"]["kind"], "LIST", "{field_name}.libraryIds");
        assert_eq!(
            arg["type"]["ofType"]["kind"], "NON_NULL",
            "{field_name}.libraryIds"
        );
        assert_eq!(
            arg["type"]["ofType"]["ofType"]["name"], "ID",
            "{field_name}.libraryIds"
        );
    }
    let client_ids = query_arg("downloadHistory", "clientIds");
    assert_eq!(
        client_ids["type"]["kind"], "LIST",
        "downloadHistory.clientIds"
    );
    assert_eq!(
        client_ids["type"]["ofType"]["kind"], "NON_NULL",
        "downloadHistory.clientIds"
    );
    assert_eq!(
        client_ids["type"]["ofType"]["ofType"]["name"], "ID",
        "downloadHistory.clientIds"
    );

    let history_filter_fields = body["data"]["titleHistoryFilter"]["inputFields"]
        .as_array()
        .expect("TitleHistoryFilterInput should expose fields");
    let history_filter_field = |name: &str| {
        history_filter_fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("TitleHistoryFilterInput.{name} should exist"))
    };
    for name in ["titleIds", "libraryIds"] {
        let field = history_filter_field(name);
        assert_eq!(field["type"]["kind"], "LIST", "{name}");
        assert_eq!(field["type"]["ofType"]["kind"], "NON_NULL", "{name}");
        assert_eq!(field["type"]["ofType"]["ofType"]["name"], "ID", "{name}");
    }
    let episode_id = history_filter_field("episodeId");
    assert_eq!(episode_id["type"]["kind"], "SCALAR");
    assert_eq!(episode_id["type"]["name"], "ID");

    let title_catalog_filter_fields = body["data"]["titleCatalogFilter"]["inputFields"]
        .as_array()
        .expect("TitleCatalogFilterInput should expose fields");
    let title_catalog_filter_field = |name: &str| {
        title_catalog_filter_fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("TitleCatalogFilterInput.{name} should exist"))
    };
    let content_statuses = title_catalog_filter_field("contentStatuses");
    assert_eq!(content_statuses["type"]["kind"], "LIST");
    assert_eq!(content_statuses["type"]["ofType"]["kind"], "NON_NULL");
    assert_eq!(content_statuses["type"]["ofType"]["ofType"]["kind"], "ENUM");
    assert_eq!(
        content_statuses["type"]["ofType"]["ofType"]["name"],
        "TitleCatalogContentStatusValue"
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_typed_timestamps_as_datetime() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          login: __type(name: "LoginPayload") { fields { name type { kind name ofType { kind name } } } }
          title: __type(name: "TitlePayload") { fields { name type { kind name ofType { kind name } } } }
          collection: __type(name: "CollectionPayload") { fields { name type { kind name ofType { kind name } } } }
          movieEntity: __type(name: "MovieEntityPayload") { fields { name type { kind name ofType { kind name } } } }
          seriesMovieLink: __type(name: "SeriesMovieLinkPayload") { fields { name type { kind name ofType { kind name } } } }
          mediaFile: __type(name: "TitleMediaFilePayload") { fields { name type { kind name ofType { kind name } } } }
          queueItem: __type(name: "DownloadQueueItemPayload") { fields { name type { kind name ofType { kind name } } } }
          wantedItem: __type(name: "WantedItemPayload") { fields { name type { kind name ofType { kind name } } } }
          releaseDecision: __type(name: "ReleaseDecisionPayload") { fields { name type { kind name ofType { kind name } } } }
          pendingRelease: __type(name: "PendingReleasePayload") { fields { name type { kind name ofType { kind name } } } }
          titleAcquisitionDiagnostics: __type(name: "TitleAcquisitionDiagnosticsPayload") { fields { name type { kind name ofType { kind name } } } }
          oauthConnectedApp: __type(name: "OauthConnectedAppPayload") { fields { name type { kind name ofType { kind name } } } }
          mediaRequest: __type(name: "MediaRequestPayload") { fields { name type { kind name ofType { kind name } } } }
          mediaRequestRequester: __type(name: "MediaRequestRequesterPayload") { fields { name type { kind name ofType { kind name } } } }
          approveMediaRequest: __type(name: "ApproveMediaRequestPayload") { fields { name type { kind name ofType { kind name } } } }
          user: __type(name: "UserPayload") { fields { name type { kind name ofType { kind name } } } }
          userLibraryPermissionGrant: __type(name: "UserLibraryPermissionGrantPayload") { fields { name type { kind name ofType { kind name } } } }
          linkedAccount: __type(name: "LinkedAccountPayload") { fields { name type { kind name ofType { kind name } } } }
          totpStatus: __type(name: "TotpStatusPayload") { fields { name type { kind name ofType { kind name } } } }
          totpEnrollmentStart: __type(name: "TotpEnrollmentStartPayload") { fields { name type { kind name ofType { kind name } } } }
          webauthnChallenge: __type(name: "WebauthnChallengePayload") { fields { name type { kind name ofType { kind name } } } }
          passkeySummary: __type(name: "PasskeySummaryPayload") { fields { name type { kind name ofType { kind name } } } }
          smgUpdateNotice: __type(name: "SmgScryerUpdateNoticePayload") { fields { name type { kind name ofType { kind name } } } }
          activityEvent: __type(name: "ActivityEventPayload") { fields { name type { kind name ofType { kind name } } } }
          domainEventEnvelope: __type(name: "DomainEventEnvelopePayload") { fields { name type { kind name ofType { kind name } } } }
          libraryScanProgress: __type(name: "LibraryScanProgressPayload") { fields { name type { kind name ofType { kind name } } } }
          jobScheduleInfo: __type(name: "JobScheduleInfoPayload") { fields { name type { kind name ofType { kind name } } } }
          jobRun: __type(name: "JobRunPayload") { fields { name type { kind name ofType { kind name } } } }
          indexerQueryStats: __type(name: "IndexerQueryStatsPayload") { fields { name type { kind name ofType { kind name } } } }
          indexerSearchResult: __type(name: "IndexerSearchResultPayload") { fields { name type { kind name ofType { kind name } } } }
          titleReleaseBlocklistEntry: __type(name: "TitleReleaseBlocklistEntryPayload") { fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } }
          importRecord: __type(name: "ImportRecordPayload") { fields { name type { kind name ofType { kind name } } } }
          episodeScope: __type(name: "EpisodeScopePayload") { fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } }
          episodeSetScope: __type(name: "EpisodeSetScopePayload") { fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } }
          seriesMovieScope: __type(name: "SeriesMovieScopePayload") { fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } }
          collectionScope: __type(name: "CollectionScopePayload") { fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } }
          autoBackupSettings: __type(name: "AutoBackupSettingsPayload") { fields { name type { kind name ofType { kind name } } } }
          indexerConfig: __type(name: "IndexerConfigPayload") { fields { name type { kind name ofType { kind name } } } }
          downloadClientConfig: __type(name: "DownloadClientConfigPayload") { fields { name type { kind name ofType { kind name } } } }
          subtitleProviderConfig: __type(name: "SubtitleProviderConfigPayload") { fields { name type { kind name ofType { kind name } } } }
          ruleSet: __type(name: "RuleSetPayload") { fields { name type { kind name ofType { kind name } } } }
          pluginInstallation: __type(name: "PluginInstallationPayload") { fields { name type { kind name ofType { kind name } } } }
          pluginCatalogStatus: __type(name: "PluginCatalogStatusPayload") { fields { name type { kind name ofType { kind name } } } }
          notificationChannel: __type(name: "NotificationChannelPayload") { fields { name type { kind name ofType { kind name } } } }
          notificationSubscription: __type(name: "NotificationSubscriptionPayload") { fields { name type { kind name ofType { kind name } } } }
          postProcessingScript: __type(name: "PostProcessingScriptPayload") { fields { name type { kind name ofType { kind name } } } }
          postProcessingScriptRun: __type(name: "PostProcessingScriptRunPayload") { fields { name type { kind name ofType { kind name } } } }
          titleHistoryEvent: __type(name: "TitleHistoryEventPayload") { fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } } }
          externalImportWarmup: __type(name: "ExternalImportMonitorWarmupProgressPayload") { fields { name type { kind name ofType { kind name } } } }
          serviceLogs: __type(name: "ServiceLogsPayload") { fields { name type { kind name ofType { kind name } } } }
          recycledItem: __type(name: "RecycledItemPayload") { fields { name type { kind name ofType { kind name } } } }
          externalSubtitle: __type(name: "ExternalSubtitlePayload") { fields { name type { kind name ofType { kind name } } } }
          externalSubtitleBlocklistEntry: __type(name: "ExternalSubtitleBlocklistEntryPayload") { fields { name type { kind name ofType { kind name } } } }
          backupInfo: __type(name: "BackupInfoPayload") { fields { name type { kind name ofType { kind name } } } }
          backupDownloadUrl: __type(name: "BackupDownloadUrlPayload") { fields { name type { kind name ofType { kind name } } } }
          restoreInspect: __type(name: "RestoreInspectPayload") { fields { name type { kind name ofType { kind name } } } }
          restoreSummary: __type(name: "RestoreSummaryPayload") { fields { name type { kind name ofType { kind name } } } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let assert_optional_datetime_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{name}");
        assert_eq!(field["type"]["name"], "DateTime", "{type_alias}.{name}");
    };
    let assert_non_null_datetime_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(
            field["type"]["ofType"]["name"], "DateTime",
            "{type_alias}.{name}"
        );
    };
    let assert_optional_id_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{name}");
        assert_eq!(field["type"]["name"], "ID", "{type_alias}.{name}");
    };
    let assert_non_null_id_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{type_alias}.{name}");
    };
    let assert_non_null_id_list_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(
            field["type"]["ofType"]["kind"], "LIST",
            "{type_alias}.{name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{type_alias}.{name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID",
            "{type_alias}.{name}"
        );
    };

    for (type_alias, name) in [
        ("oauthConnectedApp", "grantId"),
        ("mediaRequest", "id"),
        ("mediaRequest", "libraryId"),
        ("mediaRequest", "createdByUserId"),
        ("mediaRequestRequester", "userId"),
        ("approveMediaRequest", "titleId"),
        ("user", "id"),
        ("userLibraryPermissionGrant", "libraryId"),
        ("linkedAccount", "id"),
        ("linkedAccount", "userId"),
        ("linkedAccount", "connectionId"),
        ("totpEnrollmentStart", "challengeId"),
        ("webauthnChallenge", "challengeId"),
        ("passkeySummary", "id"),
        ("activityEvent", "id"),
        ("domainEventEnvelope", "eventId"),
        ("libraryScanProgress", "sessionId"),
        ("jobRun", "id"),
        ("indexerQueryStats", "indexerId"),
        ("titleReleaseBlocklistEntry", "id"),
        ("ruleSet", "id"),
        ("postProcessingScript", "id"),
        ("postProcessingScriptRun", "id"),
        ("postProcessingScriptRun", "scriptId"),
        ("titleHistoryEvent", "id"),
        ("titleHistoryEvent", "titleId"),
        ("restoreInspect", "uploadId"),
        // 0.17.0: QueueDownloadScopePayload became a union; the scope ids are
        // non-null fields on the member payloads now.
        ("episodeScope", "episodeId"),
        ("seriesMovieScope", "seriesMovieLinkId"),
        ("collectionScope", "collectionId"),
    ] {
        assert_non_null_id_field(type_alias, name);
    }

    for (type_alias, name) in [
        ("mediaRequest", "requestedQualityProfileId"),
        ("mediaRequest", "resolvedByUserId"),
        ("mediaRequest", "createdTitleId"),
        ("mediaRequest", "approvedQualityProfileId"),
        ("activityEvent", "actorUserId"),
        ("activityEvent", "titleId"),
        ("domainEventEnvelope", "actorUserId"),
        ("domainEventEnvelope", "titleId"),
        ("libraryScanProgress", "libraryId"),
        ("postProcessingScriptRun", "titleId"),
        ("titleHistoryEvent", "episodeId"),
        ("titleHistoryEvent", "collectionId"),
        ("titleHistoryEvent", "actorUserId"),
        ("titleHistoryEvent", "clientId"),
        ("titleHistoryEvent", "importId"),
    ] {
        assert_optional_id_field(type_alias, name);
    }

    for (type_alias, name) in [
        ("titleReleaseBlocklistEntry", "episodeIds"),
        ("episodeSetScope", "episodeIds"),
        ("titleHistoryEvent", "episodeIds"),
    ] {
        assert_non_null_id_list_field(type_alias, name);
    }

    for (type_alias, name) in [
        ("login", "mfaVerifiedUntil"),
        ("totpStatus", "createdAt"),
        ("totpStatus", "lastUsedAt"),
        ("passkeySummary", "lastUsedAt"),
        ("smgUpdateNotice", "publishedAt"),
        ("queueItem", "importTransferStartedAt"),
        ("queueItem", "importTransferUpdatedAt"),
        ("queueItem", "queuedAt"),
        ("queueItem", "lastUpdatedAt"),
        ("queueItem", "importedAt"),
        ("mediaFile", "grabbedAt"),
        ("wantedItem", "lastSearchAt"),
        ("titleAcquisitionDiagnostics", "latestDecisionAt"),
        ("titleAcquisitionDiagnostics", "latestWantedSearchAt"),
        ("oauthConnectedApp", "lastUsedAt"),
        ("mediaRequest", "resolvedAt"),
        ("linkedAccount", "verifiedAt"),
        ("linkedAccount", "lastLoginAt"),
        ("autoBackupSettings", "nextRunAt"),
        ("jobScheduleInfo", "nextRunAt"),
        ("jobRun", "completedAt"),
        ("indexerQueryStats", "lastQueryAt"),
        ("indexerSearchResult", "publishedAt"),
        ("importRecord", "startedAt"),
        ("importRecord", "finishedAt"),
        ("indexerConfig", "disabledUntil"),
        ("indexerConfig", "lastErrorAt"),
        ("downloadClientConfig", "lastSeenAt"),
        ("subtitleProviderConfig", "lastErrorAt"),
        ("subtitleProviderConfig", "disabledUntil"),
        ("pluginCatalogStatus", "lastCheckedAt"),
        ("postProcessingScriptRun", "completedAt"),
    ] {
        assert_optional_datetime_field(type_alias, name);
    }

    for (type_alias, name) in [
        ("login", "expiresAt"),
        ("title", "createdAt"),
        ("collection", "createdAt"),
        ("mediaFile", "createdAt"),
        ("wantedItem", "createdAt"),
        ("wantedItem", "updatedAt"),
        ("releaseDecision", "createdAt"),
        ("pendingRelease", "addedAt"),
        ("pendingRelease", "delayUntil"),
        ("totpEnrollmentStart", "expiresAt"),
        ("passkeySummary", "createdAt"),
        ("smgUpdateNotice", "checkedAt"),
        ("oauthConnectedApp", "authorizedAt"),
        ("mediaRequest", "createdAt"),
        ("mediaRequest", "updatedAt"),
        ("mediaRequestRequester", "requestedAt"),
        ("linkedAccount", "createdAt"),
        ("linkedAccount", "updatedAt"),
        ("activityEvent", "occurredAt"),
        ("domainEventEnvelope", "occurredAt"),
        ("libraryScanProgress", "startedAt"),
        ("libraryScanProgress", "updatedAt"),
        ("jobRun", "startedAt"),
        ("titleReleaseBlocklistEntry", "attemptedAt"),
        ("importRecord", "createdAt"),
        ("indexerConfig", "createdAt"),
        ("indexerConfig", "updatedAt"),
        ("downloadClientConfig", "createdAt"),
        ("downloadClientConfig", "updatedAt"),
        ("subtitleProviderConfig", "createdAt"),
        ("subtitleProviderConfig", "updatedAt"),
        ("ruleSet", "createdAt"),
        ("ruleSet", "updatedAt"),
        ("pluginInstallation", "installedAt"),
        ("pluginInstallation", "updatedAt"),
        ("notificationChannel", "createdAt"),
        ("notificationChannel", "updatedAt"),
        ("notificationSubscription", "createdAt"),
        ("notificationSubscription", "updatedAt"),
        ("postProcessingScript", "createdAt"),
        ("postProcessingScript", "updatedAt"),
        ("postProcessingScriptRun", "startedAt"),
        ("titleHistoryEvent", "occurredAt"),
        ("titleHistoryEvent", "createdAt"),
        ("externalImportWarmup", "startedAt"),
        ("externalImportWarmup", "updatedAt"),
        ("serviceLogs", "generatedAt"),
        ("recycledItem", "recycledAt"),
        ("externalSubtitle", "downloadedAt"),
        ("externalSubtitleBlocklistEntry", "createdAt"),
        ("backupInfo", "createdAt"),
        ("backupDownloadUrl", "expiresAt"),
        ("restoreSummary", "createdAt"),
    ] {
        assert_non_null_datetime_field(type_alias, name);
    }
}

#[tokio::test]
async fn graphql_introspection_domain_event_cursors_use_long_and_id() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
            }
          }
          subscriptionRoot: __type(name: "SubscriptionRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
            }
          }
          domainEventEnvelope: __type(name: "DomainEventEnvelopePayload") {
            fields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let payload_field = |name: &str| {
        body["data"]["domainEventEnvelope"]["fields"]
            .as_array()
            .expect("DomainEventEnvelopePayload should expose fields")
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("DomainEventEnvelopePayload.{name} should exist"))
            .clone()
    };
    let sequence = payload_field("sequence");
    assert_eq!(sequence["type"]["kind"], "NON_NULL");
    assert_eq!(sequence["type"]["ofType"]["name"], "Long");
    let stream_id = payload_field("streamId");
    assert_eq!(stream_id["type"]["kind"], "SCALAR");
    assert_eq!(stream_id["type"]["name"], "ID");

    let root_arg = |root_alias: &str, field_name: &str, arg_name: &str| {
        body["data"][root_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{root_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{root_alias}.{field_name} should exist"))["args"]
            .as_array()
            .unwrap_or_else(|| panic!("{root_alias}.{field_name} should expose args"))
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .unwrap_or_else(|| panic!("{root_alias}.{field_name}.{arg_name} should exist"))
            .clone()
    };
    for arg_name in ["afterSequence", "beforeSequence"] {
        let arg = root_arg("queryRoot", "auditLog", arg_name);
        assert_eq!(arg["type"]["kind"], "SCALAR");
        assert_eq!(arg["type"]["name"], "Long");
    }
    let arg = root_arg("subscriptionRoot", "domainEventFeed", "afterSequence");
    assert_eq!(arg["type"]["kind"], "SCALAR");
    assert_eq!(arg["type"]["name"], "Long");
}

#[tokio::test]
async fn graphql_introspection_exposes_calendar_dates_as_date_scalar() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          title: __type(name: "TitlePayload") { fields { name type { kind name ofType { kind name } } } }
          movieEntity: __type(name: "MovieEntityPayload") { fields { name type { kind name ofType { kind name } } } }
          episode: __type(name: "EpisodePayload") { fields { name type { kind name ofType { kind name } } } }
          calendarEpisode: __type(name: "CalendarEpisodePayload") { fields { name type { kind name ofType { kind name } } } }
          wantedItem: __type(name: "WantedItemPayload") { fields { name type { kind name ofType { kind name } } } }
          metadataSeries: __type(name: "MetadataSeriesPayload") { fields { name type { kind name ofType { kind name } } } }
          metadataEpisode: __type(name: "MetadataEpisodePayload") { fields { name type { kind name ofType { kind name } } } }
          metadataMovie: __type(name: "MetadataMoviePayload") { fields { name type { kind name ofType { kind name } } } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let assert_optional_date_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{name}");
        assert_eq!(field["type"]["name"], "Date", "{type_alias}.{name}");
    };
    let assert_non_null_date_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(
            field["type"]["ofType"]["name"], "Date",
            "{type_alias}.{name}"
        );
    };

    for (type_alias, name) in [
        ("title", "firstAired"),
        ("episode", "airDate"),
        ("calendarEpisode", "airDate"),
        ("metadataMovie", "tmdbReleaseDate"),
    ] {
        assert_optional_date_field(type_alias, name);
    }

    for (type_alias, name) in [
        ("metadataSeries", "firstAired"),
        ("metadataEpisode", "aired"),
    ] {
        assert_non_null_date_field(type_alias, name);
    }
}

#[tokio::test]
async fn graphql_introspection_exposes_byte_counts_as_long_scalar() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          longScalar: __type(name: "Long") {
            kind
            name
          }
          title: __type(name: "TitlePayload") { fields { name type { kind name ofType { kind name } } } }
          collection: __type(name: "CollectionPayload") { fields { name type { kind name ofType { kind name } } } }
          mediaFile: __type(name: "TitleMediaFilePayload") { fields { name type { kind name ofType { kind name } } } }
          queueItem: __type(name: "DownloadQueueItemPayload") { fields { name type { kind name ofType { kind name } } } }
          releaseDecision: __type(name: "ReleaseDecisionPayload") { fields { name type { kind name ofType { kind name } } } }
          pendingRelease: __type(name: "PendingReleasePayload") { fields { name type { kind name ofType { kind name } } } }
          indexerSearchResult: __type(name: "IndexerSearchResultPayload") { fields { name type { kind name ofType { kind name } } } }
          importResult: __type(name: "ImportResultPayload") { fields { name type { kind name ofType { kind name } } } }
          pendingImportBindingFile: __type(name: "PendingImportBindingFilePreviewPayload") { fields { name type { kind name ofType { kind name } } } }
          mediaRenamePlanItem: __type(name: "MediaRenamePlanItemPayload") { fields { name type { kind name ofType { kind name } } } }
          manualImportFile: __type(name: "ManualImportFilePreviewPayload") { fields { name type { kind name ofType { kind name } } } }
          backupInfo: __type(name: "BackupInfoPayload") { fields { name type { kind name ofType { kind name } } } }
          backupRowCount: __type(name: "BackupRowCountPayload") { fields { name type { kind name ofType { kind name } } } }
          restoreSummary: __type(name: "RestoreSummaryPayload") { fields { name type { kind name ofType { kind name } } } }
          recycledItem: __type(name: "RecycledItemPayload") { fields { name type { kind name ofType { kind name } } } }
          registryPlugin: __type(name: "RegistryPluginPayload") { fields { name type { kind name ofType { kind name } } } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["longScalar"]["kind"], "SCALAR");
    assert_eq!(body["data"]["longScalar"]["name"], "Long");

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let assert_optional_long_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{name}");
        assert_eq!(field["type"]["name"], "Long", "{type_alias}.{name}");
    };
    let assert_non_null_long_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(
            field["type"]["ofType"]["name"], "Long",
            "{type_alias}.{name}"
        );
    };

    for (type_alias, name) in [
        ("title", "sizeBytes"),
        ("collection", "fileSizeBytes"),
        ("queueItem", "importTransferBytes"),
        ("queueItem", "importTransferTotalBytes"),
        ("queueItem", "sizeBytes"),
        ("releaseDecision", "releaseSizeBytes"),
        ("pendingRelease", "releaseSizeBytes"),
        ("indexerSearchResult", "sizeBytes"),
        ("mediaRenamePlanItem", "sourceSizeBytes"),
        ("registryPlugin", "bytes"),
    ] {
        assert_optional_long_field(type_alias, name);
    }

    for (type_alias, name) in [
        ("mediaFile", "sizeBytes"),
        ("pendingImportBindingFile", "sizeBytes"),
        ("manualImportFile", "sizeBytes"),
        ("backupInfo", "sizeBytes"),
        ("backupRowCount", "rowCount"),
        ("restoreSummary", "totalRows"),
        ("recycledItem", "sizeBytes"),
    ] {
        assert_non_null_long_field(type_alias, name);
    }
}

#[tokio::test]
async fn graphql_introspection_delete_rename_and_cutoff_payloads_use_id_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          deleteTitlePreviewResult: __type(name: "DeleteTitlePreviewResultPayload") { fields { name type { ...TypeRef } } }
          deleteTitles: __type(name: "DeleteTitlesPayload") { fields { name type { ...TypeRef } } }
          mediaRenamePlan: __type(name: "MediaRenamePlanPayload") { fields { name type { ...TypeRef } } }
          mediaRenamePlanItem: __type(name: "MediaRenamePlanItemPayload") { fields { name type { ...TypeRef } } }
          mediaRenameApplyItem: __type(name: "MediaRenameApplyItemPayload") { fields { name type { ...TypeRef } } }
          manualImportFile: __type(name: "ManualImportFilePreviewPayload") { fields { name type { ...TypeRef } } }
          cutoffUnmetItem: __type(name: "CutoffUnmetItemPayload") { fields { name type { ...TypeRef } } }
        }

        fragment TypeRef on __Type {
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
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let assert_non_null_id = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{type_alias}.{name}");
    };
    let assert_optional_id = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{name}");
        assert_eq!(field["type"]["name"], "ID", "{type_alias}.{name}");
    };
    let assert_non_null_id_list = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(
            field["type"]["ofType"]["kind"], "LIST",
            "{type_alias}.{name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{type_alias}.{name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID",
            "{type_alias}.{name}"
        );
    };

    assert_non_null_id("deleteTitlePreviewResult", "titleId");
    assert_non_null_id_list("deleteTitles", "acceptedTitleIds");
    for (type_alias, name) in [
        ("mediaRenamePlan", "titleId"),
        ("mediaRenamePlanItem", "collectionId"),
        ("mediaRenameApplyItem", "collectionId"),
        ("manualImportFile", "suggestedEpisodeId"),
        ("cutoffUnmetItem", "episodeId"),
    ] {
        assert_optional_id(type_alias, name);
    }
    for (type_alias, name) in [
        ("mediaRenamePlanItem", "seriesMovieLinkIds"),
        ("mediaRenameApplyItem", "seriesMovieLinkIds"),
    ] {
        assert_non_null_id_list(type_alias, name);
    }
    for name in ["titleId", "libraryId"] {
        assert_non_null_id("cutoffUnmetItem", name);
    }
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
            "WAITING",
            "STANDBY",
            "PROCESSING",
            "GRABBED",
            "SUPERSEDED",
            "EXPIRED",
            "DISMISSED",
            // Pillar A3: a candidate parked because the canonical title is
            // ambiguous. No delay timer, resolved only by grab-now / dismiss.
            "NEEDS_REVIEW",
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
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/series"),
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: Some("Traversal coverage".to_string()),
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: Some(24),
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
    };
    let title = ctx.titles.create(title).await.expect("create title");

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
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let collection = ctx
        .shows
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
        image_url: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    let episode = ctx
        .shows
        .create_episode(episode)
        .await
        .expect("create episode");

    let file_path = media_root
        .path()
        .join("Graph.Traversal.Show.S01E01.1080p.WEB-DL.mkv");
    let file_id = ctx
        .media_files
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
    ctx.media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link file to episode");

    let wanted_item = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: Some(title.facet.as_str().to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(episode.id.clone()),
        collection_id: Some(collection.id.clone()),
        series_movie_link_id: None,
        season_number: Some("1".to_string()),
        episode_number: episode.episode_number.clone(),
        media_type: "episode".to_string(),
        last_search_at: None,
        status: scryer_application::AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        current_score: Some(120),
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: "2026-03-20T00:00:00Z".to_string(),
        updated_at: "2026-03-20T00:00:00Z".to_string(),
    };
    ctx.library_state
        .upsert_acquisition_scope_state(&wanted_item)
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
    scryer_infrastructure::WantedStore::new(ctx.db.datastore())
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
    scryer_infrastructure::PendingReleaseStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    )
    .insert_pending_release(&pending_release)
    .await
    .expect("seed pending release");

    let body = gql(
        &ctx,
        r#"
        query CoreGraph($titleId: ID!, $wantedItemId: ID!, $episodeId: ID!, $mismatchTitleId: ID!) {
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
              items {
                id
                title { id }
                collection { id }
                episode { id }
                pendingReleases {
                  items {
                    id
                    status
                    title { id }
                    wantedItem { id }
                  }
                }
                releaseDecisions(limit: 10) {
                  items {
                    id
                    wantedItem { id }
                    title { id }
                  }
                }
              }
            }
            releaseDecisions(limit: 10) {
              items {
                id
                wantedItem { id }
                title { id }
              }
            }
          }
          wantedItem(id: $wantedItemId) {
            id
            title { id }
            collection { id }
            episode { id }
            pendingReleases {
              items {
                id
                status
                title { id }
                wantedItem { id }
              }
            }
            releaseDecisions(limit: 10) {
              items { id }
            }
          }
          episode(titleId: $titleId, episodeId: $episodeId) {
            id
            parentTitle { id }
            mediaFiles {
              id
              episode { id }
            }
          }
          mismatchedEpisode: episode(titleId: $mismatchTitleId, episodeId: $episodeId) {
            id
          }
        }
        "#,
        json!({
            "titleId": title.id,
            "wantedItemId": wanted_item.id,
            "episodeId": episode.id,
            "mismatchTitleId": Id::new().0,
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
    let title_wanted_item = &title_data["wantedItems"]["items"][0];
    assert_eq!(title_wanted_item["title"]["id"], title.id);
    assert_eq!(title_wanted_item["collection"]["id"], collection.id);
    assert_eq!(title_wanted_item["episode"]["id"], episode.id);
    assert_eq!(
        title_wanted_item["pendingReleases"]["items"][0]["id"],
        pending_release.id
    );
    assert_eq!(
        title_wanted_item["pendingReleases"]["items"][0]["status"],
        "WAITING"
    );
    assert_eq!(
        title_wanted_item["releaseDecisions"]["items"][0]["id"],
        decision.id
    );
    assert_eq!(
        title_data["releaseDecisions"]["items"][0]["wantedItem"]["id"],
        wanted_item.id
    );

    assert_eq!(body["data"]["wantedItem"]["title"]["id"], title.id);
    assert_eq!(
        body["data"]["wantedItem"]["collection"]["id"],
        collection.id
    );
    assert_eq!(body["data"]["wantedItem"]["episode"]["id"], episode.id);
    assert_eq!(
        body["data"]["wantedItem"]["pendingReleases"]["items"][0]["id"],
        pending_release.id
    );
    assert_eq!(
        body["data"]["wantedItem"]["releaseDecisions"]["items"][0]["id"],
        decision.id
    );
    assert_eq!(body["data"]["episode"]["id"], episode.id);
    assert_eq!(body["data"]["episode"]["parentTitle"]["id"], title.id);
    assert_eq!(body["data"]["episode"]["mediaFiles"][0]["id"], file_id);
    assert_eq!(
        body["data"]["episode"]["mediaFiles"][0]["episode"]["id"],
        episode.id
    );
    assert!(body["data"]["mismatchedEpisode"].is_null());
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

    for name in ["id", "clientId"] {
        assert_eq!(field(name)["type"]["kind"], "NON_NULL", "{name}");
        assert_eq!(field(name)["type"]["ofType"]["name"], "ID", "{name}");
    }
    for name in ["titleId", "episodeId"] {
        assert_eq!(field(name)["type"]["kind"], "SCALAR", "{name}");
        assert_eq!(field(name)["type"]["name"], "ID", "{name}");
    }

    assert_eq!(field("state")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("state")["type"]["ofType"]["name"],
        "DownloadQueueStateValue"
    );
    assert_eq!(field("importStatus")["type"]["name"], "ImportStatusValue");
    assert_eq!(
        field("importErrorCode")["type"]["name"],
        "ImportErrorCodeValue"
    );
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
    assert!(queue_state_names.contains(&"IMPORT_PENDING"));
    assert!(!queue_state_names.contains(&"IMPORTPENDING"));

    let source_kinds = body["data"]["sourceKind"]["enumValues"]
        .as_array()
        .expect("DownloadSourceKindValue should expose enum values");
    let source_kind_names: Vec<&str> = source_kinds
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        source_kind_names,
        vec!["NZB_FILE", "NZB_URL", "TORRENT_FILE", "MAGNET_URI"]
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
          queueDownloadPayload: __type(name: "QueueDownloadPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          queueDownloadConflict: __type(name: "QueueDownloadConflictPayload") {
            fields { name type { kind name ofType { kind name } } }
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
    for name in ["clientId", "importId", "commandId"] {
        assert_eq!(action_field(name)["type"]["kind"], "SCALAR", "{name}");
        assert_eq!(action_field(name)["type"]["name"], "ID", "{name}");
    }

    let queue_result_fields = body["data"]["queueDownloadPayload"]["fields"]
        .as_array()
        .expect("QueueDownloadPayload should expose fields");
    let queue_result_field = |name: &str| {
        queue_result_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("queue result field should exist")
    };
    assert_eq!(queue_result_field("titleId")["type"]["kind"], "NON_NULL");
    assert_eq!(
        queue_result_field("titleId")["type"]["ofType"]["name"],
        "ID"
    );
    assert_eq!(queue_result_field("jobId")["type"]["name"], "ID");

    let conflict_fields = body["data"]["queueDownloadConflict"]["fields"]
        .as_array()
        .expect("QueueDownloadConflictPayload should expose fields");
    let conflict_field = |name: &str| {
        conflict_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("queue conflict field should exist")
    };
    assert_eq!(conflict_field("titleId")["type"]["kind"], "NON_NULL");
    assert_eq!(conflict_field("titleId")["type"]["ofType"]["name"], "ID");
    assert_eq!(conflict_field("downloadClientId")["type"]["name"], "ID");

    let action_kind_names: Vec<&str> = body["data"]["actionKind"]["enumValues"]
        .as_array()
        .expect("DownloadQueueActionKindValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(action_kind_names.contains(&"QUEUED_MANUAL_IMPORT"));
    assert!(action_kind_names.contains(&"ASSIGNED_TRACKED_DOWNLOAD_TITLE"));
    assert!(action_kind_names.contains(&"DELETE_QUEUED"));
}

#[tokio::test]
async fn graphql_manual_import_schema_exposes_candidate_only_contract() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") { fields { name } }
          queryRoot: __type(name: "QueryRoot") { fields { name } }
          queueInput: __type(name: "QueueManualImportInput") { inputFields { name } }
          mappingInput: __type(name: "ManualImportCandidateMappingInput") { inputFields { name } }
          selectionInput: __type(name: "BeginManualImportSelectionInput") { inputFields { name } }
          selectionPayload: __type(name: "ManualImportSelectionPayload") { fields { name } }
          filePayload: __type(name: "ManualImportFilePreviewPayload") { fields { name } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field_names = |key: &str| -> Vec<&str> {
        body["data"][key]
            .as_object()
            .and_then(|value| value.get("fields").or_else(|| value.get("inputFields")))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|field| field["name"].as_str())
            .collect()
    };
    let mutation_fields = field_names("mutationRoot");
    assert!(mutation_fields.contains(&"beginManualImportSelection"));
    assert!(mutation_fields.contains(&"queueManualImport"));
    assert!(!mutation_fields.contains(&"queuePathManualImport"));

    let query_fields = field_names("queryRoot");
    assert!(!query_fields.contains(&"previewManualImport"));
    assert!(!query_fields.contains(&"previewManualImportPath"));

    assert_eq!(field_names("queueInput"), ["selectionId", "files"]);
    assert_eq!(
        field_names("mappingInput"),
        ["candidateId", "episodeId", "seriesMovieLinkId"]
    );
    assert_eq!(
        field_names("selectionInput"),
        ["clientId", "clientType", "downloadClientItemId", "titleId"]
    );
    assert!(field_names("selectionPayload").contains(&"selectionId"));
    let file_fields = field_names("filePayload");
    assert!(file_fields.contains(&"candidateId"));
    assert!(!file_fields.contains(&"filePath"));
}

#[tokio::test]
async fn graphql_manual_import_rejects_the_removed_path_contract() {
    let ctx = TestContext::new().await;
    let invalid = schema_exec(
        &ctx,
        r#"
        mutation {
          queueManualImport(input: { filePath: "/etc/passwd" files: [] }) { importId }
        }
        "#,
        None,
    )
    .await;
    assert!(
        invalid["errors"]
            .as_array()
            .is_some_and(|errors| errors.iter().any(|error| {
                error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("filePath"))
            })),
        "expected removed path field to be rejected: {invalid}"
    );

    let preview = schema_exec(
        &ctx,
        r#"
        query {
          previewManualImportPath(input: { filePath: "/etc/passwd", titleId: "title" }) {
            files { filePath }
          }
        }
        "#,
        None,
    )
    .await;
    assert!(
        preview["errors"]
            .as_array()
            .is_some_and(|errors| errors.iter().any(|error| {
                error["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("previewManualImportPath"))
            })),
        "expected removed path query to be rejected: {preview}"
    );
}

#[tokio::test]
async fn graphql_delete_download_returns_ok_and_persists_queued_delete_command() {
    let ctx = TestContext::new().await;
    let client = ctx.http_client();
    let response = client
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"
                mutation DeleteDownload($input: DeleteDownloadInput!) {
                  deleteDownload(input: $input) {
                    kind
                    commandId
                    removed
                    clientType
                    queueItem { id }
                  }
                }
            "#,
            "variables": {
                "input": {
                    "clientType": "nzbget",
                    "downloadClientItemId": "queued-delete-download-1",
                    "isHistory": true
                }
            }
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(response.status(), 200);
    let body: Value = response
        .json()
        .await
        .expect("response should be valid json");
    assert_no_errors(&body);

    let action = &body["data"]["deleteDownload"];
    let command_id = action["commandId"]
        .as_str()
        .expect("delete download should return a queued command id");
    assert_eq!(action["kind"], json!("DELETE_QUEUED"));
    assert_eq!(action["removed"], json!(false));
    assert_eq!(action["clientType"], json!("nzbget"));
    assert!(action["queueItem"].is_null());

    let queued = sqlx::query(
        "SELECT action, client_type, download_client_item_id, is_history, status
         FROM download_queue_commands
         WHERE id = ?",
    )
    .bind(command_id)
    .fetch_one(ctx.db.pool())
    .await
    .expect("queued delete command should be persisted");

    assert_eq!(
        queued
            .try_get::<String, _>("action")
            .expect("action should be readable"),
        "delete"
    );
    assert_eq!(
        queued
            .try_get::<String, _>("client_type")
            .expect("client_type should be readable"),
        "nzbget"
    );
    assert_eq!(
        queued
            .try_get::<String, _>("download_client_item_id")
            .expect("download_client_item_id should be readable"),
        "queued-delete-download-1"
    );
    assert!(
        queued
            .try_get::<i64, _>("is_history")
            .expect("is_history should be readable")
            != 0
    );
    assert_eq!(
        queued
            .try_get::<String, _>("status")
            .expect("status should be readable"),
        "queued"
    );
}

#[tokio::test]
async fn graphql_delete_download_marks_history_item_completed_after_poller_runs() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let create_config_body = gql(
        &ctx,
        r#"
        mutation CreateDownloadClientConfig($input: CreateDownloadClientConfigInput!) {
          createDownloadClientConfig(input: $input) {
            id
            clientType
            isEnabled
          }
        }
        "#,
        json!({
            "input": {
                "name": "NZBGet",
                "clientType": "nzbget",
                "config": [],
                "isEnabled": true
            }
        }),
    )
    .await;
    assert_no_errors(&create_config_body);
    assert_eq!(
        create_config_body["data"]["createDownloadClientConfig"]["clientType"],
        json!("nzbget")
    );

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"listgroups""#))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/listgroups.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"postqueue""#))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/postqueue.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"history""#))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "2.0",
            "result": {
                "History": [{
                    "NZBID": 123,
                    "Name": "Queued Delete Download",
                    "Status": "SUCCESS",
                    "HistoryTime": Utc::now().timestamp(),
                    "FileSizeMB": 10
                }]
            },
            "id": "scryer-rpc"
        })))
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_string_contains(r#""method":"editqueue""#))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "2.0",
            "result": true,
            "id": "scryer-rpc"
        })))
        .mount(&ctx.nzbget_server)
        .await;

    let delete_body = gql(
        &ctx,
        r#"
        mutation DeleteDownload($input: DeleteDownloadInput!) {
          deleteDownload(input: $input) {
            kind
            commandId
            removed
          }
        }
        "#,
        json!({
            "input": {
                "clientType": "nzbget",
                "downloadClientItemId": "123",
                "isHistory": true
            }
        }),
    )
    .await;
    assert_no_errors(&delete_body);
    assert_eq!(
        delete_body["data"]["deleteDownload"]["kind"],
        json!("DELETE_QUEUED")
    );

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_download_delete_poller(
        ctx.app.clone(),
        token.child_token(),
    ));

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let status: Option<String> = sqlx::query_scalar(
                "SELECT status
                 FROM download_queue_commands
                 WHERE client_type = 'nzbget'
                   AND download_client_item_id = '123'
                   AND is_history = 1",
            )
            .fetch_optional(ctx.db.pool())
            .await
            .expect("queued delete status should load");
            if status.as_deref() == Some("completed") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("queued delete should complete");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    let queue_body = gql(
        &ctx,
        r#"
        {
          downloadQueue(includeAllActivity: true) {
            downloadClientItemId
            state
            deleteStatus
            deleteErrorMessage
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&queue_body);

    assert!(
        queue_body["data"]["downloadQueue"]
            .as_array()
            .expect("download queue should be an array")
            .iter()
            .all(|item| item["downloadClientItemId"].as_str() != Some("123"))
    );

    let history_body = gql(
        &ctx,
        r#"
        {
                    downloadHistory(limit: 100, offset: 0, filters: [ALL]) {
            items {
              downloadClientItemId
              state
              deleteStatus
              deleteErrorMessage
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&history_body);

    let item = history_body["data"]["downloadHistory"]["items"]
        .as_array()
        .expect("download history should be an array")
        .iter()
        .find(|item| item["downloadClientItemId"].as_str() == Some("123"))
        .expect("history item should remain visible in history");

    assert_eq!(item["state"], json!("COMPLETED"));
    assert_eq!(item["deleteStatus"], json!("COMPLETED"));
    assert!(item["deleteErrorMessage"].is_null());
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
          convergenceState: __type(name: "ConvergenceStateValue") {
            enumValues { name }
          }
          recencyLane: __type(name: "RecencyLaneValue") {
            enumValues { name }
          }
          wantedKind: __type(name: "WantedKindValue") {
            enumValues { name }
          }
          wantedSearchPhase: __type(name: "WantedSearchPhaseValue") { name }
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
    // Cadence display (searchPhase) is replaced by convergence + recency.
    assert!(fields.iter().all(|field| field["name"] != "searchPhase"));
    assert!(body["data"]["wantedSearchPhase"].is_null());
    assert_eq!(field("convergenceState")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("convergenceState")["type"]["ofType"]["name"],
        "ConvergenceStateValue"
    );
    assert_eq!(field("recencyLane")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("recencyLane")["type"]["ofType"]["name"],
        "RecencyLaneValue"
    );
    assert_eq!(field("indexersCovered")["type"]["ofType"]["name"], "Int");
    assert_eq!(field("indexersRouted")["type"]["ofType"]["name"], "Int");
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
        vec!["WANTED", "GRABBED", "PAUSED", "COMPLETED"]
    );

    let media_type_names: Vec<&str> = body["data"]["wantedMediaType"]["enumValues"]
        .as_array()
        .expect("WantedMediaTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(media_type_names, vec!["MOVIE", "EPISODE", "SERIES_MOVIE"]);

    let convergence_names: Vec<&str> = body["data"]["convergenceState"]["enumValues"]
        .as_array()
        .expect("ConvergenceStateValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        convergence_names,
        vec!["QUEUED", "SEARCHING", "CONVERGED", "DEFERRED"]
    );

    let recency_names: Vec<&str> = body["data"]["recencyLane"]["enumValues"]
        .as_array()
        .expect("RecencyLaneValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(recency_names, vec!["HOT", "COLD"]);

    let wanted_kind_names: Vec<&str> = body["data"]["wantedKind"]["enumValues"]
        .as_array()
        .expect("WantedKindValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(wanted_kind_names, vec!["MISSING", "CUTOFF_UPGRADE"]);
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
            "PENDING",
            "RUNNING",
            "PROCESSING",
            "COMPLETED",
            "FAILED",
            "SKIPPED"
        ]
    );

    let import_type_names: Vec<&str> = body["data"]["importType"]["enumValues"]
        .as_array()
        .expect("ImportTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(import_type_names.contains(&"SERIES_DOWNLOAD"));
    assert!(import_type_names.contains(&"RENAME_IO_FAILED"));

    let import_decision_names: Vec<&str> = body["data"]["importDecision"]["enumValues"]
        .as_array()
        .expect("ImportDecisionValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        import_decision_names,
        vec![
            "IMPORTED",
            "REJECTED",
            "SKIPPED",
            "CONFLICT",
            "UNMATCHED",
            "FAILED"
        ]
    );

    let import_skip_reason_names: Vec<&str> = body["data"]["importSkipReason"]["enumValues"]
        .as_array()
        .expect("ImportSkipReasonValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(import_skip_reason_names.contains(&"PASSWORD_REQUIRED"));
    assert!(import_skip_reason_names.contains(&"POST_DOWNLOAD_RULE_BLOCKED"));
    assert!(import_skip_reason_names.contains(&"UNPARSEABLE_EPISODE"));
}

#[tokio::test]
async fn graphql_introspection_import_payloads_use_id_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          importRecord: __type(name: "ImportRecordPayload") { fields { name type { ...TypeRef } } }
          importResult: __type(name: "ImportResultPayload") { fields { name type { ...TypeRef } } }
          pendingImportItem: __type(name: "PendingImportItemPayload") { fields { name type { ...TypeRef } } }
          pendingImportBindingFile: __type(name: "PendingImportBindingFilePreviewPayload") { fields { name type { ...TypeRef } } }
          ignorePendingImportPayload: __type(name: "IgnorePendingImportPayload") { fields { name type { ...TypeRef } } }
        }

        fragment TypeRef on __Type {
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
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let assert_non_null_id_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{type_alias}.{name}");
    };
    let assert_optional_id_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{name}");
        assert_eq!(field["type"]["name"], "ID", "{type_alias}.{name}");
    };
    let assert_non_null_id_list_field = |type_alias: &str, name: &str| {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(
            field["type"]["ofType"]["kind"], "LIST",
            "{type_alias}.{name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{type_alias}.{name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID",
            "{type_alias}.{name}"
        );
    };

    assert_non_null_id_field("importRecord", "id");
    assert_optional_id_field("importRecord", "titleId");
    assert_non_null_id_field("importResult", "importId");
    assert_optional_id_field("importResult", "titleId");
    for name in ["id", "libraryId"] {
        assert_non_null_id_field("pendingImportItem", name);
    }
    assert_optional_id_field("pendingImportItem", "titleId");
    assert_non_null_id_list_field("pendingImportBindingFile", "suggestedEpisodeIds");
    assert_non_null_id_field("ignorePendingImportPayload", "id");
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
    assert!(activity_kind_names.contains(&"TITLE_UPDATED"));
    assert!(activity_kind_names.contains(&"METADATA_HYDRATION_COMPLETED"));
    assert!(activity_kind_names.contains(&"IMPORT_REJECTED"));

    let activity_severity_names: Vec<&str> = body["data"]["activitySeverity"]["enumValues"]
        .as_array()
        .expect("ActivitySeverityValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        activity_severity_names,
        vec!["INFO", "SUCCESS", "WARNING", "ERROR"]
    );

    let activity_channel_names: Vec<&str> = body["data"]["activityChannel"]["enumValues"]
        .as_array()
        .expect("ActivityChannelValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(activity_channel_names, vec!["WEB_UI", "TOAST"]);
}
