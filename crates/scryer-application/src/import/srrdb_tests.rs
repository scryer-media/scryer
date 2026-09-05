use super::*;
use serde_json::json;
use std::io::Write;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Synthetic scene-style fixture names only.
const MEMBER_NAME: &str = "harbor.pals.s01e02.1080p.web.h264-lanterns.mkv";
const RELEASE_NAME: &str = "Harbor.Pals.S01E02.1080p.WEB.H264-LANTERNS";
const MEMBER_CRC: &str = "CBF43926";
const MEMBER_SIZE: u64 = 1_234_567;

fn lookup_for(server: &MockServer) -> SrrdbHttpFilenameLookup {
    SrrdbHttpFilenameLookup::new(
        reqwest::Url::parse(&format!("{}/v1", server.uri())).expect("base url"),
    )
}

async fn mount_search(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_details(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/details/{RELEASE_NAME}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn single_result_search() -> serde_json::Value {
    json!({
        "results": [{ "release": RELEASE_NAME, "size": 1_500_000_000u64 }],
        "resultsCount": 1,
        "warnings": [],
        "query": [MEMBER_CRC],
    })
}

fn details_with(archived: serde_json::Value) -> serde_json::Value {
    json!({
        "name": RELEASE_NAME,
        "files": [
            { "name": "lanterns-harborpals102.rar", "size": 15_000_000u64, "crc": "0053CA13" },
            { "name": "lanterns-harborpals102.nfo", "size": 2_048u64, "crc": "1A2B3C4D" }
        ],
        "archived-files": archived,
        "adds": [],
    })
}

fn accepted_member() -> serde_json::Value {
    json!([{ "name": MEMBER_NAME, "size": MEMBER_SIZE, "crc": MEMBER_CRC }])
}

// ── Gate predicate ──────────────────────────────────────────────────────────

#[test]
fn srrdb_lookup_applies_only_to_the_unpacking_usenet_clients() {
    for (enabled, client_type, expected) in [
        (false, "sabnzbd", false),
        (true, "sabnzbd", true),
        (true, "NZBGet", true),
        (true, "weaver", false),
        (true, "qbittorrent", false),
        (true, "some-plugin", false),
        (true, "", false),
    ] {
        assert_eq!(
            srrdb_lookup_applies(enabled, client_type),
            expected,
            "enabled={enabled} client_type={client_type:?}"
        );
    }
}

// ── CRC helper ──────────────────────────────────────────────────────────────

#[test]
fn crc32_iso_hdlc_matches_the_check_vector() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("check.bin");
    std::fs::write(&file, b"123456789").expect("write");

    let (crc, size) = crc32_iso_hdlc_of_file(&file).expect("crc");
    assert_eq!(format!("{crc:08X}"), "CBF43926");
    assert_eq!(size, 9);
}

#[test]
fn crc32_iso_hdlc_streams_multiple_buffers_like_a_one_shot_digest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("large.bin");
    let payload: Vec<u8> = (0..(SRRDB_CRC_CHUNK_BYTES * 2 + 4096))
        .map(|index| (index % 251) as u8)
        .collect();
    {
        let mut handle = std::fs::File::create(&file).expect("create");
        handle.write_all(&payload).expect("write");
    }

    let (crc, size) = crc32_iso_hdlc_of_file(&file).expect("crc");
    assert_eq!(size as usize, payload.len());

    let mut one_shot = crc_fast::Digest::new(crc_fast::CrcAlgorithm::Crc32IsoHdlc);
    one_shot.update(&payload);
    assert_eq!(crc, one_shot.finalize() as u32);
}

#[test]
fn crc32_iso_hdlc_reports_an_error_for_a_missing_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(crc32_iso_hdlc_of_file(&dir.path().join("absent.bin")).is_err());
}

// ── Adapter ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn recover_filename_accepts_the_single_matching_extracted_member() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    mount_details(&server, details_with(accepted_member())).await;

    let recovered = lookup_for(&server)
        .recover_filename(MEMBER_CRC, MEMBER_SIZE)
        .await
        .expect("no outage");
    assert_eq!(recovered.as_deref(), Some(MEMBER_NAME));

    // The CRC travels as a literal path segment on the search endpoint.
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        requests[0].url.path(),
        format!("/v1/search/archive-crc:{MEMBER_CRC}")
    );
    assert_eq!(
        requests[1].url.path(),
        format!("/v1/details/{RELEASE_NAME}")
    );
}

#[tokio::test]
async fn recover_filename_misses_when_the_release_only_lists_rar_volumes() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    mount_details(&server, details_with(json!([]))).await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_misses_when_the_member_size_disagrees() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    mount_details(
        &server,
        details_with(json!([
            { "name": MEMBER_NAME, "size": MEMBER_SIZE + 1, "crc": MEMBER_CRC }
        ])),
    )
    .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_misses_on_a_crc_collision() {
    let server = MockServer::start().await;
    mount_search(
        &server,
        json!({
            "results": [
                { "release": RELEASE_NAME },
                { "release": "Paper.Lantern.S02E05.1080p.WEB.H264-HARBOR" }
            ],
            "resultsCount": 2,
        }),
    )
    .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
    // The details endpoint is never reached.
    assert_eq!(server.received_requests().await.expect("requests").len(), 1);
}

#[tokio::test]
async fn recover_filename_misses_when_details_answer_for_another_release() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/details/{RELEASE_NAME}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "Paper.Lantern.S02E05.1080p.WEB.H264-HARBOR",
            "files": [],
            "archived-files": accepted_member(),
            "adds": [],
        })))
        .mount(&server)
        .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_misses_when_two_members_share_the_crc_and_size() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    mount_details(
        &server,
        details_with(json!([
            { "name": MEMBER_NAME, "size": MEMBER_SIZE, "crc": MEMBER_CRC },
            { "name": "harbor.pals.s01e03.1080p.web.h264-lanterns.mkv", "size": MEMBER_SIZE, "crc": MEMBER_CRC }
        ])),
    )
    .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_rejects_path_bearing_names() {
    for name in [
        "sub/harbor.pals.s01e02.mkv",
        "sub\\harbor.pals.s01e02.mkv",
        "../harbor.pals.s01e02.mkv",
        ".harbor.pals.s01e02.mkv",
    ] {
        let server = MockServer::start().await;
        mount_search(&server, single_result_search()).await;
        mount_details(
            &server,
            details_with(json!([{ "name": name, "size": MEMBER_SIZE, "crc": MEMBER_CRC }])),
        )
        .await;

        assert_eq!(
            lookup_for(&server)
                .recover_filename(MEMBER_CRC, MEMBER_SIZE)
                .await
                .expect("no outage"),
            None,
            "name={name}"
        );
    }
}

#[tokio::test]
async fn recover_filename_rejects_non_video_names() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    mount_details(
        &server,
        details_with(json!([
            { "name": "harbor.pals.s01e02.nfo", "size": MEMBER_SIZE, "crc": MEMBER_CRC }
        ])),
    )
    .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_rejects_oversized_names() {
    let server = MockServer::start().await;
    mount_search(&server, single_result_search()).await;
    let oversized = format!("{}.mkv", "h".repeat(SRRDB_MAX_RECOVERED_NAME_BYTES));
    mount_details(
        &server,
        details_with(json!([
            { "name": oversized, "size": MEMBER_SIZE, "crc": MEMBER_CRC }
        ])),
    )
    .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_misses_on_malformed_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(ResponseTemplate::new(200).set_body_string("{not json"))
        .mount(&server)
        .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_reports_an_outage_on_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    assert!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recover_filename_reports_an_outage_when_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    assert!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recover_filename_reports_an_outage_when_the_call_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(single_result_search())
                .set_delay(SRRDB_REQUEST_TIMEOUT + Duration::from_secs(2)),
        )
        .mount(&server)
        .await;

    assert!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recover_filename_misses_when_the_body_exceeds_its_cap() {
    let server = MockServer::start().await;
    let padding = "p".repeat(SRRDB_SEARCH_BODY_CAP_BYTES + 1024);
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "release": RELEASE_NAME }],
            "resultsCount": 1,
            "warnings": [padding],
        })))
        .mount(&server)
        .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}

#[tokio::test]
async fn recover_filename_misses_on_a_redirect() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/search/archive-crc:{MEMBER_CRC}")))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "https://example.invalid/moved"),
        )
        .mount(&server)
        .await;

    assert_eq!(
        lookup_for(&server)
            .recover_filename(MEMBER_CRC, MEMBER_SIZE)
            .await
            .expect("no outage"),
        None
    );
}
