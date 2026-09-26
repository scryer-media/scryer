//! Lists ship behind the experimental-features switch: while it is off, a
//! manager cannot follow or sync a public list and the sync job idles.

use super::*;
use crate::lists::sync::ListSyncReport;

fn list_manager() -> User {
    let mut manager = User::new_admin("list-manager");
    manager.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::MANAGE_LISTS,
        loaded: true,
        ..Default::default()
    };
    manager
}

async fn set_experimental_features(harness: &MediaRequestTestHarness, enabled: bool) {
    harness
        .app
        .services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            crate::settings::keys::EXPERIMENTAL_FEATURES_ENABLED_KEY,
            None,
            enabled.to_string().into(),
            "test",
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn public_list_sync_is_refused_until_experimental_features_are_on() {
    let harness = bootstrap_media_request_app();
    let manager = list_manager();

    let refused = harness
        .app
        .sync_all_public_lists(&manager)
        .await
        .expect_err("lists stay off by default");
    assert!(
        matches!(refused, AppError::Validation(ref message) if message.contains("experimental")),
        "unexpected error: {refused:?}"
    );
    assert_eq!(
        harness.app.run_list_sync_job(None).await.unwrap(),
        ListSyncReport::default(),
        "the sync job idles while lists are off"
    );

    set_experimental_features(&harness, true).await;
    assert_eq!(
        harness
            .app
            .sync_all_public_lists(&manager)
            .await
            .expect("lists open once the switch is on"),
        Vec::<String>::new()
    );

    set_experimental_features(&harness, false).await;
    harness
        .app
        .sync_all_public_lists(&manager)
        .await
        .expect_err("turning the switch back off closes lists again");
}
