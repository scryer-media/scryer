//! The WireGuard implementation of [`TunnelProvider`], entirely in process.
//!
//! ```text
//!   caller ──▶ SOCKS5 front ──▶ WireGuardTunnelProvider::dial
//!                                 │
//!                                 ├─ resolve the name *through* the tunnel (smoltcp DNS)
//!                                 └─ open a TCP socket on the tunnel's smoltcp interface
//!                                       │
//!                                    in-memory IpSend / IpRecv adapter
//!                                       │
//!                                    gotatun device — handshake, encrypt, rekey, keepalive
//!                                       │
//!                                    Tokio UDP socket on an ephemeral local port
//! ```
//!
//! Nothing here needs a `tun` device, a route, a firewall rule or a
//! capability: the "network interface" is a pair of channels, and the only
//! socket the operating system sees is one ordinary UDP socket. That is what
//! makes a VPN tunnel usable from inside a container that has no `NET_ADMIN`,
//! and it is why this can be a per-proxy tunnel rather than a machine-wide one.
//!
//! ## What each half owns
//!
//! * **gotatun** owns the protocol: the Noise handshake, encryption, rekeying,
//!   keepalives, peer state, the WireGuard timers, and the UDP transport.
//! * **smoltcp** owns everything above IP: the interface, TCP, UDP and DNS.
//! * This module owns the seam and the lifecycle, and nothing else.
//!
//! ## Identity
//!
//! There is no trust-on-first-use step and no host key to learn, so
//! [`TunnelObserver::host_key_pinned`] is never called for this family. The
//! peer's public key *is* its identity, the operator configures it up front,
//! and a peer that does not hold the matching private key simply never
//! answers — which is also why a wrong key and an unreachable endpoint are
//! indistinguishable from here, and why [`TunnelError::WireGuardConnect`]
//! does not pretend otherwise.

mod adapter;
mod phy;
mod spec;
mod stack;
mod tcp;
mod udp;

#[cfg(any(test, feature = "test-support"))]
mod test_peer;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use gotatun::device::{Device, DeviceBuilder, Peer};
use gotatun::packet::{Ip, IpNextProtocol, Ipv4Header, Packet};
use gotatun::udp::socket::UdpSocketFactory;
use gotatun::x25519::{PublicKey, StaticSecret};
use ipnetwork::IpNetwork;
use tokio::sync::Notify;

use crate::error::TunnelError;
use crate::provider::{TunnelObserver, TunnelProvider, TunnelStream};

pub use spec::{
    DEFAULT_WIREGUARD_KEEPALIVE, DEFAULT_WIREGUARD_MTU, IpCidr, MAX_WIREGUARD_MTU,
    MIN_WIREGUARD_MTU, WIREGUARD_KEY_MESSAGE, WireGuardSpec, parse_key, public_key_of,
    validate_wireguard_keys,
};
pub use stack::{MAX_SOCKETS, WgStack};
pub use tcp::WgTcpStream;
pub use udp::WgUdpSocket;

#[cfg(any(test, feature = "test-support"))]
pub use test_peer::{
    TEST_CLIENT_ADDRESS, TEST_PEER_ADDRESS, TEST_PEER_HTTP_PORT, WireGuardTestPeer,
    WireGuardTestPeerOptions, test_client_private_key, test_key, test_peer_private_key,
    test_preshared_key,
};

pub(crate) use adapter::ip_channels;
pub(crate) use stack::StackConfig;

/// gotatun's own point of no return: after three times `REJECT_AFTER_TIME`
/// with no handshake it clears its keys and the session is unrecoverable.
/// A tunnel that has been silent for this long is dead, not slow.
const SESSION_EXPIRY: Duration = Duration::from_secs(180 * 3);

/// How often to look for the first handshake while bringing a tunnel up.
const HANDSHAKE_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// WireGuard's `REKEY_AFTER_TIME`. A peer that completed a handshake more
/// recently than this has proven, cryptographically, that it is alive, so a
/// dial that answered nothing inside such a window is the *destination*
/// keeping quiet (a firewalled port drops SYNs without a word), not the
/// tunnel. Rebuilding the session for every such dial would churn a healthy
/// tunnel on every unreachable host behind it.
const RECENT_HANDSHAKE: Duration = Duration::from_secs(120);

/// The transports one of our devices is built from.
pub(crate) type WgTransports = (
    UdpSocketFactory,
    adapter::DeviceToStack,
    adapter::StackToDevice,
);

/// What one successful handshake established. Returned by the health probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireGuardHandshake {
    /// `host:port` of the peer's UDP endpoint, as configured.
    pub endpoint: String,
    /// The peer's base64 public key. Its identity, and safe to show.
    pub peer_public_key: String,
    /// *Our* base64 public key — the line the operator must have put in the
    /// server's `[Peer]` section. Reporting it is what turns "it does not
    /// connect" into "you configured the wrong key over there".
    pub our_public_key: String,
    /// How long the handshake took, measured from the moment the tunnel began
    /// coming up.
    pub handshake_at: Duration,
}

/// One live WireGuard tunnel: a gotatun device, a smoltcp stack, and the pump
/// that joins them.
struct WgTunnel {
    stack: WgStack,
    /// `Option` only so [`Drop`] can take it and drop it *inside* the runtime
    /// context; it is `Some` for the whole life of the tunnel.
    device: Option<Device<WgTransports>>,
    pump: tokio::task::JoinHandle<()>,
    shutdown: Arc<Notify>,
    peer_public_key: PublicKey,
    handshake_at: Duration,
    /// The runtime the device's tasks live on. gotatun stops a device on drop
    /// by spawning onto `Handle::try_current()`, and silently leaks it — UDP
    /// socket, keepalives and all — when there is no current runtime. The
    /// registry drops tunnels from plain threads (a config update on the
    /// blocking plugin worker, `stop_all` at shutdown), so the drop has to
    /// enter this handle first.
    runtime: tokio::runtime::Handle,
}

impl WgTunnel {
    /// Whether this tunnel is still worth dialling on.
    ///
    /// Two ways to be dead: the stack pump stopped (which also means the
    /// device's channels are closed), or the peer has gone quiet for longer
    /// than WireGuard itself will tolerate, at which point gotatun has thrown
    /// its keys away and no amount of waiting brings the session back.
    async fn is_alive(&self) -> bool {
        if self.pump.is_finished() {
            return false;
        }
        match self.last_handshake_age().await {
            Some(age) => age < SESSION_EXPIRY,
            None => false,
        }
    }

    async fn last_handshake_age(&self) -> Option<Duration> {
        match self.device.as_ref() {
            Some(device) => last_handshake_age(device, &self.peer_public_key).await,
            None => None,
        }
    }
}

/// How long ago the device last completed a handshake with `peer`, if ever.
async fn last_handshake_age(device: &Device<WgTransports>, peer: &PublicKey) -> Option<Duration> {
    device
        .peers()
        .await
        .into_iter()
        .find(|entry| entry.peer.public_key == *peer)
        .and_then(|entry| entry.stats.last_handshake)
}

impl Drop for WgTunnel {
    fn drop(&mut self) {
        // Stop the pump first: that closes the adapter channels, which is how
        // the device's own tasks learn to stop.
        self.shutdown.notify_waiters();
        self.pump.abort();
        // Then the device, from inside its runtime's context whatever thread
        // this drop runs on — see the field's comment for why that matters.
        let _entered = self.runtime.enter();
        drop(self.device.take());
    }
}

/// A WireGuard tunnel to one configured peer.
pub struct WireGuardTunnelProvider {
    spec: WireGuardSpec,
    observer: Arc<dyn TunnelObserver>,
    tunnel: tokio::sync::Mutex<Option<Arc<WgTunnel>>>,
}

impl WireGuardTunnelProvider {
    /// Build a provider. No I/O happens here.
    ///
    /// Deliberately the same shape as [`crate::SshTunnelProvider::new`]: the
    /// registry's factory closure is synchronous and is called from paths with
    /// no runtime at all (a blocking plugin worker), so bringing the tunnel up
    /// has to be the first dial's job. A failed bring-up therefore leaves the
    /// provider empty rather than poisoned, so the next dial tries again
    /// instead of the proxy being dead until a restart.
    pub fn new(spec: WireGuardSpec, observer: Arc<dyn TunnelObserver>) -> Self {
        Self {
            spec,
            observer,
            tunnel: tokio::sync::Mutex::new(None),
        }
    }

    /// Bring a tunnel up, report what the handshake established, tear it down.
    ///
    /// This is the health probe's entry point. Like the SSH one it deliberately
    /// does not touch the registry: probing must not leave a device behind, and
    /// it must fail with the real reason rather than "the proxy is unreachable".
    pub async fn handshake(
        spec: WireGuardSpec,
        observer: Arc<dyn TunnelObserver>,
    ) -> Result<WireGuardHandshake, TunnelError> {
        let endpoint = spec.endpoint_description();
        let peer_public_key = base64_of(&spec.peer_public_key);
        let our_public_key = spec.public_key();
        let provider = WireGuardTunnelProvider::new(spec, observer);
        let tunnel = provider.bring_up().await?;
        let handshake = WireGuardHandshake {
            endpoint,
            peer_public_key,
            our_public_key,
            handshake_at: tunnel.handshake_at,
        };
        drop(tunnel);
        Ok(handshake)
    }

    /// The tunnel's stack, bringing the tunnel up if there is none.
    ///
    /// Also the place a dead tunnel is noticed: a device whose peer has been
    /// silent past WireGuard's own expiry is replaced here rather than being
    /// discovered by a dial that then has to unwind.
    async fn tunnel(&self) -> Result<Arc<WgTunnel>, TunnelError> {
        let mut guard = self.tunnel.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.is_alive().await {
                return Ok(Arc::clone(existing));
            }
            tracing::debug!(
                proxy_config_id = self.spec.proxy_config_id.as_str(),
                tunnel = self.spec.describe().as_str(),
                "the WireGuard session went stale; bringing the tunnel up again"
            );
            *guard = None;
        }
        let tunnel = Arc::new(self.bring_up().await?);
        *guard = Some(Arc::clone(&tunnel));
        Ok(tunnel)
    }

    /// Drop `stale` if it is still the current tunnel, and build a new one.
    async fn replace_tunnel(&self, stale: &Arc<WgTunnel>) -> Result<Arc<WgTunnel>, TunnelError> {
        let mut guard = self.tunnel.lock().await;
        if let Some(current) = guard.as_ref()
            && !Arc::ptr_eq(current, stale)
        {
            // Another dial already rebuilt it; use theirs.
            return Ok(Arc::clone(current));
        }
        *guard = None;
        let tunnel = Arc::new(self.bring_up().await?);
        *guard = Some(Arc::clone(&tunnel));
        Ok(tunnel)
    }

    /// Everything between "a validated spec" and "a tunnel that has shaken
    /// hands": resolve the endpoint, start the stack, build the device, prove
    /// the peer answers.
    async fn bring_up(&self) -> Result<WgTunnel, TunnelError> {
        self.spec.validate()?;
        let started = std::time::Instant::now();

        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            TunnelError::Engine("a WireGuard tunnel needs a tokio runtime".to_string())
        })?;

        let endpoint = self.resolve_endpoint().await?;

        let (to_stack, to_device, inbound, outbound) = ip_channels(self.spec.mtu);
        let shutdown = Arc::new(Notify::new());
        let probe_channel = outbound.clone();
        let (stack, pump) = WgStack::start(
            StackConfig {
                addresses: self.spec.addresses.clone(),
                dns_servers: self.spec.dns_servers.clone(),
                mtu: self.spec.mtu,
                proxy_config_id: self.spec.proxy_config_id.clone(),
            },
            inbound,
            outbound,
            Arc::clone(&shutdown),
            &runtime,
        );

        let peer_public_key = PublicKey::from(self.spec.peer_public_key);
        let mut peer = Peer::new(peer_public_key)
            .with_endpoint(endpoint)
            .with_allowed_ips(allowed_networks(&self.spec));
        peer.preshared_key = self.spec.preshared_key;
        peer.keepalive = self.spec.persistent_keepalive;

        let device = DeviceBuilder::new()
            // An ephemeral local port on an ordinary UDP socket. No listen
            // port is configured: nothing dials *us*.
            .with_default_udp()
            .with_ip_pair(to_stack, to_device)
            .with_private_key(StaticSecret::from(self.spec.private_key))
            .with_peer(peer)
            .build()
            .await;

        let device = match device {
            Ok(device) => device,
            Err(error) => {
                shutdown.notify_waiters();
                pump.abort();
                return Err(TunnelError::WireGuardConnect {
                    host: self.spec.endpoint_host.clone(),
                    port: self.spec.endpoint_port,
                    detail: error.to_string(),
                });
            }
        };

        // Nudge the device into initiating. WireGuard only handshakes when it
        // has something to send, so a tunnel with no persistent keepalive
        // would otherwise sit idle until the first dial — and the point of
        // waiting here is that a wrong key or an unreachable endpoint surfaces
        // as a connect error rather than as a mysteriously slow first request.
        let _ = probe_channel.try_send(handshake_probe(&self.spec));

        let age = match self.await_handshake(&device, &peer_public_key, &pump).await {
            Ok(age) => age,
            Err(error) => {
                shutdown.notify_waiters();
                pump.abort();
                device.stop().await;
                return Err(error);
            }
        };
        let handshake_at = started.elapsed().saturating_sub(age);
        tracing::info!(
            proxy_config_id = self.spec.proxy_config_id.as_str(),
            tunnel = self.spec.describe().as_str(),
            handshake_ms = handshake_at.as_millis() as u64,
            "brought up a WireGuard tunnel"
        );

        Ok(WgTunnel {
            stack,
            device: Some(device),
            pump,
            shutdown,
            peer_public_key,
            handshake_at,
            runtime,
        })
    }

    /// Wait for the peer to answer, or say that it did not.
    async fn await_handshake(
        &self,
        device: &Device<WgTransports>,
        peer_public_key: &PublicKey,
        pump: &tokio::task::JoinHandle<()>,
    ) -> Result<Duration, TunnelError> {
        let waited = tokio::time::timeout(self.spec.request_timeout, async {
            loop {
                if let Some(age) = last_handshake_age(device, peer_public_key).await {
                    return age;
                }
                if pump.is_finished() {
                    // The stack died under us; there will never be a
                    // handshake, so do not sit out the whole budget.
                    return Duration::MAX;
                }
                tokio::time::sleep(HANDSHAKE_POLL_INTERVAL).await;
            }
        })
        .await;

        match waited {
            Ok(age) if age != Duration::MAX => Ok(age),
            Ok(_) => Err(TunnelError::WireGuardConnect {
                host: self.spec.endpoint_host.clone(),
                port: self.spec.endpoint_port,
                detail: "the tunnel's network stack stopped before the handshake completed"
                    .to_string(),
            }),
            Err(_) => Err(TunnelError::WireGuardConnect {
                host: self.spec.endpoint_host.clone(),
                port: self.spec.endpoint_port,
                detail: format!(
                    "the peer did not complete a handshake within {}s. WireGuard answers nothing \
                     it cannot authenticate, so this is what a wrong private key, a wrong peer \
                     public key, a wrong or missing preshared key and an unreachable endpoint all \
                     look like. Check that the server's `[Peer]` section lists this tunnel's \
                     public key ({}).",
                    self.spec.request_timeout.as_secs().max(1),
                    self.spec.public_key()
                ),
            }),
        }
    }

    /// Resolve the peer's endpoint with the **operating system's** resolver.
    ///
    /// This one name is outside the tunnel by definition: it is the hop that
    /// carries the tunnel, so there is nothing to resolve it through yet.
    async fn resolve_endpoint(&self) -> Result<SocketAddr, TunnelError> {
        let host = self.spec.endpoint_host.trim().to_string();
        if let Ok(address) = host.parse::<IpAddr>() {
            return Ok(SocketAddr::new(address, self.spec.endpoint_port));
        }

        let port = self.spec.endpoint_port;
        let resolved = tokio::time::timeout(
            self.spec.request_timeout,
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| TunnelError::WireGuardConnect {
            host: host.clone(),
            port,
            detail: "the endpoint name did not resolve in time".to_string(),
        })?;

        let mut addresses = resolved.map_err(|error| TunnelError::WireGuardConnect {
            host: host.clone(),
            port,
            detail: format!("the endpoint name could not be resolved: {error}"),
        })?;

        addresses
            .next()
            .ok_or_else(|| TunnelError::WireGuardConnect {
                host: host.clone(),
                port,
                detail: "the endpoint name resolved to no addresses".to_string(),
            })
    }

    /// One dial attempt on a tunnel that is already up.
    async fn dial_on(
        &self,
        tunnel: &WgTunnel,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn TunnelStream>, DialFailure> {
        let timeout = self.spec.request_timeout;
        let addresses = tunnel
            .stack
            .resolve(host, timeout)
            .await
            .map_err(|error| DialFailure::Destination(relabel(error, host, port)))?;

        let mut last = None;
        for address in addresses {
            match tunnel
                .stack
                .connect(SocketAddr::new(address, port), timeout)
                .await
            {
                Ok(stream) => return Ok(Box::new(stream) as Box<dyn TunnelStream>),
                Err(error) => last = Some(error),
            }
        }

        let error = last.unwrap_or_else(|| TunnelError::Dial {
            host: host.to_string(),
            port,
            detail: "the name resolved to no usable address".to_string(),
        });
        // A refusal is the destination's answer and proves the tunnel works; a
        // silence might be either, so it is the only failure allowed to cost a
        // reconnect.
        if is_silence(&error) {
            Err(DialFailure::Tunnel(error))
        } else {
            Err(DialFailure::Destination(error))
        }
    }
}

/// Whether a failed dial means "that destination is bad" or "this tunnel may
/// be gone".
enum DialFailure {
    Destination(TunnelError),
    Tunnel(TunnelError),
}

impl DialFailure {
    fn into_error(self) -> TunnelError {
        match self {
            DialFailure::Destination(error) | DialFailure::Tunnel(error) => error,
        }
    }
}

/// A dial that produced no answer at all, as opposed to a refusal.
fn is_silence(error: &TunnelError) -> bool {
    matches!(error, TunnelError::Dial { detail, .. } if detail.contains("in time"))
}

/// Re-label a resolver failure with the destination the caller asked for.
///
/// [`WgStack::resolve`] reports against the DNS port, because that is the hop
/// that failed; by the time it reaches an operator the interesting address is
/// the one they were trying to reach.
fn relabel(error: TunnelError, host: &str, port: u16) -> TunnelError {
    match error {
        TunnelError::Dial { detail, .. } => TunnelError::Dial {
            host: host.to_string(),
            port,
            detail,
        },
        other => other,
    }
}

#[async_trait::async_trait]
impl TunnelProvider for WireGuardTunnelProvider {
    async fn dial(&self, host: &str, port: u16) -> Result<Box<dyn TunnelStream>, TunnelError> {
        let tunnel = self.tunnel().await.inspect_err(|error| {
            self.observer
                .tunnel_dial_failed(&self.spec.proxy_config_id, &error.to_string());
        })?;

        let outcome = match self.dial_on(&tunnel, host, port).await {
            Ok(stream) => Ok(stream),
            Err(DialFailure::Destination(error)) => Err(error),
            Err(DialFailure::Tunnel(first)) if matches!(tunnel.last_handshake_age().await, Some(age) if age < RECENT_HANDSHAKE) =>
            {
                // The peer shook hands moments ago, so the tunnel is provably
                // up and the silence is the destination's. No rebuild.
                Err(first)
            }
            Err(DialFailure::Tunnel(first)) => {
                tracing::debug!(
                    proxy_config_id = self.spec.proxy_config_id.as_str(),
                    error = %first,
                    "the WireGuard tunnel answered nothing; bringing it up again once"
                );
                match self.replace_tunnel(&tunnel).await {
                    Ok(tunnel) => self
                        .dial_on(&tunnel, host, port)
                        .await
                        .map_err(DialFailure::into_error),
                    Err(error) => Err(error),
                }
            }
        };

        match outcome {
            Ok(stream) => {
                self.observer
                    .tunnel_dial_succeeded(&self.spec.proxy_config_id);
                Ok(stream)
            }
            Err(error) => {
                self.observer
                    .tunnel_dial_failed(&self.spec.proxy_config_id, &error.to_string());
                Err(error)
            }
        }
    }

    fn describe(&self) -> String {
        self.spec.describe()
    }
}

/// The peer's allowed IPs, as gotatun wants them.
fn allowed_networks(spec: &WireGuardSpec) -> Vec<IpNetwork> {
    spec.allowed_ips()
        .into_iter()
        .filter_map(|cidr| IpNetwork::new(cidr.address, cidr.prefix_len).ok())
        .collect()
}

/// A single inert IP packet, purely to make gotatun start a handshake.
///
/// gotatun encapsulates whatever the "tun device" hands it; with no session
/// yet it queues the packet and emits a handshake initiation instead, then
/// retransmits that on its own timer. The packet itself carries protocol 59
/// ("no next header") and no payload, so if the peer ever does decrypt it,
/// there is nothing there to act on.
fn handshake_probe(spec: &WireGuardSpec) -> Packet<Ip> {
    let source = spec
        .addresses
        .first()
        .map(|cidr| cidr.address)
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    // Addressed to ourselves: the destination only has to match the peer's
    // allowed IPs (which are the default routes), and pointing it anywhere
    // else would put an address on the wire that is not ours to use.
    match source {
        IpAddr::V4(address) => {
            let mut header = Ipv4Header::new(address, address, IpNextProtocol::Ipv6NoNxt, &[]);
            header.header_checksum = ipv4_header_checksum(&header).into();
            Packet::copy_from(&header)
                .into_bytes()
                .try_into_ip()
                .expect("a hand-built IPv4 header is a valid IP packet")
        }
        IpAddr::V6(address) => {
            let mut bytes = Vec::with_capacity(40);
            bytes.extend_from_slice(&[0x60, 0x00, 0x00, 0x00]); // version 6
            bytes.extend_from_slice(&0u16.to_be_bytes()); // payload length
            bytes.push(59); // no next header
            bytes.push(64); // hop limit
            bytes.extend_from_slice(&address.octets());
            bytes.extend_from_slice(&address.octets());
            Packet::copy_from(bytes.as_slice())
                .try_into_ip()
                .expect("a hand-built IPv6 header is a valid IP packet")
        }
    }
}

/// The standard one's-complement checksum over an IPv4 header whose own
/// checksum field is zero.
fn ipv4_header_checksum(header: &Ipv4Header) -> u16 {
    let bytes = header_bytes(header);
    let mut sum: u32 = 0;
    for word in bytes.chunks(2) {
        let value = match word {
            [high, low] => u16::from_be_bytes([*high, *low]),
            [high] => u16::from_be_bytes([*high, 0]),
            _ => unreachable!("chunks(2) yields one or two bytes"),
        };
        sum += u32::from(value);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// The header's own bytes, via a packet, so no zerocopy import is needed here.
fn header_bytes(header: &Ipv4Header) -> Vec<u8> {
    Packet::copy_from(header).into_bytes().as_ref().to_vec()
}

fn base64_of(key: &[u8; 32]) -> String {
    use base64::Engine as _;
    base64::prelude::BASE64_STANDARD.encode(key)
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use std::net::Ipv6Addr;
    use std::time::Duration;

    fn spec_with(address: &str) -> WireGuardSpec {
        WireGuardSpec {
            proxy_config_id: "probe".to_string(),
            proxy_name: "Probe".to_string(),
            revision: "probe@v1".to_string(),
            endpoint_host: "vpn.test".to_string(),
            endpoint_port: 51820,
            private_key: [1u8; 32],
            peer_public_key: [2u8; 32],
            preshared_key: None,
            addresses: vec![address.parse().expect("an address")],
            dns_servers: Vec::new(),
            mtu: DEFAULT_WIREGUARD_MTU,
            persistent_keepalive: None,
            request_timeout: Duration::from_secs(5),
        }
    }

    /// A packet the peer will drop for a bad checksum still triggers the
    /// handshake, so a wrong checksum here would be invisible in the
    /// end-to-end tests. Check it directly instead.
    #[test]
    fn the_ipv4_probe_is_a_well_formed_packet_with_a_valid_checksum() {
        let probe = handshake_probe(&spec_with("10.63.0.2/32"));
        assert_eq!(
            probe.destination(),
            Some(IpAddr::V4(Ipv4Addr::new(10, 63, 0, 2))),
            "gotatun routes the probe by its destination, so it must be readable"
        );

        let bytes = Packet::<[u8]>::from(probe);
        let bytes = bytes.as_ref();
        assert_eq!(bytes.len(), 20, "an IPv4 header and no payload");
        assert_eq!(bytes[0] >> 4, 4, "version");
        assert_eq!(bytes[9], 59, "no next header");

        // A header's checksum is valid when the one's-complement sum over the
        // whole header, checksum field included, is 0xffff.
        let mut sum: u32 = 0;
        for word in bytes.chunks(2) {
            sum += u32::from(u16::from_be_bytes([word[0], word[1]]));
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        assert_eq!(sum as u16, 0xffff, "the header checksum must verify");
    }

    #[test]
    fn the_ipv6_probe_is_a_well_formed_packet() {
        let probe = handshake_probe(&spec_with("fd00::2/128"));
        assert_eq!(
            probe.destination(),
            Some(IpAddr::V6(
                "fd00::2".parse::<Ipv6Addr>().expect("a v6 address")
            ))
        );

        let bytes = Packet::<[u8]>::from(probe);
        let bytes = bytes.as_ref();
        assert_eq!(bytes.len(), 40, "an IPv6 header and no payload");
        assert_eq!(bytes[0] >> 4, 6, "version");
        assert_eq!(
            u16::from_be_bytes([bytes[4], bytes[5]]),
            0,
            "payload length"
        );
        assert_eq!(bytes[6], 59, "no next header");
        assert_eq!(bytes[7], 64, "hop limit");
    }

    /// The probe must be inert: it carries nothing a peer could act on, and it
    /// never puts an address on the wire that is not the tunnel's own.
    #[test]
    fn the_probe_uses_only_the_tunnels_own_address() {
        let probe = handshake_probe(&spec_with("10.63.0.2/32"));
        assert_eq!(probe.source(), probe.destination());
    }
}
