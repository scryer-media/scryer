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
    // A tunnel's username and password are *SSH* credentials, consumed by the
    // engine when it establishes the session. What the HTTP client dials is our
    // own loopback SOCKS5 front, which accepts no authentication at all —
    // offering it these would put the SSH password on a socket with no use for
    // it.
    if config.is_tunnel() {
        return None;
    }
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
/// * **Tunnel** — the loopback SOCKS5 front of a tunnel this process brings up,
///   started on demand by [`crate::tunnel_proxy::resolve_tunnel_endpoint`]. A
///   tunnel that cannot be established errors here rather than degrading into a
///   direct connection.
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
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
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

    /// Was `a_tunnel_proxy_fails_closed_instead_of_egressing_directly` while
    /// there was no engine. The guarantee is unchanged — a tunnel-assigned
    /// request never takes the default route — but it is now met by *taking the
    /// tunnel* rather than by refusing: the gate hands out a loopback SOCKS5
    /// front, and both client factories build a proxied client for it.
    #[test]
    fn a_tunnel_proxy_egresses_through_its_own_loopback_socks5_front() {
        let mut config = crate::tunnel_proxy::tests::tunnel_config();
        config.id = "proxy-transport-front".to_string();

        let egress = proxy_egress_url(&config).expect("the engine publishes an endpoint");
        let port = egress
            .strip_prefix("socks5h://127.0.0.1:")
            .unwrap_or_else(|| panic!("expected a loopback socks5h front, got {egress}"))
            .parse::<u16>()
            .expect("front port");
        // `socks5h`, not `socks5`: the destination name must resolve on the far
        // side of the tunnel.
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());

        // Both factories build against it, and neither is a direct client.
        transport_proxied_reqwest_client(&config, "").expect("async client");
        blocking_transport_proxied_reqwest_client(&config, "").expect("blocking client");

        // The SSH credentials are not SOCKS credentials; the front takes none.
        assert!(transport_proxy_credentials(&config).is_none());

        crate::tunnel_proxy::stop_tunnel(&config.id);
    }

    /// The other half of the fail-closed contract: when the engine cannot even
    /// build a tunnel from the stored configuration, the caller still gets an
    /// error rather than an unproxied client.
    #[test]
    fn an_unusable_tunnel_configuration_still_fails_closed() {
        let mut config = crate::tunnel_proxy::tests::tunnel_config();
        config.id = "proxy-transport-unusable".to_string();
        config.username_encrypted = None;

        let error = proxy_egress_url(&config).expect_err("no username, no tunnel");
        assert_eq!(
            error,
            "proxy Seedbox unreachable: the tunnel has no username"
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

    /// The whole chain, against a real SSH peer: the transport client factory
    /// every consumer uses → the loopback SOCKS5 front → a `direct-tcpip`
    /// channel on a real SSH session → an HTTP origin. The SSH double records
    /// what it was asked to forward, so "the origin was reached" and "the
    /// origin was reached *through the tunnel*" are separate assertions.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_tunnel_carries_a_real_request_built_by_the_transport_client_factory() {
        let server = scryer_tunnel::test_support::SshServerDouble::start(
            scryer_tunnel::test_support::SshServerOptions::default(),
        )
        .await;
        let origin =
            scryer_tunnel::test_support::TunnelledOrigin::start("through the tunnel").await;

        let mut config = crate::tunnel_proxy::tests::tunnel_config();
        config.id = "proxy-chain".to_string();
        config.base_url = format!("ssh://{}", server.addr());

        let client = transport_proxied_reqwest_client(&config, "").expect("client");
        let response = client.get(origin.url()).send().await.expect("response");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.text().await.expect("body"), "through the tunnel");

        assert_eq!(
            server.forwarded_targets(),
            vec![("127.0.0.1".to_string(), origin.addr().port())],
            "the request must have been forwarded by the SSH server"
        );
        assert_eq!(origin.request_lines().len(), 1);

        // First use pinned the host key, ready for the repository flush.
        let pin = crate::tunnel_proxy::TunnelHostKeyLedger::shared()
            .take("proxy-chain")
            .expect("first use must queue a host key pin");
        assert_eq!(
            pin.fingerprint,
            scryer_tunnel::test_support::HOST_KEY_FINGERPRINT
        );

        crate::tunnel_proxy::stop_tunnel(&config.id);
    }

    /// A tunnel that cannot be established fails the request and records the
    /// proxy — not the destination — as unhealthy.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_tunnel_fails_the_request_and_records_the_proxy() {
        let dead_ssh_port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            let port = listener.local_addr().expect("addr").port();
            drop(listener);
            port
        };
        let origin =
            scryer_tunnel::test_support::TunnelledOrigin::start("must not be reached").await;

        let mut config = crate::tunnel_proxy::tests::tunnel_config();
        config.id = "proxy-chain-dead".to_string();
        config.base_url = format!("ssh://127.0.0.1:{dead_ssh_port}");

        let client = transport_proxied_reqwest_client(&config, "").expect("client");
        let error = client
            .get(origin.url())
            .send()
            .await
            .expect_err("an unreachable tunnel must fail the request");
        assert!(
            transport_proxy_connect_failure(&config, &error).is_some(),
            "a tunnel failure must classify as a proxy-hop failure: {error}"
        );
        assert!(
            origin.request_lines().is_empty(),
            "nothing may reach the origin when the tunnel is down"
        );

        let recorded: Vec<_> = crate::challenge_solver::SolverHealthLedger::shared()
            .drain()
            .into_iter()
            .filter(|event| event.proxy_config_id == "proxy-chain-dead")
            .collect();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert!(!recorded[0].healthy);
        let message = recorded[0].message.clone().unwrap_or_default();
        assert!(
            message.starts_with("proxy Seedbox unreachable:"),
            "the health row must name the proxy: {message}"
        );

        crate::tunnel_proxy::stop_tunnel(&config.id);
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
