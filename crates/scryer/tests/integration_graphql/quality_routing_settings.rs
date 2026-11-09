use super::*;

#[tokio::test]
async fn graphql_dolby_vision_edits_survive_save_and_read() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    for allowed in [false, true, false, true] {
        let update = gql(
            &ctx,
            r#"
            mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
              saveQualityProfileSettings(input: $input) {
                profiles { id criteria { dolbyVisionAllowed detectedHdrAllowed } }
              }
            }
            "#,
            json!({
                "input": {
                    "profiles": [{
                        "id": "dv-diagnostic",
                        "name": "DV Diagnostic",
                        "criteria": {
                            "qualityTiers": ["1080P"],
                            "archivalQuality": "1080P",
                            "allowUnknownQuality": false,
                            "sourceAllowlist": [],
                            "sourceBlocklist": [],
                            "videoCodecAllowlist": [],
                            "videoCodecBlocklist": [],
                            "audioCodecAllowlist": [],
                            "audioCodecBlocklist": [],
                            "dolbyVisionAllowed": allowed,
                            "detectedHdrAllowed": true,
                            "preferRemux": false,
                            "allowBdDisk": true,
                            "allowUpgrades": true,
                            "scoringOverrides": {}
                        }
                    }],
                    "globalProfileId": "dv-diagnostic",
                    "categorySelections": [],
                    "categoryPersonaSelections": [],
                    "replaceExisting": true
                }
            }),
        )
        .await;
        assert_no_errors(&update);
        let saved = &update["data"]["saveQualityProfileSettings"]["profiles"][0];
        assert_eq!(saved["criteria"]["dolbyVisionAllowed"], allowed);
        assert_eq!(saved["criteria"]["detectedHdrAllowed"], true);

        let read = gql(
            &ctx,
            "{ qualityProfileSettings { profiles { id criteria { dolbyVisionAllowed detectedHdrAllowed } } } }",
            json!({}),
        )
        .await;
        assert_no_errors(&read);
        assert_eq!(
            &read["data"]["qualityProfileSettings"]["profiles"][0],
            saved
        );
    }
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
            globalScoringPersona
            profiles {
              id
              name
              criteria {
                qualityTiers
              }
            }
            categorySelections {
              scope
              overrideProfileId
              effectiveProfileId
              inheritsGlobal
            }
            categoryPersonaSelections {
              scope
              overridePersona
              effectivePersona
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
                  "dolbyVisionAllowed": true,
                  "detectedHdrAllowed": true,
                  "preferRemux": false,
                  "allowBdDisk": true,
                  "allowUpgrades": true,
                  "scoringOverrides": {},
                  "cutoffTier": null,
                  "minScoreToGrab": null
                }
              }
            ],
            "globalProfileId": "custom-audio",
            "globalScoringPersona": "AUDIOPHILE",
            "categorySelections": [
              {
                "scope": "MOVIE",
                "profileId": "custom-audio",
                "inheritGlobal": false
              },
              {
                "scope": "SERIES",
                "profileId": null,
                "inheritGlobal": true
              }
            ],
            "categoryPersonaSelections": [
              {
                "scope": "ANIME",
                "persona": "COMPATIBLE",
                "inheritGlobal": false
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
        update["data"]["saveQualityProfileSettings"]["globalScoringPersona"],
        "AUDIOPHILE"
    );
    let anime_persona_selection =
        update["data"]["saveQualityProfileSettings"]["categoryPersonaSelections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|selection| selection["scope"] == "ANIME")
            .unwrap();
    assert_eq!(anime_persona_selection["overridePersona"], "COMPATIBLE");
    assert_eq!(anime_persona_selection["effectivePersona"], "COMPATIBLE");
    assert_eq!(anime_persona_selection["inheritsGlobal"], false);

    let read = gql(
        &ctx,
        r#"
        query QualityProfileSettings {
          qualityProfileSettings {
            globalProfileId
            globalScoringPersona
            profiles {
              id
              criteria {
                qualityTiers
              }
            }
            categorySelections {
              scope
              overrideProfileId
              effectiveProfileId
              inheritsGlobal
            }
            categoryPersonaSelections {
              scope
              overridePersona
              effectivePersona
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
    assert_eq!(settings["globalScoringPersona"], "AUDIOPHILE");
    let movie_selection = settings["categorySelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|selection| selection["scope"] == "MOVIE")
        .unwrap();
    assert_eq!(movie_selection["overrideProfileId"], "custom-audio");
    assert_eq!(movie_selection["inheritsGlobal"], false);

    let anime_persona_selection = settings["categoryPersonaSelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|selection| selection["scope"] == "ANIME")
        .unwrap();
    assert_eq!(anime_persona_selection["overridePersona"], "COMPATIBLE");
    assert_eq!(anime_persona_selection["effectivePersona"], "COMPATIBLE");
    assert_eq!(anime_persona_selection["inheritsGlobal"], false);
}

#[tokio::test]
async fn graphql_global_quality_profile_preserves_null_and_omitted_patch_semantics() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let mutation = r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            globalProfileId
            profiles { id }
          }
        }
    "#;
    let profile = |id: &str, name: &str, tier: &str| {
        json!({
          "id": id,
          "name": name,
          "criteria": {
            "qualityTiers": [tier],
            "archivalQuality": tier,
            "allowUnknownQuality": false,
            "sourceAllowlist": [],
            "sourceBlocklist": [],
            "videoCodecAllowlist": [],
            "videoCodecBlocklist": [],
            "audioCodecAllowlist": [],
            "audioCodecBlocklist": [],
            "dolbyVisionAllowed": true,
            "detectedHdrAllowed": true,
            "preferRemux": false,
            "allowBdDisk": true,
            "allowUpgrades": true,
            "scoringOverrides": {},
            "cutoffTier": null,
            "minScoreToGrab": null
          }
        })
    };

    let set = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "profiles": [
              profile("4k", "4K", "2160P"),
              profile("1080p", "1080P", "1080P")
            ],
            "globalProfileId": "4k",
            "categorySelections": [],
            "categoryPersonaSelections": [],
            "replaceExisting": true
          }
        }),
    )
    .await;
    assert_no_errors(&set);
    assert_eq!(
        set["data"]["saveQualityProfileSettings"]["globalProfileId"],
        "4k"
    );

    let preserve_omitted = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "profiles": [],
            "categorySelections": [],
            "categoryPersonaSelections": [],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&preserve_omitted);
    assert_eq!(
        preserve_omitted["data"]["saveQualityProfileSettings"]["globalProfileId"], "4k",
        "omitting the field must preserve the stored global profile"
    );

    let preserve_null = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "profiles": [],
            "globalProfileId": null,
            "globalScoringPersona": "EFFICIENT",
            "categorySelections": [],
            "categoryPersonaSelections": [],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&preserve_null);
    assert_eq!(
        preserve_null["data"]["saveQualityProfileSettings"]["globalProfileId"], "4k",
        "an explicit null in a partial save must preserve the stored global profile"
    );

    let reconcile = gql(
        &ctx,
        mutation,
        json!({
          "input": {
            "profiles": [profile("1080p", "1080P", "1080P")],
            "globalProfileId": null,
            "categorySelections": [],
            "categoryPersonaSelections": [],
            "replaceExisting": true
          }
        }),
    )
    .await;
    assert_no_errors(&reconcile);
    assert_eq!(
        reconcile["data"]["saveQualityProfileSettings"]["globalProfileId"], "1080p",
        "catalog replacement must reconcile a global profile removed by the replacement"
    );
    assert_eq!(
        reconcile["data"]["saveQualityProfileSettings"]["profiles"],
        json!([{ "id": "1080p" }])
    );
}

#[tokio::test]
async fn graphql_quality_profile_settings_updates_category_persona_selection_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let seed = gql(
        &ctx,
        r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            profiles {
              id
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
                  "dolbyVisionAllowed": true,
                  "detectedHdrAllowed": true,
                  "preferRemux": false,
                  "allowBdDisk": true,
                  "allowUpgrades": true,
                  "scoringOverrides": {},
                  "cutoffTier": null,
                  "minScoreToGrab": null
                }
              }
            ],
            "globalProfileId": null,
            "globalScoringPersona": "BALANCED",
            "categorySelections": [],
            "categoryPersonaSelections": [],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&seed);

    let update = gql(
        &ctx,
        r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            globalScoringPersona
            profiles {
              id
            }
            categoryPersonaSelections {
              scope
              overridePersona
              effectivePersona
              inheritsGlobal
            }
          }
        }
        "#,
        json!({
          "input": {
            "profiles": [],
            "globalProfileId": null,
            "globalScoringPersona": "BALANCED",
            "categorySelections": [],
            "categoryPersonaSelections": [
              {
                "scope": "ANIME",
                "persona": "COMPATIBLE",
                "inheritGlobal": false
              }
            ],
            "replaceExisting": false
          }
        }),
    )
    .await;
    assert_no_errors(&update);

    assert_eq!(
        update["data"]["saveQualityProfileSettings"]["globalScoringPersona"],
        "BALANCED"
    );
    let anime_override = update["data"]["saveQualityProfileSettings"]["categoryPersonaSelections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["scope"] == "ANIME")
        .unwrap();
    assert_eq!(anime_override["overridePersona"], "COMPATIBLE");
    assert_eq!(anime_override["effectivePersona"], "COMPATIBLE");
    assert_eq!(anime_override["inheritsGlobal"], false);
}

#[tokio::test]
async fn graphql_introspection_exposes_scoring_persona_values() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          scoringPersona: __type(name: "ScoringPersonaValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let names: Vec<&str> = body["data"]["scoringPersona"]["enumValues"]
        .as_array()
        .expect("ScoringPersonaValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec!["BALANCED", "AUDIOPHILE", "EFFICIENT", "COMPATIBLE"]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_plugin_config_field_metadata_enums() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          fieldPayload: __type(name: "PluginConfigFieldPayload") {
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
          fieldType: __type(name: "PluginConfigFieldTypeValue") {
            enumValues { name }
          }
          valueSource: __type(name: "PluginConfigValueSourceValue") {
            enumValues { name }
          }
          fieldRole: __type(name: "PluginConfigFieldRoleValue") {
            enumValues { name }
          }
          conditionOp: __type(name: "PluginConditionOpValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["fieldPayload"]["fields"]
        .as_array()
        .expect("PluginConfigFieldPayload should expose fields");
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("field should exist")
    };

    assert_eq!(field("fieldType")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("fieldType")["type"]["ofType"]["name"],
        "PluginConfigFieldTypeValue"
    );
    assert_eq!(field("valueSource")["type"]["kind"], "NON_NULL");
    assert_eq!(
        field("valueSource")["type"]["ofType"]["name"],
        "PluginConfigValueSourceValue"
    );
    assert_eq!(field("role")["type"]["kind"], "ENUM");
    assert_eq!(field("role")["type"]["name"], "PluginConfigFieldRoleValue");
    // Conditions are nullable objects: absent means "always shown" / "never
    // additionally required", which is what every field declared before the
    // condition vocabulary existed deserializes to.
    assert_eq!(field("visibleWhen")["type"]["kind"], "OBJECT");
    assert_eq!(
        field("visibleWhen")["type"]["name"],
        "PluginFieldConditionPayload"
    );
    assert_eq!(field("requiredWhen")["type"]["kind"], "OBJECT");
    assert_eq!(
        field("requiredWhen")["type"]["name"],
        "PluginFieldConditionPayload"
    );
    assert_eq!(field("advanced")["type"]["kind"], "NON_NULL");

    let enum_names = |path: &str| -> Vec<&str> {
        body["data"][path]["enumValues"]
            .as_array()
            .expect("enum values should exist")
            .iter()
            .filter_map(|value| value["name"].as_str())
            .collect()
    };
    assert_eq!(
        enum_names("fieldType"),
        vec![
            "STRING",
            "PASSWORD",
            "MULTILINE",
            "BOOL",
            "SELECT",
            "FILTERED_SELECT",
            "NUMBER",
            "PATH",
            "TAG",
            "INFO"
        ]
    );
    assert_eq!(enum_names("valueSource"), vec!["USER", "HOST_BINDING"]);
    assert_eq!(
        enum_names("fieldRole"),
        vec!["CONNECTION_URL", "LOGIN_CHALLENGE_ANSWER"]
    );
    assert_eq!(
        enum_names("conditionOp"),
        vec!["EQ", "NE", "IN", "NOT_IN", "NON_EMPTY"]
    );
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
            "scope": "MOVIE",
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
            "scope": "ANIME",
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
          downloadClientRouting(scope: MOVIE) {
            clientId
            category
            recentQueuePriority
          }
          indexerRouting(scope: ANIME) {
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

/// The same-tier upgrade delta is a quality-profile field: it defaults to 120,
/// round-trips a set value, survives a save that omits it, returns to the
/// default when the default is sent, and refuses a negative value with a code.
#[tokio::test]
async fn graphql_quality_profile_min_upgrade_score_delta_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let save = r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            profiles { id criteria { minUpgradeScoreDelta } }
          }
        }
        "#;
    let input = |delta: Option<i64>| {
        let mut criteria = json!({
            "qualityTiers": ["1080P"],
            "archivalQuality": "1080P",
            "allowUnknownQuality": false,
            "sourceAllowlist": [],
            "sourceBlocklist": [],
            "videoCodecAllowlist": [],
            "videoCodecBlocklist": [],
            "audioCodecAllowlist": [],
            "audioCodecBlocklist": [],
            "dolbyVisionAllowed": true,
            "detectedHdrAllowed": true,
            "preferRemux": false,
            "allowBdDisk": true,
            "allowUpgrades": true,
            "scoringOverrides": {},
            "cutoffTier": null,
            "minScoreToGrab": null
        });
        if let Some(delta) = delta {
            criteria["minUpgradeScoreDelta"] = json!(delta);
        }
        json!({
            "input": {
                "profiles": [{ "id": "delta-fixture", "name": "Delta Fixture", "criteria": criteria }],
                "categorySelections": [],
                "categoryPersonaSelections": [],
                "replaceExisting": false
            }
        })
    };
    let delta_of = |response: &serde_json::Value| {
        response["data"]["saveQualityProfileSettings"]["profiles"]
            .as_array()
            .expect("profiles list")
            .iter()
            .find(|profile| profile["id"] == "delta-fixture")
            .expect("saved profile")["criteria"]["minUpgradeScoreDelta"]
            .clone()
    };

    let unset = gql(&ctx, save, input(None)).await;
    assert_no_errors(&unset);
    assert_eq!(delta_of(&unset), 120, "an unset delta reads as the default");

    let set = gql(&ctx, save, input(Some(5))).await;
    assert_no_errors(&set);
    assert_eq!(delta_of(&set), 5);

    let omitted = gql(&ctx, save, input(None)).await;
    assert_no_errors(&omitted);
    assert_eq!(
        delta_of(&omitted),
        5,
        "omitting the field keeps the stored value"
    );

    let read = gql(
        &ctx,
        "{ qualityProfileSettings { profiles { id criteria { minUpgradeScoreDelta } } } }",
        json!({}),
    )
    .await;
    assert_no_errors(&read);
    let stored = read["data"]["qualityProfileSettings"]["profiles"]
        .as_array()
        .expect("profiles list")
        .iter()
        .find(|profile| profile["id"] == "delta-fixture")
        .expect("stored profile")["criteria"]["minUpgradeScoreDelta"]
        .clone();
    assert_eq!(stored, 5);

    let reset = gql(&ctx, save, input(Some(120))).await;
    assert_no_errors(&reset);
    assert_eq!(delta_of(&reset), 120);
    let scoring_config: String =
        sqlx::query_scalar("SELECT scoring_config FROM quality_profiles WHERE id = ?")
            .bind("delta-fixture")
            .fetch_one(ctx.db.pool())
            .await
            .expect("stored scoring config");
    let scoring_config: serde_json::Value =
        serde_json::from_str(&scoring_config).expect("scoring config is JSON");
    assert!(
        scoring_config.get("min_upgrade_score_delta").is_none(),
        "sending the default stores the profile as using the default: {scoring_config}"
    );

    let negative = gql(&ctx, save, input(Some(-1))).await;
    assert_eq!(
        negative["errors"][0]["extensions"]["reason"], "INTERFACE_QUALITY_PROFILE_VALUE_INVALID",
        "a negative delta is refused with a code: {negative}"
    );
}

/// The decision order is a quality-profile field: it defaults to tier,
/// revision, then the language steps; round-trips any other order, an empty
/// one included; survives a save that omits it; refuses a repeated step with
/// a code; and is stored as absent when the revision switch implies it, so an
/// existing profile keeps its fingerprint.
#[tokio::test]
async fn graphql_quality_profile_decision_pipeline_round_trip() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let save = r#"
        mutation SaveQualityProfileSettings($input: SaveQualityProfileSettingsInput!) {
          saveQualityProfileSettings(input: $input) {
            profiles { id criteria { decisionPipeline ignoreProperRepack } }
          }
        }
        "#;
    let input = |pipeline: Option<serde_json::Value>| {
        let mut criteria = json!({
            "qualityTiers": ["1080P"],
            "archivalQuality": "1080P",
            "allowUnknownQuality": false,
            "sourceAllowlist": [],
            "sourceBlocklist": [],
            "videoCodecAllowlist": [],
            "videoCodecBlocklist": [],
            "audioCodecAllowlist": [],
            "audioCodecBlocklist": [],
            "dolbyVisionAllowed": true,
            "detectedHdrAllowed": true,
            "preferRemux": false,
            "allowBdDisk": true,
            "allowUpgrades": true,
            "scoringOverrides": {},
            "cutoffTier": null,
            "minScoreToGrab": null
        });
        if let Some(pipeline) = pipeline {
            criteria["decisionPipeline"] = pipeline;
        }
        json!({
            "input": {
                "profiles": [{ "id": "order-fixture", "name": "Order Fixture", "criteria": criteria }],
                "categorySelections": [],
                "categoryPersonaSelections": [],
                "replaceExisting": false
            }
        })
    };
    let criteria_of = |response: &serde_json::Value| {
        response["data"]["saveQualityProfileSettings"]["profiles"]
            .as_array()
            .expect("profiles list")
            .iter()
            .find(|profile| profile["id"] == "order-fixture")
            .expect("saved profile")["criteria"]
            .clone()
    };
    let stored_config = || async {
        let raw: String =
            sqlx::query_scalar("SELECT scoring_config FROM quality_profiles WHERE id = ?")
                .bind("order-fixture")
                .fetch_one(ctx.db.pool())
                .await
                .expect("stored scoring config");
        serde_json::from_str::<serde_json::Value>(&raw).expect("scoring config is JSON")
    };
    let default_order = json!([
        "QUALITY_TIER",
        "REVISION",
        "PREFERRED_LANGUAGE",
        "OPTIONAL_LANGUAGES"
    ]);

    let unset = gql(&ctx, save, input(None)).await;
    assert_no_errors(&unset);
    assert_eq!(
        criteria_of(&unset)["decisionPipeline"],
        default_order,
        "an unset order reads as the default"
    );
    assert!(stored_config().await.get("decision_pipeline").is_none());

    let language_ahead = json!(["PREFERRED_LANGUAGE", "QUALITY_TIER", "REVISION"]);
    let set = gql(&ctx, save, input(Some(language_ahead.clone()))).await;
    assert_no_errors(&set);
    assert_eq!(criteria_of(&set)["decisionPipeline"], language_ahead);
    assert_eq!(criteria_of(&set)["ignoreProperRepack"], false);
    assert_eq!(
        stored_config().await["decision_pipeline"],
        json!(["preferred_language", "quality_tier", "revision"])
    );

    let omitted = gql(&ctx, save, input(None)).await;
    assert_no_errors(&omitted);
    assert_eq!(
        criteria_of(&omitted)["decisionPipeline"],
        language_ahead,
        "omitting the field keeps the stored value"
    );

    let score_only = gql(&ctx, save, input(Some(json!([])))).await;
    assert_no_errors(&score_only);
    assert_eq!(criteria_of(&score_only)["decisionPipeline"], json!([]));
    assert_eq!(
        criteria_of(&score_only)["ignoreProperRepack"],
        true,
        "an order without the revision step ignores PROPER and REPACK"
    );
    assert_eq!(stored_config().await["decision_pipeline"], json!([]));

    let repeated = gql(
        &ctx,
        save,
        input(Some(json!(["QUALITY_TIER", "REVISION", "QUALITY_TIER"]))),
    )
    .await;
    assert_eq!(
        repeated["errors"][0]["extensions"]["reason"],
        "INTERFACE_QUALITY_PROFILE_DECISION_PIPELINE_DUPLICATE_GATE",
        "a repeated step is refused with a code: {repeated}"
    );

    let reset = gql(&ctx, save, input(Some(default_order.clone()))).await;
    assert_no_errors(&reset);
    assert_eq!(criteria_of(&reset)["decisionPipeline"], default_order);
    let config = stored_config().await;
    assert!(
        config.get("decision_pipeline").is_none(),
        "the default order is stored as absent: {config}"
    );
}
