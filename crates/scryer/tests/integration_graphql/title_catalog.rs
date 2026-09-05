use super::*;

async fn create_title_catalog_library(
    ctx: &TestContext,
    facet: &str,
    name: &str,
    roots: &[(&str, bool)],
) -> Value {
    let body = gql(
        ctx,
        r#"mutation($input: CreateLibraryInput!) {
            createLibrary(input: $input) {
                id
                roots { id path isDefault }
            }
        }"#,
        json!({
            "input": {
                "facet": facet,
                "name": name,
                "roots": roots
                    .iter()
                    .map(|(path, is_default)| json!({
                        "path": path,
                        "isDefault": is_default,
                    }))
                    .collect::<Vec<_>>()
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["createLibrary"].clone()
}

fn library_id(library: &Value) -> String {
    library["id"].as_str().expect("library id").to_string()
}

async fn seed_title_quality_profiles(ctx: &TestContext, ids: &[&str]) {
    seed_typed_settings_definitions(ctx).await;
    let actor = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("default settings actor");
    let profiles = ids
        .iter()
        .map(|id| {
            let mut profile = scryer_application::builtin_1080p_profile();
            profile.id = (*id).to_string();
            profile.name = format!("Fixture {id}");
            profile
        })
        .collect();
    ctx.app
        .save_quality_profile_settings(
            &actor,
            scryer_application::SaveQualityProfileSettings {
                profiles,
                replace_existing: false,
                global_profile_id: None,
                category_selections: vec![],
                global_scoring_persona: None,
                category_persona_selections: vec![],
            },
        )
        .await
        .expect("seed title quality profiles through settings store");
}

fn library_root_id(library: &Value, path: &str) -> String {
    library["roots"]
        .as_array()
        .expect("library roots")
        .iter()
        .find(|root| root["path"].as_str() == Some(path))
        .and_then(|root| root["id"].as_str())
        .unwrap_or_else(|| panic!("root id for {path}"))
        .to_string()
}

fn catalog_view_actor(library_id: &str) -> User {
    User {
        id: Id::new().0,
        username: "catalog-filter-viewer".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::from([(
                library_id.to_string(),
                LibraryPermissionMask::from_permissions([LibraryPermission::View]),
            )]),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

async fn add_catalog_filter_title(
    ctx: &TestContext,
    name: &str,
    tvdb_id: &str,
    library_id: &str,
    root_folder_id: &str,
    year: i32,
) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": name,
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": tvdb_id }],
                "options": { "rootFolderId": root_folder_id }
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let title_id = body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("catalog filter title id")
        .to_string();
    sqlx::query("UPDATE titles SET year = ? WHERE id = ?")
        .bind(year)
        .bind(&title_id)
        .execute(ctx.db.pool())
        .await
        .expect("catalog filter title year should update");
    title_id
}

async fn seed_catalog_filter_metadata(
    ctx: &TestContext,
    title_id: &str,
    tags: &[(&str, &str, &str)],
    rating: Option<f64>,
) {
    for (tag_key, category, name) in tags {
        sqlx::query(
            "INSERT INTO title_metadata_tags (
                title_id, tag_key, category, name, confidence, is_adult, is_spoiler, sort_index
             ) VALUES (?, ?, ?, ?, 1.0, 0, 0, 0)",
        )
        .bind(title_id)
        .bind(tag_key)
        .bind(category)
        .bind(name)
        .execute(ctx.db.pool())
        .await
        .expect("title catalog filter tag should insert");
    }

    if let Some(rating) = rating {
        sqlx::query("INSERT INTO title_metadata_rating_summaries (title_id, rating) VALUES (?, ?)")
            .bind(title_id)
            .bind(rating)
            .execute(ctx.db.pool())
            .await
            .expect("title catalog filter rating should insert");
    }
}

async fn stored_title_root_folder_id(ctx: &TestContext, title_id: &str) -> String {
    ctx.titles
        .get_by_id(title_id)
        .await
        .expect("title should load")
        .expect("title should exist")
        .root_folder_id
}

#[tokio::test]
async fn graphql_list_titles_starts_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ titles { items { id } } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"]["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn graphql_title_catalog_rejects_invalid_filter_values() {
    let ctx = TestContext::new().await;
    for (filter, expected_message) in [
        (
            json!({ "minimumYear": 2025, "maximumYear": 2024 }),
            "minimumYear",
        ),
        (json!({ "minimumRating": 10.1 }), "minimumRating"),
        (json!({ "genreTagKeys": [" "] }), "genreTagKeys"),
    ] {
        let body = gql(
            &ctx,
            "query($filter: TitleCatalogFilterInput) { titles(filter: $filter) { items { id } } }",
            json!({ "filter": filter }),
        )
        .await;
        let message = body["errors"][0]["message"]
            .as_str()
            .expect("validation error message");
        assert!(message.contains(expected_message), "{body}");
    }
}

#[tokio::test]
async fn graphql_title_catalog_advanced_filters_apply_to_seeded_catalog_data() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Catalog Filter Library",
        &[
            ("/catalog-filter/default", true),
            ("/catalog-filter/archive", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let default_root_id = library_root_id(&library, "/catalog-filter/default");
    let archive_root_id = library_root_id(&library, "/catalog-filter/archive");
    let first_title_id = add_catalog_filter_title(
        &ctx,
        "Catalog Record A",
        "990001",
        &library_id,
        &default_root_id,
        2001,
    )
    .await;
    let second_title_id = add_catalog_filter_title(
        &ctx,
        "Catalog Record B",
        "990002",
        &library_id,
        &archive_root_id,
        2015,
    )
    .await;
    let unrated_title_id = add_catalog_filter_title(
        &ctx,
        "Catalog Record C",
        "990003",
        &library_id,
        &default_root_id,
        2004,
    )
    .await;
    seed_catalog_filter_metadata(
        &ctx,
        &first_title_id,
        &[
            ("genre-alpha", "genre", "Genre Alpha"),
            ("theme-signal", "theme", "Theme Signal"),
        ],
        Some(8.0),
    )
    .await;
    seed_catalog_filter_metadata(
        &ctx,
        &second_title_id,
        &[
            ("genre-beta", "genre", "Genre Beta"),
            ("theme-other", "theme", "Theme Other"),
        ],
        Some(9.0),
    )
    .await;
    seed_catalog_filter_metadata(
        &ctx,
        &unrated_title_id,
        &[
            ("genre-alpha", "genre", "Genre Alpha"),
            ("theme-signal", "theme", "Theme Signal"),
        ],
        None,
    )
    .await;
    let body = gql(
        &ctx,
        r#"query(
            $libraryIds: [ID!]
            $rootFolderIds: [ID!]
            $genreTagKeys: [String!]
            $themeTagKeys: [String!]
        ) {
            titles(
                facet: MOVIE,
                libraryIds: $libraryIds,
                filter: {
                    rootFolderIds: $rootFolderIds
                    genreTagKeys: $genreTagKeys
                    themeTagKeys: $themeTagKeys
                    minimumYear: 2001
                    maximumYear: 2004
                    minimumRating: 7.5
                }
            ) { items { id } totalCount }
            titleCatalogFilterOptions(
                facet: MOVIE
                libraryIds: $libraryIds
                rootFolderIds: $rootFolderIds
            ) {
                genres { key name }
                themes { key name }
                minimumYear
                maximumYear
            }
        }"#,
        json!({
            "libraryIds": [library_id],
            "rootFolderIds": [default_root_id],
            "genreTagKeys": ["genre-missing", "genre-alpha"],
            "themeTagKeys": ["theme-signal"],
        }),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"]["totalCount"], 1);
    assert_eq!(body["data"]["titles"]["items"][0]["id"], first_title_id);
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["genres"],
        json!([{ "key": "genre-alpha", "name": "Genre Alpha" }])
    );
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["themes"],
        json!([{ "key": "theme-signal", "name": "Theme Signal" }])
    );
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["minimumYear"],
        2001
    );
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["maximumYear"],
        2004
    );

    let unrated_body = gql(
        &ctx,
        r#"query($libraryIds: [ID!], $rootFolderIds: [ID!]) {
            titles(
                facet: MOVIE
                libraryIds: $libraryIds
                filter: {
                    rootFolderIds: $rootFolderIds
                    genreTagKeys: ["genre-alpha"]
                    themeTagKeys: ["theme-signal"]
                    minimumYear: 2001
                    maximumYear: 2004
                }
            ) { items { id } totalCount }
        }"#,
        json!({
            "libraryIds": [library_id],
            "rootFolderIds": [default_root_id],
        }),
    )
    .await;
    assert_no_errors(&unrated_body);
    let mut ids = unrated_body["data"]["titles"]["items"]
        .as_array()
        .expect("filtered title items")
        .iter()
        .map(|title| title["id"].as_str().expect("filtered title id"))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut expected_ids = vec![first_title_id.as_str(), unrated_title_id.as_str()];
    expected_ids.sort_unstable();
    assert_eq!(ids, expected_ids);
}

#[tokio::test]
async fn graphql_title_catalog_filters_honor_library_view_permissions() {
    let ctx = TestContext::new().await;
    let allowed_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Allowed Catalog Library",
        &[("/catalog-rbac/allowed", true)],
    )
    .await;
    let denied_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Denied Catalog Library",
        &[("/catalog-rbac/denied", true)],
    )
    .await;
    let allowed_library_id = library_id(&allowed_library);
    let denied_library_id = library_id(&denied_library);
    let allowed_root_id = library_root_id(&allowed_library, "/catalog-rbac/allowed");
    let denied_root_id = library_root_id(&denied_library, "/catalog-rbac/denied");
    let allowed_title_id = add_catalog_filter_title(
        &ctx,
        "Authorized Catalog Record",
        "991001",
        &allowed_library_id,
        &allowed_root_id,
        2002,
    )
    .await;
    let denied_title_id = add_catalog_filter_title(
        &ctx,
        "Restricted Catalog Record",
        "991002",
        &denied_library_id,
        &denied_root_id,
        2022,
    )
    .await;
    seed_catalog_filter_metadata(
        &ctx,
        &allowed_title_id,
        &[("genre-allowed", "genre", "Genre Allowed")],
        Some(8.0),
    )
    .await;
    seed_catalog_filter_metadata(
        &ctx,
        &denied_title_id,
        &[("genre-restricted", "genre", "Genre Restricted")],
        Some(9.0),
    )
    .await;

    let actor = catalog_view_actor(&allowed_library_id);
    let body = schema_exec(
        &ctx,
        &format!(
            r#"query {{
                titles(
                    facet: MOVIE
                    libraryIds: ["{allowed_library_id}", "{denied_library_id}"]
                ) {{ items {{ id }} totalCount }}
                titleCatalogFilterOptions(
                    facet: MOVIE
                    libraryIds: ["{allowed_library_id}", "{denied_library_id}"]
                    rootFolderIds: ["{allowed_root_id}", "{denied_root_id}"]
                ) {{ genres {{ key name }} minimumYear maximumYear }}
                deniedOptions: titleCatalogFilterOptions(
                    facet: MOVIE
                    libraryIds: ["{denied_library_id}"]
                ) {{ genres {{ key name }} minimumYear maximumYear }}
            }}"#,
        ),
        Some(actor),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"]["totalCount"], 1);
    assert_eq!(body["data"]["titles"]["items"][0]["id"], allowed_title_id);
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["genres"],
        json!([{ "key": "genre-allowed", "name": "Genre Allowed" }])
    );
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["minimumYear"],
        2002
    );
    assert_eq!(
        body["data"]["titleCatalogFilterOptions"]["maximumYear"],
        2002
    );
    assert_eq!(body["data"]["deniedOptions"]["genres"], json!([]));
    assert!(body["data"]["deniedOptions"]["minimumYear"].is_null());
    assert!(body["data"]["deniedOptions"]["maximumYear"].is_null());
}

async fn add_test_title_with_tvdb_id(
    ctx: &TestContext,
    name: &str,
    facet: &str,
    tvdb_id: &str,
) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) { addTitle(input: $input) { title { id name } } }"#,
        json!({
            "input": {
                "name": name,
                "facet": facet,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": tvdb_id }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn set_title_sort_title(ctx: &TestContext, title_id: &str, sort_title: &str) {
    sqlx::query("UPDATE titles SET sort_title = ? WHERE id = ?")
        .bind(sort_title)
        .bind(title_id)
        .execute(ctx.db.pool())
        .await
        .expect("title sort title fixture should update title row");
}

#[tokio::test]
async fn graphql_title_sort_uses_visible_name_ignoring_multilingual_articles_and_cjk_width() {
    let ctx = TestContext::new().await;
    let anchor_id = add_test_title_with_tvdb_id(&ctx, "ＡＮＣＨＯＲ", "ANIME", "900000").await;
    let apiary_id =
        add_test_title_with_tvdb_id(&ctx, "The Apiary Almanac", "ANIME", "900001").await;
    let arc_id = add_test_title_with_tvdb_id(&ctx, "L’Arc-en-Ciel", "MOVIE", "900002").await;
    let auto_id =
        add_test_title_with_tvdb_id(&ctx, "O Auto da Compadecida", "MOVIE", "900003").await;
    let avventura_id = add_test_title_with_tvdb_id(&ctx, "L'Avventura", "MOVIE", "900004").await;
    let better_id = add_test_title_with_tvdb_id(&ctx, "A Better Tomorrow", "MOVIE", "900005").await;
    let cercle_id = add_test_title_with_tvdb_id(&ctx, "Le Cercle Rouge", "MOVIE", "900006").await;
    let dorado_id = add_test_title_with_tvdb_id(&ctx, "El Dorado", "MOVIE", "900007").await;
    let education_id = add_test_title_with_tvdb_id(&ctx, "An Education", "MOVIE", "900008").await;
    let forgeheart_id =
        add_test_title_with_tvdb_id(&ctx, "Forgeheart Alchemy: Kinship", "ANIME", "900009").await;
    let himmel_id = add_test_title_with_tvdb_id(&ctx, "Der Himmel", "MOVIE", "900010").await;
    let jetee_id = add_test_title_with_tvdb_id(&ctx, "La Jetée", "MOVIE", "900011").await;
    let meridian_id =
        add_test_title_with_tvdb_id(&ctx, "Ｔｈｅ　Meridian", "MOVIE", "900012").await;
    set_title_sort_title(&ctx, &anchor_id, "zzzzzz").await;
    set_title_sort_title(&ctx, &apiary_id, "zzzz").await;
    set_title_sort_title(&ctx, &arc_id, "yyyy").await;
    set_title_sort_title(&ctx, &auto_id, "xxxx").await;
    set_title_sort_title(&ctx, &avventura_id, "wwww").await;
    set_title_sort_title(&ctx, &better_id, "mmmm").await;
    set_title_sort_title(&ctx, &cercle_id, "llll").await;
    set_title_sort_title(&ctx, &dorado_id, "kkkk").await;
    set_title_sort_title(&ctx, &education_id, "aaaa").await;
    set_title_sort_title(&ctx, &forgeheart_id, "鍛心 forgeheart alchemy").await;
    set_title_sort_title(&ctx, &himmel_id, "iiii").await;
    set_title_sort_title(&ctx, &jetee_id, "hhhh").await;
    set_title_sort_title(&ctx, &meridian_id, "gggg").await;

    let body = gql(
        &ctx,
        r#"query($limit: Int, $offset: Int, $sort: TitleCatalogSortInput) {
            titles(limit: $limit, offset: $offset, sort: $sort) {
                hasMore
                items { name sortTitle }
            }
        }"#,
        json!({
            "limit": 13,
            "offset": 0,
            "sort": { "key": "TITLE", "direction": "ASC" }
        }),
    )
    .await;
    assert_no_errors(&body);
    let names: Vec<&str> = body["data"]["titles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "ＡＮＣＨＯＲ",
            "The Apiary Almanac",
            "L’Arc-en-Ciel",
            "O Auto da Compadecida",
            "L'Avventura",
            "A Better Tomorrow",
            "Le Cercle Rouge",
            "El Dorado",
            "An Education",
            "Forgeheart Alchemy: Kinship",
            "Der Himmel",
            "La Jetée",
            "Ｔｈｅ　Meridian",
        ]
    );
}

#[tokio::test]
async fn graphql_catalog_quality_tier_resolves_target_profile_without_media() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Target Quality Movie", "MOVIE").await;

    let body = gql(
        &ctx,
        "{ titles { items { id qualityTier currentQualityTier } } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let title = body["data"]["titles"]["items"]
        .as_array()
        .expect("title list")
        .iter()
        .find(|item| item["id"].as_str() == Some(title_id.as_str()))
        .expect("added title");
    assert_eq!(title["qualityTier"], "1080P");
    assert!(title["currentQualityTier"].is_null());
}

#[tokio::test]
async fn graphql_titles_use_server_pagination_and_sort() {
    let ctx = TestContext::new().await;
    let zeta_id = add_test_title(&ctx, "Zeta Movie", "MOVIE").await;
    let alpha_id = add_test_title(&ctx, "Alpha Series", "SERIES").await;
    let middle_id = add_test_title(&ctx, "Middle Anime", "ANIME").await;
    insert_catalog_sort_collection(&ctx, "alpha-page-season", &alpha_id, 1, None).await;
    insert_catalog_sort_collection(&ctx, "middle-page-season", &middle_id, 1, None).await;

    let default_body = gql(
        &ctx,
        "{ titles { hasMore totalCount items { name } } }",
        json!({}),
    )
    .await;
    assert_no_errors(&default_body);
    assert_eq!(default_body["data"]["titles"]["totalCount"], 3);
    assert!(!default_body["data"]["titles"]["hasMore"].as_bool().unwrap());

    let first_page = gql(
        &ctx,
        r#"query($limit: Int, $offset: Int, $sort: TitleCatalogSortInput) {
            titles(limit: $limit, offset: $offset, sort: $sort) {
                hasMore
                totalCount
                items {
                    name
                    collections { id collectionIndex }
                }
            }
        }"#,
        json!({
            "limit": 2,
            "offset": 0,
            "sort": { "key": "TITLE", "direction": "ASC" }
        }),
    )
    .await;
    assert_no_errors(&first_page);
    assert_eq!(first_page["data"]["titles"]["totalCount"], 3);
    assert!(first_page["data"]["titles"]["hasMore"].as_bool().unwrap());
    let first_names: Vec<&str> = first_page["data"]["titles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert_eq!(first_names, vec!["Alpha Series", "Middle Anime"]);
    let first_collections: Vec<Vec<&str>> = first_page["data"]["titles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            item["collections"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|collection| collection["id"].as_str())
                .collect()
        })
        .collect();
    assert_eq!(
        first_collections,
        vec![vec!["alpha-page-season"], vec!["middle-page-season"]]
    );

    let second_page = gql(
        &ctx,
        r#"query($limit: Int, $offset: Int, $sort: TitleCatalogSortInput) {
            titles(limit: $limit, offset: $offset, sort: $sort) {
                hasMore
                items { name }
            }
        }"#,
        json!({
            "limit": 2,
            "offset": 2,
            "sort": { "key": "TITLE", "direction": "ASC" }
        }),
    )
    .await;
    assert_no_errors(&second_page);
    assert!(!second_page["data"]["titles"]["hasMore"].as_bool().unwrap());
    let second_names: Vec<&str> = second_page["data"]["titles"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert_eq!(second_names, vec!["Zeta Movie"]);

    let monitored_page = gql(
        &ctx,
        r#"query($filter: TitleCatalogFilterInput) {
            titles(filter: $filter) {
                totalCount
                items { name monitored }
            }
        }"#,
        json!({ "filter": { "monitored": true } }),
    )
    .await;
    assert_no_errors(&monitored_page);
    assert_eq!(monitored_page["data"]["titles"]["totalCount"], 3);
    assert!(
        monitored_page["data"]["titles"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["monitored"].as_bool().unwrap())
    );

    update_title_catalog_sort_fixture(&ctx, &alpha_id, true, "returning", "alpha").await;
    update_title_catalog_sort_fixture(&ctx, &middle_id, true, "upcoming", "gamma").await;
    update_title_catalog_sort_fixture(&ctx, &zeta_id, false, "finished", "beta").await;
    seed_title_size_sort_fixture(
        &ctx,
        &alpha_id,
        "alpha-size-collection",
        "/sort-fixtures/alpha/movie.mkv",
        900,
    )
    .await;
    seed_title_size_sort_fixture(
        &ctx,
        &middle_id,
        "middle-size-collection",
        "/sort-fixtures/middle/movie.mkv",
        500,
    )
    .await;
    seed_title_size_sort_fixture(
        &ctx,
        &zeta_id,
        "zeta-size-collection",
        "/sort-fixtures/zeta/movie.mkv",
        100,
    )
    .await;
    seed_title_episode_sort_fixture(&ctx, &alpha_id, "alpha-episodes", 2, 2).await;
    seed_title_episode_sort_fixture(&ctx, &zeta_id, "zeta-episodes", 1, 2).await;

    assert_eq!(
        title_catalog_sort_names(&ctx, "MONITORED", "DESC").await,
        vec!["Alpha Series", "Middle Anime", "Zeta Movie"]
    );
    assert_eq!(
        title_catalog_sort_names(&ctx, "QUALITY", "ASC").await,
        vec!["Alpha Series", "Zeta Movie", "Middle Anime"]
    );
    assert_eq!(
        title_catalog_sort_names(&ctx, "STATUS", "ASC").await,
        vec!["Alpha Series", "Zeta Movie", "Middle Anime"]
    );
    assert_eq!(
        title_catalog_sort_names(&ctx, "SIZE", "DESC").await,
        vec!["Alpha Series", "Middle Anime", "Zeta Movie"]
    );
    assert_eq!(
        title_catalog_sort_names(&ctx, "EPISODES", "DESC").await,
        vec!["Alpha Series", "Zeta Movie", "Middle Anime"]
    );
}

#[tokio::test]
async fn graphql_add_title_movie() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Movie", "MOVIE").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_tv() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Series", "SERIES").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_anime() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Anime", "ANIME").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_title_options_input_uses_root_folder_id() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{
            titleOptionsInput: __type(name: "TitleOptionsInput") {
                inputFields(includeDeprecated: true) {
                    name
                    isDeprecated
                    deprecationReason
                }
            }
            titlePayload: __type(name: "TitlePayload") {
                fields {
                    name
                    type { kind name ofType { kind name } }
                }
            }
            createLibraryRootInput: __type(name: "CreateLibraryRootInput") {
                inputFields { name }
            }
            updateLibraryRootInput: __type(name: "UpdateLibraryRootInput") {
                inputFields { name }
            }
        }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let input_fields = body["data"]["titleOptionsInput"]["inputFields"]
        .as_array()
        .expect("input fields");
    let fields = input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect::<Vec<_>>();
    assert!(fields.contains(&"rootFolderId"));
    assert!(!fields.contains(&"rootFolderPath"));

    // FR-077/SC-009: the field is still served for creation and for fileless
    // titles, but it carries the deprecation that points existing clients at
    // the move workflow, so default introspection hides it.
    let root_folder_id_input = input_fields
        .iter()
        .find(|field| field["name"].as_str() == Some("rootFolderId"))
        .expect("rootFolderId input field");
    assert_eq!(root_folder_id_input["isDeprecated"], true);
    let deprecation_reason = root_folder_id_input["deprecationReason"]
        .as_str()
        .expect("deprecation reason");
    assert!(
        deprecation_reason.contains("locationOperationPreview")
            && deprecation_reason.contains("startLocationOperation"),
        "unexpected deprecation reason: {deprecation_reason}"
    );

    let title_fields = body["data"]["titlePayload"]["fields"]
        .as_array()
        .expect("title payload fields");
    let root_folder_id_field = title_fields
        .iter()
        .find(|field| field["name"].as_str() == Some("rootFolderId"))
        .expect("rootFolderId field");
    assert_eq!(root_folder_id_field["type"]["kind"], "NON_NULL");
    assert_eq!(root_folder_id_field["type"]["ofType"]["name"], "ID");

    for input_name in ["createLibraryRootInput", "updateLibraryRootInput"] {
        let root_fields = body["data"][input_name]["inputFields"]
            .as_array()
            .expect("library root input fields")
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect::<Vec<_>>();
        assert!(root_fields.contains(&"path"));
        assert!(root_fields.contains(&"isDefault"));
        assert!(
            !root_fields.contains(&"id"),
            "{input_name} should not accept id"
        );
    }

    let rejected = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": "Path Input Should Fail",
                "facet": "ANIME",
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderPath": "/library/anime"
                }
            }
        }),
    )
    .await;
    assert!(
        rejected.get("errors").is_some(),
        "rootFolderPath input should be rejected: {rejected}"
    );
}

#[tokio::test]
async fn graphql_add_title_with_structured_options() {
    let ctx = TestContext::new().await;
    seed_title_quality_profiles(&ctx, &["anime-hd"]).await;
    // Creation is registry-gated, so the user label this fixture carries has to
    // be defined before a title can be born with it.
    define_title_tag(&ctx, "favorite", None).await;
    let library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Configured Anime Library",
        &[
            ("/library/anime-default", true),
            ("/library/anime-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let root_folder_id = library_root_id(&library, "/library/anime-custom");
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title {
                    id
                    tags
                    rootFolderId
                    qualityProfileId
                    rootFolderPath
                    monitorType
                    useSeasonFolders
                    monitorSpecials
                    interSeasonMovies
                    fillerPolicy
                    recapPolicy
                }
            }
        }"#,
        json!({
            "input": {
                "name": "Configured Anime",
                "facet": "ANIME",
                "libraryId": library_id,
                "monitored": true,
                "tags": ["favorite"],
                "options": {
                    "qualityProfileId": "anime-hd",
                    "rootFolderId": root_folder_id,
                    "monitorType": "FUTURE_EPISODES",
                    "useSeasonFolders": false,
                    "monitorSpecials": true,
                    "interSeasonMovies": false,
                    "fillerPolicy": "SKIP_FILLER",
                    "recapPolicy": "SKIP_RECAP"
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let title = &body["data"]["addTitle"]["title"];
    assert_eq!(title["qualityProfileId"], "anime-hd");
    assert_eq!(title["rootFolderId"], root_folder_id);
    assert_eq!(title["rootFolderPath"], "/library/anime-custom");
    assert_eq!(title["monitorType"], "FUTURE_EPISODES");
    assert_eq!(title["useSeasonFolders"], false);
    assert_eq!(title["monitorSpecials"], true);
    assert_eq!(title["interSeasonMovies"], false);
    assert_eq!(title["fillerPolicy"], "SKIP_FILLER");
    assert_eq!(title["recapPolicy"], "SKIP_RECAP");
    let title_id = title["id"].as_str().expect("title id");
    let stored_title = ctx
        .titles
        .get_by_id(title_id)
        .await
        .expect("title should load")
        .expect("title should exist");
    assert_eq!(stored_title.root_folder_id, root_folder_id);
    assert!(
        !stored_title
            .tags
            .iter()
            .any(|tag| tag.starts_with("scryer:root-folder:"))
    );
}

#[tokio::test]
async fn graphql_title_effective_anime_policies_inherit_defaults() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Inherited Anime Policies",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;

    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                fillerPolicy
                recapPolicy
                effectiveFillerPolicy
                effectiveRecapPolicy
            }
        }"#,
        json!({ "id": title.id }),
    )
    .await;

    assert_no_errors(&body);
    let title = &body["data"]["title"];
    assert!(title["fillerPolicy"].is_null());
    assert!(title["recapPolicy"].is_null());
    assert_eq!(title["effectiveFillerPolicy"], "DOWNLOAD_ALL");
    assert_eq!(title["effectiveRecapPolicy"], "DOWNLOAD_ALL");
}

#[tokio::test]
async fn graphql_movie_required_audio_override_resolves_and_clears_to_facet_default() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let title_id = add_test_title(&ctx, "Movie Audio Override", "MOVIE").await;

    let default_audio = gql(
        &ctx,
        r#"mutation($input: UpdateMediaSettingsInput!) {
            updateMediaSettings(input: $input) { requiredAudioLanguages }
        }"#,
        json!({
            "input": {
                "scope": "MOVIE",
                "requiredAudioLanguages": ["Original"]
            }
        }),
    )
    .await;
    assert_no_errors(&default_audio);
    assert_eq!(
        default_audio["data"]["updateMediaSettings"]["requiredAudioLanguages"],
        json!(["original"])
    );

    let set_override = gql(
        &ctx,
        r#"mutation($input: SetTitleRequiredAudioInput!) {
            setTitleRequiredAudio(input: $input) {
                titleId
                facet
                languages
                updated
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "facet": "MOVIE",
                "languages": ["jpn"]
            }
        }),
    )
    .await;
    assert_no_errors(&set_override);
    assert_eq!(
        set_override["data"]["setTitleRequiredAudio"]["facet"],
        "MOVIE"
    );
    assert_eq!(
        set_override["data"]["setTitleRequiredAudio"]["languages"],
        json!(["jpn"])
    );

    let title = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                requiredAudioLanguagesOverride
                effectiveRequiredAudioLanguages
                inheritsRequiredAudioLanguages
            }
        }"#,
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&title);
    assert_eq!(
        title["data"]["title"]["requiredAudioLanguagesOverride"],
        json!(["jpn"])
    );
    assert_eq!(
        title["data"]["title"]["effectiveRequiredAudioLanguages"],
        json!(["jpn"])
    );
    assert_eq!(
        title["data"]["title"]["inheritsRequiredAudioLanguages"],
        false
    );

    let clear_override = gql(
        &ctx,
        r#"mutation($input: SetTitleRequiredAudioInput!) {
            setTitleRequiredAudio(input: $input) { updated }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "facet": "MOVIE",
                "languages": null
            }
        }),
    )
    .await;
    assert_no_errors(&clear_override);

    let inherited_title = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                requiredAudioLanguagesOverride
                effectiveRequiredAudioLanguages
                inheritsRequiredAudioLanguages
            }
        }"#,
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&inherited_title);
    assert!(inherited_title["data"]["title"]["requiredAudioLanguagesOverride"].is_null());
    assert_eq!(
        inherited_title["data"]["title"]["effectiveRequiredAudioLanguages"],
        json!(["original"])
    );
    assert_eq!(
        inherited_title["data"]["title"]["inheritsRequiredAudioLanguages"],
        true
    );
}

#[tokio::test]
async fn graphql_title_required_audio_inherits_original_from_library() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Original Audio Library",
        &[("/library/original-audio", true)],
    )
    .await;
    let library_id = library_id(&library).to_string();
    ctx.settings_store
        .upsert_setting_json(
            "system",
            "audio.required_languages",
            Some(library_id.clone()),
            json!(["original"]).to_string(),
            "test",
            None,
        )
        .await
        .expect("set library required audio languages");

    let title = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title {
                    effectiveRequiredAudioLanguages
                    inheritsRequiredAudioLanguages
                }
            }
        }"#,
        json!({
            "input": {
                "name": "Library Original Audio Movie",
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "923456" }]
            }
        }),
    )
    .await;
    assert_no_errors(&title);
    let title = &title["data"]["addTitle"]["title"];
    assert_eq!(
        title["effectiveRequiredAudioLanguages"],
        json!(["original"])
    );
    assert_eq!(title["inheritsRequiredAudioLanguages"], true);
}

#[tokio::test]
async fn graphql_movie_rejects_season_folder_options_and_ignores_legacy_tags() {
    let ctx = TestContext::new().await;
    let rejected_add = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) { addTitle(input: $input) { title { id } } }"#,
        json!({
            "input": {
                "name": "Movie season folders must fail",
                "facet": "MOVIE",
                "monitored": true,
                "tags": [],
                "options": { "useSeasonFolders": false }
            }
        }),
    )
    .await;
    assert!(
        rejected_add
            .to_string()
            .contains("useSeasonFolders is only valid for series and anime titles"),
        "Movie season-folder option should be rejected: {rejected_add}"
    );

    let legacy = create_catalog_title(
        &ctx,
        "Legacy Movie season folders",
        MediaFacet::Movie,
        vec![],
        vec!["scryer:season-folder:disabled".to_string()],
        true,
    )
    .await;
    let body = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                useSeasonFolders
                useSeasonFoldersOverride
                effectiveUseSeasonFolders
                inheritsUseSeasonFolders
            }
        }"#,
        json!({ "id": legacy.id }),
    )
    .await;
    assert_no_errors(&body);
    let title = &body["data"]["title"];
    assert!(title["useSeasonFolders"].is_null());
    assert!(title["useSeasonFoldersOverride"].is_null());
    assert_eq!(title["effectiveUseSeasonFolders"], true);
    assert_eq!(title["inheritsUseSeasonFolders"], true);

    let rejected_update = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) { updateTitle(input: $input) { id } }"#,
        json!({
            "input": {
                "titleId": legacy.id,
                "options": { "useSeasonFolders": false }
            }
        }),
    )
    .await;
    assert!(
        rejected_update
            .to_string()
            .contains("useSeasonFolders is only valid for series and anime titles"),
        "Movie season-folder update should be rejected: {rejected_update}"
    );
}

#[tokio::test]
async fn graphql_add_title_root_folder_id_validates_library_and_infers_library() {
    let ctx = TestContext::new().await;
    let movie_library_a = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Movie Library A",
        &[
            ("/library/movies-a-default", true),
            ("/library/movies-a-custom", false),
        ],
    )
    .await;
    let movie_library_b = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Movie Library B",
        &[
            ("/library/movies-b-default", true),
            ("/library/movies-b-custom", false),
        ],
    )
    .await;
    let library_a_id = library_id(&movie_library_a);
    let library_b_root_id = library_root_id(&movie_library_b, "/library/movies-b-custom");
    let library_a_root_id = library_root_id(&movie_library_a, "/library/movies-a-custom");

    let inferred = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title {
                    id
                    libraryId
                    rootFolderId
                    rootFolderPath
                }
            }
        }"#,
        json!({
            "input": {
                "name": "Inferred Library Movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": library_a_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&inferred);
    let title = &inferred["data"]["addTitle"]["title"];
    assert_eq!(title["libraryId"], library_a_id);
    assert_eq!(title["rootFolderId"], library_a_root_id);
    assert_eq!(title["rootFolderPath"], "/library/movies-a-custom");

    let mismatched = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": "Mismatched Root Movie",
                "facet": "MOVIE",
                "libraryId": library_a_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": library_b_root_id
                }
            }
        }),
    )
    .await;
    assert!(
        mismatched.get("errors").is_some(),
        "root folder from another library should fail: {mismatched}"
    );
}

#[tokio::test]
async fn graphql_add_title_returns_async_hydration_payload_fields() {
    let ctx = TestContext::new().await;
    let query = r#"mutation($input: AddTitleInput!) {
        addTitle(input: $input) {
            metadataHydrationState
            reusedExistingTitle
            reusedQueuedDownload
            title {
                id
                name
            }
        }
    }"#;
    let variables = json!({
        "input": {
            "name": "Async Payload Movie",
            "facet": "MOVIE",
            "monitored": true,
            "tags": [],
            "externalIds": [{ "source": "tvdb", "value": "123456" }]
        }
    });

    let first = gql(&ctx, query, variables.clone()).await;
    assert_no_errors(&first);
    assert_eq!(
        first["data"]["addTitle"]["metadataHydrationState"],
        "PENDING"
    );
    assert_eq!(first["data"]["addTitle"]["reusedExistingTitle"], false);
    assert_eq!(first["data"]["addTitle"]["reusedQueuedDownload"], false);

    let second = gql(&ctx, query, variables).await;
    assert_no_errors(&second);
    assert_eq!(
        second["data"]["addTitle"]["metadataHydrationState"],
        "PENDING"
    );
    assert_eq!(second["data"]["addTitle"]["reusedExistingTitle"], true);
    assert_eq!(second["data"]["addTitle"]["reusedQueuedDownload"], false);
    assert_eq!(
        second["data"]["addTitle"]["title"]["id"],
        first["data"]["addTitle"]["title"]["id"]
    );
}

#[tokio::test]
async fn graphql_add_movie_accepts_smg_and_tmdb_identity_without_tvdb() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                metadataHydrationState
                title { externalIds { source value } }
            }
        }"#,
        json!({
            "input": {
                "name": "SMG and TMDB Identity Movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": [],
                "externalIds": [],
                "smgId": 202,
                "tmdbId": 2020
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["addTitle"]["metadataHydrationState"],
        "PENDING"
    );
    assert_eq!(
        body["data"]["addTitle"]["title"]["externalIds"],
        json!([
            { "source": "smg", "value": "202" },
            { "source": "tmdb", "value": "2020" },
        ])
    );
}

/// A series holds every external id it was added with. Search subjects, RSS
/// candidate indexes and notification payloads read imdb/tmdb/smg ids without
/// checking the facet, so `addTitle` must store them for a series exactly as it
/// does for a movie, and `externalIds` must read them all back.
#[tokio::test]
async fn graphql_add_series_keeps_every_identity_from_search_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { externalIds { source value } }
            }
        }"#,
        json!({
            "input": {
                "name": "Series Search Result",
                "facet": "SERIES",
                "monitored": true,
                "tags": [],
                "externalIds": [
                    { "source": "smg", "value": "202" },
                    { "source": "tmdb", "value": "2020" },
                    { "source": "imdb", "value": "tt0202020" },
                    { "source": "tvdb", "value": "12345" }
                ],
                "smgId": 202,
                "tvdbId": "12345",
                "tmdbId": 2020,
                "imdbId": "tt0202020"
            }
        }),
    )
    .await;

    assert_no_errors(&body);
    assert_eq!(
        body["data"]["addTitle"]["title"]["externalIds"],
        json!([
            { "source": "smg", "value": "202" },
            { "source": "tmdb", "value": "2020" },
            { "source": "imdb", "value": "tt0202020" },
            { "source": "tvdb", "value": "12345" },
        ])
    );
}

/// The e2e harness verifies an added series by reading its imdb id back out of
/// `titles { items { externalIds } }`. A series added with imdb + tvdb must
/// therefore store and list both, not just the tvdb id.
#[tokio::test]
async fn graphql_added_series_lists_both_its_imdb_and_tvdb_ids() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id externalIds { source value } }
            }
        }"#,
        json!({
            "input": {
                "name": "Quiet Meridian",
                "facet": "SERIES",
                "monitored": true,
                "tags": [],
                "tvdbId": "770001",
                "imdbId": "tt0770001"
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    let title_id = body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("added series id")
        .to_string();

    let listed = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { items { id externalIds { source value } } } }"#,
        json!({ "facet": "SERIES" }),
    )
    .await;
    assert_no_errors(&listed);
    let item = listed["data"]["titles"]["items"]
        .as_array()
        .expect("titles items")
        .iter()
        .find(|item| item["id"] == title_id.as_str())
        .expect("added series in titles readback")
        .clone();
    let mut external_ids = item["externalIds"]
        .as_array()
        .expect("externalIds")
        .iter()
        .map(|external_id| {
            (
                external_id["source"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                external_id["value"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    external_ids.sort();
    assert_eq!(
        external_ids,
        vec![
            ("imdb".to_string(), "tt0770001".to_string()),
            ("tvdb".to_string(), "770001".to_string()),
        ],
        "a series keeps both its imdb and tvdb ids through the externalIds readback"
    );
}

/// `addTitle` accepted an identity-less movie before the title-id surface
/// existed: the title simply parks unhydrated until an identity arrives.
/// Teaching the mutation to reject one would be a non-additive change to an
/// operation integrations already call.
#[tokio::test]
async fn graphql_add_movie_without_an_identity_parks_unhydrated() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                metadataHydrationState
                title { id name externalIds { source value } }
            }
        }"#,
        json!({
            "input": {
                "name": "Identity-less Movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": []
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["addTitle"]["metadataHydrationState"],
        "NOT_REQUIRED"
    );
    assert_eq!(
        body["data"]["addTitle"]["title"]["name"],
        "Identity-less Movie"
    );
    assert_eq!(body["data"]["addTitle"]["title"]["externalIds"], json!([]));
}

#[tokio::test]
async fn graphql_reused_add_applies_explicit_options_and_preserves_omitted_ones() {
    let ctx = TestContext::new().await;
    // Creation is registry-gated, so the user label this fixture carries has to
    // be defined before a title can be born with it.
    define_title_tag(&ctx, "keep-me", None).await;
    let query = r#"mutation($input: AddTitleInput!) {
        addTitle(input: $input) {
            reusedExistingTitle
            title { id tags qualityProfileId monitorType }
        }
    }"#;
    let first = gql(
        &ctx,
        query,
        json!({
            "input": {
                "name": "Reusable options movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": ["keep-me"],
                "externalIds": [{ "source": "tvdb", "value": "130001" }],
                "options": {
                    "qualityProfileId": "4k",
                    "monitorType": "ALL_EPISODES"
                }
            }
        }),
    )
    .await;
    assert_no_errors(&first);
    assert_eq!(first["data"]["addTitle"]["reusedExistingTitle"], false);

    let second = gql(
        &ctx,
        query,
        json!({
            "input": {
                "name": "Reusable options movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "130001" }],
                "options": { "qualityProfileId": "1080p" }
            }
        }),
    )
    .await;
    assert_no_errors(&second);
    assert_eq!(second["data"]["addTitle"]["reusedExistingTitle"], true);
    assert_eq!(
        second["data"]["addTitle"]["title"]["id"],
        first["data"]["addTitle"]["title"]["id"]
    );
    let second_title = &second["data"]["addTitle"]["title"];
    assert_eq!(second_title["qualityProfileId"], "1080p");
    assert_eq!(second_title["monitorType"], "ALL_EPISODES");
    let tags = second_title["tags"].as_array().expect("tags array");
    assert!(tags.iter().any(|tag| tag == "keep-me"));
    assert!(tags.iter().any(|tag| tag == "scryer:quality-profile:1080p"));
    assert!(!tags.iter().any(|tag| tag == "scryer:quality-profile:4k"));

    let third = gql(
        &ctx,
        query,
        json!({
            "input": {
                "name": "Reusable options movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "130001" }]
            }
        }),
    )
    .await;
    assert_no_errors(&third);
    let third_title = &third["data"]["addTitle"]["title"];
    assert_eq!(third_title["qualityProfileId"], "1080p");
    assert_eq!(third_title["monitorType"], "ALL_EPISODES");

    let updated = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { qualityProfileId monitorSpecials }
        }"#,
        json!({
            "input": {
                "titleId": third_title["id"],
                "options": { "monitorSpecials": true }
            }
        }),
    )
    .await;
    assert_no_errors(&updated);
    assert_eq!(updated["data"]["updateTitle"]["qualityProfileId"], "1080p");
    assert_eq!(updated["data"]["updateTitle"]["monitorSpecials"], true);

    let cleared = gql(
        &ctx,
        query,
        json!({
            "input": {
                "name": "Reusable options movie",
                "facet": "MOVIE",
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "130001" }],
                "options": { "qualityProfileId": null }
            }
        }),
    )
    .await;
    assert_no_errors(&cleared);
    assert!(cleared["data"]["addTitle"]["title"]["qualityProfileId"].is_null());
    assert_eq!(
        cleared["data"]["addTitle"]["title"]["monitorType"],
        "ALL_EPISODES"
    );
}

#[tokio::test]
async fn graphql_reused_add_preserves_metadata_language_tri_state() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;
    let query = r#"mutation($input: AddTitleInput!) {
        addTitle(input: $input) {
            reusedExistingTitle
            title {
                id
                metadataLanguageOverride
                effectiveMetadataLanguage
                inheritsMetadataLanguage
            }
        }
    }"#;
    let input = |options: Value| {
        json!({
            "input": {
                "name": "Reusable metadata language series",
                "facet": "SERIES",
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "metadata-language-reuse" }],
                "options": options,
            }
        })
    };

    let set = gql(&ctx, query, input(json!({ "metadataLanguage": " FRA " }))).await;
    assert_no_errors(&set);
    assert_eq!(set["data"]["addTitle"]["reusedExistingTitle"], false);
    assert_eq!(
        set["data"]["addTitle"]["title"]["metadataLanguageOverride"],
        "fra"
    );

    let cleared = gql(&ctx, query, input(json!({ "metadataLanguage": null }))).await;
    assert_no_errors(&cleared);
    assert_eq!(cleared["data"]["addTitle"]["reusedExistingTitle"], true);
    assert!(cleared["data"]["addTitle"]["title"]["metadataLanguageOverride"].is_null());
    assert_eq!(
        cleared["data"]["addTitle"]["title"]["effectiveMetadataLanguage"],
        "eng"
    );
    assert_eq!(
        cleared["data"]["addTitle"]["title"]["inheritsMetadataLanguage"],
        true
    );

    let omitted = gql(&ctx, query, input(json!({}))).await;
    assert_no_errors(&omitted);
    assert!(omitted["data"]["addTitle"]["title"]["metadataLanguageOverride"].is_null());

    let reset = gql(&ctx, query, input(json!({ "metadataLanguage": "JPN" }))).await;
    assert_no_errors(&reset);
    assert_eq!(
        reset["data"]["addTitle"]["title"]["metadataLanguageOverride"],
        "jpn"
    );

    let preserved = gql(&ctx, query, input(json!({}))).await;
    assert_no_errors(&preserved);
    assert_eq!(
        preserved["data"]["addTitle"]["title"]["metadataLanguageOverride"],
        "jpn"
    );

    let rejected = gql(&ctx, query, input(json!({ "metadataLanguage": "rus" }))).await;
    assert!(
        rejected.to_string().contains(
            "metadataLanguage must be one of eng, spa, fra, deu, ita, por, kor, zho, or jpn"
        ),
        "invalid metadata language should be rejected: {rejected}"
    );
}

#[tokio::test]
async fn graphql_add_title_then_list() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Listed Movie", "MOVIE").await;

    let body = gql(&ctx, "{ titles { items { id name facet } } }", json!({})).await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"]["items"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["id"], title_id);
    assert!(
        titles[0]["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
    );
    assert_eq!(titles[0]["facet"], "MOVIE");
}

#[tokio::test]
async fn graphql_add_multiple_titles() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie One", "MOVIE").await;
    add_test_title(&ctx, "Series One", "SERIES").await;
    add_test_title(&ctx, "Anime One", "ANIME").await;

    let body = gql(&ctx, "{ titles { items { id facet } } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"]["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn graphql_titles_by_external_ids_returns_catalog_titles() {
    let ctx = TestContext::new().await;
    let first = create_catalog_title(
        &ctx,
        "Mario",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "18861".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let duplicate = create_catalog_title(
        &ctx,
        "Comet Duplicate",
        MediaFacet::Series,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "18861".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    let second = create_catalog_title(
        &ctx,
        "The Super Comet Galaxy Movie",
        MediaFacet::Movie,
        vec![ExternalId {
            source: "tvdb".to_string(),
            value: "354713".to_string(),
        }],
        vec![],
        true,
    )
    .await;
    seed_catalog_filter_metadata(
        &ctx,
        &second.id,
        &[("canonical:genre:family", "genre", "Family")],
        None,
    )
    .await;

    let body = gql(
        &ctx,
        r#"query($source: String!, $values: [String!]!) {
          titlesByExternalIds(source: $source, values: $values) {
            id
            name
            facet
            externalIds { source value }
            canonicalTags { key name }
          }
        }"#,
        json!({
            "source": "tvdb",
            "values": ["18861", "18861", "000000", "354713"]
        }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titlesByExternalIds"]
        .as_array()
        .expect("titles array");
    let mut expected_duplicate_ids = [first.id.as_str(), duplicate.id.as_str()];
    expected_duplicate_ids.sort_unstable();
    assert_eq!(titles.len(), 3);
    assert_eq!(titles[0]["id"].as_str(), Some(expected_duplicate_ids[0]));
    assert_eq!(titles[1]["id"].as_str(), Some(expected_duplicate_ids[1]));
    assert_eq!(titles[2]["id"].as_str(), Some(second.id.as_str()));
    assert_eq!(
        titles[2]["canonicalTags"],
        json!([{ "key": "canonical:genre:family", "name": "Family" }])
    );
}

#[tokio::test]
async fn graphql_titles_by_external_ids_filters_all_owner_copies_before_returning_matches() {
    let ctx = TestContext::new().await;
    let first_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "External ID Copy A",
        &[("/catalog-rbac/external-copy-a", true)],
    )
    .await;
    let second_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "External ID Copy B",
        &[("/catalog-rbac/external-copy-b", true)],
    )
    .await;
    let first_library_id = library_id(&first_library);
    let second_library_id = library_id(&second_library);
    let first_title_id = add_catalog_filter_title(
        &ctx,
        "External Identity Copy A",
        "99118861",
        &first_library_id,
        &library_root_id(&first_library, "/catalog-rbac/external-copy-a"),
        2024,
    )
    .await;
    let second_title_id = add_catalog_filter_title(
        &ctx,
        "External Identity Copy B",
        "99118861",
        &second_library_id,
        &library_root_id(&second_library, "/catalog-rbac/external-copy-b"),
        2024,
    )
    .await;

    let (unauthorized_title_id, authorized_title_id, authorized_library_id) =
        if first_title_id < second_title_id {
            (&first_title_id, &second_title_id, &second_library_id)
        } else {
            (&second_title_id, &first_title_id, &first_library_id)
        };
    assert!(unauthorized_title_id < authorized_title_id);

    let body = schema_exec(
        &ctx,
        r#"query {
            titlesByExternalIds(source: "tvdb", values: ["99118861"]) { id }
        }"#,
        Some(catalog_view_actor(authorized_library_id)),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["titlesByExternalIds"],
        json!([{ "id": authorized_title_id }])
    );
}

#[tokio::test]
async fn graphql_titles_are_sorted_by_display_name() {
    let ctx = TestContext::new().await;
    create_catalog_title(&ctx, "zeta movie", MediaFacet::Movie, vec![], vec![], true).await;
    create_catalog_title(&ctx, "Alpha Movie", MediaFacet::Movie, vec![], vec![], true).await;
    create_catalog_title(&ctx, "beta movie", MediaFacet::Movie, vec![], vec![], true).await;

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { items { name } } }"#,
        json!({ "facet": "MOVIE" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"]["items"].as_array().unwrap();
    let names: Vec<&str> = titles
        .iter()
        .map(|title| title["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Alpha Movie", "beta movie", "zeta movie"]);
}

#[tokio::test]
async fn graphql_titles_expose_episode_progress_excluding_specials() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Episode Progress Show",
        MediaFacet::Series,
        vec![],
        vec![],
        false,
    )
    .await;

    let season_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("3".to_string()),
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let specials_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");

    let season_zero_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "0".to_string(),
            label: Some("Season 0".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season zero collection");

    let regular_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "1", "S01E01").await;
    let mut regular_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "2", "S01E02").await;
    let regular_episode_3 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "3", "S01E03").await;
    let special_episode_1 =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "1", "S00E01").await;
    let _special_episode_2 =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "2", "S00E02").await;
    let season_zero_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_zero_collection, "0", "3", "S00E03").await;
    let _season_zero_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_zero_collection, "0", "4", "S00E04").await;

    regular_episode_2 = ctx
        .shows
        .update_episode(
            &regular_episode_2.id,
            EpisodeUpdate {
                air_date: Some("2024-01-08".to_string()),
                monitored: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update episode monitored flag");

    let regular_episode_1 = ctx
        .shows
        .update_episode(
            &regular_episode_1.id,
            EpisodeUpdate {
                air_date: Some("2024-01-01".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update first regular episode air date");

    ctx.shows
        .update_episode(
            &regular_episode_3.id,
            EpisodeUpdate {
                air_date: Some("2024-01-15".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update third regular episode air date");

    for (index, episode) in [
        regular_episode_1,
        regular_episode_2,
        special_episode_1,
        season_zero_episode_1,
    ]
    .into_iter()
    .enumerate()
    {
        let file_path = media_root
            .path()
            .join(format!("Episode.Progress.Show.file-{index}.mkv"));
        let file_id = ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path.to_string_lossy().to_string(),
                size_bytes: 4_096 + index as i64,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
            .await
            .expect("link file to episode");
    }

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { items { id name episodesOwned episodesMonitored episodesTotal } } }"#,
        json!({ "facet": "SERIES" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"]["items"]
        .as_array()
        .expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("series title should be listed");

    assert_eq!(listed_title["name"], "Episode Progress Show");
    assert_eq!(listed_title["episodesOwned"], 2);
    assert_eq!(listed_title["episodesMonitored"], 2);
    assert_eq!(listed_title["episodesTotal"], 3);
}

#[tokio::test]
async fn graphql_titles_exclude_tba_or_incomplete_metadata_episodes_from_progress_counts() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Progress Count Filter Show",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
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
            last_episode_number: Some("4".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let countable_episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create countable episode");

    let tba_episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("TBA".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create tba episode");

    let untitled_episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("3".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E03".to_string()),
            title: None,
            air_date: Some("2024-01-15".to_string()),
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create untitled episode");

    let undated_episode = ctx
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_collection.id.clone()),
            episode_type: EpisodeType::Standard,
            episode_number: Some("4".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E04".to_string()),
            title: Some("Named but undated".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: false,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create undated episode");

    for (index, episode) in [
        countable_episode.clone(),
        tba_episode,
        untitled_episode,
        undated_episode,
    ]
    .into_iter()
    .enumerate()
    {
        let file_path = media_root
            .path()
            .join(format!("Progress.Count.Filter.Show.file-{index}.mkv"));
        let file_id = ctx
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: file_path.to_string_lossy().to_string(),
                size_bytes: 8_192 + index as i64,
                quality_label: Some("1080p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        ctx.link_primary_file_to_episode(&title.id, &file_id, &episode.id)
            .await
            .expect("link file to episode");
    }

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { items { id name episodesOwned episodesMonitored episodesTotal } } }"#,
        json!({ "facet": "SERIES" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"]["items"]
        .as_array()
        .expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("series title should be listed");

    assert_eq!(listed_title["name"], "Progress Count Filter Show");
    assert_eq!(listed_title["episodesOwned"], 1);
    assert_eq!(listed_title["episodesMonitored"], 1);
    assert_eq!(listed_title["episodesTotal"], 1);
}

#[tokio::test]
async fn graphql_titles_expose_matched_size_bytes_only_for_anime_titles() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Matched Size Anime",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;

    let season_collection = ctx
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
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season collection");

    let specials_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Specials,
            collection_index: "0".to_string(),
            label: Some("Specials".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create specials collection");

    let season_zero_collection = ctx
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "0".to_string(),
            label: Some("Season 0".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create season zero collection");

    let series_movie_path = media_root
        .path()
        .join("Matched.Size.Anime.Series.Movie.1080p.mkv");
    let series_movie_link =
        create_test_series_movie_link(&ctx, &title, "Matched Size Movie", "7654303", None, None)
            .await;

    let regular_episode_1 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "1", "S01E01").await;
    let regular_episode_2 =
        create_series_scan_episode(&ctx, &title, &season_collection, "1", "2", "S01E02").await;
    let special_episode =
        create_series_scan_episode(&ctx, &title, &specials_collection, "0", "1", "S00E01").await;
    let season_zero_episode =
        create_series_scan_episode(&ctx, &title, &season_zero_collection, "0", "2", "S00E02").await;

    let multi_episode_path = media_root.path().join("Matched.Size.Anime.S01E01-E02.mkv");
    let multi_episode_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: multi_episode_path.to_string_lossy().to_string(),
            size_bytes: 1_000,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert multi-episode file");
    for episode_id in [&regular_episode_1.id, &regular_episode_2.id] {
        ctx.media_files
            .link_file_to_episode(&multi_episode_file_id, episode_id)
            .await
            .expect("link multi-episode file");
    }

    let special_path = media_root.path().join("Matched.Size.Anime.Special.mkv");
    let special_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: special_path.to_string_lossy().to_string(),
            size_bytes: 200,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert special file");
    ctx.media_files
        .link_file_to_episode(&special_file_id, &special_episode.id)
        .await
        .expect("link special file");

    let season_zero_path = media_root.path().join("Matched.Size.Anime.Season.Zero.mkv");
    let season_zero_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: season_zero_path.to_string_lossy().to_string(),
            size_bytes: 300,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert season zero file");
    ctx.media_files
        .link_file_to_episode(&season_zero_file_id, &season_zero_episode.id)
        .await
        .expect("link season zero file");

    let series_movie_file_id = ctx
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: series_movie_path.to_string_lossy().to_string(),
            size_bytes: 400,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert series movie file");
    ctx.media_files
        .link_file_to_series_movie(&series_movie_file_id, &series_movie_link.id)
        .await
        .expect("link series movie file");

    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: media_root
                .path()
                .join("Matched.Size.Anime.Unmatched.Extra.mkv")
                .to_string_lossy()
                .to_string(),
            size_bytes: 500,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert unmatched file");

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { items { id name sizeBytes } } }"#,
        json!({ "facet": "ANIME" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"]["items"]
        .as_array()
        .expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("anime title should be listed");

    assert_eq!(listed_title["name"], "Matched Size Anime");
    assert_eq!(listed_title["sizeBytes"], json!(1_900));

    let overview = gql(
        &ctx,
        r#"
        query($titleId: ID!) {
	          title(id: $titleId) {
	            mediaFiles {
	              id
	              sizeBytes
	              seriesMovieLinkIds
	            }
	          }
        }
        "#,
        json!({ "titleId": title.id }),
    )
    .await;
    assert_no_errors(&overview);
    let series_movie_file = overview["data"]["title"]["mediaFiles"]
        .as_array()
        .expect("media files array")
        .iter()
        .find(|file| file["id"] == series_movie_file_id)
        .expect("series movie file in title media files");
    assert_eq!(
        series_movie_file["seriesMovieLinkIds"],
        json!([series_movie_link.id])
    );
    assert_eq!(series_movie_file["sizeBytes"], json!(400));
}

#[tokio::test]
async fn graphql_movie_entity_ratings_and_credits_are_read_from_local_storage() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Locally Hydrated Series Movie",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;
    let link =
        create_test_series_movie_link(&ctx, &title, "Cached Cast Movie", "7654304", None, None)
            .await;

    let body = gql(
        &ctx,
        r#"
        query($titleId: ID!, $movieId: ID!) {
          title(id: $titleId) {
            seriesMovieLinks {
              id
              movie {
                id
                ratings {
                  rating
                  ratingSources
                  externalRatings { source normalized votes url }
                }
              }
            }
          }
          movieEntity(titleId: $titleId, id: $movieId) {
            id
            credits {
              kind
              personName
              personImageUrl
              character
              language
            }
          }
        }
        "#,
        json!({ "titleId": title.id, "movieId": link.movie.id }),
    )
    .await;
    assert_no_errors(&body);

    let listed = &body["data"]["title"]["seriesMovieLinks"][0];
    assert_eq!(listed["id"], link.id);
    assert_eq!(listed["movie"]["ratings"]["rating"], json!(8.7));
    assert_eq!(listed["movie"]["ratings"]["ratingSources"], json!(["tmdb"]));
    assert_eq!(
        listed["movie"]["ratings"]["externalRatings"][0]["votes"],
        json!(1_234)
    );

    let focused = &body["data"]["movieEntity"];
    assert_eq!(focused["id"], link.movie.id);
    assert_eq!(focused["credits"][0]["personName"], "Fixture Performer");
    assert_eq!(focused["credits"][0]["character"], "Fixture Character");
    let image_url = focused["credits"][0]["personImageUrl"]
        .as_str()
        .expect("movie credit portrait proxy URL");
    let token = image_url
        .strip_prefix("/images/media/")
        .and_then(|value| value.strip_suffix("/w185"))
        .expect("movie credit portraits should use the local media route");
    assert!(!image_url.contains("images.example.com"));

    let persisted: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT owner_type, owner_id FROM image_proxy_sources WHERE token = ?")
            .bind(token)
            .fetch_one(ctx.db.pool())
            .await
            .expect("movie portrait source should persist");
    assert_eq!(
        persisted,
        (Some("movie".to_string()), Some(link.movie.id.clone()))
    );

    let mut unavailable = link.clone();
    unavailable.movie.ratings = None;
    unavailable.movie.credits = None;
    let preserved = ctx
        .shows
        .upsert_series_movie_link(unavailable)
        .await
        .expect("unavailable enrichment should preserve movie metadata");
    assert_eq!(
        preserved
            .movie
            .ratings
            .as_ref()
            .and_then(|ratings| ratings.rating),
        Some(8.7)
    );
    assert_eq!(
        ctx.shows
            .list_movie_entity_credits(&link.movie.id)
            .await
            .expect("preserved movie credits")[0]
            .person_name,
        "Fixture Performer"
    );

    let mut cleared = link.clone();
    cleared.movie.ratings = Some(scryer_domain::TitleRatingSummary::default());
    cleared.movie.credits = Some(vec![]);
    let cleared = ctx
        .shows
        .upsert_series_movie_link(cleared)
        .await
        .expect("empty enrichment should clear movie metadata");
    assert!(cleared.movie.ratings.is_none());
    assert!(
        ctx.shows
            .list_movie_entity_credits(&link.movie.id)
            .await
            .expect("cleared movie credits")
            .is_empty()
    );

    let invalid_owner = sqlx::query(
        "INSERT INTO title_credits (
             title_id, movie_entity_id, position, kind, person_id
         ) VALUES (?, ?, 999, 'actor', 'invalid-owner')",
    )
    .bind(&title.id)
    .bind(&link.movie.id)
    .execute(ctx.db.pool())
    .await;
    assert!(
        invalid_owner.is_err(),
        "credit rows must have exactly one owner"
    );

    sqlx::query("DELETE FROM movie_entities WHERE id = ?")
        .bind(&link.movie.id)
        .execute(ctx.db.pool())
        .await
        .expect("delete movie entity");
    let remaining_metadata: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM title_metadata_rating_summaries WHERE movie_entity_id = ?),
            (SELECT COUNT(*) FROM title_credits WHERE movie_entity_id = ?)",
    )
    .bind(&link.movie.id)
    .bind(&link.movie.id)
    .fetch_one(ctx.db.pool())
    .await
    .expect("count cascaded movie metadata");
    assert_eq!(remaining_metadata, (0, 0));
}

#[tokio::test]
async fn batch_series_movie_links_keep_shared_entity_ratings() {
    let ctx = TestContext::new().await;
    let first_title = create_catalog_title(
        &ctx,
        "First Crossover Series",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;
    let second_title = create_catalog_title(
        &ctx,
        "Second Crossover Series",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;
    let first_link = create_test_series_movie_link(
        &ctx,
        &first_title,
        "Shared Crossover Movie",
        "7654305",
        None,
        None,
    )
    .await;
    let second_link = create_test_series_movie_link(
        &ctx,
        &second_title,
        "Shared Crossover Movie",
        "7654305",
        None,
        None,
    )
    .await;
    assert_eq!(first_link.movie.id, second_link.movie.id);

    let links = ctx
        .shows
        .list_series_movie_links_for_titles(&[first_title.id, second_title.id])
        .await
        .expect("batch series movie links");
    assert_eq!(links.len(), 2);
    assert!(links.iter().all(|link| {
        link.movie
            .ratings
            .as_ref()
            .and_then(|ratings| ratings.rating)
            == Some(8.7)
    }));
}

#[tokio::test]
async fn graphql_titles_expose_matched_size_bytes_only_for_movies() {
    let ctx = TestContext::new().await;
    let media_root = tempfile::tempdir().expect("media root tempdir");

    let title = create_catalog_title(
        &ctx,
        "Matched Size Movie",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;

    let matched_path = media_root.path().join("Matched.Size.Movie.2160p.mkv");
    ctx.shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("Matched Size Movie".to_string()),
            ordered_path: Some(matched_path.to_string_lossy().to_string()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        })
        .await
        .expect("create movie collection");

    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: matched_path.to_string_lossy().to_string(),
            size_bytes: 1_200,
            quality_label: Some("2160p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert matched movie file");

    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: media_root
                .path()
                .join("Matched.Size.Movie.Unmatched.Extra.mkv")
                .to_string_lossy()
                .to_string(),
            size_bytes: 700,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert unmatched movie file");

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { managedBytes items { id name sizeBytes } } }"#,
        json!({ "facet": "MOVIE" }),
    )
    .await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"]["items"]
        .as_array()
        .expect("titles array");
    let listed_title = titles
        .iter()
        .find(|item| item["id"] == title.id)
        .expect("movie title should be listed");

    assert_eq!(listed_title["name"], "Matched Size Movie");
    assert_eq!(listed_title["sizeBytes"], json!(1_200));
    assert_eq!(body["data"]["titles"]["managedBytes"], json!(1_200));
}

#[tokio::test]
async fn graphql_get_title_by_id() {
    let ctx = TestContext::new().await;
    let expected_name = "Specific Movie";
    let id = add_test_title(&ctx, expected_name, "MOVIE").await;

    let body = gql(
        &ctx,
        r#"query($id: ID!) { title(id: $id) { id name monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["title"]["name"], expected_name);
    assert_eq!(body["data"]["title"]["monitored"], true);
}

#[tokio::test]
async fn graphql_get_title_not_found() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($id: ID!) { title(id: $id) { id name } }"#,
        json!({ "id": "nonexistent-id" }),
    )
    .await;
    assert!(
        body["data"]["title"].is_null(),
        "should return null for nonexistent title"
    );
}

#[tokio::test]
async fn graphql_set_title_monitored() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Monitor Test", "MOVIE").await;

    // Disable monitoring
    let body = gql(
        &ctx,
        r#"mutation($input: SetTitleMonitoredInput!) {
            setTitleMonitored(input: $input) { id monitored }
        }"#,
        json!({ "input": { "titleId": id, "monitored": false } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["setTitleMonitored"]["monitored"], false);

    // Verify via query
    let body = gql(
        &ctx,
        r#"query($id: ID!) { title(id: $id) { monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_eq!(body["data"]["title"]["monitored"], false);
}

#[tokio::test]
async fn graphql_update_title_structured_options_merge_with_existing_tags() {
    let ctx = TestContext::new().await;
    seed_title_quality_profiles(&ctx, &["anime-4k"]).await;
    // Creation is registry-gated, so the user label this fixture carries has to
    // be defined before a title can be born with it.
    define_title_tag(&ctx, "favorite", None).await;
    let library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Option Update Anime Library",
        &[
            ("/library/option-anime-default", true),
            ("/library/option-anime-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let root_folder_id = library_root_id(&library, "/library/option-anime-custom");
    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id }
            }
        }"#,
        json!({
            "input": {
                "name": "Option Update Anime",
                "facet": "ANIME",
                "libraryId": library_id,
                "monitored": true,
                "tags": ["favorite"]
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title_id = add_body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("title id")
        .to_string();

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                id
                tags
                qualityProfileId
                rootFolderId
                rootFolderPath
                useSeasonFolders
                fillerPolicy
                recapPolicy
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "qualityProfileId": "anime-4k",
                    "rootFolderId": root_folder_id,
                    "useSeasonFolders": false,
                    "fillerPolicy": "SKIP_FILLER",
                    "recapPolicy": null
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let updated = &body["data"]["updateTitle"];
    assert_eq!(updated["qualityProfileId"], "anime-4k");
    assert_eq!(updated["rootFolderId"], root_folder_id);
    assert_eq!(updated["rootFolderPath"], "/library/option-anime-custom");
    assert_eq!(updated["useSeasonFolders"], false);
    assert_eq!(updated["fillerPolicy"], "SKIP_FILLER");
    assert!(updated["recapPolicy"].is_null());

    let tags = updated["tags"].as_array().expect("tags array");
    let tag_values: Vec<&str> = tags.iter().filter_map(|tag| tag.as_str()).collect();
    assert!(tag_values.contains(&"favorite"));
    assert!(tag_values.contains(&"scryer:quality-profile:anime-4k"));
    assert!(tag_values.contains(&"scryer:season-folder:disabled"));
    assert!(tag_values.contains(&"scryer:filler-policy:skip_filler"));
    assert!(
        !tag_values
            .iter()
            .any(|tag| tag.starts_with("scryer:root-folder:"))
    );
    assert!(
        !tag_values
            .iter()
            .any(|tag| tag.starts_with("scryer:recap-policy:"))
    );
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, root_folder_id);
}

#[tokio::test]
async fn graphql_update_title_root_folder_id_validates_and_defaults() {
    let ctx = TestContext::new().await;
    let anime_library_a = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Root Update Anime Library A",
        &[
            ("/library/root-update-a-default", true),
            ("/library/root-update-a-custom", false),
        ],
    )
    .await;
    let anime_library_b = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Root Update Anime Library B",
        &[
            ("/library/root-update-b-default", true),
            ("/library/root-update-b-custom", false),
        ],
    )
    .await;
    let library_a_id = library_id(&anime_library_a);
    let default_root_id = library_root_id(&anime_library_a, "/library/root-update-a-default");
    let custom_root_id = library_root_id(&anime_library_a, "/library/root-update-a-custom");
    let other_library_root_id = library_root_id(&anime_library_b, "/library/root-update-b-custom");

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title {
                    id
                    rootFolderId
                    rootFolderPath
                }
            }
        }"#,
        json!({
            "input": {
                "name": "Root Update Anime",
                "facet": "ANIME",
                "libraryId": library_a_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": custom_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let added = &add_body["data"]["addTitle"]["title"];
    assert_eq!(added["rootFolderId"], custom_root_id);
    assert_eq!(added["rootFolderPath"], "/library/root-update-a-custom");
    let title_id = added["id"].as_str().expect("title id").to_string();
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, custom_root_id);

    let unknown = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { id }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": "missing-root"
                }
            }
        }),
    )
    .await;
    assert!(
        unknown.get("errors").is_some(),
        "unknown root folder id should fail: {unknown}"
    );

    let other_library = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { id }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": other_library_root_id
                }
            }
        }),
    )
    .await;
    assert!(
        other_library.get("errors").is_some(),
        "root folder id from another library should fail: {other_library}"
    );

    let cleared_by_default = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                libraryId
                rootFolderId
                rootFolderPath
                tags
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": default_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&cleared_by_default);
    let cleared = &cleared_by_default["data"]["updateTitle"];
    assert_eq!(cleared["libraryId"], library_a_id);
    assert_eq!(cleared["rootFolderId"], default_root_id);
    assert_eq!(cleared["rootFolderPath"], "/library/root-update-a-default");
    let cleared_tags = cleared["tags"].as_array().expect("tags array");
    assert!(
        !cleared_tags
            .iter()
            .filter_map(|tag| tag.as_str())
            .any(|tag| tag.starts_with("scryer:root-folder:"))
    );
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, default_root_id);

    let reset = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { libraryId rootFolderId rootFolderPath }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": custom_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&reset);
    assert_eq!(reset["data"]["updateTitle"]["libraryId"], library_a_id);
    assert_eq!(
        reset["data"]["updateTitle"]["rootFolderPath"],
        "/library/root-update-a-custom"
    );
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, custom_root_id);

    let cleared_by_null = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                libraryId
                rootFolderId
                rootFolderPath
                tags
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": null
                }
            }
        }),
    )
    .await;
    assert_no_errors(&cleared_by_null);
    let cleared = &cleared_by_null["data"]["updateTitle"];
    assert_eq!(cleared["libraryId"], library_a_id);
    assert_eq!(cleared["rootFolderId"], default_root_id);
    assert_eq!(cleared["rootFolderPath"], "/library/root-update-a-default");
    let cleared_tags = cleared["tags"].as_array().expect("tags array");
    assert!(
        !cleared_tags
            .iter()
            .filter_map(|tag| tag.as_str())
            .any(|tag| tag.starts_with("scryer:root-folder:"))
    );
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, default_root_id);
}

#[tokio::test]
async fn graphql_update_title_root_folder_id_omitted_preserves_override() {
    let ctx = TestContext::new().await;
    seed_title_quality_profiles(&ctx, &["anime-preserve-hd"]).await;
    let library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Root Preserve Anime Library",
        &[
            ("/library/root-preserve-default", true),
            ("/library/root-preserve-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let custom_root_id = library_root_id(&library, "/library/root-preserve-custom");

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id }
            }
        }"#,
        json!({
            "input": {
                "name": "Root Preserve Anime",
                "facet": "ANIME",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": custom_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title_id = add_body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("title id")
        .to_string();

    let body = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                tags
                qualityProfileId
                rootFolderId
                rootFolderPath
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "qualityProfileId": "anime-preserve-hd"
                }
            }
        }),
    )
    .await;
    assert_no_errors(&body);

    let updated = &body["data"]["updateTitle"];
    assert_eq!(updated["qualityProfileId"], "anime-preserve-hd");
    assert_eq!(updated["rootFolderId"], custom_root_id);
    assert_eq!(updated["rootFolderPath"], "/library/root-preserve-custom");
    let tags = updated["tags"].as_array().expect("tags array");
    let tag_values: Vec<&str> = tags.iter().filter_map(|tag| tag.as_str()).collect();
    assert!(tag_values.contains(&"scryer:quality-profile:anime-preserve-hd"));
    assert!(
        !tag_values
            .iter()
            .any(|tag| tag.starts_with("scryer:root-folder:"))
    );
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, custom_root_id);
}

/// FR-077/SC-009: the retired direct root write. A title with tracked files
/// gets a typed refusal that routes the caller into the move workflow, while
/// re-submitting the root it already sits on is not a move and still passes.
#[tokio::test]
async fn graphql_update_title_root_folder_id_is_refused_for_tracked_file_titles() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Retired Root Write Library",
        &[
            ("/library/retired-root-default", true),
            ("/library/retired-root-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let default_root_id = library_root_id(&library, "/library/retired-root-default");
    let custom_root_id = library_root_id(&library, "/library/retired-root-custom");

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id rootFolderId }
            }
        }"#,
        json!({
            "input": {
                "name": "Retired Root Write Movie",
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": custom_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let added = &add_body["data"]["addTitle"]["title"];
    assert_eq!(added["rootFolderId"], custom_root_id);
    let title_id = added["id"].as_str().expect("title id").to_string();

    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.clone(),
            file_path: "/library/retired-root-custom/Retired Root Write Movie/movie.mkv"
                .to_string(),
            size_bytes: 1_200,
            ..Default::default()
        })
        .await
        .expect("insert tracked movie file");

    let refused = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { id rootFolderId }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": default_root_id
                }
            }
        }),
    )
    .await;
    let errors = refused["errors"]
        .as_array()
        .unwrap_or_else(|| panic!("a tracked-file root write must be refused: {refused}"));
    assert_eq!(errors[0]["extensions"]["code"], "DIRECT_ROOT_WRITE_RETIRED");
    assert_eq!(errors[0]["extensions"]["titleId"], title_id);
    let message = errors[0]["message"].as_str().expect("error message");
    assert!(
        message.contains("locationOperationPreview") && message.contains("startLocationOperation"),
        "refusal must name the move workflow: {message}"
    );
    let stored_root_folder_id = stored_title_root_folder_id(&ctx, &title_id).await;
    assert_eq!(stored_root_folder_id, custom_root_id);

    let unchanged = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { id rootFolderId }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": custom_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&unchanged);
    assert_eq!(
        unchanged["data"]["updateTitle"]["rootFolderId"],
        custom_root_id
    );
}

/// FR-077/SC-009 on the creation path's reuse branch. Reusing a title that
/// already exists is not creating it, so a root rewrite there is the same
/// direct write the move workflow replaces. Genuinely new titles, same-value
/// re-adds, and fileless reuse all stay on the direct path.
#[tokio::test]
async fn graphql_add_title_reuse_is_refused_when_it_would_change_a_tracked_titles_root() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Retired Reuse Root Library",
        &[
            ("/library/retired-reuse-default", true),
            ("/library/retired-reuse-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let default_root_id = library_root_id(&library, "/library/retired-reuse-default");
    let custom_root_id = library_root_id(&library, "/library/retired-reuse-custom");

    let add = r#"mutation($input: AddTitleInput!) {
        addTitle(input: $input) {
            reusedExistingTitle
            title { id rootFolderId }
        }
    }"#;
    let add_input = |name: &str, tvdb: &str, options: Value| {
        json!({
            "input": {
                "name": name,
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": tvdb }],
                "options": options
            }
        })
    };

    let first = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse Movie",
            "770077",
            json!({ "rootFolderId": custom_root_id }),
        ),
    )
    .await;
    assert_no_errors(&first);
    assert_eq!(first["data"]["addTitle"]["reusedExistingTitle"], false);
    let title_id = first["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("title id")
        .to_string();
    assert_eq!(
        first["data"]["addTitle"]["title"]["rootFolderId"],
        custom_root_id
    );

    ctx.media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.clone(),
            file_path: "/library/retired-reuse-custom/Retired Reuse Movie/movie.mkv".to_string(),
            size_bytes: 1_200,
            ..Default::default()
        })
        .await
        .expect("insert tracked movie file");

    let refused = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse Movie",
            "770077",
            json!({ "rootFolderId": default_root_id }),
        ),
    )
    .await;
    let errors = refused["errors"]
        .as_array()
        .unwrap_or_else(|| panic!("reuse must not rewrite a tracked title's root: {refused}"));
    assert_eq!(errors[0]["extensions"]["code"], "DIRECT_ROOT_WRITE_RETIRED");
    assert_eq!(errors[0]["extensions"]["titleId"], title_id);
    assert_eq!(
        stored_title_root_folder_id(&ctx, &title_id).await,
        custom_root_id
    );

    // Reuse that does not touch the root, and reuse that re-submits the root
    // the title already sits on, are both unaffected.
    let untouched = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse Movie",
            "770077",
            json!({ "monitorType": "ALL_EPISODES" }),
        ),
    )
    .await;
    assert_no_errors(&untouched);
    assert_eq!(untouched["data"]["addTitle"]["reusedExistingTitle"], true);
    assert_eq!(
        untouched["data"]["addTitle"]["title"]["rootFolderId"],
        custom_root_id
    );

    let same_value = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse Movie",
            "770077",
            json!({ "rootFolderId": custom_root_id }),
        ),
    )
    .await;
    assert_no_errors(&same_value);
    assert_eq!(same_value["data"]["addTitle"]["reusedExistingTitle"], true);
    assert_eq!(
        same_value["data"]["addTitle"]["title"]["rootFolderId"],
        custom_root_id
    );

    // A fileless title's root is a catalog pointer, so reuse may still move it.
    let fileless_first = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse Fileless Movie",
            "770078",
            json!({ "rootFolderId": custom_root_id }),
        ),
    )
    .await;
    assert_no_errors(&fileless_first);
    let fileless_reused = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse Fileless Movie",
            "770078",
            json!({ "rootFolderId": default_root_id }),
        ),
    )
    .await;
    assert_no_errors(&fileless_reused);
    assert_eq!(
        fileless_reused["data"]["addTitle"]["reusedExistingTitle"],
        true
    );
    assert_eq!(
        fileless_reused["data"]["addTitle"]["title"]["rootFolderId"],
        default_root_id
    );

    // Creating a genuinely new title still assigns the requested root.
    let created = gql(
        &ctx,
        add,
        add_input(
            "Retired Reuse New Movie",
            "770079",
            json!({ "rootFolderId": default_root_id }),
        ),
    )
    .await;
    assert_no_errors(&created);
    assert_eq!(created["data"]["addTitle"]["reusedExistingTitle"], false);
    assert_eq!(
        created["data"]["addTitle"]["title"]["rootFolderId"],
        default_root_id
    );
}

#[tokio::test]
async fn graphql_title_root_folder_id_rejects_wrong_facet_roots() {
    let ctx = TestContext::new().await;
    let anime_library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Wrong Facet Anime Library",
        &[
            ("/library/wrong-facet-anime-default", true),
            ("/library/wrong-facet-anime-custom", false),
        ],
    )
    .await;
    let movie_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Wrong Facet Movie Library",
        &[
            ("/library/wrong-facet-movie-default", true),
            ("/library/wrong-facet-movie-custom", false),
        ],
    )
    .await;
    let anime_library_id = library_id(&anime_library);
    let anime_root_id = library_root_id(&anime_library, "/library/wrong-facet-anime-custom");
    let movie_root_id = library_root_id(&movie_library, "/library/wrong-facet-movie-custom");

    let add_wrong_facet = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": "Wrong Facet Add Anime",
                "facet": "ANIME",
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": movie_root_id
                }
            }
        }),
    )
    .await;
    assert!(
        add_wrong_facet.get("errors").is_some(),
        "movie root should not be accepted for anime addTitle: {add_wrong_facet}"
    );

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id }
            }
        }"#,
        json!({
            "input": {
                "name": "Wrong Facet Update Anime",
                "facet": "ANIME",
                "libraryId": anime_library_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": anime_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title_id = add_body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .expect("title id")
        .to_string();

    let update_wrong_facet = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) { id }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "options": {
                    "rootFolderId": movie_root_id
                }
            }
        }),
    )
    .await;
    assert!(
        update_wrong_facet.get("errors").is_some(),
        "movie root should not be accepted for anime updateTitle: {update_wrong_facet}"
    );
}

#[tokio::test]
async fn graphql_update_title_rejects_facet_change_without_library_move() {
    let ctx = TestContext::new().await;
    let anime_library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Facet Change Anime Library",
        &[
            ("/library/facet-change-anime-default", true),
            ("/library/facet-change-anime-custom", false),
        ],
    )
    .await;
    let anime_library_id = library_id(&anime_library);
    let anime_root_id = library_root_id(&anime_library, "/library/facet-change-anime-custom");

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id facet rootFolderId rootFolderPath }
            }
        }"#,
        json!({
            "input": {
                "name": "Facet Change Anime",
                "facet": "ANIME",
                "libraryId": anime_library_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": anime_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title = &add_body["data"]["addTitle"]["title"];
    let title_id = title["id"].as_str().expect("title id");
    assert_eq!(title["facet"], "ANIME");
    assert_eq!(title["rootFolderId"], anime_root_id);
    assert_eq!(
        title["rootFolderPath"],
        "/library/facet-change-anime-custom"
    );

    let update_wrong_facet = gql(
        &ctx,
        r#"mutation($input: UpdateTitleInput!) {
            updateTitle(input: $input) {
                id
                facet
                rootFolderId
                rootFolderPath
            }
        }"#,
        json!({
            "input": {
                "titleId": title_id,
                "facet": "MOVIE"
            }
        }),
    )
    .await;
    assert!(
        update_wrong_facet.get("errors").is_some(),
        "facet change should not be accepted without library move support: {update_wrong_facet}"
    );

    let after = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) {
                id
                facet
                rootFolderId
                rootFolderPath
            }
        }"#,
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&after);
    let title = &after["data"]["title"];
    assert_eq!(title["facet"], "ANIME");
    assert_eq!(title["rootFolderId"], anime_root_id);
    assert_eq!(
        title["rootFolderPath"],
        "/library/facet-change-anime-custom"
    );
}

#[tokio::test]
async fn graphql_title_root_folder_id_tracks_library_root_id_lifecycle() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Root Lifecycle Anime Library",
        &[
            ("/library/root-lifecycle-default", true),
            ("/library/root-lifecycle-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let custom_root_id = library_root_id(&library, "/library/root-lifecycle-custom");

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id rootFolderId rootFolderPath }
            }
        }"#,
        json!({
            "input": {
                "name": "Root Lifecycle Anime",
                "facet": "ANIME",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": custom_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let added = &add_body["data"]["addTitle"]["title"];
    assert_eq!(added["rootFolderId"], custom_root_id);
    assert_eq!(added["rootFolderPath"], "/library/root-lifecycle-custom");
    let title_id = added["id"].as_str().expect("title id").to_string();
    assert_eq!(
        stored_title_root_folder_id(&ctx, &title_id).await,
        custom_root_id
    );

    let referenced_path_edit = gql(
        &ctx,
        r#"mutation($input: UpdateLibraryInput!) {
            updateLibrary(input: $input) {
                roots { id path isDefault }
            }
        }"#,
        json!({
            "input": {
                "libraryId": library_id,
                "roots": [
                    {
                        "path": "/library/root-lifecycle-default",
                        "isDefault": true
                    },
                    {
                        "path": "/library/root-lifecycle-renamed",
                        "isDefault": false
                    }
                ]
            }
        }),
    )
    .await;
    assert!(
        referenced_path_edit.get("errors").is_some(),
        "referenced root path edit should be rejected: {referenced_path_edit}"
    );

    let after_rejected_edit = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) { rootFolderId rootFolderPath }
        }"#,
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&after_rejected_edit);
    assert_eq!(
        after_rejected_edit["data"]["title"]["rootFolderId"],
        custom_root_id
    );
    assert_eq!(
        after_rejected_edit["data"]["title"]["rootFolderPath"],
        "/library/root-lifecycle-custom"
    );

    let added_unreferenced_root = gql(
        &ctx,
        r#"mutation($input: UpdateLibraryInput!) {
            updateLibrary(input: $input) {
                roots { id path isDefault }
            }
        }"#,
        json!({
            "input": {
                "libraryId": library_id,
                "roots": [
                    {
                        "path": "/library/root-lifecycle-default",
                        "isDefault": true
                    },
                    {
                        "path": "/library/root-lifecycle-custom",
                        "isDefault": false
                    },
                    {
                        "path": "/library/root-lifecycle-unreferenced",
                        "isDefault": false
                    }
                ]
            }
        }),
    )
    .await;
    assert_no_errors(&added_unreferenced_root);
    let unreferenced_root_id = library_root_id(
        &added_unreferenced_root["data"]["updateLibrary"],
        "/library/root-lifecycle-unreferenced",
    );
    // Root ids are allocated, never derived from the path (FR-078).
    assert!(
        unreferenced_root_id.starts_with(scryer_domain::SYNTHETIC_ROOT_ID_PREFIX),
        "a newly configured root should carry an allocated id, got {unreferenced_root_id}"
    );
    assert_ne!(
        unreferenced_root_id,
        scryer_domain::root_folder_id_for_path("/library/root-lifecycle-unreferenced")
    );

    let renamed_unreferenced_root = gql(
        &ctx,
        r#"mutation($input: UpdateLibraryInput!) {
            updateLibrary(input: $input) {
                roots { id path isDefault }
            }
        }"#,
        json!({
            "input": {
                "libraryId": library_id,
                "roots": [
                    {
                        "path": "/library/root-lifecycle-default",
                        "isDefault": true
                    },
                    {
                        "path": "/library/root-lifecycle-custom",
                        "isDefault": false
                    },
                    {
                        "path": "/library/root-lifecycle-unreferenced-renamed",
                        "isDefault": false
                    }
                ]
            }
        }),
    )
    .await;
    assert_no_errors(&renamed_unreferenced_root);
    let renamed_unreferenced_root_id = library_root_id(
        &renamed_unreferenced_root["data"]["updateLibrary"],
        "/library/root-lifecycle-unreferenced-renamed",
    );
    // The bulk root replace still cannot express "this root moved", so an
    // unreferenced root swapped for another path reads as a removal plus an
    // addition and the addition is allocated a fresh id. Identity across a
    // deliberate path change is the move workflow's job, not this mutation's.
    assert_ne!(renamed_unreferenced_root_id, unreferenced_root_id);
    assert!(
        renamed_unreferenced_root_id.starts_with(scryer_domain::SYNTHETIC_ROOT_ID_PREFIX),
        "a newly configured root should carry an allocated id, got {renamed_unreferenced_root_id}"
    );

    let removed_unreferenced_root = gql(
        &ctx,
        r#"mutation($input: UpdateLibraryInput!) {
            updateLibrary(input: $input) {
                roots { id path isDefault }
            }
        }"#,
        json!({
            "input": {
                "libraryId": library_id,
                "roots": [
                    {
                        "path": "/library/root-lifecycle-default",
                        "isDefault": true
                    },
                    {
                        "path": "/library/root-lifecycle-custom",
                        "isDefault": false
                    }
                ]
            }
        }),
    )
    .await;
    assert_no_errors(&removed_unreferenced_root);

    let after_remove = gql(
        &ctx,
        r#"query($id: ID!) {
            title(id: $id) { rootFolderId rootFolderPath }
        }"#,
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&after_remove);
    assert_eq!(
        after_remove["data"]["title"]["rootFolderId"],
        custom_root_id
    );
    assert_eq!(
        after_remove["data"]["title"]["rootFolderPath"],
        "/library/root-lifecycle-custom"
    );
    assert_eq!(
        stored_title_root_folder_id(&ctx, &title_id).await,
        custom_root_id
    );
}

#[tokio::test]
async fn graphql_add_title_default_root_id_stores_library_default() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "ANIME",
        "Default Root Anime Library",
        &[
            ("/library/default-root-default", true),
            ("/library/default-root-custom", false),
        ],
    )
    .await;
    let library_id = library_id(&library);
    let default_root_id = library_root_id(&library, "/library/default-root-default");

    let add_body = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) {
                title { id rootFolderId rootFolderPath }
            }
        }"#,
        json!({
            "input": {
                "name": "Default Root Anime",
                "facet": "ANIME",
                "libraryId": library_id,
                "monitored": true,
                "tags": [],
                "options": {
                    "rootFolderId": default_root_id
                }
            }
        }),
    )
    .await;
    assert_no_errors(&add_body);
    let title = &add_body["data"]["addTitle"]["title"];
    assert_eq!(title["rootFolderId"], default_root_id);
    assert_eq!(title["rootFolderPath"], "/library/default-root-default");
    let title_id = title["id"].as_str().expect("title id");
    assert_eq!(
        stored_title_root_folder_id(&ctx, title_id).await,
        default_root_id
    );
}

/// The per-item `triggerTitleWantedSearch`/`triggerSeasonWantedSearch`
/// mutations were removed. A fileless monitored movie is a *derived* Missing
/// target (no seeding, no state row required): it appears directly in
/// `wantedItems(wantedKind: MISSING)` with convergence progress, and the
/// interactive `triggerAcquisitionSearch` job is what searches it now.
#[tokio::test]
async fn graphql_wanted_items_missing_view_exposes_fileless_monitored_movie() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Search Monitored Test", "MOVIE").await;

    let body = gql(
        &ctx,
        r#"query($wantedKind: WantedKindValue!, $titleSearch: String) {
            wantedItems(wantedKind: $wantedKind, titleSearch: $titleSearch) {
                totalCount
                items { id titleId mediaType status convergenceState recencyLane }
            }
        }"#,
        json!({ "wantedKind": "MISSING", "titleSearch": "Search Monitored Test" }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["wantedItems"]["totalCount"], 1);
    let item = &body["data"]["wantedItems"]["items"][0];
    assert_eq!(item["titleId"], id);
    assert_eq!(item["mediaType"], "MOVIE");
    assert_eq!(item["status"], "WANTED");
    // With no state row and no indexers routed the scope reads as converged (0/0).
    assert!(item["convergenceState"].is_string());
    assert!(item["recencyLane"].is_string());
    // The derived-target id is the scope key when no state row exists.
    assert_eq!(item["id"], format!("title:{id}"));

    // The interactive search job accepts that scope id and starts (no indexers →
    // nothing grabbed, but the job is created and its payload is well-formed).
    let body = gql(
        &ctx,
        r#"mutation($input: TriggerAcquisitionSearchInput!) {
            triggerAcquisitionSearch(input: $input) {
                id
                state
                total
                grabbedCount
                failedCount
            }
        }"#,
        json!({ "input": { "wantedItemId": format!("title:{id}") } }),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["triggerAcquisitionSearch"]["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

#[tokio::test]
async fn graphql_wanted_items_reports_standby_count_for_the_scope_anchor() {
    let ctx = TestContext::new().await;
    let title_id = add_test_title(&ctx, "Standby Count Test", "MOVIE").await;
    let wanted_item_id = Id::new().0;
    let now = Utc::now();
    ctx.library_state
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: wanted_item_id.clone(),
            title_id: title_id.clone(),
            title_name: Some("Standby Count Test".to_string()),
            title_slug: None,
            title_facet: Some("movie".to_string()),
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: scryer_application::AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
        })
        .await
        .expect("seed wanted scope");
    let pending_store =
        scryer_infrastructure_library::media::libraries::state_store::PendingReleaseStore::new(
            ctx.db.datastore(),
            ctx.db.encryption_key_state(),
        );
    for score in [500, 400] {
        pending_store
            .insert_pending_release(&PendingRelease {
                id: Id::new().0,
                wanted_item_id: wanted_item_id.clone(),
                title_id: title_id.clone(),
                release_title: format!("Standby.Count.Test.{score}.1080p.WEB-DL"),
                release_url: Some(format!("https://example.invalid/{score}.nzb")),
                source_kind: None,
                release_size_bytes: Some(1_024),
                release_score: score,
                scoring_log_json: None,
                indexer_source: Some("test-indexer".to_string()),
                indexer_id: None,
                release_guid: Some(format!("guid-{score}")),
                added_at: now.to_rfc3339(),
                last_observed_at: now.to_rfc3339(),
                delay_until: now.to_rfc3339(),
                status: scryer_application::PendingReleaseStatus::Standby,
                grabbed_at: None,
                source_password: None,
                published_at: None,
                info_hash: None,
                seed_minimums: Default::default(),
                seeders: None,
                release_identity: format!("guid:test-indexer:guid-{score}"),
                coverage_identity: format!("scope:{wanted_item_id}"),
                role: scryer_application::PendingReleaseRole::Fallback,
                last_decision_code: None,
                release_age_unknown: false,
            })
            .await
            .expect("seed standby release");
    }

    let body = gql(
        &ctx,
        r#"query($wantedKind: WantedKindValue!, $titleSearch: String) {
            wantedItems(wantedKind: $wantedKind, titleSearch: $titleSearch) {
                items { id standbyCount }
            }
        }"#,
        json!({ "wantedKind": "MISSING", "titleSearch": "Standby Count Test" }),
    )
    .await;
    assert_no_errors(&body);
    let item = &body["data"]["wantedItems"]["items"][0];
    assert_eq!(item["id"], wanted_item_id);
    assert_eq!(item["standbyCount"], 2);
}

#[tokio::test]
async fn graphql_delete_title() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "To Delete", "MOVIE").await;

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) { id } }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"]["id"], id);

    // Verify deleted
    let body = gql(
        &ctx,
        r#"query($id: ID!) { title(id: $id) { id } }"#,
        json!({ "id": id }),
    )
    .await;
    assert!(body["data"]["title"].is_null(), "title should be gone");
}

#[tokio::test]
async fn graphql_delete_title_cleans_title_workflow_state() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Delete With Cleanup", "MOVIE").await;

    ctx.library_state
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: Id::new().0,
            title_id: id.clone(),
            title_name: Some("Delete With Cleanup".to_string()),
            title_slug: None,
            title_facet: Some("movie".to_string()),
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: scryer_application::AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2026-03-12T00:00:00Z".to_string(),
            updated_at: "2026-03-12T00:00:00Z".to_string(),
        })
        .await
        .expect("seed wanted item");
    scryer_infrastructure_library::media::libraries::state_store::PendingReleaseStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    )
    .insert_pending_release(&PendingRelease {
        id: Id::new().0,
        wanted_item_id: "wanted-delete".to_string(),
        title_id: id.clone(),
        release_title: "Delete With Cleanup 2026".to_string(),
        release_url: Some("https://example.invalid/release.nzb".to_string()),
        source_kind: None,
        release_size_bytes: Some(1_024),
        release_score: 100,
        scoring_log_json: None,
        indexer_source: Some("test-indexer".to_string()),
        indexer_id: None,
        release_guid: Some("guid-delete".to_string()),
        added_at: "2026-03-12T00:00:00Z".to_string(),
        last_observed_at: "2026-03-12T00:00:00Z".to_string(),
        delay_until: "2026-03-13T00:00:00Z".to_string(),
        status: scryer_application::PendingReleaseStatus::Waiting,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
        seed_minimums: Default::default(),
        seeders: None,
        release_identity: "guid:test-indexer:guid-delete".to_string(),
        coverage_identity: "scope:wanted-delete".to_string(),
        role: scryer_application::PendingReleaseRole::Primary,
        last_decision_code: None,
        release_age_unknown: false,
    })
    .await
    .expect("seed pending release");
    let workflow_store = DownloadSubmissionStore::new(ctx.db.datastore());
    workflow_store
        .record_submission(scryer_application::DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: id.clone(),
            facet: "movie".to_string(),
            download_client_id: None,
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "queue-delete".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Delete With Cleanup".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            scope: scryer_application::SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) { id } }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"]["id"], id);

    assert!(
        scryer_infrastructure_library::media::libraries::state_store::WantedStore::new(
            ctx.db.datastore()
        )
        .list_acquisition_scope_states(scryer_application::AcquisitionScopeStatesQuery {
            title_id: Some(id.clone()),
            limit: 10,
            ..scryer_application::AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("wanted items")
        .is_empty()
    );
    assert!(
        scryer_infrastructure_library::media::libraries::state_store::PendingReleaseStore::new(
            ctx.db.datastore(),
            ctx.db.encryption_key_state(),
        )
        .list_waiting_pending_releases()
        .await
        .expect("pending releases")
        .iter()
        .all(|entry| entry.title_id != id)
    );
    assert!(
        workflow_store
            .list_for_title(&id)
            .await
            .expect("download submissions")
            .is_empty()
    );
}

#[tokio::test]
async fn graphql_filter_titles_by_facet() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie A", "MOVIE").await;
    add_test_title(&ctx, "Series A", "SERIES").await;

    let body = gql(
        &ctx,
        r#"query($facet: MediaFacetValue) { titles(facet: $facet) { items { name facet } } }"#,
        json!({ "facet": "MOVIE" }),
    )
    .await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"]["items"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["facet"], "MOVIE");
}

#[tokio::test]
async fn graphql_series_titles_expose_series_facet() {
    let ctx = TestContext::new().await;
    let expected_name = "Series A";
    add_test_title(&ctx, expected_name, "SERIES").await;

    let body = gql(&ctx, "{ titles { items { name facet } } }", json!({})).await;
    assert_no_errors(&body);

    let titles = body["data"]["titles"]["items"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    let title = &titles[0];
    assert_eq!(title["name"], expected_name);
    assert_eq!(title["facet"], "SERIES");
}

// ---------------------------------------------------------------------------
// Admin-defined title tags
// ---------------------------------------------------------------------------

/// A user who may manage titles in exactly one library and nothing else. No app
/// permission at all, so the registry read still has to work for them while
/// every registry write is refused.
fn title_tag_manager_actor(library_id: &str) -> User {
    User {
        id: Id::new().0,
        username: "title-tag-manager".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::from([(
                library_id.to_string(),
                LibraryPermissionMask::from_permissions([
                    LibraryPermission::View,
                    LibraryPermission::ManageTitles,
                ]),
            )]),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

async fn define_title_tag(ctx: &TestContext, label: &str, description: Option<&str>) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: CreateTitleTagDefinitionInput!) {
            createTitleTagDefinition(input: $input) {
                definition { id label description titleCount }
                counts { titles delayProfiles maintenanceRuleSets releaseRuleSets managedTagFilters }
            }
        }"#,
        json!({ "input": { "label": label, "description": description } }),
    )
    .await;
    assert_no_errors(&body);
    let created = &body["data"]["createTitleTagDefinition"];
    assert_eq!(created["definition"]["titleCount"], 0);
    assert_eq!(created["counts"]["titles"], 0);
    created["definition"]["id"]
        .as_str()
        .expect("title tag definition id")
        .to_string()
}

async fn stored_title_tags(ctx: &TestContext, title_id: &str) -> Vec<String> {
    let body = gql(
        ctx,
        r#"query($id: ID!) { title(id: $id) { tags } }"#,
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["title"]["tags"]
        .as_array()
        .expect("title tags")
        .iter()
        .map(|tag| tag.as_str().expect("tag string").to_string())
        .collect()
}

fn graphql_error_messages(body: &Value) -> String {
    body["errors"]
        .as_array()
        .expect("expected GraphQL errors")
        .iter()
        .filter_map(|error| error["message"].as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

#[tokio::test]
async fn graphql_title_tag_registry_round_trips_with_rewrite_counts() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Tag Registry Library",
        &[("/tag-registry/movies", true)],
    )
    .await;
    let library_id = library_id(&library);
    let root_id = library_root_id(&library, "/tag-registry/movies");
    let first =
        add_catalog_filter_title(&ctx, "Tagged One", "992001", &library_id, &root_id, 2001).await;
    let second =
        add_catalog_filter_title(&ctx, "Tagged Two", "992002", &library_id, &root_id, 2002).await;

    // The label is normalized on the way in, so the registry stores one
    // canonical spelling however the operator typed it.
    let definition_id = define_title_tag(&ctx, "  Needs   Review ", Some("  look at this  ")).await;
    define_title_tag(&ctx, "keep", None).await;

    let listed = gql(
        &ctx,
        r#"{ titleTagDefinitions { id label description titleCount } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&listed);
    let definitions = listed["data"]["titleTagDefinitions"]
        .as_array()
        .expect("title tag definitions");
    let labels = definitions
        .iter()
        .map(|definition| definition["label"].as_str().expect("label"))
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["keep", "needs review"]);
    let normalized = definitions
        .iter()
        .find(|definition| definition["label"] == "needs review")
        .expect("the normalized definition");
    assert_eq!(normalized["id"], definition_id.as_str());
    assert_eq!(normalized["description"], "look at this");
    assert_eq!(normalized["titleCount"], 0);

    let assigned = gql(
        &ctx,
        r#"mutation($input: UpdateTitleTagsInput!) {
            updateTitleTags(input: $input) { id tags }
        }"#,
        json!({
            "input": {
                "titleIds": [first, second],
                // Typed the way an operator would, not the way it is stored.
                "add": ["Needs Review"],
            }
        }),
    )
    .await;
    assert_no_errors(&assigned);
    let updated = assigned["data"]["updateTitleTags"]
        .as_array()
        .expect("updated titles");
    assert_eq!(updated.len(), 2);
    for title in updated {
        assert_eq!(title["tags"], json!(["needs review"]));
    }

    let renamed = gql(
        &ctx,
        r#"mutation($input: UpdateTitleTagDefinitionInput!) {
            updateTitleTagDefinition(input: $input) {
                definition { id label description titleCount }
                counts { titles delayProfiles maintenanceRuleSets releaseRuleSets managedTagFilters }
            }
        }"#,
        json!({
            "input": { "id": definition_id, "label": "Archive", "description": null }
        }),
    )
    .await;
    assert_no_errors(&renamed);
    let renamed = &renamed["data"]["updateTitleTagDefinition"];
    assert_eq!(renamed["definition"]["label"], "archive");
    assert!(renamed["definition"]["description"].is_null());
    assert_eq!(renamed["definition"]["titleCount"], 2);
    assert_eq!(renamed["counts"]["titles"], 2);
    assert_eq!(renamed["counts"]["delayProfiles"], 0);
    // No rules exist in this fixture; the counts are the warning surface for
    // sources a rename can never rewrite.
    assert_eq!(renamed["counts"]["maintenanceRuleSets"], 0);
    assert_eq!(renamed["counts"]["releaseRuleSets"], 0);
    assert_eq!(renamed["counts"]["managedTagFilters"], 0);
    assert_eq!(stored_title_tags(&ctx, &first).await, vec!["archive"]);
    assert_eq!(stored_title_tags(&ctx, &second).await, vec!["archive"]);

    let deleted = gql(
        &ctx,
        r#"mutation($id: ID!) {
            deleteTitleTagDefinition(id: $id) {
                id
                label
                counts { titles delayProfiles maintenanceRuleSets releaseRuleSets managedTagFilters }
            }
        }"#,
        json!({ "id": definition_id }),
    )
    .await;
    assert_no_errors(&deleted);
    let deleted = &deleted["data"]["deleteTitleTagDefinition"];
    assert_eq!(deleted["id"], definition_id.as_str());
    assert_eq!(deleted["label"], "archive");
    assert_eq!(deleted["counts"]["titles"], 2);

    assert!(stored_title_tags(&ctx, &first).await.is_empty());
    assert!(stored_title_tags(&ctx, &second).await.is_empty());
    let remaining = gql(&ctx, r#"{ titleTagDefinitions { label } }"#, json!({})).await;
    assert_no_errors(&remaining);
    assert_eq!(
        remaining["data"]["titleTagDefinitions"],
        json!([{ "label": "keep" }])
    );
}

#[tokio::test]
async fn graphql_title_tag_writes_name_the_label_they_refuse() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Tag Refusal Library",
        &[("/tag-refusal/movies", true)],
    )
    .await;
    let library_id = library_id(&library);
    let root_id = library_root_id(&library, "/tag-refusal/movies");
    let title_id = add_catalog_filter_title(
        &ctx,
        "Refusal Subject",
        "992101",
        &library_id,
        &root_id,
        2003,
    )
    .await;

    let undefined = gql(
        &ctx,
        r#"mutation($input: UpdateTitleTagsInput!) {
            updateTitleTags(input: $input) { id }
        }"#,
        json!({
            "input": { "titleIds": [title_id], "add": ["not defined"] }
        }),
    )
    .await;
    let message = graphql_error_messages(&undefined);
    assert!(
        message.contains("not defined"),
        "the refusal must name the label: {message}"
    );
    assert!(stored_title_tags(&ctx, &title_id).await.is_empty());

    let reserved = gql(
        &ctx,
        r#"mutation($input: UpdateTitleTagsInput!) {
            updateTitleTags(input: $input) { id }
        }"#,
        json!({
            "input": { "titleIds": [title_id], "add": ["scryer:monitor-type:all"] }
        }),
    )
    .await;
    let message = graphql_error_messages(&reserved);
    assert!(
        message.contains("scryer:monitor-type:all") && message.contains("reserved"),
        "the reserved-namespace refusal must name the label: {message}"
    );
    assert!(stored_title_tags(&ctx, &title_id).await.is_empty());

    // Creation is gated the same way: a title may not be born carrying a label
    // the registry never authorized.
    let created = gql(
        &ctx,
        r#"mutation($input: AddTitleInput!) {
            addTitle(input: $input) { title { id } }
        }"#,
        json!({
            "input": {
                "name": "Born Untagged",
                "facet": "MOVIE",
                "libraryId": library_id,
                "monitored": true,
                "tags": ["not defined"],
                "externalIds": [{ "source": "tvdb", "value": "992102" }],
                "options": { "rootFolderId": root_id }
            }
        }),
    )
    .await;
    let message = graphql_error_messages(&created);
    assert!(
        message.contains("not defined"),
        "the creation refusal must name the label: {message}"
    );
}

#[tokio::test]
async fn graphql_bulk_title_tag_patch_writes_nothing_when_one_library_is_denied() {
    let ctx = TestContext::new().await;
    let allowed_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Tag Allowed Library",
        &[("/tag-rbac/allowed", true)],
    )
    .await;
    let denied_library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Tag Denied Library",
        &[("/tag-rbac/denied", true)],
    )
    .await;
    let allowed_library_id = library_id(&allowed_library);
    let denied_library_id = library_id(&denied_library);
    let allowed_root_id = library_root_id(&allowed_library, "/tag-rbac/allowed");
    let denied_root_id = library_root_id(&denied_library, "/tag-rbac/denied");
    let allowed_title_id = add_catalog_filter_title(
        &ctx,
        "Allowed Tag Subject",
        "992201",
        &allowed_library_id,
        &allowed_root_id,
        2004,
    )
    .await;
    let denied_title_id = add_catalog_filter_title(
        &ctx,
        "Denied Tag Subject",
        "992202",
        &denied_library_id,
        &denied_root_id,
        2005,
    )
    .await;
    define_title_tag(&ctx, "keep", None).await;

    let actor = title_tag_manager_actor(&allowed_library_id);
    // The registry read is unprivileged, so a user with no app permission at
    // all still gets the vocabulary the picker needs.
    let vocabulary = schema_exec(
        &ctx,
        "{ titleTagDefinitions { label } }",
        Some(actor.clone()),
    )
    .await;
    assert_no_errors(&vocabulary);
    assert_eq!(
        vocabulary["data"]["titleTagDefinitions"],
        json!([{ "label": "keep" }])
    );

    let denied = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
                updateTitleTags(input: {{
                    titleIds: ["{allowed_title_id}", "{denied_title_id}"]
                    add: ["keep"]
                }}) {{ id }}
            }}"#,
        ),
        Some(actor.clone()),
    )
    .await;
    assert!(
        denied.get("errors").is_some(),
        "a bulk patch touching an unmanageable library must be refused: {denied}"
    );
    // Neither half landed: the authorization sweep runs before the first write.
    assert!(stored_title_tags(&ctx, &allowed_title_id).await.is_empty());
    assert!(stored_title_tags(&ctx, &denied_title_id).await.is_empty());

    // The same patch scoped to the library the actor manages does land.
    let allowed = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
                updateTitleTags(input: {{
                    titleIds: ["{allowed_title_id}"]
                    add: ["keep"]
                }}) {{ id tags }}
            }}"#,
        ),
        Some(actor.clone()),
    )
    .await;
    assert_no_errors(&allowed);
    assert_eq!(
        allowed["data"]["updateTitleTags"],
        json!([{ "id": allowed_title_id, "tags": ["keep"] }])
    );

    // Defining the vocabulary is catalog configuration, not title management.
    let refused = schema_exec(
        &ctx,
        r#"mutation {
            createTitleTagDefinition(input: { label: "unauthorized" }) {
                definition { id }
            }
        }"#,
        Some(actor),
    )
    .await;
    assert!(
        refused.get("errors").is_some(),
        "a registry write without ManageCatalogSettings must be refused: {refused}"
    );
}

#[tokio::test]
async fn graphql_title_catalog_filters_by_user_tag() {
    let ctx = TestContext::new().await;
    let library = create_title_catalog_library(
        &ctx,
        "MOVIE",
        "Tag Filter Library",
        &[("/tag-filter/movies", true)],
    )
    .await;
    let library_id = library_id(&library);
    let root_id = library_root_id(&library, "/tag-filter/movies");
    let tagged = add_catalog_filter_title(
        &ctx,
        "Tag Filter Hit",
        "992301",
        &library_id,
        &root_id,
        2006,
    )
    .await;
    let other_tagged = add_catalog_filter_title(
        &ctx,
        "Tag Filter Other",
        "992302",
        &library_id,
        &root_id,
        2007,
    )
    .await;
    let untagged = add_catalog_filter_title(
        &ctx,
        "Tag Filter Miss",
        "992303",
        &library_id,
        &root_id,
        2008,
    )
    .await;
    define_title_tag(&ctx, "keep", None).await;
    define_title_tag(&ctx, "needs review", None).await;

    let assigned = gql(
        &ctx,
        r#"mutation($input: UpdateTitleTagsInput!) {
            updateTitleTags(input: $input) { id }
        }"#,
        json!({ "input": { "titleIds": [tagged], "add": ["keep"] } }),
    )
    .await;
    assert_no_errors(&assigned);
    let assigned = gql(
        &ctx,
        r#"mutation($input: UpdateTitleTagsInput!) {
            updateTitleTags(input: $input) { id }
        }"#,
        json!({ "input": { "titleIds": [other_tagged], "add": ["needs review"] } }),
    )
    .await;
    assert_no_errors(&assigned);

    let filtered = gql(
        &ctx,
        r#"query($libraryIds: [ID!], $tags: [String!]) {
            titles(facet: MOVIE, libraryIds: $libraryIds, filter: { tags: $tags }) {
                items { id }
                totalCount
            }
        }"#,
        json!({ "libraryIds": [library_id], "tags": ["Keep"] }),
    )
    .await;
    assert_no_errors(&filtered);
    assert_eq!(filtered["data"]["titles"]["totalCount"], 1);
    assert_eq!(filtered["data"]["titles"]["items"][0]["id"], tagged);

    // Any-of, not all-of: two labels widen the result rather than narrowing it.
    let widened = gql(
        &ctx,
        r#"query($libraryIds: [ID!], $tags: [String!]) {
            titles(facet: MOVIE, libraryIds: $libraryIds, filter: { tags: $tags }) {
                items { id }
                totalCount
            }
        }"#,
        json!({
            "libraryIds": [library_id],
            "tags": ["keep", "needs review"],
        }),
    )
    .await;
    assert_no_errors(&widened);
    assert_eq!(widened["data"]["titles"]["totalCount"], 2);
    let mut ids = widened["data"]["titles"]["items"]
        .as_array()
        .expect("filtered items")
        .iter()
        .map(|title| title["id"].as_str().expect("title id"))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut expected = vec![tagged.as_str(), other_tagged.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert!(!ids.contains(&untagged.as_str()));

    // An empty list is not a filter at all; every title comes back.
    let unfiltered = gql(
        &ctx,
        r#"query($libraryIds: [ID!]) {
            titles(facet: MOVIE, libraryIds: $libraryIds, filter: { tags: [] }) {
                totalCount
            }
        }"#,
        json!({ "libraryIds": [library_id] }),
    )
    .await;
    assert_no_errors(&unfiltered);
    assert_eq!(unfiltered["data"]["titles"]["totalCount"], 3);
}
