//! Plan 149 / WP4: re-key the two identity digests that are stored as lookup
//! keys rather than compared values.
//!
//! Both are recomputed rather than dropped. A `DELETE` would be simpler but
//! loses real state: pending media requests would lose their dedup identity and
//! unmatched-scan rows would vanish from review until the next scan.
//!
//! Neither engine can compute BLAKE3, so this runs as a Rust hook. Both
//! backfills are idempotent — a row already carrying the new value recomputes to
//! the same value — and each is a no-op once complete.

use std::collections::{BTreeMap, HashSet};

use scryer_application::{AppError, AppResult, HashDomain, blake3_identity_hex};
use sqlx::Row;

/// Recomputed `media_requests.identity_fingerprint`, keyed by request id.
///
/// The input mirrors `media_request_identity_fingerprint`: `source:value` pairs
/// joined with `|`, in the `ORDER BY source, external_id` the loader uses.
fn media_request_fingerprints(rows: &[(String, String, String)]) -> Vec<(String, String)> {
    let mut by_request: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (request_id, source, external_id) in rows {
        by_request
            .entry(request_id.as_str())
            .or_default()
            .push(format!("{source}:{external_id}"));
    }
    by_request
        .into_iter()
        .map(|(request_id, parts)| {
            (
                request_id.to_string(),
                blake3_identity_hex(HashDomain::MediaRequestIdentity, parts.join("|")),
            )
        })
        .collect()
}

/// Recomputed `library_scan_unmatched_items.id`.
///
/// Mirrors `build_library_scan_unmatched_item_id`: the digest covers
/// `facet:library_id:item_path` and only its first 24 hex characters are used.
///
/// Returns `None` for a NULL `library_id`. The column was added by migration
/// 0104 without a backfill, so a NULL means the row predates it and the id was
/// derived from an input this function cannot reconstruct. Those rows are left
/// alone — see `plan_unmatched_rekey` for why that is safe.
fn unmatched_item_id(facet: &str, library_id: Option<&str>, item_path: &str) -> Option<String> {
    let library_id = library_id?;
    let fingerprint = blake3_identity_hex(
        HashDomain::LibraryScanUnmatchedItem,
        format!("{facet}:{library_id}:{item_path}"),
    );
    Some(format!("library_scan_unmatched:{}", &fingerprint[..24]))
}

/// `(old_id, new_id)` pairs to apply, with every row that could abort the
/// migration filtered out.
///
/// The id is the primary key, so a duplicate target aborts the whole
/// transaction and fails the upgrade. Two hazards produce one:
///
/// 1. The unique index is `(library_id, item_path)`, and both engines treat
///    NULLs as distinct — so several NULL-`library_id` rows may share an
///    `item_path`. All of them would recompute to the same id.
/// 2. Any future shape change that makes two live rows agree on the hashed
///    triple.
///
/// Skipping is free because the table re-keys itself: the scan upsert is
/// `ON CONFLICT(library_id, item_path) DO UPDATE SET id = excluded.id`, so the
/// next scan of a path rewrites its id to the current form regardless. This
/// backfill only makes that state immediate for rows that may never be
/// re-scanned; a row it declines to touch is stale, not broken.
fn plan_unmatched_rekey(
    rows: &[(String, String, Option<String>, String)],
) -> Vec<(String, String)> {
    let existing: HashSet<&str> = rows.iter().map(|(id, ..)| id.as_str()).collect();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut plan = Vec::new();
    for (id, facet, library_id, item_path) in rows {
        let Some(next_id) = unmatched_item_id(facet, library_id.as_deref(), item_path) else {
            continue;
        };
        if &next_id == id {
            continue;
        }
        // Colliding with another row's target, or with a row this pass has not
        // re-keyed yet, would violate the primary key mid-transaction.
        if !claimed.insert(next_id.clone()) || existing.contains(next_id.as_str()) {
            continue;
        }
        plan.push((id.clone(), next_id));
    }
    plan
}

fn repo_err(error: sqlx::Error) -> AppError {
    AppError::Repository(error.to_string())
}

pub async fn backfill_blake3_identities_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT request_id, source, external_id
           FROM media_request_external_ids
          ORDER BY request_id, source, external_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let external_ids = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("request_id").map_err(repo_err)?,
                row.try_get::<String, _>("source").map_err(repo_err)?,
                row.try_get::<String, _>("external_id").map_err(repo_err)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    for (request_id, fingerprint) in media_request_fingerprints(&external_ids) {
        sqlx::query("UPDATE media_requests SET identity_fingerprint = ?1 WHERE id = ?2")
            .bind(&fingerprint)
            .bind(&request_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    // `plan_unmatched_rekey` filters every row that could violate the primary
    // key, so these updates cannot abort the migration.
    let rows =
        sqlx::query("SELECT id, facet, library_id, item_path FROM library_scan_unmatched_items")
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?;
    let unmatched = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("id").map_err(repo_err)?,
                row.try_get::<String, _>("facet").map_err(repo_err)?,
                row.try_get::<Option<String>, _>("library_id")
                    .map_err(repo_err)?,
                row.try_get::<String, _>("item_path").map_err(repo_err)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    for (old_id, next_id) in plan_unmatched_rekey(&unmatched) {
        sqlx::query("UPDATE library_scan_unmatched_items SET id = ?1 WHERE id = ?2")
            .bind(&next_id)
            .bind(&old_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    Ok(())
}

pub async fn backfill_blake3_identities_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT request_id, source, external_id
           FROM media_request_external_ids
          ORDER BY request_id, source, external_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    let external_ids = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("request_id").map_err(repo_err)?,
                row.try_get::<String, _>("source").map_err(repo_err)?,
                row.try_get::<String, _>("external_id").map_err(repo_err)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    for (request_id, fingerprint) in media_request_fingerprints(&external_ids) {
        sqlx::query("UPDATE media_requests SET identity_fingerprint = $1 WHERE id = $2")
            .bind(&fingerprint)
            .bind(&request_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    let rows =
        sqlx::query("SELECT id, facet, library_id, item_path FROM library_scan_unmatched_items")
            .fetch_all(&mut **tx)
            .await
            .map_err(repo_err)?;
    let unmatched = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("id").map_err(repo_err)?,
                row.try_get::<String, _>("facet").map_err(repo_err)?,
                row.try_get::<Option<String>, _>("library_id")
                    .map_err(repo_err)?,
                row.try_get::<String, _>("item_path").map_err(repo_err)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    for (old_id, next_id) in plan_unmatched_rekey(&unmatched) {
        sqlx::query("UPDATE library_scan_unmatched_items SET id = $1 WHERE id = $2")
            .bind(&next_id)
            .bind(&old_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_request_fingerprint_groups_and_orders_external_ids() {
        let rows = vec![
            ("req-1".to_string(), "tmdb".to_string(), "42".to_string()),
            ("req-1".to_string(), "imdb".to_string(), "tt7".to_string()),
            ("req-2".to_string(), "tvdb".to_string(), "9".to_string()),
        ];
        let out = media_request_fingerprints(&rows);
        assert_eq!(out.len(), 2);

        // Grouping preserves the caller's row order within a request, matching
        // the loader's ORDER BY source, external_id.
        let expected_req_1 =
            blake3_identity_hex(HashDomain::MediaRequestIdentity, "tmdb:42|imdb:tt7");
        assert_eq!(out[0], ("req-1".to_string(), expected_req_1));
    }

    #[test]
    fn unmatched_item_id_shape_matches_the_producer() {
        let id = unmatched_item_id("movie", Some("lib"), "/media/a").expect("library-backed id");
        assert!(id.starts_with("library_scan_unmatched:"));
        assert_eq!(id.len(), "library_scan_unmatched:".len() + 24);
    }

    #[test]
    fn unmatched_item_id_separates_facets() {
        assert_ne!(
            unmatched_item_id("movie", Some("lib"), "/media/a"),
            unmatched_item_id("series", Some("lib"), "/media/a")
        );
    }

    #[test]
    fn a_null_library_id_is_not_re_keyed() {
        // The id of a pre-0104 row was derived from an input we cannot rebuild,
        // and several such rows may share an item_path because the unique index
        // treats NULLs as distinct. Re-keying them would collide on the primary
        // key and abort the upgrade.
        assert_eq!(unmatched_item_id("movie", None, "/media/a"), None);

        let rows = vec![
            (
                "old-1".to_string(),
                "movie".to_string(),
                None,
                "/media/a".to_string(),
            ),
            (
                "old-2".to_string(),
                "movie".to_string(),
                None,
                "/media/a".to_string(),
            ),
        ];
        assert!(plan_unmatched_rekey(&rows).is_empty());
    }

    #[test]
    fn plan_skips_targets_that_would_violate_the_primary_key() {
        let live = unmatched_item_id("movie", Some("lib"), "/media/a").expect("id");

        // A row whose target id is already held by a different row must be left
        // alone rather than colliding mid-transaction.
        let rows = vec![
            (
                "stale".to_string(),
                "movie".to_string(),
                Some("lib".to_string()),
                "/media/a".to_string(),
            ),
            (
                live.clone(),
                "movie".to_string(),
                Some("lib".to_string()),
                "/media/b".to_string(),
            ),
        ];
        let plan = plan_unmatched_rekey(&rows);
        assert!(
            plan.iter().all(|(_, next)| next != &live),
            "must not re-key onto an id another row still holds"
        );
    }

    #[test]
    fn plan_re_keys_an_ordinary_row_and_is_idempotent() {
        let rows = vec![(
            "library_scan_unmatched:deadbeefdeadbeefdeadbeef".to_string(),
            "movie".to_string(),
            Some("lib".to_string()),
            "/media/a".to_string(),
        )];
        let plan = plan_unmatched_rekey(&rows);
        assert_eq!(plan.len(), 1);
        let (_, next_id) = &plan[0];

        // Re-running over the already-migrated row plans nothing.
        let migrated = vec![(
            next_id.clone(),
            "movie".to_string(),
            Some("lib".to_string()),
            "/media/a".to_string(),
        )];
        assert!(plan_unmatched_rekey(&migrated).is_empty());
    }
}
