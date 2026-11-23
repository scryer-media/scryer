use std::path::PathBuf;

use scryer_application::{AppUseCase, EmptyImportLeftover};
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

/// Removes the empty files that copy imports left at their library names when
/// they failed at the last step.
///
/// The stored result of each import is the only record of where: its
/// destination field when it has one, and otherwise the destination its
/// failure sentence names. Nothing walks a library. A named path is removed
/// only as the application decides it: under a library root, a regular file,
/// of no bytes. Anything else is left alone, so a path read wrongly out of a
/// sentence costs at most one stat.
///
/// One pass over the stored imports, in pages by id, off the startup path.
/// A later start runs it again only if a read of the store failed.
pub(crate) async fn migrate(app: &AppUseCase, datastore: &StoreDatastore) -> Result<(), String> {
    let roots = app
        .empty_import_leftover_roots()
        .await
        .map_err(|error| error.to_string())?;
    let mut cursor = String::new();
    let (mut named, mut removed, mut rows_removed, mut failed) = (0usize, 0usize, 0usize, 0usize);
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
        for outcome in app
            .remove_empty_import_leftovers(&roots, &paths)
            .await
            .map_err(|error| error.to_string())?
        {
            match outcome {
                EmptyImportLeftover::Removed { catalog_row } => {
                    removed += 1;
                    rows_removed += usize::from(catalog_row);
                }
                EmptyImportLeftover::Failed => failed += 1,
                _ => {}
            }
        }
    }
    tracing::info!(
        migration_id = ID,
        named,
        removed,
        catalog_rows_removed = rows_removed,
        failed,
        "checked the destinations of failed copy imports for empty files"
    );
    Ok(())
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
    let Some(paths) = sentence
        .split_once(FAILED_COPY)
        .and_then(|(_, rest)| rest.split_once(ATTEMPTS))
        .and_then(|(_, rest)| rest.split_once(LAST_STEP))
        .map(|(paths, _)| paths)
    else {
        return Vec::new();
    };
    let mut named: Vec<PathBuf> = Vec::new();
    if let Some(dest) = result.get("dest_path").and_then(Value::as_str)
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
