//! Starting a "purge selected" batch looks up only the selected recycle
//! entries. It must accept and refuse exactly the batches the full listing
//! did, and the purge that follows must remove and keep the same entries.

use super::*;

/// Two movie libraries, each with its default recycle bin holding a valid
/// entry, a second valid entry and a damaged one.
struct PurgeFixture {
    _temp: tempfile::TempDir,
    app: AppUseCase,
    user: User,
    /// Entry directory of each seeded entry, by label.
    entries: std::collections::BTreeMap<&'static str, PathBuf>,
}

const LIBRARY_B: &str = "synthetic-library-b";

impl PurgeFixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let root_a = temp.path().join("library-a");
        let root_b = temp.path().join("library-b");
        for root in [&root_a, &root_b] {
            std::fs::create_dir_all(root).expect("create library root");
        }
        let (app, user, _) =
            bootstrap_movie_scan_app(&root_a, Vec::new(), Arc::new(EmptySearchMetadataGateway))
                .await;
        let now = Utc::now();
        app.services
            .catalog
            .libraries
            .create(
                Library {
                    id: LIBRARY_B.to_string(),
                    facet: MediaFacet::Movie,
                    name: "Synthetic Library B".to_string(),
                    slug: LIBRARY_B.to_string(),
                    is_default: false,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
                vec![LibraryRootDraft {
                    id: None,
                    path: root_b.to_string_lossy().to_string(),
                    is_default: true,
                }],
            )
            .await
            .expect("create second library");
        let app = app.with_test_overrides(|services| {
            services.with_settings(Arc::new(StoredSettingsRepo::default()))
        });
        let emberfall_folder = root_a.join("Emberfall (2001)");
        std::fs::create_dir_all(&emberfall_folder).expect("create title folder");
        let emberfall =
            create_movie_title_with_folder(&app, &user, "Emberfall", &emberfall_folder).await;

        let mut entries = std::collections::BTreeMap::new();
        for (label, root, title_id) in [
            ("a-emberfall", &root_a, Some(emberfall.id.clone())),
            ("a-cinderwake", &root_a, None),
            ("a-damaged", &root_a, None),
            ("b-ashgrove", &root_b, None),
            ("b-ashgrove-extra", &root_b, None),
            ("b-damaged", &root_b, None),
        ] {
            let config = crate::recycle_bin::RecycleBinConfig {
                enabled: true,
                base_path: root.join(".scryer-recycle"),
                retention_days: 7,
                cleanup_enabled: true,
                validation_error: None,
                source_roots: vec![root.clone()],
            };
            let source = root.join(format!("{label}.mkv"));
            std::fs::write(&source, format!("{label} content")).expect("write source");
            let manifest = crate::recycle_bin::RecycleManifest {
                schema: None,
                entry_id: None,
                source_operation_id: None,
                recycled_at: Utc::now().to_rfc3339(),
                original_path: source.to_string_lossy().to_string(),
                original_file_id: None,
                size_bytes: 0,
                title_id,
                media_root: None,
                reason: "file_deleted".to_string(),
                status: None,
                replacement_file_id: None,
                replacement_path: None,
                media_row: None,
            };
            let recycled = crate::recycle_bin::recycle_file(&config, &source, manifest)
                .await
                .expect("seed recycle entry")
                .expect("file recycled");
            let entry_dir = recycled
                .recycled_path
                .parent()
                .expect("entry directory")
                .to_path_buf();
            if label.ends_with("damaged") {
                // A hand-edited manifest whose id no longer names its directory.
                let path = entry_dir.join("manifest.json");
                let mut manifest: crate::recycle_bin::RecycleManifest =
                    serde_json::from_slice(&std::fs::read(&path).expect("read manifest"))
                        .expect("decode manifest");
                manifest.entry_id = Some("hand-edited-id".to_string());
                std::fs::write(
                    &path,
                    serde_json::to_vec(&manifest).expect("encode manifest"),
                )
                .expect("write manifest");
            }
            entries.insert(label, entry_dir);
        }
        Self {
            _temp: temp,
            app,
            user,
            entries,
        }
    }

    fn id(&self, label: &str) -> String {
        self.entries[label]
            .file_name()
            .expect("entry name")
            .to_string_lossy()
            .to_string()
    }

    /// Each entry's state: gone, or present with its manifest status.
    fn states(&self) -> Vec<(&'static str, Option<String>)> {
        self.entries
            .iter()
            .map(|(label, dir)| {
                let status = std::fs::read(dir.join("manifest.json")).ok().map(|bytes| {
                    serde_json::from_slice::<crate::recycle_bin::RecycleManifest>(&bytes)
                        .expect("decode manifest")
                        .status
                        .unwrap_or_default()
                });
                (*label, status)
            })
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The batch was refused at start, with this reason code.
    Refused(Option<&'static str>),
    /// The batch was accepted for these libraries, and each selected entry's
    /// purge ended as listed.
    Accepted(Vec<String>, Vec<(&'static str, String)>),
}

#[derive(Clone, Copy)]
enum SelectionPath {
    Listing,
    Narrow,
}

/// Starts the batch the way the job does, then purges each selected entry
/// the way the job body does.
async fn purge(fixture: &PurgeFixture, path: SelectionPath, labels: &[&'static str]) -> Outcome {
    let mut ids = labels
        .iter()
        .map(|label| {
            if fixture.entries.contains_key(label) {
                fixture.id(label)
            } else {
                (*label).to_string()
            }
        })
        .collect::<Vec<_>>();
    ids.sort();
    let selection = match path {
        SelectionPath::Listing => {
            fixture
                .app
                .recycle_purge_selection_by_listing(&fixture.user, &ids)
                .await
        }
        SelectionPath::Narrow => {
            fixture
                .app
                .recycle_purge_selection(&fixture.user, &ids)
                .await
        }
    };
    let library_ids = match selection {
        Ok(library_ids) => library_ids,
        Err(error) => return Outcome::Refused(error.reason()),
    };
    let mut results = Vec::new();
    for label in labels {
        let id = fixture.id(label);
        let result = match fixture.app.delete_recycled_item(&fixture.user, &id).await {
            Ok(true) => "completed".to_string(),
            Ok(false) => "quarantined".to_string(),
            Err(error) => format!("failed:{:?}", error.reason()),
        };
        results.push((*label, result));
    }
    // Library ids differ between fixtures only for the generated default one.
    let library_ids = library_ids
        .into_iter()
        .map(|id| {
            if id == LIBRARY_B {
                id
            } else {
                "library-a".to_string()
            }
        })
        .collect();
    Outcome::Accepted(library_ids, results)
}

#[tokio::test]
async fn narrow_purge_selection_purges_and_keeps_what_the_listing_did() {
    let selections: &[&[&'static str]] = &[
        // Valid entries in both bins.
        &["a-emberfall", "b-ashgrove"],
        // One bin only, beside an unselected damaged entry in the other.
        &["b-ashgrove", "b-ashgrove-extra"],
        // A selected damaged entry refuses the batch.
        &["a-emberfall", "a-damaged"],
        // An id that names no entry.
        &["b-ashgrove", "20260101-0000-ffffff"],
    ];
    for labels in selections {
        let listing = PurgeFixture::new().await;
        let narrow = PurgeFixture::new().await;

        let listing_outcome = purge(&listing, SelectionPath::Listing, labels).await;
        let narrow_outcome = purge(&narrow, SelectionPath::Narrow, labels).await;
        assert_eq!(narrow_outcome, listing_outcome, "selection {labels:?}");

        // The listing quarantined every damaged entry it met; the narrow
        // lookup only the selected ones. Everything else is identical.
        let listing_states = listing.states();
        let narrow_states = narrow.states();
        for ((label, listed), (_, narrowed)) in listing_states.iter().zip(&narrow_states) {
            let unselected_damaged = label.ends_with("damaged") && !labels.contains(label);
            if unselected_damaged {
                assert_eq!(listed.as_deref(), Some("quarantined"), "{label}");
                assert_eq!(narrowed.as_deref(), Some("committed"), "{label}");
            } else {
                assert_eq!(narrowed, listed, "{label} in selection {labels:?}");
            }
        }

        // The next page view quarantines what the narrow lookup passed over.
        narrow
            .app
            .list_recycled_items(&narrow.user, None)
            .await
            .expect("page view");
        listing
            .app
            .list_recycled_items(&listing.user, None)
            .await
            .expect("page view");
        assert_eq!(narrow.states(), listing.states(), "selection {labels:?}");
    }
}

#[tokio::test]
async fn narrow_purge_selection_accepts_the_same_libraries_and_refusals() {
    let fixture = PurgeFixture::new().await;
    let mut ids = vec![fixture.id("a-emberfall"), fixture.id("b-ashgrove")];
    ids.sort();
    let narrow = fixture
        .app
        .recycle_purge_selection(&fixture.user, &ids)
        .await
        .expect("narrow selection");
    let listing = fixture
        .app
        .recycle_purge_selection_by_listing(&fixture.user, &ids)
        .await
        .expect("listing selection");
    assert_eq!(narrow, listing);
    assert_eq!(narrow.len(), 2, "one library per bin");
    assert!(narrow.contains(&LIBRARY_B.to_string()));

    let damaged = vec![fixture.id("a-damaged")];
    let refusal = fixture
        .app
        .recycle_purge_selection(&fixture.user, &damaged)
        .await
        .expect_err("a damaged entry is not purgeable");
    assert_eq!(
        refusal.reason(),
        Some(crate::error_reasons::jobs::RECYCLE_ENTRY_NOT_FOUND)
    );
    assert_eq!(
        fixture
            .states()
            .into_iter()
            .find(|(label, _)| *label == "a-damaged"),
        Some(("a-damaged", Some("quarantined".to_string()))),
        "a selected damaged entry is still quarantined by the lookup"
    );
}
