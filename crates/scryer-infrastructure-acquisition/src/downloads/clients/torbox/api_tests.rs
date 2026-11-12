use std::time::Duration;

use scryer_application::{AppError, PluginFailureReason, torbox_refusal};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::api::*;

const TOKEN: &str = "tb-synthetic-token-0001";
const CDN_URL: &str = "https://cdn.synthetic.test/dl/abc123?sig=secretsig";

fn api(server: &MockServer) -> TorboxApi {
    TorboxApi::new(scryer_outbound_http::generic_reqwest_client(), TOKEN)
        .with_base_url(server.uri())
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true,
        "error": null,
        "detail": "Done.",
        "data": data,
    }))
}

fn refused(status: u16, code: &str, detail: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "success": false,
        "error": code,
        "detail": detail,
        "data": null,
    }))
}

async fn only_request(server: &MockServer) -> Request {
    let mut requests = server.received_requests().await.expect("recording is on");
    assert_eq!(requests.len(), 1, "exactly one request is sent");
    requests.remove(0)
}

/// The text value of a multipart text field, if the body has one.
fn form_text(body: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\"\r\n\r\n");
    let start = body.find(&marker)? + marker.len();
    let end = body[start..].find("\r\n")? + start;
    Some(body[start..end].to_string())
}

fn magnet_create() -> TorboxTorrentCreate {
    TorboxTorrentCreate {
        source: TorboxTorrentSource::Magnet(
            "magnet:?xt=urn:btih:0000000000000000000000000000000000000001&dn=Synthetic.Fixture"
                .to_string(),
        ),
        seed: Some(TorboxSeedPreference::DoNotSeed),
        allow_zip: false,
        name: Some("Synthetic.Fixture.One".to_string()),
        as_queued: None,
        add_only_if_cached: false,
    }
}

fn nzb_create() -> TorboxUsenetCreate {
    TorboxUsenetCreate {
        source: TorboxUsenetSource::File {
            file_name: "synthetic-fixture.nzb".to_string(),
            bytes: b"<nzb>synthetic</nzb>".to_vec(),
        },
        name: Some("Synthetic.Fixture.Two".to_string()),
        password: Some("synthetic-archive-pass".to_string()),
        post_processing: None,
        as_queued: Some(true),
        add_only_if_cached: false,
    }
}

// --- request shapes -------------------------------------------------------

#[tokio::test]
async fn user_me_sends_the_bearer_token_and_reads_plan_leniently() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ok(json!({
            "id": 9,
            "plan": 2,
            "email": "member@synthetic.test",
            "some_future_field": {"nested": true},
        })))
        .expect(1)
        .mount(&server)
        .await;

    let user = api(&server).user_me().await.expect("user");
    assert_eq!(user.plan, Some(TorboxPlan::Pro));
    assert_eq!(user.email.as_deref(), Some("member@synthetic.test"));
    let debug = format!("{user:?}");
    assert!(!debug.contains("member@synthetic.test"), "{debug}");
}

#[tokio::test]
async fn user_me_without_a_plan_or_with_an_unknown_one_still_decodes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .respond_with(ok(json!({"plan": "7"})))
        .mount(&server)
        .await;
    let user = api(&server).user_me().await.expect("user");
    assert_eq!(user.plan, Some(TorboxPlan::Unknown(7)));
    assert_eq!(user.email, None);

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .respond_with(ok(json!({})))
        .mount(&server)
        .await;
    let user = api(&server).user_me().await.expect("user");
    assert_eq!(user.plan, None);
}

#[tokio::test]
async fn create_torrent_from_a_magnet_sends_the_documented_form() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ok(json!({
            "hash": "0000000000000000000000000000000000000001",
            "torrent_id": 42,
            "auth_id": "auth-synthetic",
            "unexpected": [1, 2, 3],
        })))
        .mount(&server)
        .await;

    let created = api(&server)
        .create_torrent(magnet_create())
        .await
        .expect("created");
    assert_eq!(
        created,
        TorboxCreated {
            id: 42,
            hash: Some("0000000000000000000000000000000000000001".to_string()),
            auth_id: Some("auth-synthetic".to_string()),
            cached: None,
        }
    );

    let request = only_request(&server).await;
    let content_type = request
        .headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );
    let body = String::from_utf8_lossy(&request.body);
    assert!(
        form_text(&body, "magnet").is_some_and(|magnet| magnet.starts_with("magnet:?xt=urn:btih:")),
        "{body}"
    );
    assert_eq!(form_text(&body, "seed").as_deref(), Some("3"));
    assert_eq!(form_text(&body, "allow_zip").as_deref(), Some("false"));
    assert_eq!(
        form_text(&body, "name").as_deref(),
        Some("Synthetic.Fixture.One")
    );
    assert_eq!(
        form_text(&body, "add_only_if_cached").as_deref(),
        Some("false")
    );
    assert_eq!(form_text(&body, "as_queued"), None);
    assert!(!body.contains("name=\"file\""), "{body}");
    assert!(!body.contains("name=\"all\""), "{body}");
}

#[tokio::test]
async fn create_torrent_from_a_file_uploads_it_as_the_file_part() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(ok(json!({"torrent_id": "43", "cached": true})))
        .mount(&server)
        .await;

    let created = api(&server)
        .create_torrent(TorboxTorrentCreate {
            source: TorboxTorrentSource::File {
                file_name: "synthetic-fixture.torrent".to_string(),
                bytes: b"d8:announce0:e".to_vec(),
            },
            seed: None,
            allow_zip: true,
            name: None,
            as_queued: Some(false),
            add_only_if_cached: true,
        })
        .await
        .expect("created");
    assert_eq!(created.id, 43);
    assert_eq!(created.cached, Some(true));

    let request = only_request(&server).await;
    let body = String::from_utf8_lossy(&request.body);
    assert!(
        body.contains("name=\"file\"; filename=\"synthetic-fixture.torrent\""),
        "{body}"
    );
    assert!(
        body.contains("Content-Type: application/x-bittorrent"),
        "{body}"
    );
    assert!(body.contains("d8:announce0:e"), "{body}");
    assert!(!body.contains("name=\"magnet\""), "{body}");
    assert_eq!(form_text(&body, "seed"), None);
    assert_eq!(form_text(&body, "allow_zip").as_deref(), Some("true"));
    assert_eq!(form_text(&body, "as_queued").as_deref(), Some("false"));
    assert_eq!(
        form_text(&body, "add_only_if_cached").as_deref(),
        Some("true")
    );
}

#[tokio::test]
async fn create_usenet_from_a_file_sends_the_documented_form() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ok(json!({
            "hash": "synthetic-usenet-hash",
            "usenetdownload_id": 77,
            "auth_id": "auth-synthetic",
        })))
        .mount(&server)
        .await;

    let created = api(&server)
        .create_usenet(nzb_create())
        .await
        .expect("created");
    assert_eq!(created.id, 77);
    assert_eq!(created.hash.as_deref(), Some("synthetic-usenet-hash"));

    let request = only_request(&server).await;
    let body = String::from_utf8_lossy(&request.body);
    assert!(
        body.contains("name=\"file\"; filename=\"synthetic-fixture.nzb\""),
        "{body}"
    );
    assert!(body.contains("Content-Type: application/x-nzb"), "{body}");
    assert!(body.contains("<nzb>synthetic</nzb>"), "{body}");
    assert_eq!(
        form_text(&body, "name").as_deref(),
        Some("Synthetic.Fixture.Two")
    );
    assert_eq!(
        form_text(&body, "password").as_deref(),
        Some("synthetic-archive-pass")
    );
    assert_eq!(form_text(&body, "post_processing"), None);
    assert_eq!(form_text(&body, "as_queued").as_deref(), Some("true"));
    assert_eq!(
        form_text(&body, "add_only_if_cached").as_deref(),
        Some("false")
    );
    assert!(!body.contains("name=\"link\""), "{body}");
}

#[tokio::test]
async fn create_usenet_from_a_link_sends_link_and_a_set_post_processing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(ok(json!({"usenet_id": 78})))
        .mount(&server)
        .await;

    let created = api(&server)
        .create_usenet(TorboxUsenetCreate {
            source: TorboxUsenetSource::Link(
                "https://indexer.synthetic.test/get/1?apikey=indexer-secret".to_string(),
            ),
            name: None,
            password: None,
            post_processing: Some(-1),
            as_queued: None,
            add_only_if_cached: false,
        })
        .await
        .expect("created");
    assert_eq!(created.id, 78);

    let request = only_request(&server).await;
    let body = String::from_utf8_lossy(&request.body);
    assert_eq!(
        form_text(&body, "link").as_deref(),
        Some("https://indexer.synthetic.test/get/1?apikey=indexer-secret")
    );
    assert_eq!(form_text(&body, "post_processing").as_deref(), Some("-1"));
    assert_eq!(form_text(&body, "password"), None);
    assert_eq!(form_text(&body, "name"), None);
    assert!(!body.contains("name=\"file\""), "{body}");
}

#[tokio::test]
async fn create_usenet_reads_the_id_under_any_of_its_candidate_names() {
    for (field, id) in [("usenetdownload_id", 1), ("usenet_id", 2), ("id", 3)] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/api/usenet/createusenetdownload"))
            .respond_with(ok(Value::Object(
                [(field.to_string(), Value::from(id))].into_iter().collect(),
            )))
            .mount(&server)
            .await;
        let created = api(&server)
            .create_usenet(nzb_create())
            .await
            .expect("created");
        assert_eq!(created.id, id, "{field}");
    }
}

#[tokio::test]
async fn list_sends_paging_and_decodes_items_leniently() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(query_param("bypass_cache", "true"))
        .and(query_param("offset", "100"))
        .and(query_param("limit", "50"))
        .respond_with(ok(json!([
            {
                "id": 11,
                "hash": "0000000000000000000000000000000000000011",
                "name": "Synthetic.Fixture.Three",
                "size": 1048576,
                "active": true,
                "download_state": "completed",
                "seeds": 4,
                "peers": "2",
                "ratio": 0.5,
                "progress": 1,
                "download_speed": 0,
                "upload_speed": 1024,
                "eta": 0,
                "download_present": false,
                "download_finished": true,
                "cached": true,
                "seed_torrent": true,
                "long_term_seeding": false,
                "private": false,
                "tracker_message": null,
                "expires_at": "2026-12-01T00:00:00Z",
                "never_documented": "ignored",
                "files": [
                    {
                        "id": 0,
                        "name": "Synthetic.Fixture.Three/part.mkv",
                        "short_name": "part.mkv",
                        "size": 1048576,
                        "mimetype": "video/x-matroska",
                        "zipped": false,
                        "infected": false,
                        "absolute_path": "/synthetic/abs/part.mkv",
                        "md5": null,
                        "s3_path": "ignored"
                    }
                ]
            },
            {"id": "12", "files": null}
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let items = api(&server)
        .list(
            TorboxItemKind::Torrent,
            TorboxListQuery {
                bypass_cache: true,
                offset: Some(100),
                limit: Some(50),
            },
        )
        .await
        .expect("items");
    assert_eq!(items.len(), 2);

    let first = &items[0];
    assert_eq!(first.id, 11);
    assert_eq!(first.peers, Some(2));
    assert_eq!(first.progress, Some(1.0));
    assert_eq!(first.tracker_message, None);
    assert_eq!(first.download_state.as_deref(), Some("completed"));
    // "completed" is not the signal; the file is not present yet.
    assert!(!first.is_downloaded());
    assert_eq!(first.files.len(), 1);
    assert_eq!(first.files[0].id, 0);
    assert_eq!(first.files[0].short_name.as_deref(), Some("part.mkv"));

    let second = &items[1];
    assert_eq!(second.id, 12);
    assert_eq!(second.name, None);
    assert!(!second.active);
    assert!(second.files.is_empty());
}

#[tokio::test]
async fn list_without_paging_sends_only_bypass_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/mylist"))
        .respond_with(ok(Value::Null))
        .mount(&server)
        .await;
    let items = api(&server)
        .list(TorboxItemKind::Usenet, TorboxListQuery::default())
        .await
        .expect("items");
    assert!(items.is_empty());
    let request = only_request(&server).await;
    assert_eq!(request.url.query(), Some("bypass_cache=false"));
}

#[tokio::test]
async fn an_unreadable_item_is_skipped_and_the_rest_of_the_list_is_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/mylist"))
        .respond_with(ok(json!([
            {"id": 1},
            {"name": "Synthetic.Fixture.Four"},
            {"id": "not-a-number"},
            {"id": 3, "private": true}
        ])))
        .mount(&server)
        .await;
    let items = api(&server)
        .list(TorboxItemKind::Usenet, TorboxListQuery::default())
        .await
        .expect("the readable items");
    assert_eq!(
        items.iter().map(|item| item.id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(items[0].private, None, "an answer that says nothing");
    assert_eq!(items[1].private, Some(true));
}

#[tokio::test]
async fn item_reads_the_single_object_form() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/mylist"))
        .and(query_param("id", "7"))
        .and(query_param("bypass_cache", "true"))
        .respond_with(ok(json!({
            "id": 7,
            "name": "Synthetic.Fixture.Five",
            "download_present": true,
            "download_finished": true,
            "download_state": "cached",
        })))
        .mount(&server)
        .await;
    let item = api(&server)
        .item(TorboxItemKind::Usenet, 7, true)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(item.id, 7);
    assert!(item.is_downloaded());
}

#[tokio::test]
async fn item_reports_absence_for_null_not_found_and_an_unmatched_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .and(query_param("id", "1"))
        .respond_with(ok(Value::Null))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .and(query_param("id", "2"))
        .respond_with(refused(400, "ITEM_NOT_FOUND", "No such item."))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .and(query_param("id", "3"))
        .respond_with(ok(
            json!([{"id": 4}, {"id": 3, "name": "Synthetic.Fixture.Six"}]),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .and(query_param("id", "5"))
        .respond_with(ok(json!([{"id": 4}])))
        .mount(&server)
        .await;

    let api = api(&server);
    assert_eq!(
        api.item(TorboxItemKind::Torrent, 1, false).await.unwrap(),
        None
    );
    assert_eq!(
        api.item(TorboxItemKind::Torrent, 2, false).await.unwrap(),
        None
    );
    assert_eq!(
        api.item(TorboxItemKind::Torrent, 3, false)
            .await
            .unwrap()
            .and_then(|item| item.name),
        Some("Synthetic.Fixture.Six".to_string())
    );
    assert_eq!(
        api.item(TorboxItemKind::Torrent, 5, false).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn request_download_puts_the_token_in_the_query_and_hides_the_link() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/requestdl"))
        .and(query_param("token", TOKEN))
        .and(query_param("torrent_id", "42"))
        .and(query_param("file_id", "3"))
        .and(query_param("zip_link", "false"))
        .and(query_param("redirect", "false"))
        .respond_with(ok(json!(CDN_URL)))
        .expect(1)
        .mount(&server)
        .await;

    let link = api(&server)
        .request_download(TorboxItemKind::Torrent, 42, TorboxDownloadTarget::File(3))
        .await
        .expect("link");
    assert_eq!(link.expose(), CDN_URL);
    let debug = format!("{link:?}");
    assert!(!debug.contains("cdn.synthetic.test"), "{debug}");
    assert!(!debug.contains("secretsig"), "{debug}");

    let request = only_request(&server).await;
    assert!(
        request.headers.get("authorization").is_none(),
        "requestdl takes the token in the query only"
    );
}

#[tokio::test]
async fn request_download_of_a_usenet_zip_names_no_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/requestdl"))
        .and(query_param("usenet_id", "77"))
        .and(query_param("zip_link", "true"))
        .and(query_param("redirect", "false"))
        .respond_with(ok(json!(CDN_URL)))
        .mount(&server)
        .await;
    api(&server)
        .request_download(TorboxItemKind::Usenet, 77, TorboxDownloadTarget::Zip)
        .await
        .expect("link");
    let request = only_request(&server).await;
    let query = request.url.query().unwrap_or_default().to_string();
    assert!(!query.contains("file_id"), "{query}");
}

#[tokio::test]
async fn request_download_refuses_an_answer_that_is_not_a_link() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/requestdl"))
        .respond_with(ok(json!("not a url")))
        .mount(&server)
        .await;
    let error = api(&server)
        .request_download(TorboxItemKind::Torrent, 1, TorboxDownloadTarget::Zip)
        .await
        .expect_err("not a link");
    assert!(matches!(error, TorboxError::Decode { .. }), "{error:?}");
    assert!(!error.to_string().contains("not a url"));
}

#[tokio::test]
async fn control_calls_send_exactly_one_id_and_never_all() {
    let cases: [(TorboxItemKind, &str, Value); 5] = [
        (
            TorboxItemKind::Torrent,
            "/v1/api/torrents/controltorrent",
            json!({"torrent_id": 5, "operation": "delete"}),
        ),
        (
            TorboxItemKind::Usenet,
            "/v1/api/usenet/controlusenetdownload",
            json!({"usenet_id": 5, "operation": "delete"}),
        ),
        (
            TorboxItemKind::Torrent,
            "/v1/api/torrents/controltorrent",
            json!({"torrent_id": 5, "operation": "resume"}),
        ),
        (
            TorboxItemKind::Usenet,
            "/v1/api/usenet/controlusenetdownload",
            json!({"usenet_id": 5, "operation": "pause"}),
        ),
        (
            TorboxItemKind::Torrent,
            "/v1/api/torrents/controltorrent",
            json!({"torrent_id": 5, "operation": "reannounce"}),
        ),
    ];
    for (index, (kind, endpoint, expected)) in cases.into_iter().enumerate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(endpoint))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .and(body_json(&expected))
            .respond_with(ok(Value::Null))
            .expect(1)
            .mount(&server)
            .await;
        let api = api(&server);
        match index {
            0 | 1 => api.delete(kind, 5).await,
            2 => api.resume(kind, 5).await,
            3 => api.pause_usenet(5).await,
            _ => api.reannounce_torrent(5).await,
        }
        .expect("controlled");
        let request = only_request(&server).await;
        let body: Value = serde_json::from_slice(&request.body).expect("json body");
        let keys = body.as_object().expect("object").keys().collect::<Vec<_>>();
        assert_eq!(keys.len(), 2, "{body}");
        assert!(body.get("all").is_none(), "{body}");
    }
}

// --- error mapping --------------------------------------------------------

enum Expect {
    Refused(&'static str),
    NotFound(&'static str),
    Temporary(&'static str, Option<Duration>),
    Failed(PluginFailureReason),
}

fn assert_maps(error: AppError, expect: &Expect, label: &str) {
    match (expect, &error) {
        (Expect::Refused(reason), AppError::ValidationRefused { .. }) => {
            assert_eq!(error.reason(), Some(*reason), "{label}");
        }
        (Expect::NotFound(reason), AppError::NotFoundReason { .. }) => {
            assert_eq!(error.reason(), Some(*reason), "{label}");
        }
        (Expect::Temporary(reason, delay), AppError::TemporaryUnavailable { retry_after, .. }) => {
            assert_eq!(error.reason(), Some(*reason), "{label}");
            assert_eq!(retry_after, delay, "{label}");
        }
        (Expect::Failed(reason), AppError::PluginFailed { failure, .. }) => {
            assert_eq!(&failure.reason, reason, "{label}");
        }
        _ => panic!("{label}: unexpected mapping {error:?}"),
    }
}

#[tokio::test]
async fn every_documented_code_maps_to_its_reason() {
    let server_default = Some(TORBOX_DEFAULT_SERVER_FAULT_RETRY);
    let cases: Vec<(u16, &str, Expect)> = vec![
        (
            403,
            "NO_AUTH",
            Expect::Refused(torbox_refusal::TOKEN_REJECTED),
        ),
        (
            403,
            "BAD_TOKEN",
            Expect::Refused(torbox_refusal::TOKEN_REJECTED),
        ),
        (
            403,
            "AUTH_ERROR",
            Expect::Refused(torbox_refusal::TOKEN_REJECTED),
        ),
        (
            403,
            "PLAN_RESTRICTED_FEATURE",
            Expect::Refused(torbox_refusal::PLAN_RESTRICTED),
        ),
        (
            400,
            "ACTIVE_LIMIT",
            Expect::Temporary(torbox_refusal::ACTIVE_LIMIT, None),
        ),
        (
            400,
            "COOLDOWN_LIMIT",
            Expect::Temporary(torbox_refusal::COOLDOWN_LIMIT, None),
        ),
        (
            400,
            "MONTHLY_LIMIT",
            Expect::Refused(torbox_refusal::MONTHLY_LIMIT),
        ),
        (
            400,
            "DOWNLOAD_TOO_LARGE",
            Expect::Refused(torbox_refusal::DOWNLOAD_TOO_LARGE),
        ),
        (
            400,
            "BOZO_TORRENT",
            Expect::Refused(torbox_refusal::INVALID_TORRENT),
        ),
        (
            400,
            "BOZO_NZB",
            Expect::Refused(torbox_refusal::INVALID_NZB),
        ),
        (
            400,
            "DUPLICATE_ITEM",
            Expect::Refused(torbox_refusal::DUPLICATE_ITEM),
        ),
        (
            400,
            "LINK_OFFLINE",
            Expect::Refused(torbox_refusal::LINK_OFFLINE),
        ),
        (
            400,
            "ITEM_NOT_FOUND",
            Expect::NotFound(torbox_refusal::ITEM_NOT_FOUND),
        ),
        (
            500,
            "DATABASE_ERROR",
            Expect::Temporary(torbox_refusal::UNAVAILABLE, server_default),
        ),
        (
            500,
            "UNKNOWN_ERROR",
            Expect::Temporary(torbox_refusal::UNAVAILABLE, server_default),
        ),
        (
            500,
            "NO_SERVERS_AVAILABLE_ERROR",
            Expect::Temporary(torbox_refusal::UNAVAILABLE, server_default),
        ),
        (
            500,
            "DOWNLOAD_SERVER_ERROR",
            Expect::Temporary(torbox_refusal::UNAVAILABLE, server_default),
        ),
        // An undocumented code is still a server fault by TorBox's suffix
        // convention, even on a 400.
        (
            400,
            "FUTURE_STORAGE_ERROR",
            Expect::Temporary(torbox_refusal::UNAVAILABLE, server_default),
        ),
        (
            400,
            "VENDOR_DISABLED",
            Expect::Failed(PluginFailureReason::UpstreamUnavailable),
        ),
        (
            400,
            "INVALID_OPTION",
            Expect::Failed(PluginFailureReason::Permanent),
        ),
        (
            400,
            "ENDPOINT_NOT_FOUND",
            Expect::Failed(PluginFailureReason::Permanent),
        ),
        (
            400,
            "MISSING_REQUIRED_OPTION",
            Expect::Failed(PluginFailureReason::Permanent),
        ),
        (
            400,
            "TOO_MANY_OPTIONS",
            Expect::Failed(PluginFailureReason::Permanent),
        ),
        (
            400,
            "TOO_MUCH_DATA",
            Expect::Failed(PluginFailureReason::Permanent),
        ),
        (
            400,
            "SOMETHING_NEW",
            Expect::Failed(PluginFailureReason::Permanent),
        ),
    ];

    let server = MockServer::start().await;
    let api = api(&server);
    for (status, code, expect) in &cases {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/v1/api/torrents/createtorrent"))
            .respond_with(refused(*status, code, "A synthetic explanation."))
            .mount(&server)
            .await;
        let error = api
            .create_torrent(magnet_create())
            .await
            .expect_err("refused");
        assert_eq!(error.code().map(TorboxErrorCode::as_str), Some(*code));
        let sentence = error.to_string();
        assert!(sentence.contains("A synthetic explanation."), "{sentence}");
        assert_maps(error.into_app_error(), expect, code);
    }
}

#[tokio::test]
async fn a_duplicate_create_is_a_typed_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(refused(400, "DUPLICATE_ITEM", "Already added."))
        .mount(&server)
        .await;
    let error = api(&server)
        .create_usenet(nzb_create())
        .await
        .expect_err("duplicate");
    assert_eq!(error.code(), Some(&TorboxErrorCode::DuplicateItem));
    assert!(!error.outcome_unknown());
    assert_eq!(
        AppError::from(error).reason(),
        Some(torbox_refusal::DUPLICATE_ITEM)
    );
}

#[tokio::test]
async fn uncoded_refusals_map_by_status() {
    let server = MockServer::start().await;
    let api = api(&server);
    let cases = [
        (
            ResponseTemplate::new(403).set_body_string("Forbidden"),
            Expect::Refused(torbox_refusal::TOKEN_REJECTED),
        ),
        (
            ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"),
            Expect::Temporary(
                torbox_refusal::UNAVAILABLE,
                Some(TORBOX_DEFAULT_SERVER_FAULT_RETRY),
            ),
        ),
        (
            ResponseTemplate::new(503).insert_header("retry-after", "30"),
            Expect::Temporary(torbox_refusal::UNAVAILABLE, Some(Duration::from_secs(30))),
        ),
        (
            ResponseTemplate::new(404).set_body_string("missing"),
            Expect::Failed(PluginFailureReason::Permanent),
        ),
    ];
    for (index, (response, expect)) in cases.into_iter().enumerate() {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/v1/api/user/me"))
            .respond_with(response)
            .mount(&server)
            .await;
        let error = api.user_me().await.expect_err("refused");
        assert_maps(error.into_app_error(), &expect, &format!("case {index}"));
    }
}

#[tokio::test]
async fn a_429_honors_retry_after_and_falls_back_without_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .mount(&server)
        .await;
    let error = api(&server).user_me().await.expect_err("limited");
    match &error {
        TorboxError::RateLimited { retry_after, .. } => {
            assert_eq!(*retry_after, Some(Duration::from_secs(120)));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_maps(
        error.into_app_error(),
        &Expect::Temporary(torbox_refusal::RATE_LIMITED, Some(Duration::from_secs(120))),
        "with retry-after",
    );

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "success": false,
            "error": null,
            "detail": "Slow down.",
            "data": null,
        })))
        .mount(&server)
        .await;
    let error = api(&server).user_me().await.expect_err("limited");
    assert!(
        matches!(
            error,
            TorboxError::RateLimited {
                retry_after: None,
                ..
            }
        ),
        "{error:?}"
    );
    assert_maps(
        error.into_app_error(),
        &Expect::Temporary(
            torbox_refusal::RATE_LIMITED,
            Some(TORBOX_DEFAULT_RATE_LIMIT_RETRY),
        ),
        "without retry-after",
    );
}

#[tokio::test]
async fn a_cached_only_create_that_torbox_declines_is_not_cached() {
    // How TorBox declines is undocumented; both plausible shapes are read.
    let declines = [
        ok(Value::Null),
        ResponseTemplate::new(200).set_body_json(json!({
            "success": false,
            "error": null,
            "detail": "Not cached.",
            "data": null,
        })),
    ];
    for decline in declines {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/api/torrents/createtorrent"))
            .respond_with(decline)
            .mount(&server)
            .await;
        let mut request = magnet_create();
        request.add_only_if_cached = true;
        let error = api(&server)
            .create_torrent(request)
            .await
            .expect_err("declined");
        assert!(matches!(error, TorboxError::NotCached { .. }), "{error:?}");
        assert_maps(
            error.into_app_error(),
            &Expect::Refused(torbox_refusal::NOT_CACHED),
            "not cached",
        );
    }
}

#[tokio::test]
async fn a_create_answer_without_an_id_is_an_unknown_outcome() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(ok(json!({"hash": "only-a-hash"})))
        .mount(&server)
        .await;
    let error = api(&server)
        .create_torrent(magnet_create())
        .await
        .expect_err("no id");
    assert!(matches!(error, TorboxError::Decode { .. }), "{error:?}");
    assert!(error.outcome_unknown());
}

#[tokio::test]
async fn a_success_status_without_the_envelope_is_a_decode_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>portal</html>"))
        .mount(&server)
        .await;
    let error = api(&server).user_me().await.expect_err("not json");
    assert!(matches!(error, TorboxError::Decode { .. }), "{error:?}");
    assert_maps(
        error.into_app_error(),
        &Expect::Failed(PluginFailureReason::UpstreamUnavailable),
        "decode",
    );
}

// --- redaction ------------------------------------------------------------

#[test]
fn debug_output_hides_every_secret() {
    let api = TorboxApi::new(scryer_outbound_http::generic_reqwest_client(), TOKEN);
    let debug = format!("{api:?}");
    assert!(!debug.contains(TOKEN), "{debug}");

    let usenet = TorboxUsenetCreate {
        source: TorboxUsenetSource::Link(
            "https://indexer.synthetic.test/get/1?apikey=indexer-secret".to_string(),
        ),
        ..nzb_create()
    };
    let debug = format!("{usenet:?}");
    assert!(!debug.contains("indexer-secret"), "{debug}");
    assert!(!debug.contains("synthetic-archive-pass"), "{debug}");

    let debug = format!("{:?}", magnet_create());
    assert!(!debug.contains("magnet:?"), "{debug}");
}

#[tokio::test]
async fn a_token_echoed_in_detail_is_redacted_from_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/user/me"))
        .respond_with(refused(
            403,
            "BAD_TOKEN",
            &format!("The token {TOKEN} is not valid."),
        ))
        .mount(&server)
        .await;
    let error = api(&server).user_me().await.expect_err("refused");
    assert!(!error.to_string().contains(TOKEN), "{error}");
    assert!(!format!("{error:?}").contains(TOKEN), "{error:?}");
    let app_error = error.into_app_error();
    assert!(!app_error.to_string().contains(TOKEN), "{app_error}");
    assert!(!format!("{app_error:?}").contains(TOKEN), "{app_error:?}");
}

#[tokio::test]
async fn a_transport_failure_on_requestdl_names_neither_the_url_nor_the_token() {
    // A port nothing listens on: bound, read, and released.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address");
    drop(listener);

    let api = TorboxApi::new(scryer_outbound_http::generic_reqwest_client(), TOKEN)
        .with_base_url(format!("http://{address}"));
    let error = api
        .request_download(TorboxItemKind::Torrent, 1, TorboxDownloadTarget::Zip)
        .await
        .expect_err("nothing listens");
    match &error {
        TorboxError::Transport { kind, .. } => {
            assert_eq!(*kind, TorboxTransportKind::NotSent);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(!error.outcome_unknown());
    for text in [error.to_string(), format!("{error:?}")] {
        assert!(!text.contains(TOKEN), "{text}");
        assert!(!text.contains("requestdl"), "{text}");
    }
    let app_error = error.into_app_error();
    assert!(!app_error.to_string().contains(TOKEN), "{app_error}");
    assert!(
        matches!(
            app_error.plugin_failure().map(|failure| &failure.reason),
            Some(PluginFailureReason::UpstreamUnavailable)
        ),
        "{app_error:?}"
    );
}

#[test]
fn every_known_code_round_trips() {
    for code in [
        "DATABASE_ERROR",
        "UNKNOWN_ERROR",
        "NO_AUTH",
        "BAD_TOKEN",
        "AUTH_ERROR",
        "INVALID_OPTION",
        "ENDPOINT_NOT_FOUND",
        "ITEM_NOT_FOUND",
        "PLAN_RESTRICTED_FEATURE",
        "DUPLICATE_ITEM",
        "TOO_MUCH_DATA",
        "DOWNLOAD_TOO_LARGE",
        "MISSING_REQUIRED_OPTION",
        "TOO_MANY_OPTIONS",
        "BOZO_TORRENT",
        "NO_SERVERS_AVAILABLE_ERROR",
        "MONTHLY_LIMIT",
        "COOLDOWN_LIMIT",
        "ACTIVE_LIMIT",
        "DOWNLOAD_SERVER_ERROR",
        "BOZO_NZB",
        "LINK_OFFLINE",
        "VENDOR_DISABLED",
    ] {
        let parsed = TorboxErrorCode::parse(code);
        assert!(
            !matches!(parsed, TorboxErrorCode::Other(_)),
            "{code} is a known code"
        );
        assert_eq!(parsed.as_str(), code);
    }
    assert!(!TorboxErrorCode::AuthError.is_server_fault());
    assert_eq!(
        TorboxErrorCode::parse("NEW_THING"),
        TorboxErrorCode::Other("NEW_THING".to_string())
    );
}
