#![recursion_limit = "256"]

mod common;

use std::path::PathBuf;

use common::TestContext;
use scryer_application::{
    ActivityKind, ActivitySeverity, PostProcessingContext, run_post_processing,
};
use scryer_domain::{MediaFacet, PostProcessingScript};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a post-processing script in the DB for the given facet.
async fn create_script(
    ctx: &TestContext,
    facet: MediaFacet,
    command: &str,
    timeout_secs: i64,
    debug: bool,
) {
    let facet_str = match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Tv => "tv",
        MediaFacet::Anime => "anime",
        MediaFacet::Other => return,
    };
    let script = PostProcessingScript {
        id: format!(
            "pp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ),
        name: format!("Test script for {facet_str}"),
        description: String::new(),
        script_type: "inline".to_string(),
        script_content: command.to_string(),
        applied_facets: vec![facet_str.to_string()],
        execution_mode: "blocking".to_string(),
        timeout_secs,
        priority: 0,
        enabled: true,
        debug,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    ctx.app
        .services
        .pp_scripts
        .create_script(script)
        .await
        .expect("create script");
}

/// Build a PostProcessingContext for a movie import.
fn movie_context(
    app: &scryer_application::AppUseCase,
    dest: &std::path::Path,
) -> PostProcessingContext {
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

/// When no scripts are configured, post-processing is a no-op.
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
        "no activity event expected when no scripts configured"
    );
}

/// A script that exits 0 produces a Success activity event.
#[tokio::test]
async fn successful_script_records_success_event() {
    let ctx = TestContext::new().await;
    create_script(&ctx, MediaFacet::Movie, "true", 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Success);
    assert!(event.message.contains("Test Movie"));
}

/// A script that exits non-zero produces a Warning activity event.
#[tokio::test]
async fn failed_script_records_warning_with_stderr() {
    let ctx = TestContext::new().await;
    create_script(
        &ctx,
        MediaFacet::Movie,
        "echo 'oh no' >&2; exit 42",
        300,
        true,
    )
    .await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
}

/// A script that exceeds the timeout is killed and produces a timeout warning.
#[tokio::test]
async fn timeout_kills_script_and_records_warning() {
    let ctx = TestContext::new().await;
    create_script(&ctx, MediaFacet::Movie, "sleep 60", 1, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let event = last_post_processing_event(&ctx.app)
        .await
        .expect("should have activity event");
    assert_eq!(event.severity, ActivitySeverity::Warning);
}

/// The script receives SCRYER_METADATA and legacy environment variables.
#[tokio::test]
async fn script_receives_environment_variables() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let env_dump = output_dir.path().join("env_dump.txt");
    let script = format!("env | grep ^SCRYER_ | sort > '{}'", env_dump.display());
    create_script(&ctx, MediaFacet::Movie, &script, 300, false).await;

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
    assert!(
        content.contains("SCRYER_EVENT=post_import"),
        "content:\n{content}"
    );
    assert!(
        content.contains("SCRYER_FACET=movie"),
        "content:\n{content}"
    );
    assert!(
        content.contains(&format!("SCRYER_FILE_PATH={}", dest_file.display())),
        "content:\n{content}"
    );
    assert!(
        content.contains("SCRYER_TITLE_NAME=Env Test Movie"),
        "content:\n{content}"
    );
    assert!(
        content.contains("SCRYER_METADATA="),
        "should have JSON metadata: {content}"
    );
}

/// The script's working directory is set to the parent of the imported file.
#[tokio::test]
async fn script_working_directory_is_file_parent() {
    let ctx = TestContext::new().await;

    let output_dir = tempfile::tempdir().expect("tempdir");
    let cwd_dump = output_dir.path().join("cwd.txt");
    let script = format!("pwd > '{}'", cwd_dump.display());
    create_script(&ctx, MediaFacet::Movie, &script, 300, false).await;

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let dest_file = dest_dir.path().join("Movie.2024.1080p.mkv");
    std::fs::write(&dest_file, b"fake").expect("write");

    let pp_ctx = movie_context(&ctx.app, &dest_file);
    run_post_processing(pp_ctx).await.expect("run");

    let cwd = std::fs::read_to_string(&cwd_dump)
        .expect("read cwd dump")
        .trim()
        .to_string();

    let expected = dest_dir.path().canonicalize().expect("canonicalize dest");
    let actual = PathBuf::from(&cwd)
        .canonicalize()
        .expect("canonicalize cwd");
    assert_eq!(actual, expected);
}

/// Series facet uses series-targeted scripts.
#[tokio::test]
async fn series_facet_uses_series_script() {
    let ctx = TestContext::new().await;
    create_script(&ctx, MediaFacet::Tv, "true", 300, false).await;

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

/// Anime facet uses anime-targeted scripts.
#[tokio::test]
async fn anime_facet_uses_anime_script() {
    let ctx = TestContext::new().await;
    create_script(&ctx, MediaFacet::Anime, "true", 300, false).await;

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

/// A script that references an invalid binary records a failure.
#[tokio::test]
async fn invalid_command_records_spawn_failure() {
    let ctx = TestContext::new().await;
    create_script(
        &ctx,
        MediaFacet::Movie,
        "/nonexistent/binary_that_does_not_exist_12345",
        300,
        false,
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
}
