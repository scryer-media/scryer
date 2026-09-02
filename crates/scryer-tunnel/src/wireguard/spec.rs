//! What the engine needs to bring up one WireGuard tunnel, with no domain
//! types and no third-party types in the public surface.
//!
//! The deliberate omission from [`WireGuardSpec`] is a peer *name*: WireGuard
//! has no trust-on-first-use step and no host key to learn. The peer's public
//! key **is** the identity, it is configured up front, and a peer that does not
//! hold the matching private key simply never answers. That is why this family
//! never calls [`TunnelObserver::host_key_pinned`].
//!
//! [`TunnelObserver::host_key_pinned`]: crate::TunnelObserver::host_key_pinned

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::time::Duration;

use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;

use crate::error::TunnelError;

/// WireGuard's own default MTU, and ours. 1280 is the IPv6 minimum MTU, which
/// is the one number that never needs path-MTU discovery to survive: it fits
/// inside every tunnel-over-tunnel arrangement an operator is likely to build
/// (a 1500-byte link, minus WireGuard's 60-byte overhead, minus whatever their
/// own uplink costs) without us having to guess their path.
pub const DEFAULT_WIREGUARD_MTU: u16 = 1280;

/// Any smaller and IPv6 cannot be carried at all (RFC 8200 § 5), and a TCP
/// handshake barely fits.
pub const MIN_WIREGUARD_MTU: u16 = 1280;

/// The ceiling a jumbo-frame operator could plausibly want. Above this the
/// packet buffers gotatun hands us stop being big enough.
pub const MAX_WIREGUARD_MTU: u16 = 3800;

/// `wg-quick`'s own recommendation, and the value that keeps a NAT binding
/// alive on every consumer router worth worrying about.
pub const DEFAULT_WIREGUARD_KEEPALIVE: u16 = 25;

/// The one operator-facing sentence about WireGuard keys. It lives next to the
/// parser that enforces it so the save path, the connect path and the health
/// probe all say the same thing, and so the web form can echo it as help text.
pub const WIREGUARD_KEY_MESSAGE: &str = "WireGuard keys are 32 bytes of base64, exactly as `wg genkey` prints them \
     (44 characters ending in `=`)";

/// An address with a prefix length, e.g. `10.6.0.2/32`.
///
/// This is a local type rather than `smoltcp::wire::IpCidr` on purpose: WP8
/// has to build one of these from an operator's text field, and it must not
/// have to depend on smoltcp to do so. [`FromStr`] accepts exactly what an
/// operator pastes out of a `wg` config's `Address =` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpCidr {
    /// The interface's own address inside the tunnel.
    pub address: IpAddr,
    /// Prefix length. Defaults to the host route (`/32`, `/128`) when the
    /// operator omits it, which is what a point-to-point tunnel almost always
    /// wants.
    pub prefix_len: u8,
}

impl IpCidr {
    /// A host route: the single address, nothing else on-link.
    pub fn host(address: IpAddr) -> Self {
        let prefix_len = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        Self {
            address,
            prefix_len,
        }
    }

    /// True when this is an IPv4 address.
    pub fn is_ipv4(&self) -> bool {
        self.address.is_ipv4()
    }

    /// True when this is an IPv6 address.
    pub fn is_ipv6(&self) -> bool {
        self.address.is_ipv6()
    }

    fn max_prefix_len(&self) -> u8 {
        if self.is_ipv4() { 32 } else { 128 }
    }
}

impl fmt::Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix_len)
    }
}

impl FromStr for IpCidr {
    type Err = TunnelError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address = IpAddr::from_str(address.trim()).map_err(|_| {
            TunnelError::Configuration(format!(
                "`{text}` is not a tunnel address; expected something like `10.6.0.2/32`"
            ))
        })?;
        let cidr = match prefix {
            None => IpCidr::host(address),
            Some(prefix) => {
                let prefix_len = u8::from_str(prefix.trim()).map_err(|_| {
                    TunnelError::Configuration(format!(
                        "`{text}` has an unreadable prefix length; expected something like \
                         `10.6.0.2/32`"
                    ))
                })?;
                IpCidr {
                    address,
                    prefix_len,
                }
            }
        };
        if cidr.prefix_len > cidr.max_prefix_len() {
            return Err(TunnelError::Configuration(format!(
                "`{text}` has a prefix length above /{}",
                cidr.max_prefix_len()
            )));
        }
        Ok(cidr)
    }
}

/// Everything the engine needs to bring up one WireGuard tunnel.
///
/// The caller decrypts credentials (they are already plaintext in memory by the
/// time a config leaves the store) and hands them over per tunnel start; this
/// crate never sees an encryption key and never writes any of this anywhere.
#[derive(Clone)]
pub struct WireGuardSpec {
    /// Proxy config id. The registry key, and the id every observation is
    /// reported under.
    pub proxy_config_id: String,
    /// The operator's name for this proxy, for messages.
    pub proxy_name: String,
    /// `id@updated_at`. A change restarts the tunnel; a health write does not
    /// change it, so a flapping tunnel does not churn sessions.
    pub revision: String,
    /// The peer's public UDP endpoint host. Resolved with the **OS** resolver:
    /// this one hop is outside the tunnel by definition, because it is the hop
    /// that carries the tunnel.
    pub endpoint_host: String,
    /// The peer's public UDP endpoint port.
    pub endpoint_port: u16,
    /// Our X25519 private key, from the operator's base64 `PrivateKey`.
    pub private_key: [u8; 32],
    /// The peer's X25519 public key, from the operator's base64 `PublicKey`.
    /// This is the whole of the peer's identity — see the module docs.
    pub peer_public_key: [u8; 32],
    /// The optional symmetric `PresharedKey`.
    pub preshared_key: Option<[u8; 32]>,
    /// The interface's own addresses inside the tunnel. At least one; v4
    /// and/or v6. An address family with no address here is not carried.
    pub addresses: Vec<IpCidr>,
    /// Resolvers to use *through* the tunnel. May be empty, in which case
    /// names cannot be resolved at all — see [`crate::wireguard::WgStack::resolve`].
    pub dns_servers: Vec<IpAddr>,
    /// Tunnel MTU. [`DEFAULT_WIREGUARD_MTU`] unless the operator knows better.
    pub mtu: u16,
    /// `PersistentKeepalive`, in seconds. [`DEFAULT_WIREGUARD_KEEPALIVE`] is
    /// the sane default; `None` disables it.
    pub persistent_keepalive: Option<u16>,
    /// Budget for bringing the tunnel up, and for a single dial through it.
    pub request_timeout: Duration,
}

impl fmt::Debug for WireGuardSpec {
    /// Hand-written so a stray `{:?}` in a log line can never print key
    /// material. The peer's public key is deliberately still visible — it is
    /// the identity an operator compares against their server — but only its
    /// first eight characters, which is enough to tell two peers apart and not
    /// enough to fill a log line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WireGuardSpec")
            .field("proxy_config_id", &self.proxy_config_id)
            .field("proxy_name", &self.proxy_name)
            .field("revision", &self.revision)
            .field("endpoint_host", &self.endpoint_host)
            .field("endpoint_port", &self.endpoint_port)
            .field("private_key", &"<redacted>")
            .field("peer_public_key", &self.peer_public_key_prefix())
            .field(
                "preshared_key",
                &self.preshared_key.as_ref().map(|_| "<redacted>"),
            )
            .field("addresses", &self.addresses)
            .field("dns_servers", &self.dns_servers)
            .field("mtu", &self.mtu)
            .field("persistent_keepalive", &self.persistent_keepalive)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl WireGuardSpec {
    /// `host:port`, never credentials.
    pub fn endpoint_description(&self) -> String {
        format!("{}:{}", self.endpoint_host, self.endpoint_port)
    }

    /// The first eight base64 characters of the peer's public key, followed by
    /// an ellipsis. Enough to identify a peer in a log line or a health
    /// message; never enough to be mistaken for the key itself.
    pub fn peer_public_key_prefix(&self) -> String {
        let encoded = BASE64_STANDARD.encode(self.peer_public_key);
        format!("{}…", &encoded[..8.min(encoded.len())])
    }

    /// What [`crate::TunnelProvider::describe`] reports. Never a private key.
    pub fn describe(&self) -> String {
        format!(
            "wireguard {}@{}",
            self.peer_public_key_prefix(),
            self.endpoint_description()
        )
    }

    /// Our own public key, base64, as the peer's config must list it.
    pub fn public_key(&self) -> String {
        public_key_of(&self.private_key)
    }

    /// True when the interface carries an IPv4 address.
    pub fn has_ipv4(&self) -> bool {
        self.addresses.iter().any(IpCidr::is_ipv4)
    }

    /// True when the interface carries an IPv6 address.
    pub fn has_ipv6(&self) -> bool {
        self.addresses.iter().any(IpCidr::is_ipv6)
    }

    /// The allowed IPs to configure on the peer.
    ///
    /// Fixed to the default routes, because this tunnel is only ever dialled
    /// *into*: every packet the stack emits is destined for something on the
    /// far side, so every destination must route to the one peer. A family
    /// with no interface address is omitted — there is no source address to
    /// send it from, so accepting it would only mean accepting inbound packets
    /// we can never answer.
    pub fn allowed_ips(&self) -> Vec<IpCidr> {
        let mut allowed = Vec::with_capacity(2);
        if self.has_ipv4() {
            allowed.push(IpCidr {
                address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                prefix_len: 0,
            });
        }
        if self.has_ipv6() {
            allowed.push(IpCidr {
                address: IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                prefix_len: 0,
            });
        }
        allowed
    }

    /// Everything that can be checked without talking to anyone.
    ///
    /// Called on the connect path so a half-configured proxy fails with the
    /// real reason instead of a handshake timeout, and callable by WP8 at save
    /// time for the same message.
    pub fn validate(&self) -> Result<(), TunnelError> {
        if self.endpoint_host.trim().is_empty() {
            return Err(TunnelError::Configuration(
                "the tunnel has no endpoint host".to_string(),
            ));
        }
        if self.endpoint_port == 0 {
            return Err(TunnelError::Configuration(
                "the tunnel has no endpoint port".to_string(),
            ));
        }
        if self.addresses.is_empty() {
            return Err(TunnelError::Configuration(
                "the tunnel has no interface address; add the `Address` line from the \
                 WireGuard configuration (for example `10.6.0.2/32`)"
                    .to_string(),
            ));
        }
        if self.mtu < MIN_WIREGUARD_MTU || self.mtu > MAX_WIREGUARD_MTU {
            return Err(TunnelError::Configuration(format!(
                "the tunnel MTU must be between {MIN_WIREGUARD_MTU} and {MAX_WIREGUARD_MTU}; \
                 {} is outside that range",
                self.mtu
            )));
        }
        if self.private_key == [0u8; 32] {
            return Err(TunnelError::Configuration(
                "the tunnel has no private key".to_string(),
            ));
        }
        if self.peer_public_key == [0u8; 32] {
            return Err(TunnelError::Configuration(
                "the tunnel has no peer public key".to_string(),
            ));
        }
        // A peer whose public key is our own public key can never complete a
        // handshake, and the mistake (pasting the wrong half of a key pair) is
        // common enough to be worth naming.
        if public_key_of(&self.private_key) == BASE64_STANDARD.encode(self.peer_public_key) {
            return Err(TunnelError::Configuration(
                "the peer public key is this tunnel's own public key; the `PublicKey` in the \
                 `[Peer]` section must be the *server's* key"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// Decode an operator-pasted base64 WireGuard key.
///
/// Accepts exactly what `wg genkey`/`wg pubkey` print: 32 bytes, standard
/// base64 with padding. Whitespace around it is tolerated because it is pasted.
pub fn parse_key(base64: &str) -> Result<[u8; 32], TunnelError> {
    let trimmed = base64.trim();
    if trimmed.is_empty() {
        return Err(TunnelError::Configuration(format!(
            "the key is empty; {WIREGUARD_KEY_MESSAGE}"
        )));
    }
    let bytes = BASE64_STANDARD.decode(trimmed).map_err(|_| {
        TunnelError::Configuration(format!(
            "the key is not valid base64; {WIREGUARD_KEY_MESSAGE}"
        ))
    })?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        TunnelError::Configuration(format!(
            "the key decodes to {} bytes, not 32; {WIREGUARD_KEY_MESSAGE}",
            bytes.len()
        ))
    })
}

/// The base64 public key belonging to a private key, as `wg pubkey` prints it.
///
/// WP8 shows this to the operator: it is the line they have to paste into the
/// server's `[Peer]` section, and it is the only way they can tell that the
/// private key they pasted is the one the server was configured for.
pub fn public_key_of(private_key: &[u8; 32]) -> String {
    let secret = gotatun::x25519::StaticSecret::from(*private_key);
    let public = gotatun::x25519::PublicKey::from(&secret);
    BASE64_STANDARD.encode(public.as_bytes())
}

/// Validate operator-pasted WireGuard key material without connecting anywhere.
///
/// The same parser the connect path uses, so a key refused at save time is
/// refused for exactly the reason it would have failed later. This is the
/// WireGuard counterpart of [`crate::validate_private_key`] and WP8 calls it
/// from the same place.
pub fn validate_wireguard_keys(
    private_key: &str,
    peer_public_key: &str,
    preshared_key: Option<&str>,
) -> Result<(), TunnelError> {
    let private = parse_key(private_key).map_err(|error| prefix(error, "private key"))?;
    let peer = parse_key(peer_public_key).map_err(|error| prefix(error, "peer public key"))?;
    if let Some(preshared) = preshared_key.map(str::trim).filter(|key| !key.is_empty()) {
        parse_key(preshared).map_err(|error| prefix(error, "preshared key"))?;
    }
    if public_key_of(&private) == BASE64_STANDARD.encode(peer) {
        return Err(TunnelError::Configuration(
            "the peer public key is this tunnel's own public key; the `PublicKey` in the \
             `[Peer]` section must be the *server's* key"
                .to_string(),
        ));
    }
    Ok(())
}

/// Say which of the three keys the operator has to go and fix.
fn prefix(error: TunnelError, which: &str) -> TunnelError {
    match error {
        TunnelError::Configuration(message) => {
            TunnelError::Configuration(format!("{which}: {message}"))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic material: an X25519 secret is 32 arbitrary bytes, so a
    /// fixed pattern is as good a key as any and nothing real is committed.
    fn key(seed: u8) -> [u8; 32] {
        std::array::from_fn(|index| seed.wrapping_mul(31).wrapping_add(index as u8))
    }

    fn spec() -> WireGuardSpec {
        WireGuardSpec {
            proxy_config_id: "proxy-1".to_string(),
            proxy_name: "Seedbox VPN".to_string(),
            revision: "proxy-1@now".to_string(),
            endpoint_host: "vpn.test".to_string(),
            endpoint_port: 51820,
            private_key: key(1),
            peer_public_key: key(2),
            preshared_key: Some(key(3)),
            addresses: vec!["10.6.0.2/32".parse().expect("address")],
            dns_servers: vec![IpAddr::V4(Ipv4Addr::new(10, 6, 0, 1))],
            mtu: DEFAULT_WIREGUARD_MTU,
            persistent_keepalive: Some(DEFAULT_WIREGUARD_KEEPALIVE),
            request_timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn the_spec_debug_impl_never_prints_key_material() {
        let spec = spec();
        let rendered = format!("{spec:?}");
        assert!(
            !rendered.contains(&BASE64_STANDARD.encode(spec.private_key)),
            "{rendered}"
        );
        assert!(
            !rendered.contains(&BASE64_STANDARD.encode(spec.preshared_key.expect("psk"))),
            "{rendered}"
        );
        // The raw byte arrays must not leak either.
        assert!(!rendered.contains("31, 62"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        // The public half of the identity is deliberately still visible.
        assert!(rendered.contains("vpn.test"), "{rendered}");
        assert!(
            rendered.contains(&spec.peer_public_key_prefix()),
            "{rendered}"
        );
    }

    #[test]
    fn describe_names_the_peer_and_endpoint_without_key_material() {
        let spec = spec();
        let described = spec.describe();
        assert!(described.starts_with("wireguard "), "{described}");
        assert!(described.ends_with("@vpn.test:51820"), "{described}");
        assert!(
            !described.contains(&BASE64_STANDARD.encode(spec.private_key)),
            "{described}"
        );
        // Eight characters of the peer key plus the ellipsis, and no more.
        let full = BASE64_STANDARD.encode(spec.peer_public_key);
        assert!(described.contains(&full[..8]), "{described}");
        assert!(!described.contains(&full), "{described}");
    }

    #[test]
    fn a_cidr_parses_with_and_without_a_prefix() {
        assert_eq!(
            "10.6.0.2/24".parse::<IpCidr>().expect("v4"),
            IpCidr {
                address: IpAddr::V4(Ipv4Addr::new(10, 6, 0, 2)),
                prefix_len: 24
            }
        );
        assert_eq!(
            " 10.6.0.2 ".parse::<IpCidr>().expect("v4 host"),
            IpCidr::host(IpAddr::V4(Ipv4Addr::new(10, 6, 0, 2)))
        );
        assert_eq!(
            "fd00::2".parse::<IpCidr>().expect("v6 host").prefix_len,
            128
        );
        assert_eq!("fd00::2/64".parse::<IpCidr>().expect("v6").prefix_len, 64);
    }

    #[test]
    fn an_unusable_cidr_says_what_was_expected() {
        for text in [
            "",
            "not-an-address",
            "10.6.0.2/33",
            "10.6.0.2/x",
            "fd00::2/129",
        ] {
            let error = text.parse::<IpCidr>().expect_err(text);
            let TunnelError::Configuration(message) = error else {
                panic!("expected a configuration error for {text:?}");
            };
            assert!(!message.is_empty(), "{text}");
        }
    }

    #[test]
    fn allowed_ips_follow_the_families_the_interface_actually_has() {
        let mut spec = spec();
        assert_eq!(
            spec.allowed_ips(),
            vec![IpCidr {
                address: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                prefix_len: 0
            }]
        );

        spec.addresses.push("fd00::2/128".parse().expect("v6"));
        assert_eq!(spec.allowed_ips().len(), 2, "both families are carried");

        spec.addresses.retain(IpCidr::is_ipv6);
        assert_eq!(
            spec.allowed_ips(),
            vec![IpCidr {
                address: IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                prefix_len: 0
            }],
            "a family with no interface address is not accepted from the peer"
        );
    }

    #[test]
    fn a_key_round_trips_through_base64() {
        let encoded = BASE64_STANDARD.encode(key(7));
        assert_eq!(parse_key(&encoded).expect("key"), key(7));
        assert_eq!(parse_key(&format!("  {encoded}\n")).expect("key"), key(7));
    }

    #[test]
    fn an_unusable_key_says_what_wireguard_keys_look_like() {
        for text in ["", "not base64!", &BASE64_STANDARD.encode([0u8; 16])] {
            let error = parse_key(text).expect_err(text);
            let TunnelError::Configuration(message) = error else {
                panic!("expected a configuration error for {text:?}");
            };
            assert!(message.contains(WIREGUARD_KEY_MESSAGE), "{message}");
        }
    }

    #[test]
    fn key_validation_names_which_key_is_wrong() {
        let private = BASE64_STANDARD.encode(key(1));
        let peer = BASE64_STANDARD.encode(key(2));
        validate_wireguard_keys(&private, &peer, None).expect("valid");
        validate_wireguard_keys(&private, &peer, Some("  ")).expect("blank preshared key is none");
        validate_wireguard_keys(&private, &peer, Some(&BASE64_STANDARD.encode(key(3))))
            .expect("valid with a preshared key");

        let error = validate_wireguard_keys("nonsense", &peer, None).expect_err("bad private");
        assert!(error.to_string().starts_with("private key: "), "{error}");
        let error = validate_wireguard_keys(&private, "nonsense", None).expect_err("bad peer");
        assert!(
            error.to_string().starts_with("peer public key: "),
            "{error}"
        );
        let error =
            validate_wireguard_keys(&private, &peer, Some("nonsense")).expect_err("bad preshared");
        assert!(error.to_string().starts_with("preshared key: "), "{error}");
    }

    #[test]
    fn pointing_a_tunnel_at_its_own_public_key_is_refused() {
        let private = BASE64_STANDARD.encode(key(1));
        let own_public = public_key_of(&key(1));
        let error = validate_wireguard_keys(&private, &own_public, None).expect_err("same key");
        assert!(error.to_string().contains("own public key"), "{error}");

        let mut spec = spec();
        spec.peer_public_key = parse_key(&own_public).expect("own public key");
        let error = spec.validate().expect_err("same key");
        assert!(error.to_string().contains("own public key"), "{error}");
    }

    #[test]
    fn validation_names_every_missing_piece() {
        let cases: Vec<(fn(&mut WireGuardSpec), &str)> = vec![
            (|spec| spec.endpoint_host = String::new(), "endpoint host"),
            (|spec| spec.endpoint_port = 0, "endpoint port"),
            (|spec| spec.addresses.clear(), "interface address"),
            (|spec| spec.mtu = 500, "MTU"),
            (|spec| spec.private_key = [0u8; 32], "private key"),
            (|spec| spec.peer_public_key = [0u8; 32], "peer public key"),
        ];
        for (break_it, expected) in cases {
            let mut spec = spec();
            break_it(&mut spec);
            let error = spec.validate().expect_err(expected);
            assert!(error.to_string().contains(expected), "{error}");
        }
        spec().validate().expect("a complete spec validates");
    }

    #[test]
    fn our_public_key_is_the_one_wireguard_would_derive() {
        // x25519 clamps the scalar, so the derivation must go through the
        // library rather than being reimplemented; this pins that it is
        // stable and that the encoding is `wg pubkey`-shaped.
        let public = public_key_of(&key(1));
        assert_eq!(public.len(), 44, "{public}");
        assert!(public.ends_with('='), "{public}");
        assert_eq!(public, public_key_of(&key(1)), "derivation is stable");
        assert_ne!(public, public_key_of(&key(2)));
    }
}
