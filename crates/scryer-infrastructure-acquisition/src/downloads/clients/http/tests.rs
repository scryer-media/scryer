use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue};
use scryer_application::{
    DownloadClient, DownloadClientAddRequest, DownloadSourceKind, http_release_refusal,
};
use scryer_domain::{DownloadClientConfig, MediaFacet, Title};
use scryer_plugin_sdk::{
    HttpAvailability, HttpResourceRole, HttpSubtitleMode, HttpTransport, PluginHttpDownloadSource,
    PluginHttpHeader, PluginHttpPolicy, PluginHttpRelease, PluginHttpResource, PluginHttpSelection,
    PluginHttpSubtitleTrack, PluginHttpValidators, PluginHttpVideoTrack,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::selection::{MAX_MEDIA_FALLBACKS, plan_download, validate_output_basename};
use super::state::{
    HttpFileRole, HttpJob, HttpJobFile, HttpJobState, HttpQueueDocument, HttpStateStore,
    RecordedValidator, RefusedVariant, part_path_for,
};
use super::transfer::{
    RangeRequest, ResponseVerdict, TransferError, check_final_size, judge_response,
    parse_content_range, parse_retry_after, resume_range, scoped_headers, start_new_attempt,
};
use super::*;

const BODY: &[u8] = b"synthetic-media-payload-0123456789";
const ETAG: &str = "\"synthetic-v1\"";
/// A failure bound only; every wait below ends on a state change.
const WAIT_BOUND: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// fixtures

fn resource(url: &str, role: HttpResourceRole) -> PluginHttpResource {
    PluginHttpResource {
        transport: HttpTransport::DirectFile,
        role,
        url: url.to_string(),
        expires_at: None,
        headers: Vec::new(),
        auth_ref: None,
        size_bytes: None,
        size_estimate: None,
        validators: PluginHttpValidators::default(),
        container: None,
        video: None,
        audio: None,
        subtitle: None,
        separate_tracks: false,
        range_support: Default::default(),
    }
}

fn muxed(url: &str, height: u32) -> PluginHttpResource {
    PluginHttpResource {
        container: Some("mp4".into()),
        video: Some(PluginHttpVideoTrack {
            height: Some(height),
            ..Default::default()
        }),
        ..resource(url, HttpResourceRole::Muxed)
    }
}

fn subtitle(url: &str, language: &str) -> PluginHttpResource {
    PluginHttpResource {
        subtitle: Some(PluginHttpSubtitleTrack {
            language: Some(language.into()),
            role: None,
            format: Some("vtt".into()),
        }),
        ..resource(url, HttpResourceRole::Subtitle)
    }
}

fn source(resources: Vec<PluginHttpResource>) -> PluginHttpDownloadSource {
    PluginHttpDownloadSource {
        release: PluginHttpRelease {
            provider: "synthetic".into(),
            item_id: "item-1".into(),
            page_url: "https://synthetic.invalid/watch/item-1".into(),
            resources,
            ..Default::default()
        },
        selected: PluginHttpSelection::default(),
        policy: PluginHttpPolicy {
            output_basename: "Synthetic.Show.S01E01".into(),
            ..Default::default()
        },
    }
}

fn title() -> Title {
    let facet = MediaFacet::Series;
    Title {
        id: "title-synthetic".to_string(),
        name: "Synthetic Show".to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        facet,
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/series"),
        created_by: None,
        created_at: Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        movie_release_dates: None,
        folder_path: None,
    }
}

fn request(source: PluginHttpDownloadSource) -> DownloadClientAddRequest {
    let mut request = DownloadClientAddRequest::from_legacy(
        &title(),
        None,
        Some(DownloadSourceKind::HttpResource),
        Some("Synthetic.Show.S01E01.WEB".into()),
        None,
        None,
    );
    request.download_id = Some(scryer_domain::download_identity::DownloadId::new());
    request.http = Some(source);
    request
}

fn config(download_dir: &Path, extra: serde_json::Value) -> DownloadClientConfig {
    let mut json = serde_json::json!({ "download_dir": download_dir.to_string_lossy() });
    if let (Some(target), Some(extra)) = (json.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    DownloadClientConfig {
        id: format!("synthetic-{}", uuid::Uuid::new_v4()),
        name: "Synthetic HTTP".to_string(),
        client_type: CLIENT_TYPE.to_string(),
        config_json: json.to_string(),
        client_priority: 1,
        is_enabled: true,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        proxy_config_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn client(config: &DownloadClientConfig) -> HttpDownloadClient {
    HttpDownloadClient::for_config(config, None, scryer_outbound_http::generic_reqwest_client())
        .expect("client builds")
}

/// Wait until the job satisfies `done`, re-checking on every queue change.
async fn wait_for_job(
    client: &HttpDownloadClient,
    id: &str,
    done: impl Fn(&HttpJob) -> bool,
) -> HttpJob {
    let engine = Arc::clone(client.engine());
    let mut changes = engine.subscribe();
    tokio::time::timeout(WAIT_BOUND, async {
        loop {
            if let Some(job) = engine.job(id).await.expect("queue readable")
                && done(&job)
            {
                return job;
            }
            changes.changed().await.expect("engine alive");
        }
    })
    .await
    .expect("the job reached the awaited state")
}

fn seeded_job(download_dir: &Path, url: &str, bytes_done: u64) -> HttpJob {
    let output_dir = download_dir.join("Synthetic.Show.S01E01");
    HttpJob {
        id: "seeded-item".into(),
        download_id: Some(uuid::Uuid::new_v4().to_string()),
        title_id: "title-synthetic".into(),
        title_name: "Synthetic Show".into(),
        facet: "series".into(),
        release_name: "Synthetic.Show.S01E01.WEB".into(),
        category: None,
        output_basename: "Synthetic.Show.S01E01".into(),
        files: vec![HttpJobFile {
            role: HttpFileRole::Media,
            resource: muxed(url, 720),
            final_path: output_dir.join("Synthetic.Show.S01E01.mp4"),
            partial_path: None,
            publication: None,
            attempt: 1,
            bytes_done,
            total_bytes: Some(BODY.len() as u64),
            validator: Some(RecordedValidator {
                etag: Some(ETAG.into()),
                last_modified: None,
            }),
            complete: false,
            language: None,
            forbidden_answers: 0,
            skipped_error: None,
            skipped_reason: None,
        }],
        output_dir,
        // As a crash mid-transfer leaves it.
        state: HttpJobState::Downloading,
        error: None,
        error_reason: None,
        expected_size_bytes: None,
        expected_duration_seconds: None,
        verify: Default::default(),
        queued_at: Utc::now(),
        updated_at: Utc::now(),
        completed_at: None,
        media_fallbacks: Vec::new(),
        candidate_index: 0,
        forbidden_requests: 0,
        refused_variants: Vec::new(),
        geo_restricted: false,
        subtitle_required: false,
        progress_revision: 0,
    }
}

/// Write the queue document and the partial file an interrupted transfer
/// leaves behind.
async fn seed_interrupted(config: &DownloadClientConfig, download_dir: &Path, job: &HttpJob) {
    let file = &job.files[0];
    tokio::fs::create_dir_all(&job.output_dir).await.unwrap();
    tokio::fs::write(file.part_path(), &BODY[..file.bytes_done as usize])
        .await
        .unwrap();
    HttpStateStore::for_client(download_dir, &config.id)
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn resume_request(offset: u64, total: u64) -> RangeRequest {
    RangeRequest::From {
        offset,
        total,
        validator: RecordedValidator {
            etag: Some(ETAG.into()),
            last_modified: None,
        },
    }
}

// ---------------------------------------------------------------------------
// configuration

#[test]
fn settings_default_limits_and_require_an_absolute_directory() {
    let settings = HttpClientSettings::from_config_json(r#"{"download_dir":"/srv/synthetic"}"#)
        .expect("valid");
    assert_eq!(settings.download_dir, PathBuf::from("/srv/synthetic"));
    assert_eq!(settings.limits.max_concurrent_downloads, 2);
    assert_eq!(settings.limits.max_connections_per_host, 2);
    assert_eq!(settings.limits.bandwidth_limit_kbps, None);

    let tuned = HttpClientSettings::from_config_json(
        r#"{"download_dir":"/srv/synthetic","max_concurrent_downloads":"4","max_connections_per_host":1,"bandwidth_limit_kbps":512}"#,
    )
    .expect("valid");
    assert_eq!(tuned.limits.max_concurrent_downloads, 4);
    assert_eq!(tuned.limits.max_connections_per_host, 1);
    assert_eq!(tuned.limits.bandwidth_limit_kbps, Some(512));

    assert!(HttpClientSettings::from_config_json("{}").is_err());
    assert!(HttpClientSettings::from_config_json(r#"{"download_dir":"relative/dir"}"#).is_err());
    assert!(
        HttpClientSettings::from_config_json(
            r#"{"download_dir":"/srv/synthetic","max_concurrent_downloads":9}"#
        )
        .is_err()
    );
    assert!(
        HttpClientSettings::from_config_json(
            r#"{"download_dir":"/srv/synthetic","max_concurrent_downloads":0}"#
        )
        .is_err()
    );
}

#[tokio::test]
async fn connection_test_checks_the_directory_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!({ "download_dir": dir.path() }).to_string();
    test_http_download_dir(&json)
        .await
        .expect("writable directory passes");
    // Repeating the test is harmless and leaves the same single marker.
    test_http_download_dir(&json).await.expect("repeat passes");
    let state_entries: Vec<_> = std::fs::read_dir(dir.path().join(".scryer-http"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(state_entries, vec![std::ffi::OsString::from("write-check")]);

    let missing = serde_json::json!({ "download_dir": dir.path().join("absent") }).to_string();
    assert!(test_http_download_dir(&missing).await.is_err());

    let file_path = dir.path().join("plain-file");
    std::fs::write(&file_path, b"x").unwrap();
    let not_dir = serde_json::json!({ "download_dir": file_path }).to_string();
    assert!(test_http_download_dir(&not_dir).await.is_err());
}

// ---------------------------------------------------------------------------
// state

#[test]
fn part_paths_name_each_attempt_separately() {
    let final_path = Path::new("/srv/dl/Show/Show.mp4");
    assert_eq!(
        part_path_for(final_path, 1),
        Path::new("/srv/dl/Show/Show.mp4.part")
    );
    assert_eq!(
        part_path_for(final_path, 2),
        Path::new("/srv/dl/Show/Show.mp4.2.part")
    );
}

#[test]
fn http_queue_errors_keep_codes_and_explanations_separate() {
    for code in [
        None,
        Some("HTTP_FUTURE_CONDITION"),
        Some(http_release_refusal::UNKNOWN_CONTAINER),
    ] {
        let mut job = seeded_job(Path::new("/synthetic"), "https://media.invalid/a.mp4", 0);
        job.state = HttpJobState::Failed;
        job.error = Some("Readable failure explanation".into());
        job.error_reason = code.map(str::to_string);
        let item = job_to_queue_item(&job, None);
        assert_eq!(
            item.attention_reason.as_deref(),
            Some("Readable failure explanation")
        );
        assert_eq!(item.attention_reason_code.as_deref(), code);
        assert_eq!(
            item.tracked_status_messages,
            code.map(|_| "Readable failure explanation")
                .into_iter()
                .collect::<Vec<_>>()
        );
        job.state = HttpJobState::Completed;
        let item = job_to_queue_item(&job, None);
        assert!(item.attention_reason.is_none());
        assert!(item.attention_reason_code.is_none());
    }
}

#[test]
fn a_new_attempt_forgets_the_old_representation() {
    let mut job = seeded_job(Path::new("/srv/dl"), "https://media.invalid/a.mp4", 4);
    let file = &mut job.files[0];
    start_new_attempt(file);
    assert_eq!(file.attempt, 2);
    assert_eq!(file.bytes_done, 0);
    assert_eq!(file.total_bytes, None);
    assert_eq!(file.validator, None);
    assert_eq!(
        file.part_path(),
        Path::new("/srv/dl/Synthetic.Show.S01E01/Synthetic.Show.S01E01.mp4.2.part")
    );
}

#[test]
fn resume_needs_recorded_bytes_a_length_and_a_validator() {
    let mut job = seeded_job(Path::new("/srv/dl"), "https://media.invalid/a.mp4", 4);
    let file = &mut job.files[0];
    let total = BODY.len() as u64;
    assert_eq!(resume_range(file, 4), resume_request(4, total));
    // More bytes on disk than recorded: resume from the recorded offset.
    assert_eq!(resume_range(file, 6), resume_request(4, total));
    // Fewer bytes on disk than recorded: the partial cannot be trusted.
    assert_eq!(resume_range(file, 2), RangeRequest::Whole);
    file.validator = None;
    assert_eq!(resume_range(file, 4), RangeRequest::Whole);
    file.validator = Some(RecordedValidator::default());
    assert_eq!(resume_range(file, 4), RangeRequest::Whole);
}

#[tokio::test]
async fn queue_document_round_trips_and_an_unreadable_one_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let store = HttpStateStore::for_client(dir.path(), "client/../odd id");
    assert!(store.path().starts_with(dir.path().join(".scryer-http")));
    assert_eq!(store.load().await.unwrap().jobs.len(), 0);

    let job = seeded_job(dir.path(), "https://media.invalid/a.mp4", 4);
    store
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();
    assert_eq!(store.load().await.unwrap().jobs, vec![job]);
    // Initializing over an existing document changes nothing.
    let before = std::fs::read(store.path()).unwrap();
    store.initialize().await.unwrap();
    assert_eq!(std::fs::read(store.path()).unwrap(), before);

    std::fs::write(store.path(), b"{not json").unwrap();
    assert!(store.load().await.is_err());
    assert_eq!(std::fs::read(store.path()).unwrap(), b"{not json");
}

// ---------------------------------------------------------------------------
// selection

#[test]
fn output_lands_in_a_folder_named_after_the_basename() {
    let mut source = source(vec![
        muxed("https://media.invalid/v/1080.mp4", 1080),
        muxed("https://media.invalid/v/720.mp4", 720),
        subtitle("https://media.invalid/s/en.vtt", "en-US"),
        subtitle("https://media.invalid/s/de.vtt", "de"),
    ]);
    source.selected.max_height = Some(720);
    source.selected.subtitle_mode = HttpSubtitleMode::SidecarPreferred;
    source.selected.subtitle_languages = vec!["en".into()];
    let planned = plan_download(&source, Path::new("/srv/dl")).unwrap();
    let (output_dir, files) = (planned.output_dir, planned.files);
    assert_eq!(
        planned.media_fallbacks.len(),
        0,
        "1080p is over the ceiling"
    );
    assert_eq!(output_dir, Path::new("/srv/dl/Synthetic.Show.S01E01"));
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].resource.url, "https://media.invalid/v/720.mp4");
    assert_eq!(
        files[0].final_path,
        Path::new("/srv/dl/Synthetic.Show.S01E01/Synthetic.Show.S01E01.mp4")
    );
    assert_eq!(files[1].role, HttpFileRole::Subtitle);
    assert_eq!(
        files[1].final_path,
        Path::new("/srv/dl/Synthetic.Show.S01E01/Synthetic.Show.S01E01.en-us.vtt")
    );
}

#[test]
fn separate_tracks_and_page_only_releases_are_refused() {
    let mut video = resource("https://media.invalid/v.mp4", HttpResourceRole::Video);
    video.separate_tracks = true;
    let audio = resource("https://media.invalid/a.m4a", HttpResourceRole::Audio);
    let error = plan_download(&source(vec![video, audio]), Path::new("/srv/dl")).unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::SEPARATE_TRACKS),
        "{error}"
    );

    let mut page = resource("https://synthetic.invalid/watch/1", HttpResourceRole::Muxed);
    page.transport = HttpTransport::ProviderPage;
    let error = plan_download(&source(vec![page]), Path::new("/srv/dl")).unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::UNSUPPORTED_TRANSPORT),
        "{error}"
    );
    let error = plan_download(&source(Vec::new()), Path::new("/srv/dl")).unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::UNSUPPORTED_TRANSPORT),
        "{error}"
    );
}

#[test]
fn output_basenames_must_be_one_plain_component() {
    assert!(validate_output_basename("Synthetic.Show.S01E01").is_ok());
    for bad in [
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        ".hidden",
        "with space",
        "a\\b",
    ] {
        assert!(validate_output_basename(bad).is_err(), "{bad:?} accepted");
    }
}

// ---------------------------------------------------------------------------
// response validation

#[test]
fn a_whole_file_answer_is_fresh_and_a_range_answered_whole_restarts() {
    let response = headers(&[("content-length", "34"), ("etag", ETAG)]);
    assert_eq!(
        judge_response(StatusCode::OK, &response, &RangeRequest::Whole),
        ResponseVerdict::Fresh {
            total: Some(34),
            validator: Some(RecordedValidator {
                etag: Some(ETAG.into()),
                last_modified: None
            }),
        }
    );
    assert!(matches!(
        judge_response(StatusCode::OK, &response, &resume_request(4, 34)),
        ResponseVerdict::Restart(_)
    ));
}

#[test]
fn a_partial_answer_continues_only_when_its_range_and_identity_match() {
    let good = headers(&[
        ("content-range", "bytes 4-33/34"),
        ("content-length", "30"),
        ("etag", ETAG),
    ]);
    assert_eq!(
        judge_response(StatusCode::PARTIAL_CONTENT, &good, &resume_request(4, 34)),
        ResponseVerdict::Continue
    );
    let restarts = [
        headers(&[("content-range", "bytes 0-33/34"), ("etag", ETAG)]),
        headers(&[("content-range", "bytes 4-33/40"), ("etag", ETAG)]),
        headers(&[("content-range", "garbage"), ("etag", ETAG)]),
        headers(&[("etag", ETAG)]),
        headers(&[
            ("content-range", "bytes 4-33/34"),
            ("content-length", "12"),
            ("etag", ETAG),
        ]),
        headers(&[
            ("content-range", "bytes 4-33/34"),
            ("etag", "\"synthetic-v2\""),
        ]),
    ];
    for response in restarts {
        assert!(
            matches!(
                judge_response(
                    StatusCode::PARTIAL_CONTENT,
                    &response,
                    &resume_request(4, 34)
                ),
                ResponseVerdict::Restart(_)
            ),
            "{response:?} was trusted"
        );
    }
    assert!(matches!(
        judge_response(
            StatusCode::RANGE_NOT_SATISFIABLE,
            &HeaderMap::new(),
            &resume_request(4, 34)
        ),
        ResponseVerdict::Restart(_)
    ));
}

#[test]
fn throttling_and_errors_are_classified() {
    assert_eq!(
        judge_response(
            StatusCode::TOO_MANY_REQUESTS,
            &headers(&[("retry-after", "7")]),
            &RangeRequest::Whole
        ),
        ResponseVerdict::Failed(TransferError::RetryAfter(
            Duration::from_secs(7),
            "the server answered 429 Too Many Requests".into()
        ))
    );
    assert!(matches!(
        judge_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &headers(&[("retry-after", "86400")]),
            &RangeRequest::Whole
        ),
        ResponseVerdict::Failed(TransferError::Permanent(_))
    ));
    assert!(matches!(
        judge_response(
            StatusCode::BAD_GATEWAY,
            &HeaderMap::new(),
            &RangeRequest::Whole
        ),
        ResponseVerdict::Failed(TransferError::Transient(_))
    ));
    let now = chrono::DateTime::parse_from_rfc2822("Mon, 19 Oct 2026 10:00:00 GMT")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(
        parse_retry_after("Mon, 19 Oct 2026 10:00:30 GMT", now),
        Some(Duration::from_secs(30))
    );
}

#[test]
fn content_ranges_parse_strictly() {
    let range = parse_content_range("bytes 4-33/34").unwrap();
    assert_eq!(
        (range.start, range.end, range.complete_length),
        (4, 33, Some(34))
    );
    assert_eq!(
        parse_content_range("bytes 4-33/*").unwrap().complete_length,
        None
    );
    assert!(parse_content_range("bytes 9-3/34").is_none());
    assert!(parse_content_range("bytes 4-34/34").is_none());
    assert!(parse_content_range("items 4-33/34").is_none());
}

#[test]
fn final_size_must_match_an_exact_size_but_not_an_estimate() {
    let mut exact = muxed("https://media.invalid/a.mp4", 720);
    exact.size_bytes = Some(34);
    assert!(check_final_size(34, 34, &exact).is_ok());
    assert!(matches!(
        check_final_size(30, 30, &exact),
        Err(TransferError::Permanent(_))
    ));
    assert!(matches!(
        check_final_size(30, 34, &exact),
        Err(TransferError::Permanent(_))
    ));
    let mut estimated = muxed("https://media.invalid/a.mp4", 720);
    estimated.size_estimate = Some(1000);
    assert!(check_final_size(34, 34, &estimated).is_ok());
}

#[test]
fn resource_headers_stay_on_their_own_origin() {
    let mut media = muxed("https://media.invalid/a.mp4", 720);
    media.headers = vec![
        PluginHttpHeader {
            name: "X-Synthetic-Token".into(),
            value: "secret".into(),
        },
        PluginHttpHeader {
            name: "Range".into(),
            value: "bytes=0-1".into(),
        },
    ];
    let origin = reqwest::Url::parse("https://media.invalid/a.mp4").unwrap();
    let same = reqwest::Url::parse("https://media.invalid/elsewhere/b.mp4").unwrap();
    let other_host = reqwest::Url::parse("https://cdn.invalid/a.mp4").unwrap();
    let other_scheme = reqwest::Url::parse("http://media.invalid/a.mp4").unwrap();
    let other_port = reqwest::Url::parse("https://media.invalid:8443/a.mp4").unwrap();

    let scoped = scoped_headers(&media, &origin, &same);
    assert_eq!(scoped.get("x-synthetic-token").unwrap(), "secret");
    assert!(
        scoped.get("range").is_none(),
        "controlled headers are never taken from a resource"
    );
    for elsewhere in [other_host, other_scheme, other_port] {
        assert!(
            scoped_headers(&media, &origin, &elsewhere).is_empty(),
            "{elsewhere}"
        );
    }
}

// ---------------------------------------------------------------------------
// end to end against a local fixture server

#[tokio::test]
async fn downloads_a_file_and_its_sidecar_and_keeps_headers_on_their_origin() {
    let origin = MockServer::start().await;
    let cdn = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .and(header("x-synthetic-token", "secret"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/edge/clip.mp4", cdn.uri()).as_str()),
        )
        .expect(1)
        .mount(&origin)
        .await;
    Mock::given(method("GET"))
        .and(path("/edge/clip.mp4"))
        .and(|request: &Request| !request.headers.contains_key("x-synthetic-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", ETAG)
                .set_body_bytes(BODY),
        )
        .expect(1)
        .mount(&cdn)
        .await;
    Mock::given(method("GET"))
        .and(path("/subs/en.vtt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"WEBVTT\n".as_slice()))
        .expect(1)
        .mount(&origin)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let mut media = muxed(&format!("{}/media/clip.mp4", origin.uri()), 720);
    media.headers = vec![PluginHttpHeader {
        name: "X-Synthetic-Token".into(),
        value: "secret".into(),
    }];
    media.size_bytes = Some(BODY.len() as i64);
    let mut source = source(vec![
        media,
        subtitle(&format!("{}/subs/en.vtt", origin.uri()), "en"),
    ]);
    source.selected.subtitle_mode = HttpSubtitleMode::SidecarPreferred;
    source.selected.subtitle_languages = vec!["en".into()];
    let request = request(source);

    let grab = client.submit_download(&request).await.expect("submitted");
    assert_eq!(grab.client_type, "http");
    assert_eq!(grab.download_id, request.download_id);

    let job = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(job.state, HttpJobState::Completed, "{:?}", job.error);

    let output = dir.path().join("Synthetic.Show.S01E01");
    let media_path = output.join("Synthetic.Show.S01E01.mp4");
    let subtitle_path = output.join("Synthetic.Show.S01E01.en.vtt");
    assert_eq!(std::fs::read(&media_path).unwrap(), BODY);
    assert_eq!(std::fs::read(&subtitle_path).unwrap(), b"WEBVTT\n");
    assert!(!output.join("Synthetic.Show.S01E01.mp4.part").exists());

    let completed = client.list_completed_downloads().await.unwrap();
    assert_eq!(completed.len(), 1);
    let completed = &completed[0];
    assert_eq!(completed.download_client_item_id, grab.job_id);
    assert_eq!(completed.dest_dir, media_path.to_string_lossy());
    assert_eq!(
        completed.subtitle_paths,
        vec![subtitle_path.to_string_lossy().into_owned()]
    );
    // The wire form, exactly as the grab records the job's identity.
    let download_id_wire = request.download_id.map(|id| id.to_wire());
    assert_eq!(completed.download_id, download_id_wire);
    assert!(
        completed.parameters.contains(&(
            scryer_application::DOWNLOAD_ID_PARAMETER.to_string(),
            download_id_wire
                .clone()
                .expect("the request carries a download id"),
        ))
    );
    assert!(client.list_queue().await.unwrap().is_empty());
    let history = client.list_history().await.unwrap();
    assert_eq!(
        history[0].state,
        scryer_domain::DownloadQueueState::Completed
    );
    assert_eq!(history[0].title_id.as_deref(), Some("title-synthetic"));

    // Removing it, even when asked to remove data, leaves every file.
    assert!(!client.supports_native_data_removal());
    client
        .delete_queue_item(&grab.job_id, true, true)
        .await
        .unwrap();
    assert!(client.list_history().await.unwrap().is_empty());
    assert!(media_path.exists() && subtitle_path.exists());
    let observed = client
        .observe_download(
            &scryer_application::ClientJobLocator {
                client_id: Some(config.id.clone()),
                client_type: "http".into(),
                item_id: grab.job_id.clone(),
            },
            0,
        )
        .await
        .unwrap();
    assert!(matches!(
        observed,
        scryer_application::DownloadClientObservation::Absent
    ));
}

#[tokio::test]
async fn an_interrupted_transfer_resumes_with_a_validated_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .and(header("range", "bytes=4-"))
        .and(header("if-range", ETAG))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("etag", ETAG)
                .insert_header(
                    "content-range",
                    format!("bytes 4-{}/{}", BODY.len() - 1, BODY.len()).as_str(),
                )
                .set_body_bytes(&BODY[4..]),
        )
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let job = seeded_job(dir.path(), &format!("{}/media/clip.mp4", server.uri()), 4);
    seed_interrupted(&config, dir.path(), &job).await;

    // Building the client after the "restart" loads the queue and resumes.
    let client = client(&config);
    let done = wait_for_job(&client, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.files[0].attempt, 1);
    assert_eq!(std::fs::read(&job.files[0].final_path).unwrap(), BODY);
}

#[tokio::test]
async fn a_server_without_ranges_gets_a_fresh_attempt_and_the_old_partial_stays() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", ETAG)
                .set_body_bytes(BODY),
        )
        .expect(2)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let job = seeded_job(dir.path(), &format!("{}/media/clip.mp4", server.uri()), 4);
    seed_interrupted(&config, dir.path(), &job).await;
    let first_part = job.files[0].part_path();

    let client = client(&config);
    let done = wait_for_job(&client, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.files[0].attempt, 2);
    assert_eq!(std::fs::read(&job.files[0].final_path).unwrap(), BODY);
    // The first attempt's partial is untouched, not truncated or removed.
    assert_eq!(std::fs::read(&first_part).unwrap(), &BODY[..4]);
}

#[tokio::test]
async fn a_paused_download_holds_until_resumed() {
    let server = MockServer::start().await;
    // The first request never answers in time, so the job is mid-transfer
    // when it is paused; the next one answers at once.
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(BODY)
                .set_delay(Duration::from_secs(3600)),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let source = source(vec![muxed(
        &format!("{}/media/clip.mp4", server.uri()),
        720,
    )]);
    let grab = client.submit_download(&request(source)).await.unwrap();

    wait_for_job(&client, &grab.job_id, |job| {
        job.state == HttpJobState::Downloading
    })
    .await;
    // Wait until the slow request reached the server, so the pause lands
    // while it is in flight.
    let probe = tokio::time::timeout(WAIT_BOUND, async {
        loop {
            if !server
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    });
    probe.await.expect("the request reached the server");

    client.pause_queue_item(&grab.job_id).await.unwrap();
    let paused = client.engine().job(&grab.job_id).await.unwrap().unwrap();
    assert_eq!(paused.state, HttpJobState::Paused);
    let listed = client.list_queue().await.unwrap();
    assert_eq!(listed[0].state, scryer_domain::DownloadQueueState::Paused);

    client.resume_queue_item(&grab.job_id).await.unwrap();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
}

#[tokio::test]
async fn a_provider_page_only_release_is_refused_and_nothing_is_queued() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let mut page = resource(
        "https://synthetic.invalid/watch/item-1",
        HttpResourceRole::Muxed,
    );
    page.transport = HttpTransport::ProviderPage;
    let error = client
        .submit_download(&request(source(vec![page])))
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::UNSUPPORTED_TRANSPORT),
        "{error}"
    );
    assert!(client.list_queue().await.unwrap().is_empty());
    assert!(!dir.path().join("Synthetic.Show.S01E01").exists());
}

#[tokio::test]
async fn a_size_mismatch_against_an_exact_size_fails_the_job() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let mut media = muxed(&format!("{}/media/clip.mp4", server.uri()), 720);
    media.size_bytes = Some(BODY.len() as i64 + 1);
    let grab = client
        .submit_download(&request(source(vec![media])))
        .await
        .unwrap();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    assert!(done.error.unwrap_or_default().contains("exactly"));
    let output = dir.path().join("Synthetic.Show.S01E01");
    assert!(!output.join("Synthetic.Show.S01E01.mp4").exists());
    // The bytes that did arrive stay where they were written.
    assert!(output.join("Synthetic.Show.S01E01.mp4.part").exists());
    let history = client.list_history().await.unwrap();
    assert!(history[0].attention_required);
}

// ---------------------------------------------------------------------------
// refusals, retries and media fallbacks

/// A client whose retries do not wait on the clock.
fn eager_client(config: &DownloadClientConfig) -> HttpDownloadClient {
    let client = client(config);
    client.engine().set_backoff_unit(Duration::ZERO);
    client
}

fn sized_muxed(url: &str, height: u32, container: &str) -> PluginHttpResource {
    PluginHttpResource {
        container: Some(container.into()),
        ..muxed(url, height)
    }
}

#[test]
fn a_forbidden_answer_is_its_own_class_and_other_client_errors_are_rejections() {
    assert_eq!(
        judge_response(
            StatusCode::FORBIDDEN,
            &HeaderMap::new(),
            &RangeRequest::Whole
        ),
        ResponseVerdict::Failed(TransferError::Forbidden(
            "the server answered 403 Forbidden".into()
        ))
    );
    assert_eq!(
        judge_response(
            StatusCode::FORBIDDEN,
            &HeaderMap::new(),
            &resume_request(4, 34)
        ),
        ResponseVerdict::Failed(TransferError::Forbidden(
            "the server answered 403 Forbidden".into()
        ))
    );
    for status in [
        StatusCode::NOT_FOUND,
        StatusCode::GONE,
        StatusCode::UNAUTHORIZED,
    ] {
        assert!(
            matches!(
                judge_response(status, &HeaderMap::new(), &RangeRequest::Whole),
                ResponseVerdict::Failed(TransferError::Rejected(_))
            ),
            "{status}"
        );
    }
}

#[test]
fn fallbacks_follow_the_media_ranking_within_the_ceiling_and_are_bounded() {
    let mut source = source(vec![
        muxed("https://media.invalid/v/2160.mp4", 2160),
        muxed("https://media.invalid/v/720.mp4", 720),
        muxed("https://media.invalid/v/1080.mp4", 1080),
        muxed("https://media.invalid/v/480.mp4", 480),
        muxed("https://media.invalid/v/360.mp4", 360),
        muxed("https://media.invalid/v/540.mp4", 540),
        muxed("https://media.invalid/v/1080.mp4", 1080),
    ]);
    source.selected.max_height = Some(1080);
    let planned = plan_download(&source, Path::new("/srv/dl")).unwrap();
    assert_eq!(
        planned.files[0].resource.url,
        "https://media.invalid/v/1080.mp4"
    );
    let fallbacks: Vec<&str> = planned
        .media_fallbacks
        .iter()
        .map(|resource| resource.url.as_str())
        .collect();
    assert_eq!(fallbacks.len(), MAX_MEDIA_FALLBACKS);
    assert_eq!(
        fallbacks,
        vec![
            "https://media.invalid/v/720.mp4",
            "https://media.invalid/v/540.mp4",
            "https://media.invalid/v/480.mp4",
        ]
    );
}

#[tokio::test]
async fn a_spurious_forbidden_answer_is_retried_on_the_same_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/de/clip.mp4"))
        .respond_with(ResponseTemplate::new(403))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/de/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let source = source(vec![muxed(&format!("{}/de/clip.mp4", server.uri()), 720)]);
    let grab = client.submit_download(&request(source)).await.unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.forbidden_requests, 2);
    assert_eq!(done.candidate_index, 0);
    assert_eq!(done.files[0].attempt, 1);
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), BODY);
}

#[tokio::test]
async fn no_request_follows_the_third_forbidden_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/de/clip.mp4"))
        .respond_with(ResponseTemplate::new(403))
        .up_to_n_times(3)
        .expect(3)
        .mount(&server)
        .await;
    // Would succeed, but the retry bound is spent before it is reached.
    Mock::given(method("GET"))
        .and(path("/de/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(0)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let source = source(vec![muxed(&format!("{}/de/clip.mp4", server.uri()), 720)]);
    let grab = client.submit_download(&request(source)).await.unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    assert_eq!(
        done.error.as_deref(),
        Some("geo-blocked: every variant answered 403 Forbidden (1 variant, 3 requests)")
    );
    assert_eq!(done.forbidden_requests, 3);
}

#[tokio::test]
async fn a_geo_blocked_variant_falls_back_to_the_next_and_its_partial_stays() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/de/clip.mp4"))
        .respond_with(ResponseTemplate::new(403))
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/weltweit/clip.mkv"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", ETAG)
                .set_body_bytes(BODY),
        )
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    // An earlier run of variant A left a partial; the restart finds the
    // fallback list in the queue document.
    let mut job = seeded_job(dir.path(), &format!("{}/de/clip.mp4", server.uri()), 4);
    job.media_fallbacks = vec![sized_muxed(
        &format!("{}/weltweit/clip.mkv", server.uri()),
        720,
        "mkv",
    )];
    seed_interrupted(&config, dir.path(), &job).await;
    let first_part = job.files[0].part_path();

    let client = eager_client(&config);
    let done = wait_for_job(&client, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.candidate_index, 1);
    assert_eq!(done.forbidden_requests, 3);
    assert_eq!(
        done.refused_variants,
        vec![RefusedVariant {
            forbidden: true,
            error: "the server answered 403 Forbidden to all 3 requests".into(),
        }]
    );
    let media = &done.files[0];
    assert_eq!(media.attempt, 2);
    assert_eq!(
        media.final_path,
        job.output_dir.join("Synthetic.Show.S01E01.mkv")
    );
    assert_eq!(std::fs::read(&media.final_path).unwrap(), BODY);
    // Variant A's partial is untouched.
    assert_eq!(std::fs::read(&first_part).unwrap(), &BODY[..4]);
    let completed = client.list_completed_downloads().await.unwrap();
    assert_eq!(
        completed[0].dest_dir,
        media.final_path.to_string_lossy().into_owned()
    );
}

#[tokio::test]
async fn every_variant_forbidden_fails_with_a_geo_blocked_message_and_deletes_nothing() {
    let server = MockServer::start().await;
    for name in ["clip_1080.mp4", "clip_720.mp4"] {
        Mock::given(method("GET"))
            .and(path(format!("/de/{name}")))
            .respond_with(ResponseTemplate::new(403))
            .expect(3)
            .mount(&server)
            .await;
    }

    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("Synthetic.Show.S01E01");
    std::fs::create_dir_all(&output).unwrap();
    let unrelated = output.join("unrelated.txt");
    std::fs::write(&unrelated, b"keep").unwrap();

    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let mut source = source(vec![
        muxed(&format!("{}/de/clip_1080.mp4", server.uri()), 1080),
        muxed(&format!("{}/de/clip_720.mp4", server.uri()), 720),
    ]);
    source.release.availability = HttpAvailability::GeoRestricted;
    let grab = client.submit_download(&request(source)).await.unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    assert_eq!(
        done.error.as_deref(),
        Some(
            "geo-blocked: every variant answered 403 Forbidden (2 variants, 6 requests); the release is declared geo-restricted"
        )
    );
    assert_eq!(done.forbidden_requests, 6);
    assert_eq!(done.candidate_index, 1);
    assert!(done.geo_restricted);
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
    let history = client.list_history().await.unwrap();
    assert!(history[0].attention_required);
    assert_eq!(history[0].attention_reason, done.error);
}

#[tokio::test]
async fn a_missing_variant_falls_back_at_once_without_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gone/clip_1080.mp4"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/weltweit/clip_720.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let source = source(vec![
        muxed(&format!("{}/gone/clip_1080.mp4", server.uri()), 1080),
        muxed(&format!("{}/weltweit/clip_720.mp4", server.uri()), 720),
    ]);
    let grab = client.submit_download(&request(source)).await.unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.candidate_index, 1);
    assert_eq!(done.forbidden_requests, 0);
    assert_eq!(
        done.refused_variants,
        vec![RefusedVariant {
            forbidden: false,
            error: "the server answered 404 Not Found".into(),
        }]
    );
    assert_eq!(done.files[0].attempt, 2);
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), BODY);
}

#[tokio::test]
async fn fallback_fields_round_trip_and_an_older_document_loads_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let store = HttpStateStore::for_client(dir.path(), "synthetic-client");
    let mut job = seeded_job(dir.path(), "https://media.invalid/de/a.mp4", 4);
    job.media_fallbacks = vec![sized_muxed(
        "https://media.invalid/weltweit/a.mkv",
        720,
        "mkv",
    )];
    job.candidate_index = 1;
    job.forbidden_requests = 3;
    job.refused_variants = vec![RefusedVariant {
        forbidden: true,
        error: "the server answered 403 Forbidden to all 3 requests".into(),
    }];
    job.geo_restricted = true;
    job.files[0].forbidden_answers = 2;
    store
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();
    assert_eq!(store.load().await.unwrap().jobs, vec![job.clone()]);

    // A document written before these fields existed.
    let mut old = serde_json::to_value(&job).unwrap();
    let object = old.as_object_mut().unwrap();
    for key in [
        "media_fallbacks",
        "candidate_index",
        "forbidden_requests",
        "refused_variants",
        "geo_restricted",
    ] {
        object.remove(key);
    }
    object["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("forbidden_answers");
    let document = serde_json::json!({ "version": 1, "jobs": [old] });
    std::fs::write(store.path(), serde_json::to_vec(&document).unwrap()).unwrap();
    let loaded = store.load().await.unwrap().jobs.remove(0);
    assert!(loaded.media_fallbacks.is_empty());
    assert_eq!(loaded.candidate_index, 0);
    assert_eq!(loaded.forbidden_requests, 0);
    assert!(loaded.refused_variants.is_empty());
    assert!(!loaded.geo_restricted);
    assert_eq!(loaded.files[0].forbidden_answers, 0);
    assert_eq!(loaded.files[0].bytes_done, 4);
}

#[test]
fn a_mixed_set_of_refusals_names_the_last_one() {
    let mut job = seeded_job(Path::new("/srv/dl"), "https://media.invalid/a.mp4", 0);
    job.refused_variants = vec![
        RefusedVariant {
            forbidden: true,
            error: "the server answered 403 Forbidden to all 3 requests".into(),
        },
        RefusedVariant {
            forbidden: false,
            error: "the server answered 410 Gone".into(),
        },
    ];
    job.forbidden_requests = 3;
    assert_eq!(
        job.exhausted_message(),
        "no variant could be downloaded (2 variants); the last one failed with: the server answered 410 Gone"
    );
}

// ---------------------------------------------------------------------------
// proxied egress, error redaction and finished-media checks

fn transport_proxy(base_url: &str) -> scryer_domain::ProxyConfig {
    let created_at = Utc::now();
    scryer_domain::ProxyConfig {
        id: format!("proxy-{}", uuid::Uuid::new_v4()),
        name: "Synthetic Proxy".to_string(),
        provider_type: scryer_domain::ProxyProviderType::Http,
        protocol: None,
        base_url: base_url.to_string(),
        request_timeout_seconds: 30,
        is_enabled: true,
        username_encrypted: None,
        password_encrypted: None,
        remote_dns: false,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        created_at,
        updated_at: created_at,
        host_key_fingerprint: None,
        host_key_pinned_at: None,
        private_key_encrypted: None,
        private_key_passphrase_encrypted: None,
        peer_public_key: None,
        preshared_key_encrypted: None,
        tunnel_public_key: None,
        tunnel_addresses: Vec::new(),
        tunnel_dns_servers: Vec::new(),
        tunnel_mtu: None,
        tunnel_keepalive_seconds: None,
    }
}

#[tokio::test]
async fn a_proxied_egress_hands_every_redirect_to_the_transfer() {
    // The mock stands in for an HTTP forward proxy: every request it carries
    // is answered with a redirect.
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://elsewhere.invalid/next"),
        )
        .mount(&proxy)
        .await;
    let egress = egress_for(Some(&transport_proxy(&proxy.uri()))).expect("proxied egress");
    let Egress::Proxied(client) = egress else {
        panic!("a transport proxy must give a proxied egress");
    };

    let response = client
        .get("http://origin.invalid/media/clip.mp4")
        .send()
        .await
        .expect("the proxy answered");

    // The redirect comes back to the caller instead of being followed.
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "http://elsewhere.invalid/next"
    );
    assert_eq!(proxy.received_requests().await.unwrap().len(), 1);
}

#[test]
fn no_proxy_or_a_challenge_solver_keeps_the_direct_egress() {
    assert!(matches!(egress_for(None).unwrap(), Egress::Direct));
    let mut solver = transport_proxy("http://solver.invalid:8191");
    solver.provider_type = scryer_domain::ProxyProviderType::Trawl;
    assert!(matches!(egress_for(Some(&solver)).unwrap(), Egress::Direct));
}

#[tokio::test]
async fn a_failed_request_is_described_without_its_signed_url() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // Accept the connection and close it before answering.
    let closer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
    });
    let error = scryer_outbound_http::generic_reqwest_client()
        .get(format!("http://{address}/media/clip.mp4?token=secret"))
        .send()
        .await
        .expect_err("the connection was closed without an answer");
    closer.await.unwrap();
    assert!(
        error.to_string().contains("secret"),
        "the raw error names the URL: {error}"
    );

    let message = super::transfer::describe_request_error("request failed", error);
    assert!(message.starts_with("request failed: "), "{message}");
    assert!(!message.contains("secret"), "{message}");
    assert!(!message.contains("token="), "{message}");
}

/// A probe that answers with fixed facts and records what it was asked.
struct FixedProbe {
    outcome: super::transfer::MediaProbeOutcome,
    probed: std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl FixedProbe {
    fn new(outcome: super::transfer::MediaProbeOutcome) -> Arc<Self> {
        Arc::new(Self {
            outcome,
            probed: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn probed(&self) -> Vec<std::path::PathBuf> {
        self.probed.lock().unwrap().clone()
    }
}

#[async_trait]
impl super::transfer::MediaProbe for FixedProbe {
    async fn probe(&self, path: &Path) -> super::transfer::MediaProbeOutcome {
        self.probed.lock().unwrap().push(path.to_path_buf());
        self.outcome.clone()
    }
}

fn probed(
    duration_seconds: Option<u64>,
    has_video: Option<bool>,
) -> super::transfer::MediaProbeOutcome {
    super::transfer::MediaProbeOutcome::Probed {
        duration_seconds,
        has_video,
        container: None,
    }
}

#[test]
fn finished_media_is_judged_against_its_expected_running_time_and_video() {
    use super::transfer::{MediaProbeOutcome, judge_media};
    use scryer_plugin_sdk::PluginHttpVerify;
    let verify = PluginHttpVerify::default();

    assert_eq!(
        judge_media(&probed(Some(3500), Some(true)), Some(3600), &verify),
        Ok(())
    );
    let short = judge_media(&probed(Some(600), Some(true)), Some(3600), &verify)
        .expect_err("a sixth of the running time is a truncated file");
    assert!(matches!(short, TransferError::Permanent(_)), "{short:?}");
    let message = short.message();
    assert!(
        message.contains("600s") && message.contains("3600s"),
        "{message}"
    );

    let silent = judge_media(&probed(Some(3600), Some(false)), Some(3600), &verify)
        .expect_err("no video track");
    assert!(silent.message().contains("no video"), "{silent:?}");

    // Facts the probe could not establish are not held against the file.
    assert_eq!(
        judge_media(&probed(None, Some(true)), Some(3600), &verify),
        Ok(())
    );
    assert_eq!(judge_media(&probed(Some(10), None), None, &verify), Ok(()));
    assert_eq!(
        judge_media(
            &MediaProbeOutcome::Unverifiable("unsupported format: part".into()),
            Some(3600),
            &verify
        ),
        Ok(())
    );

    // The release's own policy decides.
    let lenient = PluginHttpVerify {
        min_duration_ratio: 0.1,
        require_video: false,
    };
    assert_eq!(
        judge_media(&probed(Some(600), Some(false)), Some(3600), &lenient),
        Ok(())
    );
}

#[tokio::test]
async fn a_truncated_media_file_fails_the_job_and_its_sidecar_is_not_probed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/subs/en.vtt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"WEBVTT\n".as_slice()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let probe = FixedProbe::new(probed(Some(600), Some(true)));
    client.engine().set_media_probe(probe.clone());

    let mut source = source(vec![
        muxed(&format!("{}/media/clip.mp4", server.uri()), 720),
        subtitle(&format!("{}/subs/en.vtt", server.uri()), "en"),
    ]);
    source.selected.subtitle_mode = HttpSubtitleMode::SidecarPreferred;
    source.selected.subtitle_languages = vec!["en".into()];
    source.policy.expected_duration_seconds = Some(3600);
    let grab = client.submit_download(&request(source)).await.unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    let error = done.error.unwrap_or_default();
    assert!(error.contains("600s") && error.contains("3600s"), "{error}");

    let output = dir.path().join("Synthetic.Show.S01E01");
    let part = output.join("Synthetic.Show.S01E01.mp4.part");
    // Only the media file was probed, under its partial name; it never
    // reached its final name and its bytes stay on disk.
    assert_eq!(probe.probed(), vec![part.clone()]);
    assert!(part.exists());
    assert!(!output.join("Synthetic.Show.S01E01.mp4").exists());
}

#[tokio::test]
async fn media_that_passes_its_checks_completes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let probe = FixedProbe::new(probed(Some(3590), Some(true)));
    client.engine().set_media_probe(probe.clone());

    let mut source = source(vec![muxed(
        &format!("{}/media/clip.mp4", server.uri()),
        720,
    )]);
    source.policy.expected_duration_seconds = Some(3600);
    let grab = client.submit_download(&request(source)).await.unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(probe.probed().len(), 1);
    assert!(
        dir.path()
            .join("Synthetic.Show.S01E01")
            .join("Synthetic.Show.S01E01.mp4")
            .exists()
    );
}

// ---------------------------------------------------------------------------
// durable progress

/// A one-shot HTTP server that answers every request with `body` sent as
/// one HTTP chunk per entry, so the client reads it as that many pieces.
/// Returns its base URL and the request heads it received.
async fn chunked_server(chunks: Vec<Vec<u8>>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let heads = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&heads);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let chunks = chunks.clone();
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());
                let mut answer = format!(
                    "HTTP/1.1 200 OK\r\nETag: {ETAG}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                )
                .into_bytes();
                for chunk in &chunks {
                    answer.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                    answer.extend_from_slice(chunk);
                    answer.extend_from_slice(b"\r\n");
                }
                answer.extend_from_slice(b"0\r\n\r\n");
                let _ = stream.write_all(&answer).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{address}"), heads)
}

#[tokio::test]
async fn byte_checkpoints_write_progress_and_structural_checkpoints_write_document() {
    let chunks: Vec<Vec<u8>> = BODY.chunks(2).map(<[u8]>::to_vec).collect();
    let chunk_count = chunks.len() as u64;
    let (base, _) = chunked_server(chunks).await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    // A checkpoint after every byte: each piece of the body is one.
    client
        .engine()
        .set_checkpoint_cadence(1, Duration::from_secs(3600));
    let grab = client
        .submit_download(&request(source(vec![muxed(
            &format!("{base}/media/clip.mp4"),
            720,
        )])))
        .await
        .unwrap();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), BODY);

    let engine = client.engine();
    assert!(
        engine.progress_writes() >= chunk_count,
        "{} progress writes for {chunk_count} pieces",
        engine.progress_writes()
    );
    // Added, started, partial name, publication identity, file finished, job
    // completed: bounded structural writes, independent of body chunk count.
    assert!(
        engine.document_writes() <= 6,
        "{} document writes",
        engine.document_writes()
    );
    // The finished job no longer carries fallbacks it will never use.
    assert!(done.media_fallbacks.is_empty());
    let store = HttpStateStore::for_client(dir.path(), &config.id);
    assert!(store.progress_path(&grab.job_id).exists());
    // Compact on disk.
    let raw = std::fs::read(store.path()).unwrap();
    assert!(!raw.contains(&b'\n'));
}

#[tokio::test]
async fn a_progress_file_newer_than_the_document_decides_the_resume_offset() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .and(header("range", "bytes=4-"))
        .and(header("if-range", ETAG))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("etag", ETAG)
                .insert_header(
                    "content-range",
                    format!("bytes 4-{}/{}", BODY.len() - 1, BODY.len()).as_str(),
                )
                .set_body_bytes(&BODY[4..]),
        )
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    // The document last recorded nothing of the file; the checkpoint after
    // it went to the progress file.
    let mut job = seeded_job(dir.path(), &format!("{}/media/clip.mp4", server.uri()), 0);
    job.files[0].total_bytes = None;
    job.files[0].validator = None;
    tokio::fs::create_dir_all(&job.output_dir).await.unwrap();
    tokio::fs::write(job.files[0].part_path(), &BODY[..4])
        .await
        .unwrap();
    let store = HttpStateStore::for_client(dir.path(), &config.id);
    store
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();
    let mut progressed = job.clone();
    progressed.progress_revision = 1;
    progressed.files[0].bytes_done = 4;
    progressed.files[0].total_bytes = Some(BODY.len() as u64);
    progressed.files[0].validator = Some(RecordedValidator {
        etag: Some(ETAG.into()),
        last_modified: None,
    });
    store
        .save_progress(&job.id, &super::state::HttpJobProgress::of(&progressed))
        .await
        .unwrap();

    let client = client(&config);
    let done = wait_for_job(&client, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.files[0].attempt, 1);
    assert_eq!(std::fs::read(&job.files[0].final_path).unwrap(), BODY);
}

#[tokio::test]
async fn a_progress_file_older_than_the_document_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let store = HttpStateStore::for_client(dir.path(), "synthetic-client");
    let mut job = seeded_job(dir.path(), "https://media.invalid/a.mp4", 4);
    job.progress_revision = 5;
    let mut stale = job.clone();
    stale.progress_revision = 5;
    stale.files[0].bytes_done = 30;
    let progress = super::state::HttpJobProgress::of(&stale);
    let mut loaded = job.clone();
    assert!(!progress.apply_if_newer(&mut loaded));
    assert_eq!(loaded, job);
    store.save_progress(&job.id, &progress).await.unwrap();
    assert_eq!(store.load_progress(&job.id).await, Some(progress));
    assert_eq!(store.load_progress("absent-job").await, None);
}

/// A one-shot HTTP server that sends `BODY` with its full length announced,
/// stops after `split` bytes and sends the rest only once `gate` opens.
async fn gated_server(split: usize, gate: Arc<tokio::sync::Notify>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                return;
            }
            head.push(byte[0]);
        }
        let mut answer = format!(
            "HTTP/1.1 200 OK\r\nETag: {ETAG}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            BODY.len()
        )
        .into_bytes();
        answer.extend_from_slice(&BODY[..split]);
        let _ = stream.write_all(&answer).await;
        let _ = stream.flush().await;
        gate.notified().await;
        let _ = stream.write_all(&BODY[split..]).await;
        let _ = stream.shutdown().await;
    });
    format!("http://{address}")
}

#[tokio::test]
async fn http_host_admission_skips_blocked_jobs_and_releases_cancelled_reservations() {
    tokio::time::timeout(WAIT_BOUND, async {
        let mut gates = Vec::new();
        let mut urls = Vec::new();
        for index in 0..4 {
            let gate = Arc::new(tokio::sync::Notify::new());
            let base = gated_server(17, gate.clone()).await;
            urls.push(if index == 3 {
                base.replace("127.0.0.1", "localhost")
            } else {
                base
            });
            gates.push(gate);
        }
        let dir = tempfile::tempdir().unwrap();
        let config = config(
            dir.path(),
            serde_json::json!({
                "max_concurrent_downloads": 3, "max_connections_per_host": 1
            }),
        );
        let client = client(&config);
        client.engine().set_enabled(false);
        let mut ids = Vec::new();
        for (index, url) in urls.iter().enumerate() {
            let mut input = request(source(vec![muxed(&format!("{url}/media.mp4"), 720)]));
            input.source_title = Some(format!("Synthetic.Show.S01E{:02}.WEB", index + 1));
            input.release_title = input.source_title.clone();
            input.http.as_mut().unwrap().policy.output_basename =
                format!("Synthetic.Show.S01E{:02}", index + 1);
            ids.push(client.submit_download(&input).await.unwrap().job_id);
        }
        client.engine().set_enabled(true);
        wait_for_live_bytes(&client, &ids[0], 17).await;
        wait_for_live_bytes(&client, &ids[3], 17).await;
        for id in &ids[1..3] {
            assert_eq!(
                client.engine().job(id).await.unwrap().unwrap().state,
                HttpJobState::Queued
            );
        }
        // Cancellation releases admission, so the next A can start while B
        // remains admitted. Neither consumes another actual-request permit.
        client.engine().pause(&ids[0]).await.unwrap();
        gates[0].notify_one();
        wait_for_live_bytes(&client, &ids[1], 17).await;
        assert_eq!(
            client.engine().job(&ids[2]).await.unwrap().unwrap().state,
            HttpJobState::Queued
        );
        client.engine().set_limits(engine::HttpEngineLimits {
            max_concurrent_downloads: 3,
            max_connections_per_host: 2,
            bandwidth_limit_kbps: None,
        });
        // A config update alone does not trigger scheduling; resuming a job
        // invokes the existing scheduler with the new limit.
        client.engine().pause(&ids[2]).await.unwrap();
        client.engine().resume(&ids[2]).await.unwrap();
        wait_for_live_bytes(&client, &ids[2], 17).await;
        for gate in &gates {
            gate.notify_one();
        }
        for id in &ids[1..] {
            wait_for_job(&client, id, |job| job.state.is_terminal()).await;
        }
        client.engine().begin_shutdown();
        client.engine().workers().wait().await;
    })
    .await
    .expect("host admission regression timed out");
}

#[tokio::test]
async fn buffered_bytes_show_live_but_are_recorded_only_after_a_sync() {
    let split = 17;
    let gate = Arc::new(tokio::sync::Notify::new());
    let base = gated_server(split, Arc::clone(&gate)).await;

    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    client
        .engine()
        .set_checkpoint_cadence(u64::MAX, Duration::from_secs(3600));
    let grab = client
        .submit_download(&request(source(vec![muxed(
            &format!("{base}/media/clip.mp4"),
            720,
        )])))
        .await
        .unwrap();

    // Wait until the first piece has reached the write buffer.
    let engine = Arc::clone(client.engine());
    let live = tokio::time::timeout(WAIT_BOUND, async {
        loop {
            let listed = engine.job_with_live(&grab.job_id).await.unwrap();
            if let Some((job, Some(live))) = listed
                && live.bytes_done == split as u64
            {
                return job;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the first piece shows as live progress");
    let job = live;
    // Not yet synced: nothing is recorded and nothing has left the buffer.
    assert_eq!(job.files[0].bytes_done, 0);
    let part = job.files[0].part_path();
    assert_eq!(
        std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0),
        0
    );
    // The attempt and its validator may be recorded; its bytes are not.
    let store = HttpStateStore::for_client(dir.path(), &config.id);
    if let Some(progress) = store.load_progress(&grab.job_id).await {
        let mut recorded = job.clone();
        recorded.progress_revision = 0;
        progress.apply_if_newer(&mut recorded);
        assert_eq!(recorded.files[0].bytes_done, 0);
    }

    gate.notify_one();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), BODY);
    assert_eq!(done.files[0].bytes_done, BODY.len() as u64);
}

// ---------------------------------------------------------------------------
// optional and required subtitles

async fn subtitle_job(
    mode: HttpSubtitleMode,
) -> (tempfile::TempDir, HttpDownloadClient, MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/subs/en.vtt"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let mut source = source(vec![
        muxed(&format!("{}/media/clip.mp4", server.uri()), 720),
        subtitle(&format!("{}/subs/en.vtt", server.uri()), "en"),
    ]);
    source.selected.subtitle_mode = mode;
    source.selected.subtitle_languages = vec!["en".into()];
    let grab = client.submit_download(&request(source)).await.unwrap();
    (dir, client, server, grab.job_id)
}

#[tokio::test]
async fn a_missing_preferred_subtitle_is_skipped_and_the_job_completes() {
    let (dir, client, _server, id) = subtitle_job(HttpSubtitleMode::SidecarPreferred).await;
    let done = wait_for_job(&client, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert!(done.files[1].skipped_error.is_some());
    assert!(!done.files[1].complete);
    assert!(
        dir.path()
            .join("Synthetic.Show.S01E01")
            .join("Synthetic.Show.S01E01.mp4")
            .exists()
    );

    let completed = client.list_completed_downloads().await.unwrap();
    assert_eq!(completed.len(), 1);
    assert!(completed[0].subtitle_paths.is_empty());
    let history = client.list_history().await.unwrap();
    assert!(!history[0].attention_required);
    assert_eq!(history[0].attention_reason, None);
}

#[tokio::test]
async fn a_missing_required_subtitle_fails_the_job_and_names_the_language() {
    let (dir, client, _server, id) = subtitle_job(HttpSubtitleMode::SidecarRequired).await;
    let done = wait_for_job(&client, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    let error = done.error.unwrap_or_default();
    assert!(
        error.contains("required en subtitle") && error.contains("media file is on disk"),
        "{error}"
    );
    // The media file finished before the subtitle was tried, and stays.
    assert!(
        dir.path()
            .join("Synthetic.Show.S01E01")
            .join("Synthetic.Show.S01E01.mp4")
            .exists()
    );
}

// ---------------------------------------------------------------------------
// deleted and disabled clients

/// Wait until the job's running file has `bytes` in the write buffer.
async fn wait_for_live_bytes(client: &HttpDownloadClient, id: &str, bytes: u64) {
    let engine = Arc::clone(client.engine());
    tokio::time::timeout(WAIT_BOUND, async {
        loop {
            if let Some((_, Some(live))) = engine.job_with_live(id).await.unwrap()
                && live.bytes_done == bytes
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the transfer reached the awaited byte count");
}

#[tokio::test]
async fn a_deleted_client_stops_its_engine_and_keeps_its_queue() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let base = gated_server(17, Arc::clone(&gate)).await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let first = client(&config);
    let grab = first
        .submit_download(&request(source(vec![muxed(
            &format!("{base}/media/clip.mp4"),
            720,
        )])))
        .await
        .unwrap();
    wait_for_live_bytes(&first, &grab.job_id, 17).await;

    forget_engine(&config.id);
    assert!(registered_engine(&config.id).is_none());
    let old = Arc::clone(first.engine());
    assert!(old.is_shut_down());
    gate.notify_one();
    tokio::time::timeout(WAIT_BOUND, old.workers().wait())
        .await
        .expect("the forgotten engine's workers stop");

    // The queue stays on disk, the job unfinished, for whatever reads it next.
    let store = HttpStateStore::for_client(dir.path(), &config.id);
    let document = store.load().await.unwrap();
    let job = document
        .jobs
        .iter()
        .find(|job| job.id == grab.job_id)
        .expect("the job is still recorded");
    assert!(!job.state.is_terminal());

    // Created again under the same id, a new engine waits for the old one's
    // workers and lists the job it left.
    let again = client(&config);
    assert!(!Arc::ptr_eq(again.engine(), &old));
    let listed = again.engine().job(&grab.job_id).await.unwrap();
    assert!(listed.is_some_and(|job| !job.state.is_terminal()));
}

#[tokio::test]
async fn a_disabled_client_finishes_the_current_file_and_starts_nothing_new() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let base = gated_server(17, Arc::clone(&gate)).await;
    let subtitles = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/subs/en.vtt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"WEBVTT\n".as_slice()))
        .mount(&subtitles)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let mut source = source(vec![
        muxed(&format!("{base}/media/clip.mp4"), 720),
        subtitle(&format!("{}/subs/en.vtt", subtitles.uri()), "en"),
    ]);
    source.selected.subtitle_mode = HttpSubtitleMode::SidecarPreferred;
    source.selected.subtitle_languages = vec!["en".into()];
    let grab = client.submit_download(&request(source)).await.unwrap();
    wait_for_live_bytes(&client, &grab.job_id, 17).await;

    let mut disabled = config.clone();
    disabled.is_enabled = false;
    apply_saved_config(&disabled);
    gate.notify_one();
    let parked = wait_for_job(&client, &grab.job_id, |job| {
        job.state == HttpJobState::Queued && job.files[0].complete
    })
    .await;
    assert!(!parked.files[1].complete);
    assert!(subtitles.received_requests().await.unwrap().is_empty());
    assert!(
        dir.path()
            .join("Synthetic.Show.S01E01")
            .join("Synthetic.Show.S01E01.mp4")
            .exists()
    );

    apply_saved_config(&config);
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(subtitles.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_client_changed_to_another_type_stops_its_engine() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    client.engine().ensure_started().await.unwrap();
    let mut changed = config.clone();
    changed.client_type = "sabnzbd".into();
    apply_saved_config(&changed);
    assert!(registered_engine(&config.id).is_none());
    assert!(client.engine().is_shut_down());
}

// ---------------------------------------------------------------------------
// content coding

async fn gzipped(bytes: &[u8]) -> Vec<u8> {
    use tokio::io::AsyncWriteExt;
    let mut encoder = async_compression::tokio::write::GzipEncoder::new(Vec::new());
    encoder.write_all(bytes).await.unwrap();
    encoder.shutdown().await.unwrap();
    encoder.into_inner()
}

#[tokio::test]
async fn an_encoded_whole_file_answer_is_stored_as_sent() {
    let encoded = gzipped(BODY).await;
    assert_ne!(encoded.as_slice(), BODY);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .and(header("accept-encoding", "identity"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "gzip")
                .insert_header("etag", ETAG)
                .set_body_bytes(encoded.clone()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let grab = client
        .submit_download(&request(source(vec![muxed(
            &format!("{}/media/clip.mp4", server.uri()),
            720,
        )])))
        .await
        .unwrap();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    // The bytes on disk are the bytes the server announced and sent.
    assert_eq!(done.files[0].total_bytes, Some(encoded.len() as u64));
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), encoded);
}

// ---------------------------------------------------------------------------
// a finished file whose completion was not recorded

#[tokio::test]
async fn a_file_already_in_place_completes_without_a_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    // Renamed into place, then interrupted before the move was recorded.
    let mut job = seeded_job(
        dir.path(),
        &format!("{}/media/clip.mp4", server.uri()),
        BODY.len() as u64,
    );
    job.files[0].publication = Some(super::state::PublicationCheckpoint {
        attempt: 1,
        partial_path: job.files[0].part_path(),
        final_path: job.files[0].final_path.clone(),
        size: BODY.len() as u64,
        sha256: aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, BODY)
            .as_ref()
            .to_vec(),
    });
    tokio::fs::create_dir_all(&job.output_dir).await.unwrap();
    tokio::fs::write(&job.files[0].final_path, BODY)
        .await
        .unwrap();
    HttpStateStore::for_client(dir.path(), &config.id)
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();

    let client = client(&config);
    let probe = FixedProbe::new(probed(Some(600), Some(true)));
    client.engine().set_media_probe(probe.clone());
    let done = wait_for_job(&client, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert!(done.files[0].complete);
    assert_eq!(done.files[0].attempt, 1);
    assert_eq!(probe.probed(), vec![job.files[0].final_path.clone()]);
    assert_eq!(std::fs::read(&job.files[0].final_path).unwrap(), BODY);
    assert!(server.received_requests().await.unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// expired resource URLs

/// Answers 403 and, in answering, moves the test clock past the expiry.
struct ExpiringRefusal(Arc<std::sync::atomic::AtomicBool>);

impl wiremock::Respond for ExpiringRefusal {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        ResponseTemplate::new(403)
    }
}

#[tokio::test]
async fn a_refusal_after_the_url_expired_fails_without_retry_or_fallback() {
    let expires_at = chrono::DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let expired = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(ExpiringRefusal(Arc::clone(&expired)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/media/fallback.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = eager_client(&config);
    let clock_flag = Arc::clone(&expired);
    client.engine().set_clock(Arc::new(move || {
        if clock_flag.load(std::sync::atomic::Ordering::SeqCst) {
            expires_at + chrono::Duration::seconds(1)
        } else {
            expires_at - chrono::Duration::hours(1)
        }
    }));
    let mut first = muxed(&format!("{}/media/clip.mp4", server.uri()), 1080);
    first.expires_at = Some(expires_at.to_rfc3339());
    let mut fallback = muxed(&format!("{}/media/fallback.mp4", server.uri()), 720);
    fallback.expires_at = Some(expires_at.to_rfc3339());
    let grab = client
        .submit_download(&request(source(vec![first, fallback])))
        .await
        .unwrap();

    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    assert_eq!(
        done.error.as_deref(),
        Some("the resource URL has expired; grab the release again")
    );
    assert_eq!(done.forbidden_requests, 0);
    assert_eq!(done.files[0].forbidden_answers, 0);
    assert!(done.refused_variants.is_empty());
}

// ---------------------------------------------------------------------------
// indexer checksums

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes).as_ref())
}

async fn checksum_job(
    checksum: String,
) -> (tempfile::TempDir, HttpDownloadClient, MockServer, HttpJob) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", ETAG)
                .set_body_bytes(BODY),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let mut media = muxed(&format!("{}/media/clip.mp4", server.uri()), 720);
    media.validators.checksum = Some(checksum);
    let grab = client
        .submit_download(&request(source(vec![media])))
        .await
        .unwrap();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    (dir, client, server, done)
}

#[test]
fn checksums_are_read_in_their_common_spellings() {
    use super::transfer::{ChecksumAlgorithm, ExpectedChecksum};
    let sha256 = sha256_hex(BODY);
    let parsed = |raw: &str| ExpectedChecksum::parse(raw).map(|expected| expected.algorithm);
    assert_eq!(
        parsed(&format!("sha256:{sha256}")),
        Some(ChecksumAlgorithm::Sha256)
    );
    assert_eq!(
        parsed(&format!("SHA-256:{}", sha256.to_uppercase())),
        Some(ChecksumAlgorithm::Sha256)
    );
    assert_eq!(parsed(&sha256), Some(ChecksumAlgorithm::Sha256));
    assert_eq!(
        parsed("md5:0123456789abcdef0123456789abcdef"),
        Some(ChecksumAlgorithm::Md5)
    );
    assert_eq!(
        parsed("sha1:0123456789abcdef0123456789abcdef01234567"),
        Some(ChecksumAlgorithm::Sha1)
    );
    // Wrong length for the algorithm, unknown algorithm, not hex, bare
    // digests other than SHA-256.
    assert_eq!(parsed("md5:0123"), None);
    assert_eq!(parsed(&format!("crc32:{sha256}")), None);
    assert_eq!(parsed(&format!("sha256:{}", "zz".repeat(32))), None);
    assert_eq!(parsed("0123456789abcdef0123456789abcdef"), None);
}

#[tokio::test]
async fn a_matching_sha256_checksum_completes() {
    let (_dir, _client, _server, done) = checksum_job(format!("sha256:{}", sha256_hex(BODY))).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), BODY);
}

#[tokio::test]
async fn a_matching_md5_checksum_completes() {
    use md5::Digest as _;
    let md5 = hex(&md5::Md5::digest(BODY));
    let (_dir, _client, _server, done) = checksum_job(format!("md5:{md5}")).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
}

#[tokio::test]
async fn a_checksum_mismatch_fails_and_leaves_the_partial() {
    let wrong = sha256_hex(b"synthetic-other-payload");
    let (_dir, _client, _server, done) = checksum_job(wrong.clone()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    let error = done.error.unwrap_or_default();
    assert_eq!(
        error,
        "the download does not match the indexer's SHA-256 checksum"
    );
    assert!(!error.contains(&wrong));
    let part = done.files[0].part_path();
    assert_eq!(std::fs::read(&part).unwrap(), BODY);
    assert!(!done.files[0].final_path.exists());
}

// ---------------------------------------------------------------------------
// servers without range support

#[tokio::test]
async fn a_server_known_not_to_support_ranges_gets_a_fresh_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", ETAG)
                .set_body_bytes(BODY),
        )
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let mut job = seeded_job(dir.path(), &format!("{}/media/clip.mp4", server.uri()), 4);
    job.files[0].resource.range_support = scryer_plugin_sdk::HttpRangeSupport::KnownNo;
    assert_eq!(resume_range(&job.files[0], 4), RangeRequest::Whole);
    tokio::fs::create_dir_all(&job.output_dir).await.unwrap();
    let first_part = job.files[0].part_path();
    tokio::fs::write(&first_part, &BODY[..4]).await.unwrap();
    HttpStateStore::for_client(dir.path(), &config.id)
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();

    let client = client(&config);
    let done = wait_for_job(&client, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.files[0].attempt, 2);
    assert_eq!(std::fs::read(&job.files[0].final_path).unwrap(), BODY);
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|request| !request.headers.contains_key("range")
                && !request.headers.contains_key("if-range"))
    );
    // The earlier partial is left where it was.
    assert_eq!(std::fs::read(&first_part).unwrap(), &BODY[..4]);
}

// ---------------------------------------------------------------------------
// egress client reuse

#[tokio::test]
async fn the_egress_client_is_rebuilt_only_when_its_proxy_changes() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let http = scryer_outbound_http::generic_reqwest_client;
    let mut proxy = transport_proxy("http://127.0.0.1:3128");

    let first = HttpDownloadClient::for_config(&config, Some(&proxy), http()).unwrap();
    let engine = Arc::clone(first.engine());
    assert_eq!(engine.egress_builds(), 1);
    for _ in 0..3 {
        let again = HttpDownloadClient::for_config(&config, Some(&proxy), http()).unwrap();
        assert!(Arc::ptr_eq(again.engine(), &engine));
    }
    assert_eq!(engine.egress_builds(), 1);

    // An edited proxy gets a new client; the same edit seen again does not.
    proxy.updated_at += chrono::Duration::seconds(1);
    HttpDownloadClient::for_config(&config, Some(&proxy), http()).unwrap();
    HttpDownloadClient::for_config(&config, Some(&proxy), http()).unwrap();
    assert_eq!(engine.egress_builds(), 2);

    // Dropping the proxy moves to the direct path once.
    HttpDownloadClient::for_config(&config, None, http()).unwrap();
    HttpDownloadClient::for_config(&config, None, http()).unwrap();
    assert_eq!(engine.egress_builds(), 3);
}

// ---------------------------------------------------------------------------
// publishing, partial names and pacing, driven through one file transfer

/// Records every checkpoint; refuses the one that marks the file complete
/// when `crash_on_complete` is set, as an interruption right after the
/// publish would.
#[derive(Default)]
struct RecordingCheckpoint {
    recorded: std::sync::Mutex<Vec<HttpJobFile>>,
    changed: tokio::sync::Notify,
    crash_on_complete: bool,
    crash_on_publication: bool,
}

#[async_trait::async_trait]
impl super::transfer::Checkpoint for RecordingCheckpoint {
    async fn record(&self, file: &HttpJobFile) -> scryer_application::AppResult<()> {
        if self.crash_on_complete && file.complete {
            return Err(scryer_application::AppError::Repository(
                "synthetic interruption".into(),
            ));
        }
        self.recorded.lock().unwrap().push(file.clone());
        self.changed.notify_one();
        if self.crash_on_publication && file.publication.is_some() && !file.complete {
            return Err(scryer_application::AppError::Repository(
                "synthetic pre-publication interruption".into(),
            ));
        }
        Ok(())
    }
}

impl RecordingCheckpoint {
    fn last(&self) -> HttpJobFile {
        self.recorded
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a checkpoint")
    }
}

async fn transfer_once(
    checkpoint: &RecordingCheckpoint,
    file: &mut HttpJobFile,
    limiter: Option<&super::transfer::BandwidthLimiter>,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(), TransferError> {
    let live_bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let context = super::transfer::TransferContext {
        egress: super::transfer::Egress::Direct,
        checkpoint,
        cancel,
        live_bytes: &live_bytes,
        limiter,
        host_limiter: None,
        backoff_unit: Duration::from_millis(1),
        cadence: super::transfer::CheckpointCadence {
            bytes: u64::MAX,
            interval: Duration::from_secs(3600),
        },
        clock: super::transfer::system_clock(),
        media_check: None,
    };
    super::transfer::transfer_file(&context, file).await
}

fn fresh_file(download_dir: &Path, url: &str) -> HttpJobFile {
    let mut file = seeded_job(download_dir, url, 0).files.remove(0);
    file.total_bytes = None;
    file.validator = None;
    file
}

async fn whole_file_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/media/clip.mp4"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", ETAG)
                .set_body_bytes(BODY),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn publishing_never_replaces_a_file_already_under_the_final_name() {
    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("Synthetic.Show.S01E01.mp4");
    let part = part_path_for(&final_path, 1);
    std::fs::write(&part, BODY).unwrap();
    std::fs::write(&final_path, b"someone else's file").unwrap();

    let refused = super::transfer::publish_part(&part, &final_path).await;
    assert!(
        matches!(
            refused,
            Err(TransferError::Refused(
                http_release_refusal::DESTINATION_EXISTS,
                _
            ))
        ),
        "{refused:?}"
    );
    assert_eq!(std::fs::read(&final_path).unwrap(), b"someone else's file");
    assert_eq!(std::fs::read(&part).unwrap(), BODY);

    // An unverified final-only file is preserved without fetching again.
    let server = whole_file_server().await;
    let checkpoint = RecordingCheckpoint::default();
    let mut file = fresh_file(dir.path(), &format!("{}/media/clip.mp4", server.uri()));
    std::fs::create_dir_all(file.final_path.parent().unwrap()).unwrap();
    std::fs::write(&file.final_path, b"someone else's file").unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let result = transfer_once(&checkpoint, &mut file, None, &cancel).await;
    assert!(
        matches!(
            result,
            Err(TransferError::Refused(
                http_release_refusal::RECOVERY_UNVERIFIED,
                _
            ))
        ),
        "{result:?}"
    );
    assert!(!file.complete);
    assert_eq!(
        std::fs::read(&file.final_path).unwrap(),
        b"someone else's file"
    );
    assert!(!file.part_path().exists());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn publishing_again_after_an_interrupted_publish_drops_only_the_partial_name() {
    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("Synthetic.Show.S01E01.mp4");
    let part = part_path_for(&final_path, 1);
    std::fs::write(&part, BODY).unwrap();
    // Linked into place, then interrupted before the partial name went.
    std::fs::hard_link(&part, &final_path).unwrap();

    super::transfer::publish_part(&part, &final_path)
        .await
        .expect("the same bytes under both names finish the publish");
    assert_eq!(std::fs::read(&final_path).unwrap(), BODY);
    assert!(!part.exists());
}

#[tokio::test]
async fn a_fresh_attempt_skips_every_partial_name_already_taken() {
    let server = whole_file_server().await;
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = RecordingCheckpoint::default();
    let mut file = fresh_file(dir.path(), &format!("{}/media/clip.mp4", server.uri()));
    std::fs::create_dir_all(file.final_path.parent().unwrap()).unwrap();
    let first = part_path_for(&file.final_path, 1);
    let second = part_path_for(&file.final_path, 2);
    std::fs::write(&first, b"first partial").unwrap();
    std::fs::write(&second, b"second partial").unwrap();

    let cancel = tokio_util::sync::CancellationToken::new();
    transfer_once(&checkpoint, &mut file, None, &cancel)
        .await
        .expect("the transfer completes under a free name");
    assert!(file.complete);
    assert_eq!(file.attempt, 3);
    assert_eq!(std::fs::read(&file.final_path).unwrap(), BODY);
    assert_eq!(std::fs::read(&first).unwrap(), b"first partial");
    assert_eq!(std::fs::read(&second).unwrap(), b"second partial");
}

#[tokio::test]
async fn an_interruption_between_publish_and_record_is_recovered_without_a_refetch() {
    // No stated length: the length is only known once the body has ended.
    let (base, heads) = chunked_server(BODY.chunks(5).map(<[u8]>::to_vec).collect()).await;
    let dir = tempfile::tempdir().unwrap();
    let crashing = RecordingCheckpoint {
        crash_on_complete: true,
        ..Default::default()
    };
    let mut file = fresh_file(dir.path(), &format!("{base}/media/clip.mp4"));
    let cancel = tokio_util::sync::CancellationToken::new();
    let interrupted = transfer_once(&crashing, &mut file, None, &cancel).await;
    assert!(interrupted.is_err(), "the completion record was refused");
    assert_eq!(std::fs::read(&file.final_path).unwrap(), BODY);

    // Restart from what was durably recorded before the interruption.
    let mut recovered = crashing.last();
    assert!(!recovered.complete);
    assert_eq!(recovered.total_bytes, Some(BODY.len() as u64));
    assert_eq!(recovered.bytes_done, BODY.len() as u64);
    let checkpoint = RecordingCheckpoint::default();
    transfer_once(&checkpoint, &mut recovered, None, &cancel)
        .await
        .expect("the published file is recognized");
    assert!(recovered.complete);
    assert!(checkpoint.last().complete);
    assert_eq!(std::fs::read(&recovered.final_path).unwrap(), BODY);
    assert_eq!(heads.lock().unwrap().len(), 1, "nothing was fetched again");
}

#[tokio::test(start_paused = true)]
async fn a_paced_wait_ends_at_once_on_cancel() {
    let limiter = Arc::new(super::transfer::BandwidthLimiter::new(1));
    let cancel = tokio_util::sync::CancellationToken::new();
    // The first chunk passes; the next must wait an hour at one byte a second.
    limiter.consume(3600, &cancel).await.unwrap();
    let started = tokio::time::Instant::now();
    let waiting = tokio::spawn({
        let limiter = Arc::clone(&limiter);
        let cancel = cancel.clone();
        async move { limiter.consume(3600, &cancel).await }
    });
    cancel.cancel();
    assert_eq!(
        waiting.await.unwrap(),
        Err(super::transfer::PacingCancelled)
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn a_rate_change_repaces_a_pending_wait() {
    let limiter = Arc::new(super::transfer::BandwidthLimiter::new(1));
    let cancel = tokio_util::sync::CancellationToken::new();
    limiter.consume(3600, &cancel).await.unwrap();
    let started = tokio::time::Instant::now();
    let waiting = tokio::spawn({
        let limiter = Arc::clone(&limiter);
        let cancel = cancel.clone();
        async move { limiter.consume(3600, &cancel).await }
    });
    // Raised to 3600 bytes a second, the pending chunk needs about a second.
    limiter.set_rate(3600);
    waiting.await.unwrap().unwrap();
    assert!(
        started.elapsed() <= Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_transfer_paused_during_pacing_is_cancelled_and_checkpointed() {
    let server = whole_file_server().await;
    let dir = tempfile::tempdir().unwrap();
    let checkpoint = RecordingCheckpoint::default();
    let mut file = fresh_file(dir.path(), &format!("{}/media/clip.mp4", server.uri()));
    let limiter = super::transfer::BandwidthLimiter::new(1);
    let cancel = tokio_util::sync::CancellationToken::new();
    // Book the limiter an hour ahead, so the body's first chunk has to wait.
    limiter.consume(3600, &cancel).await.unwrap();
    let result = tokio::time::timeout(WAIT_BOUND, async {
        let transfer = transfer_once(&checkpoint, &mut file, Some(&limiter), &cancel);
        tokio::pin!(transfer);
        tokio::select! {
            result = &mut transfer => return result,
            // The response was judged and recorded; pause the job.
            () = checkpoint.changed.notified() => cancel.cancel(),
        }
        transfer.await
    })
    .await
    .expect("the cancelled transfer does not sit out the pacing delay");
    assert_eq!(result, Err(TransferError::Cancelled));
    assert!(!file.complete);
    assert!(!file.final_path.exists());
}

#[test]
fn a_selection_without_media_or_required_subtitles_is_refused_by_code() {
    let thumbnail = resource("https://media.invalid/t.jpg", HttpResourceRole::Thumbnail);
    let error = plan_download(&source(vec![thumbnail]), Path::new("/srv/dl")).unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::NO_MEDIA_RESOURCE),
        "{error}"
    );

    let mut wanting = source(vec![muxed("https://media.invalid/clip.mp4", 720)]);
    wanting.selected.subtitle_mode = HttpSubtitleMode::SidecarRequired;
    wanting.selected.subtitle_languages = vec!["en".into()];
    let error = plan_download(&wanting, Path::new("/srv/dl")).unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::NO_SUBTITLE_RESOURCE),
        "{error}"
    );
}

#[test]
fn an_unnamed_media_file_takes_the_extension_of_the_container_it_holds() {
    use super::transfer::{MediaProbeOutcome, media_path_for_probed_container};
    let mut opaque = resource("https://media.invalid/download", HttpResourceRole::Muxed);
    opaque.container = None;
    let plan = plan_download(&source(vec![opaque]), Path::new("/srv/dl")).unwrap();
    let planned = &plan.files[0].final_path;
    assert_eq!(scryer_domain::canonical_video_extension(planned), None);

    let read = |container: Option<&str>| MediaProbeOutcome::Probed {
        duration_seconds: None,
        has_video: Some(true),
        container: container.map(str::to_string),
    };
    let named = media_path_for_probed_container(planned, &read(Some("matroska")))
        .expect("a recognized container names the file");
    assert_eq!(
        named,
        Path::new("/srv/dl/Synthetic.Show.S01E01/Synthetic.Show.S01E01.mkv")
    );
    assert_eq!(
        scryer_domain::canonical_video_extension(&named),
        Some("mkv")
    );

    // An unread container, or a name that already says video, stays as it is.
    assert_eq!(media_path_for_probed_container(planned, &read(None)), None);
    assert_eq!(
        media_path_for_probed_container(&named, &read(Some("mp4"))),
        None
    );
}

#[tokio::test]
async fn an_unnamed_media_download_is_published_under_its_probed_container() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/download"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path(), serde_json::json!({}));
    let client = client(&config);
    let probe = FixedProbe::new(super::transfer::MediaProbeOutcome::Probed {
        duration_seconds: None,
        has_video: Some(true),
        container: Some("matroska".into()),
    });
    client.engine().set_media_probe(probe.clone());
    let mut opaque = resource(
        &format!("{}/download", server.uri()),
        HttpResourceRole::Muxed,
    );
    opaque.container = None;
    let grab = client
        .submit_download(&request(source(vec![opaque])))
        .await
        .unwrap();
    let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    let published = &done.files[0].final_path;
    assert_eq!(
        scryer_domain::canonical_video_extension(published),
        Some("mkv")
    );
    assert_eq!(std::fs::read(published).unwrap(), BODY);
    // Nothing is left under the placeholder name.
    let entries: Vec<_> = std::fs::read_dir(&done.output_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
}

// Sonarr 465b5192, Parser/IsoLanguages.cs: ISO aliases share an identity,
// while Portuguese and PortugueseBrazil are distinct.
#[test]
fn http_subtitles_match_aliases_without_erasing_regions() {
    let mut input = source(vec![
        muxed("https://cdn.invalid/media.mp4", 720),
        subtitle("https://cdn.invalid/en.vtt", "eng"),
        subtitle("https://cdn.invalid/pt.vtt", "pt-PT"),
        subtitle("https://cdn.invalid/br.vtt", "pt-BR"),
    ]);
    input.selected.subtitle_mode = HttpSubtitleMode::SidecarRequired;
    input.selected.subtitle_languages = vec!["en".into(), "pob".into()];
    let planned = plan_download(&input, Path::new("/synthetic")).unwrap();
    assert_eq!(planned.files.len(), 3);
    assert_eq!(planned.files[1].resource.url, "https://cdn.invalid/en.vtt");
    assert_eq!(planned.files[2].resource.url, "https://cdn.invalid/br.vtt");
}

// Radarr 4d9bd475, DecisionEngine/Specifications/LanguageSpecification.cs:
// a known unwanted language cannot satisfy the profile.
#[test]
fn http_audio_mismatch_is_neither_primary_nor_fallback() {
    let labelled = |url: &str, height, language: &str| {
        let mut resource = muxed(url, height);
        resource.audio = Some(scryer_plugin_sdk::PluginHttpAudioTrack {
            language: Some(language.into()),
            ..Default::default()
        });
        resource
    };
    let mut input = source(vec![
        labelled("https://cdn.invalid/de.mp4", 2160, "de"),
        labelled("https://cdn.invalid/en.mp4", 1080, "en"),
        labelled("https://cdn.invalid/eng.mp4", 720, "eng"),
    ]);
    input.selected.audio_languages = vec!["eng".into()];
    let planned = plan_download(&input, Path::new("/synthetic")).unwrap();
    assert_eq!(planned.files[0].resource.url, "https://cdn.invalid/en.mp4");
    assert_eq!(planned.media_fallbacks.len(), 1);
    assert_eq!(
        planned.media_fallbacks[0].url,
        "https://cdn.invalid/eng.mp4"
    );
    input.selected.audio_languages.push("jpn".into());
    assert_eq!(
        plan_download(&input, Path::new("/synthetic"))
            .unwrap_err()
            .validation_reason(),
        Some(http_release_refusal::NO_AUDIO_RESOURCE)
    );
}

#[tokio::test]
async fn http_recovery_refuses_same_size_replacement_and_legacy_final_only() {
    tokio::time::timeout(WAIT_BOUND, async {
        let server = whole_file_server().await;
        let dir = tempfile::tempdir().unwrap();
        let crashing = RecordingCheckpoint {
            crash_on_complete: true,
            ..Default::default()
        };
        let mut file = fresh_file(dir.path(), &format!("{}/media/clip.mp4", server.uri()));
        let cancel = tokio_util::sync::CancellationToken::new();
        assert!(
            transfer_once(&crashing, &mut file, None, &cancel)
                .await
                .is_err()
        );
        let mut recorded = crashing.last();
        let replacement = vec![b'x'; BODY.len()];
        std::fs::write(&recorded.final_path, &replacement).unwrap();
        let checkpoint = RecordingCheckpoint::default();
        let error = transfer_once(&checkpoint, &mut recorded, None, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error.reason(),
            Some(http_release_refusal::RECOVERY_UNVERIFIED)
        );
        assert_eq!(std::fs::read(&recorded.final_path).unwrap(), replacement);
        recorded.publication = None;
        std::fs::write(&recorded.final_path, BODY).unwrap();
        let error = transfer_once(&checkpoint, &mut recorded, None, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error.reason(),
            Some(http_release_refusal::RECOVERY_UNVERIFIED)
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    })
    .await
    .expect("HTTP regression timed out");
}

#[test]
fn http_partial_path_survives_extension_change_and_old_documents() {
    let mut file = fresh_file(Path::new("/synthetic"), "https://cdn.invalid/media");
    let old = file.part_path();
    file.partial_path = Some(old.clone());
    file.final_path.set_extension("mkv");
    let restored: HttpJobFile =
        serde_json::from_value(serde_json::to_value(&file).unwrap()).unwrap();
    assert_eq!(restored.part_path(), old);
    let mut legacy = serde_json::to_value(&file).unwrap();
    legacy.as_object_mut().unwrap().remove("partial_path");
    legacy.as_object_mut().unwrap().remove("publication");
    let restored: HttpJobFile = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        restored.part_path(),
        part_path_for(&restored.final_path, restored.attempt)
    );
    file.publication = Some(super::state::PublicationCheckpoint {
        attempt: file.attempt,
        partial_path: file.part_path(),
        final_path: file.final_path.clone(),
        size: BODY.len() as u64,
        sha256: aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, BODY)
            .as_ref()
            .to_vec(),
    });
    start_new_attempt(&mut file);
    assert!(file.publication.is_none());
    assert_eq!(file.part_path(), part_path_for(&file.final_path, 2));
}

#[tokio::test]
async fn http_host_permits_follow_actual_hosts_limits_and_cancellation() {
    tokio::time::timeout(WAIT_BOUND, async {
        use super::transfer::HostLimiter;
        let limiter = HostLimiter::new(1);
        let cancel = tokio_util::sync::CancellationToken::new();
        let url = reqwest::Url::parse("https://CDN.invalid/one").unwrap();
        let alias = reqwest::Url::parse("https://cdn.invalid.:8443/two").unwrap();
        let other = reqwest::Url::parse("https://other.invalid/three").unwrap();
        let first = limiter.acquire(&url, &cancel).await.unwrap();
        let _other = limiter.acquire(&other, &cancel).await.unwrap();
        let waiting = limiter.acquire(&alias, &cancel);
        tokio::pin!(waiting);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                std::future::Future::poll(waiting.as_mut(), cx).is_pending()
            ))
            .await
        );
        limiter.set_limit(2);
        let second = tokio::time::timeout(WAIT_BOUND, &mut waiting)
            .await
            .unwrap()
            .unwrap();
        limiter.set_limit(1);
        drop(first);
        let waiting = limiter.acquire(&url, &cancel);
        tokio::pin!(waiting);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                std::future::Future::poll(waiting.as_mut(), cx).is_pending()
            ))
            .await
        );
        cancel.cancel();
        assert!(matches!(
            tokio::time::timeout(WAIT_BOUND, &mut waiting)
                .await
                .unwrap(),
            Err(TransferError::Cancelled)
        ));
        drop(second);
        let fresh_cancel = tokio_util::sync::CancellationToken::new();
        let _permit = tokio::time::timeout(WAIT_BOUND, limiter.acquire(&url, &fresh_cancel))
            .await
            .unwrap()
            .unwrap();
    })
    .await
    .expect("HTTP regression timed out");
}

// Radarr 4d9bd475, MediaFiles/DiskScanService.cs only discovers supported
// extensions. An unknown container must not be made eligible by hiding .bin.
#[tokio::test]
async fn http_unknown_container_preserves_payload_and_reports_reason() {
    tokio::time::timeout(WAIT_BOUND, async {
        let server = whole_file_server().await;
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path(), serde_json::json!({}));
        let client = client(&config);
        client.engine().set_media_probe(FixedProbe::new(
            super::transfer::MediaProbeOutcome::Probed {
                duration_seconds: None,
                has_video: Some(true),
                container: None,
            },
        ));
        let mut opaque = resource(
            &format!("{}/media/clip.mp4", server.uri()),
            HttpResourceRole::Muxed,
        );
        opaque.container = Some("bin".into());
        let grab = client
            .submit_download(&request(source(vec![opaque])))
            .await
            .unwrap();
        let done = wait_for_job(&client, &grab.job_id, |job| job.state.is_terminal()).await;
        assert_eq!(done.state, HttpJobState::Failed);
        assert_eq!(
            done.error_reason.as_deref(),
            Some(http_release_refusal::UNKNOWN_CONTAINER)
        );
        assert!(!done.files[0].final_path.exists());
        assert_eq!(std::fs::read(done.files[0].part_path()).unwrap(), BODY);
        let item = job_to_queue_item(&done, None);
        assert_eq!(
            item.attention_reason_code.as_deref(),
            Some(http_release_refusal::UNKNOWN_CONTAINER)
        );
        assert_eq!(item.attention_reason, done.error);
        assert_eq!(item.tracked_status_messages, vec![done.error.unwrap()]);
    })
    .await
    .expect("HTTP regression timed out");
}

#[tokio::test]
async fn http_legacy_renamed_partial_is_recovered_without_refetch() {
    tokio::time::timeout(WAIT_BOUND, async {
        let dir = tempfile::tempdir().unwrap();
        let mut file = seeded_job(
            dir.path(),
            "https://unreachable.invalid/download",
            BODY.len() as u64,
        )
        .files
        .remove(0);
        file.resource.container = None;
        file.final_path.set_extension("mkv");
        std::fs::create_dir_all(file.final_path.parent().unwrap()).unwrap();
        let original = part_path_for(&file.final_path.with_extension("bin"), file.attempt);
        std::fs::write(&original, BODY).unwrap();
        let checkpoint = RecordingCheckpoint::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        let ambiguous = file.part_path();
        std::fs::write(&ambiguous, b"unrelated partial").unwrap();
        let error = transfer_once(&checkpoint, &mut file, None, &cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error.reason(),
            Some(http_release_refusal::RECOVERY_UNVERIFIED)
        );
        assert_eq!(std::fs::read(&ambiguous).unwrap(), b"unrelated partial");
        assert_eq!(std::fs::read(&original).unwrap(), BODY);
        assert!(!file.final_path.exists());
        std::fs::remove_file(&ambiguous).unwrap();
        tokio::time::timeout(
            WAIT_BOUND,
            transfer_once(&checkpoint, &mut file, None, &cancel),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(file.complete);
        assert_eq!(file.partial_path, Some(original.clone()));
        assert_eq!(std::fs::read(&file.final_path).unwrap(), BODY);
        assert!(!original.exists());
    })
    .await
    .expect("HTTP regression timed out");
}

#[tokio::test]
async fn http_publication_failure_preserves_partial() {
    tokio::time::timeout(WAIT_BOUND, async {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("owned.part");
        std::fs::write(&partial, BODY).unwrap();
        let missing_parent = dir.path().join("absent/media.mp4");
        let error = super::transfer::publish_part(&partial, &missing_parent)
            .await
            .unwrap_err();
        assert_eq!(
            error.reason(),
            Some(http_release_refusal::PUBLICATION_FAILED)
        );
        assert_eq!(std::fs::read(&partial).unwrap(), BODY);
        assert!(!missing_parent.exists());
    })
    .await
    .expect("HTTP regression timed out");
}

#[tokio::test]
async fn http_redirect_waits_for_destination_host_and_releases_source_host() {
    tokio::time::timeout(WAIT_BOUND, async {
        let destination = whole_file_server().await;
        let origin = MockServer::start().await;
        let target = format!("{}/media/clip.mp4", destination.uri());
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", target.as_str()))
            .mount(&origin)
            .await;
        let origin_url = origin.uri().replace("127.0.0.1", "localhost");
        let dir = tempfile::tempdir().unwrap();
        let mut file = fresh_file(dir.path(), &format!("{origin_url}/redirect"));
        std::fs::create_dir_all(file.final_path.parent().unwrap()).unwrap();
        let checkpoint = RecordingCheckpoint::default();
        let cancel = tokio_util::sync::CancellationToken::new();
        let limiter = super::transfer::HostLimiter::new(1);
        let held = limiter
            .acquire(&reqwest::Url::parse(&target).unwrap(), &cancel)
            .await
            .unwrap();
        let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let context = super::transfer::TransferContext {
            egress: super::transfer::Egress::Direct,
            checkpoint: &checkpoint,
            cancel: &cancel,
            live_bytes: &bytes,
            limiter: None,
            host_limiter: Some(&limiter),
            backoff_unit: Duration::from_millis(1),
            cadence: super::transfer::CheckpointCadence {
                bytes: u64::MAX,
                interval: WAIT_BOUND,
            },
            clock: super::transfer::system_clock(),
            media_check: None,
        };
        let transfer = super::transfer::transfer_file(&context, &mut file);
        tokio::pin!(transfer);
        tokio::select! {
            result = &mut transfer => panic!("destination permit should block: {result:?}"),
            () = limiter.blocked.notified() => {}
        }
        assert_eq!(origin.received_requests().await.unwrap().len(), 1);
        assert!(destination.received_requests().await.unwrap().is_empty());
        let source_permit = limiter
            .acquire(&reqwest::Url::parse(&origin_url).unwrap(), &cancel)
            .await
            .unwrap();
        drop(source_permit);
        drop(held);
        transfer.await.unwrap();
        assert_eq!(destination.received_requests().await.unwrap().len(), 1);
    })
    .await
    .expect("redirect finishes when the destination permit is released");
}

#[tokio::test]
async fn http_auth_refusal_is_coded_before_queueing() {
    tokio::time::timeout(WAIT_BOUND, async {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path(), serde_json::json!({}));
        let client = client(&config);
        let mut media = muxed("https://cdn.invalid/secure.mp4", 720);
        media.auth_ref = Some("provider-login".into());
        let error = client
            .submit_download(&request(source(vec![media])))
            .await
            .unwrap_err();
        assert_eq!(
            error.validation_reason(),
            Some(http_release_refusal::UNSUPPORTED_AUTH)
        );
        assert!(
            client
                .engine()
                .jobs_where(|_| true)
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await
    .expect("HTTP regression timed out");
}

#[tokio::test]
async fn http_interrupted_extension_checkpoint_keeps_actual_partial_path() {
    tokio::time::timeout(WAIT_BOUND, async {
        let server = whole_file_server().await;
        let dir = tempfile::tempdir().unwrap();
        let mut file = fresh_file(dir.path(), &format!("{}/media/clip.mp4", server.uri()));
        file.resource.container = Some("bin".into());
        file.final_path.set_extension("bin");
        std::fs::create_dir_all(file.final_path.parent().unwrap()).unwrap();
        let old_part = file.part_path();
        let crashing = RecordingCheckpoint {
            crash_on_publication: true,
            ..Default::default()
        };
        let cancel = tokio_util::sync::CancellationToken::new();
        let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let probe = FixedProbe::new(super::transfer::MediaProbeOutcome::Probed {
            duration_seconds: None,
            has_video: Some(true),
            container: Some("matroska".into()),
        });
        let context = super::transfer::TransferContext {
            egress: super::transfer::Egress::Direct,
            checkpoint: &crashing,
            cancel: &cancel,
            live_bytes: &bytes,
            limiter: None,
            host_limiter: None,
            backoff_unit: Duration::from_millis(1),
            cadence: super::transfer::CheckpointCadence {
                bytes: u64::MAX,
                interval: WAIT_BOUND,
            },
            clock: super::transfer::system_clock(),
            media_check: Some(super::transfer::MediaCheck {
                probe: probe.as_ref(),
                expected_duration_seconds: None,
                verify: Default::default(),
            }),
        };
        assert!(
            super::transfer::transfer_file(&context, &mut file)
                .await
                .is_err()
        );
        let mut recovered = crashing.last();
        assert_eq!(recovered.part_path(), old_part);
        assert_eq!(recovered.final_path.extension().unwrap(), "mkv");
        assert!(!recovered.final_path.exists());
        assert_eq!(std::fs::read(&old_part).unwrap(), BODY);
        let checkpoint = RecordingCheckpoint::default();
        let context = super::transfer::TransferContext {
            checkpoint: &checkpoint,
            ..context
        };
        let replacement = vec![b'x'; BODY.len()];
        std::fs::write(&old_part, &replacement).unwrap();
        let error = super::transfer::transfer_file(&context, &mut recovered)
            .await
            .unwrap_err();
        assert_eq!(
            error.reason(),
            Some(http_release_refusal::RECOVERY_UNVERIFIED)
        );
        assert_eq!(std::fs::read(&old_part).unwrap(), replacement);
        assert!(!recovered.final_path.exists());
        std::fs::write(&old_part, BODY).unwrap();
        super::transfer::transfer_file(&context, &mut recovered)
            .await
            .unwrap();
        assert!(recovered.complete);
        assert_eq!(std::fs::read(&recovered.final_path).unwrap(), BODY);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    })
    .await
    .expect("checkpoint recovery finishes");
}

#[tokio::test]
async fn native_http_auth_ref_refusal_has_a_stable_reason_and_redacts_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(directory.path(), serde_json::json!({}));
    let client = client(&config);
    let mut media = muxed("https://synthetic.invalid/media.mp4", 1080);
    media.auth_ref = Some("private-provider-credential-reference".to_string());
    let error = client
        .submit_download(&request(source(vec![media])))
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(scryer_application::http_release_refusal::UNSUPPORTED_AUTH)
    );
    assert!(
        !error
            .to_string()
            .contains("private-provider-credential-reference")
    );
    assert!(client.list_queue().await.unwrap().is_empty());
}
