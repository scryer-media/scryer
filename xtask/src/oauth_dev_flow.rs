use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::Args;
use serde::Deserialize;
// OAuth PKCE S256 challenge, fixed by RFC 7636.
#[allow(clippy::disallowed_types)]
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};
use url::Url;
use uuid::Uuid;
use xtask_support::{TaskContext, ok, step, warn};

const DEFAULT_FRONTEND_URL: &str = "http://localhost:3000";
const DEFAULT_CLIENT_ID: &str = "generic-native";
const DEFAULT_CALLBACK_BIND: &str = "127.0.0.1:18787";
const DEFAULT_CALLBACK_PATH: &str = "/callback";
const DEFAULT_SCOPE: &str = "library";
const DEFAULT_TIMEOUT_SECONDS: u64 = 300;
#[cfg(test)]
const TEST_BACKEND_URL: &str = "http://127.0.0.1:18080";

#[derive(Args, Clone)]
pub struct OAuthDevFlowArgs {
    #[arg(long, default_value = DEFAULT_FRONTEND_URL)]
    frontend_url: String,
    #[arg(
        long,
        value_name = "BACKEND_URL",
        help = "Backend origin override for token/revoke calls; defaults to OAuth metadata from the frontend proxy"
    )]
    backend_url: Option<String>,
    #[arg(long, default_value = DEFAULT_CLIENT_ID)]
    client_id: String,
    #[arg(long, default_value = DEFAULT_CALLBACK_BIND)]
    callback_bind: String,
    #[arg(long, default_value = DEFAULT_CALLBACK_PATH)]
    callback_path: String,
    #[arg(long, default_value = DEFAULT_SCOPE)]
    scope: String,
    #[arg(long)]
    no_open: bool,
    #[arg(long)]
    print_url: bool,
    #[arg(long)]
    keep_grant: bool,
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_SECONDS)]
    timeout_seconds: u64,
}

struct OAuthDevFlowConfig {
    frontend_url: Url,
    backend_url: Option<Url>,
    client_id: String,
    callback_bind: String,
    callback_path: String,
    redirect_uri: String,
    scope: String,
    no_open: bool,
    print_url: bool,
    keep_grant: bool,
    timeout: Duration,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    expires_in: i64,
    refresh_token: String,
    scope: String,
    token_type: String,
}

#[derive(Debug, Deserialize)]
struct OAuthMetadataResponse {
    authorization_endpoint: String,
    revocation_endpoint: String,
    token_endpoint: String,
}

#[derive(Debug)]
struct OAuthDevFlowEndpoints {
    revocation_endpoint: Url,
    token_endpoint: Url,
}

#[derive(Debug)]
struct CallbackRequest {
    path: String,
    query: HashMap<String, String>,
}

struct CallbackExchange {
    stream: TcpStream,
    request: CallbackRequest,
}

struct HttpResponse {
    status: u16,
    body: String,
}

pub fn run(_ctx: &TaskContext, args: OAuthDevFlowArgs) -> Result<()> {
    let config = OAuthDevFlowConfig::from_args(args)?;
    let endpoints = preflight(&config)?;

    let verifier = generate_pkce_verifier();
    let challenge = pkce_s256_challenge(&verifier);
    let state = Uuid::new_v4().to_string();
    let listener = TcpListener::bind(&config.callback_bind).with_context(|| {
        format!(
            "failed to bind callback listener on {}",
            config.callback_bind
        )
    })?;
    listener
        .set_nonblocking(true)
        .context("failed to configure callback listener")?;

    let authorize_url = build_authorize_url(&config, &challenge, &state)?;
    println!("==> OAuth dev flow");
    println!("    Frontend: {}", config.frontend_url);
    if let Some(backend_url) = &config.backend_url {
        println!("    Backend override: {backend_url}");
    } else {
        println!("    OAuth API: {}", endpoints.token_endpoint);
    }
    println!("    Client:   {}", config.client_id);
    println!("    Callback: {}", config.redirect_uri);
    if config.print_url || config.no_open {
        println!("    URL:      {authorize_url}");
    }

    if !config.no_open {
        match open_browser(authorize_url.as_str()) {
            Ok(()) => ok("Opened OAuth authorization page"),
            Err(error) => {
                warn(format!("Could not open browser automatically: {error}"));
                println!("    Open manually: {authorize_url}");
            }
        }
    }

    step(format!(
        "Waiting up to {}s for OAuth callback",
        config.timeout.as_secs()
    ));
    let mut callback = wait_for_callback(&listener, config.timeout)
        .context("OAuth callback was not received; authorize or deny the request in the browser")?;
    let callback_request = &callback.request;

    if callback_request.path != config.callback_path {
        respond_callback_error(
            &mut callback.stream,
            "Unexpected callback path",
            &format!(
                "Expected {}, got {}",
                config.callback_path, callback_request.path
            ),
        )?;
        bail!(
            "unexpected callback path: expected {}, got {}",
            config.callback_path,
            callback_request.path
        );
    }

    if let Some(error) = callback_request.query.get("error") {
        let description = callback_request
            .query
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| "OAuth authorization returned an error".to_string());
        respond_callback_error(
            &mut callback.stream,
            "OAuth authorization ended",
            &description,
        )?;
        ok(format!("OAuth flow ended with {error}: {description}"));
        return Ok(());
    }

    let Some(received_state) = callback_request.query.get("state") else {
        respond_callback_error(
            &mut callback.stream,
            "OAuth callback missing state",
            "The callback did not include a state parameter.",
        )?;
        bail!("OAuth callback did not include state");
    };
    if received_state != &state {
        respond_callback_error(
            &mut callback.stream,
            "OAuth state mismatch",
            "The callback state did not match the request state.",
        )?;
        bail!("OAuth callback state mismatch");
    }

    let Some(code) = callback_request
        .query
        .get("code")
        .filter(|value| !value.is_empty())
    else {
        respond_callback_error(
            &mut callback.stream,
            "OAuth callback missing code",
            "The callback did not include an authorization code.",
        )?;
        bail!("OAuth callback did not include an authorization code");
    };

    let token = match exchange_code(&config, &endpoints, code, &verifier) {
        Ok(token) => token,
        Err(error) => {
            respond_callback_error(
                &mut callback.stream,
                "OAuth token exchange failed",
                &error.to_string(),
            )?;
            return Err(error);
        }
    };
    if !token.token_type.eq_ignore_ascii_case("bearer") {
        respond_callback_error(
            &mut callback.stream,
            "OAuth token exchange failed",
            "The token endpoint returned an unsupported token type.",
        )?;
        bail!("OAuth token endpoint returned unsupported token_type");
    }
    if token.access_token.trim().is_empty() || token.refresh_token.trim().is_empty() {
        respond_callback_error(
            &mut callback.stream,
            "OAuth token exchange failed",
            "The token endpoint returned an empty token.",
        )?;
        bail!("OAuth token endpoint returned an empty token");
    }
    respond_callback_success(&mut callback.stream, &config, &token)?;
    ok(format!(
        "OAuth authorization code exchanged successfully (scope={}, expires_in={}s)",
        token.scope, token.expires_in
    ));

    if config.keep_grant {
        println!("==> Grant kept for visual inspection in Profile > Connected apps");
    } else {
        revoke_refresh_token(&endpoints, &token.refresh_token)?;
        ok("OAuth refresh token revoked");
    }

    Ok(())
}

impl OAuthDevFlowConfig {
    fn from_args(args: OAuthDevFlowArgs) -> Result<Self> {
        let frontend_url = normalize_origin(&args.frontend_url, "--frontend-url")?;
        let backend_url = args
            .backend_url
            .as_deref()
            .map(|value| normalize_origin(value, "--backend-url"))
            .transpose()?;
        let callback_path = normalize_callback_path(&args.callback_path);
        let redirect_uri = format!("http://{}{}", args.callback_bind, callback_path);
        Ok(Self {
            frontend_url,
            backend_url,
            client_id: args.client_id.trim().to_string(),
            callback_bind: args.callback_bind.trim().to_string(),
            callback_path,
            redirect_uri,
            scope: args.scope.trim().to_string(),
            no_open: args.no_open,
            print_url: args.print_url,
            keep_grant: args.keep_grant,
            timeout: Duration::from_secs(args.timeout_seconds),
        })
    }
}

fn preflight(config: &OAuthDevFlowConfig) -> Result<OAuthDevFlowEndpoints> {
    step("Checking frontend OAuth route");
    let authorize_url = join_url(&config.frontend_url, "/oauth/authorize")?;
    let authorize = http_get(&authorize_url).with_context(|| {
        format!(
            "Scryer frontend was not reachable at {}. Start cargo xtask serve first.",
            config.frontend_url
        )
    })?;
    if authorize.status != 200 {
        bail!(
            "Scryer frontend is not serving /oauth/authorize at {}. Start cargo xtask serve first.",
            config.frontend_url
        );
    }

    step("Checking frontend OAuth API proxy");
    let metadata_url = join_url(
        &config.frontend_url,
        "/.well-known/oauth-authorization-server",
    )?;
    let metadata = http_get(&metadata_url).with_context(|| {
        format!(
            "OAuth metadata was not reachable through {}. Restart cargo xtask serve after updating the Vite proxy.",
            config.frontend_url
        )
    })?;
    if metadata.status != 200 {
        bail!(
            "OAuth metadata was not reachable through {}. Restart cargo xtask serve after updating the Vite proxy.",
            config.frontend_url
        );
    }
    let metadata: OAuthMetadataResponse = serde_json::from_str(&metadata.body)
        .context("OAuth metadata response was not valid JSON")?;
    resolve_oauth_endpoints(config, &metadata)
}

fn resolve_oauth_endpoints(
    config: &OAuthDevFlowConfig,
    metadata: &OAuthMetadataResponse,
) -> Result<OAuthDevFlowEndpoints> {
    let authorization_endpoint =
        parse_metadata_url(&metadata.authorization_endpoint, "authorization_endpoint")?;
    let expected_authorization_endpoint = join_url(&config.frontend_url, "/oauth/authorize")?;
    if authorization_endpoint != expected_authorization_endpoint {
        bail!(
            "OAuth metadata authorization_endpoint was {}, expected {}. Restart cargo xtask serve after updating the Vite proxy.",
            authorization_endpoint,
            expected_authorization_endpoint
        );
    }

    if let Some(backend_url) = &config.backend_url {
        return Ok(OAuthDevFlowEndpoints {
            revocation_endpoint: join_url(backend_url, "/oauth/revoke")?,
            token_endpoint: join_url(backend_url, "/oauth/token")?,
        });
    }

    Ok(OAuthDevFlowEndpoints {
        revocation_endpoint: parse_metadata_url(
            &metadata.revocation_endpoint,
            "revocation_endpoint",
        )?,
        token_endpoint: parse_metadata_url(&metadata.token_endpoint, "token_endpoint")?,
    })
}

fn parse_metadata_url(value: &str, name: &str) -> Result<Url> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("OAuth metadata response did not include {name}");
    }
    let url = Url::parse(trimmed).with_context(|| format!("{name} must be a URL"))?;
    if url.scheme() != "http" {
        bail!("{name} must be an http:// URL for local OAuth dev flow");
    }
    Ok(url)
}

fn build_authorize_url(
    config: &OAuthDevFlowConfig,
    code_challenge: &str,
    state: &str,
) -> Result<Url> {
    let mut url = join_url(&config.frontend_url, "/oauth/authorize")?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &config.redirect_uri)
        .append_pair("scope", &config.scope)
        .append_pair("state", state)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

fn generate_pkce_verifier() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

// OAuth PKCE S256 challenge, fixed by RFC 7636.
#[allow(clippy::disallowed_types)]
fn pkce_s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn wait_for_callback(listener: &TcpListener, timeout: Duration) -> Result<CallbackExchange> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_callback_request(&mut stream)?;
                let callback = parse_callback_request(&request)?;
                return Ok(CallbackExchange {
                    stream,
                    request: callback,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    bail!("timed out waiting for callback");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error).context("failed while waiting for callback"),
        }
    }
}

fn read_callback_request(stream: &mut TcpStream) -> Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = [0_u8; 4096];
    let mut request = Vec::new();
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 16 * 1024 {
            bail!("callback request headers were too large");
        }
    }
    String::from_utf8(request).context("callback request was not UTF-8")
}

fn parse_callback_request(request: &str) -> Result<CallbackRequest> {
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow!("callback request was empty"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        bail!("callback used unsupported HTTP method {method}");
    }
    let url = Url::parse(&format!("http://callback.local{target}"))
        .context("callback request target was not a valid URL path")?;
    let query = url
        .query_pairs()
        .into_owned()
        .collect::<HashMap<String, String>>();
    Ok(CallbackRequest {
        path: url.path().to_string(),
        query,
    })
}

fn exchange_code(
    config: &OAuthDevFlowConfig,
    endpoints: &OAuthDevFlowEndpoints,
    code: &str,
    verifier: &str,
) -> Result<OAuthTokenResponse> {
    step("Exchanging authorization code");
    let form = form_body(&[
        ("grant_type", "authorization_code"),
        ("client_id", &config.client_id),
        ("code", code),
        ("redirect_uri", &config.redirect_uri),
        ("code_verifier", verifier),
    ]);
    let response = http_post_form(&endpoints.token_endpoint, &form).with_context(|| {
        format!(
            "OAuth token endpoint was not reachable at {}. Start cargo xtask serve first.",
            endpoints.token_endpoint
        )
    })?;
    if response.status != 200 {
        bail!("OAuth token exchange failed with HTTP {}", response.status);
    }
    serde_json::from_str(&response.body).context("OAuth token response was not valid JSON")
}

fn revoke_refresh_token(endpoints: &OAuthDevFlowEndpoints, refresh_token: &str) -> Result<()> {
    step("Revoking OAuth refresh token");
    let form = form_body(&[
        ("token", refresh_token),
        ("token_type_hint", "refresh_token"),
    ]);
    let response = http_post_form(&endpoints.revocation_endpoint, &form).with_context(|| {
        format!(
            "OAuth revocation endpoint was not reachable at {}. Start cargo xtask serve first.",
            endpoints.revocation_endpoint
        )
    })?;
    if response.status != 200 {
        bail!("OAuth revocation failed with HTTP {}", response.status);
    }
    Ok(())
}

fn respond_callback_success(
    stream: &mut TcpStream,
    config: &OAuthDevFlowConfig,
    token: &OAuthTokenResponse,
) -> Result<()> {
    let (metadata_line, cleanup_line) = callback_success_log_lines(config, token);
    println!("{metadata_line}");
    println!("{cleanup_line}");
    let body = callback_success_page(config, token);
    stream.write_all(http_html_response(&body).as_bytes())?;
    Ok(())
}

fn callback_success_log_lines(
    config: &OAuthDevFlowConfig,
    token: &OAuthTokenResponse,
) -> (String, String) {
    (
        format!(
            "==> Browser callback received; token metadata: scope={}, expires_in={}s",
            token.scope, token.expires_in
        ),
        format!(
            "    Refresh token will {}be kept.",
            if config.keep_grant { "" } else { "not " }
        ),
    )
}

fn callback_success_page(config: &OAuthDevFlowConfig, token: &OAuthTokenResponse) -> String {
    let cleanup = if config.keep_grant {
        "The refresh grant was kept so you can inspect it in Profile > Connected apps."
    } else {
        "The harness will revoke the refresh grant before it exits."
    };
    callback_page(
        "OAuth authorization complete",
        &format!(
            "Client {} received scope {}. Access token expires in {} seconds. {cleanup}",
            config.client_id, token.scope, token.expires_in
        ),
    )
}

fn respond_callback_error(stream: &mut TcpStream, title: &str, message: &str) -> Result<()> {
    let body = callback_page(title, message);
    stream.write_all(http_html_response(&body).as_bytes())?;
    Ok(())
}

fn callback_page(title: &str, message: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>{}</title>
  <style>
    body {{ font-family: system-ui, sans-serif; margin: 3rem; max-width: 44rem; line-height: 1.45; }}
    code {{ background: #f2f2f2; border-radius: 4px; padding: 0.1rem 0.25rem; }}
  </style>
</head>
<body>
  <h1>{}</h1>
  <p>{}</p>
  <p>No OAuth codes, access tokens, or refresh tokens were printed.</p>
</body>
</html>"#,
        escape_html(title),
        escape_html(title),
        escape_html(message)
    )
}

fn http_html_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn http_get(url: &Url) -> Result<HttpResponse> {
    http_request("GET", url, &[], "")
}

fn http_post_form(url: &Url, body: &str) -> Result<HttpResponse> {
    http_request(
        "POST",
        url,
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body,
    )
}

fn http_request(
    method: &str,
    url: &Url,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<HttpResponse> {
    if url.scheme() != "http" {
        bail!("only http:// URLs are supported for local OAuth dev flow");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("URL is missing host: {url}"))?;
    let port = url.port_or_known_default().unwrap_or(80);
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("failed to connect to {host}:{port}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let path = if let Some(query) = url.query() {
        format!("{}?{query}", url.path())
    } else {
        url.path().to_string()
    };
    let host_header = if url.port().is_some() {
        format!("{host}:{port}")
    } else {
        host.to_string()
    };
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_header}\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes())?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    parse_http_response(&response)
}

fn parse_http_response(response: &str) -> Result<HttpResponse> {
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow!("invalid HTTP response"))?;
    let mut lines = headers.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| anyhow!("HTTP response missing status line"))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("HTTP response missing status code"))?
        .parse::<u16>()
        .context("HTTP response status code was invalid")?;
    let mut parsed_headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            parsed_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let body = if parsed_headers
        .get("transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        decode_chunked_http_body(body).ok_or_else(|| anyhow!("invalid chunked HTTP response"))?
    } else {
        body.to_string()
    };
    Ok(HttpResponse { status, body })
}

fn decode_chunked_http_body(body: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut rest = body;
    loop {
        let (size_line, after_size_line) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size_line.trim(), 16).ok()?;
        if size == 0 {
            return Some(decoded);
        }
        if after_size_line.len() < size + 2 {
            return None;
        }
        decoded.push_str(&after_size_line[..size]);
        rest = &after_size_line[size + 2..];
    }
}

fn form_body(fields: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

fn join_url(origin: &Url, path: &str) -> Result<Url> {
    let mut url = origin.clone();
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn normalize_origin(value: &str, name: &str) -> Result<Url> {
    let trimmed = value.trim().trim_end_matches('/');
    let url = Url::parse(trimmed).with_context(|| format!("{name} must be a URL"))?;
    if url.scheme() != "http" {
        bail!("{name} must be an http:// URL for local OAuth dev flow");
    }
    Ok(url)
}

fn normalize_callback_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let mut command = {
        let mut command = Command::new("false");
        command
    };

    let status = command.status().context("failed to run browser opener")?;
    if !status.success() {
        bail!("browser opener exited with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_and_challenge_are_base64url_without_padding() {
        let verifier = generate_pkce_verifier();
        let challenge = pkce_s256_challenge(&verifier);

        assert_eq!(verifier.len(), 43);
        assert_eq!(challenge.len(), 43);
        assert!(verifier.bytes().all(is_base64url_byte));
        assert!(challenge.bytes().all(is_base64url_byte));
        assert!(!verifier.contains('='));
        assert!(!challenge.contains('='));
    }

    #[test]
    fn callback_request_parser_extracts_path_and_query() {
        let request = "GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let callback = parse_callback_request(request).expect("callback");

        assert_eq!(callback.path, "/callback");
        assert_eq!(callback.query.get("code").map(String::as_str), Some("abc"));
        assert_eq!(callback.query.get("state").map(String::as_str), Some("xyz"));
    }

    #[test]
    fn authorize_url_uses_frontend_origin_and_never_contains_verifier() {
        let config = test_config(Some(TEST_BACKEND_URL));
        let url = build_authorize_url(&config, "challenge", "state").expect("url");

        assert!(!url.as_str().contains("code_verifier"));
        assert_eq!(url.host_str(), Some("localhost"));
        assert!(url.as_str().contains("client_id=generic-native"));
        assert!(url.as_str().contains("code_challenge=challenge"));
    }

    #[test]
    fn callback_page_escapes_html() {
        let page = callback_page("<title>", "message & more");

        assert!(page.contains("&lt;title&gt;"));
        assert!(page.contains("message &amp; more"));
    }

    #[test]
    fn callback_success_page_never_renders_token_values() {
        let config = test_config(Some(TEST_BACKEND_URL));
        let token = test_token();

        let page = callback_success_page(&config, &token);

        assert!(page.contains("expires in 3600 seconds"));
        assert!(!page.contains("secret-access-token"));
        assert!(!page.contains("secret-refresh-token"));
    }

    #[test]
    fn callback_success_log_lines_never_render_secrets() {
        let config = test_config(Some(TEST_BACKEND_URL));
        let token = test_token();
        let verifier = "secret-pkce-verifier";

        let (metadata_line, cleanup_line) = callback_success_log_lines(&config, &token);
        let output = format!("{metadata_line}\n{cleanup_line}");

        assert!(output.contains("scope=library"));
        assert!(!output.contains("secret-access-token"));
        assert!(!output.contains("secret-refresh-token"));
        assert!(!output.contains(verifier));
    }

    #[test]
    fn endpoints_default_to_metadata_urls() {
        let config = test_config(None);
        let endpoints = resolve_oauth_endpoints(&config, &test_metadata()).expect("endpoints");

        assert_eq!(
            endpoints.token_endpoint.as_str(),
            "http://localhost:3000/oauth/token"
        );
        assert_eq!(
            endpoints.revocation_endpoint.as_str(),
            "http://localhost:3000/oauth/revoke"
        );
    }

    #[test]
    fn explicit_backend_url_overrides_metadata_token_and_revoke_urls() {
        let config = test_config(Some(TEST_BACKEND_URL));
        let endpoints = resolve_oauth_endpoints(&config, &test_metadata()).expect("endpoints");

        assert_eq!(
            endpoints.token_endpoint.as_str(),
            "http://127.0.0.1:18080/oauth/token"
        );
        assert_eq!(
            endpoints.revocation_endpoint.as_str(),
            "http://127.0.0.1:18080/oauth/revoke"
        );
    }

    #[test]
    fn preflight_connection_failure_includes_serve_guidance() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        let frontend_url = format!("http://127.0.0.1:{port}");
        let config = OAuthDevFlowConfig::from_args(OAuthDevFlowArgs {
            frontend_url,
            backend_url: None,
            client_id: DEFAULT_CLIENT_ID.to_string(),
            callback_bind: DEFAULT_CALLBACK_BIND.to_string(),
            callback_path: DEFAULT_CALLBACK_PATH.to_string(),
            scope: DEFAULT_SCOPE.to_string(),
            no_open: true,
            print_url: true,
            keep_grant: false,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        })
        .expect("config");

        let error = preflight(&config).expect_err("preflight should fail");

        assert!(error.to_string().contains("Start cargo xtask serve first"));
    }

    fn test_config(backend_url: Option<&str>) -> OAuthDevFlowConfig {
        OAuthDevFlowConfig::from_args(OAuthDevFlowArgs {
            frontend_url: DEFAULT_FRONTEND_URL.to_string(),
            backend_url: backend_url.map(ToOwned::to_owned),
            client_id: DEFAULT_CLIENT_ID.to_string(),
            callback_bind: DEFAULT_CALLBACK_BIND.to_string(),
            callback_path: DEFAULT_CALLBACK_PATH.to_string(),
            scope: DEFAULT_SCOPE.to_string(),
            no_open: true,
            print_url: true,
            keep_grant: false,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        })
        .expect("config")
    }

    fn test_metadata() -> OAuthMetadataResponse {
        OAuthMetadataResponse {
            authorization_endpoint: "http://localhost:3000/oauth/authorize".to_string(),
            revocation_endpoint: "http://localhost:3000/oauth/revoke".to_string(),
            token_endpoint: "http://localhost:3000/oauth/token".to_string(),
        }
    }

    fn test_token() -> OAuthTokenResponse {
        OAuthTokenResponse {
            access_token: "secret-access-token".to_string(),
            expires_in: 3600,
            refresh_token: "secret-refresh-token".to_string(),
            scope: DEFAULT_SCOPE.to_string(),
            token_type: "Bearer".to_string(),
        }
    }

    fn is_base64url_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
    }
}
