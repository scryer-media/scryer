use super::*;
use crate::RuleSetRepository;
use crate::lib_tests::bootstrap;
use crate::rules::workflow::tests::TestRuleSetRepo;
use scryer_domain::{MediaFacet, NewTitle, RuleEvaluationPhase, RuleSet};
use std::sync::Arc;
use std::time::Instant;

fn draft(source: &str) -> RuleSetTestDraft {
    RuleSetTestDraft {
        name: "Preview draft".into(),
        description: "test-only preview draft".into(),
        rego_source: source.into(),
        enabled: true,
        priority: 0,
        applied_facets: vec![MediaFacet::Movie],
    }
}

fn request(title_id: String, source: &str) -> RuleSetTestRequest {
    RuleSetTestRequest {
        draft: draft(source),
        edit_rule_set_id: None,
        copy_source_rule_set_id: None,
        copy_disables_source: false,
        title_id,
        episode_id: None,
        release_name: "Preview.Movie.2024.1080p.WEB-DL.DDP5.1-GROUP".into(),
        size_bytes: None,
    }
}

fn preview_app() -> (AppUseCase, User, Arc<TestRuleSetRepo>) {
    let (app, user) = bootstrap();
    let rules = Arc::new(TestRuleSetRepo::new(
        super::builtin_trash::baseline_rule_sets(),
    ));
    let app = app.with_test_overrides(|services| services.with_rule_sets(rules.clone()));
    (app, user, rules)
}

fn preview_app_without_policies() -> (AppUseCase, User, Arc<TestRuleSetRepo>) {
    let (app, user) = bootstrap();
    let rules = Arc::new(TestRuleSetRepo::new(vec![]));
    let app = app.with_test_overrides(|services| services.with_rule_sets(rules.clone()));
    (app, user, rules)
}

async fn movie(app: &AppUseCase, user: &User) -> scryer_domain::Title {
    app.add_title(
        user,
        NewTitle {
            name: "Preview Movie".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec!["scryer:quality-profile:1080p".into()],
            external_ids: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("seed preview movie")
}

async fn series(app: &AppUseCase, user: &User, name: &str) -> scryer_domain::Title {
    app.add_title(
        user,
        NewTitle {
            name: name.into(),
            facet: MediaFacet::Series,
            monitored: true,
            ..Default::default()
        },
    )
    .await
    .expect("seed preview series")
}

#[tokio::test]
async fn preview_scores_unsaved_draft_without_persisting_or_mutating_active_rules() {
    let (app, user, rule_repo) = preview_app();
    let title = movie(&app, &user).await;
    let before = app.list_rule_sets(&user).await.expect("list rules before");
    let active_before = rule_repo.rules_snapshot().await;
    let result = app
        .test_rule_set(
            &user,
            request(
                title.id,
                "import rego.v1\nscore_entry[\"preview_bonus\"] := 37 if { input.release.quality == \"1080p\" }",
            ),
        )
        .await
        .expect("preview succeeds");

    assert_eq!(result.parsed.size_bytes, None, "unknown size stays unknown");
    assert_eq!(result.context.title_name, "Preview Movie");
    assert!(result.draft_contribution.matched);
    assert_eq!(result.draft_contribution.score, 37);
    assert!(result.rule_sets.iter().any(|rule| {
        rule.is_draft
            && rule
                .entries
                .iter()
                .any(|entry| entry.code == "preview_bonus" && entry.delta == 37)
    }));
    assert_eq!(
        app.list_rule_sets(&user).await.expect("list rules after"),
        before,
        "preview must not save the draft or alter saved rule state"
    );
    assert_eq!(rule_repo.rules_snapshot().await, active_before);
}

#[tokio::test]
async fn preview_explains_disabled_and_facet_mismatched_drafts() {
    let (app, user, _) = preview_app();
    let title = movie(&app, &user).await;
    let source = "import rego.v1\nscore_entry[\"should_not_run\"] := 99 if { true }";

    let mut disabled = request(title.id.clone(), source);
    disabled.draft.enabled = false;
    let disabled = app
        .test_rule_set(&user, disabled)
        .await
        .expect("disabled preview");
    assert!(!disabled.draft_contribution.enabled);
    assert_eq!(disabled.draft_contribution.score, 0);
    assert!(
        disabled
            .draft_contribution
            .message
            .as_deref()
            .is_some_and(|message| message.contains("disabled"))
    );

    let mut mismatched = request(title.id, source);
    mismatched.draft.applied_facets = vec![MediaFacet::Series];
    let mismatched = app
        .test_rule_set(&user, mismatched)
        .await
        .expect("facet mismatch preview");
    assert!(!mismatched.draft_contribution.applies);
    assert_eq!(mismatched.draft_contribution.score, 0);
    assert!(
        mismatched
            .draft_contribution
            .message
            .as_deref()
            .is_some_and(|message| message.contains("does not apply"))
    );
}

#[tokio::test]
async fn preview_replaces_edits_but_keeps_ordinary_copy_source_active() {
    let (app, user, _) = preview_app();
    let title = movie(&app, &user).await;
    let saved = app
        .create_rule_set(
            &user,
            "Saved rule".into(),
            String::new(),
            "import rego.v1\nscore_entry[\"saved_score\"] := 11 if { true }".into(),
            vec![MediaFacet::Movie],
            0,
            Some(true),
        )
        .await
        .expect("create saved rule");
    let replacement_source = "import rego.v1\nscore_entry[\"replacement_score\"] := 23 if { true }";

    let mut edit = request(title.id.clone(), replacement_source);
    edit.edit_rule_set_id = Some(saved.id.clone());
    let edit_result = app.test_rule_set(&user, edit).await.expect("edit preview");
    assert!(
        edit_result
            .rule_sets
            .iter()
            .all(|rule| { rule.entries.iter().all(|entry| entry.code != "saved_score") })
    );
    assert!(edit_result.rule_sets.iter().any(|rule| {
        rule.is_draft
            && rule
                .entries
                .iter()
                .any(|entry| entry.code == "replacement_score")
    }));

    let mut copied = request(title.id, replacement_source);
    copied.copy_source_rule_set_id = Some(saved.id);
    let copied_result = app
        .test_rule_set(&user, copied)
        .await
        .expect("copy preview");
    assert!(copied_result.rule_sets.iter().any(|rule| {
        !rule.is_draft && rule.entries.iter().any(|entry| entry.code == "saved_score")
    }));
    assert!(copied_result.rule_sets.iter().any(|rule| {
        rule.is_draft
            && rule
                .entries
                .iter()
                .any(|entry| entry.code == "replacement_score")
    }));
}

#[tokio::test]
async fn preview_ordinary_copy_uses_create_metadata_instead_of_custom_source_metadata() {
    let (app, user, repo) = preview_app_without_policies();
    let title = movie(&app, &user).await;
    let now = chrono::Utc::now();
    let source = RuleSet {
        id: "custom_baseline".to_string(),
        name: "Custom baseline".to_string(),
        description: String::new(),
        rego_source: scryer_rules::rewrite_package_declaration(
            "score_entry[\"source_baseline\"] := 11",
            "custom_baseline",
        ),
        enabled: true,
        priority: 0,
        evaluation_phase: RuleEvaluationPhase::Baseline,
        exclusive_group: Some("custom-group".to_string()),
        disabled_reason: None,
        applied_facets: vec![MediaFacet::Movie],
        created_at: now,
        updated_at: now,
        is_managed: false,
        managed_key: None,
        managed_tag_filter: None,
    };
    repo.create_rule_set(&source)
        .await
        .expect("custom source should persist");

    let mut copied = request(
        title.id,
        "score_entry[\"draft_subtotal\"] := input.builtin_score.total",
    );
    copied.copy_source_rule_set_id = Some(source.id.clone());
    let result = app
        .test_rule_set(&user, copied)
        .await
        .expect("ordinary copy preview should use create-rule metadata");

    assert_eq!(result.draft_contribution.score, 11);
    assert!(result.rule_sets.iter().any(|rule| {
        rule.rule_set_id.as_deref() == Some(source.id.as_str())
            && rule
                .entries
                .iter()
                .any(|entry| entry.code == "source_baseline")
    }));
}

#[tokio::test]
async fn preview_rejects_invalid_title_and_oversized_release_before_any_persistence() {
    let (app, user, _) = preview_app();
    let before = app.list_rule_sets(&user).await.expect("list rules before");
    let invalid_title = app
        .test_rule_set(&user, request("missing-title".into(), "import rego.v1"))
        .await
        .expect_err("missing title is rejected");
    assert!(invalid_title.to_string().contains("missing-title"));

    let title = movie(&app, &user).await;
    let mut oversized = request(title.id, "import rego.v1");
    oversized.release_name = "x".repeat(4097);
    let oversized = app
        .test_rule_set(&user, oversized)
        .await
        .expect_err("oversized release name is rejected");
    assert!(oversized.to_string().contains("too large"));
    assert_eq!(
        app.list_rule_sets(&user).await.expect("list rules after"),
        before
    );
}

#[tokio::test]
async fn preview_requires_catalog_permission_and_rejects_episode_from_another_show() {
    let (app, user, _) = preview_app();
    let title = movie(&app, &user).await;
    let mut denied = user.clone();
    denied.authorization = scryer_domain::UserAuthorization {
        loaded: true,
        ..Default::default()
    };
    let denied_error = app
        .test_rule_set(
            &denied,
            request(title.id.clone(), "score_entry[\"zero\"] := 0"),
        )
        .await
        .expect_err("preview requires catalog-management permission");
    assert!(
        matches!(denied_error, AppError::Unauthorized(_)),
        "{denied_error}"
    );

    // The existing authorization contract lets catalog administrators view
    // every library, even without a library-specific grant.
    denied.authorization.app = scryer_domain::AppPermissionMask::MANAGE_CATALOG_SETTINGS;
    assert!(
        app.test_rule_set(&denied, request(title.id, "score_entry[\"zero\"] := 0"))
            .await
            .is_ok()
    );

    let selected = series(&app, &user, "Selected show").await;
    let other = series(&app, &user, "Other show").await;
    let other_episode = app
        .create_episode(
            &user,
            other.id,
            None,
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("Other show".into()),
            None,
            Some(1_440),
            false,
            false,
        )
        .await
        .expect("seed other episode");
    let mut mismatch = request(selected.id, "import rego.v1");
    mismatch.draft.applied_facets = vec![MediaFacet::Series];
    mismatch.episode_id = Some(other_episode.id);
    let mismatch_error = app
        .test_rule_set(&user, mismatch)
        .await
        .expect_err("episode from another title is rejected");
    assert!(mismatch_error.to_string().contains("does not belong"));
}

#[tokio::test]
async fn preview_reports_draft_block_entry_without_saving_it() {
    let (app, user, _) = preview_app();
    let title = movie(&app, &user).await;
    let result = app
        .test_rule_set(
            &user,
            request(
                title.id,
                "import rego.v1\nscore_entry[\"preview_block\"] := -10000 if { true }",
            ),
        )
        .await
        .expect("block preview succeeds");
    assert!(result.blocked);
    assert!(!result.allowed);
    assert!(!result.draft_contribution.blocked);
    assert_eq!(result.draft_contribution.score, -10_000);
    assert!(result.rule_sets.iter().any(|rule| {
        rule.is_draft
            && rule.entries.iter().any(|entry| {
                entry.code == "preview_block" && !entry.blocked && entry.delta == -10_000
            })
    }));
}

#[tokio::test]
async fn recoverable_scores_preview_includes_penalties_and_other_rules() {
    let (app, user, _) = preview_app();
    let title = movie(&app, &user).await;
    app.create_rule_set(
        &user,
        "Penalty pack".into(),
        String::new(),
        "score_entry[\"penalty\"] := scryer.block_score()".into(),
        vec![],
        0,
        Some(true),
    )
    .await
    .unwrap();
    let result = app
        .test_rule_set(
            &user,
            request(title.id, "score_entry[\"group_bonus\"] := 20000"),
        )
        .await
        .unwrap();
    assert!(result.allowed, "{result:?}");
    assert!(!result.blocked);
    assert_eq!(result.draft_contribution.score, 20_000);
    assert!(
        result
            .rule_sets
            .iter()
            .any(|rule| rule.score == -10_000 && !rule.blocked)
    );
    assert_eq!(
        result.score,
        crate::quality_profile::sum_score_deltas(result.rule_sets.iter().map(|rule| rule.score))
    );
}

#[tokio::test]
async fn preview_tracked_copy_excludes_source_without_changing_membership() {
    let (app, user, repo) = preview_app_without_policies();
    let title = movie(&app, &user).await;
    let saved = app
        .create_rule_set(
            &user,
            "Pack rule".into(),
            String::new(),
            "score_entry[\"pack_bonus\"] := 100".into(),
            vec![MediaFacet::Movie],
            0,
            Some(true),
        )
        .await
        .unwrap();
    let pack = scryer_domain::RulePackInstallation {
        pack_id: "community:test".into(),
        name: "Test pack".into(),
        version: "1.0.0".into(),
        digest: "test-digest".into(),
        auto_update: true,
        revision: 1,
        last_updated: Utc::now(),
        last_error: None,
        members: vec![scryer_domain::RulePackMember {
            template_id: "template".into(),
            rule_set_id: saved.id.clone(),
            removed: false,
        }],
    };
    assert!(
        repo.apply_rule_pack_installation(&pack, None, &[], &[])
            .await
            .unwrap()
    );
    let mut copied = request(title.id, "score_entry[\"copy_bonus\"] := 25");
    copied.copy_source_rule_set_id = Some(saved.id.clone());
    assert!(app.test_rule_set(&user, copied.clone()).await.is_err());
    copied.copy_disables_source = true;
    let result = app.test_rule_set(&user, copied.clone()).await.unwrap();
    assert!(
        result
            .rule_sets
            .iter()
            .all(|item| item.rule_set_id.as_deref() != Some(&saved.id))
    );
    assert_eq!(result.draft_contribution.score, 25);
    assert_eq!(repo.rules_snapshot().await, vec![saved.clone()]);
    assert_eq!(
        repo.get_rule_pack_installation(&pack.pack_id)
            .await
            .unwrap(),
        Some(pack)
    );
    copied.copy_source_rule_set_id = None;
    copied.copy_disables_source = false;
    copied.edit_rule_set_id = Some(saved.id);
    assert!(app.test_rule_set(&user, copied).await.is_err());
}

#[tokio::test]
async fn preview_preserves_cross_rule_references_and_active_engine() {
    let (app, user, repo) = preview_app();
    let title = movie(&app, &user).await;
    let base = app
        .create_rule_set(
            &user,
            "Referenced".into(),
            String::new(),
            "score_entry[\"base\"] := 11".into(),
            vec![],
            0,
            Some(true),
        )
        .await
        .unwrap();
    let dependent_source = format!(
        "score_entry[\"dependent\"] := data.scryer.rules.user.{}.score_entry.base",
        base.id
    );
    app.create_rule_set(
        &user,
        "Dependent".into(),
        String::new(),
        dependent_source,
        vec![],
        0,
        Some(true),
    )
    .await
    .unwrap();
    let before = repo.rules_snapshot().await;
    let mut edit = request(title.id.clone(), "score_entry[\"base\"] := 23");
    edit.edit_rule_set_id = Some(base.id);
    let preview = app.test_rule_set(&user, edit).await.unwrap();
    assert!(preview.errors.is_empty(), "{:?}", preview.errors);
    assert_eq!(preview.draft_contribution.score, 23);
    assert!(
        preview
            .rule_sets
            .iter()
            .flat_map(|item| &item.entries)
            .any(|entry| entry.code == "dependent" && entry.delta == 23)
    );
    assert_eq!(repo.rules_snapshot().await, before);

    // Canonical production scoring still sees the original engine after the
    // temporary combined engine has been dropped.
    let profile = app.resolve_quality_profile_for_title(&title).await.unwrap();
    let context = app
        .resolve_canonical_scoring_context(&title, &profile)
        .await;
    let parsed = crate::release_parser::parse_release_metadata_for_target(
        &request(title.id.clone(), "").release_name,
        &crate::release_parser::build_release_parse_context_for_title(&title, &[], Some("movie")),
    );
    let normal = crate::canonical_scoring::score_release(
        &crate::canonical_scoring::ReleaseEvidence::announced(parsed, None),
        &context.view(Default::default(), false),
    );
    let entries = &normal.announced_decision.scoring_log;
    assert!(
        entries
            .iter()
            .any(|entry| entry.code == "base" && entry.delta == 11)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.code == "dependent" && entry.delta == 11)
    );
    assert_eq!(preview.score - normal.total, 24);
}

#[tokio::test]
async fn preview_surfaces_runtime_errors_and_rejects_invalid_drafts() {
    let (app, user, repo) = preview_app_without_policies();
    let title = movie(&app, &user).await;
    let source = "score_entry[\"broken\"] := lower(input.release.year) if { contains(input.release.raw_title, \"Preview.Movie\") }";
    let result = app
        .test_rule_set(&user, request(title.id.clone(), source))
        .await
        .unwrap();
    assert_eq!(result.errors.len(), 1);
    assert!(!result.draft_contribution.matched);
    assert_eq!(result.draft_contribution.score, 0);
    assert!(result.draft_contribution.message.is_some());
    assert!(
        result
            .rule_sets
            .iter()
            .filter(|item| item.is_draft)
            .all(|item| !item.matched)
    );
    let mut invalid = request(title.id, "score_entry[ :=");
    invalid.draft.enabled = false;
    assert!(app.test_rule_set(&user, invalid.clone()).await.is_err());
    invalid.draft.rego_source = "x".repeat(16 * 1024 * 1024 + 1);
    assert!(app.test_rule_set(&user, invalid).await.is_err());
    assert!(repo.rules_snapshot().await.is_empty());
    assert!(
        app.services
            .customization
            .user_rules
            .read()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn preview_uses_title_profile_and_minimum_gate() {
    let (app, user, _) = preview_app();
    let title = movie(&app, &user).await;
    let mut profile = app.resolve_quality_profile_for_title(&title).await.unwrap();
    profile.name = "Preview minimum".into();
    let baseline = app
        .test_rule_set(
            &user,
            request(title.id.clone(), "score_entry[\"zero\"] := 0"),
        )
        .await
        .unwrap();
    assert!(baseline.allowed, "baseline should pass: {baseline:?}");
    profile.criteria.min_score_to_grab = Some(baseline.score + 10);
    app.services
        .config
        .quality_profiles
        .replace_quality_profiles("system", None, vec![profile])
        .await
        .unwrap();
    let blocked = app
        .test_rule_set(
            &user,
            request(title.id.clone(), "score_entry[\"zero\"] := 0"),
        )
        .await
        .unwrap();
    assert_eq!(blocked.profile_name, "Preview minimum");
    assert!(!blocked.minimum_score_met);
    assert!(!blocked.allowed);
    assert_eq!(blocked.score, baseline.score);
    let rescued = app
        .test_rule_set(&user, request(title.id, "score_entry[\"bonus\"] := 20"))
        .await
        .unwrap();
    assert!(rescued.minimum_score_met);
    assert!(rescued.allowed);
}

#[tokio::test]
async fn preview_uses_series_and_anime_context_without_rewriting_numbering() {
    for facet in [MediaFacet::Series, MediaFacet::Anime] {
        let (app, user, _) = preview_app();
        app.create_title_tag_definition(&user, "preview-tag", None)
            .await
            .unwrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Preview Show".into(),
                    facet: facet.clone(),
                    monitored: true,
                    language: Some("jpn".into()),
                    runtime_minutes: Some(60),
                    tags: vec!["preview-tag".into(), "scryer:quality-profile:1080p".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let source = r#"score_entry["context"] := 37 if {
            input.profile.id == "1080p"
            input.context.original_language == "jpn"
            "preview-tag" in input.context.tags
            input.context.runtime_minutes == 24
            input.release.size_bytes == null
            object.get(input.release, "extra", {}) == {}
            input.file == null
        }"#;
        let mut probe = request(title.id.clone(), source);
        probe.draft.applied_facets = vec![facet];
        assert!(
            app.test_rule_set(&user, probe.clone()).await.is_err(),
            "show needs an episode"
        );
        let episode = app
            .create_episode(
                &user,
                title.id,
                None,
                "standard".into(),
                Some("1".into()),
                Some("1".into()),
                Some("S01E01".into()),
                None,
                None,
                Some(1440),
                false,
                false,
            )
            .await
            .unwrap();
        probe.episode_id = Some(episode.id);
        probe.release_name = "Preview.Show.S01E01.1080p.WEB-DL-GROUP".into();
        let result = app.test_rule_set(&user, probe.clone()).await.unwrap();
        assert_eq!(result.draft_contribution.score, 37, "{result:?}");
        assert_eq!(result.profile_name, "1080P");
        assert_eq!(result.context.language.as_deref(), Some("jpn"));
        assert_eq!(result.context.episode_label.as_deref(), Some("S01E01"));
        probe.release_name = "Preview.Show.S02E09.1080p.WEB-DL-GROUP".into();
        let numbered = app.test_rule_set(&user, probe).await.unwrap();
        assert_eq!(numbered.parsed.season.as_deref(), Some("2"));
        assert_eq!(numbered.parsed.episode.as_deref(), Some("9"));
    }
}

#[tokio::test]
#[ignore = "requires SEADEX_PREVIEW_SOURCE pointing to a generated pack JSON file"]
async fn preview_seadex_pack_latency_keeps_active_rules_unchanged() {
    let source_path = std::env::var("SEADEX_PREVIEW_SOURCE")
        .expect("set SEADEX_PREVIEW_SOURCE to a generated pack JSON file");
    let pack: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&source_path).expect("read SeaDex preview source"))
            .expect("parse SeaDex preview JSON");
    let source = pack["rules"][0]["regoSource"]
        .as_str()
        .expect("first SeaDex rule source");
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "SeaDex Preview".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec!["scryer:quality-profile:1080p".into()],
                ..Default::default()
            },
        )
        .await
        .expect("seed anime title");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            None,
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("SeaDex Preview".into()),
            None,
            Some(1_440),
            false,
            false,
        )
        .await
        .expect("seed anime episode");
    let before = app.list_rule_sets(&user).await.expect("rules before");
    let mut request = request(title.id, source);
    request.draft.applied_facets = vec![MediaFacet::Anime];
    request.episode_id = Some(episode.id);
    let started = Instant::now();
    for _ in 0..3 {
        app.test_rule_set(&user, request.clone())
            .await
            .expect("SeaDex preview succeeds");
    }
    eprintln!("SeaDex preview: {:?} for three previews", started.elapsed());
    assert_eq!(
        app.list_rule_sets(&user).await.expect("rules after"),
        before
    );
}
