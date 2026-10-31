use super::*;

fn ui_settings_test_user(username: &str) -> User {
    User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

async fn create_ui_settings_test_user(ctx: &TestContext, username: &str) -> User {
    ctx.users
        .create(ui_settings_test_user(username))
        .await
        .expect("create UI settings test user")
}

#[tokio::test]
async fn graphql_anonymous_ui_settings_round_trip_is_shared() {
    let ctx = TestContext::new().await;

    let update = gql(
        &ctx,
        r#"
        mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
          setMyUiSettings(input: $input) {
            theme
            palette
            dateTimeFormat
            highlightColor
            secondaryColor
            highContrastMode
            reduceMotion
            hideSponsorButton
            density
            sidebarMode
            defaultLandingView
            tableColumns {
              facet
              tableViewMode
              columnId
              columnOrder
              visible
            }
          }
        }
        "#,
        json!({
            "input": {
                "theme": "PRIDE",
                "palette": "OCEAN",
                "dateTimeFormat": "ISO24H",
                "highlightColor": "#ff3366",
                "secondaryColor": "#2277aa",
                "highContrastMode": true,
                "reduceMotion": true,
                "hideSponsorButton": true,
                "density": "COMPACT",
                "sidebarMode": "COLLAPSED",
                "defaultLandingView": "CALENDAR",
                "tableColumns": [
                    {
                        "facet": "MOVIES",
                        "tableViewMode": "COMPACT",
                        "columnId": "name",
                        "columnOrder": 0,
                        "visible": true
                    },
                    {
                        "facet": "SERIES",
                        "tableViewMode": "POSTER_TABLE",
                        "columnId": "episodes",
                        "columnOrder": 1,
                        "visible": false
                    }
                ]
            }
        }),
    )
    .await;
    assert_no_errors(&update);

    let read = gql(
        &ctx,
        r#"
        query MyUiSettings {
          myUiSettings {
            theme
            palette
            dateTimeFormat
            highlightColor
            secondaryColor
            highContrastMode
            reduceMotion
            hideSponsorButton
            density
            sidebarMode
            defaultLandingView
            tableColumns {
              facet
              tableViewMode
              columnId
              columnOrder
              visible
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&read);

    let settings = &read["data"]["myUiSettings"];
    // Retired theme inputs remain accepted and normalize to the supported dark theme.
    assert_eq!(settings["theme"], json!("DARK"));
    assert_eq!(settings["palette"], json!("OCEAN"));
    assert_eq!(settings["dateTimeFormat"], json!("ISO24H"));
    assert_eq!(settings["highlightColor"], json!("#ff3366"));
    assert_eq!(settings["secondaryColor"], json!("#2277aa"));
    assert_eq!(settings["highContrastMode"], json!(true));
    assert_eq!(settings["reduceMotion"], json!(true));
    assert_eq!(settings["hideSponsorButton"], json!(true));
    assert_eq!(settings["density"], json!("COMPACT"));
    assert_eq!(settings["sidebarMode"], json!("COLLAPSED"));
    assert_eq!(settings["defaultLandingView"], json!("CALENDAR"));
    assert_eq!(
        settings["tableColumns"],
        json!([
            {
                "facet": "MOVIES",
                "tableViewMode": "COMPACT",
                "columnId": "name",
                "columnOrder": 0,
                "visible": true
            },
            {
                "facet": "SERIES",
                "tableViewMode": "POSTER_TABLE",
                "columnId": "episodes",
                "columnOrder": 1,
                "visible": false
            }
        ])
    );

    let old_client_update = gql(
        &ctx,
        r#"
        mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
          setMyUiSettings(input: $input) {
            theme
            palette
            dateTimeFormat
          }
        }
        "#,
        json!({
            "input": {
                "theme": "DARK",
                "highlightColor": "#ff3366",
                "secondaryColor": "#2277aa",
                "highContrastMode": true,
                "reduceMotion": true,
                "hideSponsorButton": false,
                "density": "COMPACT",
                "sidebarMode": "COLLAPSED",
                "defaultLandingView": "CALENDAR",
                "tableColumns": []
            }
        }),
    )
    .await;
    assert_no_errors(&old_client_update);
    assert_eq!(
        old_client_update["data"]["setMyUiSettings"]["palette"],
        json!("OCEAN")
    );
    assert_eq!(
        old_client_update["data"]["setMyUiSettings"]["dateTimeFormat"],
        json!("ISO24H")
    );
}

#[tokio::test]
async fn graphql_ui_settings_are_isolated_per_logged_in_user() {
    let ctx = TestContext::new().await;
    let user_a = create_ui_settings_test_user(&ctx, "ui_user_a").await;
    let user_b = create_ui_settings_test_user(&ctx, "ui_user_b").await;

    let update_a = schema_exec(
        &ctx,
        r##"
        mutation SetMyUiSettings {
          setMyUiSettings(input: {
            theme: SYSTEM
            palette: FOREST
            dateTimeFormat: ISO24H
            highlightColor: "#112233"
            secondaryColor: "#445566"
            highContrastMode: false
            reduceMotion: true
            hideSponsorButton: true
            density: COMPACT
            sidebarMode: COLLAPSED
            defaultLandingView: SERIES
            tableColumns: [
              {
                facet: SERIES
                tableViewMode: POSTER_TABLE
                columnId: "episodes"
                columnOrder: 0
                visible: true
              }
            ]
          }) {
            theme
            palette
            dateTimeFormat
            tableColumns { facet tableViewMode columnId columnOrder visible }
          }
        }
        "##,
        Some(user_a.clone()),
    )
    .await;
    assert_no_errors(&update_a);

    let read_a = schema_exec(
        &ctx,
        r#"
        query MyUiSettings {
          myUiSettings {
            theme
            palette
            dateTimeFormat
            density
            sidebarMode
            defaultLandingView
            tableColumns { facet tableViewMode columnId columnOrder visible }
          }
        }
        "#,
        Some(user_a),
    )
    .await;
    assert_no_errors(&read_a);
    assert_eq!(read_a["data"]["myUiSettings"]["palette"], json!("FOREST"));
    assert_eq!(read_a["data"]["myUiSettings"]["theme"], json!("SYSTEM"));
    assert_eq!(
        read_a["data"]["myUiSettings"]["dateTimeFormat"],
        json!("ISO24H")
    );
    assert_eq!(
        read_a["data"]["myUiSettings"]["tableColumns"],
        json!([
            {
                "facet": "SERIES",
                "tableViewMode": "POSTER_TABLE",
                "columnId": "episodes",
                "columnOrder": 0,
                "visible": true
            }
        ])
    );

    let read_b = schema_exec(
        &ctx,
        r#"
        query MyUiSettings {
          myUiSettings {
            theme
            palette
            dateTimeFormat
            density
            sidebarMode
            defaultLandingView
            tableColumns { facet tableViewMode columnId columnOrder visible }
          }
        }
        "#,
        Some(user_b),
    )
    .await;
    assert_no_errors(&read_b);
    assert_eq!(read_b["data"]["myUiSettings"]["palette"], json!("CLASSIC"));
    assert_eq!(read_b["data"]["myUiSettings"]["theme"], json!("DARK"));
    assert_eq!(
        read_b["data"]["myUiSettings"]["dateTimeFormat"],
        json!("LOCALE")
    );
    assert_eq!(
        read_b["data"]["myUiSettings"]["density"],
        json!("COMFORTABLE")
    );
    assert_eq!(
        read_b["data"]["myUiSettings"]["sidebarMode"],
        json!("EXPANDED")
    );
    assert_eq!(
        read_b["data"]["myUiSettings"]["defaultLandingView"],
        json!("MOVIES")
    );
    assert_eq!(read_b["data"]["myUiSettings"]["tableColumns"], json!([]));
}

#[tokio::test]
async fn graphql_ui_settings_reject_invalid_table_column() {
    let ctx = TestContext::new().await;

    let body = gql(
        &ctx,
        r#"
        mutation SetMyUiSettings($input: SetMyUiSettingsInput!) {
          setMyUiSettings(input: $input) {
            theme
            palette
          }
        }
        "#,
        json!({
            "input": {
                "theme": "DARK",
                "dateTimeFormat": "LOCALE",
                "highlightColor": null,
                "secondaryColor": null,
            "highContrastMode": false,
            "reduceMotion": false,
            "hideSponsorButton": false,
            "density": "COMFORTABLE",
                "sidebarMode": "EXPANDED",
                "defaultLandingView": "MOVIES",
                "tableColumns": [
                    {
                        "facet": "MOVIES",
                        "tableViewMode": "COMPACT",
                        "columnId": "poster",
                        "columnOrder": 0,
                        "visible": true
                    }
                ]
            }
        }),
    )
    .await;

    let errors = body["errors"].as_array().expect("GraphQL errors");
    assert!(
        errors
            .first()
            .and_then(|error| error["message"].as_str())
            .is_some_and(|message| message.contains("unsupported compact table column")),
        "expected unsupported table column validation error: {body}"
    );
}

const CATALOG_VIEW_SELECTION: &str = r#"
    language
    catalogViews { deviceClass facet viewMode }
    tableColumns { deviceClass facet tableViewMode columnId columnOrder visible }
"#;

#[tokio::test]
async fn graphql_catalog_views_and_language_are_saved_per_user_and_device_class() {
    let ctx = TestContext::new().await;
    let user_a = create_ui_settings_test_user(&ctx, "catalog_user_a").await;
    let user_b = create_ui_settings_test_user(&ctx, "catalog_user_b").await;

    for (device_class, view_mode, column_id) in [
        ("DESKTOP", "POSTER_TABLE", "year"),
        ("MOBILE", "POSTER", "size"),
    ] {
        let saved = schema_exec(
            &ctx,
            &format!(
                r#"
                mutation {{
                  setMyCatalogView(input: {{
                    deviceClass: {device_class}
                    facet: MOVIES
                    viewMode: {view_mode}
                    columns: [
                      {{ tableViewMode: COMPACT, columnId: "{column_id}", columnOrder: 0, visible: true }}
                    ]
                  }}) {{ {CATALOG_VIEW_SELECTION} }}
                }}
                "#
            ),
            Some(user_a.clone()),
        )
        .await;
        assert_no_errors(&saved);
    }

    // A whole-settings save that omits the language and columns keeps both.
    let language = schema_exec(
        &ctx,
        r#"
        mutation {
          setMyUiSettings(input: {
            theme: DARK
            highContrastMode: false
            reduceMotion: false
            hideSponsorButton: false
            density: COMFORTABLE
            sidebarMode: EXPANDED
            defaultLandingView: MOVIES
            language: "fra"
          }) { language }
        }
        "#,
        Some(user_a.clone()),
    )
    .await;
    assert_no_errors(&language);
    let theme_only = schema_exec(
        &ctx,
        r#"
        mutation {
          setMyUiSettings(input: {
            theme: LIGHT
            highContrastMode: false
            reduceMotion: false
            hideSponsorButton: false
            density: COMFORTABLE
            sidebarMode: EXPANDED
            defaultLandingView: MOVIES
          }) { theme }
        }
        "#,
        Some(user_a.clone()),
    )
    .await;
    assert_no_errors(&theme_only);

    let read_a = schema_exec(
        &ctx,
        &format!("query {{ myUiSettings {{ {CATALOG_VIEW_SELECTION} }} }}"),
        Some(user_a.clone()),
    )
    .await;
    assert_no_errors(&read_a);
    let settings = &read_a["data"]["myUiSettings"];
    assert_eq!(settings["language"], json!("fra"));
    assert_eq!(
        settings["catalogViews"],
        json!([
            { "deviceClass": "DESKTOP", "facet": "MOVIES", "viewMode": "POSTER_TABLE" },
            { "deviceClass": "MOBILE", "facet": "MOVIES", "viewMode": "POSTER" }
        ])
    );
    assert_eq!(
        settings["tableColumns"],
        json!([
            {
                "deviceClass": "DESKTOP",
                "facet": "MOVIES",
                "tableViewMode": "COMPACT",
                "columnId": "year",
                "columnOrder": 0,
                "visible": true
            },
            {
                "deviceClass": "MOBILE",
                "facet": "MOVIES",
                "tableViewMode": "COMPACT",
                "columnId": "size",
                "columnOrder": 0,
                "visible": true
            }
        ])
    );

    // The API only ever reads the caller's own preferences.
    let read_b = schema_exec(
        &ctx,
        &format!("query {{ myUiSettings {{ {CATALOG_VIEW_SELECTION} }} }}"),
        Some(user_b),
    )
    .await;
    assert_no_errors(&read_b);
    assert_eq!(read_b["data"]["myUiSettings"]["language"], json!(null));
    assert_eq!(read_b["data"]["myUiSettings"]["catalogViews"], json!([]));
    assert_eq!(read_b["data"]["myUiSettings"]["tableColumns"], json!([]));

    // An empty language clears it back to the browser default.
    let cleared = schema_exec(
        &ctx,
        r#"
        mutation {
          setMyUiSettings(input: {
            theme: LIGHT
            highContrastMode: false
            reduceMotion: false
            hideSponsorButton: false
            density: COMFORTABLE
            sidebarMode: EXPANDED
            defaultLandingView: MOVIES
            language: ""
          }) { language }
        }
        "#,
        Some(user_a),
    )
    .await;
    assert_no_errors(&cleared);
    assert_eq!(cleared["data"]["setMyUiSettings"]["language"], json!(null));
}

#[tokio::test]
async fn graphql_ui_settings_ignore_column_ids_this_version_no_longer_offers() {
    let ctx = TestContext::new().await;
    let user = create_ui_settings_test_user(&ctx, "catalog_retired_column_user").await;

    let saved = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              setMyCatalogView(input: {{
                deviceClass: DESKTOP
                facet: SERIES
                columns: [
                  {{ tableViewMode: COMPACT, columnId: "runtime", columnOrder: 0, visible: true }}
                ]
              }}) {{ {CATALOG_VIEW_SELECTION} }}
            }}
            "#
        ),
        Some(user.clone()),
    )
    .await;
    assert_no_errors(&saved);

    // A column written by an earlier version that this one has since removed.
    sqlx::query(
        "INSERT INTO user_ui_table_columns
         (user_id, device_class, facet, table_view_mode, column_id, column_order, visible)
         VALUES (?, 'desktop', 'series', 'compact', 'retiredColumn', 1, 1)",
    )
    .bind(&user.id)
    .execute(ctx.db.pool())
    .await
    .expect("seed retired column");

    let read = schema_exec(
        &ctx,
        &format!("query {{ myUiSettings {{ {CATALOG_VIEW_SELECTION} }} }}"),
        Some(user),
    )
    .await;
    assert_no_errors(&read);
    let columns = read["data"]["myUiSettings"]["tableColumns"]
        .as_array()
        .expect("table columns");
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0]["columnId"], json!("runtime"));
}

#[tokio::test]
async fn graphql_ui_settings_reject_oversized_language_and_unknown_catalog_column() {
    let ctx = TestContext::new().await;
    let user = create_ui_settings_test_user(&ctx, "catalog_validation_user").await;

    let language = schema_exec(
        &ctx,
        &format!(
            r#"
            mutation {{
              setMyUiSettings(input: {{
                theme: DARK
                highContrastMode: false
                reduceMotion: false
                hideSponsorButton: false
                density: COMFORTABLE
                sidebarMode: EXPANDED
                defaultLandingView: MOVIES
                language: "{}"
              }}) {{ language }}
            }}
            "#,
            "x".repeat(64)
        ),
        Some(user.clone()),
    )
    .await;
    let message = language["errors"][0]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("language must be"), "{language}");

    let column = schema_exec(
        &ctx,
        r#"
        mutation {
          setMyCatalogView(input: {
            deviceClass: MOBILE
            facet: ANIME
            columns: [
              { tableViewMode: POSTER_TABLE, columnId: "notAColumn", columnOrder: 0, visible: true }
            ]
          }) { language }
        }
        "#,
        Some(user.clone()),
    )
    .await;
    let message = column["errors"][0]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("unsupported poster-table table column"),
        "{column}"
    );

    // Neither rejected save left anything behind.
    let read = schema_exec(
        &ctx,
        &format!("query {{ myUiSettings {{ {CATALOG_VIEW_SELECTION} }} }}"),
        Some(user),
    )
    .await;
    assert_no_errors(&read);
    assert_eq!(read["data"]["myUiSettings"]["language"], json!(null));
    assert_eq!(read["data"]["myUiSettings"]["tableColumns"], json!([]));
}

const DASHBOARD_LAYOUT_SELECTION: &str = r#"
    dashboardLayouts { breakpoint columns items { panelId x y w h } }
"#;

async fn dashboard_exec(ctx: &TestContext, query: &str, variables: Value, user: &User) -> Value {
    let request = async_graphql::Request::new(query)
        .variables(async_graphql::Variables::from_json(variables))
        .data(user.clone());
    let response = ctx.schema.execute(request).await;
    serde_json::to_value(&response).expect("serialize gql response")
}

async fn set_dashboard_layout(ctx: &TestContext, user: &User, input: Value) -> Value {
    dashboard_exec(
        ctx,
        &format!(
            "mutation($input: SetMyDashboardLayoutInput!) {{
               setMyDashboardLayout(input: $input) {{ {DASHBOARD_LAYOUT_SELECTION} }}
             }}"
        ),
        json!({ "input": input }),
        user,
    )
    .await
}

async fn reset_dashboard_layout(ctx: &TestContext, user: &User, breakpoint: Value) -> Value {
    dashboard_exec(
        ctx,
        &format!(
            "mutation($breakpoint: UiDashboardBreakpoint) {{
               resetMyDashboardLayout(breakpoint: $breakpoint) {{ {DASHBOARD_LAYOUT_SELECTION} }}
             }}"
        ),
        json!({ "breakpoint": breakpoint }),
        user,
    )
    .await
}

async fn read_dashboard_layouts(ctx: &TestContext, user: &User) -> Value {
    let body = dashboard_exec(
        ctx,
        &format!("query {{ myUiSettings {{ {DASHBOARD_LAYOUT_SELECTION} }} }}"),
        json!({}),
        user,
    )
    .await;
    assert_no_errors(&body);
    body["data"]["myUiSettings"]["dashboardLayouts"].clone()
}

fn dashboard_panel(panel_id: &str, x: i32, y: i32, w: i32, h: i32) -> Value {
    json!({ "panelId": panel_id, "x": x, "y": y, "w": w, "h": h })
}

#[tokio::test]
async fn graphql_dashboard_layouts_save_replace_and_keep_empty_layouts() {
    let ctx = TestContext::new().await;
    let user = create_ui_settings_test_user(&ctx, "dashboard_layout_user").await;

    assert_eq!(read_dashboard_layouts(&ctx, &user).await, json!([]));

    let lg = set_dashboard_layout(
        &ctx,
        &user,
        json!({
            "breakpoint": "LG",
            "columns": 12,
            "items": [
                dashboard_panel("panel-beta", 6, 0, 6, 4),
                dashboard_panel("panel-alpha", 0, 0, 6, 4),
            ]
        }),
    )
    .await;
    assert_no_errors(&lg);
    let xs = set_dashboard_layout(
        &ctx,
        &user,
        json!({
            "breakpoint": "XS",
            "columns": 2,
            "items": [dashboard_panel("panel-alpha", 0, 0, 2, 3)]
        }),
    )
    .await;
    assert_no_errors(&xs);
    // The mutation returns the full settings, both breakpoints included.
    assert_eq!(
        xs["data"]["setMyDashboardLayout"]["dashboardLayouts"],
        read_dashboard_layouts(&ctx, &user).await
    );
    assert_eq!(
        read_dashboard_layouts(&ctx, &user).await,
        json!([
            {
                "breakpoint": "LG",
                "columns": 12,
                "items": [
                    dashboard_panel("panel-alpha", 0, 0, 6, 4),
                    dashboard_panel("panel-beta", 6, 0, 6, 4),
                ]
            },
            {
                "breakpoint": "XS",
                "columns": 2,
                "items": [dashboard_panel("panel-alpha", 0, 0, 2, 3)]
            }
        ])
    );

    // Saving a breakpoint again replaces its layout and leaves the other alone.
    let replaced = set_dashboard_layout(
        &ctx,
        &user,
        json!({
            "breakpoint": "LG",
            "columns": 10,
            "items": [dashboard_panel("panel-gamma", 2, 1, 8, 2)]
        }),
    )
    .await;
    assert_no_errors(&replaced);
    // An empty list is a saved layout, not a missing one.
    let emptied = set_dashboard_layout(
        &ctx,
        &user,
        json!({ "breakpoint": "SM", "columns": 6, "items": [] }),
    )
    .await;
    assert_no_errors(&emptied);
    assert_eq!(
        read_dashboard_layouts(&ctx, &user).await,
        json!([
            {
                "breakpoint": "LG",
                "columns": 10,
                "items": [dashboard_panel("panel-gamma", 2, 1, 8, 2)]
            },
            { "breakpoint": "SM", "columns": 6, "items": [] },
            {
                "breakpoint": "XS",
                "columns": 2,
                "items": [dashboard_panel("panel-alpha", 0, 0, 2, 3)]
            }
        ])
    );
}

#[tokio::test]
async fn graphql_dashboard_layouts_reset_one_breakpoint_or_all() {
    let ctx = TestContext::new().await;
    let user = create_ui_settings_test_user(&ctx, "dashboard_reset_user").await;

    // Resetting before anything was saved returns the defaults.
    let untouched = reset_dashboard_layout(&ctx, &user, Value::Null).await;
    assert_no_errors(&untouched);
    assert_eq!(
        untouched["data"]["resetMyDashboardLayout"]["dashboardLayouts"],
        json!([])
    );

    for (breakpoint, columns) in [("LG", 12), ("MD", 10), ("XS", 2)] {
        let saved = set_dashboard_layout(
            &ctx,
            &user,
            json!({
                "breakpoint": breakpoint,
                "columns": columns,
                "items": [dashboard_panel("panel-alpha", 0, 0, 2, 2)]
            }),
        )
        .await;
        assert_no_errors(&saved);
    }

    let reset_md = reset_dashboard_layout(&ctx, &user, json!("MD")).await;
    assert_no_errors(&reset_md);
    let remaining: Vec<Value> = read_dashboard_layouts(&ctx, &user)
        .await
        .as_array()
        .expect("layouts")
        .iter()
        .map(|layout| layout["breakpoint"].clone())
        .collect();
    assert_eq!(remaining, vec![json!("LG"), json!("XS")]);
    assert_eq!(
        reset_md["data"]["resetMyDashboardLayout"]["dashboardLayouts"],
        read_dashboard_layouts(&ctx, &user).await
    );

    let reset_all = reset_dashboard_layout(&ctx, &user, Value::Null).await;
    assert_no_errors(&reset_all);
    assert_eq!(
        reset_all["data"]["resetMyDashboardLayout"]["dashboardLayouts"],
        json!([])
    );
    assert_eq!(read_dashboard_layouts(&ctx, &user).await, json!([]));
}

#[tokio::test]
async fn graphql_dashboard_layouts_are_private_to_each_user() {
    let ctx = TestContext::new().await;
    let user_a = create_ui_settings_test_user(&ctx, "dashboard_owner_a").await;
    let user_b = create_ui_settings_test_user(&ctx, "dashboard_owner_b").await;

    let saved = set_dashboard_layout(
        &ctx,
        &user_a,
        json!({
            "breakpoint": "MD",
            "columns": 8,
            "items": [dashboard_panel("panel-alpha", 0, 0, 4, 2)]
        }),
    )
    .await;
    assert_no_errors(&saved);

    assert_eq!(read_dashboard_layouts(&ctx, &user_b).await, json!([]));

    // Another user's saves and resets never reach the first user's layouts.
    let other = set_dashboard_layout(
        &ctx,
        &user_b,
        json!({ "breakpoint": "MD", "columns": 4, "items": [] }),
    )
    .await;
    assert_no_errors(&other);
    let reset = reset_dashboard_layout(&ctx, &user_b, Value::Null).await;
    assert_no_errors(&reset);

    assert_eq!(
        read_dashboard_layouts(&ctx, &user_a).await,
        json!([
            {
                "breakpoint": "MD",
                "columns": 8,
                "items": [dashboard_panel("panel-alpha", 0, 0, 4, 2)]
            }
        ])
    );
    assert_eq!(read_dashboard_layouts(&ctx, &user_b).await, json!([]));
}

#[tokio::test]
async fn graphql_dashboard_layouts_refuse_invalid_layouts_with_reasons() {
    let ctx = TestContext::new().await;
    let user = create_ui_settings_test_user(&ctx, "dashboard_refusal_user").await;

    let too_many: Vec<Value> = (0..65)
        .map(|index| dashboard_panel(&format!("panel-{index}"), 0, index, 1, 1))
        .collect();
    let cases = [
        (
            json!({ "breakpoint": "LG", "columns": 0, "items": [] }),
            "SETTINGS_DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE",
        ),
        (
            json!({ "breakpoint": "LG", "columns": 25, "items": [] }),
            "SETTINGS_DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE",
        ),
        (
            json!({ "breakpoint": "LG", "columns": 12, "items": too_many }),
            "SETTINGS_DASHBOARD_LAYOUT_TOO_MANY_ITEMS",
        ),
        (
            json!({ "breakpoint": "LG", "columns": 12, "items": [dashboard_panel("", 0, 0, 1, 1)] }),
            "SETTINGS_DASHBOARD_PANEL_ID_INVALID",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel(&"p".repeat(65), 0, 0, 1, 1)]
            }),
            "SETTINGS_DASHBOARD_PANEL_ID_INVALID",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("Panel_Alpha", 0, 0, 1, 1)]
            }),
            "SETTINGS_DASHBOARD_PANEL_ID_INVALID",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [
                    dashboard_panel("panel-alpha", 0, 0, 1, 1),
                    dashboard_panel("panel-alpha", 2, 0, 1, 1),
                ]
            }),
            "SETTINGS_DASHBOARD_PANEL_ID_DUPLICATED",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("panel-alpha", -1, 0, 1, 1)]
            }),
            "SETTINGS_DASHBOARD_ITEM_POSITION_OUT_OF_RANGE",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("panel-alpha", 0, -1, 1, 1)]
            }),
            "SETTINGS_DASHBOARD_ITEM_POSITION_OUT_OF_RANGE",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("panel-alpha", 0, 0, 0, 1)]
            }),
            "SETTINGS_DASHBOARD_ITEM_SIZE_OUT_OF_RANGE",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("panel-alpha", 0, 0, 1, 0)]
            }),
            "SETTINGS_DASHBOARD_ITEM_SIZE_OUT_OF_RANGE",
        ),
        (
            json!({
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("panel-alpha", 8, 0, 5, 1)]
            }),
            "SETTINGS_DASHBOARD_ITEM_EXCEEDS_COLUMNS",
        ),
    ];

    for (input, reason) in cases {
        let body = set_dashboard_layout(&ctx, &user, input.clone()).await;
        assert_eq!(
            body["errors"][0]["extensions"]["reason"],
            json!(reason),
            "{input}: {body}"
        );
    }

    // A panel that exactly fills the last column is accepted.
    let edge = set_dashboard_layout(
        &ctx,
        &user,
        json!({
            "breakpoint": "LG",
            "columns": 12,
            "items": [dashboard_panel("panel-alpha", 7, 0, 5, 1)]
        }),
    )
    .await;
    assert_no_errors(&edge);
    // No refused save left a layout behind.
    assert_eq!(
        read_dashboard_layouts(&ctx, &user).await,
        json!([
            {
                "breakpoint": "LG",
                "columns": 12,
                "items": [dashboard_panel("panel-alpha", 7, 0, 5, 1)]
            }
        ])
    );
}
