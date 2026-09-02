use std::fmt;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::TunnelError;

/// The one operator-facing sentence about private keys. It lives here, next to
/// the parser that enforces it, so the connect path and the health probe say
/// exactly the same thing, and so the web form can echo it as help text.
pub const ED25519_ONLY_PRIVATE_KEY_MESSAGE: &str = "only Ed25519 private keys are supported; generate one with \
     `ssh-keygen -t ed25519` and paste the OpenSSH private key";

/// A byte stream to somewhere on the far side of a tunnel.
///
/// `Box<dyn AsyncRead + AsyncWrite + Send + Unpin>` is not expressible in Rust
/// (a trait object takes at most one non-auto trait), so this alias trait is
/// the faithful rendering of it. The blanket impl means every duplex stream —
/// a russh channel, a `TcpStream`, a `tokio::io::DuplexStream` in a test —
/// already satisfies it.
pub trait TunnelStream: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T> TunnelStream for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

/// Something that can carry a TCP connection to `host:port` on the far side of
/// a tunnel.
///
/// This is the seam a second tunnel technology plugs into. A WireGuard
/// implementation (smoltcp over a userspace device) implements exactly this
/// and inherits the SOCKS5 front, the registry, the lifecycle and every
/// consumer without another line changing.
#[async_trait::async_trait]
pub trait TunnelProvider: Send + Sync {
    /// Open a stream to `host:port`, resolving `host` **on the far side**.
    ///
    /// `host` may be a name; that is the point of a tunnel. A seedbox's
    /// `localhost` must mean the seedbox.
    async fn dial(&self, host: &str, port: u16) -> Result<Box<dyn TunnelStream>, TunnelError>;

    /// Short human description of where this tunnel goes, for health text and
    /// tracing. Must not contain credentials.
    fn describe(&self) -> String;
}

/// Where a tunnel reports what it observed.
///
/// The engine cannot reach a repository (it runs on egress paths that have no
/// handle to one, including a blocking plugin worker thread), so it hands
/// observations to the caller, which owns the ledgers that async flows drain.
/// This is the same convention the challenge-solver health path established.
pub trait TunnelObserver: Send + Sync {
    /// A connection through the tunnel failed. `message` is already
    /// operator-facing and free of secrets.
    fn tunnel_dial_failed(&self, proxy_config_id: &str, message: &str);

    /// A connection through the tunnel succeeded.
    fn tunnel_dial_succeeded(&self, proxy_config_id: &str);

    /// Trust-on-first-use: this fingerprint was learned from the first
    /// successful handshake and should be persisted as the pin.
    fn host_key_pinned(&self, proxy_config_id: &str, fingerprint: &str);
}

/// Observer that discards everything. For tests and for callers that only want
/// the transport.
pub struct NoopTunnelObserver;

impl TunnelObserver for NoopTunnelObserver {
    fn tunnel_dial_failed(&self, _proxy_config_id: &str, _message: &str) {}
    fn tunnel_dial_succeeded(&self, _proxy_config_id: &str) {}
    fn host_key_pinned(&self, _proxy_config_id: &str, _fingerprint: &str) {}
}

/// Everything the engine needs to bring up one tunnel, with no domain types.
///
/// The caller decrypts credentials (they are already plaintext in memory by the
/// time a config leaves the store) and hands them over per tunnel start; this
/// crate never sees an encryption key and never writes any of this anywhere.
#[derive(Clone)]
pub struct TunnelSpec {
    /// Proxy config id. The registry key, and the id every observation is
    /// reported under.
    pub proxy_config_id: String,
    /// The operator's name for this proxy, for messages.
    pub proxy_name: String,
    /// `id@updated_at`. A change restarts the tunnel; a health write does not
    /// change it, so a flapping tunnel does not churn sessions.
    pub revision: String,
    /// SSH server host.
    pub host: String,
    /// SSH server port.
    pub port: u16,
    /// SSH username.
    pub username: String,
    /// Password, when password auth is configured.
    pub password: Option<String>,
    /// OpenSSH-format Ed25519 private key, when key auth is configured.
    pub private_key_pem: Option<String>,
    /// Passphrase for that key, when it has one.
    pub private_key_passphrase: Option<String>,
    /// `SHA256:<base64>` pinned at the first successful handshake, if any.
    /// `None` means trust-on-first-use is still pending.
    pub pinned_host_key: Option<String>,
    /// Budget for a handshake and for a single dial through the tunnel.
    pub request_timeout: Duration,
}

impl fmt::Debug for TunnelSpec {
    /// Hand-written so a stray `{:?}` in a log line can never print a password,
    /// a private key or a passphrase.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TunnelSpec")
            .field("proxy_config_id", &self.proxy_config_id)
            .field("proxy_name", &self.proxy_name)
            .field("revision", &self.revision)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field(
                "private_key_pem",
                &self.private_key_pem.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "private_key_passphrase",
                &self.private_key_passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("pinned_host_key", &self.pinned_host_key)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl TunnelSpec {
    /// `user@host:port`, never credentials.
    pub fn endpoint_description(&self) -> String {
        format!("{}@{}:{}", self.username, self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TunnelSpec {
        TunnelSpec {
            proxy_config_id: "proxy-1".to_string(),
            proxy_name: "Seedbox".to_string(),
            revision: "proxy-1@now".to_string(),
            host: "seedbox.test".to_string(),
            port: 22,
            username: "operator".to_string(),
            password: Some("s3cret".to_string()),
            private_key_pem: Some("-----BEGIN OPENSSH PRIVATE KEY-----".to_string()),
            private_key_passphrase: Some("hunter2".to_string()),
            pinned_host_key: Some("SHA256:abc".to_string()),
            request_timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn the_spec_debug_impl_never_prints_key_material() {
        let rendered = format!("{:?}", spec());
        assert!(!rendered.contains("s3cret"), "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("BEGIN OPENSSH"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        // The public half of the identity is deliberately still visible.
        assert!(rendered.contains("seedbox.test"), "{rendered}");
        assert!(rendered.contains("SHA256:abc"), "{rendered}");
    }

    #[test]
    fn the_endpoint_description_names_the_hop_without_credentials() {
        assert_eq!(spec().endpoint_description(), "operator@seedbox.test:22");
    }
}
