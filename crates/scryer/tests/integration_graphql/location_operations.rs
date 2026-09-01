//! GraphQL surface for root-move location operations (US2):
//! `locationOperationPreview`, `startLocationOperation`,
//! `cancelLocationOperation`, `resumeLocationOperation`, and `locationOperation`.

use super::*;
// The operation store is reached through its port here so a test can seed a
// queued operation without racing the runner that a real start spawns.
use scryer_application::LocationOperationRepository;
use scryer_application::location::model::{
    LocationExecutionMode, LocationOperation, LocationOperationCounters, LocationOperationState,
    LocationOperationType, VerificationDepth,
};
use scryer_infrastructure_library::media::libraries::location_operation_store::LocationOperationStore;

const PREVIEW_QUERY: &str = r#"
    query Preview($input: LocationOperationPreviewInput!) {
      locationOperationPreview(input: $input) {
        planFingerprint
        operationType
        mode
        sourceLibraryId
        destinationLibraryId
        sourceRootId
        destinationRootId
        selection
        blocksStart
        warnings
        counts {
          itemsTotal
          titlesTotal
          filesTotal
          bytesTotal
          byKind { kind count }
        }
        sections {
          kind
          itemsTotal
          bytesTotal
          complete
          items { kind titleId sourcePath destinationPath sizeBytes sameVolume reasonCode detail }
        }
        classification {
          titlesTotal
          blocksStart
          groups {
            class
            count
            titles {
              titleId
              class
              sourceLibraryId
              sourceRootId
              sourceFolderPath
              destinationLibraryId
              destinationRootId
              reasonCode
              reason
              blocksStart
              destinationIdentityMatch
              mergeTargetTitleId
              mergeTargetTitleName
              sameNamedDestinationTitleId
              sameNamedDestinationTitleName
              ambiguousDestinationTitleIds
              ambiguousDestinationCandidates { titleId titleName sharedIdentities }
            }
          }
        }
        freeSpace {
          destinationRequiredBytes
          destinationTotalRequiredBytes
          sameVolumeMove
          recyclingAvailable
          probed
          sufficient
        }
        verification { depth files bytes applies }
        confirmation { requirement typedPhrase typedPrompt }
        merges {
          sourceTitleId
          destinationTitleId
          destinationTitleName
          sourceLibraryId
          destinationLibraryId
          blocked
          blockedRecords { table reason sourceId detail }
          destinationWins { setting destinationValue sourceValue }
          dispositions { table disposition sourceRowCount note }
          roleChanges {
            fileId
            sourceEpisodeId
            destinationEpisodeId
            previousRole
            newRole
            reason
            detail
          }
          reservedTagConflicts { prefix setting destinationValue sourceValue }
          freeFormTagsAdded
          mediaRequestRepoints { requestId previousLibraryId destinationLibraryId }
          dropped { table sourceRowCount decision reason }
          postMergeWork
          notes
        }
      }
    }
"#;

const START_MUTATION: &str = r#"
    mutation Start($input: StartLocationOperationInput!) {
      startLocationOperation(input: $input) {
        planFingerprint
        operation {
          id
          operationType
          mode
          state
          sourceLibraryId
          destinationRootId
          planFingerprint
          verificationDepth
          verificationFallbackCount
          counters { titlesTotal filesTotal bytesTotal }
          titleCheckpoints { titleId state }
        }
      }
    }
"#;

const OPERATION_QUERY: &str = r#"
    query Operation($id: ID!) {
      locationOperation(id: $id) {
        id
        operationType
        mode
        state
        sourceLibraryId
        destinationLibraryId
        sourceRootId
        destinationRootId
        planFingerprint
        verificationDepth
        verificationFallbackCount
        cancelRequested
        detail
        counters { titlesTotal filesTotal bytesTotal titlesProcessed }
        titleCheckpoints {
          titleId
          sequence
          state
          classification
          sourceFolderPath
          destinationFolderPath
          mergedIntoTitleId
          mergedIntoTitleName
        }
      }
    }
"#;

const CANCEL_MUTATION: &str = r#"
    mutation Cancel($id: ID!) {
      cancelLocationOperation(id: $id) { id cancelRequested }
    }
"#;

const RESUME_MUTATION: &str = r#"
    mutation Resume($id: ID!) {
      resumeLocationOperation(id: $id) { id resumed detail }
    }
"#;

/// Two roots on the movie library, the first of them the default, so a
/// same-library root move has somewhere to move from and to.
async fn configure_two_movie_roots(
    ctx: &TestContext,
    first: &std::path::Path,
    second: &std::path::Path,
) -> (String, String) {
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let library = ctx
        .libraries
        .get_by_id(&library_id)
        .await
        .expect("lookup default movie library")
        .expect("default movie library exists");
    let updated = ctx
        .libraries
        .update(
            &library_id,
            library.name,
            library.slug,
            vec![
                LibraryRootDraft {
                    path: first.to_string_lossy().to_string(),
                    is_default: true,
                },
                LibraryRootDraft {
                    path: second.to_string_lossy().to_string(),
                    is_default: false,
                },
            ],
        )
        .await
        .expect("configure two movie roots");
    let root_id_for = |path: &std::path::Path| {
        updated
            .roots
            .iter()
            .find(|root| root.path == path.to_string_lossy())
            .map(|root| root.id.clone())
            .expect("configured root should expose its id")
    };
    (root_id_for(first), root_id_for(second))
}

fn make_folder(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let folder = root.join(name);
    std::fs::create_dir_all(&folder).expect("create folder");
    folder
}

fn write_media(folder: &std::path::Path, file_name: &str) -> std::path::PathBuf {
    let path = folder.join(file_name);
    std::fs::write(&path, vec![7_u8; 512]).expect("write media file");
    path
}

async fn seed_media_row(ctx: &TestContext, title_id: &str, path: &std::path::Path) {
    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 512,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("seed media file");
}

/// A movie title that owns a folder on the current default root and has one
/// tracked file inside it: the shape a root move actually moves.
async fn movable_title(
    ctx: &TestContext,
    name: &str,
    folder: &std::path::Path,
    file_name: &str,
) -> Title {
    let title = create_catalog_title(ctx, name, MediaFacet::Movie, vec![], vec![], true).await;
    set_title_folder_path(ctx, &title.id, folder).await;
    let file = write_media(folder, file_name);
    seed_media_row(ctx, &title.id, &file).await;
    ctx.titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists")
}

async fn move_title_to_root(ctx: &TestContext, title_id: &str, root_id: &str) {
    ctx.titles
        .update_metadata(title_id, None, None, None, Some(root_id.to_string()))
        .await
        .expect("place title on root");
}

fn group<'a>(preview: &'a Value, class: &str) -> &'a Value {
    preview["classification"]["groups"]
        .as_array()
        .expect("classification groups")
        .iter()
        .find(|group| group["class"] == class)
        .unwrap_or_else(|| panic!("class {class} should have a group: {preview}"))
}

/// Seeds one queued operation directly, so cancel and resume can be asserted
/// without racing the background runner a real start spawns.
async fn seed_queued_operation(ctx: &TestContext, library_id: &str, root_id: &str) -> String {
    let store = LocationOperationStore::new(ctx.db.datastore());
    let now = chrono::Utc::now();
    let operation = LocationOperation {
        id: Id::new().0,
        operation_type: LocationOperationType::RootMove,
        mode: LocationExecutionMode::MoveWithScryer,
        state: LocationOperationState::Queued,
        initiated_by_user_id: None,
        source_library_id: Some(library_id.to_string()),
        destination_library_id: Some(library_id.to_string()),
        source_root_id: Some(root_id.to_string()),
        destination_root_id: Some(root_id.to_string()),
        plan_fingerprint: "seeded-fingerprint".to_string(),
        verification_depth: VerificationDepth::Full,
        verification_fallback_count: 0,
        counters: LocationOperationCounters {
            titles_total: 1,
            files_total: 1,
            bytes_total: 512,
            ..LocationOperationCounters::default()
        },
        detail: None,
        job_run_id: None,
        workflow_operation_id: None,
        cancel_requested: false,
        cancel_requested_at: None,
        confirmed_at: Some(now),
        started_at: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    store
        .create_location_operation(&operation, None)
        .await
        .expect("seed location operation");
    operation.id
}

#[tokio::test]
async fn graphql_location_operation_preview_classifies_a_mixed_selection() {
    let ctx = TestContext::new().await;
    let first_root = tempfile::tempdir().expect("first root tempdir");
    let second_root = tempfile::tempdir().expect("second root tempdir");
    let (_source_root_id, destination_root_id) =
        configure_two_movie_roots(&ctx, first_root.path(), second_root.path()).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    // One title that actually moves.
    let moving_folder = make_folder(first_root.path(), "Moving Movie (2024)");
    let moving = movable_title(&ctx, "Moving Movie", &moving_folder, "Moving.2024.mkv").await;

    // One title already sitting on the destination root: a no-op.
    let settled_folder = make_folder(second_root.path(), "Settled Movie (2023)");
    let settled = movable_title(&ctx, "Settled Movie", &settled_folder, "Settled.2023.mkv").await;
    move_title_to_root(&ctx, &settled.id, &destination_root_id).await;

    // One monitored title with no tracked files: the catalog-only fast path.
    let fileless = create_catalog_title(
        &ctx,
        "Fileless Movie",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": {
            "titleIds": [moving.id, settled.id, fileless.id],
            "destination": { "rootId": destination_root_id }
        }}),
    )
    .await;
    assert_no_errors(&body);

    let preview = &body["data"]["locationOperationPreview"];
    assert_eq!(preview["operationType"], "ROOT_MOVE");
    assert_eq!(preview["sourceLibraryId"], library_id);
    assert_eq!(preview["destinationLibraryId"], library_id);
    assert_eq!(preview["destinationRootId"], destination_root_id);
    assert!(
        preview["planFingerprint"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "a preview always carries the fingerprint its confirmation is checked against: {preview}"
    );
    // The selection spans two roots, so the plan claims neither as its source.
    assert!(preview["sourceRootId"].is_null());

    // FR-015: every selected title is classified exactly once, and all six
    // groups are present whether or not the selection populated them.
    assert_eq!(preview["classification"]["titlesTotal"], 3);
    assert_eq!(preview["classification"]["blocksStart"], false);
    assert_eq!(preview["blocksStart"], false);
    assert_eq!(
        preview["classification"]["groups"]
            .as_array()
            .expect("groups")
            .len(),
        6
    );
    let root_move = group(preview, "ROOT_MOVE");
    assert_eq!(root_move["count"], 1);
    assert_eq!(root_move["titles"][0]["titleId"], moving.id);
    assert_eq!(
        root_move["titles"][0]["destinationRootId"],
        destination_root_id
    );
    assert_eq!(root_move["titles"][0]["blocksStart"], false);

    let no_op = group(preview, "NO_OP");
    assert_eq!(no_op["count"], 1);
    assert_eq!(no_op["titles"][0]["titleId"], settled.id);
    assert_eq!(no_op["titles"][0]["reasonCode"], "already_at_destination");

    let catalog_only = group(preview, "CATALOG_ONLY");
    assert_eq!(catalog_only["count"], 1);
    assert_eq!(catalog_only["titles"][0]["titleId"], fileless.id);
    assert_eq!(catalog_only["titles"][0]["reasonCode"], "no_tracked_files");

    // FR-012 / US2.1: every classified title states where it lives now, not
    // only where it would go — including the no-op and the fileless
    // catalog-only title, neither of which contributes a plan item to read a
    // source path off.
    assert_eq!(root_move["titles"][0]["sourceLibraryId"], library_id);
    assert_eq!(
        root_move["titles"][0]["sourceFolderPath"],
        moving_folder.to_string_lossy().to_string()
    );
    assert_eq!(no_op["titles"][0]["sourceRootId"], destination_root_id);
    assert_eq!(
        no_op["titles"][0]["sourceFolderPath"],
        settled_folder.to_string_lossy().to_string()
    );
    // A title with no files owns no folder, so the current folder is null
    // rather than invented.
    assert!(
        catalog_only["titles"][0]["sourceFolderPath"].is_null(),
        "a fileless title has no current folder: {catalog_only}"
    );

    for empty in ["CROSS_LIBRARY_TRANSFER", "INCOMPATIBLE", "NEEDS_RESOLUTION"] {
        let group = group(preview, empty);
        assert_eq!(group["count"], 0, "{empty} should be empty: {preview}");
        assert_eq!(group["titles"], json!([]));
    }

    // FR-080: the plan states its sections with complete counts, the space it
    // needs, and the verification depth that will apply.
    let destination_prefix = second_root.path().to_string_lossy().to_string();
    let move_section = preview["sections"]
        .as_array()
        .expect("plan sections")
        .iter()
        .find(|section| section["kind"] == "MOVE")
        .expect("the moving title contributes a move section");
    assert_eq!(move_section["complete"], true);
    assert!(
        move_section["items"]
            .as_array()
            .expect("move items")
            .iter()
            .any(|item| item["titleId"] == moving.id.as_str()
                && item["destinationPath"]
                    .as_str()
                    .is_some_and(|path| path.starts_with(destination_prefix.as_str()))),
        "the moving title's destination is calculated under the destination root: {move_section}"
    );
    assert!(preview["counts"]["filesTotal"].as_i64().unwrap_or(0) >= 1);
    assert!(
        preview["counts"]["byKind"]
            .as_array()
            .expect("per-kind counts")
            .len()
            == 10
    );
    // FR-043: the depth is stated before anything moves, and the statement says
    // how much it covers. A same-volume rename copies nothing, so the depth
    // applies to no files (FR-032); the statement stays consistent either way.
    assert_eq!(preview["verification"]["depth"], "FULL");
    let verified_files = preview["verification"]["files"]
        .as_i64()
        .expect("verification file count");
    assert_eq!(preview["verification"]["applies"], verified_files > 0);
    // A title-scoped root move is not root-wide, so a simple confirmation is enough.
    assert_eq!(preview["confirmation"]["requirement"], "SIMPLE");
    assert!(preview["confirmation"]["typedPhrase"].is_null());
    assert_eq!(preview["freeSpace"]["probed"], true);
}

#[tokio::test]
async fn graphql_start_location_operation_refuses_a_stale_fingerprint() {
    let ctx = TestContext::new().await;
    let first_root = tempfile::tempdir().expect("first root tempdir");
    let second_root = tempfile::tempdir().expect("second root tempdir");
    let (_source_root_id, destination_root_id) =
        configure_two_movie_roots(&ctx, first_root.path(), second_root.path()).await;

    let folder = make_folder(first_root.path(), "Stale Confirm (2024)");
    let title = movable_title(&ctx, "Stale Confirm", &folder, "Stale.2024.mkv").await;

    let before = std::fs::read_dir(second_root.path())
        .expect("read destination root")
        .count();

    let body = gql(
        &ctx,
        START_MUTATION,
        json!({ "input": {
            "titleIds": [title.id],
            "destination": { "rootId": destination_root_id },
            "planFingerprint": "a-fingerprint-from-some-other-plan"
        }}),
    )
    .await;
    let (message, code) = first_graphql_error_message_and_code(&body);
    assert!(
        message.contains("no longer matches"),
        "a stale confirmation should send the user back to a fresh preview, got {message}"
    );
    // FR-081: the refusal is machine-readable, so the client re-previews on
    // `stale_plan` without parsing the sentence it also shows.
    assert_eq!(code, "LOCATION_PLAN_REFUSED", "{body}");
    assert_eq!(
        body["errors"][0]["extensions"]["refusalCode"], "stale_plan",
        "{body}"
    );

    // FR-081: nothing was started, so nothing landed on the destination root.
    assert_eq!(
        std::fs::read_dir(second_root.path())
            .expect("read destination root")
            .count(),
        before
    );
}

/// FR-080 now gates the start: a destination the preview measured as too small
/// refuses the confirmation. The half that has to keep working is this one — a
/// same-volume move is a rename, needs no destination space at all, and must
/// not be refused however large the content is.
///
/// The refusing half is not reachable from here: making
/// `FreeSpaceEstimate::sufficient()` answer `false` needs a source and a
/// destination on genuinely different volumes, which a temp-directory fixture
/// cannot stage. That decision is covered where it is made, in
/// `location::preview`'s confirmation tests.
#[tokio::test]
async fn graphql_a_same_volume_move_is_never_refused_for_space() {
    let ctx = TestContext::new().await;
    let first_root = tempfile::tempdir().expect("first root tempdir");
    let second_root = tempfile::tempdir().expect("second root tempdir");
    let (_source_root_id, destination_root_id) =
        configure_two_movie_roots(&ctx, first_root.path(), second_root.path()).await;

    let folder = make_folder(first_root.path(), "Enormous Movie (2024)");
    let title = movable_title(&ctx, "Enormous Movie", &folder, "Enormous.2024.mkv").await;

    // A sparse file: the plan reads its logical length, so this is 8 TiB of
    // planned content without a byte written. 8 TiB stays inside ext4's
    // per-file ceiling, so the fixture behaves the same on every filesystem CI
    // runs on.
    std::fs::OpenOptions::new()
        .write(true)
        .open(folder.join("Enormous.2024.mkv"))
        .expect("open the fixture file")
        .set_len(8 * 1024 * 1024 * 1024 * 1024)
        .expect("grow the fixture file sparsely");

    let preview = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": {
            "titleIds": [title.id],
            "destination": { "rootId": destination_root_id }
        }}),
    )
    .await;
    assert_no_errors(&preview);
    let previewed = &preview["data"]["locationOperationPreview"];
    assert_eq!(previewed["counts"]["bytesTotal"], 8_796_093_022_208_i64);
    assert_eq!(previewed["freeSpace"]["probed"], true);
    assert_eq!(previewed["freeSpace"]["sameVolumeMove"], true);
    assert_eq!(previewed["freeSpace"]["destinationTotalRequiredBytes"], 0);
    assert_eq!(
        previewed["freeSpace"]["sufficient"], true,
        "a rename needs no destination space, whatever it is renaming: {previewed}"
    );

    let fingerprint = previewed["planFingerprint"]
        .as_str()
        .expect("preview fingerprint")
        .to_string();
    let body = gql(
        &ctx,
        START_MUTATION,
        json!({ "input": {
            "titleIds": [title.id],
            "destination": { "rootId": destination_root_id },
            "planFingerprint": fingerprint
        }}),
    )
    .await;
    assert_no_errors(&body);
}

#[tokio::test]
async fn graphql_start_location_operation_accepts_the_previewed_plan() {
    let ctx = TestContext::new().await;
    let first_root = tempfile::tempdir().expect("first root tempdir");
    let second_root = tempfile::tempdir().expect("second root tempdir");
    let (source_root_id, destination_root_id) =
        configure_two_movie_roots(&ctx, first_root.path(), second_root.path()).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let folder = make_folder(first_root.path(), "Accepted Move (2024)");
    let title = movable_title(&ctx, "Accepted Move", &folder, "Accepted.2024.mkv").await;

    let preview = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": {
            "titleIds": [title.id],
            "destination": { "rootId": destination_root_id }
        }}),
    )
    .await;
    assert_no_errors(&preview);
    let fingerprint = preview["data"]["locationOperationPreview"]["planFingerprint"]
        .as_str()
        .expect("preview fingerprint")
        .to_string();

    let body = gql(
        &ctx,
        START_MUTATION,
        json!({ "input": {
            "titleIds": [title.id],
            "destination": { "rootId": destination_root_id },
            "planFingerprint": fingerprint
        }}),
    )
    .await;
    assert_no_errors(&body);

    let payload = &body["data"]["startLocationOperation"];
    assert_eq!(payload["planFingerprint"], fingerprint);
    let operation = &payload["operation"];
    let operation_id = operation["id"].as_str().expect("operation id").to_string();
    assert!(!operation_id.is_empty());
    assert_eq!(operation["operationType"], "ROOT_MOVE");
    assert_eq!(operation["mode"], "MOVE_WITH_SCRYER");
    assert_eq!(operation["sourceLibraryId"], library_id);
    assert_eq!(operation["destinationRootId"], destination_root_id);
    assert_eq!(operation["planFingerprint"], fingerprint);
    assert_eq!(operation["verificationDepth"], "FULL");
    assert_eq!(operation["verificationFallbackCount"], 0);
    assert_eq!(operation["counters"]["titlesTotal"], 1);
    assert_eq!(operation["counters"]["filesTotal"], 1);

    // FR-030: the caller gets an identifier and watches the operation; the same
    // row reads back through the query the client polls.
    let read_back = gql(&ctx, OPERATION_QUERY, json!({ "id": operation_id })).await;
    assert_no_errors(&read_back);
    let row = &read_back["data"]["locationOperation"];
    assert_eq!(row["id"], operation_id);
    assert_eq!(row["operationType"], "ROOT_MOVE");
    assert_eq!(row["planFingerprint"], fingerprint);
    assert_eq!(row["sourceRootId"], source_root_id);
    assert_eq!(row["destinationRootId"], destination_root_id);
    assert_eq!(row["cancelRequested"], false);
    assert!(row["titleCheckpoints"].is_array());
}

#[tokio::test]
async fn graphql_location_operation_cancel_and_resume_report_what_they_did() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");
    let root_id = configure_default_library_root(&ctx, MediaFacet::Movie, media_root.path()).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let operation_id = seed_queued_operation(&ctx, &library_id, &root_id).await;

    // FR-092: the cancel is persisted; the runner honors it at its next title
    // checkpoint rather than stopping mid-file.
    let canceled = gql(&ctx, CANCEL_MUTATION, json!({ "id": operation_id })).await;
    assert_no_errors(&canceled);
    assert_eq!(
        canceled["data"]["cancelLocationOperation"]["id"],
        operation_id
    );
    assert_eq!(
        canceled["data"]["cancelLocationOperation"]["cancelRequested"],
        true
    );

    let row = gql(&ctx, OPERATION_QUERY, json!({ "id": operation_id })).await;
    assert_no_errors(&row);
    assert_eq!(row["data"]["locationOperation"]["cancelRequested"], true);

    // FR-033: an operation stored without its plan cannot be resumed, and says
    // so rather than restarting from the beginning.
    let resumed = gql(&ctx, RESUME_MUTATION, json!({ "id": operation_id })).await;
    assert_no_errors(&resumed);
    assert_eq!(resumed["data"]["resumeLocationOperation"]["resumed"], false);
    assert!(
        resumed["data"]["resumeLocationOperation"]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("nothing to resume")),
        "an unresumable operation should explain itself: {resumed}"
    );

    // An unknown operation is simply absent, not an error.
    let missing = gql(&ctx, OPERATION_QUERY, json!({ "id": "no-such-operation" })).await;
    assert_no_errors(&missing);
    assert!(missing["data"]["locationOperation"].is_null());
}

#[tokio::test]
async fn graphql_location_operation_surface_requires_library_management_permission() {
    let ctx = TestContext::new().await;
    let first_root = tempfile::tempdir().expect("first root tempdir");
    let second_root = tempfile::tempdir().expect("second root tempdir");
    let (source_root_id, destination_root_id) =
        configure_two_movie_roots(&ctx, first_root.path(), second_root.path()).await;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let folder = make_folder(first_root.path(), "Guarded Move (2024)");
    let title = movable_title(&ctx, "Guarded Move", &folder, "Guarded.2024.mkv").await;
    let operation_id = seed_queued_operation(&ctx, &library_id, &source_root_id).await;

    let preview_query = format!(
        r#"query {{
            locationOperationPreview(input: {{
              titleIds: ["{}"],
              destination: {{ rootId: "{}" }}
            }}) {{ planFingerprint }}
        }}"#,
        title.id, destination_root_id
    );
    let denied = schema_exec(&ctx, &preview_query, Some(location_outsider())).await;
    assert_graphql_field_denied(&denied, "locationOperationPreview");

    let start_mutation = format!(
        r#"mutation {{
            startLocationOperation(input: {{
              titleIds: ["{}"],
              destination: {{ rootId: "{}" }},
              planFingerprint: "irrelevant"
            }}) {{ planFingerprint }}
        }}"#,
        title.id, destination_root_id
    );
    let denied = schema_exec(&ctx, &start_mutation, Some(location_outsider())).await;
    assert_graphql_field_denied(&denied, "startLocationOperation");

    // FR-083 governs reading an operation too: it names both libraries.
    let operation_query =
        format!(r#"query {{ locationOperation(id: "{operation_id}") {{ id }} }}"#);
    let denied = schema_exec(&ctx, &operation_query, Some(location_outsider())).await;
    assert_graphql_field_denied(&denied, "locationOperation");

    let cancel_mutation =
        format!(r#"mutation {{ cancelLocationOperation(id: "{operation_id}") {{ id }} }}"#);
    let denied = schema_exec(&ctx, &cancel_mutation, Some(location_outsider())).await;
    assert_graphql_field_denied(&denied, "cancelLocationOperation");

    let resume_mutation =
        format!(r#"mutation {{ resumeLocationOperation(id: "{operation_id}") {{ id }} }}"#);
    let denied = schema_exec(&ctx, &resume_mutation, Some(location_outsider())).await;
    assert_graphql_field_denied(&denied, "resumeLocationOperation");

    // The refusals started nothing.
    assert_eq!(
        std::fs::read_dir(second_root.path())
            .expect("read destination root")
            .count(),
        0
    );
}

#[tokio::test]
async fn graphql_location_operation_preview_requires_permission_on_every_source_library() {
    let ctx = TestContext::new().await;
    let movie_root = tempfile::tempdir().expect("movie root tempdir");
    let series_root = tempfile::tempdir().expect("series root tempdir");
    let destination_root = tempfile::tempdir().expect("destination root tempdir");
    let (_movie_root_id, destination_root_id) =
        configure_two_movie_roots(&ctx, movie_root.path(), destination_root.path()).await;
    configure_default_library_root(&ctx, MediaFacet::Series, series_root.path()).await;
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let folder = make_folder(movie_root.path(), "Permitted Movie (2024)");
    let permitted = movable_title(&ctx, "Permitted Movie", &folder, "Permitted.2024.mkv").await;
    let foreign = create_catalog_title(
        &ctx,
        "Foreign Series",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;

    // The destination library is granted; the second source library is not.
    let actor = manage_one_library_actor(&movie_library_id);
    let query = format!(
        r#"query {{
            locationOperationPreview(input: {{
              titleIds: ["{}", "{}"],
              destination: {{ libraryId: "{}", rootId: "{}" }}
            }}) {{ planFingerprint }}
        }}"#,
        permitted.id, foreign.id, movie_library_id, destination_root_id
    );
    let denied = schema_exec(&ctx, &query, Some(actor)).await;
    assert_graphql_field_denied(&denied, "locationOperationPreview");

    // The same actor may preview a selection confined to the library it manages.
    let permitted_query = format!(
        r#"query {{
            locationOperationPreview(input: {{
              titleIds: ["{}"],
              destination: {{ libraryId: "{}", rootId: "{}" }}
            }}) {{ planFingerprint }}
        }}"#,
        permitted.id, movie_library_id, destination_root_id
    );
    let allowed = schema_exec(
        &ctx,
        &permitted_query,
        Some(manage_one_library_actor(&movie_library_id)),
    )
    .await;
    assert!(
        allowed.get("errors").is_none(),
        "the granted library should preview normally: {allowed}"
    );
}

fn location_outsider() -> User {
    User {
        id: Id::new().0,
        username: "location-outsider".to_string(),
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

fn manage_one_library_actor(library_id: &str) -> User {
    User {
        id: Id::new().0,
        username: "location-single-library-manager".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::from([(
                library_id.to_string(),
                LibraryPermissionMask::from_permissions([LibraryPermission::ManageTitles]),
            )]),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

// ── US7 ──────────────────────────────────────────────────────────────────────

/// A second movie library on its own root, so a selection has somewhere to
/// cross into.
async fn create_second_movie_library(
    ctx: &TestContext,
    root: &std::path::Path,
) -> (String, String) {
    let library = ctx
        .libraries
        .create(
            scryer_domain::Library {
                id: Id::new().0,
                name: "Archive Movies".to_string(),
                slug: "archive-movies".to_string(),
                facet: MediaFacet::Movie,
                is_default: false,
                roots: Vec::new(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            vec![LibraryRootDraft {
                path: root.to_string_lossy().to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("create the destination movie library");
    let root_id = library.roots[0].id.clone();
    (library.id, root_id)
}

/// A movie title placed in `library_id` on `root_id`, carrying `external_ids`.
async fn title_in_library(
    ctx: &TestContext,
    name: &str,
    library_id: &str,
    root_id: &str,
    external_ids: Vec<ExternalId>,
) -> Title {
    let title = create_catalog_title(ctx, name, MediaFacet::Movie, external_ids, vec![], true).await;
    ctx.titles
        .transfer_to_library(&title.id, library_id, root_id, None, &[])
        .await
        .expect("place the title in the destination library");
    ctx.titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists")
}

/// FR-071 / US7: a unique canonical identity match in the destination library is
/// previewed as a merge, and the preview carries the whole decision — what the
/// destination keeps, what is unioned per table, what is dropped, and the notes
/// explaining where the live schema differs from the merge inventory.
#[tokio::test]
async fn graphql_location_operation_preview_surfaces_the_merge_summary() {
    let ctx = TestContext::new().await;
    let source_root = tempfile::tempdir().expect("source root tempdir");
    let destination_root = tempfile::tempdir().expect("destination root tempdir");
    let (source_root_id, _second) =
        configure_two_movie_roots(&ctx, source_root.path(), destination_root.path()).await;
    let archive_root = tempfile::tempdir().expect("archive root tempdir");
    let (destination_library_id, destination_root_id) =
        create_second_movie_library(&ctx, archive_root.path()).await;

    // Deliberately *not* the source's name: the merge target is found by
    // canonical identity, and a name that only rides through correctly can be
    // told apart from one copied off the merging title.
    let destination = title_in_library(
        &ctx,
        "Twin Film (Restored)",
        &destination_library_id,
        &destination_root_id,
        vec![ExternalId {
            source: "tmdb".to_string(),
            value: "424242".to_string(),
        }],
    )
    .await;

    let folder = make_folder(source_root.path(), "Twin Film (2024)");
    let source = create_catalog_title(
        &ctx,
        "Twin Film",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tmdb".to_string(),
            value: "424242".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    move_title_to_root(&ctx, &source.id, &source_root_id).await;
    set_title_folder_path(&ctx, &source.id, &folder).await;
    let file = write_media(&folder, "Twin.Film.2024.mkv");
    seed_media_row(&ctx, &source.id, &file).await;

    let body = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": {
            "titleIds": [source.id],
            "destination": {
                "libraryId": destination_library_id,
                "rootId": destination_root_id
            }
        }}),
    )
    .await;
    assert_no_errors(&body);
    let preview = &body["data"]["locationOperationPreview"];

    // FR-055 + US7: a merge is a startable cross-library transfer carrying its
    // target, not a blocked title.
    assert_eq!(preview["blocksStart"], false);
    let transfers = group(preview, "CROSS_LIBRARY_TRANSFER");
    assert_eq!(transfers["count"], 1);
    assert_eq!(transfers["titles"][0]["mergeTargetTitleId"], destination.id);
    // US7: the id is not what the sentence says — the surviving title's name is.
    assert_eq!(
        transfers["titles"][0]["mergeTargetTitleName"],
        "Twin Film (Restored)",
        "the merge target rides the payload by name, not only by id: {transfers}"
    );
    assert_eq!(transfers["titles"][0]["destinationIdentityMatch"], "UNIQUE");
    assert_eq!(
        transfers["titles"][0]["ambiguousDestinationCandidates"],
        json!([]),
        "a unique match is not a choice: {transfers}"
    );

    // FR-071: the summary the user confirms.
    let merges = preview["merges"].as_array().expect("merge summaries");
    assert_eq!(merges.len(), 1, "{preview}");
    let merge = &merges[0];
    assert_eq!(merge["sourceTitleId"], source.id);
    assert_eq!(merge["destinationTitleId"], destination.id);
    // FR-071: the summary names the surviving title, read with its catalog row
    // in Group 0 rather than looked up again by the client.
    assert_eq!(merge["destinationTitleName"], "Twin Film (Restored)");
    assert_eq!(merge["destinationLibraryId"], destination_library_id);
    assert_eq!(merge["blocked"], false);
    assert_eq!(merge["blockedRecords"], json!([]));

    // FR-063: the destination keeps the title id, and the payload says which.
    let title_id_wins = merge["destinationWins"]
        .as_array()
        .expect("destination-wins entries")
        .iter()
        .find(|entry| entry["setting"] == "title id")
        .expect("FR-063 names the title id");
    assert_eq!(title_id_wins["destinationValue"], destination.id);
    assert_eq!(title_id_wins["sourceValue"], source.id);

    // FR-064: per-table dispositions, as enums rather than free text.
    let media_files = merge["dispositions"]
        .as_array()
        .expect("dispositions")
        .iter()
        .find(|entry| entry["table"] == "media_files")
        .expect("media_files is inventoried");
    assert_eq!(media_files["disposition"], "UNION");
    assert_eq!(media_files["sourceRowCount"], 1);

    // The Group 6 work list is an enum the client can act on.
    let work = merge["postMergeWork"].as_array().expect("post-merge work");
    assert!(
        work.iter()
            .any(|value| value == "DROP_SOURCE_INDEXER_COVERAGE"),
        "{merge}"
    );

    // The live-schema deviations reach the operator rather than reading as an
    // omission from the inventory.
    let notes = merge["notes"].as_array().expect("notes");
    assert!(
        notes
            .iter()
            .any(|note| note.as_str().is_some_and(|note| note.contains("wanted_items"))),
        "{merge}"
    );
}

/// FR-055 / FR-016: an ambiguous identity blocks, and the payload names the
/// candidates rather than handing the client ids it cannot render.
#[tokio::test]
async fn graphql_an_ambiguous_destination_identity_names_its_candidates() {
    let ctx = TestContext::new().await;
    let source_root = tempfile::tempdir().expect("source root tempdir");
    let spare_root = tempfile::tempdir().expect("spare root tempdir");
    let (source_root_id, _second) =
        configure_two_movie_roots(&ctx, source_root.path(), spare_root.path()).await;
    let archive_root = tempfile::tempdir().expect("archive root tempdir");
    let (destination_library_id, destination_root_id) =
        create_second_movie_library(&ctx, archive_root.path()).await;

    let first = title_in_library(
        &ctx,
        "Split A",
        &destination_library_id,
        &destination_root_id,
        vec![ExternalId {
            source: "tmdb".to_string(),
            value: "111".to_string(),
        }],
    )
    .await;
    let second = title_in_library(
        &ctx,
        "Split B",
        &destination_library_id,
        &destination_root_id,
        vec![ExternalId {
            source: "imdb".to_string(),
            value: "tt222".to_string(),
        }],
    )
    .await;

    let folder = make_folder(source_root.path(), "Split Source (2024)");
    let source = create_catalog_title(
        &ctx,
        "Split Source",
        MediaFacet::Movie,
        vec![
            ExternalId {
                source: "tmdb".to_string(),
                value: "111".to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt222".to_string(),
            },
        ],
        vec![],
        true,
    )
    .await;
    move_title_to_root(&ctx, &source.id, &source_root_id).await;
    set_title_folder_path(&ctx, &source.id, &folder).await;
    let file = write_media(&folder, "Split.Source.2024.mkv");
    seed_media_row(&ctx, &source.id, &file).await;

    let body = gql(
        &ctx,
        PREVIEW_QUERY,
        json!({ "input": {
            "titleIds": [source.id],
            "destination": {
                "libraryId": destination_library_id,
                "rootId": destination_root_id
            }
        }}),
    )
    .await;
    assert_no_errors(&body);
    let preview = &body["data"]["locationOperationPreview"];

    assert_eq!(preview["blocksStart"], true);
    let blocked = group(preview, "NEEDS_RESOLUTION");
    assert_eq!(blocked["count"], 1);
    let title = &blocked["titles"][0];
    assert_eq!(title["reasonCode"], "ambiguous_destination_identity");
    assert_eq!(title["destinationIdentityMatch"], "AMBIGUOUS");
    assert_eq!(title["mergeTargetTitleId"], Value::Null);

    let candidates = title["ambiguousDestinationCandidates"]
        .as_array()
        .expect("candidates");
    assert_eq!(candidates.len(), 2, "{title}");
    let named: Vec<&str> = candidates
        .iter()
        .filter_map(|candidate| candidate["titleName"].as_str())
        .collect();
    assert!(named.contains(&"Split A"), "{candidates:?}");
    assert!(named.contains(&"Split B"), "{candidates:?}");
    let ids: Vec<&str> = candidates
        .iter()
        .filter_map(|candidate| candidate["titleId"].as_str())
        .collect();
    assert!(ids.contains(&first.id.as_str()));
    assert!(ids.contains(&second.id.as_str()));
    // The identities are why each candidate is on the list.
    let identities: Vec<&str> = candidates
        .iter()
        .flat_map(|candidate| {
            candidate["sharedIdentities"]
                .as_array()
                .expect("shared identities")
                .iter()
                .filter_map(|value| value.as_str())
        })
        .collect();
    assert!(identities.contains(&"tmdb:111"), "{identities:?}");
    assert!(identities.contains(&"imdb:tt222"), "{identities:?}");

    // The ids-only list the web already reads stays in agreement with it.
    let id_only = title["ambiguousDestinationTitleIds"]
        .as_array()
        .expect("ids");
    assert_eq!(id_only.len(), candidates.len());

    // Nothing merges: the preview has no merge summary to confirm.
    assert_eq!(preview["merges"], json!([]));
}
