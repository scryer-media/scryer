/// Every way a tunnel can refuse to carry traffic.
///
/// The variants exist so the caller can write a *distinct* operator-facing
/// message per cause: "the seedbox is down" and "the seedbox's host key
/// changed" are the same "proxy unreachable" to reqwest but very different
/// things to the person who has to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TunnelError {
    /// The stored configuration cannot be turned into a dialable tunnel
    /// (missing host, unusable private key, and so on).
    #[error("{0}")]
    Configuration(String),

    /// The server presented a host key that is not the pinned one. This is the
    /// one failure that must never be retried away.
    #[error(
        "host key changed for {host}:{port}: pinned {expected}, server offered {actual}; \
         verify the server and reset the pinned host key if the change was expected"
    )]
    HostKeyMismatch {
        host: String,
        port: u16,
        expected: String,
        actual: String,
    },

    /// The transport to the SSH server itself failed (TCP, version exchange,
    /// key exchange).
    #[error("could not establish an SSH session to {host}:{port}: {detail}")]
    Connect {
        host: String,
        port: u16,
        detail: String,
    },

    /// The server answered but refused the credentials.
    #[error("SSH authentication failed for {username}@{host}:{port}: {detail}")]
    Auth {
        username: String,
        host: String,
        port: u16,
        detail: String,
    },

    /// The session is up, but the far side would not open a channel to the
    /// destination (closed port, forwarding disabled, DNS failure on the
    /// server).
    #[error("tunnel could not reach {host}:{port}: {detail}")]
    Dial {
        host: String,
        port: u16,
        detail: String,
    },

    /// A WireGuard tunnel could not be brought up: the UDP socket would not
    /// bind, the endpoint name would not resolve, or — the common case — no
    /// handshake completed inside the budget, which is what a wrong private
    /// key, a wrong peer key, a wrong preshared key and an unreachable
    /// endpoint all look like from this side.
    ///
    /// WireGuard is silent by design: a peer that dislikes our key does not
    /// say so, it simply never answers. So this variant deliberately does not
    /// claim to know which of those it was. It names the endpoint and what was
    /// observed and leaves the diagnosis to the operator, in the same spirit
    /// as [`TunnelError::Connect`] for SSH.
    #[error("could not establish a WireGuard tunnel to {host}:{port}: {detail}")]
    WireGuardConnect {
        host: String,
        port: u16,
        detail: String,
    },

    /// The engine itself could not start (no runtime, no loopback socket).
    #[error("tunnel engine unavailable: {0}")]
    Engine(String),
}

impl From<russh::Error> for TunnelError {
    /// Required by `russh::client::Handler`, whose associated error type must
    /// be constructible from a russh error. Handshake failures that russh
    /// raises on its own reach us here; the ones *we* raise (a host-key
    /// mismatch) are constructed directly and travel out of
    /// `client::connect` unchanged.
    fn from(error: russh::Error) -> Self {
        TunnelError::Engine(error.to_string())
    }
}
