use crate::middleware::AuthState;
use axum::{
    Json,
    body::Body,
    extract::{MatchedPath, State},
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use scryer_application::initial_auth::{SetupState, refusal};

pub(crate) const REQUIRE_SETUP_CODE_ENV: &str = "SCRYER_REQUIRE_SETUP_CODE";

/// Unset or blank leaves the code off. A value that is not a recognized boolean
/// keeps it on, so a typo cannot silently drop a protection the operator asked for.
pub(crate) fn setup_code_required(raw: Option<&str>) -> bool {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| crate::parse_env_bool_value(value) != Some(false))
}

// Standard stderr deliberately treats a missing native handle as a successful
// no-op on some platforms. Setup must observe the real output failure.
pub(crate) fn print_setup_code(code: &str) -> std::io::Result<()> {
    operator_stderr()
        .and_then(|mut output| write_setup_code(&mut output, code))
        .map_err(|error| {
            std::io::Error::new(error.kind(), format!(
                "cannot write the initial setup code to process stderr: {error}; configure console/service output capture or supply SCRYER_ADMIN_PASSWORD or SCRYER_ADMIN_PASSWORD_FILE"
            ))
        })
}

#[cfg(unix)]
fn operator_stderr() -> std::io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    // SAFETY: fcntl validates the descriptor. A successful result is a new,
    // non-inheritable descriptor, owned by the returned File.
    let descriptor = unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD_CLOEXEC, 0) };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let output = unsafe { std::fs::File::from_raw_fd(descriptor) };
    let metadata = output.metadata()?;
    // Rust startup can replace a closed standard descriptor with /dev/null.
    if metadata.file_type().is_char_device()
        && let Ok(null) = std::fs::metadata("/dev/null")
        && metadata.rdev() == null.rdev()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "process stderr discards output",
        ));
    }
    Ok(output)
}

#[cfg(windows)]
fn operator_stderr() -> std::io::Result<std::fs::File> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::{
        Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, INVALID_HANDLE_VALUE},
        System::{
            Console::{GetStdHandle, STD_ERROR_HANDLE},
            Threading::GetCurrentProcess,
        },
    };
    // SAFETY: these calls only inspect/duplicate this process's standard handle.
    let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "process stderr has no output handle",
        ));
    }
    let process = unsafe { GetCurrentProcess() };
    let mut duplicate = std::ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            handle,
            process,
            &mut duplicate,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle transferred a valid, owned handle to us.
    Ok(unsafe { std::fs::File::from_raw_handle(duplicate) })
}

#[cfg(not(any(unix, windows)))]
fn operator_stderr() -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "native setup-code output is unavailable",
    ))
}

// Format the complete operator-only record before writing. Formatting directly
// into stderr emits fragments that container logs can interleave with stdout.
pub(crate) fn write_setup_code(
    output: &mut impl std::io::Write,
    code: &str,
) -> std::io::Result<()> {
    output.write_all(
        format!(
            "\n============================================================\n\
             SCRYER FIRST-RUN SETUP — ACTION REQUIRED\n\
             ============================================================\n\n\
             Scryer one-time setup code: {code}\n\n\
             Enter this code in the setup screen to configure access\n\
             or restore a backup. Restarting replaces this code.\n\
             Keep this code private.\n\
             ============================================================\n\n"
        )
        .as_bytes(),
    )?;
    output.flush()
}

#[derive(Clone)]
pub(crate) struct PendingSetup;

#[derive(Clone)]
pub(crate) struct SetupAdmission(pub Option<SetupState>);

pub(crate) fn refused(reason: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({"errors": [{
            "message": "Complete initial authentication setup before continuing.",
            "extensions": {"code": "VALIDATION_ERROR", "reason": reason}
        }]})),
    )
        .into_response()
}

pub(crate) async fn setup_state(
    state: &AuthState,
) -> scryer_application::AppResult<Option<SetupState>> {
    state
        .auth_runtime
        .initial_authentication_state(&state.app)
        .await
}

pub(crate) async fn admit_http(
    State(state): State<AuthState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let setup = match setup_state(&state).await {
        Ok(setup) => setup,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let pending = setup.as_ref().is_some_and(|state| state.is_pending());
    request.extensions_mut().insert(SetupAdmission(setup));
    if pending {
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(|path| path.as_str());
        // nest_service strips the configured prefix from the request URI,
        // while MatchedPath still includes it.
        let allowed = match request.uri().path() {
            "/health" | "/health/ready" => request.method() == Method::GET,
            "/graphql" => request.method() == Method::POST,
            _ if route.is_none() => request.method() == Method::GET,
            _ => false,
        };
        if !allowed {
            return refused(refusal::REQUIRED);
        }
        request.extensions_mut().insert(PendingSetup);
    }
    next.run(request).await
}

pub(crate) fn allows_graphql(request: &async_graphql::Request, stage: &SetupState) -> bool {
    use async_graphql::parser::{
        parse_query,
        types::{ExecutableDocument, OperationType, Selection, SelectionSet},
    };
    let Ok(document) = parse_query(&request.query) else {
        return false;
    };
    let selected = match request.operation_name.as_deref() {
        Some(name) => document
            .operations
            .iter()
            .find(|(n, _)| n.is_some_and(|n| n.as_str() == name)),
        None => {
            let mut iter = document.operations.iter();
            let first = iter.next();
            if iter.next().is_some() {
                return false;
            }
            first
        }
    };
    let Some((_, operation)) = selected else {
        return false;
    };
    if operation.node.ty == OperationType::Subscription {
        return false;
    }
    fn allowed(
        set: &SelectionSet,
        document: &ExecutableDocument,
        stage: &SetupState,
        mutation: bool,
        depth: usize,
        budget: &mut usize,
    ) -> bool {
        if depth > 32 || set.items.is_empty() {
            return false;
        }
        for item in &set.items {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            let ok = match &item.node {
                Selection::Field(field) => matches!(
                    (mutation, field.node.name.node.as_str(), stage),
                    (false, "authRuntimeState", _)
                        | (true, "completeInitialAuthentication", SetupState::Choice)
                        | (false, "restoreStatus", SetupState::Choice)
                        | (
                            true,
                            "startInspectRestoreBundle"
                                | "inspectRestoreBundle"
                                | "startApplyRestoreBundle"
                                | "applyRestoreBundle",
                            SetupState::Choice,
                        )
                        | (
                            true,
                            "login" | "completeRequiredPasswordChange",
                            SetupState::PasswordReplacement { .. },
                        )
                ),
                Selection::InlineFragment(fragment) => allowed(
                    &fragment.node.selection_set.node,
                    document,
                    stage,
                    mutation,
                    depth + 1,
                    budget,
                ),
                Selection::FragmentSpread(spread) => document
                    .fragments
                    .get(&spread.node.fragment_name.node)
                    .is_some_and(|fragment| {
                        allowed(
                            &fragment.node.selection_set.node,
                            document,
                            stage,
                            mutation,
                            depth + 1,
                            budget,
                        )
                    }),
            };
            if !ok {
                return false;
            }
        }
        true
    }
    allowed(
        &operation.node.selection_set.node,
        &document,
        stage,
        operation.node.ty == OperationType::Mutation,
        0,
        &mut 1024,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{self, integration_test_common::TestContext};
    use scryer_application::UserRepository;
    use tower::ServiceExt;

    #[test]
    fn setup_code_is_written_as_one_complete_operator_record() {
        #[derive(Default)]
        struct RecordWriter(Vec<Vec<u8>>);
        impl std::io::Write for RecordWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if !bytes.ends_with(b"\n") {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                self.0.push(bytes.to_vec());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut output = RecordWriter::default();
        write_setup_code(&mut output, "ABC-234").unwrap();
        assert_eq!(output.0.len(), 1, "the banner must be one write");
        let banner = String::from_utf8(output.0.remove(0)).unwrap();
        assert!(banner.contains("SCRYER FIRST-RUN SETUP — ACTION REQUIRED"));
        assert!(banner.contains("\nScryer one-time setup code: ABC-234\n"));
        assert!(banner.starts_with("\n================"));
    }

    #[test]
    fn setup_code_output_failure_is_propagated() {
        struct UnavailableWriter;
        impl std::io::Write for UnavailableWriter {
            fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let error = write_setup_code(&mut UnavailableWriter, "ABC-234").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[cfg(any(unix, windows))]
    #[tokio::test]
    async fn setup_code_requires_real_operator_output() {
        const CHILD_MODE: &str = "SCRYER_TEST_SETUP_CODE_OUTPUT";
        const CODE: &str = "ABC-234";
        if let Some(mode) = std::env::var_os(CHILD_MODE) {
            if mode == "closed" {
                #[cfg(unix)]
                // SAFETY: only this isolated child test owns its stderr descriptor.
                assert_eq!(unsafe { libc::close(libc::STDERR_FILENO) }, 0);
                #[cfg(windows)]
                // SAFETY: only this isolated child test changes its standard handle.
                assert_ne!(
                    unsafe {
                        windows_sys::Win32::System::Console::SetStdHandle(
                            windows_sys::Win32::System::Console::STD_ERROR_HANDLE,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
            }
            let result = print_setup_code(CODE);
            if mode == "captured" {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                assert!(!error.to_string().contains(CODE));
            }
            return;
        }
        let modes = if cfg!(unix) {
            &["captured", "closed", "discarded"][..]
        } else {
            &["captured", "closed"][..]
        };
        for mode in modes {
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "initial_auth::tests::setup_code_requires_real_operator_output",
                    "--nocapture",
                ])
                .env(CHILD_MODE, mode)
                .kill_on_drop(true)
                .stdout(std::process::Stdio::piped())
                .stderr(if *mode == "discarded" {
                    std::process::Stdio::null()
                } else {
                    std::process::Stdio::piped()
                });
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                child.spawn().unwrap().wait_with_output(),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(
                output.status.success(),
                "{mode}: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if *mode == "captured" {
                let mut expected = Vec::new();
                write_setup_code(&mut expected, CODE).unwrap();
                assert_eq!(output.stderr, expected);
            }
        }
    }

    #[test]
    fn setup_code_flush_failure_is_propagated() {
        struct FlushFailure;
        impl std::io::Write for FlushFailure {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        assert_eq!(
            write_setup_code(&mut FlushFailure, "ABC-234")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn initial_auth_graphql_admission_checks_actual_fields_and_stage() {
        let choice = SetupState::Choice;
        assert!(allows_graphql(
            &async_graphql::Request::new(
                "query { alias: authRuntimeState { initialAuthenticationStage } }"
            ),
            &choice
        ));
        assert!(allows_graphql(
            &async_graphql::Request::new(
                "query { ...F } fragment F on Query { authRuntimeState { passwordMinLength } }"
            ),
            &choice
        ));
        for query in [
            "query { authRuntimeState { passwordMinLength } users { id } }",
            "query { ...F } fragment F on Query { alias: users { id } }",
            "mutation { completeSetup { completed } }",
            "subscription { events { id } }",
            "query A { authRuntimeState { passwordMinLength } } query B { users { id } }",
            "query { ...F } fragment F on Query { ...F }",
            "mutation { login(input: {username: \"admin\", password: \"test\"}) { token } }",
        ] {
            assert!(
                !allows_graphql(&async_graphql::Request::new(query), &choice),
                "{query}"
            );
        }
        let replacement = SetupState::PasswordReplacement {
            user_id: "admin".into(),
        };
        assert!(allows_graphql(
            &async_graphql::Request::new(
                "mutation { completeRequiredPasswordChange(input: {password: \"replacement\"}) { token } }"
            ),
            &replacement
        ));
        assert!(!allows_graphql(
            &async_graphql::Request::new(
                "mutation { completeInitialAuthentication(input: {}) { completed } }"
            ),
            &replacement
        ));
    }

    async fn fixture() -> TestContext {
        let context = tokio::time::timeout(std::time::Duration::from_secs(60), TestContext::new())
            .await
            .unwrap();
        sqlx::query("UPDATE settings_values SET value_json = '{\"stage\":\"choice\"}' WHERE id = 'initial-authentication-setup'")
            .execute(&context.db.pool).await.unwrap();
        crate::settings_bootstrap::seed_service_setting_definitions(context.settings_store.clone())
            .await
            .unwrap();
        context
    }

    #[tokio::test]
    async fn initial_auth_choice_is_atomic_and_only_one_submission_wins() {
        let context = fixture().await;
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::Choice)
        );
        let (first, second) = tokio::join!(
            context
                .app
                .complete_initial_authentication(Some("first-password".into()), false),
            context
                .app
                .complete_initial_authentication(Some("second-password".into()), false),
        );
        assert_ne!(first.is_ok(), second.is_ok());
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::Complete)
        );
        let settings = context.app.security_settings().await.unwrap();
        assert!(settings.form_login_enabled);
        assert!(!settings.skip_login_for_local_ips);
        assert!(
            context
                .users
                .get_by_username("admin")
                .await
                .unwrap()
                .unwrap()
                .password_hash
                .is_some()
        );
        assert!(
            context
                .app
                .complete_initial_authentication(None, true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn initial_auth_failed_write_preserves_pending_choice_and_credentials() {
        let context = fixture().await;
        sqlx::query("CREATE TRIGGER fail_initial_auth BEFORE UPDATE OF password_hash ON users BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END")
            .execute(&context.db.pool).await.unwrap();
        assert!(
            context
                .app
                .complete_initial_authentication(Some("test-password".into()), false)
                .await
                .is_err()
        );
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::Choice)
        );
        assert!(
            context
                .users
                .get_by_username("admin")
                .await
                .unwrap()
                .unwrap()
                .password_hash
                .is_none()
        );
        sqlx::query("DROP TRIGGER fail_initial_auth")
            .execute(&context.db.pool)
            .await
            .unwrap();
        context
            .app
            .complete_initial_authentication(Some("test-password".into()), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn initial_auth_no_auth_requires_acknowledgement_and_blocks_generic_writes() {
        let context = fixture().await;
        assert!(
            context
                .app
                .complete_initial_authentication(None, false)
                .await
                .is_err()
        );
        use scryer_application::SettingsRepository;
        assert!(
            context
                .settings_store
                .delete_setting_value(
                    "system",
                    scryer_application::initial_auth::SETTING_KEY,
                    None
                )
                .await
                .is_err()
        );
        assert!(
            context
                .settings_store
                .upsert_setting_json(
                    "system",
                    scryer_application::initial_auth::SETTING_KEY,
                    None,
                    "{\"stage\":\"complete\"}".into(),
                    "fixture",
                    None
                )
                .await
                .is_err()
        );
        context
            .app
            .complete_initial_authentication(None, true)
            .await
            .unwrap();
        assert!(
            !context
                .app
                .security_settings()
                .await
                .unwrap()
                .form_login_enabled
        );
        assert!(
            context
                .users
                .get_by_username("admin")
                .await
                .unwrap()
                .unwrap()
                .password_hash
                .is_none()
        );
    }

    #[tokio::test]
    async fn initial_auth_bootstrap_can_resume_with_an_initialized_passwordless_admin() {
        let context = fixture().await;
        let admin = context.app.find_or_create_default_user().await.unwrap();
        context
            .app
            .configure_bootstrap_admin(None, Some("temporary-password"), false, false)
            .await
            .unwrap();
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::PasswordReplacement { user_id: admin.id })
        );
        assert!(
            context
                .app
                .security_settings()
                .await
                .unwrap()
                .form_login_enabled
        );
        assert!(
            context
                .app
                .complete_initial_authentication(None, true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn initial_auth_bootstrap_requires_replacement_and_preserves_it_on_restart() {
        let context = fixture().await;
        context
            .app
            .configure_bootstrap_admin(Some("operator"), Some("temporary-password"), false, true)
            .await
            .unwrap();
        let user = context
            .users
            .get_by_username("operator")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::PasswordReplacement {
                user_id: user.id.clone()
            })
        );
        assert!(
            context
                .app
                .complete_initial_authentication(None, true)
                .await
                .is_err()
        );
        let version = context.users.auth_session_version(&user.id).await.unwrap();
        assert!(
            context
                .app
                .complete_required_password_change(&user, "temporary-password".into(), &version)
                .await
                .is_err()
        );
        let (saved, _) = context
            .app
            .complete_required_password_change(&user, "permanent-password".into(), &version)
            .await
            .unwrap();
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::Complete)
        );
        context
            .app
            .configure_bootstrap_admin(Some("operator"), Some("temporary-password"), false, true)
            .await
            .unwrap();
        let restarted = context.users.get_by_id(&user.id).await.unwrap().unwrap();
        assert_eq!(restarted.password_hash, saved.password_hash);
        assert!(!restarted.password_change_required);
    }

    fn http_state(context: &TestContext) -> middleware::AuthState {
        middleware::AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            auth_runtime: context.auth_runtime.clone(),
            rate_limiter: crate::rate_limit::ScryerRateLimiter::from_env(Default::default()),
            ws_origin_policy: middleware::WebSocketOriginPolicy::default(),
            list_account_origin_policy: Default::default(),
            authless_web_client_proof: middleware::AuthlessWebClientProofState::new(),
        }
    }

    fn http_router(context: &TestContext) -> axum::Router {
        let state = http_state(context);
        axum::Router::new()
            .route(
                "/graphql",
                axum::routing::post(middleware::graphql_handler).with_state(state.clone()),
            )
            .fallback(axum::routing::get(|| async { "web shell" }))
            .layer(axum::middleware::from_fn_with_state(state, admit_http))
    }

    async fn graphql(
        router: axum::Router,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        graphql_with_token(router, path, body, None).await
    }

    async fn graphql_with_token(
        router: axum::Router,
        path: &str,
        body: serde_json::Value,
        token: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        if let Some(token) = token {
            request
                .headers_mut()
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
        }
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(60), router.oneshot(request))
                .await
                .unwrap()
                .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn initial_auth_code_limits_isolate_http_clients_for_setup_and_restore() {
        async fn request(
            router: axum::Router,
            peer: &str,
            forwarded: &str,
            body: serde_json::Value,
        ) -> serde_json::Value {
            let mut request = Request::builder()
                .method("POST")
                .uri("/graphql")
                .header("content-type", "application/json")
                .header("x-forwarded-for", forwarded)
                .body(Body::from(body.to_string()))
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                peer.parse::<std::net::SocketAddr>().unwrap(),
            ));
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(60), router.oneshot(request))
                    .await
                    .unwrap()
                    .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }
        for proxied in [false, true] {
            let mut context = fixture().await;
            let now = std::time::Instant::now();
            context.auth_runtime.initial_authentication = std::sync::Arc::new(
                scryer_application::initial_auth::SetupChallenge::with_clock(move || now),
            );
            context.schema = scryer_interface::build_schema_with_log_buffer_and_restore(
                context.app.clone(),
                context.auth_runtime.clone(),
                None,
                None,
            );
            let code = context
                .auth_runtime
                .initial_authentication
                .initialize()
                .unwrap();
            let mut state = http_state(&context);
            state.rate_limiter = crate::rate_limit::ScryerRateLimiter::for_network_test(
                if proxied { &["127.0.0.1"] } else { &[] },
                // Keep this regression independent of the generic request quota;
                // setup proof still limits failures for configured bypass clients.
                &["192.0.2.0/24"],
            );
            let router = axum::Router::new()
                .route(
                    "/graphql",
                    axum::routing::post(middleware::graphql_handler).with_state(state.clone()),
                )
                .layer(axum::middleware::from_fn_with_state(state, admit_http));
            let attacker = if proxied {
                "127.0.0.1:12345"
            } else {
                "192.0.2.1:12345"
            };
            let operator = if proxied {
                "127.0.0.1:12345"
            } else {
                "192.0.2.2:12345"
            };
            for _ in 0..5 {
                let body = request(
                    router.clone(),
                    attacker,
                    "192.0.2.1",
                    serde_json::json!({
                        "query": "{ restoreStatus(setupCode: \"wrong\") { id } }",
                    }),
                )
                .await;
                assert_eq!(
                    body["errors"][0]["extensions"]["reason"],
                    refusal::CODE_INVALID,
                    "{body}"
                );
            }
            let mutation = serde_json::json!({
                "query": "mutation($input: CompleteInitialAuthenticationInput!) { completeInitialAuthentication(input: $input) { completed } }",
                "variables": {"input": {"mode": "NO_AUTHENTICATION", "acknowledgeNoAuthentication": true, "setupCode": code}},
            });
            let forwarded = if proxied { "192.0.2.1" } else { "192.0.2.2" };
            let limited = request(router.clone(), attacker, forwarded, mutation.clone()).await;
            assert_eq!(
                limited["errors"][0]["extensions"]["reason"],
                refusal::CODE_RATE_LIMITED,
                "{limited}"
            );
            let restored = request(
                router.clone(),
                operator,
                "192.0.2.2",
                serde_json::json!({
                    "query": "query($code: String) { restoreStatus(setupCode: $code) { id } }",
                    "variables": {"code": code},
                }),
            )
            .await;
            assert!(restored.get("errors").is_none(), "{restored}");
            let completed = request(router, operator, "192.0.2.2", mutation).await;
            assert_eq!(
                completed["data"]["completeInitialAuthentication"]["completed"], true,
                "{completed}"
            );
        }
    }

    #[tokio::test]
    async fn initial_auth_terminal_admission_is_cached_until_restore() {
        for completed in [false, true] {
            let context = fixture().await;
            if completed {
                context
                    .app
                    .complete_initial_authentication(None, true)
                    .await
                    .unwrap();
            } else {
                sqlx::query(
                    "DELETE FROM settings_values WHERE id = 'initial-authentication-setup'",
                )
                .execute(&context.db.pool)
                .await
                .unwrap();
            }
            let state = http_state(&context);
            let expected = completed.then_some(SetupState::Complete);
            assert_eq!(setup_state(&state).await.unwrap(), expected);
            // Removing the database proves settled HTTP admission does not read it.
            context.db.pool.close().await;
            assert_eq!(setup_state(&state).await.unwrap(), expected);
            let directory = tempfile::tempdir().unwrap();
            context
                .app
                .restore_coordinator()
                .start(
                    directory.path(),
                    "fixture-upload",
                    scryer_application::RestoreJobKind::Apply,
                )
                .unwrap();
            assert!(
                setup_state(&state).await.is_err(),
                "restore must invalidate settled admission and fail closed on unreadable state"
            );
        }
    }

    #[tokio::test]
    async fn initial_auth_restore_controls_require_code_and_do_not_grant_other_access() {
        let mut context = fixture().await;
        let code = context
            .auth_runtime
            .initial_authentication
            .initialize()
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        context.schema = scryer_interface::build_schema_with_log_buffer_and_restore(
            context.app.clone(),
            context.auth_runtime.clone(),
            None,
            Some(scryer_interface::RestoreContext {
                data_dir: directory.path().to_path_buf(),
                datastore_config: scryer_interface::context::RestoreDatastoreConfig {
                    engine: scryer_interface::context::RestoreDatastoreEngine::Sqlite,
                    migration_mode: scryer_interface::context::RestoreMigrationMode::Apply,
                },
                datastore: scryer_interface::context::RestoreDatastoreHandle::unavailable(),
                restart: scryer_interface::RestoreRestartHandle::new(|| {
                    panic!("no restore was staged")
                }),
            }),
        );
        let router = http_router(&context);
        for query in [
            "query($code: String) { result: restoreStatus(setupCode: $code) { id } }",
            "mutation($code: String) { startApplyRestoreBundle(uploadId: \"../invalid\", setupCode: $code) { id } }",
        ] {
            let (_, body) = graphql(
                router.clone(),
                "/graphql",
                serde_json::json!({"query": query, "variables": {"code": "wrong"}}),
            )
            .await;
            assert_eq!(
                body["errors"][0]["extensions"]["reason"],
                refusal::CODE_INVALID,
                "{body}"
            );
        }
        let (_, body) = graphql(router.clone(), "/graphql", serde_json::json!({
            "query": "query($code: String) { ...Fields } fragment Fields on QueryRoot { renamed: restoreStatus(setupCode: $code) { id } }",
            "variables": {"code": code},
        })).await;
        assert!(body.get("errors").is_none(), "{body}");
        assert!(body["data"]["renamed"].is_null());
        let (_, body) = graphql(router.clone(), "/graphql", serde_json::json!({
            "query": "mutation($code: String) { startApplyRestoreBundle(uploadId: \"../invalid\", setupCode: $code) { id } }",
            "variables": {"code": code},
        })).await;
        assert_eq!(
            body["errors"][0]["extensions"]["reason"], "INTERFACE_RESTORE_UPLOAD_ID_INVALID",
            "{body}"
        );
        let direct = context
            .schema
            .execute(
                async_graphql::Request::new(
                    "query($code: String) { restoreStatus(setupCode: $code) { id } }",
                )
                .variables(async_graphql::Variables::from_json(
                    serde_json::json!({"code": code}),
                )),
            )
            .await;
        assert_eq!(
            direct.errors[0].extensions.as_ref().unwrap().get("reason"),
            Some(&async_graphql::Value::from(refusal::REQUIRED))
        );
        // Exercise the browser's multipart upload boundary as well as JSON controls.
        for (supplied_code, filename) in [
            ("wrong", "fixture.enc"),
            (code.as_str(), "fixture.tar.zst"),
            (code.as_str(), "fixture.ENC"),
        ] {
            let operations = serde_json::json!({
                "query": "mutation($file: Upload!, $code: String) { startInspectRestoreBundle(bundleUpload: $file, setupCode: $code) { id } }",
                "variables": {"file": null, "code": supplied_code},
            });
            let body = format!(
                "--fixture-boundary\r\nContent-Disposition: form-data; name=\"operations\"\r\n\r\n{operations}\r\n--fixture-boundary\r\nContent-Disposition: form-data; name=\"map\"\r\n\r\n{{\"0\":[\"variables.file\"]}}\r\n--fixture-boundary\r\nContent-Disposition: form-data; name=\"0\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\ninvalid-fixture-bundle\r\n--fixture-boundary--\r\n"
            );
            let mut request = Request::builder()
                .method("POST")
                .uri("/graphql")
                .header(
                    "content-type",
                    "multipart/form-data; boundary=fixture-boundary",
                )
                .body(Body::from(body))
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
            ));
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                router.clone().oneshot(request),
            )
            .await
            .unwrap()
            .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if supplied_code == "wrong" {
                assert_eq!(
                    body["errors"][0]["extensions"]["reason"],
                    refusal::CODE_INVALID,
                    "{body}"
                );
                assert!(context.app.restore_coordinator().snapshot().is_none());
            } else if filename.ends_with(".tar.zst") {
                assert_eq!(
                    body["errors"][0]["extensions"]["reason"],
                    scryer_application::error_reasons::security::BACKUP_BUNDLE_INVALID,
                    "{body}"
                );
                assert!(context.app.restore_coordinator().snapshot().is_none());
            } else {
                assert!(body.get("errors").is_none(), "{body}");
                let id = body["data"]["startInspectRestoreBundle"]["id"]
                    .as_str()
                    .unwrap();
                let coordinator = context.app.restore_coordinator();
                tokio::time::timeout(std::time::Duration::from_secs(60), async {
                    loop {
                        let changed = coordinator.changed.notified();
                        tokio::pin!(changed);
                        changed.as_mut().enable();
                        let status = coordinator.snapshot().unwrap();
                        assert_eq!(status.id, id);
                        if !status.phase.active() {
                            break;
                        }
                        changed.await;
                    }
                })
                .await
                .unwrap();
            }
        }
        for query in [
            "query($code: String) { restoreStatus(setupCode: $code) { id } users { id } }",
            "mutation { completeSetup { completed } }",
            "query { ...Fields } fragment Fields on Query { renamed: users { id } }",
        ] {
            let (status, _) = graphql(
                router.clone(),
                "/graphql",
                serde_json::json!({"query": query, "variables": {"code": code}}),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
        let (status, _) = graphql(
            router,
            "/graphql",
            serde_json::json!([
                {"query": "{ restoreStatus { id } }"}, {"query": "{ users { id } }"}
            ]),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::Choice)
        );
        context
            .auth_runtime
            .initial_authentication
            .verify("127.0.0.1".parse().unwrap(), Some(&code))
            .unwrap();
    }

    #[tokio::test]
    async fn initial_auth_http_gate_blocks_routes_and_mixed_graphql_batches() {
        let context = fixture().await;
        context
            .auth_runtime
            .initial_authentication
            .initialize()
            .unwrap();
        let state = http_state(&context);
        let router = axum::Router::new()
            .route(
                "/graphql",
                axum::routing::post(middleware::graphql_handler).with_state(state.clone()),
            )
            .route(
                "/graphql/ws",
                axum::routing::get(|| async { "must not reach" }),
            )
            .route(
                "/api/test",
                axum::routing::get(|| async { "must not reach" }),
            )
            .fallback(axum::routing::get(|| async { "web shell" }))
            .layer(axum::middleware::from_fn_with_state(state, admit_http));
        for path in ["/graphql/ws", "/api/test"] {
            let response = router
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/setup?step=99")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut request = Request::builder().method("POST").uri("/graphql").header("content-type", "application/json")
            .body(Body::from(r#"[{"query":"{ authRuntimeState { passwordMinLength } }"},{"query":"{ users { id } }"}]"#)).unwrap();
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
        ));
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn initial_auth_loopback_http_choices_require_code_and_survive_prefixed_routes() {
        for password_mode in [true, false] {
            let context = fixture().await;
            let code = context
                .auth_runtime
                .initial_authentication
                .initialize()
                .unwrap();
            let router = crate::base_path::mount_router(
                http_router(&context),
                &crate::base_path::BasePath::from_raw(Some("/scryer")),
            );
            let shell = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/scryer/setup?skip=true")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(shell.status(), StatusCode::OK);
            let (status, metadata) = graphql(router.clone(), "/scryer/graphql", serde_json::json!({"query":"{ authRuntimeState { initialAuthenticationStage initialAuthenticationCodeRequired passwordMinLength } }"})).await;
            assert_eq!(status, StatusCode::OK, "{metadata}");
            assert_eq!(
                metadata["data"]["authRuntimeState"]["initialAuthenticationStage"],
                "choice"
            );
            assert_eq!(
                metadata["data"]["authRuntimeState"]["initialAuthenticationCodeRequired"],
                true
            );
            assert!(!metadata.to_string().contains(&code));
            let query = "mutation Decide($input: CompleteInitialAuthenticationInput!) { completeInitialAuthentication(input: $input) { completed login { token } } }";
            let mut input = serde_json::json!({"mode": if password_mode { "PASSWORD" } else { "NO_AUTHENTICATION" },
                "password": password_mode.then_some("permanent-password"), "acknowledgeNoAuthentication": !password_mode, "setupCode": "invalid"});
            // The loopback peer has no forwarding headers, exactly as with a
            // reverse proxy that relays remote requests without adding them.
            for supplied_code in [serde_json::Value::Null, "invalid".into()] {
                input["setupCode"] = supplied_code;
                let (_, failed) = graphql(
                    router.clone(),
                    "/scryer/graphql",
                    serde_json::json!({"query":query,"variables":{"input":input}}),
                )
                .await;
                assert_eq!(
                    failed["errors"][0]["extensions"]["reason"],
                    refusal::CODE_INVALID,
                    "{failed}"
                );
                assert_eq!(
                    context.app.initial_authentication_state().await.unwrap(),
                    Some(SetupState::Choice)
                );
            }
            input["setupCode"] = code.into();
            let (_, result) = graphql(
                router.clone(),
                "/scryer/graphql",
                serde_json::json!({"query":query,"variables":{"input":input}}),
            )
            .await;
            assert_eq!(
                result["data"]["completeInitialAuthentication"]["completed"], true,
                "{result}"
            );
            assert_eq!(
                result["data"]["completeInitialAuthentication"]["login"].is_object(),
                password_mode
            );
            assert_eq!(
                context.auth_runtime.snapshot().effective_form_login_enabled,
                password_mode
            );
            assert!(!context.auth_runtime.initial_authentication.is_available());
            let (status, stale) = graphql(
                router,
                "/scryer/graphql",
                serde_json::json!({"query":query,"variables":{"input":input}}),
            )
            .await;
            if password_mode {
                assert_eq!(
                    stale["errors"][0]["extensions"]["reason"],
                    refusal::ALREADY_COMPLETED,
                    "{stale}"
                );
            } else {
                // Ordinary authless admission again requires the web-client proof.
                assert_eq!(status, StatusCode::FORBIDDEN, "{stale}");
            }
        }
    }

    #[test]
    fn initial_auth_setup_code_is_opt_in_and_fails_closed_on_unknown_values() {
        for raw in [None, Some(""), Some("  "), Some("0"), Some("false")] {
            assert!(!setup_code_required(raw), "{raw:?}");
        }
        for raw in ["1", "true", "on", "enabled"] {
            assert!(setup_code_required(Some(raw)), "{raw}");
        }
    }

    #[tokio::test]
    async fn initial_auth_choice_without_code_is_still_required_and_completes() {
        for password_mode in [true, false] {
            let context = fixture().await;
            context
                .auth_runtime
                .initial_authentication
                .initialize_without_code();
            let router = crate::base_path::mount_router(
                http_router(&context),
                &crate::base_path::BasePath::from_raw(Some("/scryer")),
            );
            let (status, metadata) = graphql(router.clone(), "/scryer/graphql", serde_json::json!({"query":"{ authRuntimeState { initialAuthenticationStage initialAuthenticationCodeRequired } }"})).await;
            assert_eq!(status, StatusCode::OK, "{metadata}");
            assert_eq!(
                metadata["data"]["authRuntimeState"]["initialAuthenticationStage"],
                "choice"
            );
            assert_eq!(
                metadata["data"]["authRuntimeState"]["initialAuthenticationCodeRequired"],
                false
            );
            let (status, blocked) = graphql(
                router.clone(),
                "/scryer/graphql",
                serde_json::json!({"query":"{ users { id } }"}),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{blocked}");
            let (_, restore) = graphql(
                router.clone(),
                "/scryer/graphql",
                serde_json::json!({"query":"{ restoreStatus { id } }"}),
            )
            .await;
            assert!(restore.get("errors").is_none(), "{restore}");
            let query = "mutation Decide($input: CompleteInitialAuthenticationInput!) { completeInitialAuthentication(input: $input) { completed login { token } } }";
            let input = serde_json::json!({"mode": if password_mode { "PASSWORD" } else { "NO_AUTHENTICATION" },
                "password": password_mode.then_some("permanent-password"), "acknowledgeNoAuthentication": !password_mode});
            let (_, result) = graphql(
                router,
                "/scryer/graphql",
                serde_json::json!({"query":query,"variables":{"input":input}}),
            )
            .await;
            assert_eq!(
                result["data"]["completeInitialAuthentication"]["completed"], true,
                "{result}"
            );
            assert_eq!(
                context.app.initial_authentication_state().await.unwrap(),
                Some(SetupState::Complete)
            );
            assert!(!context.auth_runtime.initial_authentication.is_available());
        }
    }

    #[tokio::test]
    async fn initial_auth_admission_publishes_committed_password_mode_before_access() {
        let context = fixture().await;
        context
            .auth_runtime
            .initial_authentication
            .initialize()
            .unwrap();
        context
            .auth_runtime
            .apply_saved_security_settings(false, false);
        assert!(
            !context
                .app
                .security_settings()
                .await
                .unwrap()
                .form_login_enabled
        );
        context
            .app
            .complete_initial_authentication(Some("permanent-password".into()), false)
            .await
            .unwrap();
        assert!(!context.auth_runtime.snapshot().effective_form_login_enabled);
        assert_eq!(
            setup_state(&http_state(&context)).await.unwrap(),
            Some(SetupState::Complete)
        );
        assert!(context.auth_runtime.snapshot().effective_form_login_enabled);
    }

    #[tokio::test]
    async fn initial_auth_http_bootstrap_replacement_uses_a_restricted_session() {
        let context = fixture().await;
        context
            .app
            .configure_bootstrap_admin(Some("operator"), Some("temporary-password"), false, true)
            .await
            .unwrap();
        context
            .auth_runtime
            .apply_saved_security_settings(true, false);
        let router = http_router(&context);
        let (_, metadata) = graphql_with_token(
            router.clone(),
            "/graphql",
            serde_json::json!({"query":"{ authRuntimeState { initialAuthenticationStage } }"}),
            Some("stale-token"),
        )
        .await;
        assert_eq!(
            metadata["data"]["authRuntimeState"]["initialAuthenticationStage"],
            "password_replacement",
            "{metadata}"
        );
        let (_, login) = graphql(router.clone(), "/graphql", serde_json::json!({"query":"mutation { login(input: {username: \"operator\", password: \"temporary-password\"}) { token passwordChangeRequired } }"})).await;
        assert_eq!(
            login["data"]["login"]["passwordChangeRequired"], true,
            "{login}"
        );
        let token = login["data"]["login"]["token"].as_str().unwrap();
        let (status, _) = graphql_with_token(
            router.clone(),
            "/graphql",
            serde_json::json!({"query":"{ me { id } }"}),
            Some(token),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        context
            .auth_runtime
            .apply_saved_security_settings(false, false);
        let (_, refused) = graphql_with_token(router.clone(), "/graphql", serde_json::json!({"query":"mutation { completeRequiredPasswordChange(input: {password: \"permanent-password\"}) { token } }"}), Some(token)).await;
        assert_eq!(
            refused["errors"][0]["extensions"]["reason"],
            refusal::MODE_CONFLICT,
            "{refused}"
        );
        assert!(matches!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::PasswordReplacement { .. })
        ));
        context
            .auth_runtime
            .apply_saved_security_settings(true, false);
        let (_, changed) = graphql_with_token(router.clone(), "/graphql", serde_json::json!({"query":"mutation { completeRequiredPasswordChange(input: {password: \"permanent-password\"}) { token passwordChangeRequired } }"}), Some(token)).await;
        assert_eq!(
            changed["data"]["completeRequiredPasswordChange"]["passwordChangeRequired"], false,
            "{changed}"
        );
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            Some(SetupState::Complete)
        );
        let token = changed["data"]["completeRequiredPasswordChange"]["token"]
            .as_str()
            .unwrap();
        let (_, me) = graphql_with_token(
            router,
            "/graphql",
            serde_json::json!({"query":"{ me { username } }"}),
            Some(token),
        )
        .await;
        assert_eq!(me["data"]["me"]["username"], "operator", "{me}");
    }

    #[tokio::test]
    async fn initial_auth_recovery_requires_replacement_and_preserves_atomic_admission() {
        for bootstrap_first in [false, true] {
            let context = fixture().await;
            if bootstrap_first {
                context
                    .app
                    .configure_bootstrap_admin(
                        Some("operator"),
                        Some("lost-temporary-password"),
                        false,
                        true,
                    )
                    .await
                    .unwrap();
            }
            let previous = context.app.initial_authentication_state().await.unwrap();
            context.app.set_recovery_admin_login_enabled(true);
            sqlx::query("CREATE TRIGGER fail_recovery_selection BEFORE UPDATE OF password_change_required ON users WHEN NEW.username = 'recovery-admin' AND NEW.password_change_required = 1 BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END")
                .execute(&context.db.pool).await.unwrap();
            assert!(
                context
                    .app
                    .recover_reserved_admin_access("recovery-temporary-password")
                    .await
                    .is_err()
            );
            assert_eq!(
                context.app.initial_authentication_state().await.unwrap(),
                previous
            );
            sqlx::query("DROP TRIGGER fail_recovery_selection")
                .execute(&context.db.pool)
                .await
                .unwrap();
            let recovered = context
                .app
                .recover_reserved_admin_access("recovery-temporary-password")
                .await
                .unwrap();
            assert!(recovered.password_change_required);
            assert_eq!(
                context.app.initial_authentication_state().await.unwrap(),
                Some(SetupState::PasswordReplacement {
                    user_id: recovered.id.clone()
                })
            );
            let settings = context.app.security_settings().await.unwrap();
            assert!(settings.form_login_enabled);
            assert!(!settings.skip_login_for_local_ips);
            context
                .auth_runtime
                .apply_saved_security_settings(true, false);
            let router = http_router(&context);
            let (_, login) = graphql(router.clone(), "/graphql", serde_json::json!({"query":"mutation { login(input: {username: \"recovery-admin\", password: \"recovery-temporary-password\"}) { token passwordChangeRequired } }"})).await;
            assert_eq!(
                login["data"]["login"]["passwordChangeRequired"], true,
                "{login}"
            );
            let token = login["data"]["login"]["token"].as_str().unwrap();
            let (status, _) = graphql_with_token(
                router.clone(),
                "/graphql",
                serde_json::json!({"query":"{ me { id } }"}),
                Some(token),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            let (_, unchanged) = graphql_with_token(router.clone(), "/graphql", serde_json::json!({"query":"mutation { completeRequiredPasswordChange(input: {password: \"recovery-temporary-password\"}) { token } }"}), Some(token)).await;
            assert!(unchanged["errors"].is_array(), "{unchanged}");
            let (_, changed) = graphql_with_token(router.clone(), "/graphql", serde_json::json!({"query":"mutation { completeRequiredPasswordChange(input: {password: \"permanent-password\"}) { token passwordChangeRequired } }"}), Some(token)).await;
            assert_eq!(
                changed["data"]["completeRequiredPasswordChange"]["passwordChangeRequired"], false,
                "{changed}"
            );
            assert_eq!(
                context.app.initial_authentication_state().await.unwrap(),
                Some(SetupState::Complete)
            );
            let token = changed["data"]["completeRequiredPasswordChange"]["token"]
                .as_str()
                .unwrap();
            let (_, me) = graphql_with_token(
                router,
                "/graphql",
                serde_json::json!({"query":"{ me { username } }"}),
                Some(token),
            )
            .await;
            assert_eq!(me["data"]["me"]["username"], "recovery-admin", "{me}");
            // Explicit recovery on an already-complete installation retains its
            // existing full-session behavior, rather than reopening setup.
            let recovered_again = context
                .app
                .recover_reserved_admin_access("another-recovery-password")
                .await
                .unwrap();
            assert!(!recovered_again.password_change_required);
            assert_eq!(
                context.app.initial_authentication_state().await.unwrap(),
                Some(SetupState::Complete)
            );
        }
    }

    #[tokio::test]
    async fn initial_auth_existing_unfinished_database_is_not_reclassified_on_restart() {
        let context = fixture().await;
        sqlx::query("DELETE FROM settings_values WHERE id = 'initial-authentication-setup'")
            .execute(&context.db.pool)
            .await
            .unwrap();
        scryer_infrastructure_datastore::migrations::run_migrations(
            &context.db.pool,
            scryer_infrastructure_datastore::MigrationMode::Apply,
        )
        .await
        .unwrap();
        assert_eq!(
            context.app.initial_authentication_state().await.unwrap(),
            None
        );
        assert!(
            context
                .app
                .complete_initial_authentication(None, true)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn initial_auth_restore_replaces_destination_choice_with_backup_state() {
        use scryer_infrastructure_datastore::SqliteServices;
        use scryer_infrastructure_runtime::sqlite_backup::{
            export_backup_bundle_from_sqlite, restore_backup_bundle_into_sqlite_pool,
        };
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.db");
        let source = SqliteServices::new(source_path.to_str().unwrap())
            .await
            .unwrap();
        // An older, deliberately unauthenticated backup has no first-run state.
        sqlx::query("DELETE FROM settings_values WHERE id = 'initial-authentication-setup'")
            .execute(&source.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM settings_definitions WHERE id = 'initial-authentication-setup'")
            .execute(&source.pool)
            .await
            .unwrap();
        let bundle = directory.path().join("backup.tar.zst");
        export_backup_bundle_from_sqlite(
            source_path.to_str().unwrap(),
            scryer_application::BackupBundleExportRequest {
                output_path: bundle.clone(),
                passphrase: "fixture-passphrase".into(),
                source_migration_key: None,
                source_scryer_version: "fixture".into(),
                source_engine: "sqlite".into(),
                secrets: scryer_application::BackupExportSecrets {
                    encryption_master_key: "fixture-master-key".into(),
                    jwt_signing_secret: "fixture-jwt-secret".into(),
                    smg_registration_secret: None,
                    smg_gateway_url: None,
                },
            },
        )
        .await
        .unwrap();
        for completed in [false, true] {
            let context = fixture().await;
            if completed {
                context
                    .app
                    .complete_initial_authentication(Some("destination-password".into()), false)
                    .await
                    .unwrap();
            }
            restore_backup_bundle_into_sqlite_pool(
                &context.db.pool,
                &bundle,
                Some("fixture-passphrase"),
            )
            .await
            .unwrap();
            scryer_infrastructure_datastore::migrations::run_migrations(
                &context.db.pool,
                scryer_infrastructure_datastore::MigrationMode::Apply,
            )
            .await
            .unwrap();
            assert_eq!(
                context.app.initial_authentication_state().await.unwrap(),
                None
            );
            assert!(
                context
                    .users
                    .get_by_username("admin")
                    .await
                    .unwrap()
                    .unwrap()
                    .password_hash
                    .is_none()
            );
        }
        source.pool.close().await;
    }
}
