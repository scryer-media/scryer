use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use scryer_infrastructure_datastore::migrations::MigrationProgress;
use tokio::sync::watch;
use tower::ServiceExt;
use tower_http::compression::CompressionLayer;

use crate::base_path::{BasePath, mount_router};
use crate::middleware::{CorsConfig, cors_handler};

#[derive(Clone)]
pub(crate) enum BootstrapStatus {
    Migrating,
    Ready(Router),
    Failed(String),
}

#[derive(Clone)]
pub(crate) struct SplashState {
    pub(crate) status_rx: watch::Receiver<BootstrapStatus>,
    pub(crate) migration_progress: MigrationProgress,
    pub(crate) restore: std::sync::Arc<scryer_application::RestoreCoordinator>,
}

/// The `migrations` object the health check adds while bootstrapping, once the
/// migration run knows how many migrations it will apply.
fn migration_progress_json(progress: &MigrationProgress) -> serde_json::Value {
    match progress.snapshot() {
        Some((completed, total)) => serde_json::json!({"completed": completed, "total": total}),
        None => serde_json::Value::Null,
    }
}

/// `GET /health` — the health check for every orchestrator probe (liveness,
/// readiness, startup, reverse-proxy upstream checks).
///
/// Reports HTTP 200 for the whole time the process is alive and making
/// progress — including while migrations run — so Docker/Compose healthchecks,
/// Kubernetes probes, autoheal, etc. never recycle the container in the middle
/// of a long migration. User traffic does not need to be held back during that
/// window either: the splash fallback serves the "Upgrading database…" page on
/// every route and it reloads into the app the moment bootstrap finishes, so a
/// readiness probe here is correct too. The JSON body carries the bootstrap
/// phase (`"migrating"` / `"ok"`) plus a `ready` flag — and, while migrating,
/// `migrations: {completed, total}` once the migration run has counted what it
/// will apply (`null` before that or when nothing is pending) — and every in-tree poller
/// (splash page, web client restart overlay, xtask, seed) keys on
/// `status == "ok"`, not on the HTTP status. Only a failed bootstrap returns a
/// non-2xx code, because a process that will never become ready *should* be
/// recycled.
///
/// Automation that needs the *API* up (not just the process) uses
/// `GET /health/ready` (see [`splash_ready_handler`]).
pub(crate) async fn splash_health_handler(State(state): State<SplashState>) -> Response {
    if state.restore.blocks_application() {
        let job = state.restore.snapshot();
        return Json(serde_json::json!({
            "status": "restoring", "ready": false,
            "importPercent": job.as_ref().and_then(|job| job.import_percent),
            "phase": job.as_ref().map(|job| job.phase),
            "canRetry": matches!(*state.status_rx.borrow(), BootstrapStatus::Ready(_))
                && state.restore.can_retry(),
            "recoveryRequired": job.as_ref().is_some_and(|job| matches!(job.phase,
                scryer_application::RestoreJobPhase::Failed | scryer_application::RestoreJobPhase::Interrupted)),
        })).into_response();
    }
    let status = state.status_rx.borrow().clone();
    match status {
        BootstrapStatus::Migrating => Json(serde_json::json!({
            "status": "migrating",
            "ready": false,
            "migrations": migration_progress_json(&state.migration_progress),
        }))
        .into_response(),
        BootstrapStatus::Ready(_) => {
            Json(serde_json::json!({"status": "ok", "ready": true})).into_response()
        }
        BootstrapStatus::Failed(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "ready": false, "message": message})),
        )
            .into_response(),
    }
}

/// `GET /health/ready` — "is the application (GraphQL/UI) serving yet?"
///
/// Returns 503 until the full application router is serving, 200 afterwards,
/// and 500 if bootstrap failed. This is for automation that must not call the
/// API before bootstrap completes (CI, seed/provisioning scripts, e2e, boot-time
/// sidecars) and wants the HTTP status code rather than the JSON body to say
/// so. It is *not* the orchestrator health check — a restart-on-unhealthy or
/// `startupProbe` pointed here would kill a long migration; those belong on
/// `/health`.
pub(crate) async fn splash_ready_handler(State(state): State<SplashState>) -> Response {
    if state.restore.blocks_application() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "restoring", "ready": false})),
        )
            .into_response();
    }
    let status = state.status_rx.borrow().clone();
    match status {
        BootstrapStatus::Migrating => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "migrating", "ready": false})),
        )
            .into_response(),
        BootstrapStatus::Ready(_) => {
            Json(serde_json::json!({"status": "ok", "ready": true})).into_response()
        }
        BootstrapStatus::Failed(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "ready": false, "message": message})),
        )
            .into_response(),
    }
}

/// Where the splash page loads the turning Scryer mark from. The splash router
/// answers every other path with the splash page itself, so the artwork needs a
/// route of its own, and the web UI's assets are not being served yet.
const SPLASH_LOADING_MARK_PATH: &str = "/splash/scryer-loading.webp";
const SPLASH_LOADING_MARK_STILL_PATH: &str = "/splash/scryer-loading-still.webp";
const SPLASH_WORDMARK_PATH: &str = "/splash/scryer-wordmark.svg";

/// The full-size animated mark and its first frame, for reduced motion.
static SPLASH_LOADING_MARK: &[u8] = include_bytes!("../resources/splash/scryer-loading.webp");
static SPLASH_LOADING_MARK_STILL: &[u8] =
    include_bytes!("../resources/splash/scryer-loading-still.webp");

/// The official Scryer wordmark, the same artwork the login page shows.
static SPLASH_WORDMARK: &[u8] = include_bytes!("../resources/splash/scryer-wordmark.svg");

fn image_response(content_type: &'static str, bytes: &'static [u8]) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    )
        .into_response()
}

async fn splash_loading_mark_handler() -> Response {
    image_response("image/webp", SPLASH_LOADING_MARK)
}

async fn splash_loading_mark_still_handler() -> Response {
    image_response("image/webp", SPLASH_LOADING_MARK_STILL)
}

async fn splash_wordmark_handler() -> Response {
    image_response("image/svg+xml", SPLASH_WORDMARK)
}

pub(crate) async fn splash_fallback_handler(
    State(state): State<SplashState>,
    request: axum::extract::Request,
) -> Response {
    let status = state.status_rx.borrow().clone();
    let control_route = (request.uri().path() == "/graphql"
        && request.method() == axum::http::Method::POST)
        || request.uri().path() == "/authless-client";
    let _admission =
        if !control_route && matches!(status, BootstrapStatus::Ready(_)) {
            match state.restore.admit_request().await {
                Ok(guard) => Some(guard),
                Err(_) => {
                    if request.method() == axum::http::Method::GET
                        && request
                            .headers()
                            .get(header::ACCEPT)
                            .and_then(|value| value.to_str().ok())
                            .is_some_and(|value| value.contains("text/html"))
                    {
                        return Html(maintenance_html(None, true)).into_response();
                    }
                    return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                    "reason": scryer_application::restore_refusal::RESTORE_MAINTENANCE_ACTIVE,
                    "ready": false
                }))).into_response();
                }
            }
        } else {
            None
        };
    if state.restore.blocks_application() && !matches!(status, BootstrapStatus::Ready(_)) {
        if request.method() == axum::http::Method::GET
            && request
                .headers()
                .get(header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("text/html"))
        {
            return Html(maintenance_html(None, true)).into_response();
        }
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "reason": scryer_application::restore_refusal::RESTORE_MAINTENANCE_ACTIVE,
                "ready": false
            })),
        )
            .into_response();
    }
    match status {
        BootstrapStatus::Ready(router) => router
            .oneshot(request)
            .await
            .unwrap_or_else(|err| match err {}),
        BootstrapStatus::Migrating => {
            Html(splash_html(state.migration_progress.snapshot())).into_response()
        }
        BootstrapStatus::Failed(message) => Html(error_html(&message)).into_response(),
    }
}

pub(crate) fn build_splash_router(
    state: SplashState,
    cors: CorsConfig,
    base_path: BasePath,
) -> Router {
    let cors_for_layer = cors.clone();

    let router = Router::new()
        .route(
            "/health",
            get(splash_health_handler).with_state(state.clone()),
        )
        .route(
            "/health/ready",
            get(splash_ready_handler).with_state(state.clone()),
        )
        .route(SPLASH_LOADING_MARK_PATH, get(splash_loading_mark_handler))
        .route(
            SPLASH_LOADING_MARK_STILL_PATH,
            get(splash_loading_mark_still_handler),
        )
        .route(SPLASH_WORDMARK_PATH, get(splash_wordmark_handler))
        .fallback(splash_fallback_handler)
        .with_state(state)
        .layer(CompressionLayer::new().zstd(true).br(true).gzip(true))
        .layer(axum::middleware::from_fn(move |request, next| {
            cors_handler(request, next, cors_for_layer.clone())
        }));

    mount_router(router, &base_path)
}

/// The page shown on every route while bootstrapping. It says "Upgrading
/// database…" with a bar and a count while migrations are being applied, and
/// "Starting up…" otherwise, then keeps both current from the health check.
fn splash_html(migration_progress: Option<(usize, usize)>) -> String {
    maintenance_html(migration_progress, false)
}

fn maintenance_html(migration_progress: Option<(usize, usize)>, restoring: bool) -> String {
    let base_path = BasePath::from_env();
    let health_url = base_path.join("/health");
    let loading_mark_url = base_path.join(SPLASH_LOADING_MARK_PATH);
    let loading_mark_still_url = base_path.join(SPLASH_LOADING_MARK_STILL_PATH);
    let wordmark_url = base_path.join(SPLASH_WORDMARK_PATH);
    let graphql_url = base_path.join("/graphql");
    let proof_url = base_path.join("/authless-client");
    let (status_text, progress_hidden, completed, total) = match migration_progress {
        _ if restoring => ("Restoring backup&hellip;", " hidden", 0, 0),
        Some((completed, total)) if completed < total => {
            ("Upgrading database&hellip;", "", completed, total)
        }
        _ => ("Starting up&hellip;", " hidden", 0, 0),
    };
    let percent = if total == 0 {
        0.0
    } else {
        completed as f64 * 100.0 / total as f64
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width,initial-scale=1"/>
<title>scryer — starting</title>
<style>{SPLASH_STYLE}</style>
</head>
<body>
<main>
  <picture class="loading-mark">
    <source media="(prefers-reduced-motion: reduce)" srcset="{loading_mark_still_url}"/>
    <img src="{loading_mark_url}" width="531" height="522" alt=""/>
  </picture>
  <h1><img class="wordmark" src="{wordmark_url}" width="1200" height="400" alt="Scryer"/></h1>
  <div class="status" role="status">{status_text}</div>
  <div class="progress"{progress_hidden}>
    <div class="progress-track" role="progressbar" aria-label="Database upgrade" aria-valuemin="0" aria-valuemax="{total}" aria-valuenow="{completed}">
      <div class="progress-fill" style="width: {percent:.1}%"></div>
    </div>
    <div class="progress-count">{completed} of {total} migrations applied</div>
  </div>
  <form id="restore-retry" hidden>
    <p class="detail">Restore stopped safely. Enter the backup password to try again.</p>
    <label>Backup password <input id="restore-password" type="password" autocomplete="off"/></label>
    <label>Setup code (if your first-run restore requires one) <input id="restore-setup-code" autocomplete="one-time-code" maxlength="32"/></label>
    <button type="submit">Retry restore</button>
    <p id="retry-message" class="detail" role="status"></p>
  </form>
</main>
<script>
(function() {{
  var status = document.querySelector(".status");
  var progress = document.querySelector(".progress");
  var track = document.querySelector(".progress-track");
  var fill = document.querySelector(".progress-fill");
  var count = document.querySelector(".progress-count");
  var retry = document.getElementById("restore-retry");
  var retrying = false;
  retry.addEventListener("submit", async function(event) {{
    event.preventDefault();
    if (retrying) return;
    retrying = true;
    var button = retry.querySelector("button");
    var message = document.getElementById("retry-message");
    button.disabled = true;
    message.textContent = "Starting restore…";
    try {{
      var headers = {{"Content-Type": "application/json"}};
      var setupCode = document.getElementById("restore-setup-code").value.trim() || null;
      var token = sessionStorage.getItem("scryer_auth_token") || localStorage.getItem("scryer_auth_token_persistent");
      if (token) headers.Authorization = "Bearer " + token;
      else if (!setupCode) {{
        var proofResponse = await fetch("{proof_url}", {{credentials: "include", signal: AbortSignal.timeout(10000)}});
        // First-run setup has no web-client proof; the server decides from here.
        if (proofResponse.ok) {{
          var proof = await proofResponse.json();
          headers["x-scryer-web-client"] = proof.proof;
        }}
      }}
      async function request(query, variables) {{
        var response = await fetch("{graphql_url}", {{method: "POST", credentials: "include", headers: headers, signal: AbortSignal.timeout(10000),
          body: JSON.stringify({{query: query, variables: variables}})}});
        var result = await response.json();
        if (!response.ok || result.errors) throw new Error("The restore could not be started. Check authentication and retry.");
        return result.data;
      }}
      var current = await request("query($setupCode: String) {{ restoreStatus(setupCode: $setupCode) {{ uploadId }} }}", {{setupCode: setupCode}});
      if (!current.restoreStatus) throw new Error("The restore attempt is no longer available.");
      var password = document.getElementById("restore-password");
      var value = password.value;
      password.value = "";
      await request("mutation($id: ID!, $password: String, $setupCode: String) {{ startApplyRestoreBundle(uploadId: $id, password: $password, setupCode: $setupCode) {{ id }} }}",
        {{id: current.restoreStatus.uploadId, password: value || null, setupCode: setupCode}});
      retry.hidden = true;
      message.textContent = "";
    }} catch (error) {{
      message.textContent = error.message;
    }} finally {{
      retrying = false;
      button.disabled = false;
    }}
  }});
  function showProgress(m) {{
    var upgrading = !!m && m.completed < m.total;
    status.textContent = upgrading ? "Upgrading database…" : "Starting up…";
    progress.hidden = !upgrading;
    if (!upgrading) return;
    fill.style.width = (m.completed * 100 / m.total) + "%";
    track.setAttribute("aria-valuemax", m.total);
    track.setAttribute("aria-valuenow", m.completed);
    count.textContent = m.completed + " of " + m.total + " migrations applied";
  }}
  function poll() {{
    fetch("{health_url}")
      .then(function(r) {{ return r.json(); }})
      .then(function(d) {{
        if (d.status === "ok") {{ location.reload(); return; }}
        if (d.status === "restoring") {{
          retry.hidden = !d.canRetry;
          var percent = d.importPercent;
          var importing = typeof percent === "number" && percent < 100;
          progress.hidden = !importing || d.recoveryRequired || d.phase === "cancelling";
          status.textContent = d.recoveryRequired ? "Restore needs recovery" :
            d.phase === "cancelling" ? "Cancelling safely… Keep Scryer running." :
            importing ? "Importing database… " + percent + "%" :
            percent === 100 ? "Finalizing…" : "Restoring backup… Keep Scryer running.";
          if (importing && !d.recoveryRequired) {{
            fill.style.width = percent + "%";
            track.setAttribute("aria-label", "Database import");
            track.setAttribute("aria-valuemax", "100");
            track.setAttribute("aria-valuenow", percent);
            count.textContent = "";
          }}
          setTimeout(poll, 1000);
          return;
        }}
        if (d.status === "error") {{
          document.querySelector(".loading-mark").style.display = "none";
          progress.hidden = true;
          status.textContent = "Startup failed";
          status.classList.add("error");
          var p = document.createElement("p");
          p.className = "detail";
          p.textContent = d.message || "Unknown error";
          document.querySelector("main").appendChild(p);
          return;
        }}
        showProgress(d.migrations);
        setTimeout(poll, 500);
      }})
      .catch(function() {{ setTimeout(poll, 1000); }});
  }}
  setTimeout(poll, 200);
}})();
</script>
</body>
</html>"#
    )
}

fn error_html(message: &str) -> String {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let wordmark_url = BasePath::from_env().join(SPLASH_WORDMARK_PATH);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width,initial-scale=1"/>
<title>scryer — error</title>
<style>{SPLASH_STYLE}</style>
</head>
<body>
<main>
  <h1><img class="wordmark" src="{wordmark_url}" width="1200" height="400" alt="Scryer"/></h1>
  <div class="status error">Startup failed</div>
  <p class="detail">{escaped}</p>
</main>
</body>
</html>"#
    )
}

const SPLASH_STYLE: &str = r#"
:root { color-scheme: dark; }
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  min-height: 100vh;
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, sans-serif;
  /* The web UI's dark shell background. */
  background: linear-gradient(180deg, #070d1d 0%, #040814 42%, #02050c 100%);
  background-attachment: fixed;
  color: #dbe5ff;
  display: grid;
  place-items: center;
}
main {
  text-align: center;
  padding: 2rem;
}
.wordmark {
  display: block;
  width: 224px;
  max-width: 70vw;
  height: auto;
  margin: 0 auto 1.5rem;
  filter: drop-shadow(0 6px 14px rgba(2, 6, 23, 0.7));
}
.status {
  font-size: 0.95rem;
  color: #8b96b9;
  margin-bottom: 0.5rem;
}
.progress {
  width: 224px;
  max-width: 70vw;
  margin: 1rem auto 0;
}
.progress[hidden] {
  display: none;
}
.progress-track {
  height: 6px;
  border-radius: 999px;
  overflow: hidden;
  background: rgba(255, 255, 255, 0.08);
  box-shadow: inset 0 1px 2px rgba(2, 6, 23, 0.6);
}
.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, #5b64ff, #38bdf8);
  box-shadow: 0 0 12px rgba(91, 100, 255, 0.55);
  transition: width 0.4s ease;
}
.progress-count {
  font-size: 0.8rem;
  color: #8b96b9;
  margin-top: 0.6rem;
  font-variant-numeric: tabular-nums;
}
.status.error {
  color: #ef4444;
  font-weight: 600;
}
.detail {
  font-size: 0.85rem;
  color: #8b96b9;
  max-width: 36rem;
  margin: 1rem auto 0;
  word-break: break-word;
}
.loading-mark img {
  display: block;
  width: 160px;
  max-width: 50vw;
  height: auto;
  margin: 0 auto 0.75rem;
  filter: drop-shadow(0 18px 30px rgba(2, 6, 23, 0.75)) drop-shadow(0 0 36px rgba(91, 100, 255, 0.28));
}
"#;

#[cfg(test)]
mod tests {
    use super::{BootstrapStatus, SplashState, build_splash_router, splash_html};
    use crate::base_path::BasePath;
    use crate::middleware::CorsConfig;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::routing::get;
    use scryer_infrastructure_datastore::migrations::MigrationProgress;
    use tokio::sync::watch;
    use tower::ServiceExt;

    fn ready_splash_router(inner: Router) -> Router {
        splash_router_for(BootstrapStatus::Ready(inner))
    }

    fn splash_router_for(status: BootstrapStatus) -> Router {
        splash_router_with_progress(status, MigrationProgress::default())
    }

    fn splash_router_with_progress(
        status: BootstrapStatus,
        migration_progress: MigrationProgress,
    ) -> Router {
        // Dropping the sender is fine: `borrow()` keeps returning the last value.
        let (_status_tx, status_rx) = watch::channel(status);
        build_splash_router(
            SplashState {
                status_rx,
                migration_progress,
                restore: Default::default(),
            },
            CorsConfig {
                allow_all: false,
                allowed_origins: vec![],
            },
            BasePath::from_raw(Some("/scryer/")),
        )
    }

    async fn get_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let payload = serde_json::from_slice(&bytes).expect("json body");
        (status, payload)
    }

    #[tokio::test]
    async fn background_restore_keeps_liveness_but_blocks_readiness_and_normal_routes() {
        let directory = tempfile::tempdir().unwrap();
        let restore = std::sync::Arc::new(scryer_application::RestoreCoordinator::default());
        restore
            .start(
                directory.path(),
                "fixture",
                scryer_application::RestoreJobKind::Apply,
            )
            .unwrap();
        let deadline = scryer_application::RestoreDeadline::new();
        deadline.begin_import(4);
        deadline.imported_row();
        restore.track_deadline(&deadline);
        let (status_tx, status_rx) = watch::channel(BootstrapStatus::Ready(
            Router::new().route("/api/control", get(|| async { StatusCode::OK })),
        ));
        let app = build_splash_router(
            SplashState {
                status_rx,
                migration_progress: Default::default(),
                restore,
            },
            CorsConfig {
                allow_all: false,
                allowed_origins: vec![],
            },
            BasePath::from_raw(Some("/scryer/")),
        );
        let (status, body) = get_json(app.clone(), "/scryer/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "restoring");
        assert_eq!(body["ready"], false);
        assert_eq!(body["importPercent"], 25);
        // Import completion is not readiness: validation and startup still own
        // maintenance access after the percentage reaches 100.
        deadline.finish_import();
        let (status, body) = get_json(app.clone(), "/scryer/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["importPercent"], 100);
        assert_eq!(body["ready"], false);
        assert_eq!(
            get_json(app.clone(), "/scryer/health/ready").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let (status, body) = get_json(app.clone(), "/scryer/api/control").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["reason"], "RESTORE_MAINTENANCE_ACTIVE");
        // Credentials and different callers cannot skip the outer maintenance
        // boundary, even when the normal router has already been initialized.
        for method in ["GET", "POST", "DELETE"] {
            for (header, credential) in [
                ("authorization", "Bearer fixture-session"),
                ("x-api-key", "fixture-key"),
                ("cookie", "session=fixture-session"),
            ] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri("/scryer/api/control")
                            .header(header, credential)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            }
        }
        for path in ["/scryer/", "/scryer/settings", "/scryer/library"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header("accept", "text/html")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                response.headers()[axum::http::header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with("text/html")
            );
        }
        status_tx.send(BootstrapStatus::Migrating).unwrap();
        let (status, body) = get_json(app.clone(), "/scryer/api/control").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["reason"], "RESTORE_MAINTENANCE_ACTIVE");
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/scryer/graphql")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn liveness_reports_ok_while_migrating_but_flags_not_ready() {
        // Orchestrator liveness probes must not recycle the process during a
        // long migration: HTTP 200, with the phase carried in the body.
        let (status, payload) = get_json(
            splash_router_for(BootstrapStatus::Migrating),
            "/scryer/health",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["status"], "migrating");
        assert_eq!(payload["ready"], false);
    }

    #[tokio::test]
    async fn liveness_reports_migration_progress_once_counted() {
        let progress = MigrationProgress::default();
        let (_, payload) = get_json(
            splash_router_with_progress(BootstrapStatus::Migrating, progress.clone()),
            "/scryer/health",
        )
        .await;
        assert!(payload["migrations"].is_null(), "{payload}");

        progress.begin(30);
        for _ in 0..12 {
            progress.complete_one();
        }
        let (_, payload) = get_json(
            splash_router_with_progress(BootstrapStatus::Migrating, progress),
            "/scryer/health",
        )
        .await;
        assert_eq!(payload["migrations"]["completed"], 12);
        assert_eq!(payload["migrations"]["total"], 30);
    }

    #[test]
    fn splash_page_shows_the_count_only_while_migrations_are_applying() {
        let applying = splash_html(Some((12, 30)));
        assert!(
            applying
                .contains(r#"<div class="status" role="status">Upgrading database&hellip;</div>"#)
        );
        assert!(applying.contains(r#"<div class="progress">"#));
        assert!(applying.contains("12 of 30 migrations applied"));
        assert!(applying.contains("width: 40.0%"));

        for progress in [None, Some((30, 30))] {
            let page = splash_html(progress);
            assert!(
                page.contains(r#"<div class="status" role="status">Starting up&hellip;</div>"#),
                "{progress:?}"
            );
            assert!(
                page.contains(r#"<div class="progress" hidden>"#),
                "{progress:?}"
            );
        }
    }

    #[tokio::test]
    async fn readiness_is_unavailable_while_migrating() {
        let (status, payload) = get_json(
            splash_router_for(BootstrapStatus::Migrating),
            "/scryer/health/ready",
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(payload["status"], "migrating");
        assert_eq!(payload["ready"], false);
    }

    #[tokio::test]
    async fn liveness_and_readiness_report_ok_once_ready() {
        let inner = || Router::new().route("/", get(|| async { StatusCode::OK }));
        let (status, payload) = get_json(ready_splash_router(inner()), "/scryer/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["ready"], true);

        let (status, payload) =
            get_json(ready_splash_router(inner()), "/scryer/health/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["ready"], true);
    }

    #[tokio::test]
    async fn failed_bootstrap_reports_error_on_both_probes() {
        // A process that will never become ready should be recycled, so both
        // probes go non-2xx and carry the failure message.
        for path in ["/scryer/health", "/scryer/health/ready"] {
            let (status, payload) = get_json(
                splash_router_for(BootstrapStatus::Failed("boom".to_string())),
                path,
            )
            .await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
            assert_eq!(payload["status"], "error", "{path}");
            assert_eq!(payload["ready"], false, "{path}");
            assert_eq!(payload["message"], "boom", "{path}");
        }
    }

    #[tokio::test]
    async fn splash_serves_its_artwork_while_migrating() {
        // Every other path answers with the splash page, so the page's artwork
        // needs its own route, under the base path like the page's health poll.
        for (path, content_type, expected) in [
            (
                "/scryer/splash/scryer-loading.webp",
                "image/webp",
                super::SPLASH_LOADING_MARK,
            ),
            (
                "/scryer/splash/scryer-loading-still.webp",
                "image/webp",
                super::SPLASH_LOADING_MARK_STILL,
            ),
            (
                "/scryer/splash/scryer-wordmark.svg",
                "image/svg+xml",
                super::SPLASH_WORDMARK,
            ),
        ] {
            let response = splash_router_for(BootstrapStatus::Migrating)
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.headers()["content-type"], content_type, "{path}");
            let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
                .await
                .expect("body");
            assert_eq!(&bytes[..], expected, "{path}");
        }
    }

    #[tokio::test]
    async fn health_probes_answer_head_requests_like_get() {
        // `wget --spider` and `curl -I` style orchestrator probes send HEAD;
        // they must see the same status codes as GET on both endpoints.
        let cases = [
            (BootstrapStatus::Migrating, "/scryer/health", StatusCode::OK),
            (
                BootstrapStatus::Migrating,
                "/scryer/health/ready",
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                BootstrapStatus::Ready(Router::new()),
                "/scryer/health",
                StatusCode::OK,
            ),
            (
                BootstrapStatus::Ready(Router::new()),
                "/scryer/health/ready",
                StatusCode::OK,
            ),
        ];
        for (state, path, expected) in cases {
            let response = splash_router_for(state)
                .oneshot(
                    Request::builder()
                        .method(Method::HEAD)
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), expected, "HEAD {path}");
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body");
            assert!(bytes.is_empty(), "HEAD {path} must not carry a body");
        }
    }

    #[tokio::test]
    async fn prefixed_ready_router_serves_ui_root_without_redirect_loop() {
        let app = ready_splash_router(Router::new().route("/", get(|| async { StatusCode::OK })));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scryer/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn prefixed_ready_router_serves_subpaths() {
        let app =
            ready_splash_router(Router::new().route("/login", get(|| async { StatusCode::OK })));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scryer/login")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn prefixed_splash_router_does_not_handle_root() {
        let app = ready_splash_router(Router::new().route("/", get(|| async { StatusCode::OK })));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        // Root `/` should not be handled when a base path is configured — another
        // service may live at `/` behind the same reverse proxy.
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn prefixed_splash_health_uses_base_path() {
        let app = ready_splash_router(Router::new().route("/", get(|| async { StatusCode::OK })));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scryer/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }
}
