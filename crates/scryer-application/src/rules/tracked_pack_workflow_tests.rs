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
