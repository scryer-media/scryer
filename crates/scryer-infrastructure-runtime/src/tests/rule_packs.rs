use super::*;
use scryer_application::{RuleSetHistoryChange, RuleSetRepository};
use scryer_domain::{RulePackInstallation, RulePackMember, RuleSet};

fn managed_rule(id: &str) -> RuleSet {
    let now = Utc::now();
    RuleSet {
        id: id.to_string(),
        name: "Installed rule".to_string(),
        description: "A tracked rule".to_string(),
        rego_source: "package scryer.rules\nallow if { true }".to_string(),
        enabled: true,
        priority: 0,
        applied_facets: Vec::new(),
        created_at: now,
        updated_at: now,
        is_managed: false,
        managed_key: None,
        managed_tag_filter: None,
    }
}

fn installation(revision: i64) -> RulePackInstallation {
    RulePackInstallation {
        pack_id: "pack.example".to_string(),
        name: "Example pack".to_string(),
        version: "1.0.0".to_string(),
        digest: "sha256:example".to_string(),
        auto_update: true,
        revision,
        last_updated: Utc::now(),
        last_error: None,
        members: vec![RulePackMember {
            template_id: "template-a".to_string(),
            rule_set_id: "managed-a".to_string(),
            removed: false,
        }],
    }
}

#[tokio::test]
async fn tracked_rule_pack_apply_copy_and_uninstall_are_atomic_and_revision_guarded() {
    let (services, db) = temp_services("scryer_tracked_rule_packs").await;
    let store = crate::RuleSetStore::new(services.datastore());
    let installed = installation(1);
    let source = managed_rule("managed-a");

    assert!(
        store
            .apply_rule_pack_installation(&installed, None, std::slice::from_ref(&source), &[])
            .await
            .expect("initial install")
    );
    assert!(
        !store
            .apply_rule_pack_installation(&installed, None, std::slice::from_ref(&source), &[])
            .await
            .expect("duplicate install is a conflict")
    );

    let mut invalid_update = installation(2);
    invalid_update.version = "2.0.0".to_string();
    invalid_update.members[0].rule_set_id = "missing-rule".to_string();
    assert!(
        store
            .apply_rule_pack_installation(&invalid_update, Some(1), &[], &[])
            .await
            .is_err()
    );
    let after_rollback = store
        .get_rule_pack_installation("pack.example")
        .await
        .expect("read after rejected update")
        .expect("pack remains installed");
    assert_eq!(after_rollback.revision, 1);
    assert_eq!(after_rollback.version, "1.0.0");

    let by_member = store
        .find_rule_pack_installation_by_rule_set_id("managed-a")
        .await
        .expect("member lookup")
        .expect("pack exists");
    assert_eq!(by_member.members, installed.members);

    let mut copied = source.clone();
    copied.id = "custom-a".to_string();
    copied.name = "Copied rule".to_string();
    copied.is_managed = false;
    copied.managed_key = None;
    let history = vec![RuleSetHistoryChange {
        rule_set_id: copied.id.clone(),
        action: "copied_from_rule_pack".to_string(),
        rego_source: Some(copied.rego_source.clone()),
        actor_id: Some("user-1".to_string()),
    }];
    assert!(
        store
            .copy_rule_pack_rule_set_to_custom("pack.example", "managed-a", &copied, 1, &history)
            .await
            .expect("copy to custom")
    );
    assert!(
        !store
            .get_rule_set("managed-a")
            .await
            .expect("managed read")
            .expect("managed rule exists")
            .enabled
    );
    assert!(
        !store
            .get_rule_set("custom-a")
            .await
            .expect("custom read")
            .expect("custom rule exists")
            .is_managed
    );

    assert!(
        !store
            .uninstall_rule_pack("pack.example", 1, &[])
            .await
            .expect("stale uninstall conflict")
    );
    assert!(
        store
            .uninstall_rule_pack("pack.example", 2, &[])
            .await
            .expect("current uninstall")
    );
    assert!(
        store
            .get_rule_set("managed-a")
            .await
            .expect("managed after uninstall")
            .is_none()
    );
    assert!(
        store
            .get_rule_set("custom-a")
            .await
            .expect("custom after uninstall")
            .is_some()
    );
    assert!(
        store
            .get_rule_pack_installation("pack.example")
            .await
            .expect("pack after uninstall")
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}
