//! The SSH implementation of [`TunnelProvider`], on russh.
//!
//! One session per proxy configuration revision; every dial is a
//! `direct-tcpip` channel on that session, which is exactly what
//! `ssh -L`/`ssh -D` does. The destination name travels to the server
//! unresolved, so a seedbox's `localhost` means the seedbox.
//!
//! ## Host keys: trust on first use
//!
//! The first successful handshake *and authentication* learns the server's
//! `SHA256:<base64>` fingerprint and reports it to the observer, which persists
//! it as the pin. Every later handshake compares against the pin and refuses
//! hard on a mismatch, naming both fingerprints. An operator who genuinely
//! rebuilt their server clears the pin from the UI.
//!
//! The pin is only taken after auth succeeds: a server that will not accept the
//! configured credentials has not proven itself worth trusting, and the
//! operator is about to edit the row anyway.
//!
//! ## Key types
//!
//! russh is compiled without its `rsa` feature, so **Ed25519 is the only
//! private key type accepted** and RSA host keys are never negotiated. See
//! [`ED25519_ONLY_PRIVATE_KEY_MESSAGE`].

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use russh::client;
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKey, PublicKeyOrCertificate};

use crate::error::TunnelError;
use crate::provider::{
    ED25519_ONLY_PRIVATE_KEY_MESSAGE, TunnelObserver, TunnelProvider, TunnelSpec, TunnelStream,
};

/// How often to prod an idle session, and how many unanswered prods before it
/// is considered dead. Thirty seconds is below every NAT idle timeout worth
/// worrying about, and three misses is a minute and a half of silence.
const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const KEEPALIVE_MAX: usize = 3;

/// What one handshake established. Returned by the health probe so it can tell
/// the operator whether it just pinned a key or matched the existing pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelHandshake {
    /// `SHA256:<base64>`, OpenSSH's own format, so it can be compared against
    /// `ssh-keygen -lf` output by eye.
    pub fingerprint: String,
    /// True when this handshake is the one that learned the fingerprint.
    pub newly_pinned: bool,
    /// `user@host:port`.
    pub endpoint: String,
}

#[derive(Default)]
struct HostKeyState {
    fingerprint: Option<String>,
    /// The handshake had no pin to check against, so this fingerprint is the
    /// one to persist. Stays true for the life of the provider so the health
    /// probe can report it; `reported` is what stops the observer being told
    /// twice (a rekey calls `check_server_key` again).
    newly_pinned: bool,
    reported: bool,
}

/// An SSH tunnel to one configured server.
pub struct SshTunnelProvider {
    spec: TunnelSpec,
    observer: Arc<dyn TunnelObserver>,
    host_key: Arc<Mutex<HostKeyState>>,
    session: tokio::sync::Mutex<Option<Arc<client::Handle<SshClientHandler>>>>,
}

impl SshTunnelProvider {
    /// Build a provider. No I/O happens here: the session is established on the
    /// first dial, so starting a tunnel front never blocks the caller that
    /// merely wants an egress URL.
    pub fn new(spec: TunnelSpec, observer: Arc<dyn TunnelObserver>) -> Self {
        Self {
            spec,
            observer,
            host_key: Arc::new(Mutex::new(HostKeyState::default())),
            session: tokio::sync::Mutex::new(None),
        }
    }

    /// Connect, authenticate, report the host key, disconnect.
    ///
    /// This is the health probe's entry point. It deliberately does not touch
    /// the registry: probing must not leave a session behind, and it must fail
    /// with the real reason rather than "the proxy is unreachable".
    pub async fn handshake(
        spec: TunnelSpec,
        observer: Arc<dyn TunnelObserver>,
    ) -> Result<TunnelHandshake, TunnelError> {
        let endpoint = spec.endpoint_description();
        let provider = SshTunnelProvider::new(spec, observer);
        let handle = provider.connect().await?;
        let state = provider.host_key.lock().expect("host key state");
        let handshake = TunnelHandshake {
            fingerprint: state.fingerprint.clone().unwrap_or_default(),
            newly_pinned: state.newly_pinned,
            endpoint,
        };
        drop(state);
        drop(handle);
        Ok(handshake)
    }

    async fn connect(&self) -> Result<client::Handle<SshClientHandler>, TunnelError> {
        let private_key = match self.spec.private_key_pem.as_deref() {
            Some(pem) => Some(decode_ed25519_private_key(
                pem,
                self.spec.private_key_passphrase.as_deref(),
            )?),
            None => None,
        };
        if private_key.is_none() && self.spec.password.is_none() {
            return Err(TunnelError::Configuration(
                "the tunnel has neither a password nor a private key".to_string(),
            ));
        }

        let config = Arc::new(client::Config {
            // russh's default preference list advertises RSA host-key
            // algorithms, which this build cannot verify (no `rsa` feature).
            // Advertising only what we can check turns "signature verification
            // failed" into an honest "no matching host key algorithm".
            preferred: russh::Preferred {
                key: Cow::Owned(vec![
                    Algorithm::Ed25519,
                    Algorithm::Ecdsa {
                        curve: EcdsaCurve::NistP256,
                    },
                    Algorithm::Ecdsa {
                        curve: EcdsaCurve::NistP384,
                    },
                    Algorithm::Ecdsa {
                        curve: EcdsaCurve::NistP521,
                    },
                ]),
                ..russh::Preferred::DEFAULT
            },
            keepalive_interval: Some(KEEPALIVE_INTERVAL),
            keepalive_max: KEEPALIVE_MAX,
            nodelay: true,
            ..client::Config::default()
        });

        let handler = SshClientHandler {
            host: self.spec.host.clone(),
            port: self.spec.port,
            pinned: self.spec.pinned_host_key.clone(),
            state: Arc::clone(&self.host_key),
        };

        let connected = tokio::time::timeout(
            self.spec.request_timeout,
            client::connect(config, (self.spec.host.as_str(), self.spec.port), handler),
        )
        .await
        .map_err(|_| TunnelError::Connect {
            host: self.spec.host.clone(),
            port: self.spec.port,
            detail: "the server did not complete the handshake in time".to_string(),
        })?;

        let mut handle = match connected {
            Ok(handle) => handle,
            // A host-key mismatch is raised by our own handler and must keep
            // its identity all the way to the operator.
            Err(error @ TunnelError::HostKeyMismatch { .. }) => return Err(error),
            Err(error) => {
                return Err(TunnelError::Connect {
                    host: self.spec.host.clone(),
                    port: self.spec.port,
                    detail: error.to_string(),
                });
            }
        };

        let mut detail = String::new();
        let mut authenticated = false;
        if let Some(key) = private_key {
            // `hash_alg` is only meaningful for RSA keys, which this build does
            // not accept.
            let credential = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);
            match handle
                .authenticate_publickey(self.spec.username.clone(), credential)
                .await
            {
                Ok(result) if result.success() => authenticated = true,
                Ok(_) => detail = "the server rejected the private key".to_string(),
                Err(error) => detail = error.to_string(),
            }
        }
        if !authenticated && let Some(password) = self.spec.password.clone() {
            match handle
                .authenticate_password(self.spec.username.clone(), password)
                .await
            {
                Ok(result) if result.success() => {
                    authenticated = true;
                    detail.clear();
                }
                Ok(_) => detail = "the server rejected the password".to_string(),
                Err(error) => detail = error.to_string(),
            }
        }
        if !authenticated {
            return Err(TunnelError::Auth {
                username: self.spec.username.clone(),
                host: self.spec.host.clone(),
                port: self.spec.port,
                detail: if detail.is_empty() {
                    "no configured credential was accepted".to_string()
                } else {
                    detail
                },
            });
        }

        // Trust on first use, after auth: this is the moment the server has
        // proven to be the one the operator configured.
        let pin = {
            let mut state = self.host_key.lock().expect("host key state");
            let pin = (state.newly_pinned && !state.reported)
                .then(|| state.fingerprint.clone())
                .flatten();
            state.reported = true;
            pin
        };
        if let Some(fingerprint) = pin {
            tracing::info!(
                proxy_config_id = self.spec.proxy_config_id.as_str(),
                endpoint = self.spec.endpoint_description().as_str(),
                fingerprint = fingerprint.as_str(),
                "pinned an SSH host key on first use"
            );
            self.observer
                .host_key_pinned(&self.spec.proxy_config_id, &fingerprint);
        }

        Ok(handle)
    }

    /// The live session, establishing one if there is none.
    ///
    /// Reconnection is lazy on purpose: a dead session is replaced by the next
    /// dial that needs it, and nothing retries in a loop, so an unreachable
    /// seedbox costs one connect attempt per request rather than a spin.
    async fn session(&self) -> Result<Arc<client::Handle<SshClientHandler>>, TunnelError> {
        let mut guard = self.session.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let handle = Arc::new(self.connect().await?);
        *guard = Some(Arc::clone(&handle));
        Ok(handle)
    }

    /// Drop `stale` if it is still the current session, and connect a new one.
    async fn replace_session(
        &self,
        stale: &Arc<client::Handle<SshClientHandler>>,
    ) -> Result<Arc<client::Handle<SshClientHandler>>, TunnelError> {
        let mut guard = self.session.lock().await;
        if let Some(current) = guard.as_ref()
            && !Arc::ptr_eq(current, stale)
        {
            // Another dial already reconnected; use theirs.
            return Ok(Arc::clone(current));
        }
        *guard = None;
        let handle = Arc::new(self.connect().await?);
        *guard = Some(Arc::clone(&handle));
        Ok(handle)
    }

    async fn open_channel(
        &self,
        session: &client::Handle<SshClientHandler>,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn TunnelStream>, ChannelFailure> {
        let opened = tokio::time::timeout(
            self.spec.request_timeout,
            // The originator fields are advisory; the server logs them. There
            // is no meaningful local address for a stream that starts inside
            // our own process.
            session.channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1", 0),
        )
        .await;
        match opened {
            Ok(Ok(channel)) => Ok(Box::new(channel.into_stream())),
            // The server answered and refused: the destination is the problem,
            // not the session, so this must not trigger a reconnect.
            Ok(Err(russh::Error::ChannelOpenFailure(reason))) => {
                Err(ChannelFailure::Destination(TunnelError::Dial {
                    host: host.to_string(),
                    port,
                    detail: format!("{reason:?}"),
                }))
            }
            Ok(Err(error)) => Err(ChannelFailure::Session(TunnelError::Dial {
                host: host.to_string(),
                port,
                detail: error.to_string(),
            })),
            Err(_) => Err(ChannelFailure::Session(TunnelError::Dial {
                host: host.to_string(),
                port,
                detail: "the tunnel did not open a channel in time".to_string(),
            })),
        }
    }
}

/// Whether a failed channel open means "that destination is bad" or "this
/// session is gone".
enum ChannelFailure {
    Destination(TunnelError),
    Session(TunnelError),
}

#[async_trait::async_trait]
impl TunnelProvider for SshTunnelProvider {
    async fn dial(&self, host: &str, port: u16) -> Result<Box<dyn TunnelStream>, TunnelError> {
        let session = self.session().await?;
        match self.open_channel(&session, host, port).await {
            Ok(stream) => Ok(stream),
            Err(ChannelFailure::Destination(error)) => Err(error),
            Err(ChannelFailure::Session(first)) => {
                tracing::debug!(
                    proxy_config_id = self.spec.proxy_config_id.as_str(),
                    error = %first,
                    "SSH session looked dead; reconnecting once"
                );
                let session = self.replace_session(&session).await?;
                match self.open_channel(&session, host, port).await {
                    Ok(stream) => Ok(stream),
                    Err(ChannelFailure::Destination(error) | ChannelFailure::Session(error)) => {
                        Err(error)
                    }
                }
            }
        }
    }

    fn describe(&self) -> String {
        format!("SSH tunnel to {}", self.spec.endpoint_description())
    }
}

/// The russh client handler. Its only job is host-key policy.
struct SshClientHandler {
    host: String,
    port: u16,
    pinned: Option<String>,
    state: Arc<Mutex<HostKeyState>>,
}

impl client::Handler for SshClientHandler {
    type Error = TunnelError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let actual = host_key_fingerprint(server_public_key);
        let mut state = self.state.lock().expect("host key state");
        // The configured pin wins. Failing that, the fingerprint this provider
        // already learned on an earlier handshake *is* the pin: a reconnect on
        // the same provider must not accept a different key merely because the
        // repository has not persisted the first one yet. That closes the
        // window between first use and the flush that writes the pin.
        let expected = self.pinned.clone().or_else(|| state.fingerprint.clone());
        match expected.as_deref() {
            Some(expected) if expected != actual => Err(TunnelError::HostKeyMismatch {
                host: self.host.clone(),
                port: self.port,
                expected: expected.to_string(),
                actual,
            }),
            Some(_) => {
                state.fingerprint = Some(actual);
                if self.pinned.is_some() {
                    state.newly_pinned = false;
                }
                Ok(true)
            }
            None => {
                state.fingerprint = Some(actual);
                state.newly_pinned = true;
                Ok(true)
            }
        }
    }
}

/// Validate an operator-pasted private key without connecting anywhere.
///
/// Same parser the connect path and the health probe use, so a key refused at
/// save time is refused for exactly the reason it would have failed later:
/// not Ed25519, unreadable, or passphrase-protected without its passphrase.
pub fn validate_private_key(pem: &str, passphrase: Option<&str>) -> Result<(), TunnelError> {
    decode_ed25519_private_key(pem, passphrase).map(|_| ())
}

/// `SHA256:<unpadded base64>` — byte-identical to `ssh-keygen -lf`, so an
/// operator can compare the pin against their server without conversion.
pub(crate) fn host_key_fingerprint(key: &PublicKeyOrCertificate) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

/// Parse an operator-pasted private key, accepting Ed25519 and nothing else.
///
/// The rejection is explicit rather than a decode error so the operator is told
/// what to do about it, in the same words on the connect path and in the health
/// probe.
pub(crate) fn decode_ed25519_private_key(
    pem: &str,
    passphrase: Option<&str>,
) -> Result<PrivateKey, TunnelError> {
    let key = russh::keys::decode_secret_key(pem.trim(), passphrase).map_err(|error| {
        TunnelError::Configuration(match error {
            russh::keys::Error::KeyIsEncrypted => {
                "the private key is passphrase-protected; supply the passphrase".to_string()
            }
            russh::keys::Error::UnsupportedKeyType { .. } => {
                ED25519_ONLY_PRIVATE_KEY_MESSAGE.to_string()
            }
            other => format!("the private key could not be read: {other}"),
        })
    })?;
    if !matches!(key.algorithm(), Algorithm::Ed25519) {
        return Err(TunnelError::Configuration(format!(
            "{ED25519_ONLY_PRIVATE_KEY_MESSAGE} (this key is {})",
            key.algorithm().as_str()
        )));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::NoopTunnelObserver;
    use crate::test_support::{
        CLIENT_ED25519_PEM, CLIENT_ED25519_PEM_PASSPHRASE, CLIENT_ED25519_PEM_WITH_PASSPHRASE,
        ECDSA_P256_PEM, SshServerDouble, SshServerOptions, TunnelledOrigin,
    };

    #[test]
    fn an_ed25519_key_parses() {
        let key = decode_ed25519_private_key(CLIENT_ED25519_PEM, None).expect("ed25519 key");
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
    }

    #[test]
    fn a_passphrase_protected_ed25519_key_parses_with_its_passphrase() {
        let key = decode_ed25519_private_key(
            CLIENT_ED25519_PEM_WITH_PASSPHRASE,
            Some(CLIENT_ED25519_PEM_PASSPHRASE),
        )
        .expect("ed25519 key");
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
    }

    #[test]
    fn a_passphrase_protected_key_without_its_passphrase_says_so() {
        let error = decode_ed25519_private_key(CLIENT_ED25519_PEM_WITH_PASSPHRASE, None)
            .expect_err("passphrase required");
        assert_eq!(
            error,
            TunnelError::Configuration(
                "the private key is passphrase-protected; supply the passphrase".to_string()
            )
        );
    }

    #[test]
    fn a_non_ed25519_key_is_rejected_with_the_operator_facing_message() {
        let error = decode_ed25519_private_key(ECDSA_P256_PEM, None).expect_err("not ed25519");
        let TunnelError::Configuration(message) = error else {
            panic!("expected a configuration error");
        };
        assert!(
            message.starts_with(ED25519_ONLY_PRIVATE_KEY_MESSAGE),
            "{message}"
        );
        assert!(message.contains("ecdsa-sha2-nistp256"), "{message}");
    }

    /// Records what the engine reported, so the TOFU pin can be asserted.
    #[derive(Default)]
    struct RecordingObserver {
        pins: Mutex<Vec<(String, String)>>,
        failures: Mutex<Vec<String>>,
    }

    impl TunnelObserver for RecordingObserver {
        fn tunnel_dial_failed(&self, proxy_config_id: &str, message: &str) {
            self.failures
                .lock()
                .expect("failures")
                .push(format!("{proxy_config_id}: {message}"));
        }
        fn tunnel_dial_succeeded(&self, _proxy_config_id: &str) {}
        fn host_key_pinned(&self, proxy_config_id: &str, fingerprint: &str) {
            self.pins
                .lock()
                .expect("pins")
                .push((proxy_config_id.to_string(), fingerprint.to_string()));
        }
    }

    async fn http_get_through(stream: &mut Box<dyn TunnelStream>, host: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream
            .write_all(
                format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
            )
            .await
            .expect("request");
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await.expect("response");
        String::from_utf8_lossy(&answer).to_string()
    }

    #[tokio::test]
    async fn a_dial_round_trips_bytes_through_a_real_ssh_server() {
        let server = SshServerDouble::start(SshServerOptions::default()).await;
        let origin = TunnelledOrigin::start("tunnelled").await;
        let observer = Arc::new(RecordingObserver::default());
        let provider = SshTunnelProvider::new(
            crate::test_support::spec_for("proxy-dial", &server.host(), server.port()),
            Arc::clone(&observer) as Arc<dyn TunnelObserver>,
        );

        let mut stream = provider
            .dial(&origin.addr().ip().to_string(), origin.addr().port())
            .await
            .expect("dial");
        let response = http_get_through(&mut stream, &origin.addr().to_string()).await;
        assert!(response.contains("200 OK"), "{response}");
        assert!(response.ends_with("tunnelled"), "{response}");

        assert_eq!(
            server.forwarded_targets(),
            vec![("127.0.0.1".to_string(), origin.addr().port())],
            "the destination must have been forwarded by the SSH server"
        );
        assert_eq!(origin.request_lines().len(), 1);
        assert_eq!(server.accepted_auth(), vec!["password".to_string()]);
    }

    #[tokio::test]
    async fn the_first_successful_handshake_pins_the_host_key() {
        let server = SshServerDouble::start(SshServerOptions::default()).await;
        let observer = Arc::new(RecordingObserver::default());
        let handshake = SshTunnelProvider::handshake(
            crate::test_support::spec_for("proxy-tofu", &server.host(), server.port()),
            Arc::clone(&observer) as Arc<dyn TunnelObserver>,
        )
        .await
        .expect("handshake");

        assert_eq!(
            handshake.fingerprint,
            crate::test_support::HOST_KEY_FINGERPRINT
        );
        assert!(handshake.newly_pinned);
        assert_eq!(
            observer.pins.lock().expect("pins").as_slice(),
            [(
                "proxy-tofu".to_string(),
                crate::test_support::HOST_KEY_FINGERPRINT.to_string()
            )]
        );

        // Connecting again with the pin in place matches instead of re-pinning.
        let mut spec = crate::test_support::spec_for("proxy-tofu", &server.host(), server.port());
        spec.pinned_host_key = Some(crate::test_support::HOST_KEY_FINGERPRINT.to_string());
        let observer = Arc::new(RecordingObserver::default());
        let handshake =
            SshTunnelProvider::handshake(spec, Arc::clone(&observer) as Arc<dyn TunnelObserver>)
                .await
                .expect("handshake");
        assert!(!handshake.newly_pinned);
        assert!(observer.pins.lock().expect("pins").is_empty());
    }

    #[tokio::test]
    async fn a_changed_host_key_is_a_hard_failure_naming_both_fingerprints() {
        let server = SshServerDouble::start(SshServerOptions {
            host_key_pem: crate::test_support::OTHER_HOST_ED25519_PEM,
            ..SshServerOptions::default()
        })
        .await;
        let mut spec = crate::test_support::spec_for("proxy-mitm", &server.host(), server.port());
        spec.pinned_host_key = Some(crate::test_support::HOST_KEY_FINGERPRINT.to_string());

        let error = SshTunnelProvider::handshake(spec, Arc::new(NoopTunnelObserver))
            .await
            .expect_err("a changed host key must not connect");
        let TunnelError::HostKeyMismatch {
            expected, actual, ..
        } = &error
        else {
            panic!("expected a host key mismatch, got {error}");
        };
        assert_eq!(expected, crate::test_support::HOST_KEY_FINGERPRINT);
        assert_eq!(actual, crate::test_support::OTHER_HOST_KEY_FINGERPRINT);
        let message = error.to_string();
        assert!(
            message.contains(crate::test_support::HOST_KEY_FINGERPRINT),
            "{message}"
        );
        assert!(
            message.contains(crate::test_support::OTHER_HOST_KEY_FINGERPRINT),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_wrong_password_fails_as_an_authentication_error() {
        let server = SshServerDouble::start(SshServerOptions::default()).await;
        let mut spec =
            crate::test_support::spec_for("proxy-badpass", &server.host(), server.port());
        spec.password = Some("wrong".to_string());

        let error = SshTunnelProvider::handshake(spec, Arc::new(NoopTunnelObserver))
            .await
            .expect_err("a wrong password must fail");
        assert!(
            matches!(error, TunnelError::Auth { .. }),
            "expected an auth failure, got {error}"
        );
        assert!(
            error
                .to_string()
                .starts_with("SSH authentication failed for operator@"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_passphrase_protected_ed25519_key_authenticates() {
        let server = SshServerDouble::start(SshServerOptions {
            password: None,
            authorized_key_pem: Some(CLIENT_ED25519_PEM),
            ..SshServerOptions::default()
        })
        .await;
        let mut spec = crate::test_support::spec_for("proxy-key", &server.host(), server.port());
        spec.password = None;
        spec.private_key_pem = Some(CLIENT_ED25519_PEM_WITH_PASSPHRASE.to_string());
        spec.private_key_passphrase = Some(CLIENT_ED25519_PEM_PASSPHRASE.to_string());

        SshTunnelProvider::handshake(spec, Arc::new(NoopTunnelObserver))
            .await
            .expect("key auth");
        assert_eq!(server.accepted_auth(), vec!["publickey".to_string()]);
    }

    #[tokio::test]
    async fn a_refused_destination_does_not_take_the_session_down() {
        let server = SshServerDouble::start(SshServerOptions::default()).await;
        let origin = TunnelledOrigin::start("still here").await;
        let closed_port = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let port = listener.local_addr().expect("addr").port();
            drop(listener);
            port
        };
        let provider = SshTunnelProvider::new(
            crate::test_support::spec_for("proxy-refused", &server.host(), server.port()),
            Arc::new(NoopTunnelObserver),
        );

        let error = provider
            .dial("127.0.0.1", closed_port)
            .await
            .err()
            .expect("a closed port must not dial");
        assert!(
            matches!(error, TunnelError::Dial { .. }),
            "expected a dial failure, got {error}"
        );

        // The same session still works, so a bad destination did not cost a
        // reconnect.
        let mut stream = provider
            .dial("127.0.0.1", origin.addr().port())
            .await
            .expect("second dial");
        let response = http_get_through(&mut stream, &origin.addr().to_string()).await;
        assert!(response.ends_with("still here"), "{response}");
        assert_eq!(server.forwarded_targets().len(), 2);
    }

    #[tokio::test]
    async fn a_tunnel_with_no_credentials_refuses_to_connect() {
        let mut spec = crate::test_support::spec_for("proxy-nocreds", "127.0.0.1", 22);
        spec.password = None;
        spec.private_key_pem = None;
        let provider = SshTunnelProvider::new(spec, Arc::new(NoopTunnelObserver));
        let Err(error) = provider.dial("example.test", 80).await else {
            panic!("a tunnel with no credentials must not dial");
        };
        assert_eq!(
            error,
            TunnelError::Configuration(
                "the tunnel has neither a password nor a private key".to_string()
            )
        );
    }

    fn public_key_of(pem: &str) -> PublicKeyOrCertificate {
        let key = russh::keys::decode_secret_key(pem, None).expect("test key parses");
        PublicKeyOrCertificate::PublicKey {
            key: key.public_key().clone(),
            hash_alg: None,
        }
    }

    /// The window between first use and the repository flush: a provider that
    /// learned a host key must refuse a different one on reconnect even though
    /// the stored configuration carries no pin yet.
    #[tokio::test]
    async fn a_learned_host_key_is_enforced_on_reconnect_before_the_pin_is_persisted() {
        use russh::client::Handler as _;

        let state = Arc::new(Mutex::new(HostKeyState::default()));
        let mut handler = SshClientHandler {
            host: "seedbox.test".to_string(),
            port: 22,
            pinned: None,
            state: Arc::clone(&state),
        };
        let first = public_key_of(crate::test_support::HOST_ED25519_PEM);
        assert!(
            handler
                .check_server_key(&first)
                .await
                .expect("first use is trusted")
        );
        assert_eq!(
            state.lock().expect("state").fingerprint.as_deref(),
            Some(crate::test_support::HOST_KEY_FINGERPRINT)
        );
        assert!(state.lock().expect("state").newly_pinned);

        // A reconnect on the same provider: the learned key is the pin now.
        let other = public_key_of(crate::test_support::OTHER_HOST_ED25519_PEM);
        let error = handler
            .check_server_key(&other)
            .await
            .expect_err("a changed key must be refused before the pin is persisted");
        assert!(
            matches!(error, TunnelError::HostKeyMismatch { .. }),
            "{error}"
        );
        assert!(
            handler
                .check_server_key(&first)
                .await
                .expect("the learned key still connects")
        );
        // Still the first-use learning, so the observer will be told once.
        assert!(state.lock().expect("state").newly_pinned);
    }
}
