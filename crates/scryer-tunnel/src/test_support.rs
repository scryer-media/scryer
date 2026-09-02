//! An in-process SSH server, for tests only.
//!
//! russh ships a server API, so the tunnel engine can be tested against a real
//! SSH peer rather than a mock of one: a real key exchange, a real
//! authentication exchange and real `direct-tcpip` forwarding, all inside the
//! test process with no network and no fixtures on disk.
//!
//! It is exposed (behind the `test-support` feature) so the *consumer* crates
//! can use it too. That is what lets `scryer-plugins` and
//! `scryer-infrastructure-acquisition` prove their egress really travelled
//! through a tunnel — the double records every destination it was asked to
//! forward, so "the origin was reached" and "the origin was reached through the
//! tunnel" are different assertions.
//!
//! The keys below are fixed test material generated with `ssh-keygen -t
//! ed25519`. They are public test fixtures, protect nothing, and are never used
//! outside `cfg(test)` / the `test-support` feature.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use russh::keys::{HashAlg, PrivateKey};
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelOpenFailure};

/// Host key the double presents. Its fingerprint is
/// [`HOST_KEY_FINGERPRINT`].
pub const HOST_ED25519_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACC38FwBXgj0y7rMebgdLLDy+IwkZVhKmDCFN8oWk21ZCQAAAKB1XfQmdV30
JgAAAAtzc2gtZWQyNTUxOQAAACC38FwBXgj0y7rMebgdLLDy+IwkZVhKmDCFN8oWk21ZCQ
AAAEAvppofWdTPzGM3/gZz5kyfRrLMjkGvnyTLkGzIuuwlprfwXAFeCPTLusx5uB0ssPL4
jCRlWEqYMIU3yhaTbVkJAAAAF3Njcnllci10dW5uZWwtdGVzdC1ob3N0AQIDBAUG
-----END OPENSSH PRIVATE KEY-----
";

/// `ssh-keygen -lf` of [`HOST_ED25519_PEM`]'s public half.
pub const HOST_KEY_FINGERPRINT: &str = "SHA256:9D6rJ2/enT47oLlyKB/4uD+huVitjTfdUbZVUoKZtCE";

/// A second host key, for proving a changed host key is refused.
pub const OTHER_HOST_ED25519_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCPoM0fbYEPxX9ZFw7u064oRYMqOrGC2ZR9Blmut5BbzAAAAKBbh53tW4ed
7QAAAAtzc2gtZWQyNTUxOQAAACCPoM0fbYEPxX9ZFw7u064oRYMqOrGC2ZR9Blmut5BbzA
AAAEAd3uXY1ZzD8CPyV+LGSw7WU9QWQwmpkBsX0udHoxAaxY+gzR9tgQ/Ff1kXDu7TrihF
gyo6sYLZlH0GWa63kFvMAAAAHXNjcnllci10dW5uZWwtdGVzdC1vdGhlci1ob3N0
-----END OPENSSH PRIVATE KEY-----
";

/// `ssh-keygen -lf` of [`OTHER_HOST_ED25519_PEM`]'s public half.
pub const OTHER_HOST_KEY_FINGERPRINT: &str = "SHA256:yDQP6cD1NCLlJwsrofbOAoO899rHGCRICWLGeqJjvV4";

/// Client identity, unencrypted.
pub const CLIENT_ED25519_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCPI7e9nBVpGV9WOOQn4t8L+GsmFEn+AeRM1AUt283uWgAAAKC+Y9ahvmPW
oQAAAAtzc2gtZWQyNTUxOQAAACCPI7e9nBVpGV9WOOQn4t8L+GsmFEn+AeRM1AUt283uWg
AAAEAmB8G+LY2L/4/VYrxVr7NEUffCM0heOSHZSo3xE02t+o8jt72cFWkZX1Y45Cfi3wv4
ayYUSf4B5EzUBS3bze5aAAAAGXNjcnllci10dW5uZWwtdGVzdC1jbGllbnQBAgME
-----END OPENSSH PRIVATE KEY-----
";

/// The same client identity, protected with [`CLIENT_ED25519_PEM_PASSPHRASE`].
pub const CLIENT_ED25519_PEM_WITH_PASSPHRASE: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABDm+hAbYi
Cb6D7wYC+p3cufAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAII8jt72cFWkZX1Y4
5Cfi3wv4ayYUSf4B5EzUBS3bze5aAAAAoNFwhbRiUaKIIOL3/P2xOY+4YVqvXRiTY0X6zo
Yb+vXXFxSmVfLw06PmUL0g6qk7YFyAFzlWLadgnXRgTZFaytIIAjkoG9VvpwLSMuJWlNPa
kqm9DAzExWq7PAogmhqVhowRH1iAP/oMAzlVEdVQ+2BBveNWA4B0XACmEGtXKjPk70980k
NoMLd2yYtrdE5Ffmj9p01DnjggXX/x5yoy/Oc=
-----END OPENSSH PRIVATE KEY-----
";

/// Passphrase for [`CLIENT_ED25519_PEM_WITH_PASSPHRASE`].
pub const CLIENT_ED25519_PEM_PASSPHRASE: &str = "correct horse";

/// An ECDSA key, purely to prove non-Ed25519 keys are refused with an
/// explanation.
pub const ECDSA_P256_PEM: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAaAAAABNlY2RzYS
1zaGEyLW5pc3RwMjU2AAAACG5pc3RwMjU2AAAAQQQWriC8c6nHcAetEvSHW/K+MaX9SfQA
tTt2Izkk6eF9+TTKHrMKzQO+wKeF7t9xi+uOqTbsC1oGm4JgljkcFItgAAAAuIiWx0uIls
dLAAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBBauILxzqcdwB60S
9Idb8r4xpf1J9AC1O3YjOSTp4X35NMoeswrNA77Ap4Xu33GL646pNuwLWgabgmCWORwUi2
AAAAAhAJ81ynJPKI/A7FwEy/XeqW5hkGD3uHwrn8Y1Rcvogx1wAAAAGHNjcnllci10dW5u
ZWwtdGVzdC1lY2RzYQECAwQFBgc=
-----END OPENSSH PRIVATE KEY-----
";

/// How the double should behave.
#[derive(Clone)]
pub struct SshServerOptions {
    /// Host key to present. Defaults to [`HOST_ED25519_PEM`].
    pub host_key_pem: &'static str,
    /// The only username accepted.
    pub username: String,
    /// Accepted password, if password auth is allowed.
    pub password: Option<String>,
    /// Accepted public key (given as its private PEM), if key auth is allowed.
    pub authorized_key_pem: Option<&'static str>,
    /// When true, every `direct-tcpip` request is refused.
    pub refuse_forwarding: bool,
}

impl Default for SshServerOptions {
    fn default() -> Self {
        Self {
            host_key_pem: HOST_ED25519_PEM,
            username: "operator".to_string(),
            password: Some("s3cret".to_string()),
            authorized_key_pem: None,
            refuse_forwarding: false,
        }
    }
}

/// A running in-process SSH server.
pub struct SshServerDouble {
    addr: SocketAddr,
    forwarded: Arc<Mutex<Vec<(String, u16)>>>,
    accepted_auth: Arc<Mutex<Vec<String>>>,
}

impl SshServerDouble {
    /// Start on an ephemeral loopback port.
    pub async fn start(options: SshServerOptions) -> Self {
        let host_key = russh::keys::decode_secret_key(options.host_key_pem, None)
            .expect("test host key parses");
        let authorized_fingerprint = options.authorized_key_pem.map(|pem| {
            russh::keys::decode_secret_key(pem, None)
                .expect("test client key parses")
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string()
        });

        let config = Arc::new(server::Config {
            keys: vec![host_key],
            // Tests must not pay russh's constant-time rejection delay.
            auth_rejection_time: std::time::Duration::from_millis(1),
            auth_rejection_time_initial: Some(std::time::Duration::from_millis(1)),
            inactivity_timeout: Some(std::time::Duration::from_secs(60)),
            ..server::Config::default()
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ssh double");
        let addr = listener.local_addr().expect("ssh double addr");
        let forwarded = Arc::new(Mutex::new(Vec::new()));
        let accepted_auth = Arc::new(Mutex::new(Vec::new()));

        let options = Arc::new(options);
        let forwarded_for_task = Arc::clone(&forwarded);
        let accepted_for_task = Arc::clone(&accepted_auth);
        tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                let handler = DoubleHandler {
                    options: Arc::clone(&options),
                    authorized_fingerprint: authorized_fingerprint.clone(),
                    forwarded: Arc::clone(&forwarded_for_task),
                    accepted_auth: Arc::clone(&accepted_for_task),
                };
                let config = Arc::clone(&config);
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, stream, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });

        Self {
            addr,
            forwarded,
            accepted_auth,
        }
    }

    /// Where to point a tunnel.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Every `(host, port)` the double was asked to forward to, in order. This
    /// is the proof that traffic went *through* the tunnel.
    pub fn forwarded_targets(&self) -> Vec<(String, u16)> {
        self.forwarded.lock().expect("forwarded").clone()
    }

    /// Which authentication methods were accepted, in order.
    pub fn accepted_auth(&self) -> Vec<String> {
        self.accepted_auth.lock().expect("accepted auth").clone()
    }
}

struct DoubleHandler {
    options: Arc<SshServerOptions>,
    authorized_fingerprint: Option<String>,
    forwarded: Arc<Mutex<Vec<(String, u16)>>>,
    accepted_auth: Arc<Mutex<Vec<String>>>,
}

impl server::Handler for DoubleHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let accepted =
            user == self.options.username && self.options.password.as_deref() == Some(password);
        if accepted {
            self.accepted_auth
                .lock()
                .expect("accepted auth")
                .push("password".to_string());
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        let offered = public_key.fingerprint(HashAlg::Sha256).to_string();
        let accepted = user == self.options.username
            && self.authorized_fingerprint.as_deref() == Some(offered.as_str());
        if accepted {
            self.accepted_auth
                .lock()
                .expect("accepted auth")
                .push("publickey".to_string());
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let port = port_to_connect as u16;
        self.forwarded
            .lock()
            .expect("forwarded")
            .push((host_to_connect.to_string(), port));

        if self.options.refuse_forwarding {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }

        match tokio::net::TcpStream::connect((host_to_connect, port)).await {
            Ok(mut upstream) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
                });
            }
            Err(_) => {
                reply.reject(ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }
}

/// A [`TunnelSpec`](crate::TunnelSpec) with the double's usual credentials.
pub fn spec_for(proxy_config_id: &str, host: &str, port: u16) -> crate::TunnelSpec {
    crate::TunnelSpec {
        proxy_config_id: proxy_config_id.to_string(),
        proxy_name: "Seedbox".to_string(),
        revision: format!("{proxy_config_id}@v1"),
        host: host.to_string(),
        port,
        username: "operator".to_string(),
        password: Some("s3cret".to_string()),
        private_key_pem: None,
        private_key_passphrase: None,
        pinned_host_key: None,
        request_timeout: std::time::Duration::from_secs(10),
    }
}

/// A tiny HTTP origin that answers every request with the same body and counts
/// what it received. Used as the far end of a tunnel.
pub struct TunnelledOrigin {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl TunnelledOrigin {
    pub async fn start(body: &'static str) -> Self {
        Self::start_with_content_type("text/plain", body).await
    }

    pub async fn start_with_content_type(content_type: &'static str, body: &'static str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind origin");
        let addr = listener.local_addr().expect("origin addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    break;
                };
                let recorder = Arc::clone(&recorder);
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buffer = vec![0u8; 8192];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                    recorder
                        .lock()
                        .expect("requests")
                        .push(request.lines().next().unwrap_or_default().to_string());
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.flush().await;
                });
            }
        });
        Self { addr, requests }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Request lines the origin actually received.
    pub fn request_lines(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

#[allow(dead_code)]
fn assert_private_key_type(pem: &str) -> PrivateKey {
    russh::keys::decode_secret_key(pem, None).expect("key parses")
}

/// The WireGuard family's double, re-exported here so consumer crates find
/// both engines' test peers in one place.
///
/// Unlike the SSH double this one is a *real* second WireGuard device on a
/// real loopback UDP socket, with its own userspace IP stack: there is no
/// simpler way to exercise a handshake, and a mock could not prove that a name
/// was resolved through the tunnel rather than by this machine.
pub use crate::wireguard::{
    TEST_CLIENT_ADDRESS, TEST_PEER_ADDRESS, TEST_PEER_HTTP_PORT, WireGuardTestPeer,
    WireGuardTestPeerOptions, test_client_private_key, test_key, test_peer_private_key,
    test_preshared_key,
};
