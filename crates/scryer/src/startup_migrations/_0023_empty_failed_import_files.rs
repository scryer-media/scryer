use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use scryer_application::disk::{Disk, EntryKind, Mark, Remove};
use scryer_application::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};
use serde_json::Value;

pub(crate) const ID: &str = "0023_empty_failed_import_files";
pub(crate) const DESCRIPTION: &str =
    "remove the empty files a failed copy import left at library destinations";

/// The words a copy import's failure at its last step was stored with. Only
/// that step could leave an empty file at the destination name.
const FAILED_COPY: &str = "import copy failed after ";
const ATTEMPTS: &str = "attempt(s): ";
const LAST_STEP: &str = ": final rename: ";
const PAGE: usize = 500;
/// Paths per catalog lookup, far under either store's bind limit.
const LOOKUP_CHUNK: usize = 400;
/// An empty file written more recently than this may be a name another
/// operation has reserved and has yet to fill.
const SETTLED: Duration = Duration::from_secs(60 * 60);

/// Removes the empty files that copy imports left at their library names when
/// they failed at the last step.
///
/// The stored result of each import is the only record of where: its
/// destination field when it has one, and otherwise the destination its
/// failure sentence names. Nothing walks a library. A named path is removed
/// only when it is under a library root and is a regular file of no bytes.
/// Anything else is left alone, so a path read wrongly out of a sentence
/// costs at most one stat.
///
/// Everything this pass does lives in this file. It reads the store and
/// writes nothing to it: a catalog row that recorded the empty file is left
/// for the next scan of its title, which drops the row of a vanished file
/// and puts the item back up for search.
///
/// One pass over the stored imports, in pages by id, off the startup path.
/// A later start runs it again only if a read of the store failed.
pub(crate) async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    let roots = library_roots(datastore).await?;
    let mut cursor = String::new();
    let (mut named, mut removed, mut cataloged, mut failed) = (0usize, 0usize, 0usize, 0usize);
    loop {
        let rows = SqlRuntime::fetch_all(
            datastore.read_exec(),
            &format!(
                "SELECT id, result_json FROM imports
                 WHERE id > {{}} AND result_json IS NOT NULL
                 ORDER BY id LIMIT {PAGE}"
            ),
            &[SqlArg::Text(cursor.clone())],
        )
        .await
        .map_err(|error| error.to_string())?;
        let Some(last) = rows.last() else { break };
        cursor = last.text("id").map_err(|error| error.to_string())?;

        let mut paths: Vec<PathBuf> = Vec::new();
        for row in &rows {
            // A result that does not decode names nothing.
            if let Ok(Some(result)) = row.opt_json("result_json") {
                for path in destinations_named(&result) {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
        if paths.is_empty() {
            continue;
        }
        named += paths.len();
        for outcome in remove_empty_files(datastore, &roots, &paths).await? {
            match outcome {
                Outcome::Removed { cataloged: row } => {
                    removed += 1;
                    cataloged += usize::from(row);
                }
                Outcome::Failed => failed += 1,
                _ => {}
            }
        }
    }
    tracing::info!(
        migration_id = ID,
        named,
        removed,
        removed_with_a_catalog_row_left_to_the_scan = cataloged,
        failed,
        "checked the destinations of failed copy imports for empty files"
    );
    Ok(())
}

/// What became of one named path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Not an absolute, plain path under a library root. Never read from disk.
    OutsideLibraryRoots,
    /// Nothing there, not a regular file, a file with content, or an empty
    /// file written too recently to be a leftover.
    NotAnEmptyFile,
    /// The catalog records content for this path, so the empty file is not
    /// what the catalog knows. Left as it is.
    CatalogedWithContent,
    /// It stopped being the empty file that was read before its removal.
    Changed,
    /// The empty file is gone. `cataloged` when the catalog recorded an empty
    /// file at its path; that row is the next scan's to drop.
    Removed { cataloged: bool },
    /// The file could not be removed. Logged with the cause.
    Failed,
}

/// Every configured library root.
async fn library_roots(datastore: &StoreDatastore) -> Result<Vec<PathBuf>, String> {
    let rows = SqlRuntime::fetch_all(datastore.read_exec(), "SELECT path FROM library_roots", &[])
        .await
        .map_err(|error| error.to_string())?;
    let mut roots = Vec::with_capacity(rows.len());
    for row in &rows {
        let path = row.text("path").map_err(|error| error.to_string())?;
        if !path.trim().is_empty() {
            roots.push(stored_path_to_path_buf(path.trim()));
        }
    }
    Ok(roots)
}

/// The size the catalog records at exactly each of these stored paths. A path stored in another spelling is not matched; its file is
/// then judged as one the catalog does not know.
async fn cataloged(
    datastore: &StoreDatastore,
    stored: &[String],
) -> Result<HashMap<String, i64>, String> {
    let mut found = HashMap::new();
    for chunk in stored.chunks(LOOKUP_CHUNK) {
        let marks = vec!["{}"; chunk.len()].join(", ");
        let args: Vec<SqlArg> = chunk.iter().cloned().map(SqlArg::Text).collect();
        let rows = SqlRuntime::fetch_all(
            datastore.read_exec(),
            &format!("SELECT file_path, size_bytes FROM media_files WHERE file_path IN ({marks})"),
            &args,
        )
        .await
        .map_err(|error| error.to_string())?;
        for row in &rows {
            found.insert(
                row.text("file_path").map_err(|error| error.to_string())?,
                row.i64("size_bytes").map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(found)
}

/// Removes each of `paths` that is an empty regular file under a library
/// root, one outcome per path in order.
///
/// A link, a folder, a file with content and a path outside the roots are
/// never touched, and neither is an empty file written within the last hour,
/// which may be a name another operation has reserved. A file the catalog
/// points at goes only when the catalog too records it as empty.
///
/// Probes: one stat per path under a root, and a second only for a path that
/// read as an empty file. One catalog read for the batch.
async fn remove_empty_files(
    datastore: &StoreDatastore,
    roots: &[PathBuf],
    paths: &[PathBuf],
) -> Result<Vec<Outcome>, String> {
    let mut outcomes = vec![Outcome::OutsideLibraryRoots; paths.len()];
    let mut empty: Vec<(usize, Mark)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if !is_plain_path_under_a_root(path, roots) {
            continue;
        }
        outcomes[index] = Outcome::NotAnEmptyFile;
        if let Ok(mark) = Disk::process().fingerprint(path.clone()).await
            && is_empty_file(&mark)
        {
            empty.push((index, mark));
        }
    }
    if empty.is_empty() {
        return Ok(outcomes);
    }

    let stored: Vec<String> = empty
        .iter()
        .map(|(index, _)| path_to_stored_string(&paths[*index]))
        .collect();
    let cataloged = cataloged(datastore, &stored).await?;

    for ((index, mark), stored) in empty.into_iter().zip(stored) {
        let path = &paths[index];
        let row = cataloged.get(&stored);
        if row.is_some_and(|size_bytes| *size_bytes != 0) {
            outcomes[index] = Outcome::CatalogedWithContent;
            tracing::info!(
                path = %path.display(),
                "an empty file at a failed import's destination is kept: the catalog records content for it"
            );
            continue;
        }
        let still_empty = Disk::process()
            .fingerprint_if_same_entry(path.clone(), &mark)
            .await
            .is_some_and(|now| is_empty_file(&now));
        if !still_empty {
            outcomes[index] = Outcome::Changed;
            continue;
        }
        if let Err(error) = Disk::process().remove(path.clone(), Remove::File).await {
            outcomes[index] = Outcome::Failed;
            tracing::warn!(
                path = %path.display(),
                error = %error.words(),
                "the empty file a failed import left could not be removed"
            );
            continue;
        }
        outcomes[index] = Outcome::Removed {
            cataloged: row.is_some(),
        };
        tracing::info!(
            path = %path.display(),
            cataloged = row.is_some(),
            "removed the empty file a failed import left at its destination"
        );
    }
    Ok(outcomes)
}

/// An empty regular file last written at least [`SETTLED`] ago by this
/// host's clock. One whose time is unread or ahead of the clock is not.
fn is_empty_file(mark: &Mark) -> bool {
    mark.kind() == Some(EntryKind::File)
        && mark.is_len(0)
        && mark
            .modified()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= SETTLED)
}

/// An absolute path of plain names only, under one of `roots` and not a root
/// itself. Whole names are compared as they are spelled. No probe.
fn is_plain_path_under_a_root(path: &Path, roots: &[PathBuf]) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
        && roots
            .iter()
            .any(|root| root.is_absolute() && root != path && path.starts_with(root))
}

/// The destinations a stored import result names for a copy that failed at
/// its last step; none for any other result.
///
/// The sentence reads `… attempt(s): <source> -> <destination>: final rename:
/// …`. A source may itself hold ` -> `, so every split is offered and the
/// application keeps only one that is an empty file under a library root.
fn destinations_named(result: &Value) -> Vec<PathBuf> {
    let Some(sentence) = result.get("error_message").and_then(Value::as_str) else {
        return Vec::new();
    };
    let mut named: Vec<PathBuf> = Vec::new();
    // One sentence can hold the failures of several files.
    for failure in sentence.split(FAILED_COPY).skip(1) {
        let Some(paths) = failure
            .split_once(ATTEMPTS)
            .and_then(|(_, rest)| rest.split_once(LAST_STEP))
            .map(|(paths, _)| paths)
        else {
            continue;
        };
        if named.is_empty()
            && let Some(dest) = result.get("dest_path").and_then(Value::as_str)
            && !dest.trim().is_empty()
        {
            named.push(PathBuf::from(dest));
        }
        for (at, arrow) in paths.match_indices(" -> ") {
            let dest = PathBuf::from(&paths[at + arrow.len()..]);
            if !named.contains(&dest) {
                named.push(dest);
            }
        }
    }
    named
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn named(result: Value) -> Vec<String> {
        destinations_named(&result)
            .into_iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    #[test]
    fn a_copy_that_failed_at_its_last_step_names_its_destination() {
        let sentence = "import copy failed after 1 attempt(s): /downloads/Synthetic.Show.S01E01.mkv -> /library/Synthetic Show/Season 01/Synthetic Show - S01E01.mkv: final rename: Permission denied (os error 13)";
        assert_eq!(
            named(json!({ "error_message": sentence, "dest_path": null })),
            vec!["/library/Synthetic Show/Season 01/Synthetic Show - S01E01.mkv"]
        );
        assert_eq!(
            named(json!({ "error_message": format!("episode 1: {sentence}; kept") })),
            vec!["/library/Synthetic Show/Season 01/Synthetic Show - S01E01.mkv"]
        );
    }

    #[test]
    fn the_destination_field_is_named_first_and_once() {
        let sentence = "import copy failed after 2 attempt(s): /downloads/a.mkv -> /library/Synthetic Movie (2001)/a.mkv: final rename: Permission denied (os error 13)";
        assert_eq!(
            named(json!({
                "error_message": sentence,
                "dest_path": "/library/Synthetic Movie (2001)/a.mkv"
            })),
            vec!["/library/Synthetic Movie (2001)/a.mkv"]
        );
    }

    #[test]
    fn a_source_holding_an_arrow_offers_every_split() {
        let sentence = "import copy failed after 1 attempt(s): /downloads/a -> b.mkv -> /library/x.mkv: final rename: denied";
        assert_eq!(
            named(json!({ "error_message": sentence })),
            vec!["b.mkv -> /library/x.mkv", "/library/x.mkv"]
        );
    }

    #[test]
    fn every_failure_in_one_sentence_is_read() {
        let sentence = "episode 1: import copy failed after 1 attempt(s): /d/a.mkv -> /library/a.mkv: flush: Input/output error; episode 2: import copy failed after 1 attempt(s): /d/b.mkv -> /library/b.mkv: final rename: denied; episode 3: import copy failed after 1 attempt(s): /d/c.mkv -> /library/c.mkv: final rename: denied";
        assert_eq!(
            named(json!({ "error_message": sentence })),
            vec!["/library/b.mkv", "/library/c.mkv"]
        );
    }

    #[test]
    fn other_results_name_nothing() {
        for result in [
            json!({}),
            json!({ "error_message": null, "dest_path": "/library/x.mkv" }),
            json!({ "error_message": "no space left", "dest_path": "/library/x.mkv" }),
            json!({
                "error_message": "import copy failed after 1 attempt(s): /d/a.mkv -> /library/x.mkv: flush: Input/output error",
                "dest_path": "/library/x.mkv"
            }),
            json!({ "error_message": "/d/a.mkv -> /library/x.mkv: final rename: denied" }),
        ] {
            assert!(named(result).is_empty());
        }
    }
}

/// The pass against a real store and synthetic files: an empty regular file
/// under a library root goes, a cataloged one only when the catalog records
/// it empty, and everything else is kept, catalog rows included.
#[cfg(test)]
mod store_tests {
    use super::*;
    use crate::middleware::integration_test_common as common;
    use scryer_application::TitleRepository;

    const SENTENCE_HEAD: &str =
        "import copy failed after 1 attempt(s): /downloads/synthetic.mkv -> ";
    const SENTENCE_TAIL: &str = ": final rename: Permission denied (os error 13)";

    /// Sets a file's time well past [`SETTLED`].
    fn aged(path: &Path) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(SystemTime::now() - 2 * SETTLED))
            .expect("age the file");
    }

    struct Fixture {
        ctx: common::TestContext,
        temp: tempfile::TempDir,
    }

    impl Fixture {
        async fn new() -> Self {
            let ctx = common::TestContext::new().await;
            let temp = tempfile::tempdir().expect("fixture tempdir");
            let fixture = Self { ctx, temp };
            std::fs::create_dir_all(fixture.folder()).expect("create title folder");
            let now = chrono::Utc::now();
            fixture
                .write(
                    "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
                     VALUES ('synthetic-library', 'movie', 'Synthetic', 'synthetic', {}, {}, {})",
                    vec![
                        SqlArg::Bool(false),
                        SqlArg::Timestamp(now),
                        SqlArg::Timestamp(now),
                    ],
                )
                .await;
            let root = path_to_stored_string(fixture.root());
            fixture
                .write(
                    "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
                     VALUES ('synthetic-root', 'synthetic-library', {}, {}, {}, {}, {})",
                    vec![
                        SqlArg::Text(root.clone()),
                        SqlArg::Text(root),
                        SqlArg::Bool(false),
                        SqlArg::Timestamp(now),
                        SqlArg::Timestamp(now),
                    ],
                )
                .await;
            fixture
        }

        fn datastore(&self) -> StoreDatastore {
            self.ctx.db.datastore()
        }

        fn root(&self) -> PathBuf {
            self.temp.path().join("library-a")
        }

        fn folder(&self) -> PathBuf {
            self.root().join("Lanternfall (2001)")
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.folder().join(name);
            std::fs::write(&path, bytes).expect("write library file");
            aged(&path);
            path
        }

        async fn write(&self, sql: &str, args: Vec<SqlArg>) {
            SqlRuntime::execute_write(&self.datastore(), "seed_empty_file_fixture", sql, args)
                .await
                .expect("seed fixture row");
        }

        async fn title(&self) {
            let facet = scryer_domain::MediaFacet::Movie;
            let title = scryer_domain::Title {
                id: "synthetic-title".to_string(),
                name: "Lanternfall".to_string(),
                library_id: "synthetic-library".to_string(),
                facet,
                monitored: false,
                tags: vec![],
                canonical_tags: vec![],
                external_ids: vec![],
                root_folder_id: "synthetic-root".to_string(),
                created_by: None,
                created_at: chrono::Utc::now(),
                year: Some(2001),
                overview: None,
                poster_url: None,
                poster_source_url: None,
                background_url: None,
                background_source_url: None,
                sort_title: None,
                catalog_sort_key: String::new(),
                slug: None,
                imdb_id: None,
                runtime_minutes: None,
                popularity: None,
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
                folder_path: None,
            };
            self.ctx.titles.create(title).await.expect("seed title");
        }

        async fn catalog_row(&self, id: &str, path: &Path, size_bytes: i64) {
            self.write(
                "INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at)
                 VALUES ({}, 'synthetic-title', {}, {}, {})",
                vec![
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(path_to_stored_string(path)),
                    SqlArg::I64(size_bytes),
                    SqlArg::Timestamp(chrono::Utc::now()),
                ],
            )
            .await;
        }

        async fn catalog_row_ids(&self) -> Vec<String> {
            SqlRuntime::fetch_all(
                self.datastore().read_exec(),
                "SELECT id FROM media_files ORDER BY id",
                &[],
            )
            .await
            .expect("read catalog rows")
            .iter()
            .map(|row| row.text("id").expect("id"))
            .collect()
        }

        async fn failed_import(&self, id: &str, result: Value) {
            let now = chrono::Utc::now();
            self.write(
                "INSERT INTO imports (id, source_system, source_ref, import_type, payload_json, result_json, created_at, updated_at)
                 VALUES ({}, 'nzbget', {}, 'movie_download', {}, {}, {}, {})",
                vec![
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text(id.to_string()),
                    SqlArg::Text("{}".to_string()),
                    SqlArg::Text(result.to_string()),
                    SqlArg::Timestamp(now),
                    SqlArg::Timestamp(now),
                ],
            )
            .await;
        }

        async fn sweep(&self, paths: &[PathBuf]) -> Vec<Outcome> {
            let datastore = self.datastore();
            let roots = library_roots(&datastore).await.expect("library roots");
            remove_empty_files(&datastore, &roots, paths)
                .await
                .expect("sweep")
        }
    }

    #[tokio::test]
    async fn an_empty_file_under_a_library_root_is_removed_and_everything_else_kept() {
        let fixture = Fixture::new().await;
        let empty = fixture.file("empty.mkv", b"");
        let full = fixture.file("full.mkv", b"synthetic video bytes");
        let fresh = fixture.folder().join("fresh.mkv");
        std::fs::write(&fresh, b"").expect("write fresh");
        let missing = fixture.folder().join("missing.mkv");
        let folder = fixture.root().join("Empty Folder");
        std::fs::create_dir_all(&folder).expect("folder");
        let outside = fixture.temp.path().join("outside.mkv");
        std::fs::write(&outside, b"").expect("write outside");
        let root = fixture.root();
        let dotted = fixture.folder().join("..").join("up.mkv");
        std::fs::write(root.join("up.mkv"), b"").expect("write dotted target");

        let outcomes = fixture
            .sweep(&[
                empty.clone(),
                full.clone(),
                fresh.clone(),
                missing,
                folder.clone(),
                outside.clone(),
                root.clone(),
                dotted,
                PathBuf::from("relative/empty.mkv"),
            ])
            .await;

        assert_eq!(
            outcomes,
            vec![
                Outcome::Removed { cataloged: false },
                Outcome::NotAnEmptyFile,
                Outcome::NotAnEmptyFile,
                Outcome::NotAnEmptyFile,
                Outcome::NotAnEmptyFile,
                Outcome::OutsideLibraryRoots,
                Outcome::OutsideLibraryRoots,
                Outcome::OutsideLibraryRoots,
                Outcome::OutsideLibraryRoots,
            ]
        );
        assert!(!empty.exists());
        assert_eq!(
            std::fs::read(&full).expect("full"),
            b"synthetic video bytes"
        );
        assert!(fresh.exists());
        assert!(folder.is_dir());
        assert!(outside.exists());
        assert!(root.join("up.mkv").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_to_an_empty_file_is_kept() {
        let fixture = Fixture::new().await;
        let target = fixture.file("target.mkv", b"");
        let link = target.with_file_name("link.mkv");
        std::os::unix::fs::symlink(&target, &link).expect("link");

        let outcomes = fixture.sweep(std::slice::from_ref(&link)).await;

        assert_eq!(outcomes, vec![Outcome::NotAnEmptyFile]);
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link")
                .file_type()
                .is_symlink()
        );
        assert!(target.exists());
    }

    #[tokio::test]
    async fn a_cataloged_empty_file_goes_only_when_the_catalog_records_it_empty_and_its_row_stays()
    {
        let fixture = Fixture::new().await;
        fixture.title().await;
        let scanned_empty = fixture.file("scanned-empty.mkv", b"");
        fixture
            .catalog_row("synthetic-empty-row", &scanned_empty, 0)
            .await;
        let once_full = fixture.file("once-full.mkv", b"");
        fixture
            .catalog_row("synthetic-full-row", &once_full, 4096)
            .await;

        let outcomes = fixture
            .sweep(&[scanned_empty.clone(), once_full.clone()])
            .await;

        assert_eq!(
            outcomes,
            vec![
                Outcome::Removed { cataloged: true },
                Outcome::CatalogedWithContent,
            ]
        );
        assert!(!scanned_empty.exists());
        assert!(once_full.exists());
        assert_eq!(
            fixture.catalog_row_ids().await,
            vec!["synthetic-empty-row", "synthetic-full-row"],
            "the scan drops the row of the vanished file, not this pass"
        );
    }

    #[tokio::test]
    async fn the_pass_reads_every_page_of_stored_imports_and_removes_only_what_they_name() {
        let fixture = Fixture::new().await;
        let first = fixture.file("first.mkv", b"");
        let last = fixture.file("last.mkv", b"");
        let unnamed = fixture.file("unnamed.mkv", b"");
        let full = fixture.file("full.mkv", b"synthetic video bytes");
        let sentence = |path: &Path| format!("{SENTENCE_HEAD}{}{SENTENCE_TAIL}", path.display());

        fixture
            .failed_import(
                "synthetic-import-0000",
                serde_json::json!({ "error_message": sentence(&first), "dest_path": null }),
            )
            .await;
        fixture
            .failed_import(
                "synthetic-import-0001",
                serde_json::json!({ "error_message": sentence(&full), "dest_path": full }),
            )
            .await;
        for index in 2..=PAGE {
            fixture
                .failed_import(
                    &format!("synthetic-import-{index:04}"),
                    serde_json::json!({ "error_message": "no space left", "dest_path": unnamed }),
                )
                .await;
        }
        fixture
            .failed_import(
                &format!("synthetic-import-{:04}", PAGE + 1),
                serde_json::json!({ "error_message": sentence(&last), "dest_path": last }),
            )
            .await;

        migrate(&fixture.datastore()).await.expect("pass");

        assert!(!first.exists());
        assert!(!last.exists());
        assert!(unnamed.exists());
        assert_eq!(
            std::fs::read(&full).expect("full"),
            b"synthetic video bytes"
        );
    }
}
