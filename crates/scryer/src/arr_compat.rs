use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::stream;
use scryer_application::{
    AppError, ExternalReleaseInput, ExternalReleaseProtocol, ExternalReleaseStatus,
    IndexerResponseAttributes, TitleListProjection,
};
use scryer_domain::{MediaFacet, Title, User};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::middleware::{AuthState, map_app_error, resolve_actor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    Sonarr,
    Radarr,
}

#[derive(Clone)]
struct CompatState {
    auth: AuthState,
    flavor: Flavor,
}

pub(crate) fn router(auth: AuthState) -> Router {
    let routes = |flavor, list_path| {
        Router::new()
            .route("/api/v3/system/status", get(status))
            .route("/api/v3/release/push", post(push))
            .route("/api/v3/tag", get(tags))
            .route(list_path, get(titles))
            .layer(DefaultBodyLimit::max(64 * 1024))
            .with_state(CompatState {
                auth: auth.clone(),
                flavor,
            })
    };
    Router::new()
        .nest("/compat/sonarr", routes(Flavor::Sonarr, "/api/v3/series"))
        .nest("/compat/radarr", routes(Flavor::Radarr, "/api/v3/movie"))
}

async fn actor(state: &CompatState, headers: &HeaderMap) -> Result<User, Response> {
    let key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    let mut auth_headers = HeaderMap::new();
    auth_headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| StatusCode::UNAUTHORIZED.into_response())?,
    );
    let resolved = resolve_actor(&state.auth, &auth_headers, None)
        .await
        .map_err(map_app_error)?
        .filter(|actor| actor.is_api_key())
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    Ok(resolved.user)
}

async fn status(State(state): State<CompatState>, headers: HeaderMap) -> Response {
    if let Err(error) = actor(&state, &headers).await {
        return error;
    }
    Json(json!({ "version": crate::VERSION })).into_response()
}

async fn tags(State(state): State<CompatState>, headers: HeaderMap) -> Response {
    if let Err(error) = actor(&state, &headers).await {
        return error;
    }
    Json(json!([])).into_response()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PushInput {
    title: String,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    magnet_url: Option<String>,
    #[serde(default)]
    size: Option<i64>,
    indexer: String,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    download_protocol: Option<String>,
    #[serde(default)]
    publish_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    imdb_id: Option<Value>,
    #[serde(default)]
    tmdb_id: Option<u64>,
    #[serde(default)]
    tvdb_id: Option<u64>,
    #[serde(default)]
    indexer_flags: u32,
    #[serde(default)]
    download_client_id: Option<i64>,
    #[serde(default)]
    download_client: Option<String>,
}

fn validation(property: &str, message: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!([{ "propertyName": property, "errorMessage": message.into(), "errorCode": "Invalid", "severity": "error" }]))).into_response()
}

impl PushInput {
    fn into_release(self, flavor: Flavor) -> Result<ExternalReleaseInput, Response> {
        if self.download_client_id.is_some_and(|id| id != 0)
            || self
                .download_client
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty())
        {
            return Err(validation(
                "downloadClientId",
                "Download-client overrides are unsupported; Scryer uses its configured routing",
            ));
        }
        let parse_protocol = |value: &str| match value.to_ascii_lowercase().as_str() {
            "usenet" => Ok(ExternalReleaseProtocol::Usenet),
            "torrent" => Ok(ExternalReleaseProtocol::Torrent),
            _ => Err(validation("protocol", "Protocol must be usenet or torrent")),
        };
        let protocol = parse_protocol(
            self.protocol
                .as_deref()
                .or(self.download_protocol.as_deref())
                .unwrap_or(""),
        )?;
        if let Some(other) = self.download_protocol.as_deref()
            && parse_protocol(other)? != protocol
        {
            return Err(validation("downloadProtocol", "Protocol fields disagree"));
        }
        let download_url = if protocol == ExternalReleaseProtocol::Torrent {
            self.magnet_url
                .filter(|value| !value.is_empty())
                .or(self.download_url)
        } else {
            self.download_url
        }
        .ok_or_else(|| validation("downloadUrl", "A download URL or magnet is required"))?;
        let imdb_id = match (flavor, self.imdb_id) {
            (_, None | Some(Value::Null)) => None,
            (Flavor::Sonarr, Some(Value::String(value))) if value.is_empty() => None,
            (Flavor::Sonarr, Some(Value::String(value)))
                if value.strip_prefix("tt").is_some_and(|digits| {
                    !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit())
                }) =>
            {
                Some(value)
            }
            (Flavor::Radarr, Some(Value::Number(value))) if value.as_u64().is_some() => value
                .as_u64()
                .filter(|value| *value > 0)
                .map(|value| format!("tt{value:07}")),
            _ => {
                return Err(validation(
                    "imdbId",
                    "IMDb identity has the wrong format for this adapter",
                ));
            }
        };
        let flag_bits: &[(u32, &str)] = match flavor {
            Flavor::Sonarr => &[
                (1, "freeleech"),
                (2, "halfleech"),
                (4, "double_upload"),
                (8, "internal"),
                (16, "scene"),
                (32, "freeleech_75"),
                (64, "freeleech_25"),
                (128, "nuked"),
                (256, "subtitles"),
            ],
            Flavor::Radarr => &[
                (1, "freeleech"),
                (2, "halfleech"),
                (4, "double_upload"),
                (8, "golden"),
                (16, "approved"),
                (32, "internal"),
                (128, "scene"),
                (256, "freeleech_75"),
                (512, "freeleech_25"),
                (2048, "nuked"),
            ],
        };
        let supported = flag_bits.iter().fold(0, |mask, (bit, _)| mask | bit);
        if self.indexer_flags & !supported != 0 {
            return Err(validation("indexerFlags", "Unsupported release flags"));
        }
        Ok(ExternalReleaseInput {
            name: self.title,
            download_url,
            protocol,
            source: self.indexer,
            size_bytes: self.size,
            published_at: self.publish_date,
            external_ids: IndexerResponseAttributes {
                tvdb_id: self.tvdb_id.filter(|id| *id > 0).map(|id| id.to_string()),
                tmdb_id: self.tmdb_id.filter(|id| *id > 0).map(|id| id.to_string()),
                imdb_id,
                categories: vec![],
            },
            flags: flag_bits
                .iter()
                .filter(|(bit, _)| self.indexer_flags & bit != 0)
                .map(|(_, flag)| (*flag).to_owned())
                .collect(),
        })
    }
}

async fn push(
    State(state): State<CompatState>,
    headers: HeaderMap,
    body: Result<Json<PushInput>, JsonRejection>,
) -> Response {
    let actor = match actor(&state, &headers).await {
        Ok(actor) => actor,
        Err(error) => return error,
    };
    let body = match body {
        Ok(Json(body)) => body,
        Err(error) => return validation("release", error.body_text()),
    };
    let input = match body.into_release(state.flavor) {
        Ok(input) => input,
        Err(error) => return error,
    };
    let facets = match state.flavor {
        Flavor::Sonarr => vec![MediaFacet::Series, MediaFacet::Anime],
        Flavor::Radarr => vec![MediaFacet::Movie],
    };
    match state.auth.app.submit_external_release(&actor, input, &facets).await {
        Ok(outcome) => Json(json!([{
            "approved": outcome.status == ExternalReleaseStatus::Queued,
            "rejected": matches!(outcome.status, ExternalReleaseStatus::Rejected | ExternalReleaseStatus::Duplicate),
            "temporarilyRejected": outcome.status == ExternalReleaseStatus::Held,
            "rejections": outcome.reasons,
        }])).into_response(),
        Err(AppError::Validation(message)) => validation("release", message),
        Err(error) => map_app_error(error),
    }
}

fn title_json(title: Title) -> Value {
    let external = |source: &str| {
        title
            .external_ids
            .iter()
            .find(|id| id.source == source)
            .and_then(|id| id.value.parse::<u64>().ok())
    };
    json!({
        "title": title.name, "monitored": title.monitored,
        "alternateTitles": title.aliases.iter().map(|alias| json!({"title": alias})).collect::<Vec<_>>(),
        "tvdbId": external("tvdb"), "tmdbId": external("tmdb"), "imdbId": title.imdb_id,
        "tags": [],
    })
}

async fn titles(State(state): State<CompatState>, headers: HeaderMap) -> Response {
    let actor = match actor(&state, &headers).await {
        Ok(actor) => actor,
        Err(error) => return error,
    };
    // Each body chunk contains at most one bounded catalog page. A read error
    // aborts the stream rather than returning a silently truncated JSON array.
    let pages = stream::try_unfold(
        (state, actor, 0usize, false, false),
        |(state, actor, offset, started, done)| async move {
            if done {
                return Ok::<_, std::io::Error>(None);
            }
            let page = state
                .auth
                .app
                .list_titles(
                    &actor,
                    None,
                    None,
                    None,
                    Default::default(),
                    Default::default(),
                    300,
                    offset,
                    TitleListProjection {
                        include_external_ids: true,
                        include_canonical_tags: false,
                    },
                    Default::default(),
                )
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let mut chunk = if offset == 0 {
                "[".to_owned()
            } else {
                String::new()
            };
            let mut started = started;
            for title in page.items {
                let included = match state.flavor {
                    Flavor::Sonarr => matches!(title.facet, MediaFacet::Series | MediaFacet::Anime),
                    Flavor::Radarr => title.facet == MediaFacet::Movie,
                };
                if !included {
                    continue;
                }
                if started {
                    chunk.push(',');
                }
                chunk.push_str(&title_json(title).to_string());
                started = true;
            }
            if !page.has_more {
                chunk.push(']');
            }
            Ok(Some((
                chunk,
                (state, actor, offset + 300, started, !page.has_more),
            )))
        },
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        Body::from_stream(pages),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{
        AuthlessWebClientProofState, WebSocketOriginPolicy, integration_test_common::TestContext,
    };
    use crate::rate_limit::ScryerRateLimiter;
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    fn input(flags: u32, imdb_id: Value) -> PushInput {
        serde_json::from_value(json!({"title":"Example.2024.1080p.WEB-DL", "downloadUrl":"https://releases.invalid/example.nzb", "protocol":"usenet", "indexer":"External feed", "indexerFlags":flags, "imdbId":imdb_id})).unwrap()
    }

    #[test]
    fn arr_compat_translates_distinct_flags_and_imdb_formats() {
        let sonarr = input(8 | 16 | 128, json!("tt0123456"))
            .into_release(Flavor::Sonarr)
            .unwrap();
        let radarr = input(32 | 128 | 2048, json!(123456))
            .into_release(Flavor::Radarr)
            .unwrap();
        assert_eq!(sonarr.flags, radarr.flags);
        assert_eq!(sonarr.external_ids, radarr.external_ids);
        assert_eq!(
            input(8, Value::Null)
                .into_release(Flavor::Radarr)
                .unwrap()
                .flags,
            ["golden"]
        );
        assert!(
            input(1024, Value::Null)
                .into_release(Flavor::Sonarr)
                .is_err()
        );
        assert!(
            input(0, json!(123456))
                .into_release(Flavor::Sonarr)
                .is_err()
        );
        assert!(
            input(0, json!("tt0123456"))
                .into_release(Flavor::Radarr)
                .is_err()
        );
    }

    #[test]
    fn arr_compat_rejects_client_overrides_and_conflicting_protocols() {
        let mut request = input(0, Value::Null);
        request.download_client_id = Some(1);
        assert!(request.into_release(Flavor::Sonarr).is_err());
        let mut request = input(0, Value::Null);
        request.download_protocol = Some("torrent".into());
        assert!(request.into_release(Flavor::Sonarr).is_err());
    }

    fn test_router(context: &TestContext) -> Router {
        router(AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            auth_runtime: context.auth_runtime.clone(),
            rate_limiter: ScryerRateLimiter::from_env(Default::default()),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        })
    }

    #[tokio::test]
    async fn arr_compat_requires_keys_in_authless_mode_and_honors_revocation() {
        tokio::time::timeout(std::time::Duration::from_secs(120), async {
            let context = TestContext::new().await;
            let router = test_router(&context);
            let admin = context.app.find_or_create_default_user().await.unwrap();
            let key = context
                .app
                .create_api_key(
                    &admin,
                    scryer_application::CreateApiKey {
                        label: "Fixture".into(),
                        expiry: scryer_application::ApiKeyExpiryPreset::Never,
                    },
                )
                .await
                .unwrap();
            let request = |key: Option<&str>| {
                let mut request = Request::builder().uri("/compat/sonarr/api/v3/system/status");
                if let Some(key) = key {
                    request = request.header("X-Api-Key", key);
                }
                request.body(Body::empty()).unwrap()
            };
            assert_eq!(
                router
                    .clone()
                    .oneshot(request(None))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            let response = router
                .clone()
                .oneshot(request(Some(&key.raw_key)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(value["version"], crate::VERSION);
            context
                .app
                .revoke_api_key(&admin, &key.api_key.id)
                .await
                .unwrap();
            assert_eq!(
                router
                    .oneshot(request(Some(&key.raw_key)))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        })
        .await
        .expect("bounded compatibility fixture");
    }

    #[tokio::test]
    #[ignore = "requires an explicitly supplied isolated autobrr application driver"]
    async fn arr_compat_real_autobrr_application() {
        use scryer_application::{
            DownloadSubmissionRepository, PendingReleaseRepository, SettingsRepository,
        };
        use scryer_infrastructure_workflow::workflow::stores::DownloadSubmissionStore;
        use std::sync::{Arc, Mutex};
        use wiremock::{
            Mock, ResponseTemplate,
            matchers::{method, path},
        };

        tokio::time::timeout(std::time::Duration::from_secs(240), async {
            let driver = std::env::var_os("SCRYER_AUTOBRR_APPLICATION_DRIVER")
                .expect("supply the isolated application driver");
            let context = TestContext::new().await;
            let admin = context.app.find_or_create_default_user().await.unwrap();
            let key = context.app.create_api_key(&admin, scryer_application::CreateApiKey {
                label: "Autobrr fixture".into(), expiry: scryer_application::ApiKeyExpiryPreset::Never,
            }).await.unwrap();
            let jobs = Arc::new(Mutex::new(Vec::<Value>::new()));
            let jobs_rpc = jobs.clone();
            let unexpected_rpc = Arc::new(Mutex::new(Vec::<String>::new()));
            let unexpected_rpc_handler = unexpected_rpc.clone();
            Mock::given(method("POST")).and(path("/jsonrpc")).respond_with(move |request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let result = match body["method"].as_str().unwrap() {
                    "version" => json!("25.3"),
                    "status" => json!({"DownloadPaused":false,"DownloadRate":0}),
                    "listgroups" => json!(jobs_rpc.lock().unwrap().clone()),
                    "history" | "postqueue" => json!([]),
                    "append" => {
                        let mut jobs = jobs_rpc.lock().unwrap();
                        let id = jobs.len() + 1;
                        jobs.push(json!({"NZBID":id,"NZBName":body["params"][0],"Status":"DOWNLOADING","FileSizeMB":2000,"RemainingSizeMB":2000}));
                        json!(id)
                    }
                    other => {
                        unexpected_rpc_handler.lock().unwrap().push(other.to_owned());
                        return ResponseTemplate::new(500);
                    }
                };
                ResponseTemplate::new(200).set_body_json(json!({"version":"1.1","result":result}))
            }).mount(&context.nzbget_server).await;
            context.app.create_download_client_config(&admin, scryer_domain::NewDownloadClientConfig {
                name:"Fixture NZBGet".into(), client_type:"nzbget".into(),
                config_json:json!({"base_url":context.nzbget_server.uri()}).to_string(),
                client_priority:1,is_enabled:true,proxy_config_id:None,
            }).await.unwrap();
            context.app.create_title_tag_definition(&admin,"external-delay",None).await.unwrap();
            let mut title_ids = Vec::new();
            for (name, facet, monitored, tags) in [
                ("Fixture Movie",MediaFacet::Movie,true,vec![]),
                ("Fixture Delayed",MediaFacet::Movie,true,vec!["external-delay".to_owned()]),
                ("Fixture Series",MediaFacet::Series,true,vec![]),
                ("Fixture Unmonitored",MediaFacet::Movie,false,vec![]),
            ] {
                let series = facet == MediaFacet::Series;
                let title = context.app.add_title(&admin,scryer_domain::NewTitle {
                    name:name.into(),facet,monitored,tags,..Default::default()
                }).await.unwrap();
                if series {
                    let season = context.app.create_collection(&admin,title.id.clone(),"season".into(),"1".into(),Some("Season 1".into()),None,Some("1".into()),Some("1".into())).await.unwrap();
                    context.app.create_episode(&admin,title.id.clone(),Some(season.id),"standard".into(),Some("1".into()),Some("1".into()),Some("S01E01".into()),Some("Fixture episode".into()),Some("2024-01-01".into()),Some(1440),false,false).await.unwrap();
                }
                title_ids.push(title.id);
            }
            for number in 0..301 {
                context.app.add_title(&admin,scryer_domain::NewTitle {
                    name:format!("Catalog Movie {number:04}"),facet:MediaFacet::Movie,monitored:number % 2 == 0,..Default::default()
                }).await.unwrap();
            }
            crate::settings_bootstrap::seed_service_setting_definitions(context.settings_store.clone()).await.unwrap();
            context.settings_store.upsert_setting_json("system","acquisition.delay_profiles",None,
                json!([{"id":"external-delay","name":"Fixture delay","usenet_delay_minutes":120,"tags":["external-delay"]}]).to_string(),"test",None).await.unwrap();
            let metadata_before = context.smg_server.received_requests().await.unwrap().len();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = test_router(&context);
            let server = tokio::spawn(async move { axum::serve(listener,router).await });
            let result = tokio::process::Command::new(driver)
                .env("INTAKE_HOST",format!("http://{address}"))
                .env("INTAKE_KEY",&key.raw_key).kill_on_drop(true).output().await.unwrap();
            server.abort();
            assert!(result.status.success(),"{}\n{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            eprintln!("{}",String::from_utf8_lossy(&result.stdout));
            assert!(unexpected_rpc.lock().unwrap().is_empty(),"unexpected RPCs: {:?}",unexpected_rpc.lock().unwrap());
            assert_eq!(jobs.lock().unwrap().len(),2,"only accepted movie and episode reach the client");
            let submissions = DownloadSubmissionStore::new(context.db.datastore());
            for (id, expected) in title_ids.iter().zip([1,0,1,0]) {
                assert_eq!(submissions.list_for_title(id).await.unwrap().len(),expected,"submission count for {id}");
            }
            let pending = context.library_state.list_waiting_pending_releases().await.unwrap();
            assert_eq!(pending.len(),1);
            assert_eq!(pending[0].title_id,title_ids[1]);
            assert_eq!(context.smg_server.received_requests().await.unwrap().len(),metadata_before,"intake must not hydrate metadata");
        }).await.expect("bounded real autobrr fixture");
    }

    #[tokio::test]
    async fn arr_compat_streams_multiple_catalog_pages_and_rejects_unknown_pushes() {
        tokio::time::timeout(std::time::Duration::from_secs(120), async {
            let context = TestContext::new().await;
            let admin = context.app.find_or_create_default_user().await.unwrap();
            let key = context.app.create_api_key(&admin, scryer_application::CreateApiKey { label:"Fixture".into(), expiry:scryer_application::ApiKeyExpiryPreset::Never }).await.unwrap();
            for number in 0..301 {
                context.app.add_title(&admin, scryer_domain::NewTitle { name:format!("Catalog Movie {number:04}"), facet:MediaFacet::Movie, monitored:number % 2 == 0, ..Default::default() }).await.unwrap();
            }
            let router = test_router(&context);
            let response = router.clone().oneshot(Request::builder().uri("/compat/radarr/api/v3/movie").header("X-Api-Key", &key.raw_key).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let values: Vec<Value> = serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
            assert_eq!(values.len(), 301);
            assert_eq!(values.iter().filter(|value| value["monitored"] == true).count(), 151);
            assert!(values.iter().all(|value| value.get("id").is_none()));
            let response = router.clone().oneshot(Request::builder().method("POST").uri("/compat/radarr/api/v3/release/push").header("X-Api-Key", &key.raw_key).header(header::CONTENT_TYPE, "application/json").body(Body::from(json!({"title":"Unknown.2024.1080p.WEB-DL", "downloadUrl":"https://releases.invalid/unknown.nzb", "indexer":"Fixture", "protocol":"usenet"}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let value: Value = serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
            assert_eq!(value[0]["rejected"], true);
            assert!(!value[0]["rejections"].as_array().unwrap().is_empty());
            let native = context.graphql_json(r#"mutation { submitExternalRelease(input: { name: "Unknown.2024.1080p.WEB-DL", downloadUrl: "https://releases.invalid/unknown.nzb", source: "Fixture", protocol: USENET }) { status reasons } }"#, json!({}), None).await;
            assert_eq!(native["data"]["submitExternalRelease"]["status"], "REJECTED", "{native}");
            assert_eq!(native["data"]["submitExternalRelease"]["reasons"], value[0]["rejections"]);
            // An optional external driver exercises the unmodified upstream
            // clients without adding Go or autobrr to Scryer's dependencies.
            if let Some(driver) = std::env::var_os("SCRYER_AUTOBRR_CONTRACT_DRIVER") {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let server = tokio::spawn(async move { axum::serve(listener, router).await });
                let result = tokio::process::Command::new(driver)
                    .env("INTAKE_HOST", format!("http://{address}"))
                    .env("INTAKE_KEY", &key.raw_key)
                    .kill_on_drop(true).output().await.unwrap();
                server.abort();
                assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            }
        }).await.expect("bounded catalog fixture");
    }
}
