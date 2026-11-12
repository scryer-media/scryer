use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use scryer_application::{
    DownloadClient, DownloadClientAddRequest, DownloadSourceKind, ResolvedDownloadArtifact,
    torbox_refusal,
};
use scryer_domain::{DownloadQueueState, MediaFacet, Title};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::api::{TorboxApi, TorboxItemKind};
use super::budget::{SystemTorboxClock, TorboxHourlyBudget};
use super::client::{RuntimeParts, TorboxDownloadClient, TorboxRuntime, TorboxSettings};
use super::fetch::{
    job_folder_for, link_key, parse_link_key, plan_fetch, sanitize_component, unique_job_folder,
};
use super::records::{TorboxJobPhase, TorboxJobRecord, TorboxRecordStore};
use crate::downloads::clients::http::owned::{Egress, HttpJob, HttpJobState, STATE_DIR_NAME};

const TOKEN: &str = "tb-synthetic-token-0002";
const SIGNATURE: &str = "synthetic-link-signature";
const HASH: &str = "00000000000000000000000000000000000000a1";
const MAGNET: &str =
    "magnet:?xt=urn:btih:00000000000000000000000000000000000000a1&dn=Synthetic.Show.S01";
const NZB: &[u8] = b"<?xml version=\"1.0\"?><nzb xmlns=\"http://www.newzbin.com/DTD/2003/nzb\"><file subject=\"synthetic\"></file></nzb>";
/// A failure bound only; every wait below ends on a state change.
const WAIT_BOUND: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// fixtures

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "error": null, "detail": "Done.", "data": data,
    }))
}

fn refused(status: u16, code: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "success": false, "error": code, "detail": "synthetic refusal", "data": null,
    }))
}

fn body(file_id: i64) -> Vec<u8> {
    format!("synthetic-payload-for-file-{file_id:04}").into_bytes()
}

fn file(id: i64, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "short_name": name.rsplit('/').next().unwrap(),
        "size": body(id).len(),
        "zipped": false,
        "infected": false,
    })
}

fn item(id: i64, state: &str, finished: bool, files: Vec<Value>) -> Value {
    json!({
        "id": id,
        "hash": HASH,
        "name": "Synthetic.Show.S01.WEB",
        "size": 1000,
        "active": !finished,
        "download_state": state,
        "progress": if finished { 1.0 } else { 0.42 },
        "eta": if finished { 0 } else { 120 },
        "ratio": 0.5,
        "download_present": finished,
        "download_finished": finished,
        "private": false,
        "files": files,
    })
}

fn season_files() -> Vec<Value> {
    vec![
        file(1, "Synthetic.Show.S01/Synthetic.Show.S01E01.mkv"),
        file(2, "Synthetic.Show.S01/Synthetic.Show.S01E02.mkv"),
        file(3, "Synthetic.Show.S01/Synthetic.Show.S01E01.en.srt"),
        file(4, "Synthetic.Show.S01/synthetic.nfo"),
    ]
}

struct Fixture {
    server: MockServer,
    dir: tempfile::TempDir,
    client_id: String,
}

impl Fixture {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            dir: tempfile::tempdir().unwrap(),
            client_id: format!("synthetic-torbox-{}", uuid::Uuid::new_v4()),
        }
    }

    fn settings(&self, cached_only: bool) -> TorboxSettings {
        TorboxSettings::from_config_json(
            &json!({
                "api_key": TOKEN,
                "download_dir": self.dir.path().to_string_lossy(),
                "cached_only": cached_only,
            })
            .to_string(),
        )
        .unwrap()
    }

    fn runtime_with(
        &self,
        cached_only: bool,
        auto_poll: bool,
        budget: TorboxHourlyBudget,
    ) -> Arc<TorboxRuntime> {
        self.build_runtime(cached_only, auto_poll, budget, None)
    }

    /// A new runtime for the same client after `previous` stops, as a
    /// process restart or configuration change makes one.
    fn restarted(&self, previous: Arc<TorboxRuntime>) -> Arc<TorboxRuntime> {
        self.restarted_with(previous, false)
    }

    fn restarted_with(&self, previous: Arc<TorboxRuntime>, auto_poll: bool) -> Arc<TorboxRuntime> {
        let predecessor = previous.retire();
        drop(previous);
        self.build_runtime(
            false,
            auto_poll,
            TorboxHourlyBudget::default(),
            Some(predecessor),
        )
    }

    fn build_runtime(
        &self,
        cached_only: bool,
        auto_poll: bool,
        budget: TorboxHourlyBudget,
        predecessor: Option<tokio_util::task::TaskTracker>,
    ) -> Arc<TorboxRuntime> {
        TorboxRuntime::new(
            RuntimeParts {
                client_id: self.client_id.clone(),
                settings: self.settings(cached_only),
                api: TorboxApi::new(scryer_outbound_http::generic_reqwest_client(), TOKEN)
                    .with_base_url(self.server.uri()),
                egress: Egress::Direct,
                egress_key: "direct".to_string(),
                predecessor,
                poll_interval: Duration::from_secs(15),
                seed_poll_interval: Duration::from_secs(300),
                budget,
            },
            auto_poll,
        )
    }

    fn runtime(&self) -> Arc<TorboxRuntime> {
        self.runtime_with(false, false, TorboxHourlyBudget::default())
    }

    /// TorBox's links for files 1 to 9, each pointing at a CDN path on the
    /// mock server that serves that file's bytes.
    async fn mount_links(&self, kind: &str) {
        for file_id in 1..=9_i64 {
            Mock::given(method("GET"))
                .and(path(format!("/v1/api/{kind}/requestdl")))
                .and(query_param("file_id", file_id.to_string()))
                .respond_with(ok(json!(format!(
                    "{}/cdn/{file_id}?sig={SIGNATURE}",
                    self.server.uri()
                ))))
                .mount(&self.server)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/cdn/{file_id}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body(file_id)))
                .mount(&self.server)
                .await;
        }
    }

    async fn mount_torrent_create(&self, id: i64) {
        Mock::given(method("POST"))
            .and(path("/v1/api/torrents/createtorrent"))
            .respond_with(ok(
                json!({ "torrent_id": id, "hash": HASH, "auth_id": "a" }),
            ))
            .mount(&self.server)
            .await;
    }

    async fn mount_list(&self, kind: &str, items: Vec<Value>) {
        Mock::given(method("GET"))
            .and(path(format!("/v1/api/{kind}/mylist")))
            .respond_with(ok(Value::Array(items)))
            .mount(&self.server)
            .await;
    }

    async fn requests_to(&self, wanted: &str) -> Vec<Request> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.url.path() == wanted)
            .collect()
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

fn magnet_request(release: &str) -> DownloadClientAddRequest {
    let mut request = DownloadClientAddRequest::from_legacy(
        &title(),
        None,
        Some(DownloadSourceKind::MagnetUri),
        Some(release.to_string()),
        None,
        None,
    );
    request.download_id = Some(scryer_domain::download_identity::DownloadId::new());
    request.resolved_download_artifact = Some(ResolvedDownloadArtifact::Magnet {
        uri: MAGNET.to_string(),
        info_hash_hint: None,
    });
    request
}

fn nzb_request(release: &str, password: Option<&str>) -> DownloadClientAddRequest {
    let mut request = DownloadClientAddRequest::from_legacy(
        &title(),
        None,
        Some(DownloadSourceKind::NzbFile),
        Some(release.to_string()),
        password.map(str::to_string),
        None,
    );
    request.download_id = Some(scryer_domain::download_identity::DownloadId::new());
    request.resolved_download_artifact = Some(ResolvedDownloadArtifact::Nzb {
        password_candidates: Default::default(),
        bytes: NZB.to_vec(),
        file_name: Some("synthetic.nzb".to_string()),
        content_type: None,
    });
    request
}

/// The text value of a multipart text field, if the body has one.
fn form_text(request: &Request, name: &str) -> Option<String> {
    let body = String::from_utf8_lossy(&request.body);
    let marker = format!("name=\"{name}\"\r\n\r\n");
    let start = body.find(&marker)? + marker.len();
    let end = body[start..].find("\r\n")? + start;
    Some(body[start..end].to_string())
}

async fn record_of(runtime: &TorboxRuntime, id: &str) -> TorboxJobRecord {
    runtime
        .records()
        .await
        .into_iter()
        .find(|record| record.id == id)
        .expect("the job is recorded")
}

/// Wait until the job's engine job satisfies `done`, re-checking on every
/// engine queue change.
async fn wait_for_engine_job(
    runtime: &Arc<TorboxRuntime>,
    record_id: &str,
    done: impl Fn(&HttpJob) -> bool,
) -> HttpJob {
    let engine_id = record_of(runtime, record_id)
        .await
        .engine_job_id
        .expect("the fetch started");
    let engine = Arc::clone(&runtime.engine);
    let mut changes = engine.subscribe();
    tokio::time::timeout(WAIT_BOUND, async {
        loop {
            if let Some(job) = engine.job(&engine_id).await.expect("queue readable")
                && done(&job)
            {
                return job;
            }
            changes.changed().await.expect("engine alive");
        }
    })
    .await
    .expect("the engine job reached the awaited state")
}

/// Every byte recorded under the client's state directory.
fn recorded_state(dir: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut pending = vec![dir.join(STATE_DIR_NAME)];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                bytes.extend(std::fs::read(entry.path()).unwrap());
            }
        }
    }
    bytes
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Submit, let TorBox finish, fetch the files, and wait for the engine.
async fn run_torrent_to_completion(
    fixture: &Fixture,
    runtime: &Arc<TorboxRuntime>,
    request: &DownloadClientAddRequest,
) -> String {
    let client = TorboxDownloadClient::with_runtime(Arc::clone(runtime));
    let grab = client.submit_download(request).await.expect("submitted");
    runtime.poll_once().await.expect("polled");
    wait_for_engine_job(runtime, &grab.job_id, |job| job.state.is_terminal()).await;
    let _ = fixture;
    grab.job_id
}

// ---------------------------------------------------------------------------
// end to end

#[tokio::test]
async fn cached_magnet_is_fetched_as_a_season_folder() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list("torrents", vec![item(101, "cached", true, season_files())])
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    let grab = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect("submitted");
    assert_eq!(grab.client_type, "torbox");
    assert_eq!(grab.info_hash.as_deref(), Some(HASH));

    let create = fixture
        .requests_to("/v1/api/torrents/createtorrent")
        .await
        .remove(0);
    assert_eq!(form_text(&create, "allow_zip").as_deref(), Some("false"));
    assert_eq!(
        form_text(&create, "add_only_if_cached").as_deref(),
        Some("false")
    );
    assert_eq!(
        form_text(&create, "seed"),
        None,
        "no seed goals, no seed option"
    );
    assert_eq!(
        form_text(&create, "name").as_deref(),
        Some("Synthetic.Show.S01.WEB")
    );

    let queued = client.list_queue().await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].id, grab.job_id);
    assert_eq!(queued[0].download_client_item_id, grab.job_id);

    runtime.poll_once().await.unwrap();
    let job = wait_for_engine_job(&runtime, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(job.state, HttpJobState::Completed, "{:?}", job.error);

    let folder = fixture.dir.path().join("Synthetic.Show.S01.WEB");
    for (name, id) in [
        ("Synthetic.Show.S01E01.mkv", 1),
        ("Synthetic.Show.S01E02.mkv", 2),
        ("Synthetic.Show.S01E01.en.srt", 3),
    ] {
        assert_eq!(
            std::fs::read(folder.join(name)).unwrap(),
            body(id),
            "{name}"
        );
    }
    assert!(
        !folder.join("synthetic.nfo").exists(),
        "only media and subtitles"
    );
    assert!(
        fixture.requests_to("/cdn/4").await.is_empty(),
        "the NFO is never fetched"
    );

    let completed = client.list_completed_downloads().await.unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].download_client_item_id, grab.job_id);
    assert_eq!(completed[0].client_type, "torbox");
    assert_eq!(Path::new(&completed[0].dest_dir), folder);
    assert!(client.list_queue().await.unwrap().is_empty());

    let observed = client
        .observe_download(
            &scryer_application::ClientJobLocator {
                client_id: None,
                client_type: "torbox".into(),
                item_id: grab.job_id.clone(),
            },
            0,
        )
        .await
        .unwrap();
    match observed {
        scryer_application::DownloadClientObservation::Present(item) => {
            assert_eq!(item.id, grab.job_id);
            assert_eq!(item.state, DownloadQueueState::Completed);
        }
        other => panic!("expected the job, got {other:?}"),
    }
}

#[tokio::test]
async fn nzb_with_password_is_fetched_end_to_end() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(ok(json!({ "usenetdownload_id": 202, "hash": "nzbhash" })))
        .expect(1)
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list(
            "usenet",
            vec![item(
                202,
                "completed",
                true,
                vec![
                    file(1, "Synthetic.Movie.2001/Synthetic.Movie.2001.mkv"),
                    file(2, "Synthetic.Movie.2001/Synthetic.Movie.2001.vol00+01.par2"),
                ],
            )],
        )
        .await;
    fixture.mount_links("usenet").await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    let grab = client
        .submit_download(&nzb_request("Synthetic.Movie.2001", Some("synthetic-pass")))
        .await
        .expect("submitted");
    assert_eq!(grab.info_hash, None);
    let create = fixture
        .requests_to("/v1/api/usenet/createusenetdownload")
        .await
        .remove(0);
    assert_eq!(
        form_text(&create, "password").as_deref(),
        Some("synthetic-pass")
    );
    assert_eq!(form_text(&create, "post_processing"), None);
    assert_eq!(
        form_text(&create, "add_only_if_cached").as_deref(),
        Some("false")
    );
    assert_eq!(
        record_of(&runtime, &grab.job_id).await.kind,
        TorboxItemKind::Usenet
    );

    runtime.poll_once().await.unwrap();
    let job = wait_for_engine_job(&runtime, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(job.state, HttpJobState::Completed, "{:?}", job.error);
    let folder = fixture.dir.path().join("Synthetic.Movie.2001");
    assert_eq!(
        std::fs::read(folder.join("Synthetic.Movie.2001.mkv")).unwrap(),
        body(1)
    );
    assert!(fixture.requests_to("/cdn/2").await.is_empty());
    let queue = client.list_history().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].seeding, None, "usenet has no seeding");
}

#[tokio::test]
async fn cached_only_decline_is_refused_as_not_cached() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(ok(Value::Null))
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime_with(true, false, TorboxHourlyBudget::default());
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect_err("declined");
    assert_eq!(error.reason(), Some(torbox_refusal::NOT_CACHED));
    let create = fixture
        .requests_to("/v1/api/torrents/createtorrent")
        .await
        .remove(0);
    assert_eq!(
        form_text(&create, "add_only_if_cached").as_deref(),
        Some("true")
    );
    assert!(
        runtime.records().await.is_empty(),
        "a decline leaves no job"
    );
    assert!(client.list_queue().await.unwrap().is_empty());
}

#[tokio::test]
async fn hourly_budget_refuses_before_sending() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let budget =
        TorboxHourlyBudget::with_limit(Arc::new(SystemTorboxClock), 1, Duration::from_secs(3600));
    let runtime = fixture.runtime_with(false, false, budget);
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect("the first fits the budget");
    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S02.WEB"))
        .await
        .expect_err("the second does not");
    assert_eq!(error.reason(), Some(torbox_refusal::HOURLY_LIMIT));
    assert_eq!(
        fixture
            .requests_to("/v1/api/torrents/createtorrent")
            .await
            .len(),
        1,
        "the refused create was never sent"
    );
    assert_eq!(runtime.records().await.len(), 1);
}

#[tokio::test]
async fn a_grab_without_a_resolved_source_is_refused() {
    let fixture = Fixture::new().await;
    let client = TorboxDownloadClient::with_runtime(fixture.runtime());
    let mut request = magnet_request("Synthetic.Show.S01.WEB");
    request.resolved_download_artifact = None;
    request.source_hint = Some("https://indexer.synthetic.invalid/get/1".into());
    let error = client.submit_download(&request).await.expect_err("refused");
    assert_eq!(error.reason(), Some(torbox_refusal::SOURCE_MISSING));
    assert!(fixture.server.received_requests().await.unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// restarts

fn orphan(kind: TorboxItemKind, name: &str, info_hash: Option<&str>) -> TorboxJobRecord {
    let long_ago = Utc::now() - chrono::Duration::minutes(10);
    TorboxJobRecord {
        id: uuid::Uuid::new_v4().to_string(),
        download_id: Some(uuid::Uuid::new_v4().to_string()),
        title_id: "title-synthetic".into(),
        title_name: "Synthetic Show".into(),
        facet: "series".into(),
        release_name: name.into(),
        category: None,
        kind,
        torbox_id: None,
        info_hash: info_hash.map(str::to_string),
        submitted_name: name.into(),
        phase: TorboxJobPhase::Submitting,
        job_folder: None,
        engine_job_id: None,
        seed_goal_ratio: None,
        seed_goal_seconds: None,
        created_at: long_ago,
        updated_at: long_ago,
        remote_finished_at: None,
        error: None,
        error_reason: None,
    }
}

#[tokio::test]
async fn restart_while_submitting_finds_or_fails_each_create() {
    let fixture = Fixture::new().await;
    let found = orphan(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.WEB",
        Some(HASH),
    );
    let lost = orphan(TorboxItemKind::Usenet, "Synthetic.Movie.2001", None);
    TorboxRecordStore::for_client(fixture.dir.path(), &fixture.client_id)
        .save(&[found.clone(), lost.clone()])
        .await
        .unwrap();
    fixture
        .mount_list("torrents", vec![item(101, "downloading", false, vec![])])
        .await;
    fixture.mount_list("usenet", vec![]).await;

    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    assert_eq!(client.list_queue().await.unwrap().len(), 2);
    runtime.poll_once().await.unwrap();

    let found = record_of(&runtime, &found.id).await;
    assert_eq!(found.phase, TorboxJobPhase::Remote);
    assert_eq!(found.torbox_id, Some(101));
    let lost = record_of(&runtime, &lost.id).await;
    assert_eq!(lost.phase, TorboxJobPhase::Failed);
    assert_eq!(
        lost.error_reason.as_deref(),
        Some(torbox_refusal::ITEM_MISSING)
    );
    let history = client.list_history().await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].attention_reason_code.as_deref(),
        Some(torbox_refusal::ITEM_MISSING)
    );
}

#[tokio::test]
async fn restart_while_remote_continues_to_the_fetch() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture.mount_links("torrents").await;
    let first = fixture.runtime();
    let grab = TorboxDownloadClient::with_runtime(Arc::clone(&first))
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .unwrap();

    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    let second = fixture.restarted(first);
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&second));
    assert_eq!(client.list_queue().await.unwrap()[0].id, grab.job_id);
    second.poll_once().await.unwrap();
    let job = wait_for_engine_job(&second, &grab.job_id, |job| job.state.is_terminal()).await;
    assert_eq!(job.state, HttpJobState::Completed, "{:?}", job.error);
    assert_eq!(
        client.list_completed_downloads().await.unwrap()[0].download_client_item_id,
        grab.job_id
    );
}

#[tokio::test]
async fn restart_while_fetching_reports_the_same_job() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let first = fixture.runtime();
    let job_id =
        run_torrent_to_completion(&fixture, &first, &magnet_request("Synthetic.Show.S01.WEB"))
            .await;
    let second = fixture.restarted(first);
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&second));
    let completed = client.list_completed_downloads().await.unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].download_client_item_id, job_id);
    assert_eq!(client.list_history().await.unwrap()[0].id, job_id);
}

#[tokio::test]
async fn restart_between_folder_and_engine_job_adopts_the_engine_job() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let first = fixture.runtime();
    let job_id =
        run_torrent_to_completion(&fixture, &first, &magnet_request("Synthetic.Show.S01.WEB"))
            .await;
    let mut records = first.records().await;
    let engine_job_id = records[0].engine_job_id.clone().unwrap();
    let stopped = first.retire();
    drop(first);
    tokio::time::timeout(WAIT_BOUND, stopped.wait())
        .await
        .expect("the first runtime stops");
    // As if the process stopped after the engine job was added but before
    // its id was recorded.
    records[0].phase = TorboxJobPhase::Remote;
    records[0].engine_job_id = None;
    TorboxRecordStore::for_client(fixture.dir.path(), &fixture.client_id)
        .save(&records)
        .await
        .unwrap();
    let links_before = fixture
        .requests_to("/v1/api/torrents/requestdl")
        .await
        .len();

    let second = fixture.runtime();
    second.poll_once().await.unwrap();
    let record = record_of(&second, &job_id).await;
    assert_eq!(record.phase, TorboxJobPhase::Fetching);
    assert_eq!(
        record.engine_job_id.as_deref(),
        Some(engine_job_id.as_str())
    );
    let engine_jobs = second.engine.jobs_where(|_| true).await.unwrap();
    assert_eq!(engine_jobs.len(), 1, "no second engine job");
    assert_eq!(
        fixture
            .requests_to("/v1/api/torrents/requestdl")
            .await
            .len(),
        links_before,
        "nothing is fetched again"
    );
}

// ---------------------------------------------------------------------------
// the poller

#[tokio::test]
async fn idle_poller_parks_without_calls_or_writes() {
    let fixture = Fixture::new().await;
    let runtime = fixture.runtime_with(false, true, TorboxHourlyBudget::default());
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let mut parked = runtime.parked();
    assert!(client.list_queue().await.unwrap().is_empty());
    tokio::time::timeout(WAIT_BOUND, parked.wait_for(|parked| *parked))
        .await
        .expect("the poller parks")
        .unwrap();
    assert!(fixture.server.received_requests().await.unwrap().is_empty());
    assert_eq!(runtime.record_writes(), 0);
    assert!(
        !runtime.store().path().exists(),
        "no record document is written"
    );
}

#[tokio::test]
async fn one_list_call_per_family_per_tick_and_no_write_without_change() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(ok(json!({ "torrent_id": 101 })))
        .up_to_n_times(1)
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(ok(json!({ "torrent_id": 102 })))
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(ok(json!({ "usenetdownload_id": 202 })))
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list(
            "torrents",
            vec![
                item(101, "downloading", false, vec![]),
                item(102, "stalled (no seeds)", false, vec![]),
            ],
        )
        .await;
    fixture
        .mount_list("usenet", vec![item(202, "downloading", false, vec![])])
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .unwrap();
    let stalled = client
        .submit_download(&magnet_request("Synthetic.Show.S02.WEB"))
        .await
        .unwrap();
    client
        .submit_download(&nzb_request("Synthetic.Movie.2001", None))
        .await
        .unwrap();
    let writes = runtime.record_writes();

    runtime.poll_once().await.unwrap();
    assert_eq!(
        fixture.requests_to("/v1/api/torrents/mylist").await.len(),
        1
    );
    assert_eq!(fixture.requests_to("/v1/api/usenet/mylist").await.len(), 1);
    let list = &fixture.requests_to("/v1/api/torrents/mylist").await[0];
    assert!(
        list.url
            .query_pairs()
            .any(|(key, value)| key == "bypass_cache" && value == "true")
    );
    assert_eq!(runtime.record_writes(), writes, "nothing changed phase");

    let queue = client.list_queue().await.unwrap();
    assert_eq!(queue.len(), 3);
    let stalled = queue.iter().find(|item| item.id == stalled.job_id).unwrap();
    assert_eq!(stalled.state, DownloadQueueState::Warning);
    assert_eq!(
        stalled.attention_reason_code.as_deref(),
        Some(torbox_refusal::STALLED)
    );
    let downloading = queue
        .iter()
        .find(|item| item.state == DownloadQueueState::Downloading)
        .unwrap();
    assert_eq!(downloading.progress_percent, 42);
    assert_eq!(downloading.remaining_seconds, Some(120));
}

// ---------------------------------------------------------------------------
// refusals while fetching

#[tokio::test]
async fn infected_item_is_refused_without_fetching() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let mut files = season_files();
    files[3]["infected"] = json!(true);
    fixture
        .mount_list("torrents", vec![item(101, "completed", true, files)])
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let grab = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .unwrap();
    runtime.poll_once().await.unwrap();

    let record = record_of(&runtime, &grab.job_id).await;
    assert_eq!(record.phase, TorboxJobPhase::Failed);
    assert_eq!(
        record.error_reason.as_deref(),
        Some(torbox_refusal::FILE_INFECTED)
    );
    assert!(
        fixture
            .requests_to("/v1/api/torrents/requestdl")
            .await
            .is_empty()
    );
    let history = client.list_history().await.unwrap();
    assert_eq!(history[0].state, DownloadQueueState::Failed);
    assert_eq!(
        history[0].attention_reason_code.as_deref(),
        Some(torbox_refusal::FILE_INFECTED)
    );
}

#[test]
fn file_names_are_sanitized_deterministically() {
    assert_eq!(sanitize_component("CON.mkv"), "_CON.mkv");
    assert_eq!(sanitize_component("a:b*c?d.mkv"), "a_b_c_d.mkv");
    assert_eq!(sanitize_component("trailing. . "), "trailing");
    assert_eq!(sanitize_component(".."), "_");
    assert_eq!(sanitize_component("tab\there"), "tab_here");
    let long = format!("{}.mkv", "x".repeat(400));
    let short = sanitize_component(&long);
    assert_eq!(short.len(), 255);
    assert!(short.ends_with(".mkv"));
    assert_eq!(job_folder_for(".hidden.release"), "hidden.release");
    assert_eq!(job_folder_for("Show: Part/Two"), "Show_ Part_Two");

    let taken: HashSet<String> = ["synthetic.show", "synthetic.show (2)"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(
        unique_job_folder("Synthetic.Show", &taken),
        "Synthetic.Show (3)"
    );
    assert_eq!(unique_job_folder("Other", &taken), "Other");

    let key = link_key(TorboxItemKind::Usenet, 7, 9);
    assert_eq!(parse_link_key(&key), Some((TorboxItemKind::Usenet, 7, 9)));
    assert_eq!(parse_link_key("t:1:2:3"), None);

    let mixed: super::api::TorboxItem = serde_json::from_value(item(
        5,
        "completed",
        true,
        vec![
            file(1, "Top\\..\\Season 1\\Episode: 1.mkv"),
            file(2, "Top/Season 1/Episode: 1.en.srt"),
            file(3, "Top/sample/notes.txt"),
        ],
    ))
    .unwrap();
    let planned = plan_fetch(TorboxItemKind::Torrent, &mixed).unwrap();
    let paths: Vec<&str> = planned
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["_/Season 1/Episode_ 1.mkv", "Season 1/Episode_ 1.en.srt"]
    );
    assert_eq!(
        planned,
        plan_fetch(TorboxItemKind::Torrent, &mixed).unwrap()
    );

    let colliding: super::api::TorboxItem = serde_json::from_value(item(
        6,
        "completed",
        true,
        vec![file(1, "Show/Episode.mkv"), file(2, "Show/EPISODE.mkv")],
    ))
    .unwrap();
    assert_eq!(
        plan_fetch(TorboxItemKind::Torrent, &colliding)
            .unwrap_err()
            .reason,
        torbox_refusal::FILE_PATHS_UNUSABLE
    );
    let zipped: super::api::TorboxItem = serde_json::from_value(item(
        7,
        "completed",
        true,
        vec![json!({"id": 1, "name": "Show/Episode.mkv", "zipped": true})],
    ))
    .unwrap();
    assert_eq!(
        plan_fetch(TorboxItemKind::Torrent, &zipped)
            .unwrap_err()
            .reason,
        torbox_refusal::FILE_ZIPPED
    );
    let no_video: super::api::TorboxItem =
        serde_json::from_value(item(8, "completed", true, vec![file(1, "Show/readme.txt")]))
            .unwrap();
    assert_eq!(
        plan_fetch(TorboxItemKind::Torrent, &no_video)
            .unwrap_err()
            .reason,
        torbox_refusal::NO_VIDEO_FILE
    );
}

// ---------------------------------------------------------------------------
// removal, seeding and secrets

#[tokio::test]
async fn removal_deletes_exactly_the_recorded_item() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/controltorrent"))
        .and(body_json(
            json!({ "torrent_id": 101, "operation": "delete" }),
        ))
        .respond_with(ok(Value::Null))
        .expect(1)
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let job_id = run_torrent_to_completion(
        &fixture,
        &runtime,
        &magnet_request("Synthetic.Show.S01.WEB"),
    )
    .await;

    let unknown = client
        .delete_queue_item("not-a-torbox-job", false, true)
        .await
        .expect_err("not ours");
    assert_eq!(unknown.reason(), Some(torbox_refusal::JOB_NOT_FOUND));

    client.delete_queue_item(&job_id, true, true).await.unwrap();
    let controls = fixture.requests_to("/v1/api/torrents/controltorrent").await;
    assert_eq!(controls.len(), 1, "one control call, for our job only");
    let sent: Value = serde_json::from_slice(&controls[0].body).unwrap();
    assert_eq!(sent, json!({ "torrent_id": 101, "operation": "delete" }));
    assert!(runtime.records().await.is_empty());
    assert!(
        runtime
            .engine
            .jobs_where(|_| true)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(client.list_history().await.unwrap().is_empty());
    assert!(
        fixture
            .dir
            .path()
            .join("Synthetic.Show.S01.WEB/Synthetic.Show.S01E01.mkv")
            .exists(),
        "fetched files stay on disk"
    );
}

#[tokio::test]
async fn removal_of_an_item_torbox_no_longer_holds_succeeds() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/controltorrent"))
        .respond_with(refused(404, "ITEM_NOT_FOUND"))
        .expect(1)
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let grab = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .unwrap();
    client
        .delete_queue_item(&grab.job_id, false, false)
        .await
        .expect("a missing item counts as removed");
    assert!(runtime.records().await.is_empty());
}

#[tokio::test]
async fn token_and_links_never_reach_a_persisted_file() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.runtime();
    run_torrent_to_completion(
        &fixture,
        &runtime,
        &magnet_request("Synthetic.Show.S01.WEB"),
    )
    .await;

    let recorded = recorded_state(fixture.dir.path());
    assert!(!recorded.is_empty());
    assert!(contains(&recorded, "t:101:1"), "the link keys are recorded");
    for secret in [TOKEN, SIGNATURE, "/cdn/", MAGNET] {
        assert!(!contains(&recorded, secret), "{secret} was persisted");
    }
}

#[tokio::test]
async fn seeding_torrent_reports_ratio_until_torbox_stops_seeding() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "uploading", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let mut request = magnet_request("Synthetic.Show.S01.WEB");
    request.seed_goal_ratio = Some(1.0);
    let job_id = run_torrent_to_completion(&fixture, &runtime, &request).await;
    let create = fixture
        .requests_to("/v1/api/torrents/createtorrent")
        .await
        .remove(0);
    assert_eq!(form_text(&create, "seed").as_deref(), Some("2"));

    let history = client.list_history().await.unwrap();
    let seeding = history[0]
        .seeding
        .clone()
        .expect("a torrent reports seeding");
    assert_eq!(history[0].id, job_id);
    assert_eq!(seeding.can_remove, Some(false), "TorBox still seeds");
    assert_eq!(seeding.can_move_files, Some(true), "the local copy is done");
    assert_eq!(seeding.seed_ratio, Some(0.5));
    assert_eq!(seeding.is_private, Some(false));
    assert_eq!(seeding.seed_goal_ratio, Some(1.0));
    assert!(seeding.completed_at.is_some());

    fixture.server.reset().await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    runtime.poll_once().await.unwrap();
    let seeding = client.list_history().await.unwrap()[0]
        .seeding
        .clone()
        .unwrap();
    assert_eq!(seeding.can_remove, Some(true), "TorBox stopped seeding");

    // Nothing is left to learn: the next poll reads nothing.
    fixture.server.reset().await;
    runtime.poll_once().await.unwrap();
    assert!(fixture.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn torrent_without_seed_goal_is_removable_once_held_and_never_polled() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "uploading", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let first = fixture.runtime();
    let job_id =
        run_torrent_to_completion(&fixture, &first, &magnet_request("Synthetic.Show.S01.WEB"))
            .await;
    let seeding = TorboxDownloadClient::with_runtime(Arc::clone(&first))
        .list_history()
        .await
        .unwrap()[0]
        .seeding
        .clone()
        .expect("a torrent reports seeding");
    assert_eq!(
        seeding.can_remove,
        Some(true),
        "no goal: TorBox's own seeding does not hold it"
    );
    assert_eq!(seeding.is_private, Some(false));
    assert_eq!(seeding.seed_goal_ratio, None);

    // After a restart nothing has been polled, and nothing needs to be.
    fixture.server.reset().await;
    let second = fixture.restarted_with(first, true);
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&second));
    let mut parked = second.parked();
    let history = client.list_history().await.unwrap();
    assert_eq!(history[0].id, job_id);
    assert_eq!(history[0].seeding.as_ref().unwrap().can_remove, Some(true));
    tokio::time::timeout(WAIT_BOUND, parked.wait_for(|parked| *parked))
        .await
        .expect("the poller parks")
        .unwrap();
    second.poll_once().await.unwrap();
    assert!(
        fixture.server.received_requests().await.unwrap().is_empty(),
        "a no-goal torrent causes no TorBox call"
    );
}

#[tokio::test]
async fn seeding_torrent_off_the_list_page_is_confirmed_before_it_ends() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "uploading", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let mut request = magnet_request("Synthetic.Show.S01.WEB");
    request.seed_goal_ratio = Some(1.0);
    run_torrent_to_completion(&fixture, &runtime, &request).await;

    // The item is on a later page of the list; the single-item read finds it.
    fixture.server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .and(query_param("id", "101"))
        .respond_with(ok(item(101, "uploading", true, season_files())))
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list("torrents", vec![item(999, "uploading", true, vec![])])
        .await;

    for tick in 1..=2_usize {
        runtime.poll_once().await.unwrap();
        let reads = fixture.requests_to("/v1/api/torrents/mylist").await;
        let single = reads
            .iter()
            .filter(|read| read.url.query_pairs().any(|(key, _)| key == "id"))
            .count();
        assert_eq!(single, tick, "one confirming read per tick");
        assert_eq!(reads.len() - single, tick, "one list call per tick");
        let seeding = client.list_history().await.unwrap()[0]
            .seeding
            .clone()
            .unwrap();
        assert_eq!(seeding.can_remove, Some(false), "still seeding on TorBox");
        assert_eq!(seeding.seed_ratio, Some(0.5));
    }
}

#[tokio::test]
async fn settings_require_a_token() {
    let error = TorboxSettings::from_config_json(r#"{"download_dir": "/srv/synthetic"}"#)
        .expect_err("no token");
    assert_eq!(error.reason(), Some(torbox_refusal::TOKEN_REQUIRED));
    let settings = TorboxSettings::from_config_json(
        r#"{"api_key": "tb", "download_dir": "/srv/synthetic", "cachedOnly": "true"}"#,
    )
    .unwrap();
    assert!(settings.cached_only);
    assert!(!format!("{settings:?}").contains("\"tb\""));
}
