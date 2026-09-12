//! srrdb filename recovery inside the tracked-download import stage.
//!
//! These runs exercise the gate, the candidate rule, the per-import outage
//! breaker and the reuse of recovered names by title matching, all through the
//! real `import_with_lookup` → `run_import` path with real files on disk. The
//! recorded fake port is the whole assertion surface for "who got asked what";
//! the end-to-end rename/artifact behaviour lives in
//! `crates/scryer/tests/integration_import.rs`, which has a filesystem
//! importer and configured library roots.

use super::*;
use crate::ports::{SrrdbFilenameLookup, SrrdbOutage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A recorded srrdb port: every call is remembered, and the answer comes from
/// a canned CRC-to-name table (or is an outage for every call).
#[derive(Default)]
struct FakeSrrdbLookup {
    calls: Mutex<Vec<(String, u64)>>,
    names: HashMap<String, String>,
    outage: bool,
}

impl FakeSrrdbLookup {
    fn recovering(names: &[(&str, &str)]) -> Self {
        Self {
            names: names
                .iter()
                .map(|(name, recovered)| ((*name).to_string(), (*recovered).to_string()))
                .collect(),
            ..Self::default()
        }
    }

    fn outage() -> Self {
        Self {
            outage: true,
            ..Self::default()
        }
    }

    async fn call_count(&self) -> usize {
        self.calls.lock().await.len()
    }
}

#[async_trait]
impl SrrdbFilenameLookup for FakeSrrdbLookup {
    async fn recover_filename(
        &self,
        crc32_hex: &str,
        size_bytes: u64,
    ) -> Result<Option<String>, SrrdbOutage> {
        self.calls
            .lock()
            .await
            .push((crc32_hex.to_string(), size_bytes));
        if self.outage {
            return Err(SrrdbOutage);
        }
        Ok(self.names.get(crc32_hex).cloned())
    }
}

/// A settings repository that answers exactly one system-scope key.
struct SrrdbSettingsRepo {
    enabled: bool,
}

#[async_trait]
impl SettingsRepository for SrrdbSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        if scope == "system"
            && key_name == crate::SRRDB_FILENAME_RECOVERY_ENABLED_KEY
            && scope_id.is_none()
        {
            return Ok(Some(self.enabled.to_string()));
        }
        Ok(None)
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        self.get_setting_json(scope, key_name, scope_id).await
    }

    async fn upsert_setting_json(
        &self,
        _scope: &str,
        _key_name: &str,
        _scope_id: Option<String>,
        _value_json: String,
        _source: &str,
        _updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_setting_value(
        &self,
        _scope: &str,
        _key_name: &str,
        _scope_id: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
        Ok(0)
    }
}

// ── fixtures ───────────────────────────────────────────────────────────────

const OBFUSCATED_MEMBERS: [&str; 3] = [
    "a1b2c3d4e5f6a7b8c9d0.mkv",
    "b2c3d4e5f6a7b8c9d0e1.mkv",
    "c3d4e5f6a7b8c9d0e1f2.mkv",
];

fn recovery_titles() -> Vec<Title> {
    let mut paper_lantern = build_title("title-movie", "Paper Lantern", MediaFacet::Movie);
    paper_lantern.year = Some(2012);
    vec![
        build_title("title-series", "Harbor Pals", MediaFacet::Series),
        paper_lantern,
    ]
}

fn recovery_actor() -> User {
    let mut actor = User::new_admin("admin");
    actor.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    actor
}

/// The completed directory: a fixed, obfuscated release folder inside a temp
/// directory. The folder name never affects candidacy — only a file's own
/// stem and the caller's "needs its own name" judgement do — but a fixed name
/// keeps these runs reading the same way every time, and the obfuscated shape
/// matches the downloads the feature exists for. The well-named-folder shape
/// has its own run at the bottom of this file.
fn completed_dir() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("completed dir");
    let dir = root.path().join("a1b2c3d4e5f6a7b8c9d0");
    std::fs::create_dir(&dir).expect("create release folder");
    (root, dir)
}

/// Write a file whose bytes are unique to `name`, so every fixture file has a
/// distinct CRC. `padded` extends it past the sample-size threshold.
fn write_member(dir: &Path, name: &str, padded: bool) -> PathBuf {
    use std::io::{Seek, SeekFrom, Write};

    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create fixture member");
    file.write_all(name.as_bytes())
        .expect("write fixture member");
    if padded {
        file.seek(SeekFrom::Start(52 * 1024 * 1024))
            .expect("seek fixture member");
        file.write_all(&[0]).expect("extend fixture member");
    }
    path
}

fn member_crc(path: &Path) -> String {
    let (crc, _) = crate::import::srrdb::crc32_iso_hdlc_of_file(path).expect("checksum fixture");
    format!("{crc:08X}")
}

/// `run_import` over `dir` with the srrdb port and admin setting supplied.
async fn run_recovery_import(
    dir: &Path,
    client_type: &str,
    enabled: bool,
    lookup: Arc<FakeSrrdbLookup>,
    manual_title_id: Option<&str>,
) -> ImportResult {
    run_recovery_import_named(dir, None, client_type, enabled, lookup, manual_title_id).await
}

/// `run_recovery_import` with the download client reporting `release_name`
/// for the completed download, the way SABnzbd and NZBGet report the NZB name.
async fn run_recovery_import_named(
    dir: &Path,
    release_name: Option<&str>,
    client_type: &str,
    enabled: bool,
    lookup: Arc<FakeSrrdbLookup>,
    manual_title_id: Option<&str>,
) -> ImportResult {
    let import_repo = Arc::new(TestImportRepo::default());
    let mut completed = build_completed_download(
        release_name.unwrap_or("obfuscated download"),
        dir.to_string_lossy().as_ref(),
        None,
    );
    completed.release_name = release_name.map(str::to_string);
    completed.client_type = client_type.to_string();
    let app = build_app_with_download_client_configs_submissions_and_settings(
        recovery_titles(),
        vec![],
        vec![],
        vec![],
        TestAppRepositories {
            download_client: test_download_client_with_completed(completed.clone()),
            download_client_configs: Arc::new(NullDownloadClientConfigRepository),
            download_submissions: Arc::new(
                crate::null_repositories::NullDownloadSubmissionRepository,
            ),
            settings: Arc::new(SrrdbSettingsRepo { enabled }),
        },
    )
    .with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_srrdb_filename_lookup(lookup as Arc<dyn SrrdbFilenameLookup>)
    });

    let mut td = build_tracked_download("title-series", "series", "obfuscated download");
    td.client_type = client_type.to_string();
    td.client_item.client_type = client_type.to_string();
    td.id = format!("{client_type}:obfuscated download");
    td.state = TrackedDownloadState::ImportPending;
    td.facet = None;
    td.client_item.facet = None;
    // A titleless download: nothing bound this to a title, so import has to
    // work out the target from the files themselves.
    td.title_id = manual_title_id.map(str::to_string);
    td.client_item.title_id = td.title_id.clone();
    td.match_type = if manual_title_id.is_some() {
        TitleMatchType::Submission
    } else {
        TitleMatchType::Unmatched
    };
    td.client_item.is_scryer_origin = false;
    let lookup_index =
        index_completed_downloads(vec![completed], CompletedDownloadLookupCoverage::Recent);

    import_with_lookup(&app, &recovery_actor(), &mut td, &lookup_index).await;

    import_repo
        .last_import_result()
        .await
        .expect("import must record a result")
}

// ── the gate ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn srrdb_is_never_consulted_while_the_admin_setting_is_off() {
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    write_member(dir, OBFUSCATED_MEMBERS[0], false);
    let lookup = Arc::new(FakeSrrdbLookup::default());

    let result = run_recovery_import(dir, "sabnzbd", false, lookup.clone(), None).await;

    assert_eq!(
        lookup.call_count().await,
        0,
        "the switch is off, so nothing may be hashed or looked up"
    );
    assert_eq!(result.decision, ImportDecision::Unmatched, "{result:?}");
}

#[tokio::test]
async fn srrdb_is_never_consulted_for_a_non_usenet_client() {
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    let member = write_member(dir, OBFUSCATED_MEMBERS[0], false);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[(
        member_crc(&member).as_str(),
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
    )]));

    let result = run_recovery_import(dir, "weaver", true, lookup.clone(), None).await;

    assert_eq!(
        lookup.call_count().await,
        0,
        "recovery is a SABnzbd/NZBGet unpack story only"
    );
    assert_eq!(result.decision, ImportDecision::Unmatched, "{result:?}");
}

// ── the candidate rule ─────────────────────────────────────────────────────

#[tokio::test]
async fn srrdb_is_only_asked_about_files_with_no_title_signal_of_their_own() {
    // A pack bound to its title: one member is obfuscated, the other names
    // itself. Only the obfuscated member may be hashed or looked up.
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    let obfuscated = write_member(dir, OBFUSCATED_MEMBERS[0], true);
    let named = write_member(dir, "Harbor.Pals.S01E03.1080p.WEB.H264-LANTERNS.mkv", true);
    let obfuscated_crc = member_crc(&obfuscated);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[(
        obfuscated_crc.as_str(),
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
    )]));

    run_recovery_import(dir, "sabnzbd", true, lookup.clone(), Some("title-series")).await;

    let calls = lookup.calls.lock().await.clone();
    assert_eq!(
        calls.len(),
        1,
        "the already-titled member must never be hashed or looked up: {calls:?}"
    );
    assert_eq!(calls[0].0, obfuscated_crc);
    assert_eq!(
        calls[0].1,
        std::fs::metadata(&obfuscated).expect("member size").len()
    );
    assert!(named.exists(), "no file on disk is ever renamed");
    assert!(obfuscated.exists(), "no file on disk is ever renamed");
}

// ── recovered names drive title matching ───────────────────────────────────

#[tokio::test]
async fn a_recovered_episode_name_matches_a_titleless_series_download() {
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    let member = write_member(dir, OBFUSCATED_MEMBERS[0], false);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[(
        member_crc(&member).as_str(),
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
    )]));

    let result = run_recovery_import(dir, "sabnzbd", true, lookup.clone(), None).await;

    assert_eq!(lookup.call_count().await, 1);
    assert_eq!(
        result.title_id.as_deref(),
        Some("title-series"),
        "the recovered name is what identifies the title: {result:?}"
    );
    assert_ne!(
        result.decision,
        ImportDecision::Unmatched,
        "the download must no longer be unmatched: {result:?}"
    );
    assert!(
        member.exists(),
        "the file on disk keeps its obfuscated name"
    );
}

#[tokio::test]
async fn a_recovered_movie_name_matches_a_titleless_movie_download() {
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    let member = write_member(dir, OBFUSCATED_MEMBERS[0], false);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[(
        member_crc(&member).as_str(),
        "Paper.Lantern.2012.1080p.WEB-DL.H264-LANTERNS.mkv",
    )]));

    let result = run_recovery_import(dir, "sabnzbd", true, lookup.clone(), None).await;

    assert_eq!(lookup.call_count().await, 1);
    assert_ne!(
        result.decision,
        ImportDecision::Unmatched,
        "the recovered name must identify the title: {result:?}"
    );
    // The movie facet is proof of which title was chosen: this fixture app has
    // no configured library root, so the import gets as far as resolving the
    // movie title's own library and stops there.
    assert!(
        result
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains(&scryer_domain::default_library_id_for_facet(
                &MediaFacet::Movie
            )),
        "the import must have landed on the movie title: {result:?}"
    );
    assert!(
        member.exists(),
        "the file on disk keeps its obfuscated name"
    );
}

#[tokio::test]
async fn an_unrecoverable_member_leaves_the_import_exactly_where_it_was() {
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    write_member(dir, OBFUSCATED_MEMBERS[0], false);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[]));

    let result = run_recovery_import(dir, "sabnzbd", true, lookup.clone(), None).await;

    assert_eq!(lookup.call_count().await, 1);
    assert_eq!(
        result.decision,
        ImportDecision::Unmatched,
        "a miss must land on today's path: {result:?}"
    );
}

// ── the per-import outage breaker ──────────────────────────────────────────

#[tokio::test]
async fn the_first_outage_stops_every_remaining_lookup_in_the_import() {
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    for member in OBFUSCATED_MEMBERS {
        write_member(dir, member, false);
    }
    let lookup = Arc::new(FakeSrrdbLookup::outage());

    let result = run_recovery_import(dir, "sabnzbd", true, lookup.clone(), None).await;

    assert_eq!(
        lookup.call_count().await,
        1,
        "one outage must silence the rest of this import"
    );
    assert_eq!(
        result.decision,
        ImportDecision::Unmatched,
        "an outage must land on exactly the feature-off path: {result:?}"
    );
}

// ── the poller-retry path ──────────────────────────────────────────────────

#[tokio::test]
async fn a_download_already_bound_to_a_title_still_recovers_its_pack_members() {
    // The title is known up front (a retry carries the target), so the
    // titleless probe never runs. The pack members are still obfuscated, and
    // planning cannot place them without their original names.
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    let members: Vec<PathBuf> = OBFUSCATED_MEMBERS
        .iter()
        .map(|member| write_member(dir, member, true))
        .collect();
    let recovered = [
        "Harbor.Pals.S01E01.1080p.WEB.H264-LANTERNS.mkv",
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
        "Harbor.Pals.S01E03.1080p.WEB.H264-LANTERNS.mkv",
    ];
    let crcs: Vec<String> = members.iter().map(|member| member_crc(member)).collect();
    let table: Vec<(&str, &str)> = crcs.iter().map(String::as_str).zip(recovered).collect();
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&table));

    run_recovery_import(dir, "sabnzbd", true, lookup.clone(), Some("title-series")).await;

    assert_eq!(
        lookup.call_count().await,
        3,
        "every obfuscated member of the pack must be recovered"
    );
    for member in &members {
        assert!(member.exists(), "no file on disk is ever renamed");
    }
}

#[tokio::test]
async fn a_properly_named_release_folder_still_recovers_its_obfuscated_pack_members() {
    // The common real-world shape: the indexer's NZB carried the proper
    // release name, so SABnzbd unpacked into a well-named folder, but the
    // archive members inside are obfuscated. The folder identifies the title
    // and season; it cannot identify which member is which episode. Every
    // obfuscated member still has to be looked up or the pack parks.
    let root = tempfile::tempdir().expect("completed dir");
    let release = "Harbor.Pals.S01.1080p.WEB.H264-LANTERNS";
    let dir = root.path().join(release);
    std::fs::create_dir(&dir).expect("create release folder");
    let members: Vec<PathBuf> = OBFUSCATED_MEMBERS
        .iter()
        .map(|member| write_member(&dir, member, true))
        .collect();
    let recovered = [
        "Harbor.Pals.S01E01.1080p.WEB.H264-LANTERNS.mkv",
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
        "Harbor.Pals.S01E03.1080p.WEB.H264-LANTERNS.mkv",
    ];
    let crcs: Vec<String> = members.iter().map(|member| member_crc(member)).collect();
    let table: Vec<(&str, &str)> = crcs.iter().map(String::as_str).zip(recovered).collect();
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&table));

    let result = run_recovery_import_named(
        &dir,
        Some(release),
        "sabnzbd",
        true,
        lookup.clone(),
        Some("title-series"),
    )
    .await;

    assert_eq!(
        lookup.call_count().await,
        3,
        "a well-named folder does not tell a pack member which episode it is; every obfuscated member must be looked up (result: {result:?})"
    );
    for member in &members {
        assert!(member.exists(), "no file on disk is ever renamed");
    }
}

// ── the waste guard ────────────────────────────────────────────────────────

#[tokio::test]
async fn a_single_file_under_a_usable_release_name_is_never_looked_up() {
    // One video file, the title already bound, and a release name that names
    // the episode. Planning reads that release name and places the file today
    // with no help at all, so hashing it and asking srrdb would be a
    // third-party request that buys nothing.
    let (_root, dir) = completed_dir();
    let dir = dir.as_path();
    let member = write_member(dir, OBFUSCATED_MEMBERS[0], true);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[(
        member_crc(&member).as_str(),
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
    )]));

    let result = run_recovery_import_named(
        dir,
        Some("Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS"),
        "sabnzbd",
        true,
        lookup.clone(),
        Some("title-series"),
    )
    .await;

    assert_eq!(
        lookup.call_count().await,
        0,
        "the release name already identifies this single file; nothing may be hashed or looked up (result: {result:?})"
    );
    assert!(member.exists(), "no file on disk is ever renamed");
}

#[tokio::test]
async fn a_usable_release_name_that_matches_no_title_is_never_looked_up() {
    // An adopted download whose NZB name is a perfectly good release name for
    // a title Scryer does not have. The recovered scene name would carry the
    // same title, so the download stays unmatched without any hashing or any
    // third-party request.
    let root = tempfile::tempdir().expect("completed dir");
    let release = "Quiet.Marsh.S01E02.1080p.WEB.H264-LANTERNS";
    let dir = root.path().join(release);
    std::fs::create_dir(&dir).expect("create release folder");
    let member = write_member(&dir, OBFUSCATED_MEMBERS[0], true);
    let lookup = Arc::new(FakeSrrdbLookup::recovering(&[(
        member_crc(&member).as_str(),
        "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS.mkv",
    )]));

    let result =
        run_recovery_import_named(&dir, Some(release), "sabnzbd", true, lookup.clone(), None).await;

    assert_eq!(
        lookup.call_count().await,
        0,
        "a usable release name is never second-guessed through srrdb: {result:?}"
    );
    assert_eq!(result.decision, ImportDecision::Unmatched, "{result:?}");
}
