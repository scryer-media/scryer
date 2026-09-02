//! A real WireGuard peer, in process, for tests.
//!
//! The engine is tested against an actual second gotatun device rather than a
//! mock of one: a real Noise handshake over a real loopback UDP socket, real
//! encryption, and a second smoltcp stack on the far side answering real TCP
//! and real DNS. That is what makes assertions like "the name was resolved
//! *through* the tunnel" mean something — the peer's DNS log is the proof, and
//! the operating system's resolver is demonstrably never asked.
//!
//! The keys are derived in code from fixed seeds. Nothing that protects
//! anything is committed, and an X25519 secret is just 32 bytes, so a fixed
//! pattern is as good a key as a random one.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gotatun::device::{Device, DeviceBuilder, Peer};
use gotatun::x25519::{PublicKey, StaticSecret};
use ipnetwork::IpNetwork;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use crate::wireguard::adapter::ip_channels;
use crate::wireguard::spec::{DEFAULT_WIREGUARD_KEEPALIVE, DEFAULT_WIREGUARD_MTU, IpCidr};
use crate::wireguard::stack::{StackConfig, WgStack};
use crate::wireguard::{WgTransports, WireGuardSpec};

/// The peer's tunnel address — the "server" side of the point-to-point link.
pub const TEST_PEER_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 63, 0, 1);
/// The client's tunnel address, and what the peer routes to.
pub const TEST_CLIENT_ADDRESS: Ipv4Addr = Ipv4Addr::new(10, 63, 0, 2);
/// The port the peer's echo/HTTP service listens on inside the tunnel.
pub const TEST_PEER_HTTP_PORT: u16 = 8080;

/// Deterministic key material. Never used outside tests.
pub fn test_key(seed: u8) -> [u8; 32] {
    std::array::from_fn(|index| {
        seed.wrapping_mul(97)
            .wrapping_add((index as u8).wrapping_mul(13))
            .wrapping_add(index as u8)
    })
}

/// The client's private key in every test spec.
pub fn test_client_private_key() -> [u8; 32] {
    test_key(11)
}

/// The peer's private key.
pub fn test_peer_private_key() -> [u8; 32] {
    test_key(29)
}

/// A preshared key, for the tests that use one.
pub fn test_preshared_key() -> [u8; 32] {
    test_key(43)
}

/// How the peer should behave.
#[derive(Clone)]
pub struct WireGuardTestPeerOptions {
    /// The peer's own private key. Defaults to [`test_peer_private_key`].
    pub private_key: [u8; 32],
    /// The client public key it will accept. Defaults to the public half of
    /// [`test_client_private_key`].
    pub client_public_key: [u8; 32],
    /// Preshared key the peer requires, if any.
    pub preshared_key: Option<[u8; 32]>,
    /// Names the peer's resolver knows.
    pub names: HashMap<String, Vec<IpAddr>>,
    /// The port the echo/HTTP service binds inside the tunnel.
    pub http_port: u16,
    /// Body the HTTP service answers with.
    pub body: String,
}

impl Default for WireGuardTestPeerOptions {
    fn default() -> Self {
        let mut names = HashMap::new();
        names.insert(
            "origin.tunnel.test".to_string(),
            vec![IpAddr::V4(TEST_PEER_ADDRESS)],
        );
        Self {
            private_key: test_peer_private_key(),
            client_public_key: *PublicKey::from(&StaticSecret::from(test_client_private_key()))
                .as_bytes(),
            preshared_key: None,
            names,
            http_port: TEST_PEER_HTTP_PORT,
            body: "through the tunnel".to_string(),
        }
    }
}

/// A running WireGuard peer: a gotatun device on loopback UDP with its own
/// smoltcp stack, a TCP service and a DNS resolver.
pub struct WireGuardTestPeer {
    endpoint: SocketAddr,
    public_key: [u8; 32],
    client_public_key: [u8; 32],
    preshared_key: Option<[u8; 32]>,
    http_port: u16,
    dns_queries: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<Notify>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _device: Device<WgTransports>,
    _stack: WgStack,
    _pump: tokio::task::JoinHandle<()>,
}

impl WireGuardTestPeer {
    /// Start a peer with the default behaviour.
    pub async fn start() -> Self {
        Self::start_with(WireGuardTestPeerOptions::default()).await
    }

    /// Start a peer.
    pub async fn start_with(options: WireGuardTestPeerOptions) -> Self {
        // Ask the operating system for a free UDP port, then let gotatun bind
        // it. A test-only race, and the alternative — reading the port back
        // off the device — is not exposed by gotatun.
        let port = {
            let probe = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("a free UDP port");
            probe.local_addr().expect("the probe's address").port()
        };
        let endpoint = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

        let shutdown = Arc::new(Notify::new());
        let (to_stack, to_device, inbound, outbound) = ip_channels(DEFAULT_WIREGUARD_MTU);
        let (stack, pump) = WgStack::start(
            StackConfig {
                addresses: vec![IpCidr::host(IpAddr::V4(TEST_PEER_ADDRESS))],
                dns_servers: Vec::new(),
                mtu: DEFAULT_WIREGUARD_MTU,
                proxy_config_id: "wireguard-test-peer".to_string(),
            },
            inbound,
            outbound,
            Arc::clone(&shutdown),
            &tokio::runtime::Handle::current(),
        );

        let mut peer =
            Peer::new(PublicKey::from(options.client_public_key)).with_allowed_ips(vec![
                IpNetwork::new(IpAddr::V4(TEST_CLIENT_ADDRESS), 32).expect("a /32"),
            ]);
        peer.preshared_key = options.preshared_key;

        let device = DeviceBuilder::new()
            .with_default_udp()
            .with_ip_pair(to_stack, to_device)
            .with_listen_port(port)
            .with_private_key(StaticSecret::from(options.private_key))
            .with_peer(peer)
            .build()
            .await
            .expect("the test peer's device binds");

        let dns_queries = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        tasks.push(tokio::spawn(serve_http(
            stack.clone(),
            options.http_port,
            options.body.clone(),
            Arc::clone(&requests),
        )));
        tasks.push(tokio::spawn(serve_dns(
            stack.clone(),
            options.names.clone(),
            Arc::clone(&dns_queries),
        )));

        Self {
            endpoint,
            public_key: *PublicKey::from(&StaticSecret::from(options.private_key)).as_bytes(),
            client_public_key: options.client_public_key,
            preshared_key: options.preshared_key,
            http_port: options.http_port,
            dns_queries,
            requests,
            shutdown,
            tasks,
            _device: device,
            _stack: stack,
            _pump: pump,
        }
    }

    /// The peer's public UDP endpoint.
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    /// The peer's public key, as a client's `[Peer] PublicKey` must list it.
    pub fn public_key(&self) -> [u8; 32] {
        self.public_key
    }

    /// The peer's tunnel address.
    pub fn tunnel_address(&self) -> IpAddr {
        IpAddr::V4(TEST_PEER_ADDRESS)
    }

    /// The port the peer's TCP service is on, inside the tunnel.
    pub fn http_port(&self) -> u16 {
        self.http_port
    }

    /// Names the peer's resolver was asked about. The proof that a lookup
    /// travelled through the tunnel rather than to the operating system.
    pub fn dns_queries(&self) -> Vec<String> {
        self.dns_queries.lock().expect("dns queries").clone()
    }

    /// Bytes the peer has received from the client, and how long ago the
    /// client last shook hands. Proof of whether a client device is still
    /// talking: a stopped one sends nothing, not even keepalives.
    pub async fn client_rx_bytes(&self) -> usize {
        self._device
            .peers()
            .await
            .into_iter()
            .find(|entry| entry.peer.public_key.as_bytes() == &self.client_public_key)
            .map(|entry| entry.stats.rx_bytes)
            .unwrap_or_default()
    }

    /// Request lines the peer's TCP service received.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }

    /// A [`WireGuardSpec`] a client can use to reach this peer.
    pub fn client_spec(&self, proxy_config_id: &str) -> WireGuardSpec {
        WireGuardSpec {
            proxy_config_id: proxy_config_id.to_string(),
            proxy_name: "Test VPN".to_string(),
            revision: format!("{proxy_config_id}@v1"),
            endpoint_host: self.endpoint.ip().to_string(),
            endpoint_port: self.endpoint.port(),
            private_key: test_client_private_key(),
            peer_public_key: self.public_key,
            preshared_key: self.preshared_key,
            addresses: vec![IpCidr::host(IpAddr::V4(TEST_CLIENT_ADDRESS))],
            dns_servers: vec![IpAddr::V4(TEST_PEER_ADDRESS)],
            mtu: DEFAULT_WIREGUARD_MTU,
            persistent_keepalive: Some(DEFAULT_WIREGUARD_KEEPALIVE),
            request_timeout: Duration::from_secs(10),
        }
    }
}

impl Drop for WireGuardTestPeer {
    fn drop(&mut self) {
        self.shutdown.notify_waiters();
        for task in &self.tasks {
            task.abort();
        }
        self._pump.abort();
    }
}

/// A tiny HTTP origin inside the tunnel: answers every request with the same
/// body, and records the request line.
async fn serve_http(stack: WgStack, port: u16, body: String, requests: Arc<Mutex<Vec<String>>>) {
    let Ok(mut listener) = stack.listen(port, 4) else {
        return;
    };
    loop {
        let Ok(mut stream) = listener.accept().await else {
            return;
        };
        let body = body.clone();
        let requests = Arc::clone(&requests);
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 8192];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            if read == 0 {
                return;
            }
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            requests
                .lock()
                .expect("requests")
                .push(request.lines().next().unwrap_or_default().to_string());
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
            // Give the FIN a chance to leave before the socket is retired.
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
    }
}

/// A DNS resolver inside the tunnel, answering from a fixed table.
async fn serve_dns(
    stack: WgStack,
    names: HashMap<String, Vec<IpAddr>>,
    log: Arc<Mutex<Vec<String>>>,
) {
    let Ok(socket) = stack.bind_udp(53) else {
        return;
    };
    let mut buffer = vec![0u8; 1500];
    loop {
        let Ok((read, from)) = socket.recv_from(&mut buffer).await else {
            return;
        };
        let Some((name, answer)) = build_dns_answer(&buffer[..read], &names) else {
            continue;
        };
        log.lock().expect("dns log").push(name);
        let _ = socket.send_to(&answer, from).await;
    }
}

/// Parse one DNS query and compose the response.
///
/// Hand-rolled because smoltcp's `DnsRepr` only emits *queries*; a resolver has
/// to write answer records, which means writing the wire format directly.
/// Returns the queried name and the response bytes.
fn build_dns_answer(
    query: &[u8],
    names: &HashMap<String, Vec<IpAddr>>,
) -> Option<(String, Vec<u8>)> {
    if query.len() < 12 {
        return None;
    }
    let transaction_id = u16::from_be_bytes([query[0], query[1]]);
    if u16::from_be_bytes([query[4], query[5]]) != 1 {
        return None;
    }

    // Walk the question's labels. No compression pointers: a query never has
    // one, because there is nothing earlier in the packet to point at.
    let mut cursor = 12;
    let mut labels = Vec::new();
    loop {
        let length = *query.get(cursor)? as usize;
        cursor += 1;
        if length == 0 {
            break;
        }
        if length & 0xc0 != 0 {
            return None;
        }
        let label = query.get(cursor..cursor + length)?;
        labels.push(String::from_utf8_lossy(label).to_string());
        cursor += length;
    }
    let question_end = cursor + 4;
    let query_type = u16::from_be_bytes([*query.get(cursor)?, *query.get(cursor + 1)?]);
    let name = labels.join(".");

    let wanted_v4 = query_type == 1;
    let wanted_v6 = query_type == 28;
    if !wanted_v4 && !wanted_v6 {
        return None;
    }

    let answers: Vec<IpAddr> = names
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(&name))
        .map(|(_, addresses)| addresses.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|address| address.is_ipv4() == wanted_v4)
        .collect();

    let mut response = Vec::with_capacity(query.len() + answers.len() * 32);
    response.extend_from_slice(&transaction_id.to_be_bytes());
    // QR=1 (response), RD and RA copied/asserted, RCODE 0 — or NXDOMAIN when
    // the table has nothing.
    let flags: u16 = if answers.is_empty() { 0x8183 } else { 0x8180 };
    response.extend_from_slice(&flags.to_be_bytes());
    response.extend_from_slice(&1u16.to_be_bytes()); // question count
    response.extend_from_slice(&(answers.len() as u16).to_be_bytes());
    response.extend_from_slice(&0u16.to_be_bytes()); // authority count
    response.extend_from_slice(&0u16.to_be_bytes()); // additional count
    // Echo the question verbatim, which is what the client matches on.
    response.extend_from_slice(query.get(12..question_end)?);

    for address in &answers {
        // The name again, uncompressed. Compression is optional and a client
        // that cannot read an uncompressed name is broken.
        for label in &labels {
            response.push(label.len() as u8);
            response.extend_from_slice(label.as_bytes());
        }
        response.push(0);
        response.extend_from_slice(&query_type.to_be_bytes());
        response.extend_from_slice(&1u16.to_be_bytes()); // class IN
        response.extend_from_slice(&60u32.to_be_bytes()); // ttl
        match address {
            IpAddr::V4(address) => {
                response.extend_from_slice(&4u16.to_be_bytes());
                response.extend_from_slice(&address.octets());
            }
            IpAddr::V6(address) => {
                response.extend_from_slice(&16u16.to_be_bytes());
                response.extend_from_slice(&address.octets());
            }
        }
    }

    Some((name, response))
}
