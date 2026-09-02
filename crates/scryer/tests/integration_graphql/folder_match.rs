//! GraphQL surface for folder-match correction (US1):
//! `changeTitleFolderPreview` and `applyTitleFolderChange`.

use super::*;
// The unmatched-item store is reached through its port here; the parent test
// module does not already have that trait in scope.
use scryer_application::LibraryScanUnmatchedItemRepository;

const PREVIEW_QUERY: &str = r#"
    query Preview($input: ChangeTitleFolderPreviewInput!) {
      changeTitleFolderPreview(input: $input) {
        title { id name folderPath }
        facet
        libraryId
        libraryName
        currentRootPath
        selectedFolderPath
        selectedRootId
        selectedRootPath
        ownership
        currentOwner { id name folderPath }
        currentFolderTrackedMediaCount
        selectedFolderTrackedMediaCount
        filesWillMove
        noOp
        availableResolutions
      }
    }
"#;

const APPLY_MUTATION: &str = r#"
    mutation Apply($input: ApplyTitleFolderChangeInput!) {
      applyTitleFolderChange(input: $input) {
        outcome
        title { id name folderPath }
        previousFolderPath
        detachedMediaFileCount
        scan { scanned matched imported skipped unmatched }
        swappedTitle { id name folderPath }
        swappedTitleScan { scanned }
        displacedTitle { id name previousFolderPath repairReasonCode }
      }
    }
"#;

async fn movie_title_in_folder(
    ctx: &TestContext,
    media_root: &std::path::Path,
    name: &str,
    folder: &std::path::Path,
) -> Title {
    configure_default_library_root(ctx, MediaFacet::Movie, media_root).await;
    let title = create_catalog_title(ctx, name, MediaFacet::Movie, vec![], vec![], true).await;
    set_title_folder_path(ctx, &title.id, folder).await;
    ctx.titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists")
}

fn make_folder(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let folder = root.join(name);
    std::fs::create_dir_all(&folder).expect("create folder");
    folder
}

fn write_media(folder: &std::path::Path, file_name: &str) -> std::path::PathBuf {
    let path = folder.join(file_name);
    std::fs::write(&path, vec![3_u8; 1024]).expect("write media file");
    path
}

async fn seed_media_row(ctx: &TestContext, title_id: &str, path: &std::path::Path) {
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 1024,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed media file");
}

async fn media_paths(ctx: &TestContext, title_id: &str) -> Vec<String> {
    let mut paths = ctx
        .media_files
        .list_media_files_for_title(title_id)
        .await
        .expect("list media files")
        .into_iter()
        .map(|file| file.file_path)
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

async fn folder_path_of(ctx: &TestContext, title_id: &str) -> Option<String> {
    ctx.titles
        .get_by_id(title_id)
        .await
        .expect("load title")
        .expect("title exists")
        .folder_path
        .filter(|folder| !folder.is_empty())
}

/// Content and mtime of every file under `root`, so a test can prove the
/// filesystem did not change (SC-001).
fn snapshot_tree(root: &std::path::Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>, std::time::SystemTime)>) {
        let mut entries = std::fs::read_dir(dir)
            .expect("read directory")
            .map(|entry| entry.expect("directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                let metadata = std::fs::metadata(&path).expect("file metadata");
                out.push((
                    path.to_string_lossy().to_string(),
                    std::fs::read(&path).expect("read file"),
                    metadata.modified().expect("file mtime"),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

#[tokio::test]
async fn graphql_change_title_folder_preview_reports_an_unowned_candidate() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let current = make_folder(media_root.path(), "Wrong Folder (2019)");
    let candidate = make_folder(media_root.path(), "Right Folder (2024)");
    let current_file = write_media(&current, "Wrong.Folder.2019.1080p.mkv");

    let title = movie_title_in_folder(&ctx, media_root.path(), "Right Folder", &current).await;
    seed_media_row(&ctx, &title.id, &current_file).await;

    let body = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": { "titleId": title.id, "folderPath": candidate.to_string_lossy() } }),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["changeTitleFolderPreview"];
    assert_eq!(preview["title"]["id"], title.id);
    assert_eq!(
        preview["title"]["folderPath"],
        current.to_string_lossy().to_string()
    );
    assert_eq!(preview["facet"], "MOVIE");
    assert_eq!(
        preview["selectedFolderPath"],
        candidate.to_string_lossy().to_string()
    );
    assert_eq!(
        preview["selectedRootPath"],
        media_root.path().to_string_lossy().to_string()
    );
    assert_eq!(preview["ownership"], "UNOWNED");
    assert!(preview["currentOwner"].is_null());
    assert_eq!(preview["currentFolderTrackedMediaCount"], 1);
    assert_eq!(preview["selectedFolderTrackedMediaCount"], 0);
    // FR-002: the dialog states plainly that nothing on disk moves.
    assert_eq!(preview["filesWillMove"], false);
    assert_eq!(preview["noOp"], false);
    assert_eq!(preview["availableResolutions"], json!(["ASSIGN"]));
}

#[tokio::test]
async fn graphql_change_title_folder_preview_rejects_folders_outside_the_library_roots() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let current = make_folder(media_root.path(), "Inside Root (2024)");

    let title = movie_title_in_folder(&ctx, media_root.path(), "Inside Root", &current).await;

    let body = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": { "titleId": title.id, "folderPath": outside.path().to_string_lossy() } }),
    )
    .await;
    let (message, _code) = first_graphql_error_message_and_code(&body);
    assert!(
        message.contains("is not inside a root"),
        "expected a root-scope error, got {message}"
    );
}

#[tokio::test]
async fn graphql_apply_title_folder_change_assigns_an_unowned_folder_without_touching_files() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let current = make_folder(media_root.path(), "Wrong Assign (2019)");
    let candidate = make_folder(media_root.path(), "Right Assign (2024)");
    let current_file = write_media(&current, "Wrong.Assign.2019.1080p.mkv");
    write_media(&candidate, "Right.Assign.2024.1080p.mkv");

    let title = movie_title_in_folder(&ctx, media_root.path(), "Right Assign", &current).await;
    seed_media_row(&ctx, &title.id, &current_file).await;

    let before = snapshot_tree(media_root.path());

    let body = gql(
        &ctx,
        APPLY_MUTATION,
        json!({ "input": { "titleId": title.id, "folderPath": candidate.to_string_lossy() } }),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["applyTitleFolderChange"];
    assert_eq!(payload["outcome"], "ASSIGNED");
    assert_eq!(
        payload["previousFolderPath"],
        current.to_string_lossy().to_string()
    );
    assert_eq!(payload["detachedMediaFileCount"], 1);
    assert_eq!(
        payload["title"]["folderPath"],
        candidate.to_string_lossy().to_string()
    );
    assert!(payload["scan"].is_object());
    assert!(payload["displacedTitle"].is_null());

    assert_eq!(
        folder_path_of(&ctx, &title.id).await,
        Some(candidate.to_string_lossy().to_string())
    );
    let paths = media_paths(&ctx, &title.id).await;
    assert!(
        !paths.contains(&current_file.to_string_lossy().to_string()),
        "old-folder association should be detached, got {paths:?}"
    );

    // The old folder went back to unmatched discovery: nothing claims it, so the
    // next scan offers it again. Asked through the same contract the dialog
    // uses, it now reads as unowned.
    let released = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": { "titleId": title.id, "folderPath": current.to_string_lossy() } }),
    )
    .await;
    assert_no_errors(&released);
    assert_eq!(
        released["data"]["changeTitleFolderPreview"]["ownership"],
        "UNOWNED"
    );

    // SC-001: the correction changed the catalog and nothing else.
    assert_eq!(snapshot_tree(media_root.path()), before);
}

#[tokio::test]
async fn graphql_apply_title_folder_change_is_an_explicit_no_op_for_the_owned_folder() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let current = make_folder(media_root.path(), "Already Owned (2024)");

    let title = movie_title_in_folder(&ctx, media_root.path(), "Already Owned", &current).await;

    let preview = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": { "titleId": title.id, "folderPath": current.to_string_lossy() } }),
    )
    .await;
    assert_no_errors(&preview);
    let preview = &preview["data"]["changeTitleFolderPreview"];
    assert_eq!(preview["ownership"], "OWNED_BY_THIS_TITLE");
    assert_eq!(preview["noOp"], true);
    assert_eq!(preview["availableResolutions"], json!([]));

    let body = gql(
        &ctx,
        APPLY_MUTATION,
        json!({ "input": { "titleId": title.id, "folderPath": current.to_string_lossy() } }),
    )
    .await;
    assert_no_errors(&body);
    let payload = &body["data"]["applyTitleFolderChange"];
    assert_eq!(payload["outcome"], "ALREADY_OWNED");
    assert!(payload["scan"].is_null());
    assert_eq!(payload["detachedMediaFileCount"], 0);
}

#[tokio::test]
async fn graphql_apply_title_folder_change_refuses_to_take_an_owned_folder_by_default() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let requester_folder = make_folder(media_root.path(), "Requester (2020)");
    let owned_folder = make_folder(media_root.path(), "Owner (2021)");

    let requester =
        movie_title_in_folder(&ctx, media_root.path(), "Requester", &requester_folder).await;
    let owner = create_catalog_title(&ctx, "Owner", MediaFacet::Movie, vec![], vec![], true).await;
    set_title_folder_path(&ctx, &owner.id, &owned_folder).await;

    let preview = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": { "titleId": requester.id, "folderPath": owned_folder.to_string_lossy() } }),
    )
    .await;
    assert_no_errors(&preview);
    let preview = &preview["data"]["changeTitleFolderPreview"];
    assert_eq!(preview["ownership"], "OWNED_BY_ANOTHER_TITLE");
    assert_eq!(preview["currentOwner"]["id"], owner.id);
    assert_eq!(preview["availableResolutions"], json!(["SWAP", "TAKE_OVER"]));

    let body = gql(
        &ctx,
        APPLY_MUTATION,
        json!({ "input": { "titleId": requester.id, "folderPath": owned_folder.to_string_lossy() } }),
    )
    .await;
    let (message, _code) = first_graphql_error_message_and_code(&body);
    assert!(
        message.contains("Owner") && message.contains("swap"),
        "the refusal should name the owner and the way forward, got {message}"
    );
    // FR-006: nothing was taken.
    assert_eq!(
        folder_path_of(&ctx, &owner.id).await,
        Some(owned_folder.to_string_lossy().to_string())
    );
    assert_eq!(
        folder_path_of(&ctx, &requester.id).await,
        Some(requester_folder.to_string_lossy().to_string())
    );
}

#[tokio::test]
async fn graphql_apply_title_folder_change_swaps_two_titles_folders() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let first_folder = make_folder(media_root.path(), "First Swap (2020)");
    let second_folder = make_folder(media_root.path(), "Second Swap (2021)");
    let first_file = write_media(&first_folder, "First.Swap.2020.1080p.mkv");
    let second_file = write_media(&second_folder, "Second.Swap.2021.1080p.mkv");

    let first = movie_title_in_folder(&ctx, media_root.path(), "First Swap", &first_folder).await;
    let second =
        create_catalog_title(&ctx, "Second Swap", MediaFacet::Movie, vec![], vec![], true).await;
    set_title_folder_path(&ctx, &second.id, &second_folder).await;
    seed_media_row(&ctx, &first.id, &first_file).await;
    seed_media_row(&ctx, &second.id, &second_file).await;

    let before = snapshot_tree(media_root.path());

    let body = gql(
        &ctx,
        APPLY_MUTATION,
        json!({ "input": {
            "titleId": first.id,
            "folderPath": second_folder.to_string_lossy(),
            "resolution": "SWAP"
        }}),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["applyTitleFolderChange"];
    assert_eq!(payload["outcome"], "SWAPPED");
    assert_eq!(
        payload["title"]["folderPath"],
        second_folder.to_string_lossy().to_string()
    );
    assert_eq!(payload["swappedTitle"]["id"], second.id);
    assert_eq!(
        payload["swappedTitle"]["folderPath"],
        first_folder.to_string_lossy().to_string()
    );
    assert!(payload["swappedTitleScan"].is_object());

    assert_eq!(
        folder_path_of(&ctx, &first.id).await,
        Some(second_folder.to_string_lossy().to_string())
    );
    assert_eq!(
        folder_path_of(&ctx, &second.id).await,
        Some(first_folder.to_string_lossy().to_string())
    );
    assert_eq!(snapshot_tree(media_root.path()), before);
}

#[tokio::test]
async fn graphql_apply_title_folder_change_takeover_surfaces_the_displaced_title_for_repair() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let taker_folder = make_folder(media_root.path(), "Taker (2020)");
    let owned_folder = make_folder(media_root.path(), "Displaced (2021)");
    let owned_file = write_media(&owned_folder, "Displaced.2021.1080p.mkv");

    let taker = movie_title_in_folder(&ctx, media_root.path(), "Taker", &taker_folder).await;
    let displaced =
        create_catalog_title(&ctx, "Displaced", MediaFacet::Movie, vec![], vec![], true).await;
    set_title_folder_path(&ctx, &displaced.id, &owned_folder).await;
    seed_media_row(&ctx, &displaced.id, &owned_file).await;

    let before = snapshot_tree(media_root.path());

    let body = gql(
        &ctx,
        APPLY_MUTATION,
        json!({ "input": {
            "titleId": taker.id,
            "folderPath": owned_folder.to_string_lossy(),
            "resolution": "TAKE_OVER"
        }}),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["applyTitleFolderChange"];
    assert_eq!(payload["outcome"], "TAKEN_OVER");
    assert_eq!(
        payload["title"]["folderPath"],
        owned_folder.to_string_lossy().to_string()
    );
    assert_eq!(payload["displacedTitle"]["id"], displaced.id);
    assert_eq!(
        payload["displacedTitle"]["previousFolderPath"],
        owned_folder.to_string_lossy().to_string()
    );
    assert_eq!(
        payload["displacedTitle"]["repairReasonCode"],
        "folder_ownership_changed_by_user"
    );

    assert_eq!(folder_path_of(&ctx, &displaced.id).await, None);
    assert!(media_paths(&ctx, &displaced.id).await.is_empty());

    // SC-008: discoverable in the repair experience with the documented reason.
    let unmatched = ctx
        .library_scan_unmatched
        .list_library_scan_unmatched_items(Some(MediaFacet::Movie), None, None, 50, 0)
        .await
        .expect("list unmatched items");
    let item = unmatched
        .iter()
        .find(|item| item.title_id.as_deref() == Some(displaced.id.as_str()))
        .expect("displaced title surfaces in unmatched discovery");
    assert_eq!(item.reason_code, "folder_ownership_changed_by_user");
    assert_eq!(item.item_path, owned_folder.to_string_lossy());

    assert_eq!(snapshot_tree(media_root.path()), before);
}

#[tokio::test]
async fn graphql_folder_match_surface_requires_library_management_permission() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let current = make_folder(media_root.path(), "Guarded (2024)");
    let candidate = make_folder(media_root.path(), "Guarded Target (2024)");

    let title = movie_title_in_folder(&ctx, media_root.path(), "Guarded", &current).await;

    let preview_query = format!(
        r#"query {{
            changeTitleFolderPreview(input: {{ titleId: "{}", folderPath: "{}" }}) {{ ownership }}
        }}"#,
        title.id,
        candidate.to_string_lossy()
    );
    let denied = schema_exec(&ctx, &preview_query, Some(folder_match_outsider())).await;
    assert_graphql_field_denied(&denied, "changeTitleFolderPreview");

    let apply_mutation = format!(
        r#"mutation {{
            applyTitleFolderChange(input: {{ titleId: "{}", folderPath: "{}" }}) {{ outcome }}
        }}"#,
        title.id,
        candidate.to_string_lossy()
    );
    let denied = schema_exec(&ctx, &apply_mutation, Some(folder_match_outsider())).await;
    assert_graphql_field_denied(&denied, "applyTitleFolderChange");

    // The refusal changed nothing.
    assert_eq!(
        folder_path_of(&ctx, &title.id).await,
        Some(current.to_string_lossy().to_string())
    );
}

fn folder_match_outsider() -> User {
    User {
        id: Id::new().0,
        username: "folder-match-outsider".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}
