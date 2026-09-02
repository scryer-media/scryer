//! Egress seam for transport proxies (HTTP CONNECT, SOCKS4, SOCKS5).
//!
//! A challenge solver is an *application* protocol: Scryer posts a solve
//! request and interprets the answer. A transport proxy is the opposite — it
//! only carries bytes, so every path that speaks to the indexer just swaps its
//! HTTP client for one that dials through the proxy. This module owns that swap
//! so the plugin host, the download router and the health probe cannot drift.
//!
//! ## Where credentials are decrypted
//!
//! Nowhere here. `ProxyConfig::username_encrypted` / `password_encrypted`
//! are named for the columns they persist to but hold **plaintext in memory**;
//! `ProxyConfigStore` encrypts on write and decrypts on read, exactly as
//! `IndexerConfig::api_key_encrypted` does. That is what keeps
//! `scryer-outbound-http` free of key material: it never sees an
//! `EncryptionKey`, it never sees a config, it takes two `&str`s. This module is
//! the only place that reads the credential fields off a config, and it is one
//! layer above the store that decrypted them.

use std::time::Duration;

use scryer_domain::ProxyConfig;
use scryer_outbound_http::TransportProxyCredentials;

/// Prefix for a failure of the proxy hop itself, as opposed to the indexer
/// behind it. Named separately so callers can classify a message they only
/// receive as a string (the blocking plugin host does).
pub const TRANSPORT_PROXY_EGRESS_UNREACHABLE_MARKER: &str = "unreachable:";

/// The stored proxy endpoint as reqwest must see it, including the
/// `remote_dns` → `socks5h`/`socks4a` scheme mapping.
pub fn transport_proxy_egress_url(config: &ProxyConfig) -> String {
    scryer_outbound_http::transport_proxy_egress_url(&config.base_url, config.remote_dns)
}

/// Proxy credentials, or `None` when the operator configured an open proxy.
///
/// A password without a username is rejected at configuration time, so a
/// username is the presence test; a username without a password authenticates
/// with an empty one.
pub fn transport_proxy_credentials(config: &ProxyConfig) -> Option<TransportProxyCredentials<'_>> {
    let username = config.username_encrypted.as_deref()?;
    Some(TransportProxyCredentials {
        username,
        password: config.password_encrypted.as_deref().unwrap_or(""),
    })
}

/// Per-request budget for traffic carried through this proxy. Same clamp the
/// solver and health paths use, so a transport proxy cannot buy an indexer more
/// wall clock than a solver can.
pub fn transport_proxy_request_timeout(config: &ProxyConfig) -> Duration {
    scryer_outbound_http::effective_proxy_request_timeout(config.request_timeout_seconds)
}

/// Cache revision for a built client.
///
/// A cached transport-proxied client must be dropped the moment the operator
/// edits the proxy, and `updated_at` is already the repo-wide revision marker
/// for exactly this purpose: proxy *health* writes go through
/// `record_health`, which deliberately does not bump `updated_at`, so a health
/// flap does not churn clients while an endpoint, credential or `remote_dns`
/// edit does. Pairing it with the id also covers reassignment to a different
/// proxy row.
pub fn transport_proxy_revision(config: &ProxyConfig) -> String {
    format!("{}@{}", config.id, config.updated_at.to_rfc3339())
}

/// The proxy URL a non-solver proxy egresses through, or the reason it cannot.
///
/// This is the fail-closed gate for the whole egress family: every client
/// factory below goes through it, so a proxy kind with no working egress can
/// never end up silently omitted from the request. The caller gets an error
/// instead of a direct, unproxied connection.
///
/// * **Transport** — the stored endpoint, with the `remote_dns` mapping.
/// * **Tunnel** — fails until the tunnel engine lands. Resolution goes through
///   [`crate::tunnel_proxy::resolve_tunnel_endpoint`], which is the single
///   place a real engine gets wired in.
/// * **Challenge solver** — reaching here at all is a routing bug: a solver is
///   an application protocol, not a hop to dial. Say so rather than inventing
///   an endpoint.
pub fn proxy_egress_url(config: &ProxyConfig) -> Result<String, String> {
    match config.kind() {
        scryer_domain::ProxyKind::Transport => Ok(transport_proxy_egress_url(config)),
        scryer_domain::ProxyKind::Tunnel => {
            crate::tunnel_proxy::resolve_tunnel_endpoint(config).map(|tunnel| tunnel.egress_url)
        }
        scryer_domain::ProxyKind::ChallengeSolver => Err(transport_proxy_unreachable_message(
            config,
            crate::challenge_solver::TRANSPORT_PROXY_NOT_A_SOLVER_MESSAGE,
        )),
    }
}

/// Build an async client whose every request egresses through `config`.
pub fn transport_proxied_reqwest_client(
    config: &ProxyConfig,
    extra_ca_bundle_pem: &str,
) -> Result<reqwest::Client, String> {
    scryer_outbound_http::transport_proxy_reqwest_client_with_extra_ca(
        &proxy_egress_url(config)?,
        transport_proxy_credentials(config),
        transport_proxy_request_timeout(config),
        extra_ca_bundle_pem,
    )
    .map_err(|error| transport_proxy_unreachable_message(config, &error))
}

/// Blocking twin of [`transport_proxied_reqwest_client`], for the blocking
/// plugin HTTP worker.
pub fn blocking_transport_proxied_reqwest_client(
    config: &ProxyConfig,
    extra_ca_bundle_pem: &str,
) -> Result<reqwest::blocking::Client, String> {
    scryer_outbound_http::blocking_transport_proxy_reqwest_client(
        &proxy_egress_url(config)?,
        transport_proxy_credentials(config),
        transport_proxy_request_timeout(config),
        extra_ca_bundle_pem,
    )
    .map_err(|error| transport_proxy_unreachable_message(config, &error))
}

/// Message for a failure of the proxy hop. Names the operator's proxy so the
/// operator can tell "my proxy is down" from "this indexer is down"; the detail
/// is sanitized with the same redaction the solver health path uses.
pub fn transport_proxy_unreachable_message(config: &ProxyConfig, detail: &str) -> String {
    format!(
        "proxy {} {TRANSPORT_PROXY_EGRESS_UNREACHABLE_MARKER} {}",
        config.name.trim(),
        crate::challenge_solver::sanitize_proxy_error(detail)
    )
}

/// Classify a reqwest failure observed on a transport-proxied request.
///
/// `is_connect` covers the connector: the TCP dial to the proxy, the SOCKS
/// handshake and the HTTP `CONNECT` exchange all fail here. Every hop on this
/// client goes through the proxy, so a connector failure is a proxy-path
/// failure and is reported as one. Anything past the connector (a timeout, a
/// mid-body reset) is the indexer's answer arriving late or badly and keeps its
/// existing classification.
pub fn transport_proxy_connect_failure(
    config: &ProxyConfig,
    error: &reqwest::Error,
) -> Option<String> {
    error
        .is_connect()
        .then(|| transport_proxy_unreachable_message(config, &error.to_string()))
}

/// Record a proxy-hop failure against the shared proxy health ledger.
///
/// This is the *same* ledger and the same convention the solver paths use:
/// egress sites that cannot reach the repository record here, and the async
/// flows that own a repository handle (`prepare_download_request`, the search
/// pass) drain it through `flush_solver_health`. No second health pipeline.
pub fn record_transport_proxy_failure(config: &ProxyConfig, message: &str) {
    crate::challenge_solver::SolverHealthLedger::shared().record_failure(&config.id, message);
}

/// Record a successful hop through the proxy, so a recovered proxy clears its
/// unhealthy marker on the next flush.
pub fn record_transport_proxy_success(config: &ProxyConfig) {
    crate::challenge_solver::SolverHealthLedger::shared().record_success(&config.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use scryer_domain::ProxyProviderType;

    fn transport_config(provider_type: ProxyProviderType, base_url: &str) -> ProxyConfig {
        let created_at = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
        ProxyConfig {
            id: "proxy-1".to_string(),
            name: "House VPN".to_string(),
            provider_type,
            protocol: None,
            base_url: base_url.to_string(),
            request_timeout_seconds: 30,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at,
            updated_at: created_at,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
        }
    }

    #[test]
    fn egress_url_carries_the_remote_dns_flag_onto_the_scheme() {
        let mut config = transport_config(ProxyProviderType::Socks5, "socks5://gw:1080");
        assert_eq!(transport_proxy_egress_url(&config), "socks5://gw:1080");
        config.remote_dns = true;
        assert_eq!(transport_proxy_egress_url(&config), "socks5h://gw:1080");
    }

    #[test]
    fn credentials_are_read_as_plaintext_from_the_config() {
        let mut config = transport_config(ProxyProviderType::Http, "http://gw:3128");
        assert!(transport_proxy_credentials(&config).is_none());
        config.username_encrypted = Some("operator".to_string());
        let credentials = transport_proxy_credentials(&config).expect("credentials");
        assert_eq!(credentials.username, "operator");
        assert_eq!(credentials.password, "");
        config.password_encrypted = Some("s3cret".to_string());
        assert_eq!(
            transport_proxy_credentials(&config)
                .expect("credentials")
                .password,
            "s3cret"
        );
    }

    #[test]
    fn the_cache_revision_changes_only_when_the_configuration_does() {
        let config = transport_config(ProxyProviderType::Socks5, "socks5://gw:1080");
        let baseline = transport_proxy_revision(&config);

        // A health write does not touch `updated_at`, so it must not evict a
        // perfectly good cached client.
        let mut flapped = config.clone();
        flapped.last_health_status = Some(scryer_domain::ProxyHealthStatus::Unhealthy);
        flapped.last_error_message = Some("transient".to_string());
        assert_eq!(transport_proxy_revision(&flapped), baseline);

        // An operator edit does.
        let mut edited = config.clone();
        edited.base_url = "socks5://other:1080".to_string();
        edited.updated_at = config.updated_at + chrono::Duration::seconds(1);
        assert_ne!(transport_proxy_revision(&edited), baseline);

        // So does reassignment to a different proxy row.
        let mut reassigned = config.clone();
        reassigned.id = "proxy-2".to_string();
        assert_ne!(transport_proxy_revision(&reassigned), baseline);
    }

    #[test]
    fn the_failure_message_names_the_proxy_and_redacts_secrets() {
        let config = transport_config(ProxyProviderType::Socks5, "socks5://gw:1080");
        let message = transport_proxy_unreachable_message(&config, "connection refused");
        assert_eq!(message, "proxy House VPN unreachable: connection refused");
        assert!(message.contains(TRANSPORT_PROXY_EGRESS_UNREACHABLE_MARKER));

        let redacted = transport_proxy_unreachable_message(
            &config,
            "failed to reach https://indexer.test/api?apikey=supersecret",
        );
        assert!(
            !redacted.contains("supersecret"),
            "credentials must not leak into proxy health: {redacted}"
        );
    }

    #[test]
    fn a_tunnel_proxy_fails_closed_instead_of_egressing_directly() {
        let config = crate::tunnel_proxy::tests::tunnel_config();

        // No egress URL can be produced, so no client can be built. The
        // alternative — returning an unproxied client — would put the traffic
        // an operator explicitly tunnelled onto the default route.
        let error = proxy_egress_url(&config).expect_err("a tunnel has no endpoint yet");
        assert_eq!(
            error,
            "proxy Seedbox unreachable: tunnel engine not available"
        );
        assert_eq!(
            transport_proxied_reqwest_client(&config, "").expect_err("async client"),
            error
        );
        assert_eq!(
            blocking_transport_proxied_reqwest_client(&config, "").expect_err("blocking client"),
            error
        );
    }

    #[test]
    fn a_solver_proxy_never_resolves_a_transport_endpoint() {
        let mut config = transport_config(ProxyProviderType::Socks5, "socks5://gw:1080");
        config.provider_type = ProxyProviderType::Trawl;
        let error = proxy_egress_url(&config).expect_err("a solver is not a hop to dial");
        assert!(
            error.contains(crate::challenge_solver::TRANSPORT_PROXY_NOT_A_SOLVER_MESSAGE),
            "{error}"
        );
    }

    #[test]
    fn clients_build_for_every_transport_provider() {
        for (provider, base_url) in [
            (ProxyProviderType::Http, "http://gw:3128"),
            (ProxyProviderType::Socks4, "socks4://gw:1080"),
            (ProxyProviderType::Socks5, "socks5://gw:1080"),
        ] {
            let mut config = transport_config(provider, base_url);
            config.username_encrypted = Some("operator".to_string());
            config.password_encrypted = Some("s3cret".to_string());
            transport_proxied_reqwest_client(&config, "").expect("async client");
            blocking_transport_proxied_reqwest_client(&config, "").expect("blocking client");
        }
    }
}
