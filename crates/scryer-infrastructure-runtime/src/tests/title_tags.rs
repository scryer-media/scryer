//! Title tag registry against the real store, on both engines.
//!
//! The JSON-bag idioms (`json_each` versus `jsonb_array_elements_text`) are the
//! only thing that differs between SQLite and PostgreSQL here, and they are
//! exactly the part that a unit test over SQL strings cannot prove. So the
//! assertions live in one function and both engines run it.

use super::*;
use crate::queries::sql_runtime::StoreDatastore;
use scryer_application::{TitleCatalogFilter, TitleCatalogSort};

fn tag_definition(id: &str, label: &str) -> scryer_domain::TitleTagDefinition {
    let now = Utc::now();
    scryer_domain::TitleTagDefinition {
        id: id.to_string(),
        label: label.to_string(),
        description: None,
        created_by: Some("operator-one".to_string()),
        created_at: now,
        updated_at: now,
    }
}

async fn stored_tags(catalog: &TitleStore, title_id: &str) -> Vec<String> {
    TitleRepository::get_by_id(catalog, title_id)
        .await
        .expect("title should load")
        .expect("title should exist")
        .tags
}

async fn assert_title_tag_registry_behaviour(catalog: &TitleStore) -> AppResult<()> {
    let created =
        TitleRepository::create_title_tag_definition(catalog, &tag_definition("tag-keep", "keep"))
            .await?;
    assert_eq!(created.label, "keep");
    assert_eq!(created.created_by.as_deref(), Some("operator-one"));
    TitleRepository::create_title_tag_definition(
        catalog,
        &tag_definition("tag-review", "needs review"),
    )
    .await?;

    // The label is the join key against the bag, so a second row claiming it is
    // a validation error the caller can show, not a raw constraint violation.
    let duplicate =
        TitleRepository::create_title_tag_definition(catalog, &tag_definition("tag-other", "keep"))
            .await;
    assert!(
        matches!(duplicate, Err(AppError::Validation(ref message)) if message.contains("keep")),
        "{duplicate:?}"
    );

    let mut first = make_test_title("tagstore-one", None);
    first.tags = vec!["scryer:monitor-type:all".to_string()];
    TitleRepository::create(catalog, first.clone()).await?;
    let second = make_test_title("tagstore-two", None);
    TitleRepository::create(catalog, second.clone()).await?;
    let untagged = make_test_title("tagstore-three", None);
    TitleRepository::create(catalog, untagged.clone()).await?;

    // The patch is a read-modify-write inside the transaction: the structured
    // entry that was already on the title survives it.
    let patched = TitleRepository::update_user_tags(
        catalog,
        &first.id,
        &["keep".to_string(), "needs review".to_string()],
        &[],
    )
    .await?;
    assert_eq!(
        patched.tags,
        vec![
            "scryer:monitor-type:all".to_string(),
            "keep".to_string(),
            "needs review".to_string(),
        ]
    );
    TitleRepository::update_user_tags(catalog, &second.id, &["keep".to_string()], &[]).await?;

    // Removals run first, so a label named in both lists ends up present.
    let both_ways = TitleRepository::update_user_tags(
        catalog,
        &first.id,
        &["needs review".to_string()],
        &["needs review".to_string()],
    )
    .await?;
    assert!(both_ways.tags.iter().any(|tag| tag == "needs review"));

    let registry = TitleRepository::list_title_tag_definitions(catalog).await?;
    assert_eq!(registry.len(), 2);
    assert_eq!(registry[0].definition.label, "keep");
    assert_eq!(registry[0].title_count, 2, "counted across the JSON bag");
    assert_eq!(registry[1].definition.label, "needs review");
    assert_eq!(registry[1].title_count, 1);

    // Any-of filtering reads the bag directly, per dialect.
    let filtered = TitleRepository::list_for_libraries_catalog(
        catalog,
        None,
        &[first.library_id.clone()],
        None,
        TitleCatalogFilter {
            user_tags: vec!["needs review".to_string()],
            ..TitleCatalogFilter::default()
        },
        TitleCatalogSort::default(),
        50,
        0,
        false,
        true,
    )
    .await?;
    assert_eq!(filtered.total_count, 1);
    assert_eq!(filtered.items[0].id, first.id);

    let two_labels = TitleRepository::list_for_libraries_catalog(
        catalog,
        None,
        &[first.library_id.clone()],
        None,
        TitleCatalogFilter {
            user_tags: vec!["keep".to_string(), "needs review".to_string()],
            ..TitleCatalogFilter::default()
        },
        TitleCatalogSort::default(),
        50,
        0,
        false,
        true,
    )
    .await?;
    assert_eq!(
        two_labels.total_count, 2,
        "any-of, so a title matching either label is included exactly once"
    );

    // Rename rewrites every bag carrying the label, in the same transaction as
    // the registry row.
    let (renamed, rewritten) = TitleRepository::update_title_tag_definition(
        catalog,
        &created.id,
        Some("archive".to_string()),
        Some(Some("kept for later".to_string())),
        Utc::now(),
    )
    .await?;
    assert_eq!(renamed.label, "archive");
    assert_eq!(renamed.description.as_deref(), Some("kept for later"));
    assert_eq!(rewritten, 2);
    assert_eq!(
        stored_tags(catalog, &first.id).await,
        vec![
            "scryer:monitor-type:all".to_string(),
            "needs review".to_string(),
            "archive".to_string(),
        ]
    );
    assert_eq!(
        stored_tags(catalog, &second.id).await,
        vec!["archive".to_string()]
    );
    assert!(stored_tags(catalog, &untagged.id).await.is_empty());

    // A description-only edit touches no title at all.
    let (_, untouched) = TitleRepository::update_title_tag_definition(
        catalog,
        &created.id,
        None,
        Some(None),
        Utc::now(),
    )
    .await?;
    assert_eq!(untouched, 0);

    // Delete strips the label from every title in the same transaction and
    // leaves the reserved entry exactly where it was.
    let (deleted, stripped) =
        TitleRepository::delete_title_tag_definition(catalog, &created.id).await?;
    assert_eq!(deleted.label, "archive");
    assert_eq!(stripped, 2);
    assert_eq!(
        stored_tags(catalog, &first.id).await,
        vec![
            "scryer:monitor-type:all".to_string(),
            "needs review".to_string()
        ]
    );
    assert!(stored_tags(catalog, &second.id).await.is_empty());
    assert_eq!(
        TitleRepository::list_title_tag_definitions(catalog)
            .await?
            .len(),
        1
    );

    // A missing row is a not-found, not a silent success.
    assert!(matches!(
        TitleRepository::delete_title_tag_definition(catalog, "tag-absent").await,
        Err(AppError::NotFound(_))
    ));

    Ok(())
}

#[tokio::test]
async fn title_tag_registry_reads_writes_and_rewrites_on_sqlite() {
    let (services, db) = temp_services("scryer_title_tag_registry").await;
    let catalog = title_store(&services);

    assert_title_tag_registry_behaviour(&catalog)
        .await
        .expect("the registry should behave consistently on sqlite");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_tag_registry_reads_writes_and_rewrites_on_postgres() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        eprintln!(
            "skipping PostgreSQL title tag registry test; SCRYER_TEST_POSTGRES_URL is not set"
        );
        return Ok(());
    };

    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_test_{}_{}",
        std::process::id(),
        Id::new().0.replace('-', "_")
    );

    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        let catalog = TitleStore::new(services.datastore());
        let result = assert_title_tag_registry_behaviour(&catalog).await;
        services.pool().close().await;
        result
    }
    .await;

    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    admin_pool.close().await;
    cleanup.map_err(|error| AppError::Repository(format!("failed to drop schema: {error}")))?;
    result
}

/// Guard against the `StoreDatastore` variants drifting apart: the registry
/// list and the catalog filter both branch on the dialect, and a new variant
/// must be given both idioms rather than silently falling into one of them.
#[tokio::test]
async fn the_registry_uses_the_bag_idiom_of_the_datastore_it_was_built_on() {
    let (services, db) = temp_services("scryer_title_tag_dialect").await;
    assert!(matches!(
        services.datastore(),
        StoreDatastore::Sqlite { .. }
    ));
    let _ = std::fs::remove_file(db);
}
