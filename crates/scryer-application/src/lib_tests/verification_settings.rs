use super::*;

/// FR-042: full verification is the default, so an install that has never
/// touched the preference gets the strong guarantee.
#[tokio::test]
async fn verification_depth_defaults_to_full() {
    let (app, actor) = bootstrap();

    let settings = app
        .get_verification_settings(&actor)
        .await
        .expect("read verification settings");

    assert_eq!(settings.depth, VerificationDepth::Full);
    assert_eq!(
        app.resolve_verification_depth().await,
        VerificationDepth::Full
    );
}

/// The preference round-trips through the settings store in both directions.
#[tokio::test]
async fn verification_depth_round_trips_through_settings() {
    let (app, actor) = bootstrap();

    let saved = app
        .update_verification_settings(
            &actor,
            UpdateVerificationSettings {
                depth: VerificationDepth::Quick,
            },
        )
        .await
        .expect("save quick verification depth");
    assert_eq!(saved.depth, VerificationDepth::Quick);
    assert_eq!(
        app.get_verification_settings(&actor)
            .await
            .expect("read back quick depth")
            .depth,
        VerificationDepth::Quick
    );
    assert_eq!(
        app.resolve_verification_depth().await,
        VerificationDepth::Quick
    );

    let restored = app
        .update_verification_settings(
            &actor,
            UpdateVerificationSettings {
                depth: VerificationDepth::Full,
            },
        )
        .await
        .expect("save full verification depth");
    assert_eq!(restored.depth, VerificationDepth::Full);
    assert_eq!(
        app.resolve_verification_depth().await,
        VerificationDepth::Full
    );
}

/// A corrupt or unknown stored value must never silently weaken verification:
/// the resolver falls back to the `full` default (FR-042's floor rule read in
/// the safe direction).
#[tokio::test]
async fn unparseable_verification_depth_falls_back_to_full() {
    let (app, actor) = bootstrap();

    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_MEDIA,
            VERIFICATION_DEPTH_KEY,
            None,
            "\"shallow\"".to_string(),
            SETTINGS_SOURCE_TYPED_GRAPHQL,
            None,
        )
        .await
        .expect("seed unsupported depth value");

    assert_eq!(
        app.resolve_verification_depth().await,
        VerificationDepth::Full
    );
    assert_eq!(
        app.get_verification_settings(&actor)
            .await
            .expect("read verification settings")
            .depth,
        VerificationDepth::Full
    );
}

/// Writing the preference requires the system-settings permission (C-side of
/// the recycle-bin pattern this follows).
#[tokio::test]
async fn updating_verification_depth_requires_system_settings_permission() {
    let (app, _) = bootstrap();
    let viewer =
        test_user_with_app_permissions("verification-depth-viewer", AppPermissionMask::NONE);

    let error = app
        .update_verification_settings(
            &viewer,
            UpdateVerificationSettings {
                depth: VerificationDepth::Quick,
            },
        )
        .await
        .expect_err("viewer must not be able to weaken verification");

    assert!(matches!(error, AppError::Unauthorized(_)), "got {error:?}");
    assert_eq!(
        app.resolve_verification_depth().await,
        VerificationDepth::Full
    );
}
