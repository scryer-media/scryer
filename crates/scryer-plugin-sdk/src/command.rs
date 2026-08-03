//! Versioned stdin/stdout command protocol for native Scryer plugins.
//!
//! Command artifacts are ordinary `wasm32-wasip1` commands. The host writes
//! one JSON [`PluginCommandRequest`] to stdin and expects one matching
//! [`PluginCommandResponse`] on stdout. The artifact's
//! `scryer.plugin.command_abi` custom section selects this protocol; legacy
//! Extism artifacts retain their export-based protocol.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    PluginCompletedDownload, PluginDownloadClientAddRequest, PluginDownloadClientAddResponse,
    PluginDownloadClientControlRequest, PluginDownloadClientMarkImportedRequest,
    PluginDownloadClientStatus, PluginDownloadItem, PluginNotificationRequest,
    PluginNotificationResponse, PluginResult, PluginSearchRequest, PluginSearchResponse,
    SubtitlePluginDownloadRequest, SubtitlePluginDownloadResponse, SubtitlePluginGenerateRequest,
    SubtitlePluginGenerateResponse, SubtitlePluginSearchRequest, SubtitlePluginSearchResponse,
    SubtitlePluginValidateConfigRequest, SubtitlePluginValidateConfigResponse,
};

/// WebAssembly custom section which opts an artifact into the native command ABI.
pub const COMMAND_ABI_CUSTOM_SECTION: &str = "scryer.plugin.command_abi";
/// Current native command ABI version encoded in [`COMMAND_ABI_CUSTOM_SECTION`].
pub const COMMAND_ABI_VERSION: u16 = 1;

/// One native command request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginCommandRequest {
    pub abi_version: u16,
    pub command: PluginCommand,
}

impl PluginCommandRequest {
    pub fn new(command: PluginCommand) -> Self {
        Self {
            abi_version: COMMAND_ABI_VERSION,
            command,
        }
    }
}

/// One native command response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginCommandResponse {
    pub abi_version: u16,
    pub response: PluginCommandResult,
}

impl PluginCommandResponse {
    pub fn new(response: PluginCommandResult) -> Self {
        Self {
            abi_version: COMMAND_ABI_VERSION,
            response,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "family", content = "command", rename_all = "snake_case")]
pub enum PluginCommand {
    Indexer(PluginIndexerCommand),
    DownloadClient(PluginDownloadClientCommand),
    Notification(PluginNotificationCommand),
    Subtitle(PluginSubtitleCommand),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "family", content = "result", rename_all = "snake_case")]
pub enum PluginCommandResult {
    Indexer(PluginIndexerCommandResult),
    DownloadClient(PluginDownloadClientCommandResult),
    Notification(PluginNotificationCommandResult),
    Subtitle(PluginSubtitleCommandResult),
}

/// The existing action exports take an action name and arbitrary structured
/// plugin payload. Keeping that payload as JSON preserves their public shape
/// while the command family and operation itself remain typed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginActionRequest {
    pub action: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginActionResponse {
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum PluginIndexerCommand {
    Search(PluginSearchRequest),
    Action(PluginActionRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum PluginIndexerCommandResult {
    Search(PluginResult<PluginSearchResponse>),
    Action(PluginResult<PluginActionResponse>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginDownloadGetCompletedRequest {
    pub client_item_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum PluginDownloadClientCommand {
    Add(PluginDownloadClientAddRequest),
    ListQueue,
    ListHistory,
    ListCompleted,
    ListRecentCompleted(crate::PluginDownloadListRecentCompletedRequest),
    GetCompleted(PluginDownloadGetCompletedRequest),
    Control(PluginDownloadClientControlRequest),
    MarkImported(PluginDownloadClientMarkImportedRequest),
    Status,
    TestConnection,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum PluginDownloadClientCommandResult {
    Add(PluginResult<PluginDownloadClientAddResponse>),
    ListQueue(PluginResult<Vec<PluginDownloadItem>>),
    ListHistory(PluginResult<Vec<PluginCompletedDownload>>),
    ListCompleted(PluginResult<Vec<PluginCompletedDownload>>),
    ListRecentCompleted(PluginResult<Vec<PluginCompletedDownload>>),
    GetCompleted(PluginResult<Option<PluginCompletedDownload>>),
    Control(PluginResult<()>),
    MarkImported(PluginResult<()>),
    Status(PluginResult<PluginDownloadClientStatus>),
    TestConnection(PluginResult<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum PluginNotificationCommand {
    Send(PluginNotificationRequest),
    Action(PluginActionRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum PluginNotificationCommandResult {
    Send(PluginResult<PluginNotificationResponse>),
    Action(PluginResult<PluginActionResponse>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum PluginSubtitleCommand {
    ValidateConfig(SubtitlePluginValidateConfigRequest),
    Search(SubtitlePluginSearchRequest),
    Download(SubtitlePluginDownloadRequest),
    Generate(SubtitlePluginGenerateRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum PluginSubtitleCommandResult {
    ValidateConfig(PluginResult<SubtitlePluginValidateConfigResponse>),
    Search(PluginResult<SubtitlePluginSearchResponse>),
    Download(PluginResult<SubtitlePluginDownloadResponse>),
    Generate(PluginResult<SubtitlePluginGenerateResponse>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_completed_command_round_trips() {
        let request = PluginCommandRequest::new(PluginCommand::DownloadClient(
            PluginDownloadClientCommand::GetCompleted(PluginDownloadGetCompletedRequest {
                client_item_id: "opaque-client-id".to_string(),
            }),
        ));
        let json = serde_json::to_string(&request).expect("serialize request");
        let decoded: PluginCommandRequest =
            serde_json::from_str(&json).expect("deserialize request");
        assert_eq!(decoded.abi_version, COMMAND_ABI_VERSION);
        assert!(matches!(
            decoded.command,
            PluginCommand::DownloadClient(PluginDownloadClientCommand::GetCompleted(_))
        ));
    }
}
