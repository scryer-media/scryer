#[tokio::test]
async fn tracked_pack_install_tracks_all_with_explicit_enable_and_no_automatic_updates() {
    let (app, repo) = build_test_app_with_rule_repo(vec![], vec![]);
    let actor = User::system_execution_actor();
    let pack = tracked_fixture("1.0.0", &[("a", 200), ("b", 900)]);
    assert!(
        app.install_verified_rule_pack(&actor, &pack, &["unknown".into()])
            .await
            .is_err()
    );
    assert!(repo.list_rule_sets().await.unwrap().is_empty());
    let installed = app
        .install_verified_rule_pack(&actor, &pack, &["a".into()])
        .await
        .unwrap();
    assert!(!installed.auto_update);
    assert_eq!(installed.members.len(), 2);
    assert_eq!(tracked_score(&app), 200);
    assert!(
        app.install_verified_rule_pack(&actor, &pack, &[])
            .await
            .is_err()
    );
    assert_eq!(repo.list_rule_sets().await.unwrap().len(), 2);
}

#[tokio::test]
async fn tracked_pack_listing_requires_catalog_permission_but_mutations_require_both_permissions() {
    let (app, _, installation) = tracked_app().await;
    let catalog_only = User {
        id: Id::new().0,
        username: "catalog-only".into(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: scryer_domain::UserAuthorization {
            app: scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            loaded: true,
            ..Default::default()
        },
    };
    let no_permissions = User {
        id: Id::new().0,
        username: "viewer".into(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: scryer_domain::UserAuthorization {
            loaded: true,
            ..Default::default()
        },
    };

    assert_eq!(
        app.list_tracked_rule_packs(&catalog_only)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        app.set_tracked_rule_pack_settings(
            &catalog_only,
            &installation.pack_id,
            &["a".into()],
            &[],
            false,
            installation.revision,
        )
        .await
        .is_err()
    );
    assert!(app.list_tracked_rule_packs(&no_permissions).await.is_err());
}

fn tracked_fixture(
    version: &str,
    templates: &[(&str, i32)],
) -> crate::plugins::plugins::VerifiedRulePack {
    crate::plugins::plugins::VerifiedRulePack {
        registry: crate::RulePackRegistryEntry {
            id: "community-fixture".into(),
            name: "Community fixture".into(),
            description: "Test".into(),
            author: "Community".into(),
            version: version.into(),
            digest: format!("sha256:{version}"),
            source_url: "https://example.test/pack".into(),
            customizable: true,
            min_scryer_version: None,
        },
        revision: version.into(),
        templates: templates
            .iter()
            .map(|(id, score)| crate::RulePackTemplate {
                id: (*id).into(),
                title: (*id).into(),
                description: "Test rule".into(),
                category: "test".into(),
                rego_source: format!(
                    "package example\nimport rego.v1\nscore_entry[\"bonus\"] := {score}"
                ),
                applied_facets: vec!["movie".into()],
                evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
                default_enabled: true,
                exclusive_group: None,
            })
            .collect(),
    }
}

async fn tracked_app() -> (
    AppUseCase,
    Arc<TestRuleSetRepo>,
    scryer_domain::RulePackInstallation,
) {
    let (app, repo) = build_test_app_with_rule_repo(vec![], vec![]);
    let now = Utc::now();
    let rule = RuleSet {
        id: "tracked_fixture".into(),
        name: "a".into(),
        description: "Test rule".into(),
        rego_source: scryer_rules::rewrite_package_declaration(
            &tracked_fixture("1.0.0", &[("a", 200)]).templates[0].rego_source,
            "tracked_fixture",
        ),
        enabled: true,
        priority: 17,
        evaluation_phase: scryer_domain::RuleEvaluationPhase::Additional,
        exclusive_group: None,
        disabled_reason: None,
        applied_facets: vec![MediaFacet::Movie],
        created_at: now,
        updated_at: now,
        is_managed: false,
        managed_key: None,
        managed_tag_filter: None,
    };
    let pack = scryer_domain::RulePackInstallation {
        pack_id: "community-fixture".into(),
        name: "Community fixture".into(),
        version: "1.0.0".into(),
        digest: "sha256:1.0.0".into(),
        customizable: true,
        auto_update: false,
        revision: 1,
        last_updated: now,
        last_error: None,
        members: vec![scryer_domain::RulePackMember {
            template_id: "a".into(),
            rule_set_id: rule.id.clone(),
            removed: false,
        }],
    };
    assert!(
        repo.apply_rule_pack_installation(&pack, None, &[rule], &[])
            .await
            .unwrap()
    );
    app.rebuild_user_rules_engine().await.unwrap();
    (app, repo, pack)
}

fn tracked_score(app: &AppUseCase) -> i32 {
    let guard = app.services.customization.user_rules.read().unwrap();
    let result = guard
        .evaluator()
        .evaluate(&multi_audio_rule_input("profile", false, false), "movie")
        .unwrap();
    result.entries.iter().map(|entry| entry.delta).sum()
}

#[tokio::test]
async fn tracked_pack_update_keeps_ids_preferences_and_disables_removed_and_reappearing_rules() {
    let (app, repo, initial) = tracked_app().await;
    let actor = User::system_execution_actor();
    let updated = app
        .apply_verified_tracked_pack_update(
            &actor,
            &tracked_fixture("1.0.1", &[("a", 400), ("b", 900)]),
            1,
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        updated.members[0].rule_set_id,
        initial.members[0].rule_set_id
    );
    let a = repo.get_rule_set("tracked_fixture").await.unwrap().unwrap();
    assert!(a.enabled);
    assert!(!a.is_managed);
    assert_eq!(a.priority, 17);
    assert_eq!(a.created_at, initial.last_updated);
    assert_eq!(tracked_score(&app), 400);
    let b = updated
        .members
        .iter()
        .find(|member| member.template_id == "b")
        .unwrap();
    assert!(
        !repo
            .get_rule_set(&b.rule_set_id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    let removed = app
        .apply_verified_tracked_pack_update(
            &actor,
            &tracked_fixture("1.0.2", &[("b", 900)]),
            2,
            false,
        )
        .await
        .unwrap();
    assert!(
        removed
            .members
            .iter()
            .find(|member| member.template_id == "a")
            .unwrap()
            .removed
    );
    assert!(
        !repo
            .get_rule_set("tracked_fixture")
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    assert_eq!(tracked_score(&app), 0);
    assert!(
        app.set_tracked_rule_pack_settings(&actor, &initial.pack_id, &["a".into()], &[], false, 3)
            .await
            .is_err()
    );
    let reappeared = app
        .apply_verified_tracked_pack_update(
            &actor,
            &tracked_fixture("1.0.3", &[("a", 400), ("b", 900)]),
            3,
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        reappeared
            .members
            .iter()
            .find(|member| member.template_id == "a")
            .unwrap()
            .rule_set_id,
        "tracked_fixture"
    );
    assert_eq!(tracked_score(&app), 0);
}

#[tokio::test]
async fn tracked_pack_apply_normalizes_prefixed_catalog_and_installed_versions() {
    let (app, repo, mut installation) = tracked_app().await;
    installation.version = "v1.0.0".to_string();
    assert!(
        repo.apply_rule_pack_installation(&installation, Some(1), &[], &[])
            .await
            .expect("legacy installation should be stored")
    );

    let updated = app
        .apply_verified_tracked_pack_update(
            &User::system_execution_actor(),
            &tracked_fixture("v1.0.1", &[("a", 400)]),
            1,
            false,
        )
        .await
        .expect("prefixed catalog versions should update a prefixed installation");

    assert_eq!(updated.version, "v1.0.1");
    assert_eq!(tracked_score(&app), 400);
}

#[tokio::test]
async fn tracked_pack_failed_validation_commit_and_stale_updates_preserve_the_active_engine() {
    let (app, repo, initial) = tracked_app().await;
    let actor = User::system_execution_actor();
    let mut invalid = tracked_fixture("1.0.1", &[("a", 400), ("disabled", 900)]);
    invalid.templates[1].rego_source = "invalid rego !!!".into();
    assert!(
        app.apply_verified_tracked_pack_update(&actor, &invalid, 1, false)
            .await
            .is_err()
    );
    assert_eq!(
        repo.get_rule_pack_installation(&initial.pack_id)
            .await
            .unwrap()
            .unwrap(),
        initial
    );
    assert_eq!(tracked_score(&app), 200);
    let valid = tracked_fixture("1.0.1", &[("a", 400)]);
    repo.fail_pack_apply
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        app.apply_verified_tracked_pack_update(&actor, &valid, 1, false)
            .await
            .is_err()
    );
    assert_eq!(tracked_score(&app), 200);
    assert_eq!(
        repo.get_rule_pack_installation(&initial.pack_id)
            .await
            .unwrap()
            .unwrap(),
        initial
    );
    repo.fail_pack_apply
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let enabled = ["a".into()];
    let (first, second) = tokio::join!(
        app.apply_verified_tracked_pack_update(&actor, &valid, 1, false),
        app.set_tracked_rule_pack_settings(&actor, &initial.pack_id, &enabled, &[], true, 1)
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "exactly one concurrent revision wins"
    );
    assert_eq!(
        repo.get_rule_pack_installation(&initial.pack_id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
}

#[tokio::test]
async fn tracked_pack_copy_scores_once_and_survives_uninstall() {
    let (app, repo, initial) = tracked_app().await;
    let actor = User::system_execution_actor();
    assert!(
        app.toggle_rule_set(&actor, "tracked_fixture", false)
            .await
            .is_err()
    );
    assert!(
        app.update_rule_set(
            &actor,
            "tracked_fixture".into(),
            Some("bypass".into()),
            None,
            None,
            None,
            None,
            None
        )
        .await
        .is_err()
    );
    assert!(
        app.delete_rule_set(&actor, "tracked_fixture")
            .await
            .is_err()
    );
    let custom = app
        .copy_tracked_rule_pack_rule(
            &actor,
            "tracked_fixture",
            "Custom".into(),
            "Customized".into(),
            tracked_fixture("1.0.0", &[("a", 300)])
                .templates
                .remove(0)
                .rego_source,
            vec![MediaFacet::Movie],
            22,
        )
        .await
        .unwrap();
    assert!(custom.enabled);
    assert!(!custom.is_managed);
    assert!(
        !repo
            .get_rule_set("tracked_fixture")
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    assert_eq!(tracked_score(&app), 300);
    assert!(
        app.uninstall_tracked_rule_pack(&actor, &initial.pack_id, 1)
            .await
            .is_err()
    );
    app.uninstall_tracked_rule_pack(&actor, &initial.pack_id, 2)
        .await
        .unwrap();
    assert!(repo.get_rule_set(&custom.id).await.unwrap().is_some());
    assert_eq!(tracked_score(&app), 300);
}

#[tokio::test]
async fn locked_tracked_pack_copy_rejection_preserves_source_and_installation() {
    let (app, repo, initial) = tracked_app().await;
    let actor = User::system_execution_actor();
    let mut locked = initial.clone();
    locked.customizable = false;
    locked.revision += 1;
    assert!(repo
        .apply_rule_pack_installation(&locked, Some(initial.revision), &[], &[])
        .await
        .unwrap());
    let rules_before = repo.rules_snapshot().await;

    assert!(app
        .copy_tracked_rule_pack_rule(
            &actor,
            "tracked_fixture",
            "Custom".into(),
            "Customized".into(),
            tracked_fixture("1.0.0", &[("a", 300)])
                .templates
                .remove(0)
                .rego_source,
            vec![MediaFacet::Movie],
            22,
        )
        .await
        .is_err());
    assert_eq!(repo.rules_snapshot().await, rules_before);
    assert_eq!(
        repo.get_rule_pack_installation(&locked.pack_id).await.unwrap(),
        Some(locked)
    );
}

#[tokio::test]
async fn tracked_pack_automatic_apply_rechecks_opt_in_and_version_boundary() {
    let (app, repo, initial) = tracked_app().await;
    let actor = User::system_execution_actor();
    assert!(
        app.apply_verified_tracked_pack_update(
            &actor,
            &tracked_fixture("1.0.1", &[("a", 400)]),
            1,
            true
        )
        .await
        .is_err()
    );
    app.set_tracked_rule_pack_settings(&actor, &initial.pack_id, &["a".into()], &[], true, 1)
        .await
        .unwrap();
    for version in ["1.1.0", "2.0.0", "1.0.1-beta.1", "1.0.0", "0.9.9"] {
        assert!(
            app.apply_verified_tracked_pack_update(
                &actor,
                &tracked_fixture(version, &[("a", 400)]),
                2,
                true
            )
            .await
            .is_err(),
            "{version}"
        );
    }
    assert_eq!(
        repo.get_rule_pack_installation(&initial.pack_id)
            .await
            .unwrap()
            .unwrap()
            .version,
        "1.0.0"
    );
    assert_eq!(tracked_score(&app), 200);
    app.apply_verified_tracked_pack_update(
        &actor,
        &tracked_fixture("1.0.1", &[("a", 400)]),
        2,
        true,
    )
    .await
    .unwrap();
    assert_eq!(tracked_score(&app), 400);
}

#[tokio::test]
async fn builtin_trash_bootstrap_preserves_user_choices_and_rebuilds_after_restart() {
    let (app, repo) = build_test_app_with_rule_repo(vec![], vec![]);
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    let first = repo.list_rule_pack_installations().await.unwrap().remove(0);
    assert!(!first.auto_update);
    let rules = repo.list_rule_sets().await.unwrap();
    assert!(rules.iter().any(|rule| rule.enabled));
    assert!(
        rules
            .iter()
            .filter(|rule| rule.exclusive_group.is_some())
            .all(|rule| !rule.enabled)
    );
    let actor = User::system_execution_actor();
    let updated = app
        .set_tracked_rule_pack_settings(&actor, &first.pack_id, &[], &[], true, first.revision)
        .await
        .unwrap();
    assert_eq!(tracked_score(&app), 0);
    *app.services.customization.user_rules.write().unwrap() =
        scryer_rules::UserRulesEngine::empty();
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    let restarted = repo.list_rule_pack_installations().await.unwrap().remove(0);
    assert_eq!(restarted.version, updated.version);
    assert_eq!(restarted.revision, updated.revision);
    assert!(restarted.auto_update);
    assert_eq!(repo.list_rule_sets().await.unwrap().len(), rules.len());
    assert!(
        repo.list_rule_sets()
            .await
            .unwrap()
            .iter()
            .all(|rule| !rule.enabled)
    );

    let enabled = vec![first.members[0].template_id.clone()];
    app.set_tracked_rule_pack_settings(
        &actor,
        &first.pack_id,
        &enabled,
        &[],
        true,
        restarted.revision,
    )
    .await
    .unwrap();
    let identity = app
        .services
        .customization
        .user_rules
        .read()
        .unwrap()
        .rule_identity();
    *app.services.customization.user_rules.write().unwrap() =
        scryer_rules::UserRulesEngine::empty();
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    assert_eq!(
        app.services
            .customization
            .user_rules
            .read()
            .unwrap()
            .rule_identity(),
        identity
    );
}

#[tokio::test]
async fn builtin_trash_migration_disables_retired_sources_and_requires_repair_to_enable() {
    let mut affected = legacy_managed_rule("retired_custom", "unused", "Keep my rule", "");
    affected.is_managed = false;
    affected.managed_key = None;
    affected.priority = 42;
    affected.rego_source = scryer_rules::rewrite_package_declaration(
        "score_entry[\"legacy\"] := 7 if { \"x\" in input.release.guide_facts }",
        &affected.id,
    );
    let mut already_disabled = affected.clone();
    already_disabled.id = "already_disabled".into();
    already_disabled.enabled = false;
    already_disabled.disabled_reason = Some("operator disabled".into());
    already_disabled.rego_source =
        scryer_rules::rewrite_package_declaration(&affected.rego_source, &already_disabled.id);
    let (app, repo) =
        build_test_app_with_rule_repo(vec![], vec![affected.clone(), already_disabled.clone()]);
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    for original in [&affected, &already_disabled] {
        let saved = repo.get_rule_set(&original.id).await.unwrap().unwrap();
        assert!(!saved.enabled);
        assert!(
            saved
                .disabled_reason
                .as_deref()
                .unwrap()
                .contains("guide_facts")
        );
        if let Some(reason) = original.disabled_reason.as_deref() {
            assert!(
                saved
                    .disabled_reason
                    .as_deref()
                    .unwrap()
                    .starts_with(reason)
            );
        }
        assert_eq!(saved.rego_source, original.rego_source);
        assert_eq!(saved.created_at, original.created_at);
        assert_eq!(saved.name, original.name);
        assert_eq!(saved.priority, original.priority);
        assert_eq!(saved.applied_facets, original.applied_facets);
    }
    let actor = User::system_execution_actor();
    assert!(
        app.toggle_rule_set(&actor, &affected.id, true)
            .await
            .is_err()
    );
    let repaired = app
        .update_rule_set(
            &actor,
            affected.id.clone(),
            None,
            None,
            Some("score_entry[\"repaired\"] := 7".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(!repaired.enabled);
    assert!(repaired.disabled_reason.is_none());
    assert!(
        app.toggle_rule_set(&actor, &affected.id, true)
            .await
            .unwrap()
            .enabled
    );
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    assert!(
        repo.get_rule_set(&affected.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    assert!(
        !repo
            .get_rule_set(&already_disabled.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn builtin_trash_bootstrap_failure_preserves_saved_rules_and_live_engine() {
    let (app, repo, _) = tracked_app().await;
    let before = repo.rules_snapshot().await;
    repo.fail_pack_apply
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(app.bootstrap_builtin_trash_rule_pack().await.is_err());
    assert_eq!(repo.list_rule_pack_installations().await.unwrap().len(), 1);
    assert_eq!(repo.rules_snapshot().await.len(), before.len());
    assert_eq!(
        repo.rules_snapshot().await[0].rego_source,
        before[0].rego_source
    );
    assert_eq!(tracked_score(&app), 200);
}

#[tokio::test]
async fn builtin_trash_adopts_locale_scope_and_preserves_it_through_copy() {
    let mut legacy = legacy_managed_rule(
        "existing_german",
        "trash-guides:locale:german",
        "My German policy",
        "package old\nimport rego.v1\nscore_entry[\"old\"] := 5",
    );
    legacy.applied_facets = vec![MediaFacet::Movie];
    legacy.managed_tag_filter = Some(vec!["german-library".into()]);
    legacy.priority = 37;
    let (app, repo) = build_test_app_with_rule_repo(vec![], vec![legacy.clone()]);
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    let adopted = repo.get_rule_set(&legacy.id).await.unwrap().unwrap();
    assert!(adopted.enabled);
    assert_eq!(adopted.id, legacy.id);
    assert_eq!(adopted.priority, legacy.priority);
    assert_eq!(adopted.created_at, legacy.created_at);
    assert_eq!(adopted.applied_facets, legacy.applied_facets);
    assert_eq!(adopted.managed_tag_filter, legacy.managed_tag_filter);
    assert_ne!(adopted.rego_source, legacy.rego_source);

    let check_scope = |rule_id: &str| {
        let engine = app.services.customization.user_rules.read().unwrap();
        let mut evaluator = engine.evaluator();
        for (tags, required, facet, expected) in [
            (vec![], vec![], "movie", false),
            (vec!["german-library".into()], vec![], "movie", true),
            (vec![], vec!["deu".into()], "movie", true),
            (vec!["locale:de".into()], vec![], "movie", true),
            (vec!["german-library".into()], vec![], "anime", false),
        ] {
            let mut input = multi_audio_rule_input("german-profile", false, false);
            input.context.tags = tags;
            input.profile.required_audio_languages = required;
            input.release.languages_audio.clear();
            let result = evaluator.evaluate(&input, facet).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(
                result
                    .entries
                    .iter()
                    .any(|entry| entry.rule_set_id == rule_id),
                expected
            );
        }
    };
    check_scope(&adopted.id);
    let copied = app
        .copy_tracked_rule_pack_rule(
            &User::system_execution_actor(),
            &adopted.id,
            "Custom German".into(),
            "My preferences".into(),
            adopted.rego_source.clone(),
            adopted.applied_facets.clone(),
            38,
        )
        .await
        .unwrap();
    assert_eq!(copied.evaluation_phase, adopted.evaluation_phase);
    assert_eq!(copied.managed_tag_filter, adopted.managed_tag_filter);
    assert!(
        !repo
            .get_rule_set(&adopted.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    check_scope(&copied.id);
    app.bootstrap_builtin_trash_rule_pack().await.unwrap();
    assert!(
        !repo
            .get_rule_set(&adopted.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    assert!(
        repo.get_rule_set(&copied.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn tracked_pack_custom_baseline_keeps_phase_and_exclusive_group() {
    let (app, repo) = build_test_app_with_rule_repo(vec![], vec![]);
    let actor = User::system_execution_actor();
    let mut pack = tracked_fixture("1.0.0", &[("a", 200), ("b", 900)]);
    for template in &mut pack.templates {
        template.evaluation_phase = scryer_domain::RuleEvaluationPhase::Baseline;
        template.exclusive_group = Some("alternative-baselines".into());
    }
    assert!(
        app.install_verified_rule_pack(&actor, &pack, &["a".into(), "b".into()])
            .await
            .is_err()
    );
    assert!(repo.list_rule_sets().await.unwrap().is_empty());
    let installed = app
        .install_verified_rule_pack(&actor, &pack, &["a".into()])
        .await
        .unwrap();
    let original_id = &installed
        .members
        .iter()
        .find(|member| member.template_id == "a")
        .unwrap()
        .rule_set_id;
    let custom = app
        .copy_tracked_rule_pack_rule(
            &actor,
            original_id,
            "Custom baseline".into(),
            "Customized".into(),
            "score_entry[\"bonus\"] := 300".into(),
            vec![MediaFacet::Movie],
            0,
        )
        .await
        .unwrap();
    assert_eq!(
        custom.evaluation_phase,
        scryer_domain::RuleEvaluationPhase::Baseline
    );
    assert_eq!(
        custom.exclusive_group.as_deref(),
        Some("alternative-baselines")
    );
    assert_eq!(tracked_score(&app), 300);
    assert!(
        app.set_tracked_rule_pack_settings(
            &actor,
            &installed.pack_id,
            &["b".into()],
            &[],
            false,
            2,
        )
        .await
        .is_err()
    );
    assert_eq!(tracked_score(&app), 300);
    assert_eq!(
        repo.get_rule_pack_installation(&installed.pack_id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert!(
        app.update_rule_set(
            &actor,
            custom.id.clone(),
            None,
            None,
            Some("score_entry[\"cycle\"] := input.builtin_score.total".into()),
            None,
            None,
            None,
        )
        .await
        .is_err()
    );
    assert_eq!(
        repo.get_rule_set(&custom.id)
            .await
            .unwrap()
            .unwrap()
            .rego_source,
        custom.rego_source
    );
    app.toggle_rule_set(&actor, &custom.id, false)
        .await
        .unwrap();
    app.set_tracked_rule_pack_settings(&actor, &installed.pack_id, &["b".into()], &[], false, 2)
        .await
        .unwrap();
    assert_eq!(tracked_score(&app), 900);
}
