use super::*;
use std::os::unix::fs::PermissionsExt;

struct ConfiguredPermissions;

#[async_trait]
impl PlacedContentPermissions for ConfiguredPermissions {
    async fn apply_to_file(&self, path: &Path) -> AppResult<()> {
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
            .await
            .map_err(|error| relay_failure(error.to_string()))
    }

    async fn apply_to_directory(&self, path: &Path) -> AppResult<()> {
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750))
            .await
            .map_err(|error| relay_failure(error.to_string()))
    }
}

async fn permissions_fixture() -> (tempfile::TempDir, RecoveryFile) {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().canonicalize().unwrap();
    let source = folder.join("original.mkv");
    let destination = folder.join("copy.mkv");
    tokio::fs::write(&source, b"synthetic media").await.unwrap();
    tokio::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600))
        .await
        .unwrap();
    tokio::fs::copy(&source, &destination).await.unwrap();
    let file = RecoveryFile {
        media: RecoveryMedia {
            id: "media".into(),
            title_id: "title".into(),
            file_path: path_to_stored_string(&source),
            size_bytes: 15,
        },
        destination: path_to_stored_string(&destination),
        source_version: Some(recovery_file_version(&source).await.unwrap()),
        expected_hash: None,
    };
    (temp, file)
}

async fn mode(path: &Path) -> u32 {
    tokio::fs::metadata(path)
        .await
        .unwrap()
        .permissions()
        .mode()
        & 0o777
}

#[tokio::test]
async fn recovery_permissions_apply_to_independent_copies_and_preserve_original_modes() {
    let (_temp, file) = permissions_fixture().await;
    prove_and_apply_recovery_permissions(&file, CopyProgress::none(), &ConfiguredPermissions)
        .await
        .unwrap();
    assert_eq!(mode(Path::new(&file.destination)).await, 0o644);
    assert_eq!(mode(Path::new(&file.media.file_path)).await, 0o600);
    assert_eq!(
        tokio::fs::read(&file.media.file_path).await.unwrap(),
        b"synthetic media"
    );
    assert_eq!(
        tokio::fs::read(&file.destination).await.unwrap(),
        b"synthetic media"
    );
}

#[tokio::test]
async fn recovery_permissions_leave_in_place_originals_unchanged() {
    let (_temp, mut file) = permissions_fixture().await;
    file.destination = file.media.file_path.clone();
    prove_and_apply_recovery_permissions(&file, CopyProgress::none(), &ConfiguredPermissions)
        .await
        .unwrap();
    assert_eq!(mode(Path::new(&file.media.file_path)).await, 0o600);
    assert_eq!(
        tokio::fs::read(&file.media.file_path).await.unwrap(),
        b"synthetic media"
    );
}

#[tokio::test]
async fn recovery_permissions_reject_hardlinks_before_changing_original_modes() {
    let (_temp, mut file) = permissions_fixture().await;
    let link = Path::new(&file.media.file_path).with_extension("hardlink.mkv");
    tokio::fs::hard_link(&file.media.file_path, &link)
        .await
        .unwrap();
    file.destination = path_to_stored_string(&link);
    file.source_version = Some(
        recovery_file_version(Path::new(&file.media.file_path))
            .await
            .unwrap(),
    );
    assert!(
        prove_and_apply_recovery_permissions(&file, CopyProgress::none(), &ConfiguredPermissions)
            .await
            .is_err()
    );
    assert_eq!(mode(Path::new(&file.media.file_path)).await, 0o600);
    assert_eq!(mode(&link).await, 0o600);
    assert_eq!(tokio::fs::read(&link).await.unwrap(), b"synthetic media");
}

#[tokio::test]
async fn recovery_permissions_apply_to_new_directories_and_preserve_existing_modes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    tokio::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o711))
        .await
        .unwrap();
    let parent = root.join("Title/Season 01");
    create_recovery_destination_directories(&parent, &ConfiguredPermissions)
        .await
        .unwrap();
    assert_eq!(mode(&root).await, 0o711);
    assert_eq!(mode(&root.join("Title")).await, 0o750);
    assert_eq!(mode(&parent).await, 0o750);
    tokio::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
        .await
        .unwrap();
    create_recovery_destination_directories(&parent, &ConfiguredPermissions)
        .await
        .unwrap();
    assert_eq!(mode(&parent).await, 0o700);
}
