use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::types::{
    PluginDescriptor, PluginError, PluginErrorCode, SocketCloseRequest, SocketCloseResponse,
    SocketError, SocketErrorCode, SocketOpenRequest, SocketOpenResponse, SocketReadRequest,
    SocketReadResponse, SocketStartTlsRequest, SocketStartTlsResponse, SocketTlsMode,
    SocketWriteRequest, SocketWriteResponse, allowed_host_pattern_is_valid,
    socket_host_pattern_config_key,
};

const MAX_OPEN_SOCKETS: usize = 4;
const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_WRITE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_READ_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(crate) struct SocketHost {
    state: Arc<Mutex<SocketHostState>>,
}

impl SocketHost {
    pub(crate) fn disabled() -> Self {
        Self {
            state: Arc::new(Mutex::new(SocketHostState::new(Vec::new()))),
        }
    }

    pub(crate) fn from_descriptor(
        descriptor: &PluginDescriptor,
        config_json: Option<&str>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(SocketHostState::new(
                resolve_socket_permissions(descriptor, config_json),
            ))),
        }
    }

    #[cfg(test)]
    pub(crate) fn allows_for_test(&self, host: &str, port: u16, tls_mode: SocketTlsMode) -> bool {
        self.state.lock().expect("socket host state lock").allows(
            &normalize_host(host),
            port,
            tls_mode,
        )
    }

    /// Open a socket, subject to the descriptor's resolved permissions.
    ///
    /// This and its four siblings are the *only* implementations. The shared
    /// host-call service layer hands them the SDK request types directly, so
    /// nothing above can grow its own notion of what a socket grant means.
    pub(crate) fn open(
        &self,
        request: SocketOpenRequest,
    ) -> Result<SocketOpenResponse, SocketCallError> {
        self.with_state(|state| state.open(request))
    }

    pub(crate) fn read(
        &self,
        request: SocketReadRequest,
    ) -> Result<SocketReadResponse, SocketCallError> {
        self.with_state(|state| state.read(request))
    }

    pub(crate) fn write(
        &self,
        request: SocketWriteRequest,
    ) -> Result<SocketWriteResponse, SocketCallError> {
        self.with_state(|state| state.write(request))
    }

    pub(crate) fn starttls(
        &self,
        request: SocketStartTlsRequest,
    ) -> Result<SocketStartTlsResponse, SocketCallError> {
        self.with_state(|state| state.starttls(request))
    }

    pub(crate) fn close(
        &self,
        request: SocketCloseRequest,
    ) -> Result<SocketCloseResponse, SocketCallError> {
        self.with_state(|state| Ok(state.close(request)))
    }

    fn with_state<T>(
        &self,
        call: impl FnOnce(&mut SocketHostState) -> Result<T, SocketError>,
    ) -> Result<T, SocketCallError> {
        let mut state = self.state.lock().map_err(|error| {
            SocketCallError::Poisoned(format!("socket state lock poisoned: {error}"))
        })?;
        call(&mut state).map_err(SocketCallError::Socket)
    }

    /// Number of sockets this host currently holds open.
    ///
    /// Exists so a test can assert that a transport released the channel's
    /// handles after an invocation, which is not otherwise observable.
    #[cfg(test)]
    pub(crate) fn open_socket_count(&self) -> usize {
        self.state
            .lock()
            .expect("socket host state lock")
            .sockets
            .len()
    }

    pub(crate) fn cleanup(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.cleanup();
        }
    }
}

/// How a typed socket call failed.
///
/// The distinction matters because the host-call door treats the two cases
/// differently: a `SocketError` is an in-band answer the guest can act on, while
/// a poisoned lock is a host fault and becomes a temporary service failure.
#[derive(Debug)]
pub(crate) enum SocketCallError {
    Poisoned(String),
    Socket(SocketError),
}

#[derive(Debug)]
struct SocketHostState {
    permissions: Vec<ResolvedSocketPermission>,
    sockets: HashMap<u32, OpenSocket>,
    next_handle: u32,
    total_read_bytes: usize,
}

impl SocketHostState {
    fn new(permissions: Vec<ResolvedSocketPermission>) -> Self {
        Self {
            permissions,
            sockets: HashMap::new(),
            next_handle: 1,
            total_read_bytes: 0,
        }
    }

    fn cleanup(&mut self) {
        self.sockets.clear();
        self.total_read_bytes = 0;
    }

    fn open(&mut self, request: SocketOpenRequest) -> Result<SocketOpenResponse, SocketError> {
        let host = normalize_host(&request.host);
        if host.is_empty() {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                "socket host must not be empty",
            ));
        }

        if self.sockets.len() >= MAX_OPEN_SOCKETS {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                format!("socket handle limit of {MAX_OPEN_SOCKETS} reached"),
            ));
        }

        if !self.allows(&host, request.port, request.tls_mode) {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                format!(
                    "socket permission denied for {host}:{} using {:?}",
                    request.port, request.tls_mode
                ),
            ));
        }

        let read_timeout = timeout_or_default(request.read_timeout_ms, DEFAULT_READ_TIMEOUT);
        let write_timeout = timeout_or_default(request.write_timeout_ms, DEFAULT_WRITE_TIMEOUT);
        let connect_timeout =
            timeout_or_default(request.connect_timeout_ms, DEFAULT_CONNECT_TIMEOUT);

        let stream = connect_tcp(
            &host,
            request.port,
            connect_timeout,
            read_timeout,
            write_timeout,
        )?;
        let stream = match request.tls_mode {
            SocketTlsMode::Plain | SocketTlsMode::Starttls => SocketStream::Plain(stream),
            SocketTlsMode::Tls => SocketStream::Tls(Box::new(upgrade_tls(stream, &host)?)),
        };

        let handle = self.allocate_handle();
        self.sockets.insert(
            handle,
            OpenSocket {
                host,
                port: request.port,
                mode: request.tls_mode,
                stream,
            },
        );

        Ok(SocketOpenResponse { handle })
    }

    fn read(&mut self, request: SocketReadRequest) -> Result<SocketReadResponse, SocketError> {
        if request.max_bytes == 0 {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                "socket read max_bytes must be greater than zero",
            ));
        }
        if self.total_read_bytes >= MAX_TOTAL_READ_BYTES {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                format!("socket total read limit of {MAX_TOTAL_READ_BYTES} bytes exceeded"),
            ));
        }

        let max_remaining = MAX_TOTAL_READ_BYTES - self.total_read_bytes;
        let max_bytes = request.max_bytes.min(MAX_READ_BYTES).min(max_remaining);
        let socket = self.socket_mut(request.handle)?;
        let mut buffer = vec![0_u8; max_bytes];
        let bytes_read = socket.stream.read(&mut buffer).map_err(map_io_error)?;
        self.total_read_bytes += bytes_read;
        buffer.truncate(bytes_read);

        Ok(SocketReadResponse {
            data_base64: STANDARD.encode(buffer),
            eof: bytes_read == 0,
        })
    }

    fn write(&mut self, request: SocketWriteRequest) -> Result<SocketWriteResponse, SocketError> {
        let data = STANDARD
            .decode(request.data_base64.as_bytes())
            .map_err(|error| {
                socket_error(
                    SocketErrorCode::ProtocolError,
                    format!("failed to decode socket write payload: {error}"),
                )
            })?;
        if data.len() > MAX_WRITE_BYTES {
            return Err(socket_error(
                SocketErrorCode::ProtocolError,
                format!("socket write payload exceeds {MAX_WRITE_BYTES} bytes"),
            ));
        }

        let socket = self.socket_mut(request.handle)?;
        socket.stream.write_all(&data).map_err(map_io_error)?;
        socket.stream.flush().map_err(map_io_error)?;

        Ok(SocketWriteResponse {
            bytes_written: data.len(),
        })
    }

    fn starttls(
        &mut self,
        request: SocketStartTlsRequest,
    ) -> Result<SocketStartTlsResponse, SocketError> {
        let requested_host = normalize_host(&request.host);
        let socket = self.sockets.get(&request.handle).ok_or_else(|| {
            socket_error(
                SocketErrorCode::RemoteClosed,
                format!("socket handle {} is not open", request.handle),
            )
        })?;
        if requested_host != socket.host {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                "STARTTLS host must match the connected socket host",
            ));
        }
        if socket.mode != SocketTlsMode::Starttls
            || !self.allows(&socket.host, socket.port, SocketTlsMode::Starttls)
        {
            return Err(socket_error(
                SocketErrorCode::PermissionDenied,
                format!(
                    "socket STARTTLS permission denied for {}:{}",
                    socket.host, socket.port
                ),
            ));
        }
        if !matches!(&socket.stream, SocketStream::Plain(_)) {
            return Err(socket_error(
                SocketErrorCode::StartTlsFailed,
                "socket is already using TLS",
            ));
        }

        let mut socket = self.take_socket(request.handle)?;
        let SocketStream::Plain(stream) = socket.stream else {
            unreachable!("socket stream was checked before removal");
        };
        socket.stream = SocketStream::Tls(Box::new(upgrade_tls(stream, &socket.host)?));
        self.sockets.insert(request.handle, socket);

        Ok(SocketStartTlsResponse {
            handle: request.handle,
        })
    }

    fn close(&mut self, request: SocketCloseRequest) -> SocketCloseResponse {
        SocketCloseResponse {
            closed: self.sockets.remove(&request.handle).is_some(),
        }
    }

    fn allows(&self, host: &str, port: u16, tls_mode: SocketTlsMode) -> bool {
        self.permissions
            .iter()
            .any(|permission| permission.allows(host, port, tls_mode))
    }

    fn allocate_handle(&mut self) -> u32 {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        handle
    }

    fn socket_mut(&mut self, handle: u32) -> Result<&mut OpenSocket, SocketError> {
        self.sockets.get_mut(&handle).ok_or_else(|| {
            socket_error(
                SocketErrorCode::RemoteClosed,
                format!("socket handle {handle} is not open"),
            )
        })
    }

    fn take_socket(&mut self, handle: u32) -> Result<OpenSocket, SocketError> {
        self.sockets.remove(&handle).ok_or_else(|| {
            socket_error(
                SocketErrorCode::RemoteClosed,
                format!("socket handle {handle} is not open"),
            )
        })
    }
}

#[derive(Debug)]
struct OpenSocket {
    host: String,
    port: u16,
    mode: SocketTlsMode,
    stream: SocketStream,
}

#[derive(Debug)]
enum SocketStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for SocketStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for SocketStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

#[derive(Debug)]
struct ResolvedSocketPermission {
    host_pattern: String,
    ports: Vec<u16>,
    tls_modes: Vec<SocketTlsMode>,
}

impl ResolvedSocketPermission {
    fn allows(&self, host: &str, port: u16, tls_mode: SocketTlsMode) -> bool {
        self.ports.contains(&port)
            && self.tls_modes.contains(&tls_mode)
            && host_matches_pattern(&self.host_pattern, host)
    }
}

fn host_matches_pattern(pattern: &str, host: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1);
    }
    pattern == host
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn resolve_socket_permissions(
    descriptor: &PluginDescriptor,
    config_json: Option<&str>,
) -> Vec<ResolvedSocketPermission> {
    let config = config_json.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    descriptor
        .socket_permissions
        .iter()
        .filter_map(|permission| {
            let host_pattern =
                resolve_socket_host_pattern(&permission.host_pattern, config.as_ref())?;
            if permission.ports.is_empty()
                || permission.tls_modes.is_empty()
                || !allowed_host_pattern_is_valid(&host_pattern)
            {
                return None;
            }
            Some(ResolvedSocketPermission {
                host_pattern: normalize_host(&host_pattern),
                ports: permission.ports.clone(),
                tls_modes: permission.tls_modes.clone(),
            })
        })
        .collect()
}

fn resolve_socket_host_pattern(
    pattern: &str,
    config: Option<&serde_json::Value>,
) -> Option<String> {
    let pattern = pattern.trim();
    let Some(key) = socket_host_pattern_config_key(pattern) else {
        return Some(pattern.to_string());
    };
    config?
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn timeout_or_default(value_ms: Option<u64>, default: Duration) -> Duration {
    value_ms
        .filter(|value| *value > 0)
        .map(Duration::from_millis)
        .unwrap_or(default)
}

fn connect_tcp(
    host: &str,
    port: u16,
    connect_timeout: Duration,
    read_timeout: Duration,
    write_timeout: Duration,
) -> Result<TcpStream, SocketError> {
    let addresses = (host, port).to_socket_addrs().map_err(|error| {
        socket_error(
            SocketErrorCode::DnsFailed,
            format!("failed to resolve {host}:{port}: {error}"),
        )
    })?;

    let mut saw_address = false;
    let mut last_error = None;
    for address in addresses {
        saw_address = true;
        match TcpStream::connect_timeout(&address, connect_timeout) {
            Ok(stream) => {
                stream.set_read_timeout(Some(read_timeout)).ok();
                stream.set_write_timeout(Some(write_timeout)).ok();
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }

    if !saw_address {
        return Err(socket_error(
            SocketErrorCode::DnsFailed,
            format!("{host}:{port} did not resolve to any socket addresses"),
        ));
    }

    let error = last_error.unwrap_or_else(|| io::Error::other("connect failed"));
    Err(socket_error(
        if error.kind() == io::ErrorKind::TimedOut {
            SocketErrorCode::ConnectTimeout
        } else {
            SocketErrorCode::IoFailed
        },
        format!("failed to connect to {host}:{port}: {error}"),
    ))
}

fn upgrade_tls(
    stream: TcpStream,
    host: &str,
) -> Result<StreamOwned<ClientConnection, TcpStream>, SocketError> {
    let server_name = ServerName::try_from(host.to_string()).map_err(|error| {
        socket_error(
            SocketErrorCode::TlsVerificationFailed,
            format!("invalid TLS server name {host}: {error}"),
        )
    })?;
    let connection = ClientConnection::new(tls_config()?, server_name).map_err(|error| {
        socket_error(
            SocketErrorCode::TlsVerificationFailed,
            format!("failed to create TLS connection: {error}"),
        )
    })?;
    let mut tls_stream = StreamOwned::new(connection, stream);

    while tls_stream.conn.is_handshaking() {
        tls_stream
            .conn
            .complete_io(&mut tls_stream.sock)
            .map_err(|error| {
                socket_error(
                    SocketErrorCode::TlsVerificationFailed,
                    format!("TLS handshake failed: {error}"),
                )
            })?;
    }

    Ok(tls_stream)
}

fn tls_config() -> Result<Arc<ClientConfig>, SocketError> {
    let native = rustls_native_certs::load_native_certs();
    let mut roots = RootCertStore::empty();
    let (added, _) = roots.add_parsable_certificates(native.certs);
    if added == 0 || roots.is_empty() {
        return Err(socket_error(
            SocketErrorCode::TlsVerificationFailed,
            "no platform TLS root certificates were available",
        ));
    }

    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// Project a socket failure onto the SDK's error shape for the host-call door.
///
/// `PluginError` has no room for a `SocketErrorCode` — `PluginErrorDetails` is a
/// closed, indexer-only enum — so a `{"code":...,"message":...}` document is
/// carried in `debug_message` instead. A guest that
/// needs the exact discriminant (an SMTP client distinguishing
/// `tls_verification_failed` from `starttls_failed`, say) parses it back out;
/// one that only needs to branch reads `code`. `public_message` stays the socket
/// layer's own message, unchanged from what the legacy envelope carries.
pub(crate) fn socket_plugin_error(error: &SocketError) -> PluginError {
    let code = match error.code {
        // A denial is a decision about this plugin's descriptor permissions,
        // and retrying cannot change it. It is deliberately NOT `Unsupported`:
        // that code means "this host has no such service", which is the
        // different answer a non-notification host gives.
        SocketErrorCode::PermissionDenied
        | SocketErrorCode::TlsVerificationFailed
        | SocketErrorCode::StartTlsFailed
        | SocketErrorCode::ProtocolError => PluginErrorCode::Permanent,
        SocketErrorCode::AuthFailed => PluginErrorCode::AuthFailed,
        SocketErrorCode::DnsFailed => PluginErrorCode::UpstreamUnavailable,
        SocketErrorCode::ConnectTimeout
        | SocketErrorCode::IoFailed
        | SocketErrorCode::RemoteClosed => PluginErrorCode::Temporary,
        SocketErrorCode::Unsupported => PluginErrorCode::Unsupported,
    };
    PluginError {
        code,
        public_message: error.message.clone(),
        debug_message: Some(serde_json::to_string(error).unwrap_or_else(|_| error.message.clone())),
        retry_after_seconds: Some(0),
        details: None,
    }
}

fn socket_error(code: SocketErrorCode, message: impl Into<String>) -> SocketError {
    SocketError {
        code,
        message: message.into(),
    }
}

fn map_io_error(error: io::Error) -> SocketError {
    let code = match error.kind() {
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotConnected => SocketErrorCode::RemoteClosed,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => SocketErrorCode::ConnectTimeout,
        _ => SocketErrorCode::IoFailed,
    };
    socket_error(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        NotificationCapabilities, NotificationDescriptor, PluginDescriptor, ProviderDescriptor,
        SDK_VERSION, SocketPermission, current_sdk_constraint,
    };

    #[test]
    fn disabled_socket_host_denies_open() {
        let host = SocketHost::disabled();
        let request = SocketOpenRequest {
            host: "127.0.0.1".to_string(),
            port: 25,
            tls_mode: SocketTlsMode::Plain,
            connect_timeout_ms: Some(1),
            read_timeout_ms: Some(1),
            write_timeout_ms: Some(1),
        };

        let error = host
            .open(request)
            .expect_err("a disabled socket host grants nothing");

        let SocketCallError::Socket(error) = error else {
            panic!("a permission denial is a socket answer, not a host fault");
        };
        assert!(
            matches!(error.code, SocketErrorCode::PermissionDenied),
            "{error:?}"
        );
    }

    #[test]
    fn descriptor_socket_permissions_resolve_notification_config_host() {
        let descriptor = PluginDescriptor {
            id: "email".to_string(),
            name: "Email".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: SDK_VERSION.to_string(),
            sdk_constraint: current_sdk_constraint(),
            socket_permissions: vec![SocketPermission {
                host_pattern: "${smtp_host}".to_string(),
                ports: vec![25, 465, 587],
                tls_modes: vec![
                    SocketTlsMode::Plain,
                    SocketTlsMode::Tls,
                    SocketTlsMode::Starttls,
                ],
            }],
            provider: ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: "email".to_string(),
                provider_aliases: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: NotificationCapabilities::default(),
                config_fields: Vec::new(),
            }),
        };

        let host = SocketHost::from_descriptor(&descriptor, Some(r#"{"smtp_host":"smtp"}"#));

        assert!(host.allows_for_test("smtp", 587, SocketTlsMode::Starttls));
        assert!(!host.allows_for_test("localhost", 587, SocketTlsMode::Starttls));
        assert!(!host.allows_for_test("smtp", 2525, SocketTlsMode::Starttls));
    }
}
