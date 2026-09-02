//! Seam for tunnel proxies — the family Scryer brings up itself rather than
//! dials into.
//!
//! A transport proxy already exists when Scryer starts talking to it: the HTTP
//! client is handed `socks5://gateway:1080` and that is the whole story. A
//! tunnel does not exist until Scryer establishes it, so something has to run
//! the SSH (later: WireGuard) session and then expose an endpoint the ordinary
//! transport client factories can dial. [`resolve_tunnel_endpoint`] is that
//! seam.
//!
//! **This file contains no engine.** WP6 lands the russh client, the tunnel
//! lifecycle and the loopback front; until then every tunnel resolution fails,
//! deliberately and loudly, so a configured tunnel can never degrade into
//! unproxied traffic. The whole point of assigning a tunnel is that the traffic
//! must not leave by the default route.

use scryer_domain::ProxyConfig;

/// The endpoint an established tunnel exposes to the existing transport client
/// factories: a proxy URL they already know how to dial (WP6's plan is a
/// loopback-bound SOCKS5 front, `socks5://127.0.0.1:<ephemeral>`).
///
/// Modelling the seam as a URL rather than as a socket is what keeps a second
/// tunnel implementation (WireGuard over smoltcp) a drop-in: it brings up a
/// different session and publishes the same kind of endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelEndpoint {
    /// Proxy URL to hand to `scryer_outbound_http`'s transport factories.
    pub egress_url: String,
}

/// Something that can bring up a tunnel for a configuration and publish the
/// endpoint traffic should egress through.
///
/// One implementation exists per tunnel technology. WP6 implements it for SSH
/// with russh; WireGuard is the planned second. Callers never learn which.
#[async_trait::async_trait]
pub trait TunnelProvider: Send + Sync {
    /// Ensure a tunnel for `config` is running and return its egress endpoint.
    ///
    /// Implementations are expected to cache per configuration revision
    /// (`transport_proxy::transport_proxy_revision`) rather than dial per
    /// request.
    async fn resolve_endpoint(&self, config: &ProxyConfig) -> Result<TunnelEndpoint, String>;
}

/// Detail appended to the proxy-named egress failure when a tunnel is assigned
/// but no engine can bring it up.
pub const TUNNEL_ENGINE_UNAVAILABLE_DETAIL: &str = "tunnel engine not available";

/// Message returned by the health probe for a tunnel row.
pub const TUNNEL_PROBE_UNAVAILABLE_MESSAGE: &str =
    "SSH tunnel probing arrives with the tunnel engine";

/// The failure an operator sees when their tunnel-assigned traffic cannot go
/// anywhere. Shares the `proxy <name> unreachable: <detail>` shape (and the
/// redaction) with every other proxy-hop failure.
pub fn tunnel_engine_unavailable_message(config: &ProxyConfig) -> String {
    crate::transport_proxy::transport_proxy_unreachable_message(
        config,
        TUNNEL_ENGINE_UNAVAILABLE_DETAIL,
    )
}

/// Resolve the egress endpoint for a tunnel configuration.
///
/// The only implementation today fails. That is the fail-closed contract: an
/// egress path that cannot resolve a tunnel must error, not fall back to a
/// direct connection.
pub fn resolve_tunnel_endpoint(config: &ProxyConfig) -> Result<TunnelEndpoint, String> {
    Err(tunnel_engine_unavailable_message(config))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use scryer_domain::{ProxyKind, ProxyProviderType};

    pub(crate) fn tunnel_config() -> ProxyConfig {
        let created_at = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
        ProxyConfig {
            id: "proxy-tunnel".to_string(),
            name: "Seedbox".to_string(),
            provider_type: ProxyProviderType::SshTunnel,
            protocol: None,
            base_url: "ssh://seedbox.test:22".to_string(),
            request_timeout_seconds: 30,
            is_enabled: true,
            username_encrypted: Some("operator".to_string()),
            password_encrypted: Some("s3cret".to_string()),
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn resolving_a_tunnel_endpoint_fails_until_the_engine_lands() {
        let config = tunnel_config();
        assert_eq!(config.kind(), ProxyKind::Tunnel);
        let error = resolve_tunnel_endpoint(&config).expect_err("no engine exists yet");
        assert_eq!(
            error,
            "proxy Seedbox unreachable: tunnel engine not available"
        );
    }
}
