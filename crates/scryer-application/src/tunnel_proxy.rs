//! Seam for tunnel proxies — the family Scryer brings up itself rather than
//! dials into.
//!
//! A transport proxy already exists when Scryer starts talking to it: the HTTP
//! client is handed `socks5://gateway:1080` and that is the whole story. A
//! tunnel does not exist until Scryer establishes it, so something has to run
//! the session — an SSH connection or a userspace WireGuard device — and then
//! expose an endpoint the ordinary transport client factories can dial.
//! [`resolve_tunnel_endpoint`] is that seam, and it is the only function the
//! egress gate calls.
//!
//! The engines themselves live in `scryer-tunnel`, which owns the russh and
//! gotatun dependencies and the loopback SOCKS5 front. This module is the
//! translation layer: it maps a [`ProxyConfig`] onto the engine's domain-free
//! `TunnelSpec` or `WireGuardSpec`, and it owns the ledgers the engines report
//! into, because they cannot reach a repository from the paths they run on.
//!
//! ```text
//! ProxyConfig ─┬─ tunnel_spec ────▶ TunnelRegistry::ensure_ssh_tunnel
//!              └─ wireguard_spec ─▶ TunnelRegistry::ensure_wireguard_tunnel
//!                                        │
//!                              socks5h://127.0.0.1:<port>
//!                                        │
//!                       every existing transport client factory
//! ```
//!
//! The only thing that differs between the two families is which provider the
//! registry builds. Nothing downstream of the front knows which one it got.

use std::sync::{Arc, LazyLock, Mutex};

use chrono::{DateTime, Utc};
use scryer_domain::{ProxyConfig, ProxyProviderType};
use scryer_tunnel::{TunnelObserver, TunnelRegistry, TunnelSpec, WireGuardSpec};

pub use scryer_tunnel::{
    ED25519_ONLY_PRIVATE_KEY_MESSAGE, TunnelError, TunnelHandshake, WIREGUARD_KEY_MESSAGE,
    WireGuardHandshake,
};

/// The endpoint an established tunnel exposes to the existing transport client
/// factories: a proxy URL they already know how to dial.
///
/// Modelling the seam as a URL rather than as a socket is what keeps a second
/// tunnel implementation (WireGuard over smoltcp) a drop-in: it brings up a
/// different session and publishes the same kind of endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelEndpoint {
    /// Proxy URL to hand to `scryer_outbound_http`'s transport factories.
    pub egress_url: String,
}

/// Turn a stored configuration into what the engine needs.
///
/// The credential fields are named for their columns but hold plaintext in
/// memory (`ProxyConfigStore` decrypts on read), exactly as they do on the
/// transport path — this is the only place they are read for a tunnel.
pub fn tunnel_spec(config: &ProxyConfig) -> Result<TunnelSpec, String> {
    let (host, port) = crate::integration::workflow::transport_proxy_endpoint(config)
        .map_err(|error| error.to_string())?;
    let username = config
        .username_encrypted
        .as_deref()
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .ok_or_else(|| "the tunnel has no username".to_string())?;
    Ok(TunnelSpec {
        proxy_config_id: config.id.clone(),
        proxy_name: config.name.trim().to_string(),
        revision: crate::transport_proxy::transport_proxy_revision(config),
        host,
        port,
        username: username.to_string(),
        password: config.password_encrypted.clone(),
        private_key_pem: config.private_key_encrypted.clone(),
        private_key_passphrase: config.private_key_passphrase_encrypted.clone(),
        pinned_host_key: config.host_key_fingerprint.clone(),
        request_timeout: crate::transport_proxy::transport_proxy_request_timeout(config),
    })
}

/// Resolve the egress endpoint for a tunnel configuration, starting the tunnel
/// if it is not already running.
///
/// Synchronous on purpose: the gate that calls it
/// (`transport_proxy::proxy_egress_url`) runs on async hosts *and* on the
/// blocking plugin HTTP worker. Starting a tunnel binds a loopback socket and
/// hands the accept loop to the engine's own runtime, so this never blocks and
/// never needs an ambient runtime.
///
/// The scheme is `socks5h`, not `socks5`: names must resolve on the far side of
/// the tunnel, or a seedbox's `localhost` would mean this machine.
pub fn resolve_tunnel_endpoint(config: &ProxyConfig) -> Result<TunnelEndpoint, String> {
    match start_tunnel(config) {
        Ok(endpoint) => Ok(endpoint),
        Err(detail) => {
            let message =
                crate::transport_proxy::transport_proxy_unreachable_message(config, &detail);
            crate::transport_proxy::record_transport_proxy_failure(config, &message);
            Err(message)
        }
    }
}

/// Turn a stored configuration into what the WireGuard engine needs.
///
/// The counterpart of [`tunnel_spec`], and the only place a `wireguard://` row
/// is decoded. Everything an operator pasted is parsed here rather than at the
/// device, so a malformed address is a configuration error with the operator's
/// own text in it and never a mid-handshake surprise — and the same parsers run
/// at save time, so a row that reaches here has already been through them once.
pub fn wireguard_spec(config: &ProxyConfig) -> Result<WireGuardSpec, String> {
    let (endpoint_host, endpoint_port) =
        crate::integration::workflow::transport_proxy_endpoint(config)
            .map_err(|error| error.to_string())?;
    let private_key = config
        .private_key_encrypted
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "the tunnel has no private key".to_string())?;
    let private_key =
        scryer_tunnel::parse_key(private_key).map_err(|error| format!("private key: {error}"))?;
    let peer_public_key = config
        .peer_public_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "the tunnel has no peer public key".to_string())?;
    let peer_public_key = scryer_tunnel::parse_key(peer_public_key)
        .map_err(|error| format!("peer public key: {error}"))?;
    let preshared_key = config
        .preshared_key_encrypted
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(|key| scryer_tunnel::parse_key(key).map_err(|error| format!("preshared key: {error}")))
        .transpose()?;
    let addresses = config
        .tunnel_addresses
        .iter()
        .map(|address| {
            address
                .parse::<scryer_tunnel::IpCidr>()
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let dns_servers = config
        .tunnel_dns_servers
        .iter()
        .map(|server| {
            server.parse::<std::net::IpAddr>().map_err(|_| {
                format!("`{server}` is not a DNS server address; the `DNS` line takes IP addresses")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WireGuardSpec {
        proxy_config_id: config.id.clone(),
        proxy_name: config.name.trim().to_string(),
        revision: crate::transport_proxy::transport_proxy_revision(config),
        endpoint_host,
        endpoint_port,
        private_key,
        peer_public_key,
        preshared_key,
        addresses,
        dns_servers,
        mtu: config
            .tunnel_mtu
            .unwrap_or(scryer_tunnel::DEFAULT_WIREGUARD_MTU),
        // A stored 0 is the operator switching keepalive off, which is a
        // different statement from having no opinion — so only an absent value
        // takes the default.
        persistent_keepalive: match config.tunnel_keepalive_seconds {
            None => Some(scryer_tunnel::DEFAULT_WIREGUARD_KEEPALIVE),
            Some(0) => None,
            Some(seconds) => Some(seconds),
        },
        request_timeout: crate::transport_proxy::transport_proxy_request_timeout(config),
    })
}

/// Our own public key for a pasted WireGuard private key, base64, as the
/// server's `[Peer]` section must list it.
///
/// The workflow calls this on every private-key write so `tunnel_public_key`
/// is maintained in the row, and nothing above this layer ever needs the tunnel
/// crate to show the operator that line.
pub fn wireguard_public_key(private_key: &str) -> Result<String, String> {
    scryer_tunnel::parse_key(private_key)
        .map(|key| scryer_tunnel::public_key_of(&key))
        .map_err(|error| error.to_string())
}

fn start_tunnel(config: &ProxyConfig) -> Result<TunnelEndpoint, String> {
    let registry = TunnelRegistry::shared().map_err(|error| error.to_string())?;
    // One front, one lifecycle, one key: the only thing that differs between
    // the two tunnel families is which provider the registry builds.
    let front = if config.provider_type == ProxyProviderType::WireGuard {
        let spec = wireguard_spec(config)?;
        let observer: Arc<dyn TunnelObserver> = Arc::new(LedgerObserver {
            proxy_name: spec.proxy_name.clone(),
        });
        registry.ensure_wireguard_tunnel(spec, observer)
    } else {
        let spec = tunnel_spec(config)?;
        let observer: Arc<dyn TunnelObserver> = Arc::new(LedgerObserver {
            proxy_name: spec.proxy_name.clone(),
        });
        registry.ensure_ssh_tunnel(spec, observer)
    }
    .map_err(|error| error.to_string())?;
    Ok(TunnelEndpoint {
        egress_url: format!("socks5h://127.0.0.1:{}", front.port()),
    })
}

/// Connect, authenticate and report the host key without leaving a tunnel
/// behind. The health probe's entry point.
pub async fn probe_tunnel_handshake(config: &ProxyConfig) -> Result<TunnelHandshake, String> {
    let spec = tunnel_spec(config)?;
    scryer_tunnel::SshTunnelProvider::handshake(
        spec,
        Arc::new(LedgerObserver {
            proxy_name: config.name.trim().to_string(),
        }),
    )
    .await
    .map_err(|error| error.to_string())
}

/// The WireGuard counterpart: bring a device and stack up, read what the
/// handshake established, tear both down.
///
/// There is no host key to settle — the peer's public key is its identity and
/// the operator configured it — so this reports what it *did* establish
/// instead: which peer answered, how long it took, and our own public key.
pub async fn probe_wireguard_handshake(config: &ProxyConfig) -> Result<WireGuardHandshake, String> {
    let spec = wireguard_spec(config)?;
    scryer_tunnel::WireGuardTunnelProvider::handshake(
        spec,
        Arc::new(LedgerObserver {
            proxy_name: config.name.trim().to_string(),
        }),
    )
    .await
    .map_err(|error| error.to_string())
}

/// Stop every running tunnel. Called on process shutdown.
pub fn stop_all_tunnels() {
    if let Ok(registry) = TunnelRegistry::shared() {
        registry.stop_all();
    }
}

/// Stop one tunnel, so a session cannot outlive the configuration that
/// justified it.
pub fn stop_tunnel(proxy_config_id: &str) {
    if let Ok(registry) = TunnelRegistry::shared() {
        registry.stop(proxy_config_id);
    }
}

/// A host key learned by a trust-on-first-use handshake, waiting to be
/// persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingHostKeyPin {
    pub proxy_config_id: String,
    pub fingerprint: String,
    pub pinned_at: DateTime<Utc>,
}

/// Process-shared ledger of host keys learned on first use.
///
/// Same shape and the same reason as `SolverHealthLedger`: the tunnel engine
/// runs on egress paths that hold no repository handle (including a blocking
/// plugin worker thread), so it records here and the async flows that own a
/// repository drain it. `flush_solver_health` is that drain — one call-site
/// list for both ledgers.
pub struct TunnelHostKeyLedger {
    pins: Mutex<std::collections::HashMap<String, PendingHostKeyPin>>,
}

static SHARED_HOST_KEYS: LazyLock<TunnelHostKeyLedger> = LazyLock::new(|| TunnelHostKeyLedger {
    pins: Mutex::new(std::collections::HashMap::new()),
});

impl TunnelHostKeyLedger {
    pub fn shared() -> &'static TunnelHostKeyLedger {
        &SHARED_HOST_KEYS
    }

    pub fn record(&self, proxy_config_id: &str, fingerprint: &str) {
        self.pins
            .lock()
            .expect("tunnel host key ledger lock poisoned")
            .insert(
                proxy_config_id.to_string(),
                PendingHostKeyPin {
                    proxy_config_id: proxy_config_id.to_string(),
                    fingerprint: fingerprint.to_string(),
                    pinned_at: Utc::now(),
                },
            );
    }

    /// Remove and return one proxy's pending pin.
    ///
    /// The health probe owns a repository handle and pins directly, so it takes
    /// its own entry back off the queue rather than leaving a redundant write
    /// for the next flush.
    pub fn take(&self, proxy_config_id: &str) -> Option<PendingHostKeyPin> {
        self.pins
            .lock()
            .expect("tunnel host key ledger lock poisoned")
            .remove(proxy_config_id)
    }

    pub fn drain(&self) -> Vec<PendingHostKeyPin> {
        self.pins
            .lock()
            .expect("tunnel host key ledger lock poisoned")
            .drain()
            .map(|(_, pin)| pin)
            .collect()
    }
}

/// Bridges the engine's observations onto the ledgers.
struct LedgerObserver {
    proxy_name: String,
}

impl TunnelObserver for LedgerObserver {
    fn tunnel_dial_failed(&self, proxy_config_id: &str, message: &str) {
        // Same wording as every other proxy-hop failure, so "my proxy is down"
        // still reads differently from "this indexer is down".
        crate::challenge_solver::SolverHealthLedger::shared().record_failure(
            proxy_config_id,
            &format!(
                "proxy {} {} {message}",
                self.proxy_name,
                crate::transport_proxy::TRANSPORT_PROXY_EGRESS_UNREACHABLE_MARKER
            ),
        );
    }

    fn tunnel_dial_succeeded(&self, proxy_config_id: &str) {
        crate::challenge_solver::SolverHealthLedger::shared().record_success(proxy_config_id);
    }

    fn host_key_pinned(&self, proxy_config_id: &str, fingerprint: &str) {
        TunnelHostKeyLedger::shared().record(proxy_config_id, fingerprint);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::TimeZone;
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
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
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
    fn a_tunnel_config_maps_onto_an_engine_spec_with_the_stored_credentials() {
        let mut config = tunnel_config();
        config.private_key_encrypted = Some("-----BEGIN OPENSSH PRIVATE KEY-----".to_string());
        config.private_key_passphrase_encrypted = Some("hunter2".to_string());
        config.host_key_fingerprint = Some("SHA256:abc".to_string());
        let spec = tunnel_spec(&config).expect("spec");

        assert_eq!(spec.proxy_config_id, "proxy-tunnel");
        assert_eq!(spec.host, "seedbox.test");
        assert_eq!(spec.port, 22);
        assert_eq!(spec.username, "operator");
        assert_eq!(spec.password.as_deref(), Some("s3cret"));
        assert_eq!(
            spec.private_key_pem.as_deref(),
            Some("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert_eq!(spec.private_key_passphrase.as_deref(), Some("hunter2"));
        assert_eq!(spec.pinned_host_key.as_deref(), Some("SHA256:abc"));
        assert_eq!(
            spec.revision,
            crate::transport_proxy::transport_proxy_revision(&config)
        );
    }

    #[test]
    fn a_tunnel_url_without_a_port_takes_the_ssh_default() {
        let mut config = tunnel_config();
        config.base_url = "ssh://seedbox.test".to_string();
        assert_eq!(tunnel_spec(&config).expect("spec").port, 22);
    }

    /// The engine replaces WP4's fail-closed stub: a tunnel now resolves to a
    /// loopback SOCKS5 front, and `socks5h` is what makes names resolve on the
    /// far side of it.
    #[test]
    fn resolving_a_tunnel_endpoint_starts_a_loopback_socks5_front() {
        let mut config = tunnel_config();
        config.id = "proxy-endpoint-front".to_string();
        assert_eq!(config.kind(), ProxyKind::Tunnel);

        let endpoint = resolve_tunnel_endpoint(&config).expect("the engine starts a front");
        let port = endpoint
            .egress_url
            .strip_prefix("socks5h://127.0.0.1:")
            .unwrap_or_else(|| panic!("unexpected egress url: {}", endpoint.egress_url))
            .parse::<u16>()
            .expect("port");
        assert_ne!(port, 0);

        // The front is really listening, on loopback.
        let front = std::net::TcpStream::connect(("127.0.0.1", port)).expect("front is listening");
        assert!(front.peer_addr().expect("peer").ip().is_loopback());
        drop(front);

        // The same revision reuses the same front.
        assert_eq!(
            resolve_tunnel_endpoint(&config).expect("reuse").egress_url,
            endpoint.egress_url
        );

        // An operator edit moves it.
        let mut edited = config.clone();
        edited.updated_at = config.updated_at + chrono::Duration::seconds(1);
        assert_ne!(
            resolve_tunnel_endpoint(&edited)
                .expect("restart")
                .egress_url,
            endpoint.egress_url
        );
        stop_tunnel(&config.id);
    }

    #[test]
    fn a_tunnel_without_a_username_fails_closed_and_names_the_proxy() {
        let mut config = tunnel_config();
        config.id = "proxy-nouser".to_string();
        config.username_encrypted = None;
        let error = resolve_tunnel_endpoint(&config).expect_err("no username");
        assert_eq!(
            error,
            "proxy Seedbox unreachable: the tunnel has no username"
        );
    }

    /// Base64 for a raw 32-byte key, exactly as `wg genkey` prints it — which
    /// is what the operator pastes and therefore what the column holds.
    pub(crate) fn encoded_key(key: &[u8; 32]) -> String {
        use base64::Engine as _;
        base64::prelude::BASE64_STANDARD.encode(key)
    }

    /// A WireGuard configuration pointing at `endpoint`, carrying the test
    /// peer's key material.
    pub(crate) fn wireguard_config(id: &str, endpoint: &str) -> ProxyConfig {
        let created_at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
        let private_key = scryer_tunnel::test_support::test_client_private_key();
        ProxyConfig {
            id: id.to_string(),
            name: "House VPN".to_string(),
            provider_type: ProxyProviderType::WireGuard,
            protocol: None,
            base_url: format!("wireguard://{endpoint}"),
            request_timeout_seconds: 15,
            is_enabled: true,
            // No user, no password, no passphrase: WireGuard authenticates
            // with keys alone.
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: Some(encoded_key(&private_key)),
            private_key_passphrase_encrypted: None,
            peer_public_key: Some(scryer_tunnel::public_key_of(
                &scryer_tunnel::test_support::test_peer_private_key(),
            )),
            preshared_key_encrypted: None,
            tunnel_public_key: Some(scryer_tunnel::public_key_of(&private_key)),
            tunnel_addresses: vec![format!(
                "{}/32",
                scryer_tunnel::test_support::TEST_CLIENT_ADDRESS
            )],
            tunnel_dns_servers: vec![scryer_tunnel::test_support::TEST_PEER_ADDRESS.to_string()],
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
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
    fn a_wireguard_config_maps_onto_an_engine_spec_with_the_stored_key_material() {
        let mut config = wireguard_config("wg-spec", "vpn.test:51820");
        config.preshared_key_encrypted = Some(encoded_key(
            &scryer_tunnel::test_support::test_preshared_key(),
        ));
        config.tunnel_mtu = Some(1420);
        config.tunnel_keepalive_seconds = Some(15);
        let spec = wireguard_spec(&config).expect("spec");

        assert_eq!(spec.proxy_config_id, "wg-spec");
        assert_eq!(spec.endpoint_host, "vpn.test");
        assert_eq!(spec.endpoint_port, 51820);
        assert_eq!(
            spec.private_key,
            scryer_tunnel::test_support::test_client_private_key()
        );
        assert_eq!(
            encoded_key(&spec.peer_public_key),
            scryer_tunnel::public_key_of(&scryer_tunnel::test_support::test_peer_private_key())
        );
        assert_eq!(
            spec.preshared_key,
            Some(scryer_tunnel::test_support::test_preshared_key())
        );
        assert_eq!(spec.addresses.len(), 1);
        assert_eq!(spec.dns_servers.len(), 1);
        assert_eq!(spec.mtu, 1420);
        assert_eq!(spec.persistent_keepalive, Some(15));
        assert_eq!(
            spec.revision,
            crate::transport_proxy::transport_proxy_revision(&config)
        );
        // A stored public key is not consulted: the spec derives its own from
        // the private key, so the two can never disagree at connect time.
        assert_eq!(
            spec.public_key(),
            scryer_tunnel::public_key_of(&scryer_tunnel::test_support::test_client_private_key())
        );
    }

    #[test]
    fn a_wireguard_url_without_a_port_takes_the_wireguard_default() {
        let config = wireguard_config("wg-default-port", "vpn.test");
        assert_eq!(wireguard_spec(&config).expect("spec").endpoint_port, 51820);
    }

    /// The three states of the keepalive column are three different
    /// statements, and only the absent one takes the default.
    #[test]
    fn the_keepalive_column_distinguishes_off_from_unset() {
        let mut config = wireguard_config("wg-keepalive", "vpn.test:51820");
        assert_eq!(
            wireguard_spec(&config).expect("spec").persistent_keepalive,
            Some(scryer_tunnel::DEFAULT_WIREGUARD_KEEPALIVE)
        );
        config.tunnel_keepalive_seconds = Some(0);
        assert_eq!(
            wireguard_spec(&config).expect("spec").persistent_keepalive,
            None
        );
        config.tunnel_keepalive_seconds = Some(60);
        assert_eq!(
            wireguard_spec(&config).expect("spec").persistent_keepalive,
            Some(60)
        );
        // Same for the MTU: absent means the engine's number, not zero.
        assert_eq!(
            wireguard_spec(&config).expect("spec").mtu,
            scryer_tunnel::DEFAULT_WIREGUARD_MTU
        );
    }

    #[test]
    fn a_wireguard_config_without_its_keys_fails_closed_and_names_the_proxy() {
        let mut config = wireguard_config("wg-nokeys", "vpn.test:51820");
        config.private_key_encrypted = None;
        assert_eq!(
            resolve_tunnel_endpoint(&config).expect_err("no private key"),
            "proxy House VPN unreachable: the tunnel has no private key"
        );

        let mut config = wireguard_config("wg-nopeer", "vpn.test:51820");
        config.peer_public_key = None;
        assert_eq!(
            resolve_tunnel_endpoint(&config).expect_err("no peer public key"),
            "proxy House VPN unreachable: the tunnel has no peer public key"
        );
    }

    #[test]
    fn the_derived_public_key_is_what_the_server_must_list() {
        let private_key = scryer_tunnel::test_support::test_client_private_key();
        assert_eq!(
            wireguard_public_key(&encoded_key(&private_key)).expect("derives"),
            scryer_tunnel::public_key_of(&private_key)
        );
        assert!(wireguard_public_key("not base64!").is_err());
    }

    /// The WireGuard family reaches egress through exactly the seam the SSH
    /// family does: one loopback SOCKS5 front, one `socks5h` URL, the same
    /// revision keying. Run against a real second WireGuard device, so this
    /// proves the tunnel came up and not merely that a socket was bound.
    #[tokio::test(flavor = "multi_thread")]
    async fn resolving_a_wireguard_endpoint_starts_a_loopback_socks5_front() {
        let peer = scryer_tunnel::test_support::WireGuardTestPeer::start().await;
        let config = wireguard_config("wg-endpoint-front", &peer.endpoint().to_string());
        assert_eq!(config.kind(), ProxyKind::Tunnel);

        let endpoint = resolve_tunnel_endpoint(&config).expect("the engine starts a front");
        let port = endpoint
            .egress_url
            .strip_prefix("socks5h://127.0.0.1:")
            .unwrap_or_else(|| panic!("unexpected egress url: {}", endpoint.egress_url))
            .parse::<u16>()
            .expect("port");
        assert_ne!(port, 0);
        let front = std::net::TcpStream::connect(("127.0.0.1", port)).expect("front is listening");
        assert!(front.peer_addr().expect("peer").ip().is_loopback());
        drop(front);

        // The same revision reuses the same front; an operator edit moves it.
        assert_eq!(
            resolve_tunnel_endpoint(&config).expect("reuse").egress_url,
            endpoint.egress_url
        );
        let mut edited = config.clone();
        edited.updated_at = config.updated_at + chrono::Duration::seconds(1);
        assert_ne!(
            resolve_tunnel_endpoint(&edited)
                .expect("restart")
                .egress_url,
            endpoint.egress_url
        );

        // And the tunnel really carries bytes: a request through the front
        // reaches an origin that only exists inside the tunnel.
        let client =
            crate::transport_proxy::transport_proxied_reqwest_client(&edited, "").expect("client");
        let response = client
            .get(format!(
                "http://{}:{}/",
                scryer_tunnel::test_support::TEST_PEER_ADDRESS,
                scryer_tunnel::test_support::TEST_PEER_HTTP_PORT
            ))
            .send()
            .await
            .expect("the request must travel through the tunnel");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.text().await.expect("body"), "through the tunnel");
        assert_eq!(peer.requests().len(), 1, "{:?}", peer.requests());

        // WireGuard has no trust-on-first-use step, so nothing was pinned.
        assert!(
            TunnelHostKeyLedger::shared()
                .take("wg-endpoint-front")
                .is_none(),
            "a WireGuard tunnel must never queue a host key pin"
        );

        stop_tunnel(&config.id);
    }

    /// The health probe's entry point, against the same real peer.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_wireguard_probe_reports_the_peer_and_our_own_public_key() {
        let peer = scryer_tunnel::test_support::WireGuardTestPeer::start().await;
        let config = wireguard_config("wg-probe", &peer.endpoint().to_string());

        let handshake = probe_wireguard_handshake(&config)
            .await
            .expect("the probe should complete a handshake");
        assert_eq!(handshake.endpoint, peer.endpoint().to_string());
        assert_eq!(
            handshake.peer_public_key,
            scryer_tunnel::public_key_of(&scryer_tunnel::test_support::test_peer_private_key())
        );
        assert_eq!(
            handshake.our_public_key,
            scryer_tunnel::public_key_of(&scryer_tunnel::test_support::test_client_private_key())
        );
    }

    #[test]
    fn learned_host_keys_queue_for_the_repository() {
        let ledger = TunnelHostKeyLedger::shared();
        ledger.record("proxy-pin-test", "SHA256:one");
        ledger.record("proxy-pin-test", "SHA256:two");
        // Latest observation wins, one entry per proxy.
        let taken = ledger.take("proxy-pin-test").expect("a queued pin");
        assert_eq!(taken.fingerprint, "SHA256:two");
        assert!(ledger.take("proxy-pin-test").is_none());
    }
}
