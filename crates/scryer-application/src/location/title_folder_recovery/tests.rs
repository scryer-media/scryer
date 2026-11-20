use super::*;
use chrono::Utc;

fn root(path: &str) -> LibraryRoot {
    LibraryRoot {
        id: "root".into(),
        library_id: "anime".into(),
        path: path.into(),
        is_default: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn recovery_enrolls_only_root_and_ancestor_ownership() {
    let roots = vec![root(r"T:\Videos\Anime")];
    assert!(eligible(Some(r"T:\"), &roots));
    assert!(eligible(Some(r"T:\Videos"), &roots));
    assert!(eligible(Some(r"T:\Videos\Anime"), &roots));
    for folder in [
        None,
        Some(""),
        Some(r"T:\Videos\Anime\Example"),
        Some(r"U:\Other\Example"),
    ] {
        assert!(!eligible(folder, &roots));
    }
}

#[test]
fn recovery_does_not_infer_a_grouping_folder_as_a_title() {
    assert!(unambiguous_layout(
        "/library/Series",
        "/library/Series/E01.mkv"
    ));
    assert!(unambiguous_layout(
        "/library/Series",
        "/library/Series/Season 01/E01.mkv"
    ));
    assert!(!unambiguous_layout(
        "/library/Group",
        "/library/Group/Series/E01.mkv"
    ));
    assert!(!unambiguous_layout(
        "/library/Group",
        "/library/Group/Series/Season 01/E01.mkv"
    ));
}

async fn file_fixture() -> (tempfile::TempDir, RecoveryFile) {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let source = base.join("source.mkv");
    let destination = base.join("destination.mkv");
    tokio::fs::write(&source, b"synthetic media").await.unwrap();
    tokio::fs::copy(&source, &destination).await.unwrap();
    let file = RecoveryFile {
        media: RecoveryMedia {
            id: "media".into(),
            title_id: "title".into(),
            file_path: path_to_stored_string(&source),
            size_bytes: 15,
        },
        destination: path_to_stored_string(destination),
        source_version: Some(recovery_file_version(&source).await.unwrap()),
        expected_hash: None,
        matched_by_name_and_size: false,
    };
    (temp, file)
}

#[tokio::test]
async fn recovery_verifies_content_and_preserves_both_files_on_mismatch() {
    for depth in [VerificationDepth::Quick, VerificationDepth::Full] {
        let (_temp, file) = file_fixture().await;
        execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
            .await
            .unwrap();
        tokio::fs::write(&file.destination, b"different media")
            .await
            .unwrap();
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(&file.media.file_path).await.unwrap(),
            b"synthetic media"
        );
        assert_eq!(
            tokio::fs::read(&file.destination).await.unwrap(),
            b"different media"
        );
    }
}

#[tokio::test]
async fn recovery_missing_source_requires_full_content_proof() {
    for depth in [VerificationDepth::Quick, VerificationDepth::Full] {
        let (_temp, mut file) = file_fixture().await;
        tokio::fs::remove_file(&file.media.file_path).await.unwrap();
        file.source_version = None;
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
        file.expected_hash = Some(blake3::hash(b"synthetic media").to_hex().to_string());
        execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
            .await
            .unwrap();
        file.expected_hash = Some(blake3::hash(b"different media").to_hex().to_string());
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(&file.destination).await.unwrap(),
            b"synthetic media"
        );
    }
}

/// With the source present, quick depth compares the two files without
/// reading either whole and records no hashes; full depth hashes both. A
/// missing source is proven by its stored full hash at either depth.
#[tokio::test]
async fn recovery_proof_follows_the_depth_while_the_source_is_present() {
    crate::disk::Disk::process().count_whole_reads();
    let reads = |path: &str| crate::disk::Disk::process().whole_reads_of(Path::new(path));
    let (_temp, file) = file_fixture().await;
    let proof = execute::prove_file(
        &file,
        crate::location::verify::CopyProgress::none(),
        VerificationDepth::Quick,
    )
    .await
    .unwrap();
    assert!(proof.is_none());
    assert_eq!(
        (reads(&file.media.file_path), reads(&file.destination)),
        (0, 0)
    );
    let proof = execute::prove_file(
        &file,
        crate::location::verify::CopyProgress::none(),
        VerificationDepth::Full,
    )
    .await
    .unwrap();
    assert_eq!(
        proof.map(|hashes| hashes.full_blake3),
        Some(blake3::hash(b"synthetic media").to_hex().to_string())
    );

    let (_temp, mut file) = file_fixture().await;
    tokio::fs::remove_file(&file.media.file_path).await.unwrap();
    file.source_version = None;
    file.expected_hash = Some(blake3::hash(b"synthetic media").to_hex().to_string());
    let proof = execute::prove_file(
        &file,
        crate::location::verify::CopyProgress::none(),
        VerificationDepth::Quick,
    )
    .await
    .unwrap();
    assert_eq!(
        proof.map(|hashes| hashes.full_blake3),
        file.expected_hash.clone()
    );
}

/// The catalog's length is held to the destination at quick depth as it is
/// at full depth.
#[tokio::test]
async fn recovery_quick_proof_holds_the_catalog_length() {
    let (_temp, mut file) = file_fixture().await;
    file.media.size_bytes = 14;
    let error = execute::prove_file(
        &file,
        crate::location::verify::CopyProgress::none(),
        VerificationDepth::Quick,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.reason(),
        Some(crate::error_reasons::location::RECOVERY_DESTINATION_CONTENT_MISMATCH)
    );
}

#[tokio::test]
async fn recovery_detects_source_replacement_even_with_identical_content() {
    for depth in [VerificationDepth::Quick, VerificationDepth::Full] {
        let (_temp, file) = file_fixture().await;
        let source = stored_path_to_path_buf(&file.media.file_path);
        let replacement = source.with_extension("replacement");
        tokio::fs::write(&replacement, b"synthetic media")
            .await
            .unwrap();
        tokio::fs::rename(&source, source.with_extension("retained"))
            .await
            .unwrap();
        tokio::fs::rename(&replacement, &source).await.unwrap();
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
        assert!(source.with_extension("retained").exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn recovery_rejects_symlink_ancestors_and_final_files() {
    let (_temp, file) = file_fixture().await;
    let source = stored_path_to_path_buf(&file.media.file_path);
    let link = source.with_extension("link");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    assert!(inspect_path(&link, false).await.is_err());
    let dir_link = source.with_extension("directory-link");
    std::os::unix::fs::symlink(source.parent().unwrap(), &dir_link).unwrap();
    assert!(
        inspect_path(&dir_link.join("missing.mkv"), true)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(&source).await.unwrap(), b"synthetic media");
}

fn title() -> Title {
    Title {
        id: "title".into(),
        library_id: "anime".into(),
        name: "Example Series".into(),
        facet: scryer_domain::MediaFacet::Anime,
        monitored: true,
        tags: vec![],
        external_ids: vec![scryer_domain::ExternalId::new("tvdb", "123456")],
        root_folder_id: "root".into(),
        created_by: None,
        created_at: Utc::now(),
        year: Some(2014),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: "example series".into(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        canonical_tags: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        movie_release_dates: None,
        folder_path: Some("/drive".into()),
    }
}

fn catalog_fixture() -> (RecoveryPlan, RecoveryCatalog) {
    let title = title();
    let root = root("/drive/library");
    let media = RecoveryMedia {
        id: "media".into(),
        title_id: title.id.clone(),
        file_path: "/drive/E01.mkv".into(),
        size_bytes: 15,
    };
    let catalog = RecoveryCatalog {
        roots: vec![root.clone()],
        owners: vec![RecoveryOwner {
            id: title.id.clone(),
            library_id: title.library_id.clone(),
            folder_path: title.folder_path.clone(),
            revision: None,
        }],
        media: vec![media.clone()],
    };
    let plan = RecoveryPlan {
        version: RECOVERY_VERSION.into(),
        title,
        owner_revision: None,
        roots: catalog.roots.clone(),
        expected_media: catalog.media.clone(),
        destination_root: Some(root),
        destination: Some("/drive/library/Example".into()),
        generated_destination: false,
        files: vec![RecoveryFile {
            media,
            destination: "/drive/library/Example/E01.mkv".into(),
            source_version: Some("identity".into()),
            expected_hash: None,
            matched_by_name_and_size: false,
        }],
        unresolved: None,
    };
    (plan, catalog)
}

#[test]
fn recovery_ignores_corrupt_ancestor_claims_but_rejects_real_conflicts() {
    let (plan, mut catalog) = catalog_fixture();
    catalog.owners.push(RecoveryOwner {
        id: "other".into(),
        library_id: "anime".into(),
        folder_path: Some("/drive".into()),
        revision: None,
    });
    validate_catalog(&plan, &catalog).unwrap();
    catalog.owners[1].folder_path = Some("/drive/library/Example/Child".into());
    assert!(validate_catalog(&plan, &catalog).is_err());
    catalog.owners[1].folder_path = Some("/drive/library/Neighbor".into());
    validate_catalog(&plan, &catalog).unwrap();
    catalog.media.push(RecoveryMedia {
        id: "shared".into(),
        title_id: "other".into(),
        file_path: plan.files[0].media.file_path.clone(),
        size_bytes: 15,
    });
    assert!(validate_catalog(&plan, &catalog).is_err());
}

#[test]
fn recovery_rejects_changed_membership_roots_and_revisions() {
    let (plan, catalog) = catalog_fixture();
    validate_catalog(&plan, &catalog).unwrap();
    let mut changed = catalog.clone();
    changed.roots[0].path = "/other-drive".into();
    assert!(validate_catalog(&plan, &changed).is_err());
    let mut changed = catalog.clone();
    changed.media[0].file_path = "/drive/changed.mkv".into();
    assert!(validate_catalog(&plan, &changed).is_err());
    let mut changed = catalog.clone();
    changed.media.clear();
    assert!(validate_catalog(&plan, &changed).is_err());
    let mut changed = catalog;
    changed.owners[0].revision = Some(Utc::now());
    assert!(validate_catalog(&plan, &changed).is_err());
}

#[test]
fn recovery_rejects_destination_escape_and_colliding_media_records() {
    let (mut plan, catalog) = catalog_fixture();
    plan.files[0].destination = "/drive/unrelated.mkv".into();
    assert!(validate_catalog(&plan, &catalog).is_err());
    let (mut plan, mut catalog) = catalog_fixture();
    let mut second = plan.files[0].clone();
    second.media.id = "second".into();
    second.media.file_path = "/drive/second.mkv".into();
    catalog.media.push(second.media.clone());
    plan.expected_media = catalog.media.clone();
    plan.expected_media.sort_by(|a, b| a.id.cmp(&b.id));
    plan.files.push(second);
    assert!(validate_catalog(&plan, &catalog).is_err());
}

#[tokio::test]
async fn recovery_recognizes_both_external_id_spellings_and_rejects_conflicting_nfo() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    for name in [
        "Example Series (2014) - [tvdbid-123456]",
        "Example Series (2014) {tvdb-123456}",
    ] {
        let folder = base.join(name);
        tokio::fs::create_dir(&folder).await.unwrap();
        assert!(folder_matches(&title(), &folder).await.unwrap(), "{name}");
        tokio::fs::write(
            folder.join("tvshow.nfo"),
            "<tvshow><uniqueid type=\"tvdb\" default=\"true\">999999</uniqueid></tvshow>",
        )
        .await
        .unwrap();
        assert!(folder_matches(&title(), &folder).await.is_err());
    }
}

#[tokio::test]
async fn recovery_rejects_hardlinked_destination() {
    for depth in [VerificationDepth::Quick, VerificationDepth::Full] {
        let (_temp, mut file) = file_fixture().await;
        tokio::fs::remove_file(&file.destination).await.unwrap();
        tokio::fs::hard_link(&file.media.file_path, &file.destination)
            .await
            .unwrap();
        file.source_version = Some(
            recovery_file_version(&stored_path_to_path_buf(&file.media.file_path))
                .await
                .unwrap(),
        );
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(&file.media.file_path).await.unwrap(),
            b"synthetic media"
        );
        assert_eq!(
            tokio::fs::read(&file.destination).await.unwrap(),
            b"synthetic media"
        );
    }
}

#[tokio::test]
async fn recovery_hash_search_requires_one_match_and_excludes_recycle_and_staging() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().canonicalize().unwrap();
    let season = folder.join("Season 01");
    let recycle = folder.join("Retained");
    tokio::fs::create_dir(&season).await.unwrap();
    tokio::fs::create_dir(&recycle).await.unwrap();
    let candidate = season.join("Episode.mkv");
    tokio::fs::write(&candidate, b"media").await.unwrap();
    tokio::fs::write(recycle.join("Old.mkv"), b"media")
        .await
        .unwrap();
    for staged in [
        ".scryer-recovery-abandoned",
        ".scry-rc-abandoned",
        ".scryer-upgrade-replacement-abandoned",
        ".scry-ur-abandoned",
    ] {
        tokio::fs::write(folder.join(staged), b"media")
            .await
            .unwrap();
    }
    let hash = blake3::hash(b"media").to_hex().to_string();
    let recycle = path_to_stored_string(&recycle);
    assert_eq!(
        locate_hashed_destination(
            &folder,
            5,
            MissingSourceMatch::FullHash(&hash),
            Some(&recycle)
        )
        .await
        .unwrap(),
        candidate
    );
    assert!(
        locate_hashed_destination(&folder, 5, MissingSourceMatch::FullHash(&hash), None)
            .await
            .is_err()
    );
    assert!(
        locate_hashed_destination(
            &folder,
            5,
            MissingSourceMatch::FullHash(blake3::hash(b"other").to_hex().as_ref()),
            Some(&recycle)
        )
        .await
        .is_err()
    );
    assert_eq!(tokio::fs::read(&candidate).await.unwrap(), b"media");
}

#[tokio::test]
async fn recovery_hash_search_still_counts_a_name_that_only_looks_like_staging() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().canonicalize().unwrap();
    tokio::fs::write(folder.join("Episode.mkv"), b"media")
        .await
        .unwrap();
    tokio::fs::write(folder.join(".scry-r-lookalike"), b"media")
        .await
        .unwrap();
    let hash = blake3::hash(b"media").to_hex().to_string();
    assert!(
        locate_hashed_destination(&folder, 5, MissingSourceMatch::FullHash(&hash), None)
            .await
            .is_err(),
        "a lookalike name is a second match, not staging"
    );
}

#[test]
fn recovery_writes_the_short_staging_name_and_recognises_both() {
    assert_eq!(RECOVERY_STAGING_PREFIX, ".scry-rc-");
    assert!(is_recovery_staging_name(".scry-rc-1"));
    assert!(is_recovery_staging_name(".scryer-recovery-1"));
    for unrelated in [".scry-r-1", ".scry-ur-1", "scry-rc-1", ".scryer-ax-1"] {
        assert!(!is_recovery_staging_name(unrelated), "{unrelated}");
    }
    for staged in [
        "/library/Show/.scry-rc-1",
        "/library/Show/.scryer-recovery-1/Episode.mkv",
        "/library/Show/.scry-ur-1-Episode.mkv",
        "/library/Show/.scryer-upgrade-replacement-1-Episode.mkv",
    ] {
        assert!(path_is_in_staging(staged), "{staged}");
    }
    for live in [
        "/library/Show/Episode.mkv",
        "/library/Show/.scry-uo-1-Episode.mkv",
        "/library/Show/x.scry-rc-1",
        ".scry-rc-1/Episode.mkv",
    ] {
        assert!(!path_is_in_staging(live), "{live}");
    }
}

#[tokio::test]
async fn recovery_journaled_copy_preserves_staging_collisions_and_sources() {
    use crate::location::verify::{CopyProgress, recorded_copy};
    let policy = crate::disk::placement::journaled_staging_copy;
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().canonicalize().unwrap();
    let source = folder.join("source.mkv");
    let staging = folder.join(".scryer-recovery-attempt");
    tokio::fs::write(&source, b"original").await.unwrap();
    tokio::fs::write(&staging, b"unrelated").await.unwrap();
    let copier = Disk::process();
    assert!(
        recorded_copy(&copier, &source, &staging, policy(), &CopyProgress::none())
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(&staging).await.unwrap(), b"unrelated");
    let fresh = folder.join(".scryer-recovery-next-attempt");
    let (hash, _) = recorded_copy(&copier, &source, &fresh, policy(), &CopyProgress::none())
        .await
        .unwrap();
    assert_eq!(
        hash.full_blake3,
        blake3::hash(b"original").to_hex().to_string()
    );
    tokio::fs::write(&fresh, b"changed copy").await.unwrap();
    assert_eq!(tokio::fs::read(&source).await.unwrap(), b"original");
    assert_eq!(tokio::fs::read(&staging).await.unwrap(), b"unrelated");
}

#[tokio::test]
async fn recovery_tracked_folder_cannot_override_conflicting_external_id() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("Example Series (2014) - [tvdbid-999999]");
    tokio::fs::create_dir(&folder).await.unwrap();
    assert!(!folder_matches(&title(), &folder).await.unwrap());
    assert!(
        folder_identity_matches(&title(), &folder, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recovery_placement_never_replaces_an_existing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().canonicalize().unwrap();
    let staging = folder.join(".scryer-recovery-verified");
    let destination = folder.join("episode.mkv");
    tokio::fs::write(&staging, b"verified copy").await.unwrap();
    tokio::fs::write(&destination, b"existing file")
        .await
        .unwrap();
    assert!(
        crate::fs_safety::promote_recovery_staged_file(&staging, &destination)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(&staging).await.unwrap(), b"verified copy");
    assert_eq!(
        tokio::fs::read(&destination).await.unwrap(),
        b"existing file"
    );
}

/// In a library that keeps the full-hash job off, a missing source with no
/// recorded hash is found by its file name and catalog length, without
/// reading any file whole; a second file of that name and length is
/// ambiguous, and none is reported as such.
#[tokio::test]
async fn a_missing_source_is_located_by_name_and_size_without_reading() {
    crate::disk::Disk::process().count_whole_reads();
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().canonicalize().unwrap();
    tokio::fs::create_dir_all(folder.join("Season 01"))
        .await
        .unwrap();
    let found = folder.join("Season 01").join("Synthetic Show S01E01.mkv");
    tokio::fs::write(&found, b"synthetic media").await.unwrap();
    // Same length under another name, and the same name at another length.
    tokio::fs::write(folder.join("Other Name.mkv"), b"synthetic media")
        .await
        .unwrap();
    tokio::fs::write(folder.join("Synthetic Show S01E01.mkv.part"), b"x")
        .await
        .unwrap();
    let name = std::ffi::OsStr::new("Synthetic Show S01E01.mkv");

    let located = locate_hashed_destination(&folder, 15, MissingSourceMatch::Name(name), None)
        .await
        .unwrap();
    assert_eq!(located, found);
    assert_eq!(crate::disk::Disk::process().whole_reads_of(&found), 0);

    assert_eq!(
        locate_hashed_destination(&folder, 16, MissingSourceMatch::Name(name), None)
            .await
            .unwrap_err()
            .validation_reason(),
        Some(crate::error_reasons::location::RECOVERY_DESTINATION_NAME_SIZE_NOT_FOUND)
    );

    tokio::fs::write(folder.join("Synthetic Show S01E01.mkv"), b"synthetic media")
        .await
        .unwrap();
    assert_eq!(
        locate_hashed_destination(&folder, 15, MissingSourceMatch::Name(name), None)
            .await
            .unwrap_err()
            .validation_reason(),
        Some(crate::error_reasons::location::RECOVERY_DESTINATION_NAME_SIZE_AMBIGUOUS)
    );
}

/// A file the plan matched by name and size is proven again the same way at
/// either depth, reads neither file whole, and is refused once the
/// destination's name or length no longer fits.
#[tokio::test]
async fn a_name_and_size_match_is_proven_by_name_and_length_at_either_depth() {
    crate::disk::Disk::process().count_whole_reads();
    for depth in [VerificationDepth::Quick, VerificationDepth::Full] {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        tokio::fs::create_dir_all(base.join("recovered"))
            .await
            .unwrap();
        let source = base.join("Synthetic Movie (2001).mkv");
        let destination = base.join("recovered").join("Synthetic Movie (2001).mkv");
        tokio::fs::write(&destination, b"synthetic media")
            .await
            .unwrap();
        let mut file = RecoveryFile {
            media: RecoveryMedia {
                id: "media".into(),
                title_id: "title".into(),
                file_path: path_to_stored_string(&source),
                size_bytes: 15,
            },
            destination: path_to_stored_string(&destination),
            source_version: None,
            expected_hash: None,
            matched_by_name_and_size: true,
        };
        let proof =
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .unwrap();
        assert!(proof.is_none());
        assert_eq!(crate::disk::Disk::process().whole_reads_of(&destination), 0);

        file.media.size_bytes = 16;
        assert_eq!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .unwrap_err()
                .validation_reason(),
            Some(crate::error_reasons::location::RECOVERY_DESTINATION_CONTENT_MISMATCH)
        );
        file.media.size_bytes = 15;
        let renamed = base.join("recovered").join("Renamed.mkv");
        tokio::fs::rename(&destination, &renamed).await.unwrap();
        file.destination = path_to_stored_string(&renamed);
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(&renamed).await.unwrap(), b"synthetic media");

        // Without the marker a missing source still needs its recorded hash.
        file.destination = path_to_stored_string(&renamed);
        file.matched_by_name_and_size = false;
        assert!(
            execute::prove_file(&file, crate::location::verify::CopyProgress::none(), depth)
                .await
                .is_err()
        );
    }
}
