mod common;

use std::path::PathBuf;

use common::TestContext;
use scryer_application::{ActivityKind, ActivitySeverity, PostProcessingContext, run_post_processing};
use scryer_domain::MediaFacet;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Seed a post-processing script setting for the given facet.
async fn set_script(ctx: &TestContext, facet: MediaFacet, command: &str) {
    let key = match facet {
        MediaFacet::Movie => "post_processing.script.movie",
        MediaFacet::Tv => "post_processing.script.series",
        MediaFacet::Anime => "post_processing.script.anime",
        MediaFacet::Other => return,
    };
    ctx.db
        .ensure_setting_definition("post_processing", "system", key, "string", "\"\"", false, None)
        .await
        .expect("ensure setting definition");
    ctx.db
        .upsert_setting_value("system", key, None, format!("\"{}\"", command), "test", None)
        .await
        .expect("upsert setting value");
}

/// Seed the timeout setting.
async fn set_timeout(ctx: &TestContext, secs: u64) {
    let key = "post_processing.timeout_secs";
    ctx.db
        .ensure_setting_definition("post_processing", "system", key, "number", "1800", false, None)
        .await
        .expect("ensure setting definition");
    ctx.db
        .upsert_setting_value("system", key, None, secs.to_string(), "test", None)
        .await
        .expect("upsert setting value");
}

/// Build a PostProcessingContext for a movie import.
fn movie_context(app: &scryer_application::AppUseCase, dest: &std::path::Path) -> PostProcessingContext {
    PostProcessingContext {
        app: app.clone(),
        actor_id: None,
        title_id: "title-pp-test".to_string(),
        title_name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        dest_path: dest.to_path_buf(),
        year: Some(2024),
        imdb_id: Some("tt1234567".to_string()),
        tvdb_id: None,
        season: None,
        episode: None,
        quality: Some("1080p".to_string()),
    }
}

/// Retrieve the most recent activity events and find one matching PostProcessingCompleted.
async fn last_post_processing_event(
    app: &scryer_application::AppUseCase,
) -> Option<scryer_application::ActivityEvent> {
    let events = app.services.activity_stream.list(10, 0).await;
    events
        .into_iter()
        .find(|e| e.kind == ActivityKind::PostProcessingCompleted)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// When no script is configured (empty string), post-processing is a no-op
/// and produces no activity event.
#[tokio::test]
async fn skips_when_no_script_configured() {
    let ctx = TestContext::new().await;
    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    assert!(
        last_post_processing_event(&ctx.app).await.is_none(),
        "no activity event expected when script is empty"
    );
}

/// A script that exits 0 produces a Success activity event.
#[tokio::test]
async fn successful_script_records_success_event() {
    let ctx = TestContext::new().await;
    set_script(&ctx, MediaFacet::Movie, "true").await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("succeeded"));
    assert!(event.message.contains("Test Movie"));
    assert_eq!(event.title_id.as_deref(), Some("title-pp-test"));
}

/// A script that exits non-zero produces a Warning activity event
/// with the exit code and stderr tail.
#[tokio::test]
async fn failed_script_records_warning_with_stderr() {
    let ctx = TestContext::new().await;
    set_script(&ctx, MediaFacet::Movie, "echo 'oh no' >&2; exit 42").await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
    assert!(event.message.contains("exit 42"), "message: {}", event.message);
    assert!(event.message.contains("oh no"), "stderr should appear: {}", event.message);
}

/// A script that exceeds the timeout is killed and produces a timeout warning.
#[tokio::test]
async fn timeout_kills_script_and_records_warning() {
    let ctx = TestContext::new().await;
    set_script(&ctx, MediaFacet::Movie, "sleep 60").await;
    set_timeout(&ctx, 1).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
    assert!(event.message.contains("timed out"), "message: {}", event.message);
    assert!(event.message.contains("1s"), "should mention timeout duration: {}", event.message);
}

/// The script receives the correct environment variables.
#[tokio::test]
async fn script_receives_environment_variables() {
    let ctx = TestContext::new().await;

    // Script writes all SCRYER_ env vars to a file so we can inspect them.
    let output_dir = tempfile::tempdir().expect("tempdir");
    let env_dump = output_dir.path().join("env_dump.txt");
    let script = format!("env | grep ^SCRYER_ | sort > '{}'", env_dump.display());
    set_script(&ctx, MediaFacet::Movie, &script).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor_id: None,
        title_id: "title-env-test".to_string(),
        title_name: "Env Test Movie".to_string(),
        facet: MediaFacet::Movie,
        dest_path: dest_file.clone(),
        year: Some(2024),
        imdb_id: Some("tt9999999".to_string()),
        tvdb_id: Some("12345".to_string()),
        season: None,
        episode: None,
        quality: Some("720p".to_string()),
    };
    run_post_processing(pp_ctx).await.expect("run");

    let content = std::fs::read_to_string(&env_dump).expect("read env dump");
    assert!(content.contains("SCRYER_EVENT=post_import"), "content:\n{content}");
    assert!(content.contains("SCRYER_FACET=movie"), "content:\n{content}");
    assert!(content.contains(&format!("SCRYER_FILE_PATH={}", dest_file.display())), "content:\n{content}");
    assert!(content.contains("SCRYER_TITLE_NAME=Env Test Movie"), "content:\n{content}");
    assert!(content.contains("SCRYER_TITLE_ID=title-env-test"), "content:\n{content}");
    assert!(content.contains("SCRYER_YEAR=2024"), "content:\n{content}");
    assert!(content.contains("SCRYER_IMDB_ID=tt9999999"), "content:\n{content}");
    assert!(content.contains("SCRYER_TVDB_ID=12345"), "content:\n{content}");
    assert!(content.contains("SCRYER_QUALITY=720p"), "content:\n{content}");
}

/// The script's working directory is set to the parent of the imported file.
#[tokio::test]
async fn script_working_directory_is_file_parent() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let cwd_dump = output_dir.path().join("cwd.txt");
    let script = format!("pwd > '{}'", cwd_dump.display());
    set_script(&ctx, MediaFacet::Movie, &script).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let cwd = std::fs::read_to_string(&cwd_dump)
        .expect("read cwd dump")
        .trim()
        .to_string();

    // On macOS, /tmp is a symlink to /private/tmp, so canonicalize both.
    let expected = dest_dir.path().canonicalize().expect("canonicalize dest");
    let actual = PathBuf::from(&cwd).canonicalize().expect("canonicalize cwd");
    assert_eq!(actual, expected);
}

/// Series facet uses the series script key.
#[tokio::test]
async fn series_facet_uses_series_script() {
    let ctx = TestContext::new().await;
    // Only set series script; movie script left empty.
    set_script(&ctx, MediaFacet::Tv, "true").await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Show.S01E01.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor_id: None,
        title_id: "title-series-pp".to_string(),
        title_name: "Test Show".to_string(),
        facet: MediaFacet::Tv,
        dest_path: dest_file,
        year: None,
        imdb_id: None,
        tvdb_id: Some("54321".to_string()),
        season: Some(1),
        episode: Some(1),
        quality: Some("1080p".to_string()),
    };
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("Test Show"));
}

/// Anime facet uses the anime script key.
#[tokio::test]
async fn anime_facet_uses_anime_script() {
    let ctx = TestContext::new().await;
    set_script(&ctx, MediaFacet::Anime, "true").await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Anime.S01E01.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor_id: None,
        title_id: "title-anime-pp".to_string(),
        title_name: "Test Anime".to_string(),
        facet: MediaFacet::Anime,
        dest_path: dest_file,
        year: None,
        imdb_id: None,
        tvdb_id: None,
        season: Some(1),
        episode: Some(5),
        quality: None,
    };
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("Test Anime"));
}

/// Other facet skips immediately — no script lookup, no activity event.
#[tokio::test]
async fn other_facet_is_noop() {
    let ctx = TestContext::new().await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("file.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor_id: None,
        title_id: "title-other".to_string(),
        title_name: "Other".to_string(),
        facet: MediaFacet::Other,
        dest_path: dest_file,
        year: None,
        imdb_id: None,
        tvdb_id: None,
        season: None,
        episode: None,
        quality: None,
    };
    run_post_processing(pp_ctx).await.expect("run");

    assert!(last_post_processing_event(&ctx.app).await.is_none());
}

/// A script that references an invalid binary records a spawn failure warning.
#[tokio::test]
async fn invalid_command_records_spawn_failure() {
    let ctx = TestContext::new().await;
    // /nonexistent/binary will fail to execute inside sh -c, giving exit 127
    set_script(&ctx, MediaFacet::Movie, "/nonexistent/binary_that_does_not_exist_12345").await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
    // sh -c will report a non-zero exit (127) for command not found
    assert!(
        event.message.contains("failed") || event.message.contains("exit"),
        "message: {}",
        event.message
    );
}

/// A script that produces output on both stdout and stderr completes normally,
/// with stderr captured in the failure message (stdout is discarded).
#[tokio::test]
async fn stdout_is_drained_stderr_captured_on_failure() {
    let ctx = TestContext::new().await;
    set_script(
        &ctx,
        MediaFacet::Movie,
        "echo 'stdout line'; echo 'stderr detail' >&2; exit 1",
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
    assert!(event.message.contains("stderr detail"), "message: {}", event.message);
    // stdout should NOT appear in the message
    assert!(!event.message.contains("stdout line"), "stdout should not leak: {}", event.message);
}

/// Season and episode env vars are populated for series imports.
#[tokio::test]
async fn series_env_includes_season_and_episode() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let env_dump = output_dir.path().join("env_dump.txt");
    let script = format!("env | grep ^SCRYER_ | sort > '{}'", env_dump.display());
    set_script(&ctx, MediaFacet::Tv, &script).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Show.S03E07.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = PostProcessingContext {
        app: ctx.app.clone(),
        actor_id: None,
        title_id: "title-se-env".to_string(),
        title_name: "Season Episode Show".to_string(),
        facet: MediaFacet::Tv,
        dest_path: dest_file,
        year: None,
        imdb_id: None,
        tvdb_id: Some("99999".to_string()),
        season: Some(3),
        episode: Some(7),
        quality: Some("2160p".to_string()),
    };
    run_post_processing(pp_ctx).await.expect("run");

    let content = std::fs::read_to_string(&env_dump).expect("read env dump");
    assert!(content.contains("SCRYER_FACET=series"), "content:\n{content}");
    assert!(content.contains("SCRYER_SEASON=3"), "content:\n{content}");
    assert!(content.contains("SCRYER_EPISODE=7"), "content:\n{content}");
    assert!(content.contains("SCRYER_TVDB_ID=99999"), "content:\n{content}");
    assert!(content.contains("SCRYER_QUALITY=2160p"), "content:\n{content}");
}
