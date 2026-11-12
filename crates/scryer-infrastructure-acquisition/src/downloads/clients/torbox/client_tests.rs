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

/// An item with a hash of its own, so no create of [`MAGNET`] takes it for
/// an item the account already held; [`item_with_hash`] gives it that one.
fn item(id: i64, state: &str, finished: bool, files: Vec<Value>) -> Value {
    json!({
        "id": id,
        "hash": format!("{id:040x}"),
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

fn item_with_hash(id: i64, hash: &str, state: &str, finished: bool) -> Value {
    let mut item = item(id, state, finished, vec![]);
    item["hash"] = json!(hash);
    item
}

fn season_files() -> Vec<Value> {
    vec![
        file(1, "Synthetic.Show.S01/Synthetic.Show.S01E01.mkv"),
        file(2, "Synthetic.Show.S01/Synthetic.Show.S01E02.mkv"),
        file(3, "Synthetic.Show.S01/Synthetic.Show.S01E01.en.srt"),
        file(4, "Synthetic.Show.S01/synthetic.nfo"),
    ]
}

/// Which item ids a mounted create will answer with, and which it already
/// has: a list does not show an item before the create that makes it.
#[derive(Default)]
struct CreateGate {
    pending: HashSet<i64>,
    created: HashSet<i64>,
}

type SharedGate = Arc<std::sync::Mutex<CreateGate>>;

fn answered_id(data: &Value) -> Option<i64> {
    ["torrent_id", "usenetdownload_id", "id"]
        .iter()
        .find_map(|key| data.get(key)?.as_i64())
}

/// A create's answer that marks the item it names as existing.
struct CreateAnswer {
    data: Value,
    gate: SharedGate,
}

impl wiremock::Respond for CreateAnswer {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        if let Some(id) = answered_id(&self.data) {
            self.gate.lock().unwrap().created.insert(id);
        }
        ok(self.data.clone())
    }
}

/// The account's items, without those a create has yet to make.
struct Listing {
    items: Vec<Value>,
    gate: SharedGate,
}

impl wiremock::Respond for Listing {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let gate = self.gate.lock().unwrap();
        let shown: Vec<Value> = self
            .items
            .iter()
            .filter(|item| {
                let id = item["id"].as_i64().unwrap_or_default();
                !gate.pending.contains(&id) || gate.created.contains(&id)
            })
            .cloned()
            .collect();
        ok(Value::Array(shown))
    }
}

struct Fixture {
    server: MockServer,
    dir: tempfile::TempDir,
    client_id: String,
    gate: SharedGate,
}

impl Fixture {
    async fn new() -> Self {
        let fixture = Self {
            server: MockServer::start().await,
            dir: tempfile::tempdir().unwrap(),
            client_id: format!("synthetic-torbox-{}", uuid::Uuid::new_v4()),
            gate: SharedGate::default(),
        };
        // An empty account unless a test says otherwise: the read every
        // create makes first finds nothing.
        for kind in ["torrents", "usenet"] {
            Mock::given(method("GET"))
                .and(path(format!("/v1/api/{kind}/mylist")))
                .respond_with(ok(json!([])))
                .with_priority(u8::MAX)
                .mount(&fixture.server)
                .await;
        }
        fixture
    }

    fn settings(&self, cached_only: bool) -> TorboxSettings {
        settings_in(self.dir.path(), cached_only)
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
        self.build_runtime_in(
            self.dir.path(),
            cached_only,
            auto_poll,
            budget,
            predecessor,
            None,
        )
    }

    /// A runtime whose download directory is `dir`, carrying the records of
    /// a predecessor that used `carry_from`.
    fn build_runtime_in(
        &self,
        dir: &Path,
        cached_only: bool,
        auto_poll: bool,
        budget: TorboxHourlyBudget,
        predecessor: Option<tokio_util::task::TaskTracker>,
        carry_from: Option<std::path::PathBuf>,
    ) -> Arc<TorboxRuntime> {
        TorboxRuntime::new(
            RuntimeParts {
                client_id: self.client_id.clone(),
                settings: settings_in(dir, cached_only),
                api: TorboxApi::new(scryer_outbound_http::generic_reqwest_client(), TOKEN)
                    .with_base_url(self.server.uri())
                    .allow_plain_http_links(),
                egress: Egress::Direct,
                egress_key: "direct".to_string(),
                predecessor,
                carry_records_from: carry_from,
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

    /// The answer of a create that makes the item `data` names.
    fn create_answer(&self, data: Value) -> CreateAnswer {
        if let Some(id) = answered_id(&data) {
            self.gate.lock().unwrap().pending.insert(id);
        }
        CreateAnswer {
            data,
            gate: Arc::clone(&self.gate),
        }
    }

    async fn mount_torrent_create(&self, id: i64) {
        Mock::given(method("POST"))
            .and(path("/v1/api/torrents/createtorrent"))
            .respond_with(
                self.create_answer(json!({ "torrent_id": id, "hash": HASH, "auth_id": "a" })),
            )
            .mount(&self.server)
            .await;
    }

    async fn mount_list(&self, kind: &str, items: Vec<Value>) {
        Mock::given(method("GET"))
            .and(path(format!("/v1/api/{kind}/mylist")))
            .respond_with(Listing {
                items,
                gate: Arc::clone(&self.gate),
            })
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

fn settings_in(dir: &Path, cached_only: bool) -> TorboxSettings {
    TorboxSettings::from_config_json(
        &json!({
            "api_key": TOKEN,
            "download_dir": dir.to_string_lossy(),
            "cached_only": cached_only,
        })
        .to_string(),
    )
    .unwrap()
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
        .respond_with(fixture.create_answer(json!({ "usenetdownload_id": 202, "hash": "nzbhash" })))
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
        preexisting_ids: Vec::new(),
        private: None,
        cancelled: false,
        phase: TorboxJobPhase::Submitting,
        job_folder: None,
        engine_job_id: None,
        seed_goal_ratio: None,
        seed_goal_seconds: None,
        never_remove: false,
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
        .mount_list(
            "torrents",
            vec![item_with_hash(101, HASH, "downloading", false)],
        )
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
        .respond_with(fixture.create_answer(json!({ "torrent_id": 101 })))
        .up_to_n_times(1)
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(fixture.create_answer(json!({ "torrent_id": 102 })))
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(fixture.create_answer(json!({ "usenetdownload_id": 202 })))
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
    let torrent_reads = fixture.requests_to("/v1/api/torrents/mylist").await.len();
    let usenet_reads = fixture.requests_to("/v1/api/usenet/mylist").await.len();
    assert_eq!((torrent_reads, usenet_reads), (2, 1), "one read per create");

    runtime.poll_once().await.unwrap();
    assert_eq!(
        fixture.requests_to("/v1/api/torrents/mylist").await.len(),
        torrent_reads + 1
    );
    assert_eq!(
        fixture.requests_to("/v1/api/usenet/mylist").await.len(),
        usenet_reads + 1
    );
    let list = fixture
        .requests_to("/v1/api/torrents/mylist")
        .await
        .pop()
        .unwrap();
    assert!(
        list.url
            .query_pairs()
            .any(|(key, value)| key == "bypass_cache" && value == "true")
    );
    // The first read learns the torrents' privacy, which is recorded once.
    let writes = runtime.record_writes();
    runtime.poll_once().await.unwrap();
    assert_eq!(
        fixture.requests_to("/v1/api/torrents/mylist").await.len(),
        torrent_reads + 2
    );
    assert_eq!(runtime.record_writes(), writes, "nothing changed");

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

// ---------------------------------------------------------------------------
// ownership of TorBox items

/// A job that holds `torbox_id`, as an earlier grab leaves it.
fn held(kind: TorboxItemKind, name: &str, hash: Option<&str>, torbox_id: i64) -> TorboxJobRecord {
    let mut record = orphan(kind, name, hash);
    record.torbox_id = Some(torbox_id);
    record.phase = TorboxJobPhase::Remote;
    record
}

async fn save_records(fixture: &Fixture, records: &[TorboxJobRecord]) {
    TorboxRecordStore::for_client(fixture.dir.path(), &fixture.client_id)
        .save(records)
        .await
        .unwrap();
}

/// A create whose answer never arrives in a readable form.
fn lost_answer() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_string("<html>synthetic gateway page</html>")
}

#[tokio::test]
async fn a_torrent_the_account_already_holds_is_refused_and_left_alone() {
    let fixture = Fixture::new().await;
    fixture
        .mount_list("torrents", vec![item_with_hash(55, HASH, "seeding", true)])
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(fixture.create_answer(json!({ "torrent_id": 56 })))
        .expect(0)
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect_err("TorBox already holds it");
    assert_eq!(error.reason(), Some(torbox_refusal::DUPLICATE_ITEM));
    assert!(runtime.records().await.is_empty(), "no job was recorded");
    assert!(
        fixture
            .requests_to("/v1/api/torrents/controltorrent")
            .await
            .is_empty(),
        "the item already there is never touched"
    );
    // The account still lists it.
    let still_listed = TorboxApi::new(scryer_outbound_http::generic_reqwest_client(), TOKEN)
        .with_base_url(fixture.server.uri())
        .list(TorboxItemKind::Torrent, Default::default())
        .await
        .unwrap();
    assert_eq!(still_listed[0].id, 55);
}

#[tokio::test]
async fn a_lost_answer_never_adopts_an_item_the_account_already_held() {
    let fixture = Fixture::new().await;
    // Another job of this client holds item 55, which has the same hash.
    let other = held(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.REPACK",
        Some(HASH),
        55,
    );
    save_records(&fixture, std::slice::from_ref(&other)).await;
    fixture
        .mount_list(
            "torrents",
            vec![item_with_hash(55, HASH, "downloading", false)],
        )
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(lost_answer())
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect_err("the answer was lost");
    assert!(error.is_download_submit_ambiguous(), "{error:?}");
    let pending = runtime
        .records()
        .await
        .into_iter()
        .find(|record| record.id != other.id)
        .unwrap();
    assert_eq!(pending.phase, TorboxJobPhase::Submitting);
    assert_eq!(pending.torbox_id, None, "item 55 is not this job's");
    assert_eq!(pending.preexisting_ids, vec![55]);

    // Neither does the poller, nor a restart, take it over.
    runtime.poll_once().await.unwrap();
    let restarted = fixture.restarted(runtime);
    restarted.poll_once().await.unwrap();
    let pending = record_of(&restarted, &pending.id).await;
    assert_eq!(pending.torbox_id, None);
    assert_eq!(record_of(&restarted, &other.id).await.torbox_id, Some(55));
}

#[tokio::test]
async fn a_create_answering_an_item_another_job_holds_is_refused() {
    let fixture = Fixture::new().await;
    let other = held(TorboxItemKind::Usenet, "Synthetic.Movie.2001", None, 55);
    save_records(&fixture, std::slice::from_ref(&other)).await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(fixture.create_answer(json!({ "usenetdownload_id": 55 })))
        .expect(1)
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));

    let error = client
        .submit_download(&nzb_request("Synthetic.Movie.2001.PROPER", None))
        .await
        .expect_err("item 55 belongs to another job");
    assert_eq!(error.reason(), Some(torbox_refusal::DUPLICATE_ITEM));
    let records = runtime.records().await;
    assert_eq!(records.len(), 1, "the refused job was dropped");
    assert_eq!(records[0].id, other.id);
    assert_eq!(records[0].torbox_id, Some(55));
    assert!(
        fixture
            .requests_to("/v1/api/usenet/controlusenetdownload")
            .await
            .is_empty(),
        "the other job's item is never deleted"
    );
}

#[tokio::test]
async fn a_create_answering_an_item_listed_before_it_is_refused() {
    let fixture = Fixture::new().await;
    // Listed before the create, under another name, and held by no job.
    let mut existing = item(77, "completed", true, vec![]);
    existing["name"] = json!("Synthetic.Other.Release");
    fixture.mount_list("usenet", vec![existing]).await;
    Mock::given(method("POST"))
        .and(path("/v1/api/usenet/createusenetdownload"))
        .respond_with(ok(json!({ "usenetdownload_id": 77 })))
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let error = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .submit_download(&nzb_request("Synthetic.Movie.2001", None))
        .await
        .expect_err("item 77 was there before the create");
    assert_eq!(error.reason(), Some(torbox_refusal::DUPLICATE_ITEM));
    assert!(runtime.records().await.is_empty());
    assert!(
        fixture
            .requests_to("/v1/api/usenet/controlusenetdownload")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn deleting_one_job_never_deletes_an_item_another_job_holds() {
    let fixture = Fixture::new().await;
    let other = held(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.REPACK",
        Some(HASH),
        55,
    );
    save_records(&fixture, std::slice::from_ref(&other)).await;
    fixture
        .mount_list(
            "torrents",
            vec![item_with_hash(55, HASH, "downloading", false)],
        )
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(lost_answer())
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/controltorrent"))
        .respond_with(ok(Value::Null))
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect_err("the answer was lost");
    let job = runtime
        .records()
        .await
        .into_iter()
        .find(|record| record.id != other.id)
        .unwrap();

    client
        .delete_queue_item(&job.id, false, false)
        .await
        .unwrap();
    runtime.poll_once().await.unwrap();
    assert!(
        fixture
            .requests_to("/v1/api/torrents/controltorrent")
            .await
            .is_empty(),
        "no delete for item 55"
    );
    assert_eq!(record_of(&runtime, &other.id).await.torbox_id, Some(55));
    let queue = client.list_queue().await.unwrap();
    assert_eq!(
        queue
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![other.id.as_str()],
        "the removed job is gone from the queue"
    );
}

// ---------------------------------------------------------------------------
// privacy of torrents with no seed goal

#[tokio::test]
async fn private_torrent_without_seed_goal_holds_after_a_restart() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let mut private = item(101, "uploading", true, season_files());
    private["private"] = json!(true);
    fixture.mount_list("torrents", vec![private]).await;
    fixture.mount_links("torrents").await;
    let first = fixture.runtime();
    let job_id =
        run_torrent_to_completion(&fixture, &first, &magnet_request("Synthetic.Show.S01.WEB"))
            .await;
    assert_eq!(record_of(&first, &job_id).await.private, Some(true));

    // A fresh runtime reads the persisted file and asks TorBox nothing.
    fixture.server.reset().await;
    let second = fixture.restarted(first);
    let history = TorboxDownloadClient::with_runtime(Arc::clone(&second))
        .list_history()
        .await
        .unwrap();
    let seeding = history[0].seeding.clone().unwrap();
    assert_eq!(history[0].id, job_id);
    assert_eq!(
        seeding.is_private,
        Some(true),
        "the seeding gate holds a private torrent with no goal"
    );
    assert_eq!(seeding.seed_goal_ratio, None);
    assert_eq!(seeding.seed_goal_seconds, None);
    second.poll_once().await.unwrap();
    assert!(fixture.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn torrent_of_unknown_privacy_holds_and_is_read_once() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let mut silent = item(101, "uploading", true, season_files());
    silent.as_object_mut().unwrap().remove("private");
    fixture.mount_list("torrents", vec![silent]).await;
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
        .unwrap();
    assert_eq!(seeding.can_remove, None, "nobody knows whether it may go");
    assert_eq!(seeding.is_private, None);

    // After a restart it is read once more, and then left alone.
    let second = fixture.restarted(first);
    let lists_before = fixture.requests_to("/v1/api/torrents/mylist").await.len();
    second.poll_once().await.unwrap();
    second.poll_once().await.unwrap();
    assert_eq!(
        fixture.requests_to("/v1/api/torrents/mylist").await.len(),
        lists_before + 1,
        "one read, not a standing poll"
    );
    let record = record_of(&second, &job_id).await;
    assert_eq!(record.private, None);
    let seeding = TorboxDownloadClient::with_runtime(Arc::clone(&second))
        .list_history()
        .await
        .unwrap()[0]
        .seeding
        .clone()
        .unwrap();
    assert_eq!(seeding.can_remove, None, "still unknown, still held");
}

#[tokio::test]
async fn privacy_learned_after_the_fetch_is_recorded_once() {
    let fixture = Fixture::new().await;
    let mut record = held(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.WEB",
        Some(HASH),
        101,
    );
    record.phase = TorboxJobPhase::Fetching;
    save_records(&fixture, std::slice::from_ref(&record)).await;
    let mut private = item(101, "uploading", true, vec![]);
    private["private"] = json!(true);
    fixture.mount_list("torrents", vec![private]).await;
    let runtime = fixture.runtime();
    runtime.poll_once().await.unwrap();
    assert_eq!(record_of(&runtime, &record.id).await.private, Some(true));
    let stored = TorboxRecordStore::for_client(fixture.dir.path(), &fixture.client_id)
        .load()
        .await
        .unwrap();
    assert_eq!(stored[0].private, Some(true), "persisted");
    let writes = runtime.record_writes();
    runtime.poll_once().await.unwrap();
    assert_eq!(
        runtime.record_writes(),
        writes,
        "nothing more to read or write"
    );
}

#[tokio::test]
async fn torrent_of_unknown_privacy_whose_item_is_gone_is_removable() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let mut silent = item(101, "uploading", true, season_files());
    silent.as_object_mut().unwrap().remove("private");
    fixture.mount_list("torrents", vec![silent]).await;
    fixture.mount_links("torrents").await;
    let first = fixture.runtime();
    let job_id =
        run_torrent_to_completion(&fixture, &first, &magnet_request("Synthetic.Show.S01.WEB"))
            .await;

    // After a restart, the single-item read confirms TorBox holds it no more.
    fixture.server.reset().await;
    fixture.mount_list("torrents", vec![]).await;
    let second = fixture.restarted(first);
    second.poll_once().await.unwrap();
    let single_reads = fixture
        .requests_to("/v1/api/torrents/mylist")
        .await
        .into_iter()
        .filter(|read| {
            read.url
                .query_pairs()
                .any(|(key, value)| key == "id" && value == "101")
        })
        .count();
    assert_eq!(single_reads, 1, "the item was read by id");
    assert_eq!(record_of(&second, &job_id).await.private, None);
    let seeding = TorboxDownloadClient::with_runtime(Arc::clone(&second))
        .list_history()
        .await
        .unwrap()[0]
        .seeding
        .clone()
        .unwrap();
    assert_eq!(seeding.is_private, None);
    assert_eq!(
        seeding.can_remove,
        Some(true),
        "TorBox holds nothing left to seed"
    );
}

#[tokio::test]
async fn records_written_without_privacy_still_load() {
    let fixture = Fixture::new().await;
    let path = TorboxRecordStore::for_client(fixture.dir.path(), &fixture.client_id)
        .path()
        .to_path_buf();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut record = serde_json::to_value(held(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.WEB",
        Some(HASH),
        101,
    ))
    .unwrap();
    for field in ["private", "preexisting_ids", "cancelled", "never_remove"] {
        record.as_object_mut().unwrap().remove(field);
    }
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({ "version": 1, "jobs": [record] })).unwrap(),
    )
    .unwrap();
    let loaded = TorboxRecordStore::for_client(fixture.dir.path(), &fixture.client_id)
        .load()
        .await
        .unwrap();
    assert_eq!(loaded[0].private, None);
    assert!(loaded[0].preexisting_ids.is_empty());
    assert!(!loaded[0].cancelled);
    assert!(!loaded[0].never_remove);
}

// ---------------------------------------------------------------------------
// leaks

#[tokio::test]
async fn a_moved_download_directory_takes_the_jobs_along() {
    let fixture = Fixture::new().await;
    let old_dir = fixture.dir.path().join("synthetic-old");
    let new_dir = fixture.dir.path().join("synthetic-new");
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::create_dir_all(&new_dir).unwrap();
    let fetching = held(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.WEB",
        Some(HASH),
        101,
    );
    let mut fetching = fetching;
    fetching.phase = TorboxJobPhase::Fetching;
    fetching.engine_job_id = Some("synthetic-engine-job".into());
    fetching.private = Some(false);
    let remote = held(TorboxItemKind::Usenet, "Synthetic.Movie.2001", None, 202);
    let old_store = TorboxRecordStore::for_client(&old_dir, &fixture.client_id);
    old_store
        .save(&[fetching.clone(), remote.clone()])
        .await
        .unwrap();
    let first = fixture.build_runtime_in(
        &old_dir,
        false,
        false,
        TorboxHourlyBudget::default(),
        None,
        None,
    );
    assert_eq!(
        TorboxDownloadClient::with_runtime(Arc::clone(&first))
            .list_queue()
            .await
            .unwrap()
            .len(),
        1,
        "the remote job; the fetch has no engine job here"
    );

    let predecessor = first.retire();
    drop(first);
    let second = fixture.build_runtime_in(
        &new_dir,
        false,
        false,
        TorboxHourlyBudget::default(),
        Some(predecessor),
        Some(old_dir.clone()),
    );
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&second));
    let queue = client.list_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].id, remote.id);
    let carried = TorboxRecordStore::for_client(&new_dir, &fixture.client_id)
        .load()
        .await
        .unwrap();
    assert_eq!(carried.len(), 2, "written at the new location");
    assert_eq!(
        old_store.load().await.unwrap().len(),
        2,
        "the old document is left in place"
    );

    // The fetch's engine job stayed behind, but removing the job still
    // removes its TorBox item.
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/controltorrent"))
        .and(body_json(
            json!({ "torrent_id": 101, "operation": "delete" }),
        ))
        .respond_with(ok(Value::Null))
        .expect(1)
        .mount(&fixture.server)
        .await;
    client
        .delete_queue_item(&fetching.id, true, false)
        .await
        .unwrap();
    assert_eq!(second.records().await.len(), 1);
}

/// A create that answers only when the test lets it.
struct HeldCreate {
    arrived: tokio::sync::mpsc::UnboundedSender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    answer: Value,
}

impl wiremock::Respond for HeldCreate {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let _ = self.arrived.send(());
        self.release
            .lock()
            .unwrap()
            .recv_timeout(WAIT_BOUND)
            .expect("the test releases the create");
        ok(self.answer.clone())
    }
}

/// Mount a held torrent create; returns the arrival signal and the release.
async fn mount_held_create(
    fixture: &Fixture,
    answer: Value,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (arrived_tx, arrived_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(HeldCreate {
            arrived: arrived_tx,
            release: std::sync::Mutex::new(release_rx),
            answer,
        })
        .mount(&fixture.server)
        .await;
    (arrived_rx, release_tx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removal_during_the_create_deletes_what_the_create_made() {
    let fixture = Fixture::new().await;
    let (mut arrived, release) = mount_held_create(&fixture, json!({ "torrent_id": 101 })).await;
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
    let client = Arc::new(TorboxDownloadClient::with_runtime(Arc::clone(&runtime)));
    let submitting = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
                .await
        })
    };
    tokio::time::timeout(WAIT_BOUND, arrived.recv())
        .await
        .expect("the create was sent")
        .unwrap();
    let job = runtime.records().await.remove(0);
    client
        .delete_queue_item(&job.id, false, false)
        .await
        .unwrap();
    assert!(client.list_queue().await.unwrap().is_empty());
    release.send(()).unwrap();

    let error = tokio::time::timeout(WAIT_BOUND, submitting)
        .await
        .expect("the grab returns")
        .unwrap()
        .expect_err("the grab was removed");
    assert_eq!(error.reason(), Some(torbox_refusal::JOB_NOT_FOUND));
    assert!(runtime.records().await.is_empty());
    assert_eq!(
        fixture
            .requests_to("/v1/api/torrents/controltorrent")
            .await
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_delete_after_a_removed_create_is_retried_by_the_poller() {
    let fixture = Fixture::new().await;
    let (mut arrived, release) = mount_held_create(&fixture, json!({ "torrent_id": 101 })).await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/controltorrent"))
        .respond_with(refused(503, "DATABASE_ERROR"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/controltorrent"))
        .respond_with(ok(Value::Null))
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let client = Arc::new(TorboxDownloadClient::with_runtime(Arc::clone(&runtime)));
    let submitting = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
                .await
        })
    };
    tokio::time::timeout(WAIT_BOUND, arrived.recv())
        .await
        .expect("the create was sent")
        .unwrap();
    let job = runtime.records().await.remove(0);
    client
        .delete_queue_item(&job.id, false, false)
        .await
        .unwrap();
    release.send(()).unwrap();
    tokio::time::timeout(WAIT_BOUND, submitting)
        .await
        .expect("the grab returns")
        .unwrap()
        .expect_err("the grab was removed");

    let kept = record_of(&runtime, &job.id).await;
    assert!(kept.cancelled);
    assert_eq!(kept.torbox_id, Some(101), "kept so the delete is retried");
    assert!(client.list_queue().await.unwrap().is_empty());

    runtime.poll_once().await.unwrap();
    assert!(runtime.records().await.is_empty());
    let deletes = fixture.requests_to("/v1/api/torrents/controltorrent").await;
    assert_eq!(deletes.len(), 2);
    for delete in deletes {
        let sent: Value = serde_json::from_slice(&delete.body).unwrap();
        assert_eq!(sent, json!({ "torrent_id": 101, "operation": "delete" }));
    }
}

/// A minimal single-file v1 torrent and its info hash.
fn synthetic_torrent() -> (Vec<u8>, String) {
    let info = b"d6:lengthi1e4:name14:Synthetic.File12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
    let mut bytes = b"d4:info".to_vec();
    bytes.extend_from_slice(info);
    bytes.push(b'e');
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY, info);
    let hash = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    (bytes, hash)
}

fn torrent_file_request(release: &str, bytes: Vec<u8>) -> DownloadClientAddRequest {
    let mut request = magnet_request(release);
    request.resolved_download_artifact = Some(ResolvedDownloadArtifact::TorrentFile {
        bytes,
        file_name: Some("synthetic.torrent".into()),
        info_hash_hint: None,
        content_type: None,
    });
    request
}

#[tokio::test]
async fn a_torrent_file_without_a_hint_is_identified_by_its_info_hash() {
    let fixture = Fixture::new().await;
    let (bytes, hash) = synthetic_torrent();
    fixture
        .mount_list("torrents", vec![item_with_hash(55, &hash, "seeding", true)])
        .await;
    let error = TorboxDownloadClient::with_runtime(fixture.runtime())
        .submit_download(&torrent_file_request(
            "Synthetic.Show.S01.WEB",
            bytes.clone(),
        ))
        .await
        .expect_err("the account already holds that torrent");
    assert_eq!(error.reason(), Some(torbox_refusal::DUPLICATE_ITEM));

    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let runtime = fixture.runtime();
    let grab = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .submit_download(&torrent_file_request("Synthetic.Show.S01.WEB", bytes))
        .await
        .unwrap();
    assert_eq!(grab.info_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(
        record_of(&runtime, &grab.job_id).await.info_hash.as_deref(),
        Some(hash.as_str())
    );
}

#[tokio::test]
async fn an_unreadable_torrent_file_falls_back_to_a_new_item_by_name() {
    let fixture = Fixture::new().await;
    let named = |id: i64| {
        let mut item = item(id, "downloading", false, vec![]);
        item["hash"] = Value::Null;
        item["name"] = json!("Synthetic.Show.S01.WEB");
        item
    };
    // The account had item 55 under the same name; the lost create made 56.
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .respond_with(ok(json!([named(55)])))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list("torrents", vec![named(55), named(56)])
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(lost_answer())
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let error = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .submit_download(&torrent_file_request(
            "Synthetic.Show.S01.WEB",
            b"not bencode".to_vec(),
        ))
        .await;
    let record = runtime.records().await.remove(0);
    assert_eq!(record.info_hash, None);
    assert_eq!(record.preexisting_ids, vec![55]);
    let grab = error.expect("the new item was found by name");
    assert_eq!(grab.job_id, record.id);
    assert_eq!(
        record.torbox_id,
        Some(56),
        "the item the account already had is skipped"
    );
}

// ---------------------------------------------------------------------------
// a bad poll

fn waiting_remote(name: &str, torbox_id: i64) -> TorboxJobRecord {
    held(TorboxItemKind::Usenet, name, None, torbox_id)
}

#[tokio::test]
async fn a_failed_item_read_holds_back_only_its_own_job() {
    let fixture = Fixture::new().await;
    let broken = waiting_remote("Synthetic.Movie.2001", 301);
    let healthy = waiting_remote("Synthetic.Movie.2002", 302);
    save_records(&fixture, &[broken.clone(), healthy.clone()]).await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/mylist"))
        .and(query_param("id", "301"))
        .respond_with(refused(500, "DATABASE_ERROR"))
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/mylist"))
        .and(query_param("id", "302"))
        .respond_with(ok(item(302, "downloading", false, vec![])))
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    runtime
        .poll_once()
        .await
        .expect("one failed read is not a failed poll");
    assert_eq!(
        record_of(&runtime, &broken.id).await.phase,
        TorboxJobPhase::Remote,
        "not failed on a read that did not answer"
    );
    let queue = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .list_queue()
        .await
        .unwrap();
    let healthy = queue.iter().find(|item| item.id == healthy.id).unwrap();
    assert_eq!(healthy.state, DownloadQueueState::Downloading);
}

#[tokio::test]
async fn single_item_reads_are_bounded_and_take_turns() {
    use super::client::MAX_ITEM_READS_PER_TICK;
    let fixture = Fixture::new().await;
    let records: Vec<TorboxJobRecord> = (0..6)
        .map(|n| waiting_remote(&format!("Synthetic.Movie.{n}"), 400 + n))
        .collect();
    save_records(&fixture, &records).await;
    for n in 0..6_i64 {
        Mock::given(method("GET"))
            .and(path("/v1/api/usenet/mylist"))
            .and(query_param("id", (400 + n).to_string()))
            .respond_with(ok(item(400 + n, "downloading", false, vec![])))
            .with_priority(1)
            .mount(&fixture.server)
            .await;
    }
    let runtime = fixture.runtime();
    let single_reads = || async {
        fixture
            .requests_to("/v1/api/usenet/mylist")
            .await
            .into_iter()
            .filter_map(|read| {
                read.url
                    .query_pairs()
                    .find(|(key, _)| key == "id")
                    .map(|(_, id)| id.parse::<i64>().unwrap())
            })
            .collect::<Vec<_>>()
    };
    runtime.poll_once().await.unwrap();
    assert_eq!(single_reads().await.len(), MAX_ITEM_READS_PER_TICK);
    runtime.poll_once().await.unwrap();
    let read: HashSet<i64> = single_reads().await.into_iter().collect();
    assert_eq!(read, (400..406).collect(), "every job is read in turn");
    assert_eq!(single_reads().await.len(), 2 * MAX_ITEM_READS_PER_TICK);
}

#[tokio::test]
async fn a_rate_limited_poll_waits_as_torbox_asks() {
    let fixture = Fixture::new().await;
    save_records(&fixture, &[waiting_remote("Synthetic.Movie.2001", 301)]).await;
    Mock::given(method("GET"))
        .and(path("/v1/api/usenet/mylist"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "120")
                .set_body_json(json!({ "success": false, "error": null, "detail": "slow down" })),
        )
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime();
    let before = tokio::time::Instant::now();
    runtime
        .poll_once()
        .await
        .expect("a rate limit is not a failed poll");
    let not_before = runtime
        .poll_not_before()
        .expect("the next poll is pushed out");
    assert!(not_before >= before + Duration::from_secs(120));
    assert_eq!(
        record_of(&runtime, &runtime.records().await[0].id)
            .await
            .phase,
        TorboxJobPhase::Remote
    );
}

// ---------------------------------------------------------------------------
// engine refusals while starting the fetch

#[test]
fn engine_refusals_are_permanent_only_with_a_permanent_reason() {
    use super::client::engine_refusal_is_transient;
    use scryer_application::AppError;
    let transient = [
        AppError::download_submit_unavailable("the engine is full"),
        AppError::download_submit_unavailable("the engine is shutting down"),
        AppError::validation_refused(
            crate::error_reasons::ACQUISITION_DOWNLOAD_DIRECTORY_UNREACHABLE,
            "unreachable",
        ),
        AppError::Repository("the queue could not be written".into()),
    ];
    for error in &transient {
        assert!(engine_refusal_is_transient(error), "{error:?}");
    }
    let permanent = AppError::validation_refused(
        crate::error_reasons::ACQUISITION_HTTP_DOWNLOAD_ALREADY_SUBMITTED,
        "already submitted",
    );
    assert!(!engine_refusal_is_transient(&permanent));
}

#[tokio::test]
async fn an_unreachable_download_directory_leaves_the_job_with_torbox() {
    let fixture = Fixture::new().await;
    let dir = fixture.dir.path().join("synthetic-library");
    let away = fixture.dir.path().join("synthetic-library-away");
    std::fs::create_dir_all(&dir).unwrap();
    let mut record = held(
        TorboxItemKind::Torrent,
        "Synthetic.Show.S01.WEB",
        Some(HASH),
        101,
    );
    record.job_folder = Some("Synthetic.Show.S01.WEB".into());
    // Already as TorBox reports it, so the poll has nothing to record
    // before it starts the fetch.
    record.private = Some(false);
    TorboxRecordStore::for_client(&dir, &fixture.client_id)
        .save(std::slice::from_ref(&record))
        .await
        .unwrap();
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.build_runtime_in(
        &dir,
        false,
        false,
        TorboxHourlyBudget::default(),
        None,
        None,
    );
    TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .list_queue()
        .await
        .unwrap();

    // The directory goes away (a share that dropped), then comes back.
    std::fs::rename(&dir, &away).unwrap();
    runtime.poll_once().await.unwrap();
    let waiting = record_of(&runtime, &record.id).await;
    assert_eq!(waiting.phase, TorboxJobPhase::Remote, "{:?}", waiting.error);
    assert_eq!(waiting.error_reason, None);

    std::fs::rename(&away, &dir).unwrap();
    runtime.poll_once().await.unwrap();
    assert_eq!(
        record_of(&runtime, &record.id).await.phase,
        TorboxJobPhase::Fetching
    );
}

#[tokio::test]
async fn a_permanent_engine_refusal_fails_the_job() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(fixture.create_answer(json!({ "torrent_id": 101 })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(fixture.create_answer(json!({ "torrent_id": 102 })))
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list(
            "torrents",
            vec![
                item(101, "completed", true, season_files()),
                item(102, "completed", true, season_files()),
            ],
        )
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.runtime();
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    // Two grabs under one download id: the engine takes only the first.
    let first = magnet_request("Synthetic.Show.S01.WEB");
    let mut second = magnet_request("Synthetic.Show.S01.REPACK");
    second.download_id = first.download_id;
    let first = client.submit_download(&first).await.unwrap();
    let second = client.submit_download(&second).await.unwrap();
    runtime.poll_once().await.unwrap();

    assert_eq!(
        record_of(&runtime, &first.job_id).await.phase,
        TorboxJobPhase::Fetching
    );
    let failed = record_of(&runtime, &second.job_id).await;
    assert_eq!(failed.phase, TorboxJobPhase::Failed);
    assert_eq!(
        failed.error_reason.as_deref(),
        Some(crate::error_reasons::ACQUISITION_HTTP_DOWNLOAD_ALREADY_SUBMITTED)
    );
}

// ---------------------------------------------------------------------------
// the hourly budget and retention

#[tokio::test]
async fn a_create_torbox_refuses_gives_its_budget_slot_back() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(refused(400, "BOZO_TORRENT"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    fixture.mount_torrent_create(101).await;
    let budget =
        TorboxHourlyBudget::with_limit(Arc::new(SystemTorboxClock), 1, Duration::from_secs(3600));
    let runtime = fixture.runtime_with(false, false, budget);
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let refused = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect_err("TorBox refused it");
    assert_eq!(refused.reason(), Some(torbox_refusal::INVALID_TORRENT));
    client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect("the refused create did not use the only slot");
    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S02.WEB"))
        .await
        .expect_err("the accepted one did");
    assert_eq!(error.reason(), Some(torbox_refusal::HOURLY_LIMIT));
}

#[test]
fn only_failed_jobs_holding_nothing_on_torbox_are_pruned_past_the_retention() {
    use super::client::prune_finished_records;
    use crate::downloads::clients::http::owned::MAX_RETAINED_FINISHED_JOBS;
    let base = Utc::now() - chrono::Duration::days(30);
    let failed = |n: i64, torbox_id: Option<i64>, reason: &str| {
        let mut record = orphan(
            TorboxItemKind::Usenet,
            &format!("Synthetic.Movie.{n}"),
            None,
        );
        record.torbox_id = torbox_id;
        record.phase = TorboxJobPhase::Failed;
        record.error_reason = Some(reason.into());
        record.updated_at = base + chrono::Duration::seconds(n);
        record
    };
    let excess = 3_i64;
    let empty = MAX_RETAINED_FINISHED_JOBS as i64 + excess;
    let mut records: Vec<TorboxJobRecord> = (0..empty)
        .map(|n| {
            if n % 2 == 0 {
                failed(n, None, torbox_refusal::ITEM_MISSING)
            } else {
                failed(n, Some(n), torbox_refusal::ITEM_MISSING)
            }
        })
        .collect();
    // Oldest of all, but TorBox may still hold their items.
    let holding: Vec<TorboxJobRecord> = (0..5)
        .map(|n| failed(-100 - n, Some(9000 + n), torbox_refusal::REMOTE_FAILED))
        .collect();
    let remote = held(TorboxItemKind::Usenet, "Synthetic.Movie.Remote", None, 8000);
    records.extend(holding.iter().cloned());
    records.push(remote.clone());

    prune_finished_records(&mut records);
    assert_eq!(
        records.len(),
        MAX_RETAINED_FINISHED_JOBS + holding.len() + 1
    );
    let kept: HashSet<&str> = records.iter().map(|record| record.id.as_str()).collect();
    for record in holding.iter().chain(std::iter::once(&remote)) {
        assert!(kept.contains(record.id.as_str()), "{}", record.release_name);
    }
    for n in 0..excess {
        assert!(
            !records
                .iter()
                .any(|record| record.release_name == format!("Synthetic.Movie.{n}")),
            "the oldest empty failure {n} is pruned"
        );
    }
}

// ---------------------------------------------------------------------------
// failed torrents and cleanup

async fn seeding_of(
    runtime: &Arc<TorboxRuntime>,
    job_id: &str,
) -> scryer_domain::DownloadSeedingSnapshot {
    TorboxDownloadClient::with_runtime(Arc::clone(runtime))
        .list_history()
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == job_id)
        .expect("in the history")
        .seeding
        .expect("a torrent reports seeding")
}

#[tokio::test]
async fn a_torrent_that_failed_before_torbox_finished_is_removable() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list("torrents", vec![item(101, "error", false, vec![])])
        .await;
    let runtime = fixture.runtime();
    let grab = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .unwrap();
    runtime.poll_once().await.unwrap();
    let record = record_of(&runtime, &grab.job_id).await;
    assert_eq!(record.phase, TorboxJobPhase::Failed);
    assert_eq!(
        record.error_reason.as_deref(),
        Some(torbox_refusal::REMOTE_FAILED)
    );
    assert_eq!(
        seeding_of(&runtime, &grab.job_id).await.can_remove,
        Some(true),
        "nothing was downloaded, so nothing is seeded"
    );
    let lists = fixture.requests_to("/v1/api/torrents/mylist").await.len();
    runtime.poll_once().await.unwrap();
    assert_eq!(
        fixture.requests_to("/v1/api/torrents/mylist").await.len(),
        lists,
        "nothing more to read"
    );
}

#[tokio::test]
async fn a_torrent_refused_after_torbox_finished_follows_the_privacy_rule() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let mut silent = item(101, "uploading", true, vec![file(1, "Show/readme.txt")]);
    silent.as_object_mut().unwrap().remove("private");
    fixture.mount_list("torrents", vec![silent.clone()]).await;
    let runtime = fixture.runtime();
    let grab = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .unwrap();
    runtime.poll_once().await.unwrap();
    let record = record_of(&runtime, &grab.job_id).await;
    assert_eq!(record.phase, TorboxJobPhase::Failed);
    assert_eq!(
        record.error_reason.as_deref(),
        Some(torbox_refusal::NO_VIDEO_FILE)
    );
    assert!(record.remote_finished_at.is_some());
    assert_eq!(
        seeding_of(&runtime, &grab.job_id).await.can_remove,
        None,
        "TorBox may be seeding a private torrent"
    );

    // The poller reads it once for its privacy, then leaves it alone.
    fixture.server.reset().await;
    let mut private = silent;
    private["private"] = json!(true);
    fixture.mount_list("torrents", vec![private]).await;
    runtime.poll_once().await.unwrap();
    runtime.poll_once().await.unwrap();
    assert_eq!(
        fixture.requests_to("/v1/api/torrents/mylist").await.len(),
        1
    );
    assert_eq!(record_of(&runtime, &grab.job_id).await.private, Some(true));
    let seeding = seeding_of(&runtime, &grab.job_id).await;
    assert_eq!(seeding.is_private, Some(true), "the seeding gate holds it");
    assert_eq!(seeding.seed_goal_ratio, None);
}

#[tokio::test]
async fn a_torrent_refused_after_torbox_finished_waits_for_its_seed_goal() {
    let fixture = Fixture::new().await;
    fixture.mount_torrent_create(101).await;
    let seeding = item(101, "uploading", true, vec![file(1, "Show/readme.txt")]);
    fixture.mount_list("torrents", vec![seeding]).await;
    let runtime = fixture.runtime();
    let mut request = magnet_request("Synthetic.Show.S01.WEB");
    request.seed_goal_ratio = Some(1.0);
    let grab = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .submit_download(&request)
        .await
        .unwrap();
    runtime.poll_once().await.unwrap();
    assert_eq!(
        record_of(&runtime, &grab.job_id).await.phase,
        TorboxJobPhase::Failed
    );
    runtime.poll_once().await.unwrap();
    assert_eq!(
        seeding_of(&runtime, &grab.job_id).await.can_remove,
        Some(false),
        "TorBox still seeds it"
    );

    fixture.server.reset().await;
    fixture
        .mount_list(
            "torrents",
            vec![item(
                101,
                "completed",
                true,
                vec![file(1, "Show/readme.txt")],
            )],
        )
        .await;
    runtime.poll_once().await.unwrap();
    assert_eq!(
        seeding_of(&runtime, &grab.job_id).await.can_remove,
        Some(true),
        "TorBox stopped seeding"
    );
}

// ---------------------------------------------------------------------------
// the job folder on disk

#[tokio::test]
async fn a_job_folder_another_client_made_is_never_used() {
    let fixture = Fixture::new().await;
    let foreign = fixture.dir.path().join("Synthetic.Show.S01.WEB");
    std::fs::create_dir_all(&foreign).unwrap();
    let foreign_file = foreign.join("synthetic-other-client.mkv");
    std::fs::write(&foreign_file, b"synthetic other client bytes").unwrap();
    fixture.mount_torrent_create(101).await;
    fixture
        .mount_list(
            "torrents",
            vec![item(101, "completed", true, season_files())],
        )
        .await;
    fixture.mount_links("torrents").await;
    let runtime = fixture.runtime();
    let job_id = run_torrent_to_completion(
        &fixture,
        &runtime,
        &magnet_request("Synthetic.Show.S01.WEB"),
    )
    .await;

    let record = record_of(&runtime, &job_id).await;
    assert_eq!(
        record.job_folder.as_deref(),
        Some("Synthetic.Show.S01.WEB (2)")
    );
    let completed = TorboxDownloadClient::with_runtime(Arc::clone(&runtime))
        .list_completed_downloads()
        .await
        .unwrap();
    assert_eq!(
        Path::new(&completed[0].dest_dir),
        fixture.dir.path().join("Synthetic.Show.S01.WEB (2)")
    );
    let entries: Vec<_> = std::fs::read_dir(&foreign)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("synthetic-other-client.mkv")]
    );
    assert_eq!(
        std::fs::read(&foreign_file).unwrap(),
        b"synthetic other client bytes"
    );
}

// ---------------------------------------------------------------------------
// a create a gateway answered with a server error

fn budget_of_one() -> TorboxHourlyBudget {
    TorboxHourlyBudget::with_limit(Arc::new(SystemTorboxClock), 1, Duration::from_secs(3600))
}

fn bad_gateway() -> ResponseTemplate {
    ResponseTemplate::new(502).set_body_string("<html>synthetic bad gateway</html>")
}

#[tokio::test]
async fn a_gateway_error_on_the_create_adopts_the_item_torbox_made() {
    let fixture = Fixture::new().await;
    // Empty before the create, the new item after it.
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .respond_with(ok(json!([])))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list(
            "torrents",
            vec![item_with_hash(101, HASH, "downloading", false)],
        )
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(bad_gateway())
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime_with(false, false, budget_of_one());
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let grab = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect("the item TorBox made is found");
    let record = record_of(&runtime, &grab.job_id).await;
    assert_eq!(record.torbox_id, Some(101));
    assert_eq!(record.phase, TorboxJobPhase::Remote);
    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S02.WEB"))
        .await
        .expect_err("the slot stays counted");
    assert_eq!(error.reason(), Some(torbox_refusal::HOURLY_LIMIT));
}

#[tokio::test]
async fn a_gateway_error_with_no_item_leaves_the_job_for_the_poller() {
    let fixture = Fixture::new().await;
    Mock::given(method("POST"))
        .and(path("/v1/api/torrents/createtorrent"))
        .respond_with(bad_gateway())
        .mount(&fixture.server)
        .await;
    let runtime = fixture.runtime_with(false, false, budget_of_one());
    let client = TorboxDownloadClient::with_runtime(Arc::clone(&runtime));
    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
        .await
        .expect_err("nobody knows yet");
    assert!(error.is_download_submit_ambiguous(), "{error:?}");
    let records = runtime.records().await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].phase, TorboxJobPhase::Submitting);
    assert_eq!(records[0].torbox_id, None);
    let error = client
        .submit_download(&magnet_request("Synthetic.Show.S02.WEB"))
        .await
        .expect_err("the slot stays counted");
    assert_eq!(error.reason(), Some(torbox_refusal::HOURLY_LIMIT));
}

// ---------------------------------------------------------------------------
// a submit dropped mid-create

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_submit_is_reconciled_by_the_poller() {
    let fixture = Fixture::new().await;
    Mock::given(method("GET"))
        .and(path("/v1/api/torrents/mylist"))
        .respond_with(ok(json!([])))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&fixture.server)
        .await;
    fixture
        .mount_list(
            "torrents",
            vec![item_with_hash(101, HASH, "downloading", false)],
        )
        .await;
    let (mut arrived, release) = mount_held_create(&fixture, json!({ "torrent_id": 101 })).await;
    let runtime = fixture.runtime();
    let client = Arc::new(TorboxDownloadClient::with_runtime(Arc::clone(&runtime)));
    let submitting = {
        let client = Arc::clone(&client);
        tokio::spawn(async move {
            client
                .submit_download(&magnet_request("Synthetic.Show.S01.WEB"))
                .await
        })
    };
    tokio::time::timeout(WAIT_BOUND, arrived.recv())
        .await
        .expect("the create was sent")
        .unwrap();
    submitting.abort();
    assert!(submitting.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();

    let job_id = runtime.records().await.remove(0).id;
    tokio::time::timeout(WAIT_BOUND, async {
        loop {
            runtime.poll_once().await.unwrap();
            if record_of(&runtime, &job_id).await.torbox_id.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the poller adopts the item the dropped create made");
    let record = record_of(&runtime, &job_id).await;
    assert_eq!(record.torbox_id, Some(101));
    assert_eq!(record.phase, TorboxJobPhase::Remote);
}

// ---------------------------------------------------------------------------
// never-remove profiles

#[tokio::test]
async fn never_remove_without_a_goal_asks_torbox_to_seed_and_is_not_watched() {
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
    let mut request = magnet_request("Synthetic.Show.S01.WEB");
    request.seed_never_remove = true;
    let job_id = run_torrent_to_completion(&fixture, &runtime, &request).await;
    let create = fixture
        .requests_to("/v1/api/torrents/createtorrent")
        .await
        .remove(0);
    assert_eq!(form_text(&create, "seed").as_deref(), Some("2"));
    let record = record_of(&runtime, &job_id).await;
    assert!(record.never_remove);
    assert!(!record.watches_seeding(), "no goal TorBox could reach");
    // Privacy was read with the item; nothing is left to read.
    assert_eq!(record.private, Some(false));
    let seeding = seeding_of(&runtime, &job_id).await;
    assert_eq!(seeding.can_remove, Some(false), "TorBox keeps seeding it");

    // After a restart the flag is still there, and the poller parks
    // without a seeding watch.
    fixture.server.reset().await;
    let restarted = fixture.restarted_with(runtime, true);
    let mut parked = restarted.parked();
    assert_eq!(
        seeding_of(&restarted, &job_id).await.can_remove,
        Some(false)
    );
    assert!(record_of(&restarted, &job_id).await.never_remove);
    tokio::time::timeout(WAIT_BOUND, parked.wait_for(|parked| *parked))
        .await
        .expect("the poller parks")
        .unwrap();
    restarted.poll_once().await.unwrap();
    assert!(
        fixture.server.received_requests().await.unwrap().is_empty(),
        "nothing TorBox reports changes a never-remove torrent"
    );
}
