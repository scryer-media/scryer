use axum::Router;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BasePath {
    prefix: String,
}

impl BasePath {
    pub(crate) fn from_env() -> Self {
        Self::from_raw(std::env::var("SCRYER_BASE_PATH").ok().as_deref())
    }

    pub(crate) fn from_raw(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self {
                prefix: String::new(),
            };
        };

        let normalized = raw.trim().replace('\\', "/");
        let segments = normalized
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();

        if segments.is_empty() {
            return Self {
                prefix: String::new(),
            };
        }

        Self {
            prefix: format!("/{}", segments.join("/")),
        }
    }

    pub(crate) fn is_root(&self) -> bool {
        self.prefix.is_empty()
    }

    pub(crate) fn basename(&self) -> &str {
        if self.is_root() { "/" } else { &self.prefix }
    }

    pub(crate) fn ui_root(&self) -> String {
        if self.is_root() {
            "/".to_string()
        } else {
            format!("{}/", self.prefix)
        }
    }

    pub(crate) fn join(&self, suffix: &str) -> String {
        let normalized_suffix = if suffix.is_empty() {
            String::new()
        } else if suffix.starts_with('/') {
            suffix.to_string()
        } else {
            format!("/{suffix}")
        };

        if self.is_root() {
            if normalized_suffix.is_empty() {
                "/".to_string()
            } else {
                normalized_suffix
            }
        } else {
            format!("{}{}", self.prefix, normalized_suffix)
        }
    }
}

pub(crate) fn mount_router(router: Router, base_path: &BasePath) -> Router {
    if base_path.is_root() {
        return router;
    }

    let ui_root = base_path.ui_root();
    let base_prefix = base_path.basename().to_string();

    // Redirect the bare prefix (e.g. `/scryer`) to the trailing-slash version
    // (`/scryer/`).  Without the trailing slash the browser resolves relative
    // asset paths (`./manifest.json`) against the parent directory (`/`) instead
    // of the base path, breaking all static asset loads.
    let bare = base_prefix.clone();
    let target = ui_root.clone();

    Router::new()
        .nest_service(&base_prefix, router)
        .layer(axum::middleware::from_fn(
            move |req: Request, next: Next| {
                let bare = bare.clone();
                let target = target.clone();
                async move {
                    if req.uri().path() == bare {
                        Redirect::temporary(&target).into_response()
                    } else {
                        next.run(req).await
                    }
                }
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::BasePath;
    use super::mount_router;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    #[test]
    fn normalizes_root_base_path() {
        assert_eq!(BasePath::from_raw(None).basename(), "/");
        assert_eq!(BasePath::from_raw(Some("")).basename(), "/");
        assert_eq!(BasePath::from_raw(Some("/")).basename(), "/");
    }

    #[test]
    fn normalizes_prefixed_base_path() {
        assert_eq!(BasePath::from_raw(Some("scryer")).basename(), "/scryer");
        assert_eq!(
            BasePath::from_raw(Some("/nested/scryer/")).basename(),
            "/nested/scryer"
        );
    }

    #[test]
    fn joins_routes_without_double_slashes() {
        let root = BasePath::from_raw(None);
        let prefixed = BasePath::from_raw(Some("/scryer/"));

        assert_eq!(root.join("/graphql"), "/graphql");
        assert_eq!(prefixed.join("/graphql"), "/scryer/graphql");
        assert_eq!(prefixed.ui_root(), "/scryer/");
    }

    #[tokio::test]
    async fn prefixed_router_serves_trailing_slash_root() {
        let app = mount_router(
            Router::new().route("/", get(|| async { StatusCode::OK })),
            &BasePath::from_raw(Some("/scryer/")),
        );

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
    async fn prefixed_router_temporarily_redirects_bare_root_to_trailing_slash() {
        let app = mount_router(
            Router::new().route("/", get(|| async { StatusCode::OK })),
            &BasePath::from_raw(Some("/scryer/")),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scryer")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response
                .headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap(),
            "/scryer/"
        );
    }

    #[tokio::test]
    async fn prefixed_router_serves_subpaths() {
        let app = mount_router(
            Router::new().route("/login", get(|| async { StatusCode::OK })),
            &BasePath::from_raw(Some("/scryer/")),
        );

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
}
