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
