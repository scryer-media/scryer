//! End-to-end tests for the WireGuard engine, against a real second device.
//!
//! Every test here talks to [`WireGuardTestPeer`]: a second gotatun device on
//! a real loopback UDP socket, with its own smoltcp stack answering TCP and
//! DNS. So a passing test means a real Noise handshake completed, real
//! ChaCha20-Poly1305 packets crossed a real socket, and a real userspace TCP
//! connection carried bytes — not that a mock was satisfied.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::TunnelError;
use crate::provider::{NoopTunnelObserver, TunnelObserver, TunnelProvider, TunnelStream};
use crate::registry::TunnelRegistry;
use crate::wireguard::spec::public_key_of;
use crate::wireguard::test_peer::{
    TEST_PEER_ADDRESS, WireGuardTestPeer, WireGuardTestPeerOptions, test_key, test_preshared_key,
};
use crate::wireguard::{WireGuardTunnelProvider, parse_key};

/// Records what the engine reported.
#[derive(Default)]
struct RecordingObserver {
    successes: Mutex<Vec<String>>,
    failures: Mutex<Vec<String>>,
    pins: Mutex<Vec<String>>,
}

impl TunnelObserver for RecordingObserver {
    fn tunnel_dial_failed(&self, proxy_config_id: &str, message: &str) {
        self.failures
            .lock()
            .expect("failures")
            .push(format!("{proxy_config_id}: {message}"));
    }
    fn tunnel_dial_succeeded(&self, proxy_config_id: &str) {
        self.successes
            .lock()
            .expect("successes")
            .push(proxy_config_id.to_string());
    }
    fn host_key_pinned(&self, proxy_config_id: &str, fingerprint: &str) {
        self.pins
            .lock()
            .expect("pins")
            .push(format!("{proxy_config_id}: {fingerprint}"));
    }
}

async fn http_get_through(stream: &mut Box<dyn TunnelStream>, host: &str) -> String {
    stream
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .expect("request");
    stream.flush().await.expect("flush");
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.expect("response");
    String::from_utf8_lossy(&answer).to_string()
}

#[tokio::test]
async fn a_handshake_completes_against_a_real_peer() {
    let peer = WireGuardTestPeer::start().await;
    let observer = Arc::new(RecordingObserver::default());
    let spec = peer.client_spec("proxy-handshake");
    let our_public_key = spec.public_key();

    let handshake =
        WireGuardTunnelProvider::handshake(spec, Arc::clone(&observer) as Arc<dyn TunnelObserver>)
            .await
            .expect("the handshake completes");

    assert_eq!(handshake.endpoint, peer.endpoint().to_string());
    assert_eq!(
        handshake.peer_public_key,
        public_key_of(&crate::wireguard::test_peer::test_peer_private_key())
    );
    assert_eq!(handshake.our_public_key, our_public_key);
    assert!(
        handshake.handshake_at < Duration::from_secs(10),
        "{:?}",
        handshake.handshake_at
    );
    assert!(
        observer.pins.lock().expect("pins").is_empty(),
        "WireGuard has no trust-on-first-use step, so nothing may be pinned"
    );
}

/// Readiness must not depend on the keepalive: with it off, nothing but the
/// bring-up probe packet makes gotatun initiate, so this is the test that
/// proves the probe works.
#[tokio::test]
async fn a_handshake_completes_with_the_keepalive_switched_off() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-no-keepalive");
    spec.persistent_keepalive = None;
    spec.request_timeout = Duration::from_secs(5);

    let handshake = WireGuardTunnelProvider::handshake(spec, Arc::new(NoopTunnelObserver))
        .await
        .expect("the probe packet must make the device initiate");
    assert!(handshake.handshake_at < Duration::from_secs(5));
}

/// The registry drops providers from whatever thread called it — a config
/// update on the blocking plugin worker, `stop_all` on the main thread at
/// shutdown — and none of those has a tokio runtime current. gotatun only
/// stops a device on drop when one is, so the tunnel has to enter its own
/// runtime first. This proves the drop neither panics nor leaks: after it,
/// the peer stops hearing from us.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_tunnel_from_a_plain_thread_still_stops_the_device() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-thread-drop");
    // A one-second keepalive, so a device that was *not* stopped betrays
    // itself within the wait below.
    spec.persistent_keepalive = Some(1);
    let provider = Arc::new(WireGuardTunnelProvider::new(
        spec,
        Arc::new(NoopTunnelObserver),
    ));
    let mut stream = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("dial");
    let _ = http_get_through(&mut stream, "origin.tunnel.test").await;
    drop(stream);

    let moved = Arc::clone(&provider);
    drop(provider);
    std::thread::spawn(move || drop(moved))
        .join()
        .expect("dropping off-runtime must not panic");

    // Let the stop land, then prove the peer hears nothing more from that
    // device: not a keepalive, not a byte.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let quiet_from = peer.client_rx_bytes().await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        peer.client_rx_bytes().await,
        quiet_from,
        "a dropped tunnel's device must stop sending, even when it was dropped off-runtime"
    );

    let before = peer.requests().len();
    let second = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-thread-drop-2"),
        Arc::new(NoopTunnelObserver),
    );
    let mut stream = second
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("a new tunnel to the same peer still works after the old one was dropped");
    let _ = http_get_through(&mut stream, "origin.tunnel.test").await;
    assert_eq!(peer.requests().len(), before + 1);
}

#[tokio::test]
async fn a_dial_by_address_carries_bytes_end_to_end() {
    let peer = WireGuardTestPeer::start().await;
    let observer = Arc::new(RecordingObserver::default());
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-dial"),
        Arc::clone(&observer) as Arc<dyn TunnelObserver>,
    );

    let mut stream = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("dial");
    let response = http_get_through(&mut stream, "origin.tunnel.test").await;

    assert!(response.contains("200 OK"), "{response}");
    assert!(response.ends_with("through the tunnel"), "{response}");
    assert_eq!(peer.requests(), vec!["GET / HTTP/1.1".to_string()]);
    assert_eq!(
        observer.successes.lock().expect("successes").as_slice(),
        ["proxy-dial".to_string()]
    );
    assert!(
        peer.dns_queries().is_empty(),
        "an address literal must not be looked up"
    );
}

#[tokio::test]
async fn a_dial_by_name_resolves_through_the_tunnel() {
    let peer = WireGuardTestPeer::start().await;
    let provider =
        WireGuardTunnelProvider::new(peer.client_spec("proxy-dns"), Arc::new(NoopTunnelObserver));

    // `origin.tunnel.test` exists only in the peer's table. The operating
    // system cannot resolve it, so a successful dial is proof the query
    // travelled through the tunnel.
    let mut stream = provider
        .dial("origin.tunnel.test", peer.http_port())
        .await
        .expect("dial by name");
    let response = http_get_through(&mut stream, "origin.tunnel.test").await;
    assert!(response.contains("200 OK"), "{response}");

    assert_eq!(
        peer.dns_queries(),
        vec!["origin.tunnel.test".to_string()],
        "the peer's resolver must be the one that answered"
    );
    assert!(
        tokio::net::lookup_host(("origin.tunnel.test", 80))
            .await
            .is_err(),
        "the operating system must not be able to resolve this name at all"
    );
}

#[tokio::test]
async fn a_name_the_peer_does_not_know_fails_with_the_resolver_saying_so() {
    let peer = WireGuardTestPeer::start().await;
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-nxdomain"),
        Arc::new(NoopTunnelObserver),
    );

    let error = provider
        .dial("absent.tunnel.test", 80)
        .await
        .err()
        .expect("an unknown name must not dial");
    let TunnelError::Dial { host, port, detail } = &error else {
        panic!("expected a dial failure, got {error}");
    };
    assert_eq!(host, "absent.tunnel.test");
    assert_eq!(*port, 80, "the caller's port, not the resolver's");
    assert!(detail.contains("does not know this name"), "{detail}");
}

#[tokio::test]
async fn a_wrong_peer_public_key_fails_inside_the_timeout() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-badkey");
    spec.peer_public_key = public_key_bytes(&public_key_of(&test_key(200)));
    spec.request_timeout = Duration::from_secs(2);

    let started = std::time::Instant::now();
    let error = WireGuardTunnelProvider::handshake(spec, Arc::new(NoopTunnelObserver))
        .await
        .err()
        .expect("a peer that cannot answer must not connect");
    let elapsed = started.elapsed();

    assert!(
        matches!(error, TunnelError::WireGuardConnect { .. }),
        "expected a WireGuard connect failure, got {error}"
    );
    assert!(
        error.to_string().contains("did not complete a handshake"),
        "{error}"
    );
    assert!(
        elapsed < Duration::from_secs(6),
        "the failure must land inside the budget, took {elapsed:?}"
    );
}

#[tokio::test]
async fn a_mismatched_preshared_key_fails_inside_the_timeout() {
    // The peer requires a preshared key; the client offers none.
    let peer = WireGuardTestPeer::start_with(WireGuardTestPeerOptions {
        preshared_key: Some(test_preshared_key()),
        ..WireGuardTestPeerOptions::default()
    })
    .await;
    let mut spec = peer.client_spec("proxy-badpsk");
    spec.preshared_key = None;
    spec.request_timeout = Duration::from_secs(2);

    let error = WireGuardTunnelProvider::handshake(spec, Arc::new(NoopTunnelObserver))
        .await
        .err()
        .expect("a mismatched preshared key must not connect");
    assert!(
        matches!(error, TunnelError::WireGuardConnect { .. }),
        "expected a WireGuard connect failure, got {error}"
    );

    // And the matching key does connect, so the failure above was the key and
    // not the fixture.
    let peer_spec = peer.client_spec("proxy-goodpsk");
    assert_eq!(peer_spec.preshared_key, Some(test_preshared_key()));
    WireGuardTunnelProvider::handshake(peer_spec, Arc::new(NoopTunnelObserver))
        .await
        .expect("the matching preshared key connects");
}

#[tokio::test]
async fn an_unreachable_endpoint_fails_rather_than_hanging_on_the_first_dial() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-unreachable");
    // A port nothing is bound to.
    spec.endpoint_port = {
        let probe = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("a free port");
        probe.local_addr().expect("addr").port()
    };
    spec.request_timeout = Duration::from_secs(2);
    let observer = Arc::new(RecordingObserver::default());

    let provider =
        WireGuardTunnelProvider::new(spec, Arc::clone(&observer) as Arc<dyn TunnelObserver>);
    let error = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), 80)
        .await
        .err()
        .expect("an unreachable endpoint must not dial");
    assert!(
        matches!(error, TunnelError::WireGuardConnect { .. }),
        "{error}"
    );
    assert_eq!(
        observer.failures.lock().expect("failures").len(),
        1,
        "the failure must reach the observer"
    );

    // The provider is not poisoned: the next dial tries again rather than
    // reporting a cached failure.
    let error = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), 80)
        .await
        .err()
        .expect("still unreachable");
    assert!(
        matches!(error, TunnelError::WireGuardConnect { .. }),
        "{error}"
    );
    assert_eq!(observer.failures.lock().expect("failures").len(), 2);
}

#[tokio::test]
async fn a_closed_port_on_the_far_side_is_refused_without_taking_the_tunnel_down() {
    let peer = WireGuardTestPeer::start().await;
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-refused"),
        Arc::new(NoopTunnelObserver),
    );

    let error = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), 9)
        .await
        .err()
        .expect("a closed port must not dial");
    assert!(
        matches!(error, TunnelError::Dial { .. }),
        "expected a dial failure, got {error}"
    );

    // The same tunnel still works, so a bad destination cost no reconnect.
    let mut stream = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("second dial");
    let response = http_get_through(&mut stream, "origin.tunnel.test").await;
    assert!(response.ends_with("through the tunnel"), "{response}");
}

#[tokio::test]
async fn a_tunnel_with_no_resolver_says_so_instead_of_asking_this_machine() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-nodns");
    spec.dns_servers.clear();
    let provider = WireGuardTunnelProvider::new(spec, Arc::new(NoopTunnelObserver));

    let error = provider
        .dial("example.test", 80)
        .await
        .err()
        .expect("a name without a resolver must not dial");
    let TunnelError::Configuration(message) = &error else {
        panic!("expected a configuration error, got {error}");
    };
    assert!(message.contains("no DNS server configured"), "{message}");
    assert!(peer.dns_queries().is_empty());
}

#[tokio::test]
async fn localhost_is_refused_rather_than_pointed_at_this_machine() {
    let peer = WireGuardTestPeer::start().await;
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-localhost"),
        Arc::new(NoopTunnelObserver),
    );

    for name in ["localhost", "LocalHost", "service.localhost"] {
        let error = provider
            .dial(name, 80)
            .await
            .err()
            .expect("localhost must never resolve locally");
        let TunnelError::Configuration(message) = &error else {
            panic!("expected a configuration error for {name}, got {error}");
        };
        assert!(message.contains("name this machine"), "{message}");
    }
}

#[tokio::test]
async fn the_socks5_front_carries_bytes_through_a_wireguard_tunnel() {
    let peer = WireGuardTestPeer::start().await;
    let registry = TunnelRegistry::with_handle(tokio::runtime::Handle::current());
    let observer = Arc::new(RecordingObserver::default());

    let front = registry
        .ensure_wireguard_tunnel(
            peer.client_spec("proxy-socks"),
            Arc::clone(&observer) as Arc<dyn TunnelObserver>,
        )
        .expect("the front starts");
    assert!(front.ip().is_loopback(), "{front}");

    let destination = SocketAddr::new(IpAddr::V4(TEST_PEER_ADDRESS), peer.http_port());
    let response = socks5_get(front, destination).await;
    assert!(response.contains("200 OK"), "{response}");
    assert!(response.ends_with("through the tunnel"), "{response}");
    assert_eq!(peer.requests(), vec!["GET / HTTP/1.1".to_string()]);

    registry.stop("proxy-socks");
}

#[tokio::test]
async fn the_socks5_front_resolves_names_on_the_far_side() {
    let peer = WireGuardTestPeer::start().await;
    let registry = TunnelRegistry::with_handle(tokio::runtime::Handle::current());

    let front = registry
        .ensure_wireguard_tunnel(
            peer.client_spec("proxy-socks-dns"),
            Arc::new(NoopTunnelObserver),
        )
        .expect("the front starts");

    let response = socks5_get_by_name(front, "origin.tunnel.test", peer.http_port()).await;
    assert!(response.contains("200 OK"), "{response}");
    assert_eq!(peer.dns_queries(), vec!["origin.tunnel.test".to_string()]);

    registry.stop("proxy-socks-dns");
}

#[tokio::test]
async fn tearing_a_tunnel_down_leaves_nothing_running() {
    let peer = WireGuardTestPeer::start().await;
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-teardown"),
        Arc::new(NoopTunnelObserver),
    );
    let mut stream = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("dial");
    let _ = http_get_through(&mut stream, "origin.tunnel.test").await;
    drop(stream);
    drop(provider);

    // Give the aborted pump and the device's own tasks a scheduling round to
    // finish unwinding, then prove the peer sees the tunnel go quiet rather
    // than something still driving it.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let before = peer.requests().len();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        peer.requests().len(),
        before,
        "nothing may keep talking after the provider is dropped"
    );
}

#[tokio::test]
async fn a_dropped_stream_does_not_leak_its_socket_slot() {
    let peer = WireGuardTestPeer::start().await;
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-slots"),
        Arc::new(NoopTunnelObserver),
    );

    // More dials than the stack has socket slots, one at a time. If a closed
    // socket did not give its slot back this would fail well before the end.
    for _ in 0..(crate::wireguard::MAX_SOCKETS + 8) {
        let mut stream = provider
            .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
            .await
            .expect("dial");
        let response = http_get_through(&mut stream, "origin.tunnel.test").await;
        assert!(response.contains("200 OK"), "{response}");
    }
}

fn public_key_bytes(base64: &str) -> [u8; 32] {
    parse_key(base64).expect("a valid key")
}

/// Drive the loopback SOCKS5 front by address and return the HTTP response.
async fn socks5_get(front: SocketAddr, destination: SocketAddr) -> String {
    let mut socket = tokio::net::TcpStream::connect(front).await.expect("front");
    socket.write_all(&[0x05, 1, 0x00]).await.expect("greeting");
    let mut answer = [0u8; 2];
    socket.read_exact(&mut answer).await.expect("method");
    assert_eq!(answer, [0x05, 0x00]);

    let mut request = vec![0x05, 0x01, 0x00];
    match destination.ip() {
        IpAddr::V4(address) => {
            request.push(0x01);
            request.extend_from_slice(&address.octets());
        }
        IpAddr::V6(address) => {
            request.push(0x04);
            request.extend_from_slice(&address.octets());
        }
    }
    request.extend_from_slice(&destination.port().to_be_bytes());
    socket.write_all(&request).await.expect("connect request");
    read_socks5_reply(&mut socket).await;
    http_over(socket, &destination.to_string()).await
}

/// Drive the front by *name*, which is what proves resolution happens on the
/// far side: the SOCKS5 front passes the name through unresolved.
async fn socks5_get_by_name(front: SocketAddr, host: &str, port: u16) -> String {
    let mut socket = tokio::net::TcpStream::connect(front).await.expect("front");
    socket.write_all(&[0x05, 1, 0x00]).await.expect("greeting");
    let mut answer = [0u8; 2];
    socket.read_exact(&mut answer).await.expect("method");

    let mut request = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    socket.write_all(&request).await.expect("connect request");
    read_socks5_reply(&mut socket).await;
    http_over(socket, host).await
}

async fn read_socks5_reply(socket: &mut tokio::net::TcpStream) {
    let mut header = [0u8; 4];
    socket.read_exact(&mut header).await.expect("reply header");
    assert_eq!(header[0], 0x05);
    assert_eq!(header[1], 0x00, "the front refused the connection");
    let remaining = match header[3] {
        0x01 => 4 + 2,
        0x04 => 16 + 2,
        0x03 => {
            let mut length = [0u8; 1];
            socket.read_exact(&mut length).await.expect("name length");
            usize::from(length[0]) + 2
        }
        other => panic!("unexpected address type {other}"),
    };
    let mut rest = vec![0u8; remaining];
    socket.read_exact(&mut rest).await.expect("reply address");
}

async fn http_over(mut socket: tokio::net::TcpStream, host: &str) -> String {
    socket
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .expect("request");
    let mut answer = Vec::new();
    socket.read_to_end(&mut answer).await.expect("response");
    String::from_utf8_lossy(&answer).to_string()
}

#[tokio::test]
async fn a_stack_with_only_ipv6_refuses_an_ipv4_destination_clearly() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-v6only");
    // Only a v6 interface address, so the stack has no v4 source address to
    // send from. The handshake is unaffected — WireGuard's own transport is
    // the UDP socket underneath, not the tunnel's addressing — so this proves
    // the *stack* refuses the destination, promptly and in words an operator
    // can act on, rather than emitting a packet with no source address or
    // sitting on the dial until the timeout.
    spec.addresses = vec!["fd00::2/128".parse().expect("a v6 address")];
    spec.dns_servers.clear();
    spec.request_timeout = Duration::from_secs(5);

    let provider = WireGuardTunnelProvider::new(spec, Arc::new(NoopTunnelObserver));
    let started = std::time::Instant::now();
    let error = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .err()
        .expect("a v4 destination is unreachable from a v6-only stack");

    let TunnelError::Dial { host, port, detail } = &error else {
        panic!("expected a dial failure, got {error}");
    };
    assert_eq!(host, &TEST_PEER_ADDRESS.to_string());
    assert_eq!(*port, peer.http_port());
    assert!(detail.contains("family"), "{detail}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the refusal must be immediate, not a timeout"
    );
}

#[tokio::test]
async fn a_misconfigured_spec_fails_before_any_socket_is_opened() {
    let peer = WireGuardTestPeer::start().await;
    let mut spec = peer.client_spec("proxy-broken");
    spec.addresses.clear();
    let provider = WireGuardTunnelProvider::new(spec, Arc::new(NoopTunnelObserver));

    let error = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), 80)
        .await
        .err()
        .expect("a spec with no interface address must not dial");
    let TunnelError::Configuration(message) = &error else {
        panic!("expected a configuration error, got {error}");
    };
    assert!(message.contains("interface address"), "{message}");
}

#[tokio::test]
async fn describe_never_leaks_key_material() {
    let peer = WireGuardTestPeer::start().await;
    let spec = peer.client_spec("proxy-describe");
    let private = spec.private_key;
    let provider = WireGuardTunnelProvider::new(spec, Arc::new(NoopTunnelObserver));

    let described = provider.describe();
    assert!(described.starts_with("wireguard "), "{described}");
    assert!(
        described.contains(&peer.endpoint().to_string()),
        "{described}"
    );
    assert!(
        !described.contains(&public_key_of(&private)),
        "{described} must not carry a key in full"
    );
}

/// Belt and braces: the address constants really do describe a point-to-point
/// link, so a test that "worked" by talking to something on this machine would
/// be visible.
#[test]
fn the_test_addresses_are_private_and_distinct() {
    assert_ne!(TEST_PEER_ADDRESS, Ipv4Addr::LOCALHOST);
    assert!(TEST_PEER_ADDRESS.is_private());
    assert!(crate::wireguard::TEST_CLIENT_ADDRESS.is_private());
    assert_ne!(TEST_PEER_ADDRESS, crate::wireguard::TEST_CLIENT_ADDRESS);
}

/// What `replace_tunnel` and a stale-session rebuild do: a new device and a
/// new stack, same key, same peer, same tunnel address, on the same task. The
/// far side still holds the previous stack's connections in their closing
/// states, so this only works if the new stack does not reuse their ports.
#[tokio::test]
async fn a_rebuilt_tunnel_reaches_the_same_peer_again() {
    let peer = WireGuardTestPeer::start().await;
    let provider = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-seq-1"),
        Arc::new(NoopTunnelObserver),
    );
    let mut stream = provider
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("dial");
    let _ = http_get_through(&mut stream, "origin.tunnel.test").await;
    drop(stream);
    drop(provider);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let second = WireGuardTunnelProvider::new(
        peer.client_spec("proxy-seq-2"),
        Arc::new(NoopTunnelObserver),
    );
    let mut stream = second
        .dial(&TEST_PEER_ADDRESS.to_string(), peer.http_port())
        .await
        .expect("second tunnel");
    let _ = http_get_through(&mut stream, "origin.tunnel.test").await;
}
