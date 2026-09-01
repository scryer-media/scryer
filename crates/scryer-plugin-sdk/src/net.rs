use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SocketTlsMode {
    Plain,
    Starttls,
    Tls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SocketPermission {
    pub host_pattern: String,
    pub ports: Vec<u16>,
    pub tls_modes: Vec<SocketTlsMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SocketErrorCode {
    PermissionDenied,
    DnsFailed,
    ConnectTimeout,
    IoFailed,
    TlsVerificationFailed,
    StartTlsFailed,
    AuthFailed,
    RemoteClosed,
    ProtocolError,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SocketError {
    pub code: SocketErrorCode,
    pub message: String,
}

impl SocketError {
    pub fn new(code: SocketErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for SocketError {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct SocketResponse<T> {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SocketError>,
}

impl<T> SocketResponse<T> {
    pub fn ok(value: T) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn error(code: SocketErrorCode, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(SocketError::new(code, message)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketOpenRequest {
    pub host: String,
    pub port: u16,
    pub tls_mode: SocketTlsMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketOpenResponse {
    pub handle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketReadRequest {
    pub handle: u32,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketReadResponse {
    pub data_base64: String,
    pub eof: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketWriteRequest {
    pub handle: u32,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketWriteResponse {
    pub bytes_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketStartTlsRequest {
    pub handle: u32,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketStartTlsResponse {
    pub handle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketCloseRequest {
    pub handle: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SocketCloseResponse {
    pub closed: bool,
}

pub type SocketResult<T> = Result<T, SocketError>;

/// Stub socket entry points retained for source compatibility.
///
/// The types above are the live contract: they are what a guest serializes
/// into `PluginHostRequest::Socket*` and what the host answers with. The
/// *transport* that once lived here was the Extism pointer ABI, and Scryer's
/// host no longer serves it — a component that linked these entry points would
/// carry an `extism:host/user` import nothing satisfies and fail to
/// instantiate. Guests reach sockets through `scryer-plugin-pdk`'s host-call
/// helpers instead, so every call here is answered `Unsupported` on all
/// targets rather than being wired to a door that is not there.
mod guest {
    use super::*;

    fn unsupported<T>() -> SocketResult<T> {
        Err(SocketError::new(
            SocketErrorCode::Unsupported,
            "socket host functions are not served by this SDK; use scryer-plugin-pdk's host-call helpers",
        ))
    }

    pub fn socket_open(_request: SocketOpenRequest) -> SocketResult<SocketOpenResponse> {
        unsupported()
    }

    pub fn socket_read(_request: SocketReadRequest) -> SocketResult<SocketReadResponse> {
        unsupported()
    }

    pub fn socket_write(_request: SocketWriteRequest) -> SocketResult<SocketWriteResponse> {
        unsupported()
    }

    pub fn socket_starttls(
        _request: SocketStartTlsRequest,
    ) -> SocketResult<SocketStartTlsResponse> {
        unsupported()
    }

    pub fn socket_close(_request: SocketCloseRequest) -> SocketResult<SocketCloseResponse> {
        unsupported()
    }
}

pub use guest::{socket_close, socket_open, socket_read, socket_starttls, socket_write};
