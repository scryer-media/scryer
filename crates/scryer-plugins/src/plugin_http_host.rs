use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use glob::Pattern;
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use scryer_application::challenge_solver as solver;

pub(crate) const HTTP_ENV_NAMESPACE: &str = "extism:host/env";
const DEFAULT_MAX_HTTP_RESPONSE_BYTES: u64 = 50 * 1024 * 1024;
type HostResult<T> = Result<T, String>;

static SHARED_PLUGIN_HTTP_RUNTIME: LazyLock<PluginHttpRuntime> =
    LazyLock::new(PluginHttpRuntime::default);

#[derive(Clone, Default)]
pub struct PluginHttpRuntime {
    state: Arc<Mutex<PluginHttpRuntimeState>>,
}

#[derive(Default)]
struct PluginHttpRuntimeState {
    extra_ca_bundle_pem: String,
    cached_client: Option<Client>,
}

pub(crate) struct PluginHttpHost {
    state: Arc<Mutex<PluginHttpHostState>>,
}

struct PluginHttpHostState {
    runtime: PluginHttpRuntime,
    allowed_hosts: Option<Vec<String>>,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
    max_http_response_bytes: Option<u64>,
    last_responses: HashMap<String, PluginHttpLastResponse>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct PluginHttpRequest {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
struct PluginHttpLastResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct IndexerProxyPolicy {
    pub indexer_id: String,
    pub indexer_name: String,
    pub config: scryer_domain::IndexerProxyConfig,
}

struct ProxiedHttpResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

pub fn shared_plugin_http_runtime() -> PluginHttpRuntime {
    SHARED_PLUGIN_HTTP_RUNTIME.clone()
}

impl scryer_application::PluginHttpTrustConfigRuntime for PluginHttpRuntime {
    fn set_plugin_http_ca_bundle_pem(
        &self,
        bundle_pem: String,
    ) -> scryer_application::AppResult<()> {
        self.set_extra_ca_bundle_pem(bundle_pem)
            .map_err(scryer_application::AppError::Repository)
    }
}

impl PluginHttpRuntime {
    pub fn set_extra_ca_bundle_pem(&self, bundle_pem: impl Into<String>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
        state.extra_ca_bundle_pem = bundle_pem.into();
        state.cached_client = None;
        Ok(())
    }

    /// Operator-trusted client for indexer-proxy endpoints (e.g. Byparr). Those
    /// targets are operator-configured, so they are not subject to the plugin
    /// egress guard and may legitimately live on the LAN.
    fn client(&self) -> HostResult<Client> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
        if let Some(client) = &state.cached_client {
            return Ok(client.clone());
        }

        let client = scryer_outbound_http::blocking_plugin_host_client(&state.extra_ca_bundle_pem)
            .map_err(|error| error.to_string())?;
        state.cached_client = Some(client.clone());
        Ok(client)
    }

    /// Builds a DNS-pinned, guarded blocking client for an untrusted
    /// plugin-controlled request URL under the plugin egress policy. A fresh
    /// client is built per request because DNS pinning is host-specific; this
    /// is what stops a plugin from reaching cloud-metadata / link-local space
    /// (even via DNS rebinding) once its `allowed_hosts` allowlist has passed.
    fn pinned_request_client(&self, url: &str) -> HostResult<Client> {
        let extra_ca_bundle_pem = {
            let state = self
                .state
                .lock()
                .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
            state.extra_ca_bundle_pem.clone()
        };
        scryer_outbound_http::prepare_plugin_blocking_http_target(
            url,
            &extra_ca_bundle_pem,
            "plugin HTTP",
        )
        .map(scryer_outbound_http::PinnedPluginBlockingHttpTarget::into_client)
        .map_err(|error| error.to_string())
    }
}

impl PluginHttpHost {
    pub(crate) fn new(
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        max_http_response_bytes: Option<u64>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(PluginHttpHostState {
                runtime: shared_plugin_http_runtime(),
                allowed_hosts: Some(allowed_hosts),
                indexer_proxy_policy,
                max_http_response_bytes,
                last_responses: HashMap::new(),
            })),
        }
    }

    pub(crate) fn request(
        &self,
        plugin_id: &str,
        request: PluginHttpRequest,
        body: Option<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> HostResult<Vec<u8>> {
        let (runtime, allowed_hosts, indexer_proxy_policy, max_http_response_bytes) = {
            let mut host_state = self
                .state
                .lock()
                .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
            host_state
                .last_responses
                .insert(plugin_id.to_string(), PluginHttpLastResponse::default());
            (
                host_state.runtime.clone(),
                host_state.allowed_hosts.clone(),
                host_state.indexer_proxy_policy.clone(),
                host_state.max_http_response_bytes,
            )
        };

        enforce_allowed_hosts(allowed_hosts.as_deref(), &request.url)?;

        // The allowlist is the primary boundary; the guarded, DNS-pinned client
        // is the second layer that keeps a declared host from reaching
        // link-local / cloud-metadata space.
        let request_client = runtime.pinned_request_client(&request.url)?;
        let started_at = Instant::now();
        let request_is_get = request
            .method
            .as_deref()
            .unwrap_or("GET")
            .eq_ignore_ascii_case("GET");
        // Reuse a previously solved clearance session for this proxy + origin so
        // repeat requests skip the solver entirely until the session goes stale.
        let session_headers = indexer_proxy_policy
            .as_ref()
            .filter(|_| request_is_get)
            .map(|policy| {
                solver::SolvedSessionCache::shared()
                    .session_headers(&policy.config.id, &request.url)
            })
            .unwrap_or_default();
        let response = execute_request_with_extra_headers(
            &request_client,
            &request,
            body.clone(),
            timeout,
            &session_headers,
        )?;
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response_headers(&response);

        if status == StatusCode::TOO_MANY_REQUESTS {
            self.store_last_response(plugin_id, status_code, headers)?;
            tracing::debug!(
                plugin_id,
                status = status_code,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                response_bytes = 0_u64,
                "plugin HTTP request skipped indexer proxy after direct rate limit"
            );
            return Ok(Vec::new());
        }

        let should_read_body = status.is_success()
            || indexer_proxy_policy.is_some() && solver::challenge_candidate_status(status_code);
        let direct_body = if should_read_body {
            read_response_body(response, max_http_response_bytes)?
        } else {
            Vec::new()
        };

        if let Some(policy) = indexer_proxy_policy.as_ref()
            && solver::looks_like_challenge_response(status_code, &headers, &direct_body)
        {
            let method = request.method.as_deref().unwrap_or("GET");
            if !method.eq_ignore_ascii_case("GET") {
                return Err(format!(
                    "indexer proxy only supports GET challenge solving for plugin HTTP requests; got {method}"
                ));
            }
            if !session_headers.is_empty() {
                // The cached session no longer clears the challenge.
                solver::SolvedSessionCache::shared().invalidate(&policy.config.id, &request.url);
            }

            tracing::debug!(
                plugin_id,
                indexer_id = policy.indexer_id.as_str(),
                indexer_name = policy.indexer_name.as_str(),
                proxy_config_id = policy.config.id.as_str(),
                status = status_code,
                request_url = solver::sanitized_url_for_log(&request.url).as_str(),
                "plugin HTTP request detected browser challenge"
            );

            // The proxy endpoint itself is operator-configured, so it uses the
            // trusted client; the plugin URL retry inside stays on the guarded
            // pinned client.
            let proxy_client = runtime.client()?;
            let solved = match execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                policy,
                &request,
                body,
                timeout,
                max_http_response_bytes,
            ) {
                Ok(solved) => {
                    solver::SolverHealthLedger::shared().record_success(&policy.config.id);
                    solved
                }
                Err(error) => {
                    if solver::is_solver_service_error_message(&error) {
                        solver::SolverHealthLedger::shared()
                            .record_failure(&policy.config.id, &error);
                    }
                    return Err(error);
                }
            };
            let response_bytes = solved.body.len();
            self.store_last_response(plugin_id, solved.status_code, solved.headers)?;
            tracing::debug!(
                plugin_id,
                indexer_id = policy.indexer_id.as_str(),
                proxy_config_id = policy.config.id.as_str(),
                status = solved.status_code,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                response_bytes,
                "plugin HTTP request completed through indexer proxy"
            );
            return Ok(solved.body);
        }

        let response_bytes = direct_body.len();
        self.store_last_response(plugin_id, status_code, headers)?;
        tracing::debug!(
            plugin_id,
            status = status_code,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            response_bytes,
            "plugin HTTP request completed"
        );
        if status.is_success() {
            Ok(direct_body)
        } else {
            Ok(Vec::new())
        }
    }

    pub(crate) fn status_code(&self, plugin_id: &str) -> HostResult<u16> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .map(|response| response.status_code)
            .unwrap_or(0))
    }

    pub(crate) fn headers(&self, plugin_id: &str) -> HostResult<Option<BTreeMap<String, String>>> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .filter(|response| !response.headers.is_empty())
            .map(|response| response.headers.clone()))
    }

    fn store_last_response(
        &self,
        plugin_id: &str,
        status_code: u16,
        headers: BTreeMap<String, String>,
    ) -> HostResult<()> {
        let mut host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        host_state.last_responses.insert(
            plugin_id.to_string(),
            PluginHttpLastResponse {
                status_code,
                headers,
            },
        );
        Ok(())
    }
}

fn enforce_allowed_hosts(allowed_hosts: Option<&[String]>, request_url: &str) -> HostResult<()> {
    let url = url::Url::parse(request_url).map_err(|error| format!("Invalid URL: {error:?}"))?;
    let host = url.host_str().unwrap_or_default();
    let matches = allowed_hosts.is_some_and(|patterns| {
        patterns.iter().any(|pattern| {
            Pattern::new(pattern)
                .map(|compiled| compiled.matches(host))
                .unwrap_or_else(|_| pattern == host)
        })
    });

    if matches {
        return Ok(());
    }

    // Never interpolate the raw request URL here: for indexer requests it
    // carries `?apikey=`/`passkey=` credentials that would otherwise leak into
    // WARN logs and the user-facing error. Log the query-stripped URL instead.
    Err(format!(
        "HTTP request to {} is not allowed",
        solver::sanitized_url_for_log(request_url)
    ))
}

fn merge_cookie_headers(original: Option<&str>, solved: &str) -> Option<String> {
    let mut cookies: Vec<(String, String)> = Vec::new();
    for header in original.into_iter().chain(std::iter::once(solved)) {
        for raw_pair in header.split(';') {
            let pair = raw_pair.trim();
            if !solver::safe_cookie_pair(pair) {
                continue;
            }
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if let Some(existing) = cookies.iter_mut().find(|(existing, _)| existing == &name) {
                existing.1 = value;
            } else {
                cookies.push((name, value));
            }
        }
    }
    (!cookies.is_empty()).then(|| {
        cookies
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

fn execute_request_with_extra_headers(
    client: &Client,
    request: &PluginHttpRequest,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
    extra_headers: &[(String, String)],
) -> HostResult<reqwest::blocking::Response> {
    let method = Method::from_bytes(
        request
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_uppercase()
            .as_bytes(),
    )
    .map_err(|error| format!("Invalid HTTP method: {error}"))?;

    let solved_cookie = extra_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.as_str());
    let original_cookie = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.as_str());
    let merged_cookie =
        solved_cookie.and_then(|solved| merge_cookie_headers(original_cookie, solved));

    let mut builder = client.request(method, &request.url);
    for (name, value) in &request.headers {
        // Solver-session headers replace matching plugin headers, except Cookie:
        // preserve unrelated indexer cookies and overlay solved cookie names.
        if extra_headers
            .iter()
            .any(|(extra, _)| extra.eq_ignore_ascii_case(name))
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    for (name, value) in extra_headers {
        if name.eq_ignore_ascii_case("cookie") {
            if let Some(value) = merged_cookie.as_deref() {
                builder = builder.header(name, value);
            }
        } else {
            builder = builder.header(name, value);
        }
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }

    scryer_outbound_http::send_blocking_reqwest_request_with_cooldown_budget(builder, timeout)
        .map_err(|error| match error {
            scryer_outbound_http::BlockingOutboundHttpError::Request(error)
                if error.is_timeout() =>
            {
                "timeout".to_string()
            }
            other => other.to_string(),
        })
}

fn response_headers(response: &reqwest::blocking::Response) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str().to_string(), value.to_string());
        }
    }
    headers
}

fn read_response_body(
    response: reqwest::blocking::Response,
    max_http_response_bytes: Option<u64>,
) -> HostResult<Vec<u8>> {
    let mut body = Vec::new();
    let max = max_http_response_bytes.unwrap_or(DEFAULT_MAX_HTTP_RESPONSE_BYTES);
    response
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() > max as usize {
        return Err(format!(
            "HTTP response exceeds the configured maximum number of bytes: {max}"
        ));
    }
    Ok(body)
}

fn execute_challenge_solver_request(
    proxy_client: &Client,
    request_client: &Client,
    policy: &IndexerProxyPolicy,
    request: &PluginHttpRequest,
    original_body: Option<Vec<u8>>,
    original_timeout: Option<Duration>,
    max_http_response_bytes: Option<u64>,
) -> HostResult<ProxiedHttpResponse> {
    if !policy.config.is_enabled {
        return Err("Indexer proxy is disabled for this indexer.".to_string());
    }

    let provider = policy.config.provider_type;
    let provider_name = solver::solver_provider_name(provider);
    let endpoint = solver::solver_solve_endpoint(&policy.config.base_url);
    let solver_timeout = Duration::from_secs(policy.config.request_timeout_seconds as u64 + 5);
    tracing::debug!(
        indexer_id = policy.indexer_id.as_str(),
        indexer_name = policy.indexer_name.as_str(),
        proxy_config_id = policy.config.id.as_str(),
        proxy_provider = policy.config.provider_type.as_str(),
        request_url = solver::sanitized_url_for_log(&request.url).as_str(),
        "challenge solver request started"
    );

    let response = proxy_client
        .post(&endpoint)
        .timeout(solver_timeout)
        .json(&solver::solver_solve_request(
            provider,
            &request.url,
            policy.config.request_timeout_seconds,
        ))
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                solver::solver_error_message(provider, solver::SolverErrorKind::Timeout).to_string()
            } else {
                solver::solver_error_message(provider, solver::SolverErrorKind::Unreachable)
                    .to_string()
            }
        })?;

    let solver_status = response.status();
    if solver_status == StatusCode::TOO_MANY_REQUESTS || solver_status.is_server_error() {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solver_status.as_u16(),
            "challenge solver service unavailable for indexer proxy request"
        );
        return Err(
            solver::solver_error_message(provider, solver::SolverErrorKind::Unavailable)
                .to_string(),
        );
    }

    let response_body = read_response_body(response, max_http_response_bytes)?;
    let solution = solver::parse_solver_solution(&response_body)
        .map_err(|error| error.message(provider).to_string())?;

    let solution_status = solution.status.unwrap_or_else(|| solver_status.as_u16());
    let solved_final_url = solution.url.as_deref().map(solver::sanitized_url_for_log);
    if solution_status == StatusCode::TOO_MANY_REQUESTS.as_u16() {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            "challenge solver reported target indexer rate limit"
        );
        return Err(solver::target_rate_limit_message(&solution));
    }

    let solved_body = solution.response.clone().unwrap_or_default().into_bytes();
    if solver::solved_body_looks_rate_limited(&solved_body) {
        return Err(solver::target_rate_limit_message(&solution));
    }
    if (200..300).contains(&solution_status) && !solved_body.is_empty() {
        // Cache the clearance session so follow-up requests to this origin skip
        // the solver until the session expires or stops clearing challenges.
        solver::SolvedSessionCache::shared().store_solution(
            &policy.config.id,
            &request.url,
            &solution,
        );
        tracing::debug!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            response_bytes = solved_body.len(),
            final_url = solved_final_url.as_deref(),
            "challenge solver response used"
        );
        return Ok(ProxiedHttpResponse {
            status_code: solution_status,
            headers: solver::safe_solution_response_headers(solution.headers.as_ref()),
            body: solved_body,
        });
    }

    let retry_headers = solver::solution_retry_headers(&solution);
    if !retry_headers.is_empty() {
        tracing::debug!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            "retrying original request with challenge solver headers"
        );
        let retry = execute_request_with_extra_headers(
            request_client,
            request,
            original_body,
            original_timeout,
            &retry_headers,
        )?;
        let status = retry.status();
        let headers = response_headers(&retry);
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = solver::header_value(&headers, "retry-after").and_then(|value| {
                scryer_outbound_http::parse_retry_after(value).map(|(delay, _)| delay)
            });
            return Err(solver::rate_limit_message_with_retry_after(retry_after));
        }
        if !status.is_success() {
            return Err(solver::solver_error_message(
                provider,
                solver::SolverErrorKind::MissingSolution,
            )
            .to_string());
        }
        let body = read_response_body(retry, max_http_response_bytes)?;
        solver::SolvedSessionCache::shared().store_solution(
            &policy.config.id,
            &request.url,
            &solution,
        );
        return Ok(ProxiedHttpResponse {
            status_code: status.as_u16(),
            headers,
            body,
        });
    }

    if !(200..300).contains(&solution_status) {
        return Err(format!(
            "{provider_name} target request returned HTTP {solution_status}."
        ));
    }

    Err(
        solver::solver_error_message(provider, solver::SolverErrorKind::MissingSolution)
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );

    #[test]
    fn build_plugin_http_client_accepts_empty_trust_bundle() {
        scryer_outbound_http::blocking_plugin_host_client("")
            .expect("default trust bundle should build");
    }

    #[test]
    fn build_plugin_http_client_accepts_uploaded_certificates() {
        scryer_outbound_http::blocking_plugin_host_client(TEST_PLUGIN_HTTP_CA_CERT_PEM)
            .expect("uploaded trust bundle should build");
    }

    #[test]
    fn add_uploaded_certificates_rejects_non_certificate_pem_items() {
        let error = scryer_outbound_http::blocking_plugin_host_client(
            "-----BEGIN PRIVATE KEY-----\nZmFrZQ==\n-----END PRIVATE KEY-----\n",
        )
        .expect_err("non-certificate bundle should be rejected");

        assert!(error.contains("uploaded trusted certificate bundle"));
    }

    #[test]
    fn solved_cookies_overlay_matching_names_and_preserve_original_cookies() {
        let merged = merge_cookie_headers(
            Some("auth=secret; cf_clearance=stale; theme=dark"),
            "cf_clearance=fresh; bot_session=ready",
        );

        assert_eq!(
            merged.as_deref(),
            Some("auth=secret; cf_clearance=fresh; theme=dark; bot_session=ready")
        );
    }

    #[test]
    fn enforce_allowed_hosts_rejects_disallowed_hosts() {
        let error = enforce_allowed_hosts(
            Some(&["example.com".to_string()]),
            "https://jimaku.example.test/search",
        )
        .expect_err("disallowed host should fail");

        assert!(error.to_string().contains("is not allowed"));
    }

    #[test]
    fn enforce_allowed_hosts_error_omits_query_credentials() {
        let error = enforce_allowed_hosts(
            Some(&["allowed.example".to_string()]),
            "https://tracker.example.test/download?apikey=SECRETKEY&passkey=TOPSECRET",
        )
        .expect_err("disallowed host should fail");

        assert!(error.contains("is not allowed"));
        assert!(
            !error.contains('?'),
            "error must not carry a query string: {error}"
        );
        assert!(
            !error.contains("apikey"),
            "error must not leak apikey: {error}"
        );
        assert!(
            !error.contains("passkey"),
            "error must not leak passkey: {error}"
        );
        assert!(
            !error.contains("SECRETKEY"),
            "error must not leak secrets: {error}"
        );
        assert!(
            !error.contains("TOPSECRET"),
            "error must not leak secrets: {error}"
        );
    }

    #[test]
    fn plugin_request_egress_blocks_cloud_metadata() {
        let result = scryer_outbound_http::prepare_plugin_blocking_http_target(
            "http://169.254.169.254/latest/meta-data/",
            "",
            "plugin HTTP",
        );

        assert!(
            matches!(
                result,
                Err(
                    scryer_outbound_http::OutboundDestinationError::BlockedLinkLocalOrMetadata { .. }
                )
            ),
            "cloud metadata address must be rejected on the plugin HTTP host path"
        );
    }

    #[test]
    fn plugin_request_egress_allows_loopback_companion() {
        scryer_outbound_http::prepare_plugin_blocking_http_target(
            "http://127.0.0.1:9117/api",
            "",
            "plugin HTTP",
        )
        .expect("loopback companion must be allowed for self-hosted plugins");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trawl_solver_response_is_used_for_plugin_indexer_request() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = "https://indexer.example/api?t=search";
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{ "name": "cf_clearance", "value": "abc" }],
                    "userAgent": "Trawl UA",
                    "response": "<html>Trawl</html>"
                }
            })))
            .mount(&server)
            .await;

        let now = chrono::Utc::now();
        let policy = IndexerProxyPolicy {
            indexer_id: "indexer-1".into(),
            indexer_name: "Indexer".into(),
            config: scryer_domain::IndexerProxyConfig {
                id: "trawl-1".into(),
                name: "Trawl".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 60,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            },
        };
        let request = PluginHttpRequest {
            url: target_url.into(),
            method: Some("GET".into()),
            headers: BTreeMap::new(),
        };

        let solved = tokio::task::spawn_blocking(move || {
            let proxy_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("proxy client");
            let request_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("request client");
            execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                &policy,
                &request,
                None,
                None,
                Some(1024 * 1024),
            )
        })
        .await
        .expect("solver task should join")
        .expect("Trawl plugin solve should succeed");

        assert_eq!(solved.status_code, 200);
        assert_eq!(solved.body, b"<html>Trawl</html>");
        assert!(solved.headers.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn byparr_non_success_solution_refetches_with_clearance_session() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/api", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 503,
                    "headers": { "content-type": "text/html" },
                    "cookies": [{ "name": "cf_clearance", "value": "abc" }],
                    "userAgent": "Byparr UA",
                    "response": "<html>Just a moment</html>"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(header("cookie", "cf_clearance=abc"))
            .and(header("user-agent", "Byparr UA"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/xml")
                    .set_body_bytes(b"<rss></rss>"),
            )
            .mount(&server)
            .await;

        let now = chrono::Utc::now();
        let policy = IndexerProxyPolicy {
            indexer_id: "indexer-1".into(),
            indexer_name: "Indexer".into(),
            config: scryer_domain::IndexerProxyConfig {
                id: "byparr-1".into(),
                name: "Byparr".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 60,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            },
        };
        let request = PluginHttpRequest {
            url: target_url,
            method: Some("GET".into()),
            headers: BTreeMap::new(),
        };

        let solved = tokio::task::spawn_blocking(move || {
            let proxy_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("proxy client");
            let request_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("request client");
            execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                &policy,
                &request,
                None,
                None,
                Some(1024 * 1024),
            )
        })
        .await
        .expect("solver task should join")
        .expect("Byparr session refetch should succeed");

        assert_eq!(solved.status_code, 200);
        assert_eq!(solved.body, b"<rss></rss>");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trawl_server_error_is_classified_as_solver_unavailable() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = "https://indexer.example/api?t=search";
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
                "status": "error",
                "message": "Browser pool initializing, retry in a few seconds",
                "solution": {
                    "url": target_url,
                    "status": 0,
                    "headers": {},
                    "response": "",
                    "cookies": [],
                    "userAgent": ""
                }
            })))
            .mount(&server)
            .await;

        let now = chrono::Utc::now();
        let policy = IndexerProxyPolicy {
            indexer_id: "indexer-1".into(),
            indexer_name: "Indexer".into(),
            config: scryer_domain::IndexerProxyConfig {
                id: "trawl-unavailable".into(),
                name: "Trawl".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 60,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            },
        };
        let request = PluginHttpRequest {
            url: target_url.into(),
            method: Some("GET".into()),
            headers: BTreeMap::new(),
        };

        let result = tokio::task::spawn_blocking(move || {
            let proxy_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("proxy client");
            let request_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("request client");
            execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                &policy,
                &request,
                None,
                None,
                Some(1024 * 1024),
            )
        })
        .await
        .expect("solver task should join");
        let error = match result {
            Ok(_) => panic!("Trawl server errors must fail as solver unavailable"),
            Err(error) => error,
        };

        assert_eq!(error, solver::TRAWL_UNAVAILABLE_MESSAGE);
    }
}
