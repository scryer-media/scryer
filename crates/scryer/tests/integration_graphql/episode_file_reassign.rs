use super::*;

/// One synthetic series with three episodes in season one, a recorded title
/// folder under a temp library root, and the season folder that holds files.
struct ReassignFixture {
    _media_root: tempfile::TempDir,
    title: Title,
    episodes: Vec<Episode>,
    season_dir: std::path::PathBuf,
}

/// A synthetic series title with three season-one episodes, in the default
/// series library.
async fn series_with_three_episodes(
    ctx: &TestContext,
    name: &str,
    tvdb_id: &str,
) -> (Title, Vec<Episode>) {
    let title = create_catalog_title(
        ctx,
        name,
        MediaFacet::Series,
        vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
        vec![],
        true,
    )
    .await;
    let collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");
    let mut episodes = Vec::new();
    for number in 1..=3 {
        episodes.push(
            create_series_scan_episode(
                ctx,
                &title,
                &collection,
                "1",
                &number.to_string(),
                &format!("S01E{number:02}"),
            )
            .await,
        );
    }
    (title, episodes)
}

async fn reassign_fixture(ctx: &TestContext, name: &str, tvdb_id: &str) -> ReassignFixture {
    seed_typed_settings_definitions(ctx).await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    configure_default_library_root(ctx, MediaFacet::Series, media_root.path()).await;
    let (title, episodes) = series_with_three_episodes(ctx, name, tvdb_id).await;
    let title_dir = media_root.path().join(format!("{name} (2024)"));
    let season_dir = title_dir.join("Season 1");
    std::fs::create_dir_all(&season_dir).expect("create season dir");
    set_title_folder_path(ctx, &title.id, &title_dir).await;
    ReassignFixture {
        _media_root: media_root,
        title,
        episodes,
        season_dir,
    }
}

/// Write a synthetic file into the season folder and link it as the primary
/// file of `episode`.
async fn seed_primary_file(
    ctx: &TestContext,
    fixture: &ReassignFixture,
    file_name: &str,
    episode: &Episode,
) -> (String, std::path::PathBuf) {
    let path = fixture.season_dir.join(file_name);
    let bytes = format!("synthetic {file_name}").into_bytes();
    std::fs::write(&path, &bytes).expect("write synthetic file");
    let file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: path.to_string_lossy().into_owned(),
            size_bytes: bytes.len() as i64,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    ctx.link_primary_file_to_episode(&fixture.title.id, &file_id, &episode.id)
        .await
        .expect("link primary file");
    (file_id, path)
}

/// Every (episode, role) link row of one file, sorted.
async fn file_links(ctx: &TestContext, title_id: &str, file_id: &str) -> Vec<(String, String)> {
    let mut links = ctx
        .media_files
        .list_media_files_for_title(title_id)
        .await
        .expect("list title media files")
        .into_iter()
        .filter(|file| file.id == file_id)
        .filter_map(|file| {
            file.episode_id
                .map(|episode_id| (episode_id, file.role.as_str().to_string()))
        })
        .collect::<Vec<_>>();
    links.sort();
    links
}

fn app_with_filesystem_renamer(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder.with_library_renamer(std::sync::Arc::new(FileSystemLibraryRenamer::new()))
    })
}

const REASSIGN_MUTATION: &str = r#"
    mutation Reassign($input: ReassignEpisodeFileInput!) {
      reassignEpisodeFile(input: $input) {
        previousPath
        episodeIds
        file { id filePath }
      }
    }
"#;

#[tokio::test]
async fn reassign_episode_file_renames_the_file_on_disk_and_records_history() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Reassign Fixture", "97101").await;
    let (file_id, old_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Reassign.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;
    let bytes = std::fs::read(&old_path).expect("read seeded file");
    let app = app_with_filesystem_renamer(&ctx);
    let actor = app.find_or_create_default_user().await.expect("actor");

    let result = app
        .reassign_episode_file(
            &actor,
            &file_id,
            std::slice::from_ref(&fixture.episodes[1].id),
            false,
        )
        .await
        .expect("reassign file");

    let new_path = std::path::PathBuf::from(&result.file.file_path);
    assert_eq!(result.previous_path, old_path.to_string_lossy());
    assert_ne!(new_path, old_path);
    assert!(!old_path.exists(), "the old path no longer exists");
    assert_eq!(std::fs::read(&new_path).expect("read renamed file"), bytes);
    let file_name = new_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("file name");
    assert!(
        file_name.contains("S01E02"),
        "renamed for the new episode: {file_name}"
    );
    assert!(
        !file_name.contains("S01E01"),
        "old episode gone from the name: {file_name}"
    );
    assert!(new_path.starts_with(fixture.season_dir.parent().expect("title folder")));
    assert_eq!(
        result.previous_episode_ids,
        vec![fixture.episodes[0].id.clone()]
    );
    assert_eq!(result.episode_ids, vec![fixture.episodes[1].id.clone()]);

    // The stored path is the one a fresh rename of the new episode renders.
    let plan = app
        .preview_rename_for_title(&actor, &fixture.title.id, MediaFacet::Series)
        .await
        .expect("rename preview");
    let item = plan
        .items
        .iter()
        .find(|item| item.media_file_id.as_deref() == Some(file_id.as_str()))
        .expect("plan item for the file");
    assert_eq!(item.current_path, result.file.file_path);
    assert_eq!(
        item.write_action,
        scryer_application::RenameWriteAction::Noop,
        "the file already sits at its rendered name: {:?}",
        item.proposed_path
    );

    let stored = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("read file")
        .expect("file row");
    assert_eq!(stored.file_path, result.file.file_path);
    assert_eq!(stored.size_bytes, bytes.len() as i64);
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &file_id).await,
        vec![(fixture.episodes[1].id.clone(), "primary".to_string())]
    );

    let history = gql(
        &ctx,
        r#"
        query TitleHistory($titleId: ID!) {
          titleHistory(filter: { titleIds: [$titleId], limit: 10 }) {
            items { eventType destPath }
          }
        }
        "#,
        json!({ "titleId": fixture.title.id }),
    )
    .await;
    assert_no_errors(&history);
    let items = history["data"]["titleHistory"]["items"]
        .as_array()
        .expect("history items");
    assert!(
        items.iter().any(|item| item["eventType"] == "rematched"
            && item["destPath"] == json!(result.file.file_path)),
        "reassignment recorded in title history: {items:?}"
    );

    let files = gql(
        &ctx,
        r#"
        query Title($id: ID!) {
          title(id: $id) { mediaFiles { id episodeId } }
        }
        "#,
        json!({ "id": fixture.title.id }),
    )
    .await;
    assert_no_errors(&files);
    let media_files = files["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files");
    assert!(
        media_files.iter().any(|file| file["id"] == json!(file_id)
            && file["episodeId"] == json!(fixture.episodes[1].id))
    );

    // The reassignment leaves nothing behind that holds the links: a later
    // re-link pass may still re-derive them.
    let replaced = ctx
        .media_files
        .replace_file_episode_links(
            &file_id,
            std::slice::from_ref(&fixture.episodes[1].id),
            std::slice::from_ref(&fixture.episodes[0].id),
        )
        .await
        .expect("replace links");
    assert_eq!(
        replaced,
        scryer_application::EpisodeLinkReplacement::Replaced
    );
}

#[tokio::test]
async fn reassign_episode_file_with_replace_demotes_the_existing_primary() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Replace Fixture", "97102").await;
    let (moving_id, moving_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Replace.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;
    let (holder_id, holder_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Replace.Fixture.S01E02.720p.HDTV.mkv",
        &fixture.episodes[1],
    )
    .await;
    let app = app_with_filesystem_renamer(&ctx);
    let actor = app.find_or_create_default_user().await.expect("actor");

    let result = app
        .reassign_episode_file(
            &actor,
            &moving_id,
            std::slice::from_ref(&fixture.episodes[1].id),
            true,
        )
        .await
        .expect("reassign with replace");

    assert_eq!(result.demoted_file_ids, vec![holder_id.clone()]);
    assert!(!moving_path.exists());
    assert!(std::path::Path::new(&result.file.file_path).exists());
    assert!(holder_path.exists(), "the demoted file is never deleted");
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &moving_id).await,
        vec![(fixture.episodes[1].id.clone(), "primary".to_string())]
    );
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &holder_id).await,
        vec![(fixture.episodes[1].id.clone(), "additional".to_string())]
    );
    let first_episode_files = ctx
        .media_files
        .list_media_files_for_title(&fixture.title.id)
        .await
        .expect("list files")
        .into_iter()
        .filter(|file| file.episode_id.as_deref() == Some(fixture.episodes[0].id.as_str()))
        .count();
    assert_eq!(
        first_episode_files, 0,
        "the episode the file left is now missing"
    );
}

#[tokio::test]
async fn graphql_reassign_episode_file_refuses_a_primary_conflict_with_typed_facts() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Conflict Fixture", "97103").await;
    let (moving_id, moving_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Conflict.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;
    let (holder_id, _) = seed_primary_file(
        &ctx,
        &fixture,
        "Conflict.Fixture.S01E02.1080p.WEB-DL.mkv",
        &fixture.episodes[1],
    )
    .await;

    let body = gql(
        &ctx,
        REASSIGN_MUTATION,
        json!({ "input": { "fileId": moving_id, "episodeIds": [fixture.episodes[1].id] } }),
    )
    .await;
    let extensions = &body["errors"][0]["extensions"];
    assert_eq!(extensions["code"], "VALIDATION_ERROR", "{body}");
    assert_eq!(
        extensions["reason"],
        "EPISODE_FILE_REASSIGN_PRIMARY_CONFLICT"
    );
    assert_eq!(extensions["conflictingFileId"], json!(holder_id));
    assert_eq!(extensions["episodeId"], json!(fixture.episodes[1].id));
    assert!(moving_path.exists());
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &moving_id).await,
        vec![(fixture.episodes[0].id.clone(), "primary".to_string())]
    );
}

#[tokio::test]
async fn graphql_reassign_episode_file_refuses_an_episode_of_another_title() {
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Owner Fixture", "97104").await;
    let (_, other_episodes) = series_with_three_episodes(&ctx, "Other Fixture", "97105").await;
    let (file_id, file_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Owner.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;

    let body = gql(
        &ctx,
        REASSIGN_MUTATION,
        json!({ "input": { "fileId": file_id, "episodeIds": [other_episodes[1].id] } }),
    )
    .await;
    let extensions = &body["errors"][0]["extensions"];
    assert_eq!(
        extensions["reason"], "EPISODE_FILE_REASSIGN_EPISODE_OUTSIDE_TITLE",
        "{body}"
    );
    assert_eq!(extensions["episodeId"], json!(other_episodes[1].id));
    assert!(file_path.exists());
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &file_id).await,
        vec![(fixture.episodes[0].id.clone(), "primary".to_string())]
    );
}

#[tokio::test]
async fn graphql_reassign_episode_file_puts_the_links_back_when_the_rename_fails() {
    // The served app has no library renamer configured, so the move fails
    // after the links have moved; the links must return to the old episode.
    let ctx = TestContext::new().await;
    let fixture = reassign_fixture(&ctx, "Rollback Fixture", "97106").await;
    let (file_id, file_path) = seed_primary_file(
        &ctx,
        &fixture,
        "Rollback.Fixture.S01E01.1080p.WEB-DL.mkv",
        &fixture.episodes[0],
    )
    .await;

    let body = gql(
        &ctx,
        REASSIGN_MUTATION,
        json!({ "input": { "fileId": file_id, "episodeIds": [fixture.episodes[2].id] } }),
    )
    .await;
    assert_eq!(
        body["errors"][0]["extensions"]["reason"], "EPISODE_FILE_REASSIGN_RENAME_FAILED",
        "{body}"
    );
    assert!(file_path.exists(), "the file stays where it was");
    let stored = ctx
        .media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("read file")
        .expect("file row");
    assert_eq!(stored.file_path, file_path.to_string_lossy());
    assert_eq!(
        file_links(&ctx, &fixture.title.id, &file_id).await,
        vec![(fixture.episodes[0].id.clone(), "primary".to_string())]
    );
}
