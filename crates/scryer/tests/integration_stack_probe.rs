#![recursion_limit = "256"]

mod common;

use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use common::TestContext;
use scryer_application::{SettingsRepository, ShowRepository, TitleRepository};
use scryer_domain::MediaFacet;
use scryer_infrastructure::{SettingDefinitionSeed, SqliteServices};

const HAIKYU_TVDB_ID: i64 = 420_424;
const MINIMUM_HAIKYU_IMPORTED_FILE_COUNT: usize = 12;

#[test]
fn haikyu_post_hydration_title_scan_subprocess_probe() {
    let exe = std::env::current_exe().expect("resolve current test executable");
    let mut child = Command::new(exe)
        .arg("--exact")
        .arg("haikyu_post_hydration_title_scan_subprocess_probe_child")
        .arg("--ignored")
        .arg("--nocapture")
        .env("RUST_TEST_THREADS", "1")
        .env("RUST_BACKTRACE", "1")
        .env("SCRYER_STACK_PROBE_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn subprocess stack probe");

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if child
            .try_wait()
            .expect("poll subprocess stack probe status")
            .is_some()
        {
            let output = child
                .wait_with_output()
                .expect("collect subprocess stack probe output");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "stack probe child failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout,
                stderr
            );
            return;
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .expect("collect timed-out subprocess stack probe output");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!(
                "stack probe child timed out after {}s\nstdout:\n{}\nstderr:\n{}",
                deadline.elapsed().as_secs(),
                stdout,
                stderr
            );
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "subprocess-only stack probe child"]
fn haikyu_post_hydration_title_scan_subprocess_probe_child() {
    if std::env::var_os("SCRYER_STACK_PROBE_CHILD").is_none() {
        return;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()
        .expect("build tokio runtime for stack probe");

    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), run_haikyu_stack_probe())
            .await
            .expect("stack probe should finish within timeout");
    });
}

async fn run_haikyu_stack_probe() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    install_haikyu_metadata_fixture(&ctx).await;

    let media_root = tempfile::tempdir().expect("create anime media root");
    let show_dir = create_haikyu_library_root(media_root.path());
    set_media_path(
        &ctx,
        "anime.path",
        media_root.path().to_string_lossy().as_ref(),
    )
    .await;

    let token = tokio_util::sync::CancellationToken::new();
    let hydration_app = ctx.app.clone();
    let hydration_token = token.clone();
    tokio::spawn(async move {
        scryer_application::start_background_hydration_loop(hydration_app, hydration_token).await;
    });
    let title_scan_app = ctx.app.clone();
    let title_scan_token = token.clone();
    tokio::spawn(async move {
        scryer_application::start_background_post_hydration_title_scan_workers(
            title_scan_app,
            title_scan_token,
        )
        .await;
    });

    let admin = ctx.app.find_or_create_default_user().await.unwrap();

    eprintln!("stack probe: first manual anime library scan");
    ctx.app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("first anime library scan should succeed");
    let first_metadata_fetched_at =
        wait_for_haikyu_scan_to_settle(&ctx, Some(MINIMUM_HAIKYU_IMPORTED_FILE_COUNT)).await;

    eprintln!("stack probe: second manual anime library scan");
    ctx.app
        .scan_library(&admin, MediaFacet::Anime)
        .await
        .expect("second anime library scan should succeed");
    wait_for_library_scan_sessions_to_clear(&ctx).await;

    let titles = ctx
        .db
        .list(Some(MediaFacet::Anime), None)
        .await
        .expect("list anime titles after stack probe");
    assert_eq!(titles.len(), 1, "expected exactly one anime title");
    assert_eq!(titles[0].name, "Haikyu!!");
    assert_eq!(
        titles[0].folder_path.as_deref(),
        Some(show_dir.to_string_lossy().as_ref())
    );
    assert!(
        titles[0]
            .metadata_fetched_at
            .is_some_and(|value| value == first_metadata_fetched_at),
        "second manual scan should not force a metadata re-hydration for the same title",
    );

    eprintln!("stack probe: manually re-enqueueing post-hydration title scan for existing title");
    assert!(
        ctx.app
            .services
            .post_hydration_title_scan_queue
            .enqueue(titles[0].id.clone())
            .await,
        "post-hydration queue should accept the replayed title scan"
    );
    sleep(Duration::from_secs(2)).await;

    let collections = ctx
        .db
        .list_collections_for_title(&titles[0].id)
        .await
        .expect("list anime collections after stack probe");
    assert!(
        collections
            .iter()
            .any(|collection| collection.collection_type
                == scryer_domain::CollectionType::Interstitial),
        "expected at least one interstitial collection to validate the complex anime shape",
    );

    let media_files = ctx
        .db
        .list_media_files_for_title(&titles[0].id)
        .await
        .expect("list anime media files after stack probe");
    assert!(
        media_files.len() >= MINIMUM_HAIKYU_IMPORTED_FILE_COUNT,
        "expected most Haikyu!! files to be inserted during the stack probe, got {}",
        media_files.len()
    );

    token.cancel();
}

async fn seed_media_path_settings(ctx: &TestContext) {
    ctx.db
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/movies\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/series\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/anime\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed media path setting definitions");
}

async fn set_media_path(ctx: &TestContext, key_name: &str, value: &str) {
    <SqliteServices as SettingsRepository>::upsert_setting_json(
        &ctx.db,
        "media",
        key_name,
        None,
        serde_json::to_string(value).expect("serialize setting value"),
        "integration_test",
        None,
    )
    .await
    .expect("upsert media path setting");
}

async fn install_haikyu_metadata_fixture(ctx: &TestContext) {
    let fixture = build_haikyu_metadata_fixture();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;
}

fn build_haikyu_metadata_fixture() -> String {
    let seasons = vec![
        json!({ "tvdb_id": 610000, "number": 0, "label": "Specials", "episode_type": "special" }),
        json!({ "tvdb_id": 610001, "number": 1, "label": "Season 1", "episode_type": "default" }),
        json!({ "tvdb_id": 610002, "number": 2, "label": "Season 2", "episode_type": "default" }),
        json!({ "tvdb_id": 610003, "number": 3, "label": "Season 3", "episode_type": "default" }),
        json!({ "tvdb_id": 610004, "number": 4, "label": "Season 4", "episode_type": "default" }),
        json!({ "tvdb_id": 610005, "number": 5, "label": "Season 5", "episode_type": "default" }),
    ];

    let season_counts = [(0_i32, 4_i32), (1, 25), (2, 25), (3, 10), (4, 25), (5, 10)];
    let mut episodes = Vec::new();
    let mut tvdb_episode_id = 710000_i64;
    let mut absolute_number = 1_i32;
    for (season_number, count) in season_counts {
        for episode_number in 1..=count {
            let absolute = if season_number == 0 {
                String::new()
            } else {
                let value = absolute_number.to_string();
                absolute_number += 1;
                value
            };
            let aired_month = if season_number == 0 {
                1_u32
            } else {
                u32::try_from(season_number + 1).unwrap_or(1)
            };
            let aired_day = u32::try_from(((episode_number - 1) % 28) + 1).unwrap_or(1);
            episodes.push(json!({
                "tvdb_id": tvdb_episode_id,
                "episode_number": episode_number,
                "season_number": season_number,
                "name": if season_number == 0 {
                    format!("Special {episode_number}")
                } else {
                    format!("Haikyu Episode {absolute}")
                },
                "aired": format!("2014-{aired_month:02}-{aired_day:02}"),
                "runtime_minutes": 24,
                "is_filler": false,
                "is_recap": false,
                "overview": format!("Episode {tvdb_episode_id} overview"),
                "absolute_number": absolute
            }));
            tvdb_episode_id += 1;
        }
    }

    let anime_movies = vec![
        json!({
            "movie_tvdb_id": 880001,
            "movie_tmdb_id": 770001,
            "movie_imdb_id": "tt4200001",
            "movie_mal_id": 50001,
            "movie_anidb_id": null,
            "name": "Haikyu!! Movie 1",
            "slug": "haikyu-movie-1",
            "year": 2015,
            "content_status": "released",
            "overview": "Interstitial movie 1",
            "poster_url": "https://example.invalid/haikyu-movie-1.jpg",
            "language": "eng",
            "runtime_minutes": 90,
            "sort_title": "Haikyu Movie 1",
            "imdb_id": "tt4200001",
            "genres": ["Animation", "Sports"],
            "studio": "Production I.G",
            "digital_release_date": "2015-11-01",
            "association_confidence": "high",
            "continuity_status": "canon",
            "movie_form": "movie",
            "placement": "ordered",
            "confidence": "high",
            "signal_summary": "Mapped TVDB special to interstitial movie 1"
        }),
        json!({
            "movie_tvdb_id": 880002,
            "movie_tmdb_id": 770002,
            "movie_imdb_id": "tt4200002",
            "movie_mal_id": 50002,
            "movie_anidb_id": null,
            "name": "Haikyu!! Movie 2",
            "slug": "haikyu-movie-2",
            "year": 2016,
            "content_status": "released",
            "overview": "Interstitial movie 2",
            "poster_url": "https://example.invalid/haikyu-movie-2.jpg",
            "language": "eng",
            "runtime_minutes": 92,
            "sort_title": "Haikyu Movie 2",
            "imdb_id": "tt4200002",
            "genres": ["Animation", "Sports"],
            "studio": "Production I.G",
            "digital_release_date": "2016-11-01",
            "association_confidence": "high",
            "continuity_status": "canon",
            "movie_form": "movie",
            "placement": "ordered",
            "confidence": "high",
            "signal_summary": "Mapped TVDB special to interstitial movie 2"
        }),
        json!({
            "movie_tvdb_id": 880003,
            "movie_tmdb_id": 770003,
            "movie_imdb_id": "tt4200003",
            "movie_mal_id": 50003,
            "movie_anidb_id": null,
            "name": "Haikyu!! Movie 3",
            "slug": "haikyu-movie-3",
            "year": 2017,
            "content_status": "released",
            "overview": "Interstitial movie 3",
            "poster_url": "https://example.invalid/haikyu-movie-3.jpg",
            "language": "eng",
            "runtime_minutes": 94,
            "sort_title": "Haikyu Movie 3",
            "imdb_id": "tt4200003",
            "genres": ["Animation", "Sports"],
            "studio": "Production I.G",
            "digital_release_date": "2017-11-01",
            "association_confidence": "high",
            "continuity_status": "canon",
            "movie_form": "movie",
            "placement": "ordered",
            "confidence": "high",
            "signal_summary": "Mapped TVDB special to interstitial movie 3"
        }),
    ];

    let anime_mappings = vec![
        json!({
            "mal_id": 50001,
            "anilist_id": null,
            "anidb_id": null,
            "kitsu_id": null,
            "thetvdb_id": 880001,
            "themoviedb_id": 770001,
            "alt_tvdb_id": 880001,
            "thetvdb_season": 0,
            "score": null,
            "anime_media_type": "MOVIE",
            "global_media_type": "movie",
            "status": "finished",
            "mapping_type": "special",
            "episode_mappings": [{ "tvdb_season": 0, "episode_start": 1, "episode_end": 1 }]
        }),
        json!({
            "mal_id": 50002,
            "anilist_id": null,
            "anidb_id": null,
            "kitsu_id": null,
            "thetvdb_id": 880002,
            "themoviedb_id": 770002,
            "alt_tvdb_id": 880002,
            "thetvdb_season": 0,
            "score": null,
            "anime_media_type": "MOVIE",
            "global_media_type": "movie",
            "status": "finished",
            "mapping_type": "special",
            "episode_mappings": [{ "tvdb_season": 0, "episode_start": 2, "episode_end": 2 }]
        }),
        json!({
            "mal_id": 50003,
            "anilist_id": null,
            "anidb_id": null,
            "kitsu_id": null,
            "thetvdb_id": 880003,
            "themoviedb_id": 770003,
            "alt_tvdb_id": 880003,
            "thetvdb_season": 0,
            "score": null,
            "anime_media_type": "MOVIE",
            "global_media_type": "movie",
            "status": "finished",
            "mapping_type": "special",
            "episode_mappings": [{ "tvdb_season": 0, "episode_start": 3, "episode_end": 3 }]
        }),
    ];

    json!({
        "data": {
            "s0": {
                "series": {
                    "tvdb_id": HAIKYU_TVDB_ID,
                    "name": "Haikyu!!",
                    "sort_name": "Haikyu!!",
                    "slug": "haikyu",
                    "status": "Ended",
                    "year": 2014,
                    "first_aired": "2014-04-06",
                    "overview": "A volleyball anime fixture for the stack probe.",
                    "network": "MBS",
                    "runtime_minutes": 24,
                    "poster_url": "https://example.invalid/haikyu-poster.jpg",
                    "country": "jpn",
                    "genres": ["Animation", "Sports"],
                    "aliases": ["Haikyuu"],
                    "tagged_aliases": [],
                    "artworks": [],
                    "seasons": seasons,
                    "episodes": episodes,
                    "anime_mappings": anime_mappings,
                    "anime_movies": anime_movies
                }
            }
        }
    })
    .to_string()
}

fn create_haikyu_library_root(root: &Path) -> PathBuf {
    let show_dir = root.join("Haikyu!! [BD]");
    std::fs::create_dir_all(&show_dir).expect("create Haikyu show dir");
    std::fs::write(
        show_dir.join("tvshow.nfo"),
        format!("<tvshow><title>Haikyu!!</title><tvdbid>{HAIKYU_TVDB_ID}</tvdbid></tvshow>"),
    )
    .expect("write Haikyu tvshow.nfo");

    // Keep the metadata shape large (99 episodes plus interstitial movies),
    // but keep the on-disk fixture smaller so the subprocess probe spends its
    // time in the complex title-scan logic instead of file analysis.
    let season_sample_episodes = [
        (0_i32, vec![1_i32, 2, 3]),
        (1, vec![1, 12, 25]),
        (2, vec![1, 12, 25]),
        (3, vec![1, 5, 10]),
        (4, vec![1, 12, 25]),
        (5, vec![1, 5, 10]),
    ];
    for (season_number, episodes) in season_sample_episodes {
        let season_dir = show_dir.join(format!("Season {season_number:02}"));
        std::fs::create_dir_all(&season_dir).expect("create Haikyu season dir");
        for episode_number in episodes {
            let file_name = format!(
                "Haikyu!!.S{season_number:02}E{episode_number:02}.1080p.BluRay.x264-GRP.mkv"
            );
            std::fs::write(season_dir.join(file_name), b"not-a-real-video")
                .expect("write Haikyu episode file");
        }
    }

    show_dir
}

async fn wait_for_haikyu_scan_to_settle(
    ctx: &TestContext,
    minimum_media_files: Option<usize>,
) -> DateTime<Utc> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last_log = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    loop {
        let titles = ctx
            .db
            .list(Some(MediaFacet::Anime), None)
            .await
            .expect("list anime titles while waiting for stack probe scan");
        let title = titles
            .iter()
            .find(|title| title.name == "Haikyu!!")
            .cloned();
        let sessions = ctx.app.services.library_scan_tracker.list_active().await;

        if last_log.elapsed() >= Duration::from_secs(1) {
            if let Some(title) = title.as_ref() {
                let media_files = ctx
                    .db
                    .list_media_files_for_title(&title.id)
                    .await
                    .expect("list media files while logging stack probe wait state");
                eprintln!(
                    "stack probe: wait state metadata_fetched_at={:?} active_sessions={} media_files={}",
                    title.metadata_fetched_at,
                    sessions.len(),
                    media_files.len(),
                );
                if let Some(session) = sessions.first() {
                    eprintln!(
                        "stack probe: session status={:?} found_titles={} metadata={}/{} file={}/{} failed_files={}",
                        session.status,
                        session.found_titles,
                        session.metadata_progress.completed,
                        session.metadata_progress.total,
                        session.file_progress.completed,
                        session.file_progress.total,
                        session.file_progress.failed,
                    );
                }
            } else {
                eprintln!(
                    "stack probe: wait state title_not_found active_sessions={}",
                    sessions.len()
                );
            }
            last_log = Instant::now();
        }

        if let Some(title) = title
            && let Some(metadata_fetched_at) = title.metadata_fetched_at.clone()
        {
            let collections = ctx
                .db
                .list_collections_for_title(&title.id)
                .await
                .expect("list collections while waiting for stack probe scan");
            let media_files = ctx
                .db
                .list_media_files_for_title(&title.id)
                .await
                .expect("list media files while waiting for stack probe scan");
            let enough_files = minimum_media_files.is_none_or(|value| media_files.len() >= value);
            let has_interstitial = collections.iter().any(|collection| {
                collection.collection_type == scryer_domain::CollectionType::Interstitial
            });

            if sessions.is_empty() && enough_files && has_interstitial {
                return metadata_fetched_at;
            }
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for Haikyu!! scan to settle"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_library_scan_sessions_to_clear(ctx: &TestContext) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if ctx
            .app
            .services
            .library_scan_tracker
            .list_active()
            .await
            .is_empty()
        {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for manual library scan sessions to clear"
        );
        sleep(Duration::from_millis(50)).await;
    }
}
