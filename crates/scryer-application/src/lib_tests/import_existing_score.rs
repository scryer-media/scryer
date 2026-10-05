//! The `existing_score` an import hands to rules is the owned file judged by
//! the rules in force now, not the score it was stored with when it landed.

use super::*;
use crate::rules::workflow::tests::TestRuleSetRepo;

const OWNED_RELEASE: &str = "Synthetic.Feature.2024.1080p.WEB-DL.DDP5.1-SYNGRP";
const STORED_SCORE: i32 = 12_345;

async fn owned_movie() -> (AppUseCase, User, Title, Arc<MockMediaFileRepo>, String) {
    let (app, user) = bootstrap();
    let media_files = Arc::new(MockMediaFileRepo::default());
    let rules = Arc::new(TestRuleSetRepo::new(vec![]));
    let app = app.with_test_overrides(|services| {
        services
            .with_rule_sets(rules.clone())
            .with_media_files(media_files.clone())
    });
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Feature".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["scryer:quality-profile:1080p".into()],
                ..Default::default()
            },
        )
        .await
        .expect("seed synthetic movie");
    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/synthetic/library/Synthetic Feature (2024)/feature.mkv".into(),
            size_bytes: 6_000_000_000,
            grabbed_release_title: Some(OWNED_RELEASE.into()),
            acquisition_score: Some(STORED_SCORE),
            ..Default::default()
        })
        .await
        .expect("seed owned file");
    (app, user, title, media_files, file_id)
}

async fn import_existing_score(app: &AppUseCase, title: &Title) -> Option<i32> {
    let profile = app
        .resolve_quality_profile_for_title(title)
        .await
        .expect("resolve profile");
    let context = app.resolve_canonical_scoring_context(title, &profile).await;
    app.current_incumbent_score_for_import_scope(
        title,
        &crate::SubmissionScope::Title,
        &context,
        crate::quality_profile::CoverageSizeBasis::single(title.runtime_minutes)
            .total_runtime_minutes,
    )
    .await
}

#[tokio::test]
async fn a_rule_edit_changes_the_existing_score_without_reimporting_the_file() {
    let (app, user, title, media_files, file_id) = owned_movie().await;
    let baseline = import_existing_score(&app, &title)
        .await
        .expect("an owned file has a score");

    let rule = app
        .create_rule_set(
            &user,
            "Owned bonus".into(),
            String::new(),
            "score_entry[\"owned_bonus\"] := 250".into(),
            vec![MediaFacet::Movie],
            0,
            Some(true),
        )
        .await
        .expect("create rule");
    assert_eq!(
        import_existing_score(&app, &title).await,
        Some(baseline + 250),
        "a new rule reaches the owned file's score"
    );

    app.update_rule_set(
        &user,
        rule.id,
        None,
        None,
        Some("score_entry[\"owned_bonus\"] := 40".into()),
        None,
        None,
        None,
    )
    .await
    .expect("edit rule");
    assert_eq!(
        import_existing_score(&app, &title).await,
        Some(baseline + 40),
        "an edited rule reaches the owned file's score"
    );

    let stored = media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("read owned file")
        .expect("owned file exists");
    assert_eq!(
        stored.acquisition_score,
        Some(STORED_SCORE),
        "the stored score stays for display"
    );
}

#[tokio::test]
async fn an_unoccupied_scope_has_no_existing_score() {
    let (app, user) = bootstrap();
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = app.with_test_overrides(|services| services.with_media_files(media_files.clone()));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Empty Feature".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["scryer:quality-profile:1080p".into()],
                ..Default::default()
            },
        )
        .await
        .expect("seed synthetic movie");

    assert_eq!(import_existing_score(&app, &title).await, None);
}
