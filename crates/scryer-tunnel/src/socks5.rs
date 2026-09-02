//! A minimal SOCKS5 front, hand-rolled over tokio.
//!
//! This is the adaptor that makes a tunnel look like an ordinary transport
//! proxy: every existing egress site already knows how to hand reqwest a
//! `socks5h://` URL, so a tunnel publishes one and nothing else in the codebase
//! learns a new concept.
//!
//! Deliberate limits, because this front serves exactly one caller (our own
//! process, over loopback) and every additional feature is attack surface:
//!
//! * **Loopback peers only.** The listener binds `127.0.0.1:0`, and the peer
//!   address is re-checked on accept anyway.
//! * **No authentication.** Method `0x00` only; anything else is answered with
//!   `0xFF` (no acceptable methods). There is nothing to authenticate — reaching
//!   the socket already means running as this process.
//! * **CONNECT only.** BIND and UDP ASSOCIATE are answered `0x07`
//!   (command not supported).
//! * **All three address types.** IPv4, IPv6 and — the one that matters —
//!   domain names, which are passed to the provider unresolved so they resolve
//!   on the far side of the tunnel.
//! * **A handshake budget.** A peer that opens a socket and says nothing is
//!   dropped after the configured timeout instead of holding a task forever.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::TunnelError;
use crate::provider::{TunnelObserver, TunnelProvider, TunnelStream};

const SOCKS5_VERSION: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_NONE_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

const REPLY_SUCCEEDED: u8 = 0x00;
const REPLY_GENERAL_FAILURE: u8 = 0x01;
const REPLY_HOST_UNREACHABLE: u8 = 0x04;
const REPLY_CONNECTION_REFUSED: u8 = 0x05;
const REPLY_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REPLY_ADDRESS_TYPE_NOT_SUPPORTED: u8 = 0x08;

/// Everything one front needs to serve connections.
pub struct Socks5Front {
    provider: Arc<dyn TunnelProvider>,
    observer: Arc<dyn TunnelObserver>,
    proxy_config_id: String,
    timeout: Duration,
}

/// Why a single SOCKS5 conversation ended early. Never surfaced to an
/// operator — the peer is our own reqwest client, which reports the failure
/// in its own words — but traced, and mapped onto a SOCKS5 reply code.
#[derive(Debug)]
enum Socks5Failure {
    Io(std::io::Error),
    Protocol(&'static str),
    Dial(TunnelError),
}

impl std::fmt::Display for Socks5Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Socks5Failure::Io(error) => write!(f, "socket error: {error}"),
            Socks5Failure::Protocol(detail) => write!(f, "protocol error: {detail}"),
            Socks5Failure::Dial(error) => write!(f, "{error}"),
        }
    }
}

impl From<std::io::Error> for Socks5Failure {
    fn from(error: std::io::Error) -> Self {
        Socks5Failure::Io(error)
    }
}

/// A peer is allowed only if it is on the loopback interface.
///
/// The listener already binds a loopback address, so this can only fire if the
/// bind is ever changed; it is cheap insurance that a tunnel front never
/// becomes an open proxy for the network.
pub fn peer_is_permitted(peer: &SocketAddr) -> bool {
    peer.ip().is_loopback()
}

impl Socks5Front {
    pub fn new(
        provider: Arc<dyn TunnelProvider>,
        observer: Arc<dyn TunnelObserver>,
        proxy_config_id: String,
        timeout: Duration,
    ) -> Self {
        Self {
            provider,
            observer,
            proxy_config_id,
            timeout,
        }
    }

    /// Accept loop. Runs until `shutdown` is notified or the listener dies.
    pub async fn serve(self: Arc<Self>, listener: TcpListener, shutdown: Arc<tokio::sync::Notify>) {
        loop {
            let accepted = tokio::select! {
                biased;
                () = shutdown.notified() => break,
                accepted = listener.accept() => accepted,
            };
            match accepted {
                Ok((stream, peer)) => {
                    if !peer_is_permitted(&peer) {
                        tracing::warn!(
                            proxy_config_id = self.proxy_config_id.as_str(),
                            %peer,
                            "refused a non-loopback peer on a tunnel SOCKS5 front"
                        );
                        continue;
                    }
                    let front = Arc::clone(&self);
                    tokio::spawn(async move {
                        if let Err(failure) = front.handle_connection(stream).await {
                            tracing::debug!(
                                proxy_config_id = front.proxy_config_id.as_str(),
                                failure = %failure,
                                "tunnel SOCKS5 connection ended early"
                            );
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_config_id = self.proxy_config_id.as_str(),
                        error = %error,
                        "tunnel SOCKS5 front stopped accepting"
                    );
                    break;
                }
            }
        }
        tracing::debug!(
            proxy_config_id = self.proxy_config_id.as_str(),
            "tunnel SOCKS5 front stopped"
        );
    }

    async fn handle_connection(&self, mut stream: TcpStream) -> Result<(), Socks5Failure> {
        // The negotiation is bounded; the tunnelled conversation that follows
        // is not (an indexer response can legitimately take as long as the
        // request timeout the consumer's own client enforces).
        let (host, port) =
            match tokio::time::timeout(self.timeout, self.negotiate(&mut stream)).await {
                Ok(result) => result?,
                Err(_) => return Err(Socks5Failure::Protocol("peer did not finish the handshake")),
            };

        let dialled = tokio::time::timeout(self.timeout, self.provider.dial(&host, port)).await;
        let upstream = match dialled {
            Ok(Ok(upstream)) => upstream,
            Ok(Err(error)) => {
                self.report_failure(&error);
                let _ = reply(&mut stream, reply_code_for(&error)).await;
                return Err(Socks5Failure::Dial(error));
            }
            Err(_) => {
                let error = TunnelError::Dial {
                    host: host.clone(),
                    port,
                    detail: "the tunnel did not answer in time".to_string(),
                };
                self.report_failure(&error);
                let _ = reply(&mut stream, REPLY_HOST_UNREACHABLE).await;
                return Err(Socks5Failure::Dial(error));
            }
        };

        self.observer.tunnel_dial_succeeded(&self.proxy_config_id);
        reply(&mut stream, REPLY_SUCCEEDED).await?;
        self.pump(stream, upstream).await
    }

    fn report_failure(&self, error: &TunnelError) {
        self.observer
            .tunnel_dial_failed(&self.proxy_config_id, &error.to_string());
    }

    async fn pump(
        &self,
        mut stream: TcpStream,
        mut upstream: Box<dyn TunnelStream>,
    ) -> Result<(), Socks5Failure> {
        tokio::io::copy_bidirectional(&mut stream, &mut upstream)
            .await
            .map(|_| ())
            .map_err(Socks5Failure::Io)
    }

    /// Method negotiation followed by the CONNECT request. Returns the
    /// destination the peer asked for, unresolved.
    async fn negotiate(&self, stream: &mut TcpStream) -> Result<(String, u16), Socks5Failure> {
        let mut greeting = [0u8; 2];
        stream.read_exact(&mut greeting).await?;
        if greeting[0] != SOCKS5_VERSION {
            return Err(Socks5Failure::Protocol("peer is not speaking SOCKS5"));
        }
        let method_count = greeting[1] as usize;
        if method_count == 0 {
            stream
                .write_all(&[SOCKS5_VERSION, METHOD_NONE_ACCEPTABLE])
                .await?;
            return Err(Socks5Failure::Protocol("peer offered no auth methods"));
        }
        let mut methods = vec![0u8; method_count];
        stream.read_exact(&mut methods).await?;
        if !methods.contains(&METHOD_NO_AUTH) {
            stream
                .write_all(&[SOCKS5_VERSION, METHOD_NONE_ACCEPTABLE])
                .await?;
            return Err(Socks5Failure::Protocol("peer requires an auth method"));
        }
        stream.write_all(&[SOCKS5_VERSION, METHOD_NO_AUTH]).await?;

        let mut request = [0u8; 4];
        stream.read_exact(&mut request).await?;
        if request[0] != SOCKS5_VERSION {
            return Err(Socks5Failure::Protocol("request is not SOCKS5"));
        }
        if request[1] != CMD_CONNECT {
            let _ = reply(stream, REPLY_COMMAND_NOT_SUPPORTED).await;
            return Err(Socks5Failure::Protocol("only CONNECT is supported"));
        }

        let host = match request[3] {
            ATYP_IPV4 => {
                let mut octets = [0u8; 4];
                stream.read_exact(&mut octets).await?;
                IpAddr::V4(Ipv4Addr::from(octets)).to_string()
            }
            ATYP_IPV6 => {
                let mut octets = [0u8; 16];
                stream.read_exact(&mut octets).await?;
                IpAddr::V6(Ipv6Addr::from(octets)).to_string()
            }
            ATYP_DOMAIN => {
                let mut length = [0u8; 1];
                stream.read_exact(&mut length).await?;
                if length[0] == 0 {
                    let _ = reply(stream, REPLY_GENERAL_FAILURE).await;
                    return Err(Socks5Failure::Protocol("empty destination name"));
                }
                let mut name = vec![0u8; length[0] as usize];
                stream.read_exact(&mut name).await?;
                match String::from_utf8(name) {
                    Ok(name) => name,
                    Err(_) => {
                        let _ = reply(stream, REPLY_GENERAL_FAILURE).await;
                        return Err(Socks5Failure::Protocol("destination name is not UTF-8"));
                    }
                }
            }
            _ => {
                let _ = reply(stream, REPLY_ADDRESS_TYPE_NOT_SUPPORTED).await;
                return Err(Socks5Failure::Protocol("unsupported address type"));
            }
        };

        let mut port = [0u8; 2];
        stream.read_exact(&mut port).await?;
        Ok((host, u16::from_be_bytes(port)))
    }
}

/// Map a dial failure onto the closest SOCKS5 reply code. reqwest reports every
/// one of these as a connect error, which is what makes the proxy-hop
/// classification in `transport_proxy` fire.
fn reply_code_for(error: &TunnelError) -> u8 {
    match error {
        TunnelError::Dial { detail, .. } if detail.contains("ConnectFailed") => {
            REPLY_CONNECTION_REFUSED
        }
        TunnelError::Dial { .. } => REPLY_HOST_UNREACHABLE,
        _ => REPLY_GENERAL_FAILURE,
    }
}

/// A SOCKS5 reply with a zeroed bound address. The client never uses BND.ADDR
/// on a CONNECT, and there is no meaningful local address to report for a
/// stream that lives inside an SSH channel.
async fn reply(stream: &mut TcpStream, code: u8) -> std::io::Result<()> {
    stream
        .write_all(&[SOCKS5_VERSION, code, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0])
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A provider that hands back one end of an in-memory duplex and records
    /// what it was asked to dial. No SSH, no sockets.
    struct FakeProvider {
        dialled: Mutex<Vec<(String, u16)>>,
        outcome: Mutex<Option<TunnelError>>,
        echo_prefix: &'static str,
    }

    impl FakeProvider {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                dialled: Mutex::new(Vec::new()),
                outcome: Mutex::new(None),
                echo_prefix: "echo:",
            })
        }

        fn failing(error: TunnelError) -> Arc<Self> {
            Arc::new(Self {
                dialled: Mutex::new(Vec::new()),
                outcome: Mutex::new(Some(error)),
                echo_prefix: "echo:",
            })
        }

        fn dialled(&self) -> Vec<(String, u16)> {
            self.dialled.lock().expect("dialled").clone()
        }
    }

    #[async_trait::async_trait]
    impl TunnelProvider for FakeProvider {
        async fn dial(&self, host: &str, port: u16) -> Result<Box<dyn TunnelStream>, TunnelError> {
            self.dialled
                .lock()
                .expect("dialled")
                .push((host.to_string(), port));
            if let Some(error) = self.outcome.lock().expect("outcome").clone() {
                return Err(error);
            }
            let (near, mut far) = tokio::io::duplex(64 * 1024);
            let prefix = self.echo_prefix;
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 1024];
                while let Ok(read) = far.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    let mut answer = prefix.as_bytes().to_vec();
                    answer.extend_from_slice(&buffer[..read]);
                    if far.write_all(&answer).await.is_err() {
                        break;
                    }
                }
            });
            Ok(Box::new(near))
        }

        fn describe(&self) -> String {
            "fake tunnel".to_string()
        }
    }

    struct RecordingObserver {
        failures: Mutex<Vec<String>>,
        successes: Mutex<Vec<String>>,
    }

    impl RecordingObserver {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                failures: Mutex::new(Vec::new()),
                successes: Mutex::new(Vec::new()),
            })
        }
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
        fn host_key_pinned(&self, _proxy_config_id: &str, _fingerprint: &str) {}
    }

    async fn start_front(
        provider: Arc<dyn TunnelProvider>,
        observer: Arc<dyn TunnelObserver>,
    ) -> (SocketAddr, Arc<tokio::sync::Notify>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let front = Arc::new(Socks5Front::new(
            provider,
            observer,
            "proxy-front".to_string(),
            Duration::from_secs(5),
        ));
        tokio::spawn(front.serve(listener, Arc::clone(&shutdown)));
        (addr, shutdown)
    }

    async fn greet(stream: &mut TcpStream) -> [u8; 2] {
        stream
            .write_all(&[SOCKS5_VERSION, 1, METHOD_NO_AUTH])
            .await
            .expect("greeting");
        let mut answer = [0u8; 2];
        stream.read_exact(&mut answer).await.expect("method reply");
        answer
    }

    async fn read_reply(stream: &mut TcpStream) -> [u8; 10] {
        let mut reply = [0u8; 10];
        stream.read_exact(&mut reply).await.expect("reply");
        reply
    }

    #[tokio::test]
    async fn a_connect_to_a_domain_name_is_dialled_unresolved_and_copies_both_ways() {
        let provider = FakeProvider::new();
        let observer = RecordingObserver::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::clone(&observer) as Arc<dyn TunnelObserver>,
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        assert_eq!(greet(&mut stream).await, [SOCKS5_VERSION, METHOD_NO_AUTH]);

        let name = b"indexer.internal";
        let mut request = vec![
            SOCKS5_VERSION,
            CMD_CONNECT,
            0x00,
            ATYP_DOMAIN,
            name.len() as u8,
        ];
        request.extend_from_slice(name);
        request.extend_from_slice(&443u16.to_be_bytes());
        stream.write_all(&request).await.expect("request");

        let reply = read_reply(&mut stream).await;
        assert_eq!(reply[0], SOCKS5_VERSION);
        assert_eq!(reply[1], REPLY_SUCCEEDED);

        // The name is handed over unresolved: remote DNS is the whole point.
        assert_eq!(
            provider.dialled(),
            vec![("indexer.internal".to_string(), 443)]
        );

        stream.write_all(b"ping").await.expect("payload");
        let mut answer = vec![0u8; 9];
        stream.read_exact(&mut answer).await.expect("echo");
        assert_eq!(&answer, b"echo:ping");

        assert_eq!(
            observer.successes.lock().expect("successes").as_slice(),
            ["proxy-front".to_string()]
        );
    }

    #[tokio::test]
    async fn a_connect_to_an_ipv4_literal_is_dialled() {
        let provider = FakeProvider::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::new(crate::provider::NoopTunnelObserver),
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        greet(&mut stream).await;
        let mut request = vec![SOCKS5_VERSION, CMD_CONNECT, 0x00, ATYP_IPV4];
        request.extend_from_slice(&[10, 1, 2, 3]);
        request.extend_from_slice(&8080u16.to_be_bytes());
        stream.write_all(&request).await.expect("request");
        assert_eq!(read_reply(&mut stream).await[1], REPLY_SUCCEEDED);
        assert_eq!(provider.dialled(), vec![("10.1.2.3".to_string(), 8080)]);
    }

    #[tokio::test]
    async fn a_connect_to_an_ipv6_literal_is_dialled() {
        let provider = FakeProvider::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::new(crate::provider::NoopTunnelObserver),
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        greet(&mut stream).await;
        let mut request = vec![SOCKS5_VERSION, CMD_CONNECT, 0x00, ATYP_IPV6];
        request.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        request.extend_from_slice(&9090u16.to_be_bytes());
        stream.write_all(&request).await.expect("request");
        assert_eq!(read_reply(&mut stream).await[1], REPLY_SUCCEEDED);
        assert_eq!(provider.dialled(), vec![("::1".to_string(), 9090)]);
    }

    #[tokio::test]
    async fn bind_is_rejected_as_an_unsupported_command() {
        let provider = FakeProvider::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::new(crate::provider::NoopTunnelObserver),
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        greet(&mut stream).await;
        let mut request = vec![SOCKS5_VERSION, 0x02, 0x00, ATYP_IPV4];
        request.extend_from_slice(&[127, 0, 0, 1]);
        request.extend_from_slice(&80u16.to_be_bytes());
        stream.write_all(&request).await.expect("request");
        assert_eq!(
            read_reply(&mut stream).await[1],
            REPLY_COMMAND_NOT_SUPPORTED
        );
        assert!(provider.dialled().is_empty(), "BIND must not dial");
    }

    #[tokio::test]
    async fn a_non_socks5_greeting_is_refused() {
        let provider = FakeProvider::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::new(crate::provider::NoopTunnelObserver),
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream.write_all(&[0x04, 0x01]).await.expect("greeting");
        let mut answer = [0u8; 1];
        // The front closes without answering a version it does not speak.
        assert!(matches!(stream.read_exact(&mut answer).await, Err(_)));
        assert!(provider.dialled().is_empty());
    }

    #[tokio::test]
    async fn an_authenticated_greeting_is_refused_with_no_acceptable_methods() {
        let provider = FakeProvider::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::new(crate::provider::NoopTunnelObserver),
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        // Username/password only.
        stream
            .write_all(&[SOCKS5_VERSION, 1, 0x02])
            .await
            .expect("greeting");
        let mut answer = [0u8; 2];
        stream.read_exact(&mut answer).await.expect("method reply");
        assert_eq!(answer, [SOCKS5_VERSION, METHOD_NONE_ACCEPTABLE]);
        assert!(provider.dialled().is_empty());
    }

    #[tokio::test]
    async fn a_refused_dial_is_reported_to_the_observer_and_answered_with_a_failure_code() {
        let provider = FakeProvider::failing(TunnelError::Dial {
            host: "indexer.internal".to_string(),
            port: 443,
            detail: "ConnectFailed".to_string(),
        });
        let observer = RecordingObserver::new();
        let (addr, _shutdown) = start_front(
            Arc::clone(&provider) as Arc<dyn TunnelProvider>,
            Arc::clone(&observer) as Arc<dyn TunnelObserver>,
        )
        .await;

        let mut stream = TcpStream::connect(addr).await.expect("connect");
        greet(&mut stream).await;
        let name = b"indexer.internal";
        let mut request = vec![
            SOCKS5_VERSION,
            CMD_CONNECT,
            0x00,
            ATYP_DOMAIN,
            name.len() as u8,
        ];
        request.extend_from_slice(name);
        request.extend_from_slice(&443u16.to_be_bytes());
        stream.write_all(&request).await.expect("request");

        assert_eq!(read_reply(&mut stream).await[1], REPLY_CONNECTION_REFUSED);
        let failures = observer.failures.lock().expect("failures").clone();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].starts_with("proxy-front: tunnel could not reach indexer.internal:443"),
            "{failures:?}"
        );
    }

    #[test]
    fn only_loopback_peers_are_served() {
        assert!(peer_is_permitted(&"127.0.0.1:5000".parse().expect("addr")));
        assert!(peer_is_permitted(&"[::1]:5000".parse().expect("addr")));
        assert!(!peer_is_permitted(&"10.0.0.4:5000".parse().expect("addr")));
        assert!(!peer_is_permitted(
            &"[2001:db8::1]:5000".parse().expect("addr")
        ));
    }
}
