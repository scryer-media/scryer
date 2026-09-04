//! Migration 0213 — synthetic stable root ids (FR-078, plan D1).
//!
//! Until now a root's id *was* its path: `root_folder_id_for_path` hashed the
//! platform-normalized path, and every root write recomputed it. Changing a
//! root's path therefore changed the root's identity, and every
//! `titles.root_folder_id` pointing at it went stale. This hook breaks that
//! functional dependency once, in one transaction, so that from 0210 onward the
//! path is a mutable attribute of a root rather than its name.
//!
//! ## What it does
//!
//! For every `library_roots` row it computes the path-derived id the old scheme
//! would produce and records it in `library_roots.legacy_path_derived_id` and in
//! `library_root_id_remaps`, so a caller still holding a path-derived id can
//! resolve the real root and so the remap is auditable afterwards.
//!
//! A root whose current id *equals* its path-derived id is re-keyed to a
//! synthetic id and every referent is rewritten to match. A root whose id was
//! never path-derived — the baseline's seeded `canonical_root_for_*` rows, or an
//! operator-supplied id — already satisfies the invariant this migration exists
//! to establish, so it keeps the id it has. Churning those would break stable
//! references for no gain. The post-condition the hook asserts is the one that
//! matters: after 0210 no root's id is its path-derived id.
//!
//! ## Referents
//!
//! Root ids live in exactly two places in the schema: `library_roots.id` and
//! `titles.root_folder_id` (0136 introduced the column; there is no foreign key,
//! only a non-empty trigger). Both are rewritten here. `library_root_id_remaps`
//! carries the audit trail for anything holding an id outside the database.
//!
//! ## Determinism
//!
//! The synthetic id is derived once, from the *legacy id* rather than from the
//! path, so a SQLite catalog and a PostgreSQL catalog carrying the same rows
//! migrate to the same ids — which the engine-to-engine transfer tooling depends
//! on. The derivation runs exactly once, here; the runtime never recomputes an
//! id for an existing root, which is the whole point of the change.

use std::collections::HashSet;

use scryer_application::{AppError, AppResult};
use scryer_domain::{normalize_library_root_path, root_folder_id_for_normalized_path};
use sqlx::Row;

/// Domain separator so the derived value can never collide with a path hash.
const SYNTHETIC_ROOT_ID_DOMAIN: &str = "scryer:synthetic-root-id:v1:";
const SYNTHETIC_ROOT_ID_PREFIX: &str = "root_";
const SYNTHETIC_ROOT_ID_HEX_LEN: usize = 32;

#[derive(Clone, Debug)]
struct RootRow {
    id: String,
    path: String,
    normalized_path: String,
}

/// One root's before/after, plus the legacy id every caller may still hold.
#[derive(Clone, Debug)]
struct RootRemap {
    old_id: String,
    new_id: String,
    legacy_path_derived_id: String,
    normalized_path: String,
}

impl RootRemap {
    fn id_changed(&self) -> bool {
        self.old_id != self.new_id
    }
}

/// The synthetic id for a root that is being re-keyed off its path.
///
/// Derived from the legacy id so both engines agree, but the derivation is a
/// one-time seeding step: nothing recomputes this value afterwards.
pub fn synthetic_root_id_from_legacy_id(legacy_id: &str) -> String {
    let digest = blake3::hash(format!("{SYNTHETIC_ROOT_ID_DOMAIN}{legacy_id}").as_bytes());
    let hex = digest.to_hex();
    format!(
        "{SYNTHETIC_ROOT_ID_PREFIX}{}",
        &hex[..SYNTHETIC_ROOT_ID_HEX_LEN]
    )
}

pub async fn migrate_synthetic_root_ids_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let roots = sqlite_roots(tx).await?;
    let remaps = build_remaps(roots)?;

    for remap in &remaps {
        if remap.id_changed() {
            sqlx::query("UPDATE library_roots SET id = ?1 WHERE id = ?2")
                .bind(&remap.new_id)
                .bind(&remap.old_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
            sqlx::query("UPDATE titles SET root_folder_id = ?1 WHERE root_folder_id = ?2")
                .bind(&remap.new_id)
                .bind(&remap.old_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }

        sqlx::query("UPDATE library_roots SET legacy_path_derived_id = ?1 WHERE id = ?2")
            .bind(&remap.legacy_path_derived_id)
            .bind(&remap.new_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;

        sqlx::query(
            "INSERT INTO library_root_id_remaps
                 (legacy_root_id, root_id, normalized_path, remapped)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(legacy_root_id) DO UPDATE SET
                 root_id = excluded.root_id,
                 normalized_path = excluded.normalized_path,
                 remapped = excluded.remapped",
        )
        .bind(&remap.legacy_path_derived_id)
        .bind(&remap.new_id)
        .bind(&remap.normalized_path)
        .bind(i64::from(remap.id_changed()))
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }

    sqlite_assert_no_path_derived_root_ids(tx).await
}

pub async fn migrate_synthetic_root_ids_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let roots = postgres_roots(tx).await?;
    let remaps = build_remaps(roots)?;

    for remap in &remaps {
        if remap.id_changed() {
            sqlx::query("UPDATE library_roots SET id = $1 WHERE id = $2")
                .bind(&remap.new_id)
                .bind(&remap.old_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
            sqlx::query("UPDATE titles SET root_folder_id = $1 WHERE root_folder_id = $2")
                .bind(&remap.new_id)
                .bind(&remap.old_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_err)?;
        }

        sqlx::query("UPDATE library_roots SET legacy_path_derived_id = $1 WHERE id = $2")
            .bind(&remap.legacy_path_derived_id)
            .bind(&remap.new_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;

        sqlx::query(
            "INSERT INTO library_root_id_remaps
                 (legacy_root_id, root_id, normalized_path, remapped)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (legacy_root_id) DO UPDATE SET
                 root_id = excluded.root_id,
                 normalized_path = excluded.normalized_path,
                 remapped = excluded.remapped",
        )
        .bind(&remap.legacy_path_derived_id)
        .bind(&remap.new_id)
        .bind(&remap.normalized_path)
        .bind(remap.id_changed())
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }

    postgres_assert_no_path_derived_root_ids(tx).await
}

/// Engine-independent plan. Kept separate so both engines provably agree.
fn build_remaps(roots: Vec<RootRow>) -> AppResult<Vec<RootRemap>> {
    let existing_ids = roots
        .iter()
        .map(|root| root.id.clone())
        .collect::<HashSet<_>>();
    let mut claimed_ids = HashSet::<String>::new();
    let mut seen_legacy_ids = HashSet::<String>::new();
    let mut remaps = Vec::with_capacity(roots.len());

    for root in roots {
        let normalized_path = effective_normalized_path(&root);
        let legacy_path_derived_id = root_folder_id_for_normalized_path(&normalized_path);
        // Pre-0136 rows can carry a normalized_path that was written on another
        // platform; accept either spelling as "path-derived" before deciding.
        let alternate_path_derived_id =
            root_folder_id_for_normalized_path(&normalize_library_root_path(&root.path));
        let is_path_derived =
            root.id == legacy_path_derived_id || root.id == alternate_path_derived_id;

        let new_id = if is_path_derived {
            synthetic_root_id_from_legacy_id(&root.id)
        } else {
            root.id.clone()
        };

        if new_id != root.id && existing_ids.contains(&new_id) {
            return Err(AppError::Repository(format!(
                "synthetic root id {new_id} for root {} collides with an existing root id",
                root.id
            )));
        }
        if !claimed_ids.insert(new_id.clone()) {
            return Err(AppError::Repository(format!(
                "synthetic root id {new_id} was claimed twice while remapping root {}",
                root.id
            )));
        }
        // `library_roots.normalized_path` is uniquely indexed, so two roots
        // sharing a legacy id means the catalog is already inconsistent.
        if !seen_legacy_ids.insert(legacy_path_derived_id.clone()) {
            return Err(AppError::Repository(format!(
                "two library roots share the path-derived id {legacy_path_derived_id}"
            )));
        }

        remaps.push(RootRemap {
            old_id: root.id,
            new_id,
            legacy_path_derived_id,
            normalized_path,
        });
    }

    Ok(remaps)
}

fn effective_normalized_path(root: &RootRow) -> String {
    if root.normalized_path.trim().is_empty() {
        normalize_library_root_path(&root.path)
    } else {
        root.normalized_path.clone()
    }
}

async fn sqlite_roots(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<Vec<RootRow>> {
    let rows = sqlx::query("SELECT id, path, normalized_path FROM library_roots ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(RootRow {
                id: row.try_get("id").map_err(repo_err)?,
                path: row.try_get("path").map_err(repo_err)?,
                normalized_path: row.try_get("normalized_path").map_err(repo_err)?,
            })
        })
        .collect()
}

async fn postgres_roots(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<Vec<RootRow>> {
    let rows = sqlx::query("SELECT id, path, normalized_path FROM library_roots ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(RootRow {
                id: row.try_get("id").map_err(repo_err)?,
                path: row.try_get("path").map_err(repo_err)?,
                normalized_path: row.try_get("normalized_path").map_err(repo_err)?,
            })
        })
        .collect()
}

/// The invariant 0210 exists to establish: no root's id is a function of its path.
async fn sqlite_assert_no_path_derived_root_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let roots = sqlite_roots(tx).await?;
    assert_no_path_derived_root_ids(&roots)
}

async fn postgres_assert_no_path_derived_root_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let roots = postgres_roots(tx).await?;
    assert_no_path_derived_root_ids(&roots)
}

fn assert_no_path_derived_root_ids(roots: &[RootRow]) -> AppResult<()> {
    for root in roots {
        let normalized_path = effective_normalized_path(root);
        if root.id == root_folder_id_for_normalized_path(&normalized_path) {
            return Err(AppError::Repository(format!(
                "library root {} still carries a path-derived id after migration 0213",
                root.id
            )));
        }
    }
    Ok(())
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(id: &str, path: &str) -> RootRow {
        RootRow {
            id: id.to_string(),
            path: path.to_string(),
            normalized_path: normalize_library_root_path(path),
        }
    }

    #[test]
    fn path_derived_roots_are_rekeyed_and_others_are_left_alone() {
        let path_derived_id =
            root_folder_id_for_normalized_path(&normalize_library_root_path("/data/movies"));
        let remaps = build_remaps(vec![
            root(&path_derived_id, "/data/movies"),
            root("canonical_root_for_series_default_library", "/data/series"),
        ])
        .expect("plan should build");

        let rekeyed = remaps
            .iter()
            .find(|remap| remap.old_id == path_derived_id)
            .expect("path-derived root should be planned");
        assert!(rekeyed.id_changed());
        assert!(rekeyed.new_id.starts_with(SYNTHETIC_ROOT_ID_PREFIX));
        assert_eq!(rekeyed.legacy_path_derived_id, path_derived_id);

        let preserved = remaps
            .iter()
            .find(|remap| remap.old_id == "canonical_root_for_series_default_library")
            .expect("seeded root should be planned");
        assert!(!preserved.id_changed());
        // The alias is still recorded, so a caller holding the path-derived id
        // can resolve the seeded root.
        assert_ne!(preserved.legacy_path_derived_id, preserved.new_id);
    }

    #[test]
    fn synthetic_ids_are_stable_and_domain_separated() {
        let legacy = root_folder_id_for_normalized_path("/data/movies");
        assert_eq!(
            synthetic_root_id_from_legacy_id(&legacy),
            synthetic_root_id_from_legacy_id(&legacy)
        );
        assert_ne!(synthetic_root_id_from_legacy_id(&legacy), legacy);
        assert_ne!(
            synthetic_root_id_from_legacy_id(&legacy),
            synthetic_root_id_from_legacy_id("some-other-root")
        );
    }

    #[test]
    fn duplicate_path_derived_ids_fail_closed() {
        let error = build_remaps(vec![
            root("root-a", "/data/movies"),
            root("root-b", "/data/movies"),
        ])
        .expect_err("two roots on one path must not migrate silently");
        assert!(matches!(error, AppError::Repository(_)));
    }
}
