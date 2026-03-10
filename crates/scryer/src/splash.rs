use std::sync::OnceLock;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tokio::sync::watch;
use tower::ServiceExt;
use tower_http::compression::CompressionLayer;

use crate::middleware::{cors_handler, CorsConfig};

#[derive(Clone)]
pub(crate) enum BootstrapStatus {
    Migrating,
    Ready(Router),
    Failed(String),
}

#[derive(Clone)]
pub(crate) struct SplashState {
    pub(crate) status_rx: watch::Receiver<BootstrapStatus>,
}

pub(crate) async fn splash_health_handler(State(state): State<SplashState>) -> Response {
    let status = state.status_rx.borrow().clone();
    match status {
        BootstrapStatus::Migrating => {
            Json(serde_json::json!({"status": "migrating"})).into_response()
        }
        BootstrapStatus::Ready(_) => Json(serde_json::json!({"status": "ok"})).into_response(),
        BootstrapStatus::Failed(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "message": message})),
        )
            .into_response(),
    }
}

pub(crate) async fn splash_fallback_handler(
    State(state): State<SplashState>,
    request: axum::extract::Request,
) -> Response {
    let status = state.status_rx.borrow().clone();
    match status {
        BootstrapStatus::Ready(router) => router
            .oneshot(request)
            .await
            .unwrap_or_else(|err| match err {}),
        BootstrapStatus::Migrating => Html(splash_html()).into_response(),
        BootstrapStatus::Failed(message) => Html(error_html(&message)).into_response(),
    }
}

pub(crate) fn build_splash_router(state: SplashState, cors: CorsConfig) -> Router {
    let cors_for_layer = cors.clone();

    Router::new()
        .route(
            "/health",
            get(splash_health_handler).with_state(state.clone()),
        )
        .fallback(splash_fallback_handler)
        .with_state(state)
        .layer(CompressionLayer::new().zstd(true).br(true).gzip(true))
        .layer(axum::middleware::from_fn(move |request, next| {
            cors_handler(request, next, cors_for_layer.clone())
        }))
}

fn splash_html() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| {
        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width,initial-scale=1"/>
<title>scryer — upgrading</title>
<style>{SPLASH_STYLE}</style>
</head>
<body>
<main>
  <h1>scryer</h1>
  <div class="spinner"></div>
  <div class="status">Upgrading database&hellip;</div>
</main>
<script>
(function() {{
  var delay = 200;
  function poll() {{
    fetch("/health")
      .then(function(r) {{ return r.json(); }})
      .then(function(d) {{
        if (d.status === "ok") location.reload();
        else if (d.status === "error") {{
          document.querySelector(".spinner").style.display = "none";
          var s = document.querySelector(".status");
          s.textContent = "Startup failed";
          s.classList.add("error");
          var p = document.createElement("p");
          p.className = "detail";
          p.textContent = d.message || "Unknown error";
          document.querySelector("main").appendChild(p);
          return;
        }}
      }})
      .catch(function() {{}})
      .finally(function() {{ delay = 1000; setTimeout(poll, delay); }});
  }}
  setTimeout(poll, delay);
}})();
</script>
</body>
</html>"#
        )
    })
}

fn error_html(message: &str) -> String {
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
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
  <h1>scryer</h1>
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
  background: #070b18;
  color: #dbe5ff;
  display: grid;
  place-items: center;
}
main {
  text-align: center;
  padding: 2rem;
}
h1 {
  font-family: "Space Grotesk", Inter, ui-sans-serif, system-ui, sans-serif;
  font-size: 2rem;
  font-weight: 700;
  letter-spacing: -0.02em;
  margin-bottom: 2rem;
  color: #dbe5ff;
}
.status {
  font-size: 0.95rem;
  color: #8b96b9;
  margin-bottom: 0.5rem;
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
@keyframes spin { to { transform: rotate(360deg); } }
.spinner {
  width: 28px;
  height: 28px;
  border: 3px solid #273255;
  border-top-color: #5b64ff;
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  margin: 0 auto 1.5rem;
}
"#;
