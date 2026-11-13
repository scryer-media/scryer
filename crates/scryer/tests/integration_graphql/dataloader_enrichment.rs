//! Slice-1 verification for the GraphQL dataloader / dual-mode refactor.
//!
//! Enrichment fields on `TitlePayload` (sizeBytes, episode progress, library
//! name/slug, quality tier, movie media descriptors) used to be populated only
//! by the four catalog queries and returned `null` on every relationship path
//! ("dual resolution mode"). They are now loader-backed `ComplexObject`
//! resolvers that resolve identically on every path: batched through the
//! request-scoped `RequestLoaders` when present, and via direct application
//! calls when they are absent (the WebSocket/subscription shape).
//!
//! These tests pin three properties:
//!   1. enrichment is equal + non-null whether a title is reached via the
//!      catalog query or via a relationship path (the silent-null regression),
//!   2. relationship enrichment still resolves when no `RequestLoaders` are in
//!      context (the fallback path), and
//!   3. resolving N titles' `sizeBytes` in one query with loaders issues exactly
//!      one batched repository call, versus one per title on the fallback path.

use super::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Enrichment field selection shared by the catalog and relationship queries so
/// the two payloads are directly comparable.
const ENRICH_FIELDS: &str = r#"
    id
    sizeBytes
    episodesOwned
    episodesMonitored
    episodesTotal
    libraryName
    librarySlug
    qualityTier
    currentQualityTier
    mediaResolution
    mediaHdr
    mediaAudioCodec
"#;

struct EnrichedTitleSeed {
    title_id: String,
    owned: i64,
    monitored: i64,
    total: i64,
}

/// Seed one series title with a season, episodes, and episode-linked media
/// files so `sizeBytes` and the episode-progress trio all resolve non-null.
/// File paths are synthetic: sizes come from the stored row, not from disk.
async fn seed_enriched_series_title(ctx: &TestContext, name: &str) -> EnrichedTitleSeed {
    let title = create_catalog_title(ctx, name, MediaFacet::Series, vec![], vec![], true).await;

    let season = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let total = 3usize;
    let owned = 2usize;
    for index in 1..=total {
        let episode = create_series_scan_episode(
            ctx,
            &title,
            &season,
            "1",
            &index.to_string(),
            &format!("S01E0{index}"),
        )
        .await;
        // Episode progress counts only aired episodes with a concrete air date;
        // create_series_scan_episode leaves air_date unset.
        ctx.shows
            .update_episode(
                &episode.id,
                EpisodeUpdate {
                    air_date: Some(Some(format!("2024-01-0{index}"))),
                    ..Default::default()
                },
            )
            .await
            .expect("set episode air date");
        if index <= owned {
            let file_path = format!("/enrichment-fixtures/{name}/S01E0{index}.mkv");
            let file_id = ctx
                .media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path,
                    size_bytes: 4_096 + index as i64,
                    ..Default::default()
                })
                .await
                .expect("insert media file");
            ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
                .await
                .expect("link media file to episode");
        }
    }

    EnrichedTitleSeed {
        title_id: title.id,
        owned: owned as i64,
        monitored: total as i64,
        total: total as i64,
    }
}

/// Resolve a fully-authorized default user to attach to schema-level requests.
async fn authorized_default_user(ctx: &TestContext) -> scryer_domain::User {
    let user = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default user");
    ctx.app
        .attach_user_authorization(user)
        .await
        .expect("attach user authorization")
}

/// Assert a GraphQL response carries no errors, tolerant of both the HTTP shape
/// (errors omitted) and the direct-schema shape (empty errors array).
fn assert_ok(body: &Value) {
    if let Some(errors) = body["errors"].as_array() {
        assert!(errors.is_empty(), "unexpected GraphQL errors: {body}");
    }
}

/// Query `title` selection from a `titles { items { ... } }` catalog response.
fn catalog_enrichment(body: &Value, title_id: &str) -> Value {
    assert_ok(body);
    body["data"]["titles"]["items"]
        .as_array()
        .expect("catalog titles items")
        .iter()
        .find(|item| item["id"] == title_id)
        .unwrap_or_else(|| panic!("seeded title {title_id} missing from catalog: {body}"))
        .clone()
}

/// Nested `title` selection reached through `title -> collections -> title`.
fn relationship_enrichment(body: &Value, title_id: &str) -> Value {
    assert_ok(body);
    body["data"]["title"]["collections"]
        .as_array()
        .expect("collections")
        .iter()
        .filter_map(|collection| collection.get("title"))
        .find(|nested| nested["id"] == title_id)
        .unwrap_or_else(|| panic!("nested title {title_id} missing via relationship: {body}"))
        .clone()
}

fn catalog_query() -> String {
    String::from("{ titles { items { ") + ENRICH_FIELDS + " } } }"
}

fn relationship_query(title_id: &str) -> String {
    format!(
        "{{ title(id: \"{}\") {{ collections {{ title {{ {} }} }} }} }}",
        title_id, ENRICH_FIELDS
    )
}

/// Execute a query directly against the schema with request-scoped loaders
/// injected, mirroring the production HTTP handler's loader wiring.
async fn exec_with_request_loaders(
    ctx: &TestContext,
    query: &str,
    user: scryer_domain::User,
) -> Value {
    let loaders = scryer_interface::RequestLoaders::new(ctx.app.clone(), user.clone());
    let request = async_graphql::Request::new(query).data(user).data(loaders);
    serde_json::to_value(&ctx.schema.execute(request).await).expect("serialize gql response")
}

/// Regression test for the dual-resolution silent-null bug: a title reached via
/// the catalog query and via a relationship path must enrich identically, and
/// the loader-backed path must match the fallback path field-for-field.
#[tokio::test]
async fn enrichment_matches_between_catalog_and_relationship_paths() {
    let ctx = TestContext::new().await;
    let seed = seed_enriched_series_title(&ctx, "Enrichment Parity Show").await;

    // Catalog path over HTTP: the test router injects no RequestLoaders, so this
    // exercises the direct-application fallback resolvers.
    let catalog_body = gql(&ctx, &catalog_query(), json!({})).await;
    let catalog = catalog_enrichment(&catalog_body, &seed.title_id);

    // Relationship path over HTTP (also fallback): title -> collections -> title.
    let rel_query = relationship_query(&seed.title_id);
    let rel_body = gql(&ctx, &rel_query, json!({})).await;
    let relationship = relationship_enrichment(&rel_body, &seed.title_id);

    assert_eq!(
        catalog, relationship,
        "catalog and relationship enrichment diverged (silent-null regression)"
    );

    // Core enrichment must be populated, not silently null.
    assert!(!catalog["sizeBytes"].is_null(), "sizeBytes null: {catalog}");
    assert_eq!(catalog["episodesOwned"].as_i64(), Some(seed.owned));
    assert_eq!(catalog["episodesMonitored"].as_i64(), Some(seed.monitored));
    assert_eq!(catalog["episodesTotal"].as_i64(), Some(seed.total));
    assert!(
        !catalog["libraryName"].is_null(),
        "libraryName null: {catalog}"
    );
    assert!(
        !catalog["librarySlug"].is_null(),
        "librarySlug null: {catalog}"
    );

    // The same relationship path resolved WITH request loaders must produce the
    // identical enrichment: loader and fallback code paths agree on every field.
    let user = authorized_default_user(&ctx).await;
    let rel_loaded_body = exec_with_request_loaders(&ctx, &rel_query, user).await;
    let relationship_loaded = relationship_enrichment(&rel_loaded_body, &seed.title_id);
    assert_eq!(
        catalog, relationship_loaded,
        "loader-backed relationship enrichment diverged from catalog fallback"
    );
}

/// A relationship field's nested enrichment must still resolve when there are no
/// `RequestLoaders` in context (the WebSocket/subscription execution shape).
#[tokio::test]
async fn relationship_enrichment_resolves_without_request_loaders() {
    let ctx = TestContext::new().await;
    let seed = seed_enriched_series_title(&ctx, "Loader Free Fallback Show").await;
    let user = authorized_default_user(&ctx).await;

    // `schema_exec` attaches ONLY the User to the request and never an
    // `Arc<RequestLoaders>`, forcing every enrichment resolver onto its direct
    // application-call fallback branch.
    let rel_query = relationship_query(&seed.title_id);
    let body = schema_exec(&ctx, &rel_query, Some(user)).await;
    let relationship = relationship_enrichment(&body, &seed.title_id);

    assert_eq!(relationship["id"].as_str(), Some(seed.title_id.as_str()));
    assert!(
        !relationship["sizeBytes"].is_null(),
        "sizeBytes null on loader-free fallback: {relationship}"
    );
    assert_eq!(relationship["episodesOwned"].as_i64(), Some(seed.owned));
    assert_eq!(relationship["episodesTotal"].as_i64(), Some(seed.total));
    assert!(
        !relationship["libraryName"].is_null(),
        "libraryName null on loader-free fallback: {relationship}"
    );
}

/// Resolving `sizeBytes` across N titles in a single query must coalesce into
/// exactly one media-size summary repository call when request loaders are
/// present, and fall back to one call per title when they are absent.
#[tokio::test]
async fn enrichment_size_summary_is_batched_across_titles_with_loaders() {
    let ctx = TestContext::new().await;

    // Seed N titles that each own one "matched" sized media file (the season
    // collection's ordered_path equals the file path), so sizeBytes is non-null.
    let title_count = 3usize;
    for index in 0..title_count {
        let title = create_catalog_title(
            &ctx,
            &format!("Batched Enrichment Title {index}"),
            MediaFacet::Series,
            vec![],
            vec![],
            true,
        )
        .await;
        seed_title_size_sort_fixture(
            &ctx,
            &title.id,
            &format!("batched-collection-{index}"),
            &format!("/batched-enrichment/{index}.mkv"),
            5_000 + index as i64,
        )
        .await;
    }

    // Swap in a counting media-file repository double while keeping every other
    // dependency; only the media-size summary port is instrumented.
    let size_summary_calls = Arc::new(AtomicUsize::new(0));
    let inner_media = ctx.media_files.clone();
    let calls = size_summary_calls.clone();
    let counting_app = ctx.app.with_test_overrides(move |builder| {
        builder.with_media_files(Arc::new(CountingMediaFileRepo {
            inner: inner_media,
            size_summary_calls: calls,
        }))
    });
    let schema = scryer_interface::build_schema(counting_app.clone(), ctx.auth_runtime.clone());
    let user = authorized_default_user(&ctx).await;
    let query = "{ titles { items { id sizeBytes } } }";

    // With request loaders, the per-title sizeBytes resolvers coalesce into one
    // batched media-size summary call.
    let loaders = scryer_interface::RequestLoaders::new(counting_app.clone(), user.clone());
    let batched = schema
        .execute(
            async_graphql::Request::new(query)
                .data(user.clone())
                .data(loaders),
        )
        .await;
    let batched = serde_json::to_value(&batched).expect("serialize batched response");
    assert_ok(&batched);
    let items = batched["data"]["titles"]["items"]
        .as_array()
        .expect("titles items")
        .clone();
    assert!(
        items.len() >= title_count,
        "expected at least {title_count} seeded titles: {batched}"
    );
    for item in &items {
        assert!(
            !item["sizeBytes"].is_null(),
            "sizeBytes null under request loaders: {item}"
        );
    }
    let resolved_titles = items.len();
    assert_eq!(
        size_summary_calls.load(Ordering::SeqCst),
        1,
        "resolving {resolved_titles} titles under request loaders must issue exactly one batched media-size summary call"
    );

    // Without loaders, the identical query falls back to one direct call per
    // title, proving the single call above is genuine batching, not a seeding
    // artifact.
    size_summary_calls.store(0, Ordering::SeqCst);
    let fallback = schema
        .execute(async_graphql::Request::new(query).data(user.clone()))
        .await;
    let fallback = serde_json::to_value(&fallback).expect("serialize fallback response");
    assert_ok(&fallback);
    assert_eq!(
        size_summary_calls.load(Ordering::SeqCst),
        resolved_titles,
        "loader-absent fallback must issue one media-size summary call per title"
    );
}

/// Tiered metadata-language and season-folder fields must use the request
/// loaders rather than resolving each catalog row through direct settings
/// reads. The loader-backed result must still match the direct resolver path.
#[tokio::test]
async fn catalog_tiered_overrides_batch_settings_reads_with_loader_parity() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let series_title_override = create_catalog_title(
        &ctx,
        "Batch title metadata override",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let series_library_override = create_catalog_title(
        &ctx,
        "Batch library metadata override",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    let anime_facet_override = create_catalog_title(
        &ctx,
        "Batch facet season override",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;

    ctx.settings_store
        .upsert_setting_json(
            "system",
            "metadata_language.title_override",
            Some(series_title_override.id.clone()),
            "\"fra\"".to_string(),
            "test",
            None,
        )
        .await
        .expect("set title metadata override");
    ctx.settings_store
        .upsert_setting_json(
            "system",
            "metadata_language",
            Some(series_library_override.library_id.clone()),
            "\"jpn\"".to_string(),
            "test",
            None,
        )
        .await
        .expect("set series library metadata override");
    ctx.settings_store
        .upsert_setting_json(
            "system",
            "rename.use_season_folders",
            Some(series_library_override.library_id.clone()),
            "false".to_string(),
            "test",
            None,
        )
        .await
        .expect("set series library season-folder override");
    ctx.settings_store
        .upsert_setting_json(
            "system",
            "rename.use_season_folders",
            Some("anime".to_string()),
            "false".to_string(),
            "test",
            None,
        )
        .await
        .expect("set anime facet season-folder override");

    let direct_explicit_reads = Arc::new(AtomicUsize::new(0));
    let batch_explicit_reads = Arc::new(AtomicUsize::new(0));
    let global_reads = Arc::new(AtomicUsize::new(0));
    let counting_app = ctx.app.with_test_overrides({
        let settings = Arc::new(CountingSettingsRepo {
            inner: ctx.settings_store.clone(),
            direct_explicit_reads: direct_explicit_reads.clone(),
            batch_explicit_reads: batch_explicit_reads.clone(),
            global_reads: global_reads.clone(),
        });
        move |builder| builder.with_settings(settings)
    });
    let schema = scryer_interface::build_schema(counting_app.clone(), ctx.auth_runtime.clone());
    let user = authorized_default_user(&ctx).await;
    let query = r#"{
        titles {
            items {
                id
                effectiveMetadataLanguage
                effectiveUseSeasonFolders
            }
        }
    }"#;

    let direct = schema
        .execute(async_graphql::Request::new(query).data(user.clone()))
        .await;
    let direct = serde_json::to_value(&direct).expect("serialize direct response");
    assert_ok(&direct);
    assert_eq!(batch_explicit_reads.load(Ordering::SeqCst), 0);
    assert!(
        direct_explicit_reads.load(Ordering::SeqCst) > 3,
        "direct fields should resolve each tier separately"
    );

    direct_explicit_reads.store(0, Ordering::SeqCst);
    batch_explicit_reads.store(0, Ordering::SeqCst);
    global_reads.store(0, Ordering::SeqCst);
    let loaders = scryer_interface::RequestLoaders::new(counting_app, user.clone());
    let batched = schema
        .execute(async_graphql::Request::new(query).data(user).data(loaders))
        .await;
    let batched = serde_json::to_value(&batched).expect("serialize batched response");
    assert_ok(&batched);
    assert_eq!(
        batched["data"]["titles"]["items"], direct["data"]["titles"]["items"],
        "request loaders must preserve direct title resolution"
    );
    assert_eq!(
        batch_explicit_reads.load(Ordering::SeqCst),
        4,
        "three tiered settings loaders plus the facet fallback should each issue one batched read"
    );
    assert_eq!(direct_explicit_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        global_reads.load(Ordering::SeqCst),
        1,
        "the global metadata language should be read once per request"
    );

    let items = batched["data"]["titles"]["items"]
        .as_array()
        .expect("catalog items");
    let item = |title_id: &str| {
        items
            .iter()
            .find(|item| item["id"] == title_id)
            .unwrap_or_else(|| panic!("missing title {title_id}: {batched}"))
    };
    assert_eq!(
        item(&series_title_override.id)["effectiveMetadataLanguage"],
        "fra"
    );
    assert_eq!(
        item(&series_library_override.id)["effectiveMetadataLanguage"],
        "jpn"
    );
    assert_eq!(
        item(&anime_facet_override.id)["effectiveMetadataLanguage"],
        "eng"
    );
    assert_eq!(
        item(&series_library_override.id)["effectiveUseSeasonFolders"],
        false
    );
    assert_eq!(
        item(&anime_facet_override.id)["effectiveUseSeasonFolders"],
        false
    );
}

/// The countable trio (`episodesOwned`/`episodesMonitored`/`episodesTotal`)
/// excludes placeholder episodes (unnamed/TBA/undated) while
/// `episodeRecordsTotal` counts every stored row, so the season-scoped panel
/// can tell "no episodes at all" apart from "only placeholder episodes"
/// without hydrating the rows. Both resolution paths must agree.
#[tokio::test]
async fn collection_counts_split_countable_and_record_totals() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Record Count Show",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;

    let season = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    // Three episode rows: two countable (named + dated, first one owned) and a
    // third left undated so it stays a placeholder record.
    for index in 1..=3usize {
        let episode = create_series_scan_episode(
            &ctx,
            &title,
            &season,
            "1",
            &index.to_string(),
            &format!("S01E0{index}"),
        )
        .await;
        if index <= 2 {
            ctx.shows
                .update_episode(
                    &episode.id,
                    EpisodeUpdate {
                        air_date: Some(Some(format!("2024-02-0{index}"))),
                        ..Default::default()
                    },
                )
                .await
                .expect("set episode air date");
        }
        if index == 1 {
            let file_id = ctx
                .media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: "/record-count-fixtures/S01E01.mkv".to_string(),
                    size_bytes: 4_096,
                    ..Default::default()
                })
                .await
                .expect("insert media file");
            ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
                .await
                .expect("link media file to episode");
        }
    }

    let query = format!(
        "{{ title(id: \"{}\") {{ collections {{ id episodesOwned episodesMonitored episodesTotal episodeRecordsTotal }} }} }}",
        title.id
    );

    // HTTP path: no request loaders, so this exercises the fallback resolvers.
    let body = gql(&ctx, &query, json!({})).await;
    assert_ok(&body);
    let collection = body["data"]["title"]["collections"][0].clone();
    assert_eq!(collection["episodesOwned"].as_i64(), Some(1), "{body}");
    assert_eq!(collection["episodesMonitored"].as_i64(), Some(2), "{body}");
    assert_eq!(collection["episodesTotal"].as_i64(), Some(2), "{body}");
    assert_eq!(
        collection["episodeRecordsTotal"].as_i64(),
        Some(3),
        "{body}"
    );

    // Loader-backed path must produce the identical counts.
    let user = authorized_default_user(&ctx).await;
    let loaded_body = exec_with_request_loaders(&ctx, &query, user).await;
    assert_ok(&loaded_body);
    assert_eq!(
        loaded_body["data"]["title"]["collections"][0], collection,
        "loader-backed collection counts diverged from fallback"
    );
}

/// Counts library row reads while delegating every call to the real store.
struct CountingLibraryRepo {
    inner: Arc<dyn LibraryRepository>,
    list_calls: Arc<AtomicUsize>,
    get_by_id_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LibraryRepository for CountingLibraryRepo {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list(facet).await
    }
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        self.get_by_id_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.get_by_id(id).await
    }
    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        self.inner.default_for_facet(facet).await
    }
    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        self.inner.create(library, roots).await
    }
    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        self.inner.update(library_id, name, slug, roots).await
    }
    async fn set_root_path(&self, root_id: &str, path: &str) -> AppResult<Library> {
        self.inner.set_root_path(root_id, path).await
    }
    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        self.inner.delete_library(library_id).await
    }
    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        self.inner.app_permission_mask_for_user(user_id).await
    }
    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        self.inner
            .set_app_permission_mask_for_user(user_id, permissions)
            .await
    }
    async fn permission_masks_for_user(
        &self,
        user_id: &str,
    ) -> AppResult<Vec<scryer_domain::LibraryGrant>> {
        self.inner.permission_masks_for_user(user_id).await
    }
    async fn set_grants_for_user(
        &self,
        user_id: &str,
        grants: Vec<scryer_domain::LibraryGrant>,
    ) -> AppResult<()> {
        self.inner.set_grants_for_user(user_id, grants).await
    }
    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        self.inner.title_library_id(title_id).await
    }
    async fn root_title_counts(&self) -> AppResult<std::collections::HashMap<String, i64>> {
        self.inner.root_title_counts().await
    }
}

const DEFAULT_MONITOR_TYPE_KEY: &str = "catalog.default_monitor_type";
const QUALITY_PROFILE_ID_KEY: &str = "quality.profile_id";
const REQUEST_QUALITY_PROFILE_IDS_KEY: &str = "quality.request_profile_ids";

async fn seed_request_default_definitions(ctx: &TestContext) {
    let seed = |category: &str, key_name: &str, data_type: &str, default_value_json: &str| {
        SettingDefinitionSeed {
            category: category.into(),
            scope: "system".into(),
            key_name: key_name.into(),
            data_type: data_type.into(),
            default_value_json: default_value_json.into(),
            is_sensitive: false,
            validation_json: None,
        }
    };
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            seed("catalog", DEFAULT_MONITOR_TYPE_KEY, "string", "null"),
            seed("quality", QUALITY_PROFILE_ID_KEY, "string", "null"),
            seed("quality", REQUEST_QUALITY_PROFILE_IDS_KEY, "json", "[]"),
        ])
        .await
        .expect("seed request default definitions");
}

async fn save_library_setting(ctx: &TestContext, key_name: &str, library_id: &str, value: Value) {
    ctx.settings_store
        .upsert_setting_json(
            "system",
            key_name,
            Some(library_id.to_string()),
            value.to_string(),
            "test",
            None,
        )
        .await
        .expect("save library setting");
}

async fn create_request_default_library(
    ctx: &TestContext,
    admin: &scryer_domain::User,
    facet: MediaFacet,
    name: &str,
    path: &str,
) -> Library {
    ctx.app
        .create_library(
            admin,
            facet,
            name.to_string(),
            vec![LibraryRootDraft {
                id: None,
                path: path.to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("create library")
}

/// The batched request-default readers behind the library loaders must give
/// every library the same value or the same error as the per-library readers,
/// for an administrator and for an actor holding different grants per library.
#[tokio::test]
async fn library_request_default_batches_match_per_library_reads() {
    let ctx = TestContext::new().await;
    seed_request_default_definitions(&ctx).await;
    let admin = authorized_default_user(&ctx).await;
    let movies = create_request_default_library(
        &ctx,
        &admin,
        MediaFacet::Movie,
        "Synthetic Request Movies",
        "/request-defaults/movies",
    )
    .await;
    let series = create_request_default_library(
        &ctx,
        &admin,
        MediaFacet::Series,
        "Synthetic Request Series",
        "/request-defaults/series",
    )
    .await;
    let anime = create_request_default_library(
        &ctx,
        &admin,
        MediaFacet::Anime,
        "Synthetic Request Anime",
        "/request-defaults/anime",
    )
    .await;
    let profile_id = ctx
        .app
        .request_quality_profile_settings_for_library(&admin, &movies.id)
        .await
        .expect("baseline request settings")
        .default_profile_id;

    save_library_setting(
        &ctx,
        DEFAULT_MONITOR_TYPE_KEY,
        &series.id,
        json!("ALL_EPISODES"),
    )
    .await;
    // A series choice saved on a movie library falls back to the movie default.
    save_library_setting(
        &ctx,
        DEFAULT_MONITOR_TYPE_KEY,
        &movies.id,
        json!("FUTURE_EPISODES"),
    )
    .await;
    save_library_setting(&ctx, QUALITY_PROFILE_ID_KEY, &movies.id, json!(profile_id)).await;
    save_library_setting(
        &ctx,
        QUALITY_PROFILE_ID_KEY,
        &anime.id,
        json!("synthetic-missing-profile"),
    )
    .await;
    save_library_setting(
        &ctx,
        REQUEST_QUALITY_PROFILE_IDS_KEY,
        &movies.id,
        json!([profile_id, "synthetic-unknown-profile"]),
    )
    .await;
    save_library_setting(&ctx, REQUEST_QUALITY_PROFILE_IDS_KEY, &series.id, json!([])).await;

    let mut mixed = scryer_domain::User::new_admin("synthetic-mixed-grants");
    mixed.authorization = UserAuthorization::default();
    mixed.authorization.loaded = true;
    mixed.authorization.libraries.insert(
        movies.id.clone(),
        LibraryPermissionMask::from_permissions([
            LibraryPermission::View,
            LibraryPermission::Request,
        ]),
    );
    mixed.authorization.libraries.insert(
        series.id.clone(),
        LibraryPermissionMask::from_permissions([LibraryPermission::ManageTitles]),
    );
    mixed.authorization.libraries.insert(
        anime.id.clone(),
        LibraryPermissionMask::from_permissions([LibraryPermission::View]),
    );

    let libraries = ctx
        .app
        .libraries_for_request_defaults()
        .await
        .expect("library rows");
    let mut ids: Vec<String> = libraries.keys().cloned().collect();
    ids.sort();
    ids.push("synthetic-missing-library".to_string());
    assert!(ids.len() >= 4, "seeded libraries missing: {ids:?}");

    let mut outcomes = std::collections::BTreeSet::new();
    for actor in [&admin, &mixed] {
        let monitor = ctx
            .app
            .default_monitor_types_for_libraries(actor, &ids, &libraries)
            .await
            .expect("batched monitor defaults");
        let quality = ctx
            .app
            .title_quality_profile_ids_for_libraries(actor, &ids, &libraries)
            .await
            .expect("batched quality profiles");
        let request = ctx
            .app
            .request_quality_profile_settings_for_libraries(actor, &ids, &libraries)
            .await
            .expect("batched request profiles");
        for id in &ids {
            let direct = ctx.app.default_monitor_type_for_library(actor, id).await;
            assert_eq!(
                format!("{:?}", monitor[id]),
                format!("{direct:?}"),
                "monitor {id}"
            );
            let direct = ctx
                .app
                .title_quality_profile_id_for_library(actor, id)
                .await;
            assert_eq!(
                format!("{:?}", quality[id]),
                format!("{direct:?}"),
                "quality {id}"
            );
            let direct = ctx
                .app
                .request_quality_profile_settings_for_library(actor, id)
                .await;
            assert_eq!(
                format!("{:?}", request[id]),
                format!("{direct:?}"),
                "request {id}"
            );
            for outcome in [
                monitor[id].as_ref().map(|_| ()),
                quality[id].as_ref().map(|_| ()),
                request[id].as_ref().map(|_| ()),
            ] {
                outcomes.insert(match outcome {
                    Ok(()) => "ok".to_string(),
                    Err(error) => error.reason().unwrap_or("uncoded").to_string(),
                });
            }
        }
    }

    let admin_monitor = ctx
        .app
        .default_monitor_types_for_libraries(&admin, &ids, &libraries)
        .await
        .expect("admin monitor defaults");
    assert_eq!(
        admin_monitor[&series.id].as_deref().ok(),
        Some("ALL_EPISODES")
    );
    assert_eq!(admin_monitor[&movies.id].as_deref().ok(), Some("MONITORED"));
    let admin_request = ctx
        .app
        .request_quality_profile_settings_for_libraries(&admin, &ids, &libraries)
        .await
        .expect("admin request profiles");
    assert_eq!(
        admin_request[&movies.id]
            .as_ref()
            .expect("movie request profiles")
            .profile_ids,
        vec![profile_id.clone()]
    );
    assert!(
        outcomes.len() >= 3,
        "fixture must cover values and several refusals: {outcomes:?}"
    );
}

/// One `libraries` query selecting every request-default field reads no
/// library row per library and batches the per-library setting reads when
/// request loaders are present, and returns the same data as without them.
#[tokio::test]
async fn library_request_default_fields_are_batched_with_loaders() {
    let ctx = TestContext::new().await;
    seed_request_default_definitions(&ctx).await;
    let admin = authorized_default_user(&ctx).await;
    let mut created = Vec::new();
    for (index, facet) in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
        .into_iter()
        .enumerate()
    {
        created.push(
            create_request_default_library(
                &ctx,
                &admin,
                facet,
                &format!("Synthetic Batched Library {index}"),
                &format!("/request-defaults/batched-{index}"),
            )
            .await,
        );
    }
    save_library_setting(
        &ctx,
        DEFAULT_MONITOR_TYPE_KEY,
        &created[1].id,
        json!("ALL_EPISODES"),
    )
    .await;
    save_library_setting(
        &ctx,
        REQUEST_QUALITY_PROFILE_IDS_KEY,
        &created[0].id,
        json!([]),
    )
    .await;

    let list_calls = Arc::new(AtomicUsize::new(0));
    let get_by_id_calls = Arc::new(AtomicUsize::new(0));
    let direct_explicit_reads = Arc::new(AtomicUsize::new(0));
    let batch_explicit_reads = Arc::new(AtomicUsize::new(0));
    let with_default_reads = Arc::new(AtomicUsize::new(0));
    let counting_app = ctx.app.with_test_overrides({
        let libraries = Arc::new(CountingLibraryRepo {
            inner: Arc::new(ctx.libraries.clone()),
            list_calls: list_calls.clone(),
            get_by_id_calls: get_by_id_calls.clone(),
        });
        let settings = Arc::new(CountingSettingsRepo {
            inner: ctx.settings_store.clone(),
            direct_explicit_reads: direct_explicit_reads.clone(),
            batch_explicit_reads: batch_explicit_reads.clone(),
            global_reads: with_default_reads.clone(),
        });
        move |builder| builder.with_libraries(libraries).with_settings(settings)
    });
    let schema = scryer_interface::build_schema(counting_app.clone(), ctx.auth_runtime.clone());
    let query = "{ libraries { id defaultMonitorType qualityProfileId requestQualityProfileIds requestQualityProfileDefaultId } }";
    let counts = || {
        [
            list_calls.swap(0, Ordering::SeqCst),
            get_by_id_calls.swap(0, Ordering::SeqCst),
            direct_explicit_reads.swap(0, Ordering::SeqCst),
            batch_explicit_reads.swap(0, Ordering::SeqCst),
            with_default_reads.swap(0, Ordering::SeqCst),
        ]
    };
    counts();

    let direct = schema
        .execute(async_graphql::Request::new(query).data(admin.clone()))
        .await;
    let direct = serde_json::to_value(&direct).expect("serialize direct response");
    assert_ok(&direct);
    let direct_counts = counts();

    let run_batched = || async {
        let loaders = scryer_interface::RequestLoaders::new(counting_app.clone(), admin.clone());
        let batched = schema
            .execute(
                async_graphql::Request::new(query)
                    .data(admin.clone())
                    .data(loaders),
            )
            .await;
        let batched = serde_json::to_value(&batched).expect("serialize batched response");
        assert_ok(&batched);
        batched
    };
    let batched = run_batched().await;
    let batched_counts = counts();

    let library_count = batched["data"]["libraries"]
        .as_array()
        .expect("libraries")
        .len();
    eprintln!(
        "libraries={library_count} [list, get_by_id, explicit, batched explicit, with-defaults] direct={direct_counts:?} loaders={batched_counts:?}"
    );
    assert_eq!(
        batched["data"], direct["data"],
        "loaders must not change values"
    );
    assert!(
        library_count >= 6,
        "expected default and seeded libraries: {batched}"
    );
    assert_eq!(
        direct_counts[1],
        4 * library_count,
        "without loaders each library is read once per request-default field"
    );
    assert_eq!(batched_counts[1], 0, "loaders read no library row by id");
    assert_eq!(
        batched_counts[0],
        direct_counts[0] + 1,
        "loaders read every library row once for all request-default fields"
    );
    assert_eq!(
        batched_counts[3], 4,
        "one batched read each for the monitor, quality, request profile ids and request default keys"
    );

    for (index, facet) in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
        .into_iter()
        .enumerate()
    {
        create_request_default_library(
            &ctx,
            &admin,
            facet,
            &format!("Synthetic Added Library {index}"),
            &format!("/request-defaults/added-{index}"),
        )
        .await;
    }
    counts();
    let grown = run_batched().await;
    let grown_counts = counts();
    let grown_count = grown["data"]["libraries"]
        .as_array()
        .expect("libraries")
        .len();
    eprintln!("libraries={grown_count} loaders={grown_counts:?}");
    assert_eq!(grown_count, library_count + 3, "three libraries were added");
    for (counter, (before, after)) in batched_counts.iter().zip(grown_counts).enumerate() {
        assert!(
            after <= *before,
            "loader reads must not grow with the library count: counter {counter} went from {before} to {after}"
        );
    }
}
