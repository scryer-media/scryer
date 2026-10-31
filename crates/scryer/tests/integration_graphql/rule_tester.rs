use super::*;

const TEST_RULE_SET: &str = r#"
mutation($input: TestRuleSetInput!) {
  testRuleSet(input: $input) {
    score
    releaseName
    mediaFileId
    listing {
      protocol
      publishedAt
      ageDays
      thumbsUp
      thumbsDown
      isPasswordProtected
      indexerLanguages
      extra
      capturedAt
    }
    draftContribution { score matched }
  }
}
"#;

const LISTING_RULES: &str = r#"import rego.v1
score_entry["listing_liked"] := 7 if { input.release.thumbs_up >= 10 }
score_entry["listing_freeleech"] := 11 if { input.release.extra.freeleech == true }
score_entry["listing_known_age"] := 13 if { input.release.age_days >= 0 }
"#;

const RELEASE_NAME: &str = "Synthetic.Feature.2024.1080p.WEB-DL.DDP5.1-SYNGRP";

fn draft() -> Value {
    json!({
        "name": "Listing draft",
        "description": "",
        "regoSource": LISTING_RULES,
        "enabled": true,
        "priority": 0,
        "appliedFacets": ["movie"],
    })
}

async fn synthetic_movie(ctx: &TestContext) -> Title {
    seed_typed_settings_definitions(ctx).await;
    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default settings actor");
    let mut profile = scryer_application::builtin_1080p_profile();
    profile.id = "synthetic-1080p".to_string();
    profile.name = "Synthetic 1080p".to_string();
    ctx.app
        .save_quality_profile_settings(
            &actor,
            scryer_application::SaveQualityProfileSettings {
                profiles: vec![profile],
                replace_existing: false,
                global_profile_id: None,
                category_selections: vec![],
                global_scoring_persona: None,
                category_persona_selections: vec![],
            },
        )
        .await
        .expect("seed quality profile");
    create_catalog_title(
        ctx,
        "Synthetic Feature",
        MediaFacet::Movie,
        vec![],
        vec!["scryer:quality-profile:synthetic-1080p".to_string()],
        true,
    )
    .await
}

#[tokio::test]
async fn graphql_rule_tester_protocol_reaches_scoring_and_echoes_unknown() {
    let ctx = TestContext::new().await;
    let title = synthetic_movie(&ctx).await;
    let mut rule = draft();
    rule["regoSource"] = json!(
        r#"import rego.v1
score_entry["usenet"] := 10 if { input.release.protocol == "usenet" }
score_entry["torrent"] := 20 if { input.release.protocol == "torrent" }
score_entry["http"] := 30 if { input.release.protocol == "http" }
score_entry["unknown"] := 1 if { input.release.protocol == null }
"#
    );
    for (supplied, expected, score) in [
        (json!("usenet"), json!("usenet"), 10),
        (json!("torrent"), json!("torrent"), 20),
        (json!("http"), json!("http"), 30),
        (json!(null), json!(null), 1),
        (json!("ftp"), json!(null), 1),
    ] {
        let body = gql(&ctx, TEST_RULE_SET, json!({"input":{"draft":rule,"titleId":title.id,"releaseName":RELEASE_NAME,"listing":{"protocol":supplied}}})).await;
        assert_no_errors(&body);
        let result = &body["data"]["testRuleSet"];
        assert_eq!(result["listing"]["protocol"], expected);
        assert_eq!(result["draftContribution"]["score"], score);
    }
}

#[tokio::test]
async fn graphql_rule_tester_scores_a_release_name_with_listing_facts() {
    let ctx = TestContext::new().await;
    let title = synthetic_movie(&ctx).await;

    let body = gql(
        &ctx,
        TEST_RULE_SET,
        json!({
            "input": {
                "draft": draft(),
                "titleId": title.id,
                "releaseName": RELEASE_NAME,
                "listing": {
                    "publishedAt": "2020-01-01T00:00:00Z",
                    "thumbsUp": 12,
                    "indexerLanguages": ["en"],
                    "extra": { "freeleech": true, "nested": { "dropped": 1 } },
                },
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let result = &body["data"]["testRuleSet"];
    assert_eq!(result["draftContribution"]["score"], 31, "{result}");
    assert_eq!(result["releaseName"], RELEASE_NAME);
    assert!(result["mediaFileId"].is_null());
    let listing = &result["listing"];
    assert_eq!(listing["thumbsUp"], 12);
    assert!(listing["thumbsDown"].is_null());
    assert_eq!(listing["indexerLanguages"], json!(["en"]));
    assert_eq!(listing["extra"], json!({ "freeleech": true }));
    assert!(listing["ageDays"].as_i64().is_some_and(|days| days > 365));
    assert!(listing["capturedAt"].is_string());

    // Without listing inputs every fact is unknown to the rules.
    let body = gql(
        &ctx,
        TEST_RULE_SET,
        json!({
            "input": {
                "draft": draft(),
                "titleId": title.id,
                "releaseName": RELEASE_NAME,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let result = &body["data"]["testRuleSet"];
    assert_eq!(result["draftContribution"]["score"], 0, "{result}");
    assert!(result["listing"]["ageDays"].is_null());
    assert!(result["listing"]["thumbsUp"].is_null());
}

#[tokio::test]
async fn graphql_rule_tester_scores_a_stored_file_from_its_frozen_snapshot() {
    let ctx = TestContext::new().await;
    let title = synthetic_movie(&ctx).await;
    let snapshot = json!({
        "v": 1,
        "published_at": "2025-01-01T12:00:00Z",
        "thumbs_up": 12,
        "is_password_protected": false,
        "indexer_languages": ["en"],
        "extra": { "freeleech": true },
        "captured_at": "2025-06-01T12:00:00Z",
    });
    let with_snapshot = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/synthetic/library/Synthetic Feature (2024)/feature.mkv".into(),
            size_bytes: 6_000_000_000,
            grabbed_release_title: Some(RELEASE_NAME.into()),
            release_listing_json: Some(snapshot.to_string()),
            ..Default::default()
        })
        .await
        .expect("insert stored file");
    let without_snapshot = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/synthetic/library/Synthetic Feature (2024)/scanned.mkv".into(),
            size_bytes: 6_000_000_000,
            ..Default::default()
        })
        .await
        .expect("insert scanned file");

    let body = gql(
        &ctx,
        TEST_RULE_SET,
        json!({
            "input": {
                "draft": draft(),
                "titleId": title.id,
                "mediaFileId": with_snapshot,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let result = &body["data"]["testRuleSet"];
    assert_eq!(result["draftContribution"]["score"], 31, "{result}");
    assert_eq!(result["releaseName"], RELEASE_NAME);
    assert_eq!(result["mediaFileId"], with_snapshot.as_str());
    // Aged at the grab, not today: 151 days, however long ago that was.
    assert_eq!(result["listing"]["ageDays"], 151);
    assert_eq!(result["listing"]["capturedAt"], "2025-06-01T12:00:00+00:00");

    let body = gql(
        &ctx,
        TEST_RULE_SET,
        json!({
            "input": {
                "draft": draft(),
                "titleId": title.id,
                "mediaFileId": without_snapshot,
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let result = &body["data"]["testRuleSet"];
    assert!(result["listing"].is_null(), "{result}");
    assert_eq!(result["releaseName"], "scanned.mkv");
    assert_eq!(result["draftContribution"]["score"], 0);

    for input in [
        json!({ "draft": draft(), "titleId": title.id }),
        json!({
            "draft": draft(),
            "titleId": title.id,
            "releaseName": RELEASE_NAME,
            "mediaFileId": with_snapshot,
        }),
        json!({
            "draft": draft(),
            "titleId": title.id,
            "mediaFileId": with_snapshot,
            "listing": { "thumbsUp": 1 },
        }),
        json!({
            "draft": draft(),
            "titleId": title.id,
            "releaseName": RELEASE_NAME,
            "listing": { "extra": [1, 2] },
        }),
    ] {
        let body = gql(&ctx, TEST_RULE_SET, json!({ "input": input })).await;
        assert!(
            body["errors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty()),
            "{input}: {body}"
        );
    }
}
