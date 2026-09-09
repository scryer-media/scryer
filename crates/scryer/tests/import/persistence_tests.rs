use super::*;

#[tokio::test]
async fn catalog_write_failure_preserves_move_source_and_retry_imports_once() {
    exercise_catalog_write_retry(false).await;
}

#[tokio::test]
async fn catalog_write_retry_refuses_changed_bytes_and_preserves_both_files() {
    exercise_catalog_write_retry(true).await;
}

async fn exercise_catalog_write_retry(change_source: bool) {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx).await;
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "media".into(),
            scope: "system".into(),
            key_name: "import.mode".into(),
            data_type: "string".into(),
            default_value_json: "\"hardlink_or_copy\"".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .unwrap();
    ctx.settings_store
        .upsert_setting_value("system", "import.mode", None, "\"move\"", "test", None)
        .await
        .unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    let source = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Durable.Movie.2024.1080p.WEB-DL.H264.mkv",
    );
    let bytes = std::fs::read(&source).unwrap();
    let dest_root = tempfile::tempdir().unwrap();
    let title = add_movie_title(
        &ctx,
        "durable-import-title",
        "Durable Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let completed = scryer_completed(
        "durable-import",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );
    let import_id = queue_import_record(&ctx, &completed).await;
    sqlx::raw_sql(
        "CREATE TRIGGER reject_import_catalog_write BEFORE INSERT ON media_files
        BEGIN SELECT RAISE(ABORT, 'synthetic catalog write failure'); END",
    )
    .execute(ctx.db.pool())
    .await
    .unwrap();
    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![movie_manual_mapping(&source)],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 1);
    assert!(!results[0].success, "{results:#?}");
    assert!(
        results[0]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("synthetic catalog write failure"),
        "{results:#?}"
    );
    assert_eq!(
        std::fs::read(&source).unwrap(),
        bytes,
        "failed catalog write must retain the original source"
    );
    assert!(
        ctx.media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::raw_sql("DROP TRIGGER reject_import_catalog_write")
        .execute(ctx.db.pool())
        .await
        .unwrap();
    if change_source {
        let mut changed = bytes.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        // Move placement may hardlink the source until cleanup. Replace its
        // directory entry so this mutation cannot also change the destination.
        let replacement = source.with_extension("replacement");
        std::fs::write(&replacement, &changed).unwrap();
        std::fs::rename(&replacement, &source).unwrap();
        let error = scryer_application::execute_manual_import(
            &app,
            &user,
            &import_id,
            &title.id,
            Some(&completed),
            vec![movie_manual_mapping(&source)],
            Some(source_dir.path().to_path_buf()),
        )
        .await
        .expect_err("changed content must require reconciliation");
        assert!(
            matches!(
                error,
                scryer_application::AppError::ManualReconciliationRequired(_)
            ),
            "{error}"
        );
        assert_eq!(std::fs::read(&source).unwrap(), changed);
        assert!(
            ctx.media_files
                .list_media_files_for_title(&title.id)
                .await
                .unwrap()
                .is_empty()
        );
        // Restoring the original source permits recovery against the untouched
        // destination, proving the rejected retry did not overwrite it.
        std::fs::write(&source, &bytes).unwrap();
    }
    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &import_id,
        &title.id,
        Some(&completed),
        vec![movie_manual_mapping(&source)],
        Some(source_dir.path().to_path_buf()),
    )
    .await
    .unwrap();
    assert!(results[0].success, "{results:#?}");
    let files = ctx
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(&files[0].file_path).unwrap(), bytes);
    assert!(
        !source.exists(),
        "successful move removes the source only after catalog persistence"
    );
}
