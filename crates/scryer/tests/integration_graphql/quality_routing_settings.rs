use super::*;

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
            "TAG"
        ]
    );
    assert_eq!(enum_names("valueSource"), vec!["USER", "HOST_BINDING"]);
    assert_eq!(enum_names("fieldRole"), vec!["CONNECTION_URL"]);
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
