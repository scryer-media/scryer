use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult, LibraryRepository, LibraryRootDraft};
use scryer_domain::{
    AppPermissionMask, Library, LibraryGrant, LibraryPermissionMask, LibraryRoot, MediaFacet,
    allocate_root_folder_id, default_library_id_for_facet, normalize_library_root_path,
};
use std::collections::{HashMap, HashSet};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

const LIBRARY_ROOT_COLUMNS: &str = "libraries.id, libraries.facet, libraries.name, libraries.slug,
    libraries.is_default, libraries.created_at, libraries.updated_at,
    library_roots.id AS root_id, library_roots.library_id,
    library_roots.path AS root_path, library_roots.is_default AS root_is_default,
    library_roots.created_at AS root_created_at, library_roots.updated_at AS root_updated_at";

#[derive(Clone)]
pub struct LibraryStore {
    datastore: StoreDatastore,
}

impl LibraryStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl LibraryRepository for LibraryStore {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        list_libraries(self.datastore.read_exec(), facet).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        get_library_by_id(self.datastore.read_exec(), id).await
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        let expected_id = default_library_id_for_facet(&facet);
        self.get_by_id(&expected_id).await
    }

    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        let library_id = library.id.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "create_library", move |tx| {
            let library = library.clone();
            let library_id = library_id.clone();
            let roots = roots.clone();
            Box::pin(async move {
                insert_library_tx(tx, &library, roots).await?;
                load_library_tx(tx, &library_id).await?.ok_or_else(|| {
                    AppError::Repository("created library was not found".to_string())
                })
            })
        })
        .await
    }

    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        let library_id = library_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_library", move |tx| {
            let library_id = library_id.clone();
            let name = name.clone();
            let slug = slug.clone();
            let roots = roots.clone();
            Box::pin(async move {
                update_library_tx(tx, &library_id, name, slug, roots).await?;
                load_library_tx(tx, &library_id).await?.ok_or_else(|| {
                    AppError::Repository("updated library was not found".to_string())
                })
            })
        })
        .await
    }

    async fn set_root_path(&self, root_id: &str, path: &str) -> AppResult<Library> {
        let root_id = root_id.to_string();
        let path = path.trim().to_string();
        if path.is_empty() {
            return Err(AppError::Validation(
                "a library root path cannot be empty".to_string(),
            ));
        }
        SqlRuntime::run_in_transaction(&self.datastore, "set_library_root_path", move |tx| {
            let root_id = root_id.clone();
            let path = path.clone();
            Box::pin(async move {
                let library_id = set_root_path_tx(tx, &root_id, path).await?;
                load_library_tx(tx, &library_id).await?.ok_or_else(|| {
                    AppError::Repository("library of the relocated root was not found".to_string())
                })
            })
        })
        .await
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        let library_id = library_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_library", move |tx| {
            let library_id = library_id.clone();
            Box::pin(async move {
                tx.execute(
                    "DELETE FROM libraries WHERE id = {}",
                    &[SqlArg::Text(library_id)],
                )
                .await
                .map(|rows| rows > 0)
            })
        })
        .await
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT permission_mask FROM user_app_permission_masks WHERE user_id = {}",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        let mask = row
            .as_ref()
            .map(|row| row.opt_i64("permission_mask"))
            .transpose()?
            .flatten()
            .unwrap_or(0);
        Ok(AppPermissionMask::from_bits_retain(mask_from_db_value(
            mask,
        )))
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "set_app_permission_mask_for_user",
            move |tx| {
                let user_id = user_id.clone();
                Box::pin(async move {
                    tx.execute(
                    "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
                     VALUES ({}, {}, {})
                     ON CONFLICT(user_id) DO UPDATE SET
                        permission_mask = excluded.permission_mask,
                        updated_at = excluded.updated_at",
                    &[
                        SqlArg::Text(user_id),
                        SqlArg::I64(mask_to_db_value(permissions.bits())),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT user_id, library_id, permission_mask
             FROM user_library_permission_masks
             WHERE user_id = {}
             ORDER BY library_id ASC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_library_grant).collect()
    }

    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        let user_id = user_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_library_grants_for_user", move |tx| {
            let user_id = user_id.clone();
            let grants = grants.clone();
            Box::pin(async move { replace_library_grants_tx(tx, &user_id, grants).await })
        })
        .await
    }

    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT library_id FROM titles WHERE id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;
        row.as_ref()
            .map(|row| row.opt_text("library_id"))
            .transpose()
            .map(Option::flatten)
    }
}

async fn list_libraries(
    exec: SqlExec<'_, '_>,
    facet: Option<MediaFacet>,
) -> AppResult<Vec<Library>> {
    let mut sql = format!(
        "SELECT {LIBRARY_ROOT_COLUMNS}
         FROM libraries
         LEFT JOIN library_roots ON library_roots.library_id = libraries.id"
    );
    let args = if let Some(facet) = facet {
        sql.push_str(" WHERE libraries.facet = {}");
        vec![SqlArg::Text(facet.as_str().to_string())]
    } else {
        Vec::new()
    };
    sql.push_str(
        " ORDER BY libraries.facet ASC, libraries.is_default DESC, LOWER(libraries.name) ASC,
                 libraries.id ASC, library_roots.is_default DESC, library_roots.path ASC",
    );
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows_to_libraries(&rows)
}

async fn get_library_by_id(exec: SqlExec<'_, '_>, id: &str) -> AppResult<Option<Library>> {
    let sql = format!(
        "SELECT {LIBRARY_ROOT_COLUMNS}
         FROM libraries
         LEFT JOIN library_roots ON library_roots.library_id = libraries.id
         WHERE libraries.id = {{}}
         ORDER BY library_roots.is_default DESC, library_roots.path ASC"
    );
    let rows = SqlRuntime::fetch_all(exec, &sql, &[SqlArg::Text(id.to_string())]).await?;
    Ok(rows_to_libraries(&rows)?.into_iter().next())
}

async fn load_library_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<Option<Library>> {
    get_library_by_id(SqlExec::Tx(tx), id).await
}

async fn insert_library_tx(
    tx: &mut SqlTx<'_>,
    library: &Library,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(library.id.clone()),
            SqlArg::Text(library.facet.as_str().to_string()),
            SqlArg::Text(library.name.clone()),
            SqlArg::Text(library.slug.clone()),
            SqlArg::Bool(library.is_default),
            SqlArg::Timestamp(library.created_at),
            SqlArg::Timestamp(library.updated_at),
        ],
    )
    .await?;

    // A brand new library has no roots to inherit identity from.
    insert_library_roots_tx(tx, &library.id, roots, &HashMap::new()).await
}

async fn update_library_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
    name: String,
    slug: String,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<()> {
    let rows = tx
        .execute(
            "UPDATE libraries SET name = {}, slug = {}, updated_at = {} WHERE id = {}",
            &[
                SqlArg::Text(name),
                SqlArg::Text(slug),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(library_id.to_string()),
            ],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("library {library_id}")));
    }

    // The stored ids have to be read before the rows are deleted: a root's
    // identity is permanent, so reinserting it must land it back on the id it
    // already had rather than deriving a new one from its path (FR-078).
    let existing_root_ids = existing_root_ids_by_normalized_path_tx(tx, library_id).await?;

    reject_referenced_root_removals_tx(tx, library_id, &existing_root_ids, &roots).await?;
    tx.execute(
        "DELETE FROM library_roots WHERE library_id = {}",
        &[SqlArg::Text(library_id.to_string())],
    )
    .await?;
    insert_library_roots_tx(tx, library_id, roots, &existing_root_ids).await
}

/// Point one existing root row at a new path, in place (FR-021, FR-078).
///
/// The row is never deleted and reinserted, so the id, the library, the default
/// flag, and every `titles.root_folder_id` reference survive untouched — the
/// whole point of a root change is that the identity does not move with the
/// path. Returns the library the root belongs to.
async fn set_root_path_tx(tx: &mut SqlTx<'_>, root_id: &str, path: String) -> AppResult<String> {
    let library_id = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT library_id FROM library_roots WHERE id = {}",
        &[SqlArg::Text(root_id.to_string())],
    )
    .await?
    .map(|row| row.text("library_id"))
    .transpose()?
    .ok_or_else(|| AppError::NotFound(format!("library root {root_id}")))?;

    let normalized_path = normalize_library_root_path(&path);
    // `library_roots.normalized_path` is uniquely indexed, so a destination that
    // is already somebody's root would fail here with a constraint violation the
    // user cannot read. The caller refuses that case by name (it is
    // consolidation, US5); this is the last line of defence, phrased.
    let conflicting = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id FROM library_roots WHERE normalized_path = {} AND id <> {}",
        &[
            SqlArg::Text(normalized_path.clone()),
            SqlArg::Text(root_id.to_string()),
        ],
    )
    .await?
    .map(|row| row.text("id"))
    .transpose()?;
    if let Some(conflicting) = conflicting {
        return Err(AppError::Validation(format!(
            "library root '{path}' is already configured as root {conflicting}"
        )));
    }

    tx.execute(
        "UPDATE library_roots SET path = {}, normalized_path = {}, updated_at = {} WHERE id = {}",
        &[
            SqlArg::Text(path),
            SqlArg::Text(normalized_path),
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(root_id.to_string()),
        ],
    )
    .await?;

    Ok(library_id)
}

/// A library's stored root ids, keyed by the normalized path each one points at.
///
/// Root ids are read back, never recomputed. This is the lookup that replaces
/// the pre-0204 `root_folder_id_for_path` derivation.
async fn existing_root_ids_by_normalized_path_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
) -> AppResult<HashMap<String, String>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT id, normalized_path FROM library_roots WHERE library_id = {}",
        &[SqlArg::Text(library_id.to_string())],
    )
    .await?;
    rows.into_iter()
        .map(|row| Ok((row.text("normalized_path")?, row.text("id")?)))
        .collect()
}

async fn reject_referenced_root_removals_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
    existing_root_ids: &HashMap<String, String>,
    roots: &[LibraryRootDraft],
) -> AppResult<()> {
    let desired_ids = roots
        .iter()
        .filter_map(|root| {
            existing_root_ids
                .get(&normalize_library_root_path(&root.path))
                .cloned()
        })
        .collect::<HashSet<_>>();
    let mut removed_root_ids = existing_root_ids
        .values()
        .filter(|id| !desired_ids.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    removed_root_ids.sort();

    for root_id in removed_root_ids {
        let referenced_count = SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT COUNT(*) AS referenced_count
               FROM titles
              WHERE root_folder_id = {}
                AND COALESCE(
                        library_id,
                        CASE facet
                            WHEN 'movie' THEN 'movie_default_library'
                            WHEN 'series' THEN 'series_default_library'
                            WHEN 'anime' THEN 'anime_default_library'
                            ELSE 'movie_default_library'
                        END
                    ) = {}",
            &[SqlArg::Text(root_id), SqlArg::Text(library_id.to_string())],
        )
        .await?
        .map(|row| row.i64("referenced_count"))
        .transpose()?
        .unwrap_or(0);
        if referenced_count > 0 {
            return Err(AppError::Validation(
                "library root cannot be removed while titles reference it".into(),
            ));
        }
    }
    Ok(())
}

async fn insert_library_roots_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
    roots: Vec<LibraryRootDraft>,
    existing_root_ids: &HashMap<String, String>,
) -> AppResult<()> {
    let now = Utc::now();
    for root in roots {
        let path = root.path.trim();
        if path.is_empty() {
            continue;
        }
        let normalized_path = normalize_library_root_path(path);
        // An existing root keeps the identity it already has; only a genuinely
        // new root is allocated one.
        let root_id = existing_root_ids
            .get(&normalized_path)
            .cloned()
            .unwrap_or_else(allocate_root_folder_id);
        tx.execute(
            "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(root_id),
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(path.to_string()),
                SqlArg::Text(normalized_path),
                SqlArg::Bool(root.is_default),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn replace_library_grants_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    grants: Vec<LibraryGrant>,
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM user_library_permission_masks WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    let now = Utc::now();
    for grant in grants {
        if grant.permissions.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO user_library_permission_masks
             (user_id, library_id, permission_mask, updated_at)
             VALUES ({}, {}, {}, {})
             ON CONFLICT(user_id, library_id) DO UPDATE SET
                permission_mask = excluded.permission_mask,
                updated_at = excluded.updated_at",
            &[
                SqlArg::Text(user_id.to_string()),
                SqlArg::Text(grant.library_id),
                SqlArg::I64(mask_to_db_value(grant.permissions.bits())),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

fn rows_to_libraries(rows: &[SqlRow]) -> AppResult<Vec<Library>> {
    let mut libraries = Vec::<Library>::new();
    for row in rows {
        let library_id = row.text("id")?;
        if libraries
            .last()
            .is_none_or(|library| library.id != library_id)
        {
            libraries.push(row_to_library(row, library_id.clone())?);
        }
        if row.opt_text("root_id")?.is_some()
            && let Some(library) = libraries.last_mut()
        {
            library.roots.push(row_to_root(row)?);
        }
    }
    Ok(libraries)
}

fn row_to_library(row: &SqlRow, id: String) -> AppResult<Library> {
    let facet = MediaFacet::parse(&row.text("facet")?).unwrap_or_default();
    Ok(Library {
        id,
        facet,
        name: row.text("name")?,
        slug: row.text("slug")?,
        is_default: row.bool("is_default")?,
        roots: Vec::new(),
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn row_to_root(row: &SqlRow) -> AppResult<LibraryRoot> {
    Ok(LibraryRoot {
        id: row.text("root_id")?,
        library_id: row.text("library_id")?,
        path: row.text("root_path")?,
        is_default: row.bool("root_is_default")?,
        created_at: row.timestamp("root_created_at")?,
        updated_at: row.timestamp("root_updated_at")?,
    })
}

fn row_to_library_grant(row: &SqlRow) -> AppResult<LibraryGrant> {
    Ok(LibraryGrant {
        user_id: row.text("user_id")?,
        library_id: row.text("library_id")?,
        permissions: LibraryPermissionMask::from_bits_retain(mask_from_db_value(
            row.i64("permission_mask")?,
        )),
    })
}

fn mask_to_db_value(mask: u64) -> i64 {
    mask as i64
}

fn mask_from_db_value(mask: i64) -> u64 {
    mask as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_store() -> LibraryStore {
        scryer_infrastructure_datastore::register_spellfix_auto_extension()
            .expect("spellfix extension should register before migrations");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
            &pool, None, true,
        )
        .await
        .expect("fresh migrations should apply");
        LibraryStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn draft(path: &str, is_default: bool) -> LibraryRootDraft {
        LibraryRootDraft {
            path: path.to_string(),
            is_default,
        }
    }

    #[tokio::test]
    async fn a_new_root_is_allocated_an_opaque_id_rather_than_one_derived_from_its_path() {
        let store = test_store().await;
        let now = Utc::now();
        let library = store
            .create(
                Library {
                    id: "allocated-root-library".to_string(),
                    facet: MediaFacet::Movie,
                    name: "Allocated".to_string(),
                    slug: "allocated".to_string(),
                    is_default: false,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
                vec![draft("/mnt/allocated", true)],
            )
            .await
            .expect("library should create");

        let root = library.roots.first().expect("library should have a root");
        assert!(
            root.id.starts_with(scryer_domain::SYNTHETIC_ROOT_ID_PREFIX),
            "root id should be allocated, got {}",
            root.id
        );
        assert_ne!(
            root.id,
            scryer_domain::root_folder_id_for_path("/mnt/allocated"),
            "root identity must not be a function of the root path"
        );
    }

    #[tokio::test]
    async fn updating_a_library_keeps_the_identity_of_the_roots_it_still_has() {
        let store = test_store().await;
        let now = Utc::now();
        let created = store
            .create(
                Library {
                    id: "stable-root-library".to_string(),
                    facet: MediaFacet::Movie,
                    name: "Stable".to_string(),
                    slug: "stable".to_string(),
                    is_default: false,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
                vec![draft("/mnt/stable", true)],
            )
            .await
            .expect("library should create");
        let original_root_id = created
            .roots
            .first()
            .expect("library should have a root")
            .id
            .clone();

        // The update path deletes and reinserts every root row, so this is where
        // a derived id would silently mint a new identity.
        let updated = store
            .update(
                "stable-root-library",
                "Stable Renamed".to_string(),
                "stable-renamed".to_string(),
                vec![
                    draft("/mnt/stable", true),
                    draft("/mnt/stable-extra", false),
                ],
            )
            .await
            .expect("library should update");

        let kept = updated
            .roots
            .iter()
            .find(|root| root.path == "/mnt/stable")
            .expect("the original root should survive the update");
        assert_eq!(kept.id, original_root_id);

        let added = updated
            .roots
            .iter()
            .find(|root| root.path == "/mnt/stable-extra")
            .expect("the added root should exist");
        assert_ne!(added.id, original_root_id);
        assert!(
            added
                .id
                .starts_with(scryer_domain::SYNTHETIC_ROOT_ID_PREFIX),
            "a newly added root should be allocated an id, got {}",
            added.id
        );
    }

    /// US4.2, FR-021/FR-078: a root change writes the new path onto the row the
    /// root already has. Everything that identifies it survives.
    #[tokio::test]
    async fn setting_a_root_path_keeps_the_root_id_and_its_default_status() {
        let store = test_store().await;
        let now = Utc::now();
        let created = store
            .create(
                Library {
                    id: "relocating-library".to_string(),
                    facet: MediaFacet::Movie,
                    name: "Relocating".to_string(),
                    slug: "relocating".to_string(),
                    is_default: false,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
                vec![draft("/mnt/old-disk", true), draft("/mnt/other", false)],
            )
            .await
            .expect("library should create");
        let root_id = created
            .roots
            .iter()
            .find(|root| root.path == "/mnt/old-disk")
            .expect("the relocating root exists")
            .id
            .clone();

        let updated = store
            .set_root_path(&root_id, "/mnt/new-disk")
            .await
            .expect("the root path should flip");

        let relocated = updated
            .roots
            .iter()
            .find(|root| root.id == root_id)
            .expect("the root keeps its identity across a path change");
        assert_eq!(relocated.path, "/mnt/new-disk");
        assert!(
            relocated.is_default,
            "a path change never moves the library default (FR-021)"
        );
        assert!(
            !updated
                .roots
                .iter()
                .any(|root| root.path == "/mnt/old-disk"),
            "the old path is gone rather than left beside the new one"
        );
        assert_eq!(
            updated.roots.len(),
            2,
            "the library's other root is untouched"
        );

        // The normalized column travels with the path, or the next lookup by
        // path would still answer with the retired location.
        let reread = store
            .get_by_id("relocating-library")
            .await
            .expect("read back")
            .expect("library exists");
        assert_eq!(
            reread
                .roots
                .iter()
                .find(|root| root.id == root_id)
                .expect("root still there")
                .path,
            "/mnt/new-disk"
        );
    }

    /// FR-020: a destination that is already a configured root is
    /// *consolidation* (US5), not a root change. The store refuses it in words
    /// rather than as a unique-index violation.
    #[tokio::test]
    async fn setting_a_root_path_refuses_a_path_another_root_already_holds() {
        let store = test_store().await;
        let now = Utc::now();
        let created = store
            .create(
                Library {
                    id: "colliding-library".to_string(),
                    facet: MediaFacet::Movie,
                    name: "Colliding".to_string(),
                    slug: "colliding".to_string(),
                    is_default: false,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                },
                vec![draft("/mnt/left", true), draft("/mnt/right", false)],
            )
            .await
            .expect("library should create");
        let left = created
            .roots
            .iter()
            .find(|root| root.path == "/mnt/left")
            .expect("left root")
            .id
            .clone();

        let error = store
            .set_root_path(&left, "/mnt/right")
            .await
            .expect_err("a configured root is not a root-change destination");
        assert!(
            matches!(error, AppError::Validation(_)),
            "expected a validation refusal, got {error:?}"
        );

        // An unknown root is a not-found, not a silently ignored write.
        let missing = store
            .set_root_path("no-such-root", "/mnt/anywhere")
            .await
            .expect_err("an unknown root cannot be relocated");
        assert!(
            matches!(missing, AppError::NotFound(_)),
            "expected a not-found refusal, got {missing:?}"
        );
    }
}
