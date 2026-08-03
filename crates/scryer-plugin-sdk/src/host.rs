//! Typed host-service messages for native command plugins.
//!
//! A command guest sends these values through the `scryer:host/v1` binary
//! import. The transport is postcard; the SDK types stay serialization-format
//! neutral so the same contract remains inspectable in the published schema.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    PluginResult, SocketCloseRequest, SocketCloseResponse, SocketOpenRequest, SocketOpenResponse,
    SocketReadRequest, SocketReadResponse, SocketStartTlsRequest, SocketStartTlsResponse,
    SocketWriteRequest, SocketWriteResponse,
};

/// The Wasmtime import module implemented by Scryer for native command guests.
pub const HOST_ABI_MODULE: &str = "scryer:host/v1";
/// Version of the typed host-service contract.
pub const HOST_ABI_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginConfigGetRequest {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginConfigGetResponse {
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginStateGetRequest {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginStateGetResponse {
    #[serde(default)]
    pub value: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginStateSetRequest {
    pub key: String,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginStateDeleteRequest {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginStateMutationResponse {
    pub changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginHttpRequest {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginHttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Vec<u8>,
}

/// A process request evaluated only against the descriptor's process allowlist.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginProcessExecRequest {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub stdin: Vec<u8>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginProcessExecResponse {
    pub exit_code: i32,
    #[serde(default)]
    pub stdout: Vec<u8>,
    #[serde(default)]
    pub stderr: Vec<u8>,
}

/// A single request over the native host-service transport.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum PluginHostRequest {
    ConfigGet(PluginConfigGetRequest),
    StateGet(PluginStateGetRequest),
    StateSet(PluginStateSetRequest),
    StateDelete(PluginStateDeleteRequest),
    Http(PluginHttpRequest),
    SocketOpen(SocketOpenRequest),
    SocketRead(SocketReadRequest),
    SocketWrite(SocketWriteRequest),
    SocketStartTls(SocketStartTlsRequest),
    SocketClose(SocketCloseRequest),
    ProcessExec(PluginProcessExecRequest),
}

/// A single response over the native host-service transport.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum PluginHostResponse {
    ConfigGet(PluginResult<PluginConfigGetResponse>),
    StateGet(PluginResult<PluginStateGetResponse>),
    StateSet(PluginResult<PluginStateMutationResponse>),
    StateDelete(PluginResult<PluginStateMutationResponse>),
    Http(PluginResult<PluginHttpResponse>),
    SocketOpen(PluginResult<SocketOpenResponse>),
    SocketRead(PluginResult<SocketReadResponse>),
    SocketWrite(PluginResult<SocketWriteResponse>),
    SocketStartTls(PluginResult<SocketStartTlsResponse>),
    SocketClose(PluginResult<SocketCloseResponse>),
    ProcessExec(PluginResult<PluginProcessExecResponse>),
}
