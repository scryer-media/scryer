//! The rule editor's tester reads the same frozen listing facts every scoring
//! lane reads: supplied listing inputs for a release name, and a stored file's
//! own grab-time snapshot.

use super::*;
use crate::rules::workflow::tests::TestRuleSetRepo;
use crate::{RuleSetTestDraft, RuleSetTestListingInput, RuleSetTestRequest};
use chrono::TimeZone;

const RELEASE_NAME: &str = "Synthetic.Feature.2024.1080p.WEB-DL.DDP5.1-SYNGRP";

/// Rules that fire only when each listing fact reached the rule input.
const LISTING_RULES: &str = r#"import rego.v1
score_entry["listing_old"] := 5 if { input.release.age_days > 365 }
score_entry["listing_liked"] := 7 if { input.release.thumbs_up >= 10 }
score_entry["listing_freeleech"] := 11 if { input.release.extra.freeleech == true }
score_entry["listing_age_unknown"] := 1 if { input.release.age_days == null }
score_entry["listing_thumbs_unknown"] := 2 if { input.release.thumbs_up == null }
"#;

fn at(year: i32, month: u32, day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
}

fn tester_app(now: chrono::DateTime<Utc>) -> (AppUseCase, User, Arc<MockMediaFileRepo>) {
    let (app, user) = bootstrap();
    let media_files = Arc::new(MockMediaFileRepo::default());
    let rules = Arc::new(TestRuleSetRepo::new(vec![]));
    let app = app.with_test_overrides(|services| {
        services
            .with_rule_sets(rules.clone())
            .with_media_files(media_files.clone())
    });
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    (app, user, media_files)
}

async fn synthetic_movie(app: &AppUseCase, user: &User) -> Title {
    app.add_title(
        user,
        NewTitle {
            name: "Synthetic Feature".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec!["scryer:quality-profile:1080p".into()],
            external_ids: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("seed synthetic movie")
}

fn draft(source: &str) -> RuleSetTestDraft {
    RuleSetTestDraft {
        name: "Listing draft".into(),
        description: String::new(),
        rego_source: source.into(),
        enabled: true,
        priority: 0,
        applied_facets: vec![MediaFacet::Movie],
    }
}

fn release_request(title_id: &str) -> RuleSetTestRequest {
    RuleSetTestRequest {
        draft: Some(draft(LISTING_RULES)),
        test_rule_set_id: None,
        edit_rule_set_id: None,
        copy_source_rule_set_id: None,
        copy_disables_source: false,
        title_id: title_id.to_string(),
        episode_id: None,
        release_name: Some(RELEASE_NAME.into()),
        size_bytes: None,
        listing: RuleSetTestListingInput::default(),
        media_file_id: None,
    }
}

fn draft_codes(result: &crate::RuleSetTestResult) -> Vec<String> {
    let mut codes: Vec<String> = result
        .rule_sets
        .iter()
        .filter(|rule| rule.is_draft)
        .flat_map(|rule| rule.entries.iter().map(|entry| entry.code.clone()))
        .collect();
    codes.sort();
    codes
}

#[tokio::test]
async fn listing_inputs_reach_the_rules_a_release_name_is_tested_with() {
    let now = at(2026, 3, 1);
    let (app, user, _) = tester_app(now);
    let title = synthetic_movie(&app, &user).await;

    let mut request = release_request(&title.id);
    request.listing = RuleSetTestListingInput {
        protocol: None,
        published_at: Some("2025-01-01T12:00:00Z".into()),
        thumbs_up: Some(12),
        thumbs_down: Some(1),
        is_password_protected: Some(false),
        indexer_languages: Some(vec![" en ".into(), String::new()]),
        extra: Some(
            serde_json::json!({ "freeleech": true, "nested": { "dropped": 1 } })
                .as_object()
                .unwrap()
                .clone(),
        ),
    };
    let result = app
        .test_rule_set(&user, request)
        .await
        .expect("preview with listing facts");

    assert_eq!(
        draft_codes(&result),
        ["listing_freeleech", "listing_liked", "listing_old"]
    );
    assert_eq!(result.draft_contribution.score, 23);
    assert_eq!(result.release_name, RELEASE_NAME);
    assert_eq!(result.media_file_id, None);
    let listing = result.listing.expect("a tested release echoes its facts");
    assert_eq!(
        listing.age_days,
        Some(424),
        "a tester candidate is aged now"
    );
    assert_eq!(listing.captured_at, now);
    assert_eq!(listing.thumbs_up, Some(12));
    assert_eq!(listing.thumbs_down, Some(1));
    assert_eq!(listing.is_password_protected, Some(false));
    assert_eq!(listing.indexer_languages, ["en"]);
    assert_eq!(
        listing.extra.keys().collect::<Vec<_>>(),
        ["freeleech"],
        "extra is bounded like a live listing's"
    );
}

#[tokio::test]
async fn protocol_reaches_release_and_stored_file_rules() {
    use scryer_rules::ReleaseProtocol::{Http, Torrent, Usenet};
    let (app, user, media_files) = tester_app(at(2026, 3, 1));
    let title = synthetic_movie(&app, &user).await;
    let rules = r#"import rego.v1
score_entry["usenet"] := 10 if { input.release.protocol == "usenet" }
score_entry["torrent"] := 20 if { input.release.protocol == "torrent" }
score_entry["http"] := 30 if { input.release.protocol == "http" }
score_entry["unknown"] := 1 if { input.release.protocol == null }
"#;
    for (protocol, code, score) in [
        (Some(Usenet), "usenet", 10),
        (Some(Torrent), "torrent", 20),
        (Some(Http), "http", 30),
        (None, "unknown", 1),
    ] {
        let mut request = release_request(&title.id);
        request.draft = Some(draft(rules));
        request.listing.protocol = protocol;
        let result = app.test_rule_set(&user, request).await.unwrap();
        assert_eq!(draft_codes(&result), [code]);
        assert_eq!(result.draft_contribution.score, score);
        assert_eq!(result.listing.unwrap().protocol, protocol);

        let snapshot =
            serde_json::json!({"v":1,"captured_at":"2024-01-01T00:00:00Z","protocol":protocol})
                .to_string();
        let file_id = seed_stored_file(&media_files, &title.id, Some(snapshot)).await;
        let mut request = stored_file_request(&title.id, &file_id);
        request.draft = Some(draft(rules));
        let result = app.test_rule_set(&user, request).await.unwrap();
        assert_eq!(draft_codes(&result), [code]);
        assert_eq!(result.draft_contribution.score, score);
        assert_eq!(result.listing.unwrap().protocol, protocol);
    }
    let file_id = seed_stored_file(&media_files, &title.id, None).await;
    let mut request = stored_file_request(&title.id, &file_id);
    request.draft = Some(draft(rules));
    let result = app.test_rule_set(&user, request).await.unwrap();
    assert_eq!(draft_codes(&result), ["unknown"]);
}

#[tokio::test]
async fn absent_listing_inputs_reach_rules_as_null() {
    let now = at(2026, 3, 1);
    let (app, user, _) = tester_app(now);
    let title = synthetic_movie(&app, &user).await;

    let result = app
        .test_rule_set(&user, release_request(&title.id))
        .await
        .expect("preview without listing facts");

    assert_eq!(
        draft_codes(&result),
        ["listing_age_unknown", "listing_thumbs_unknown"]
    );
    let listing = result.listing.expect("a tested release echoes its facts");
    assert_eq!(listing.age_days, None);
    assert_eq!(listing.thumbs_up, None);
    assert!(listing.extra.is_empty());
    assert_eq!(listing.captured_at, now);
}

#[tokio::test]
async fn an_unparseable_publish_time_is_rejected() {
    let (app, user, _) = tester_app(at(2026, 3, 1));
    let title = synthetic_movie(&app, &user).await;
    let mut request = release_request(&title.id);
    request.listing.published_at = Some("last tuesday".into());
    let error = app
        .test_rule_set(&user, request)
        .await
        .expect_err("publish time must be RFC 3339");
    assert!(
        matches!(
            error,
            AppError::Validation(_)
                | AppError::ValidationRefused { .. }
                | AppError::ValidationRefusedWithCount { .. }
        ),
        "{error}"
    );
}

async fn seed_stored_file(
    media_files: &MockMediaFileRepo,
    title_id: &str,
    release_listing_json: Option<String>,
) -> String {
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: "/synthetic/library/Synthetic Feature (2024)/feature.mkv".into(),
            size_bytes: 6_000_000_000,
            announced_size_bytes: Some(6_100_000_000),
            grabbed_release_title: Some(RELEASE_NAME.into()),
            release_listing_json,
            ..Default::default()
        })
        .await
        .expect("seed stored file")
}

fn stored_file_request(title_id: &str, media_file_id: &str) -> RuleSetTestRequest {
    RuleSetTestRequest {
        release_name: None,
        media_file_id: Some(media_file_id.to_string()),
        ..release_request(title_id)
    }
}

#[tokio::test]
async fn a_stored_file_scores_from_its_frozen_snapshot_exactly_as_its_bar() {
    let grabbed_at = at(2025, 6, 1);
    let now = at(2026, 6, 1);
    let (app, user, media_files) = tester_app(now);
    let title = synthetic_movie(&app, &user).await;

    let mut extra = std::collections::BTreeMap::new();
    extra.insert("freeleech".to_string(), serde_json::json!(true));
    let snapshot = crate::quality::release_listing::ReleaseListingSnapshot {
        protocol: None,
        grab_scoring: None,
        // 151 days old at the grab, 516 days old now.
        published_at: Some("2025-01-01T12:00:00Z".into()),
        thumbs_up: Some(12),
        thumbs_down: None,
        is_password_protected: Some(false),
        indexer_languages: vec!["en".into()],
        extra,
        captured_at: grabbed_at,
    };
    let file_id = seed_stored_file(&media_files, &title.id, Some(snapshot.to_json_string())).await;

    let result = app
        .test_rule_set(&user, stored_file_request(&title.id, &file_id))
        .await
        .expect("stored-file preview");

    // Age is frozen at the grab: 151 days, so the "older than a year" rule
    // stays silent even though the release is older than that today.
    assert_eq!(draft_codes(&result), ["listing_freeleech", "listing_liked"]);
    assert_eq!(result.release_name, RELEASE_NAME);
    assert_eq!(result.media_file_id.as_deref(), Some(file_id.as_str()));
    let listing = result
        .listing
        .clone()
        .expect("the row's snapshot is echoed");
    assert_eq!(listing.age_days, Some(151));
    assert_eq!(listing.captured_at, grabbed_at);
    assert_eq!(listing.thumbs_up, Some(12));
    assert_eq!(listing.indexer_languages, ["en"]);

    // The same row through `score_media_file`, with the draft compiled the
    // way the preview compiles it.
    let file = media_files
        .get_media_file_by_id(&file_id)
        .await
        .unwrap()
        .unwrap();
    let draft_id = "stored_file_draft";
    let engine = AppUseCase::build_user_rules_engine_for_purpose(
        vec![RuleSet {
            id: draft_id.into(),
            name: "Listing draft".into(),
            description: String::new(),
            rego_source: scryer_rules::rewrite_package_declaration(LISTING_RULES, draft_id),
            enabled: true,
            priority: 0,
            evaluation_phase: Default::default(),
            exclusive_group: None,
            disabled_reason_code: None,
            disabled_reason: None,
            applied_facets: vec![MediaFacet::Movie],
            created_at: now,
            updated_at: now,
            is_managed: false,
            managed_key: None,
            managed_tag_filter: None,
        }],
        vec![],
        crate::rules::metrics::Purpose::Preview,
    )
    .expect("compile draft");
    let profile = app.resolve_quality_profile_for_title(&title).await.unwrap();
    let context = app
        .resolve_canonical_scoring_context_without_rules(&title, &profile)
        .await;
    let view = crate::canonical_scoring::ScoringContext {
        rules: Some(&engine),
        ..context.view(crate::quality_profile::CoverageSizeBasis::default(), false)
    };
    let bar = crate::canonical_scoring::score_media_file(&file, &view);
    assert_eq!(result.score, bar.total);
    assert_eq!(result.parsed.size_bytes, Some(6_100_000_000));
}

#[tokio::test]
async fn a_stored_file_without_a_snapshot_reports_unknown_listing_facts() {
    let (app, user, media_files) = tester_app(at(2026, 6, 1));
    let title = synthetic_movie(&app, &user).await;
    let file_id = seed_stored_file(&media_files, &title.id, None).await;

    let result = app
        .test_rule_set(&user, stored_file_request(&title.id, &file_id))
        .await
        .expect("stored-file preview");

    assert_eq!(result.listing, None);
    assert_eq!(
        draft_codes(&result),
        ["listing_age_unknown", "listing_thumbs_unknown"]
    );
}

#[tokio::test]
async fn a_stored_file_of_another_title_is_not_found() {
    let (app, user, media_files) = tester_app(at(2026, 6, 1));
    let title = synthetic_movie(&app, &user).await;
    let other = app
        .add_title(
            &user,
            NewTitle {
                name: "Other Synthetic Feature".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let file_id = seed_stored_file(&media_files, &other.id, None).await;

    let error = app
        .test_rule_set(&user, stored_file_request(&title.id, &file_id))
        .await
        .expect_err("another title's file is not reachable through this title");
    assert!(
        matches!(
            error,
            AppError::NotFound(_) | AppError::NotFoundReason { .. }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn a_preview_needs_exactly_one_of_a_release_name_and_a_stored_file() {
    let (app, user, media_files) = tester_app(at(2026, 6, 1));
    let title = synthetic_movie(&app, &user).await;
    let file_id = seed_stored_file(&media_files, &title.id, None).await;

    let both = RuleSetTestRequest {
        media_file_id: Some(file_id.clone()),
        ..release_request(&title.id)
    };
    let neither = RuleSetTestRequest {
        release_name: None,
        ..release_request(&title.id)
    };
    let blank = RuleSetTestRequest {
        release_name: Some("   ".into()),
        ..release_request(&title.id)
    };
    let stored_with_listing = RuleSetTestRequest {
        listing: RuleSetTestListingInput {
            thumbs_up: Some(3),
            ..Default::default()
        },
        ..stored_file_request(&title.id, &file_id)
    };
    let stored_with_size = RuleSetTestRequest {
        size_bytes: Some(1),
        ..stored_file_request(&title.id, &file_id)
    };
    for (case, request) in [
        ("both", both),
        ("neither", neither),
        ("blank", blank),
        ("stored with listing", stored_with_listing),
        ("stored with size", stored_with_size),
    ] {
        let error = app.test_rule_set(&user, request).await.expect_err(case);
        assert!(
            matches!(
                error,
                AppError::Validation(_)
                    | AppError::ValidationRefused { .. }
                    | AppError::ValidationRefusedWithCount { .. }
            ),
            "{case}: {error}"
        );
    }
}

#[tokio::test]
async fn a_newznab_publish_time_is_accepted_as_written() {
    let (app, user, _) = tester_app(at(2026, 3, 1));
    let title = synthetic_movie(&app, &user).await;
    let mut request = release_request(&title.id);
    request.listing.published_at = Some(" Wed, 01 Jan 2025 12:00:00 +0000 ".into());
    let result = app
        .test_rule_set(&user, request)
        .await
        .expect("an RFC 2822 pubDate is kept as a live listing keeps it");
    let listing = result.listing.expect("a tested release echoes its facts");
    assert_eq!(
        listing.published_at.as_deref(),
        Some("Wed, 01 Jan 2025 12:00:00 +0000")
    );
    assert_eq!(listing.age_days, Some(424));
}

#[tokio::test]
async fn negative_votes_are_rejected() {
    let (app, user, _) = tester_app(at(2026, 3, 1));
    let title = synthetic_movie(&app, &user).await;
    for (up, down) in [(Some(-1), None), (None, Some(-1))] {
        let mut request = release_request(&title.id);
        request.listing.thumbs_up = up;
        request.listing.thumbs_down = down;
        let error = app
            .test_rule_set(&user, request)
            .await
            .expect_err("votes cannot be negative");
        assert!(
            matches!(
                error,
                AppError::Validation(_)
                    | AppError::ValidationRefused { .. }
                    | AppError::ValidationRefusedWithCount { .. }
            ),
            "{error}"
        );
    }
}

#[tokio::test]
async fn a_blank_stored_file_id_tests_the_release_name() {
    let (app, user, _) = tester_app(at(2026, 3, 1));
    let title = synthetic_movie(&app, &user).await;
    let request = RuleSetTestRequest {
        media_file_id: Some("  ".into()),
        ..release_request(&title.id)
    };
    let result = app
        .test_rule_set(&user, request)
        .await
        .expect("a blank file id is no stored file");
    assert_eq!(result.release_name, RELEASE_NAME);
    assert_eq!(result.media_file_id, None);
}

/// A series title with two episodes, and one disc image linked to both whose
/// first mapped title is 2160p and second is 1080p.
async fn synthetic_series_disc(
    app: &AppUseCase,
    user: &User,
    media_files: &MockMediaFileRepo,
) -> (Title, TitleMediaFile, [String; 2]) {
    use scryer_media_types::{
        DiscEpisodeMapping, DiscMetadata, DiscSelection, DiscTitle, ProbeReport, ProbeStatus,
        StreamDetail, StreamKind, StreamMetadata,
    };
    let title = app
        .add_title(
            user,
            NewTitle {
                name: "Synthetic Serial".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec!["scryer:quality-profile:1080p".into()],
                ..Default::default()
            },
        )
        .await
        .expect("seed synthetic series");
    let collection = app
        .create_collection(
            user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("2".into()),
        )
        .await
        .expect("create collection");
    let mut episode_ids = Vec::new();
    for number in 1..=2 {
        let episode = app
            .create_episode(
                user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some("2025-01-01T00:00:00Z".into()),
                Some(30),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }
    let episode_ids: [String; 2] = episode_ids.try_into().unwrap();

    let titles =
        [("00001", 3840, 2160), ("00002", 1920, 1080)].map(|(id, width, height)| DiscTitle {
            id: id.into(),
            duration_seconds: Some(1800.0),
            report: ProbeReport {
                status: ProbeStatus::Complete,
                ..Default::default()
            },
            streams: vec![
                StreamDetail {
                    kind: StreamKind::Video,
                    codec: Some("hevc".into()),
                    width: Some(width),
                    height: Some(height),
                    metadata: StreamMetadata {
                        id: Some("v0".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                StreamDetail {
                    kind: StreamKind::Audio,
                    codec: Some("aac".into()),
                    channels: Some(2),
                    language: Some("en".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
    let mut image = MediaFileAnalysis::default();
    image.details.revision = scryer_media_types::ANALYSIS_REVISION;
    image.details.disc = Some(DiscMetadata {
        titles: titles.to_vec(),
        selected_title_id: Some("00001".into()),
        selection: DiscSelection {
            episode_mappings: vec![
                DiscEpisodeMapping {
                    disc_title_id: "00001".into(),
                    episode_ids: vec![episode_ids[0].clone()],
                },
                DiscEpisodeMapping {
                    disc_title_id: "00002".into(),
                    episode_ids: vec![episode_ids[1].clone()],
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    });
    let image = crate::media::disc_analysis::for_title(&image, &titles[0]);

    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/synthetic/library/Synthetic Serial/Season 01/disc.iso".into(),
            size_bytes: 50_000_000_000,
            scene_name: Some("Synthetic.Serial.S01.2160p.BluRay.HEVC-SYNGRP".into()),
            ..Default::default()
        })
        .await
        .expect("seed disc file");
    // One row per linked episode, as the store lists a multi-episode file.
    let mut store = media_files.store.lock().await;
    let row = store.iter_mut().find(|row| row.id == file_id).unwrap();
    row.analysis_details = image.details.clone();
    row.video_width = image.video_width;
    row.video_height = image.video_height;
    row.video_codec = image.video_codec;
    row.episode_id = Some(episode_ids[0].clone());
    let mut second = row.clone();
    let file = row.clone();
    second.episode_id = Some(episode_ids[1].clone());
    store.push(second);
    drop(store);
    (title, file, episode_ids)
}

#[tokio::test]
async fn a_stored_disc_scores_each_episode_exactly_as_its_bar() {
    let (app, user, media_files) = tester_app(at(2026, 6, 1));
    let (title, file, episode_ids) = synthetic_series_disc(&app, &user, &media_files).await;

    let profile = app.resolve_quality_profile_for_title(&title).await.unwrap();
    let context = app
        .resolve_canonical_scoring_context_without_rules(&title, &profile)
        .await;
    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .unwrap();
    let span = episode_ids.to_vec();
    let basis = crate::acquisition_coverage::episode_span_size_basis(
        &episodes,
        &span,
        context.default_runtime_minutes(),
    );

    // A draft that never fires, so the tester's total is the file's bar.
    let quiet_draft = draft("import rego.v1\nscore_entry[\"never\"] := 1 if { false }\n");
    let mut totals = Vec::new();
    for episode_id in &episode_ids {
        let request = RuleSetTestRequest {
            draft: Some(RuleSetTestDraft {
                applied_facets: vec![MediaFacet::Series],
                ..quiet_draft.clone()
            }),
            episode_id: Some(episode_id.clone()),
            ..stored_file_request(&title.id, &file.id)
        };
        let result = app
            .test_rule_set(&user, request)
            .await
            .expect("stored disc preview");
        let bar = app.incumbent_bar_for_episodes(
            &file,
            &context,
            basis,
            std::slice::from_ref(episode_id),
        );
        assert_eq!(result.score, bar.score, "episode {episode_id}");
        totals.push(result.score);
    }
    assert_ne!(
        totals[0], totals[1],
        "each episode is judged by its own mapped disc title"
    );
}

#[tokio::test]
async fn a_stored_file_bound_to_no_episode_scores_the_whole_file() {
    let (app, user, media_files) = tester_app(at(2026, 6, 1));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Unbound Serial".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec!["scryer:quality-profile:1080p".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let file_id = seed_stored_file(&media_files, &title.id, None).await;
    let request = RuleSetTestRequest {
        draft: Some(RuleSetTestDraft {
            applied_facets: vec![MediaFacet::Series],
            ..draft(LISTING_RULES)
        }),
        ..stored_file_request(&title.id, &file_id)
    };
    let result = app
        .test_rule_set(&user, request)
        .await
        .expect("an unbound file needs no episode");
    assert_eq!(result.media_file_id.as_deref(), Some(file_id.as_str()));
    assert_eq!(result.listing, None);
}
