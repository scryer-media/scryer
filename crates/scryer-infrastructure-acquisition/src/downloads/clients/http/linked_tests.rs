use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use scryer_application::{AppError, http_release_refusal};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::engine::{HttpEngine, HttpEngineLimits, MAX_LINK_REFRESHES};
use super::linked::{
    CONNECTIONS_PER_LINK, LinkOutcome, LinkPurpose, LinkRequest, LinkSource, LinkedFile,
    LinkedJobRequest, ResolvedLink, linked_resource, plan_linked_job,
};
use super::state::{
    HttpFileRole, HttpJob, HttpJobState, HttpQueueDocument, HttpStateStore, RecordedValidator,
    part_path_for,
};
use super::transfer::Egress;
use super::{completed_download_for, job_to_completed_download, queue_item_for};

const BODY: &[u8] = b"synthetic-linked-payload-0123456789";
const ETAG: &str = "\"synthetic-linked-v1\"";
/// A failure bound only; every wait below ends on a state change.
const WAIT_BOUND: Duration = Duration::from_secs(60);
const OWNER_CLIENT_TYPE: &str = "synthetic-debrid";

// ---------------------------------------------------------------------------
// fixtures

type Answer = Box<dyn Fn(&LinkRequest<'_>, usize) -> LinkOutcome + Send + Sync>;

/// A link source that answers from a script: given the request and how many
/// times its key was asked for before, and records every request.
struct ScriptedLinks {
    answer: Answer,
    calls: std::sync::Mutex<Vec<(String, String, LinkPurpose)>>,
}

impl ScriptedLinks {
    fn new(
        answer: impl Fn(&LinkRequest<'_>, usize) -> LinkOutcome + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            answer: Box::new(answer),
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Every request so far, as (key, purpose).
    fn calls(&self) -> Vec<(String, LinkPurpose)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, key, purpose)| (key.clone(), *purpose))
            .collect()
    }
}

#[async_trait]
impl LinkSource for ScriptedLinks {
    async fn resolve(&self, request: &LinkRequest<'_>) -> LinkOutcome {
        let earlier = {
            let mut calls = self.calls.lock().unwrap();
            let earlier = calls
                .iter()
                .filter(|(_, key, _)| key == request.key)
                .count();
            calls.push((
                request.job_id.to_string(),
                request.key.to_string(),
                request.purpose,
            ));
            earlier
        };
        (self.answer)(request, earlier)
    }
}

fn ready(url: &str) -> LinkOutcome {
    LinkOutcome::Ready(ResolvedLink::new(reqwest::Url::parse(url).unwrap()))
}

fn engine_named(
    dir: &Path,
    client_id: &str,
    source: Option<Arc<dyn LinkSource>>,
) -> Arc<HttpEngine> {
    let engine = HttpEngine::new_with_link_source(
        client_id.to_string(),
        dir.to_path_buf(),
        HttpEngineLimits {
            max_concurrent_downloads: 2,
            max_connections_per_host: 2,
            bandwidth_limit_kbps: None,
        },
        Egress::Direct,
        "direct".to_string(),
        None,
        source,
    );
    engine.set_backoff_unit(Duration::ZERO);
    engine
}

fn engine(dir: &Path, source: Arc<ScriptedLinks>) -> Arc<HttpEngine> {
    engine_named(
        dir,
        &format!("synthetic-linked-{}", uuid::Uuid::new_v4()),
        Some(source as Arc<dyn LinkSource>),
    )
}

/// Wait until the job satisfies `done`, re-checking on every queue change.
async fn wait_for(engine: &Arc<HttpEngine>, id: &str, done: impl Fn(&HttpJob) -> bool) -> HttpJob {
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

fn file(role: HttpFileRole, relative_path: &str, key: &str, size: Option<u64>) -> LinkedFile {
    LinkedFile {
        role,
        relative_path: relative_path.to_string(),
        link_key: key.to_string(),
        size_bytes: size,
        language: (role == HttpFileRole::Subtitle).then(|| "en".to_string()),
    }
}

fn job_request(folder: &str, files: Vec<LinkedFile>) -> LinkedJobRequest {
    LinkedJobRequest {
        download_id: Some(uuid::Uuid::new_v4().to_string()),
        title_id: "title-synthetic".into(),
        title_name: "Synthetic Show".into(),
        facet: "series".into(),
        release_name: "Synthetic.Show.S01.WEB".into(),
        category: Some("series".into()),
        job_folder: folder.to_string(),
        files,
        expected_size_bytes: None,
        expected_duration_seconds: None,
        verify: Default::default(),
        subtitle_required: false,
    }
}

fn media(relative_path: &str, key: &str) -> LinkedFile {
    file(
        HttpFileRole::Media,
        relative_path,
        key,
        Some(BODY.len() as u64),
    )
}

fn plan(request: LinkedJobRequest) -> Result<HttpJob, AppError> {
    plan_linked_job(
        request,
        Path::new("/srv/synthetic"),
        "planned".into(),
        chrono::Utc::now(),
    )
}

/// Every byte the downloader recorded under its state directory.
fn recorded_state(dir: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut pending = vec![dir.join(super::state::STATE_DIR_NAME)];
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

/// An HTTP server that answers each path in `bodies` with its body and
/// counts the connections it is answering at once. A connection counts as
/// active from its request until all but the last byte of its answer is
/// written, so only a genuinely overlapping request can raise the peak.
async fn counting_server(bodies: HashMap<String, Vec<u8>>) -> (String, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let reported = Arc::clone(&peak);
    let bodies = Arc::new(bodies);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (active, peak, bodies) =
                (Arc::clone(&active), Arc::clone(&peak), Arc::clone(&bodies));
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                let head = String::from_utf8_lossy(&head).into_owned();
                let target = head.split_whitespace().nth(1).unwrap_or_default();
                let path = target.split('?').next().unwrap_or_default();
                let Some(body) = bodies.get(path) else {
                    active.fetch_sub(1, Ordering::SeqCst);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    return;
                };
                let mut answer = format!(
                    "HTTP/1.1 200 OK\r\nETag: {ETAG}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                answer.extend_from_slice(&body[..body.len() - 1]);
                let _ = stream.write_all(&answer).await;
                let _ = stream.flush().await;
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = stream.write_all(&body[body.len() - 1..]).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{address}"), reported)
}

// ---------------------------------------------------------------------------
// planning

#[test]
fn paths_that_leave_or_blur_the_job_folder_are_refused_by_code() {
    for bad in [
        "",
        "/abs/Synthetic.Show.S01E01.mkv",
        "../Synthetic.Show.S01E01.mkv",
        "Season 01/../../Synthetic.Show.S01E01.mkv",
        "Season 01/./Synthetic.Show.S01E01.mkv",
        "Season 01//Synthetic.Show.S01E01.mkv",
        "Season 01/",
        "Season 01\\..\\Synthetic.Show.S01E01.mkv",
        "C:Synthetic.Show.S01E01.mkv",
        "Season 01/.. /Synthetic.Show.S01E01.mkv",
        "Season 01/Synthetic.Show.S01E01.mkv.",
        "Season 01/nul.mkv",
        "Season 01/Synthetic\0.mkv",
    ] {
        let error = plan(job_request(
            "Synthetic.Show.S01",
            vec![media(bad, "item-1/file-1")],
        ))
        .expect_err(bad);
        assert_eq!(
            error.validation_reason(),
            Some(http_release_refusal::FILE_PATH_INVALID),
            "{bad:?}: {error}"
        );
    }
    for folder in ["", ".", "..", ".scryer-http", "a/b", "a\\b"] {
        let error = plan(job_request(
            folder,
            vec![media("Synthetic.Show.S01E01.mkv", "item-1/file-1")],
        ))
        .expect_err(folder);
        assert_eq!(
            error.validation_reason(),
            Some(http_release_refusal::FILE_PATH_INVALID),
            "{folder:?}"
        );
    }
    // Two files under one name, without regard to case, and a file where
    // another needs a folder.
    for (first, second) in [
        ("Season 01/A.mkv", "season 01/a.MKV"),
        ("Season 01", "Season 01/A.mkv"),
        ("Season 01/A.mkv", "Season 01"),
    ] {
        let error = plan(job_request(
            "Synthetic.Show.S01",
            vec![
                media(first, "item-1/file-1"),
                media(second, "item-1/file-2"),
            ],
        ))
        .expect_err(second);
        assert_eq!(
            error.validation_reason(),
            Some(http_release_refusal::FILE_PATH_INVALID),
            "{first:?} then {second:?}"
        );
    }
}

#[test]
fn a_planned_job_lands_under_its_folder_and_records_keys_not_urls() {
    let job = plan(job_request(
        "Synthetic.Show.S01",
        vec![
            media("Season 01/Synthetic.Show.S01E01.mkv", "item-1/file-1"),
            file(
                HttpFileRole::Subtitle,
                "Season 01/Subs/Synthetic.Show.S01E01.en.srt",
                "item-1/file-2",
                None,
            ),
        ],
    ))
    .unwrap();
    let folder = Path::new("/srv/synthetic/Synthetic.Show.S01");
    assert_eq!(job.output_dir, folder);
    assert_eq!(job.output_basename, "Synthetic.Show.S01");
    assert_eq!(
        job.files[0].final_path,
        folder.join("Season 01").join("Synthetic.Show.S01E01.mkv")
    );
    assert_eq!(
        job.files[1].final_path,
        folder
            .join("Season 01")
            .join("Subs")
            .join("Synthetic.Show.S01E01.en.srt")
    );
    assert!(job.files.iter().all(|file| file.resource.url.is_empty()));
    assert_eq!(job.files[0].link_key.as_deref(), Some("item-1/file-1"));
    assert_eq!(job.files[0].resource.size_bytes, Some(BODY.len() as i64));
    assert!(job.reports_folder());
}

#[test]
fn a_job_without_media_too_many_files_or_an_empty_key_is_refused_by_code() {
    let subtitle_only = plan(job_request(
        "Synthetic.Show.S01",
        vec![file(HttpFileRole::Subtitle, "a.srt", "item-1/file-1", None)],
    ))
    .unwrap_err();
    assert_eq!(
        subtitle_only.validation_reason(),
        Some(http_release_refusal::NO_MEDIA_RESOURCE)
    );
    let crowded = plan(job_request(
        "Synthetic.Show.S01",
        (0..=super::linked::MAX_LINKED_JOB_FILES)
            .map(|index| media(&format!("f{index}.mkv"), "item-1/file"))
            .collect(),
    ))
    .unwrap_err();
    assert_eq!(
        crowded.validation_reason(),
        Some(http_release_refusal::TOO_MANY_FILES)
    );
    let keyless = plan(job_request("Synthetic.Show.S01", vec![media("a.mkv", " ")])).unwrap_err();
    assert_eq!(
        keyless.validation_reason(),
        Some(http_release_refusal::LINK_KEY_INVALID)
    );
}

#[tokio::test]
async fn an_engine_without_a_link_source_refuses_linked_jobs_and_queues_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_named(dir.path(), "synthetic-plain", None);
    let error = engine
        .add_linked_job(job_request(
            "Synthetic.Show.S01",
            vec![media("a.mkv", "item-1/file-1")],
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.validation_reason(),
        Some(http_release_refusal::LINK_SOURCE_UNAVAILABLE)
    );
    assert!(engine.jobs_where(|_| true).await.unwrap().is_empty());
}

#[test]
fn a_resolved_link_debugs_as_its_host_only() {
    let link = ResolvedLink::new(
        reqwest::Url::parse("https://cdn.synthetic.invalid/dl/1?token=link-secret").unwrap(),
    );
    let shown = format!("{link:?}");
    assert!(shown.contains("cdn.synthetic.invalid"), "{shown}");
    assert!(!shown.contains("link-secret"), "{shown}");
}

// ---------------------------------------------------------------------------
// transfers

#[tokio::test]
async fn a_multi_file_job_completes_into_its_folder_one_connection_at_a_time() {
    let first = b"synthetic-episode-one-payload".to_vec();
    let second = b"synthetic-episode-two-payload-longer".to_vec();
    let subtitle = b"1\n00:00:01,000 --> 00:00:02,000\nsynthetic\n".to_vec();
    let (base, peak) = counting_server(HashMap::from([
        ("/dl/one".to_string(), first.clone()),
        ("/dl/two".to_string(), second.clone()),
        ("/dl/sub".to_string(), subtitle.clone()),
    ]))
    .await;
    let links = ScriptedLinks::new(move |request, _| {
        let name = request.key.rsplit('/').next().unwrap();
        ready(&format!("{base}/dl/{name}?token=secret-{name}"))
    });
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), Arc::clone(&links));
    // A checkpoint after every byte, so progress is recorded while each
    // link is in use.
    engine.set_checkpoint_cadence(1, Duration::from_secs(3600));

    let id = engine
        .add_linked_job(job_request(
            "Synthetic.Show.S01",
            vec![
                file(
                    HttpFileRole::Media,
                    "Season 01/Synthetic.Show.S01E01.mkv",
                    "item-7/one",
                    Some(first.len() as u64),
                ),
                file(
                    HttpFileRole::Media,
                    "Season 01/Synthetic.Show.S01E02.mkv",
                    "item-7/two",
                    Some(second.len() as u64),
                ),
                file(
                    HttpFileRole::Subtitle,
                    "Season 01/Subs/Synthetic.Show.S01E01.en.srt",
                    "item-7/sub",
                    None,
                ),
            ],
        ))
        .await
        .unwrap();
    let done = wait_for(&engine, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);

    let folder = dir.path().join("Synthetic.Show.S01");
    let season = folder.join("Season 01");
    assert_eq!(
        std::fs::read(season.join("Synthetic.Show.S01E01.mkv")).unwrap(),
        first
    );
    assert_eq!(
        std::fs::read(season.join("Synthetic.Show.S01E02.mkv")).unwrap(),
        second
    );
    let subtitle_path = season.join("Subs").join("Synthetic.Show.S01E01.en.srt");
    assert_eq!(std::fs::read(&subtitle_path).unwrap(), subtitle);

    // Each file asked for its link once, in order, and no two transfers
    // ever overlapped.
    assert_eq!(
        links.calls(),
        vec![
            ("item-7/one".to_string(), LinkPurpose::Start),
            ("item-7/two".to_string(), LinkPurpose::Start),
            ("item-7/sub".to_string(), LinkPurpose::Start),
        ]
    );
    assert_eq!(peak.load(Ordering::SeqCst), CONNECTIONS_PER_LINK);

    // Reported as its folder, sized across every file.
    let total = (first.len() + second.len() + subtitle.len()) as u64;
    assert_eq!(done.durable_bytes(), total);
    assert_eq!(done.known_total_bytes(), Some(total));
    let completed = completed_download_for(&done, OWNER_CLIENT_TYPE).unwrap();
    assert_eq!(completed.client_type, OWNER_CLIENT_TYPE);
    assert_eq!(completed.dest_dir, folder.to_string_lossy());
    assert_eq!(completed.name, "Synthetic.Show.S01");
    assert_eq!(completed.size_bytes, Some(total as i64));
    assert_eq!(
        completed.subtitle_paths,
        vec![subtitle_path.to_string_lossy().into_owned()]
    );
    let item = queue_item_for(&done, None, OWNER_CLIENT_TYPE);
    assert_eq!(item.client_type, OWNER_CLIENT_TYPE);
    assert_eq!(item.size_bytes, Some(total as i64));
    assert_eq!(item.progress_percent, 100);

    // Progress was recorded while the links were in use, yet no link
    // reached the queue document or a progress file.
    assert!(engine.progress_writes() >= 3);
    let recorded = recorded_state(dir.path());
    assert!(!contains(&recorded, "secret-"));
    assert!(!contains(&recorded, "/dl/"));
}

#[tokio::test]
async fn a_restart_mid_job_resumes_the_unfinished_file_with_a_fresh_link() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dl/two"))
        .and(query_param("token", "resume-secret"))
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
    let client_id = "synthetic-linked-restart";
    // As an interruption leaves it: the first file finished, the second
    // four bytes in, the job still marked as downloading.
    let mut job = plan_linked_job(
        job_request(
            "Synthetic.Show.S01",
            vec![
                media("Synthetic.Show.S01E01.mkv", "item-7/one"),
                media("Synthetic.Show.S01E02.mkv", "item-7/two"),
            ],
        ),
        dir.path(),
        "linked-item".into(),
        chrono::Utc::now(),
    )
    .unwrap();
    std::fs::create_dir_all(&job.output_dir).unwrap();
    let done_file = &mut job.files[0];
    std::fs::write(&done_file.final_path, BODY).unwrap();
    done_file.complete = true;
    done_file.bytes_done = BODY.len() as u64;
    done_file.total_bytes = Some(BODY.len() as u64);
    let partial = &mut job.files[1];
    let part_path = part_path_for(&partial.final_path, 1);
    std::fs::write(&part_path, &BODY[..4]).unwrap();
    partial.partial_path = Some(part_path);
    partial.bytes_done = 4;
    partial.total_bytes = Some(BODY.len() as u64);
    partial.validator = Some(RecordedValidator {
        etag: Some(ETAG.into()),
        last_modified: None,
    });
    job.state = HttpJobState::Downloading;
    HttpStateStore::for_client(dir.path(), client_id)
        .save(&HttpQueueDocument {
            version: 1,
            jobs: vec![job.clone()],
        })
        .await
        .unwrap();

    let uri = server.uri();
    let links = ScriptedLinks::new(move |request, _| match request.key {
        "item-7/two" => ready(&format!("{uri}/dl/two?token=resume-secret")),
        _ => LinkOutcome::Refused {
            reason: http_release_refusal::LINK_REFUSED,
            message: "only the unfinished file should ask for a link".into(),
        },
    });
    let engine = engine_named(
        dir.path(),
        client_id,
        Some(Arc::clone(&links) as Arc<dyn LinkSource>),
    );
    engine.ensure_started().await.unwrap();
    let done = wait_for(&engine, &job.id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(
        links.calls(),
        vec![("item-7/two".to_string(), LinkPurpose::Resume)]
    );
    assert_eq!(done.files[1].attempt, 1);
    assert_eq!(std::fs::read(&done.files[1].final_path).unwrap(), BODY);
    assert!(!contains(&recorded_state(dir.path()), "resume-secret"));
}

#[tokio::test]
async fn a_refused_link_is_replaced_by_a_fresh_one() {
    let server = MockServer::start().await;
    for (token, status) in [("first", 410), ("second", 403)] {
        Mock::given(method("GET"))
            .and(path("/dl/one"))
            .and(query_param("token", token))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/dl/one"))
        .and(query_param("token", "third"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;

    let uri = server.uri();
    let links = ScriptedLinks::new(move |_, earlier| {
        let token = ["first", "second", "third"][earlier];
        ready(&format!("{uri}/dl/one?token={token}"))
    });
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), Arc::clone(&links));
    let id = engine
        .add_linked_job(job_request(
            "Synthetic.Movie.2001",
            vec![media("Synthetic.Movie.2001.mkv", "item-3/one")],
        ))
        .await
        .unwrap();
    let done = wait_for(&engine, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(MAX_LINK_REFRESHES, 2);
    assert_eq!(
        links.calls(),
        vec![
            ("item-3/one".to_string(), LinkPurpose::Start),
            ("item-3/one".to_string(), LinkPurpose::Refused),
            ("item-3/one".to_string(), LinkPurpose::Refused),
        ]
    );
    // A refused link is neither a geo block nor a media variant.
    assert_eq!(done.forbidden_requests, 0);
    assert_eq!(done.files[0].forbidden_answers, 0);
    assert!(done.refused_variants.is_empty());
    assert_eq!(done.files[0].attempt, 1);
    assert_eq!(std::fs::read(&done.files[0].final_path).unwrap(), BODY);
}

#[tokio::test]
async fn links_refused_past_the_bound_fail_the_job_with_a_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dl/one"))
        .respond_with(ResponseTemplate::new(403))
        .expect(u64::from(MAX_LINK_REFRESHES) + 1)
        .mount(&server)
        .await;
    let uri = server.uri();
    let links = ScriptedLinks::new(move |_, earlier| {
        ready(&format!("{uri}/dl/one?token=attempt-{earlier}"))
    });
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), Arc::clone(&links));
    let id = engine
        .add_linked_job(job_request(
            "Synthetic.Movie.2001",
            vec![media("Synthetic.Movie.2001.mkv", "item-3/one")],
        ))
        .await
        .unwrap();
    let done = wait_for(&engine, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    assert_eq!(
        done.error_reason.as_deref(),
        Some(http_release_refusal::LINK_REFUSED)
    );
    assert_eq!(links.calls().len(), MAX_LINK_REFRESHES as usize + 1);
    assert!(!contains(&recorded_state(dir.path()), "attempt-"));
}

#[tokio::test]
async fn a_source_asking_to_wait_is_waited_out_and_asked_again() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dl/one"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;
    let uri = server.uri();
    let links = ScriptedLinks::new(move |_, earlier| match earlier {
        0 => LinkOutcome::RetryAfter {
            wait: Duration::from_millis(5),
            message: "the file is still being prepared".into(),
        },
        _ => ready(&format!("{uri}/dl/one?token=late")),
    });
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), Arc::clone(&links));
    let id = engine
        .add_linked_job(job_request(
            "Synthetic.Movie.2001",
            vec![media("Synthetic.Movie.2001.mkv", "item-3/one")],
        ))
        .await
        .unwrap();
    let done = wait_for(&engine, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Completed, "{:?}", done.error);
    assert_eq!(done.error, None);
    assert_eq!(engine.link_waits(), vec![Duration::from_millis(5)]);
    assert_eq!(
        links.calls(),
        vec![
            ("item-3/one".to_string(), LinkPurpose::Start),
            ("item-3/one".to_string(), LinkPurpose::Start),
        ]
    );
}

#[tokio::test]
async fn a_permanent_refusal_from_the_source_fails_the_job_with_its_reason() {
    let links = ScriptedLinks::new(|_, _| LinkOutcome::Refused {
        reason: http_release_refusal::REMOVED,
        message: "the remote item was removed".into(),
    });
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), Arc::clone(&links));
    let id = engine
        .add_linked_job(job_request(
            "Synthetic.Movie.2001",
            vec![media("Synthetic.Movie.2001.mkv", "item-3/one")],
        ))
        .await
        .unwrap();
    let done = wait_for(&engine, &id, |job| job.state.is_terminal()).await;
    assert_eq!(done.state, HttpJobState::Failed);
    assert_eq!(
        done.error_reason.as_deref(),
        Some(http_release_refusal::REMOVED)
    );
    assert_eq!(done.error.as_deref(), Some("the remote item was removed"));
    assert_eq!(links.calls().len(), 1);
}

// ---------------------------------------------------------------------------
// compatibility

#[tokio::test]
async fn a_single_file_queue_document_from_before_link_keys_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("Synthetic.Show.S01E01");
    let final_path = output_dir.join("Synthetic.Show.S01E01.mp4");
    let mut resource = serde_json::to_value(linked_resource(
        HttpFileRole::Media,
        Some(BODY.len() as u64),
    ))
    .unwrap();
    resource["url"] = "https://media.invalid/clip.mp4".into();
    let document = serde_json::json!({
        "version": 1,
        "jobs": [{
            "id": "legacy-item",
            "title_id": "title-synthetic",
            "title_name": "Synthetic Show",
            "facet": "series",
            "release_name": "Synthetic.Show.S01E01.WEB",
            "output_basename": "Synthetic.Show.S01E01",
            "output_dir": output_dir,
            "files": [{
                "role": "media",
                "resource": resource,
                "final_path": final_path,
                "attempt": 1,
                "bytes_done": BODY.len(),
                "total_bytes": BODY.len(),
                "complete": true
            }],
            "state": "completed",
            "queued_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:01:00Z",
            "completed_at": "2026-01-01T00:01:00Z"
        }]
    });
    let store = HttpStateStore::for_client(dir.path(), "synthetic-legacy");
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(store.path(), serde_json::to_vec(&document).unwrap()).unwrap();

    let loaded = store.load().await.unwrap();
    let job = &loaded.jobs[0];
    assert_eq!(job.files[0].link_key, None);
    assert!(!job.reports_folder());
    let completed = job_to_completed_download(job).unwrap();
    assert_eq!(completed.client_type, "http");
    assert_eq!(completed.dest_dir, final_path.to_string_lossy());

    // Saved again, it gains no field it did not have.
    store.save(&loaded).await.unwrap();
    let saved = std::fs::read(store.path()).unwrap();
    assert!(!contains(&saved, "link_key"));
}
