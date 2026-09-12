//! Versioned command envelope for Scryer plugins.
//!
//! One JSON [`PluginCommandRequest`] in, one matching [`PluginCommandResponse`]
//! out. Today the envelope travels as the opaque UTF-8 payload of each plugin
//! family's WIT world (`process` on the WASI Preview 2 component hosts); the
//! `scryer.plugin.command_abi` custom section and [`COMMAND_ABI_VERSION`] pin
//! which revision of the envelope an artifact speaks.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    PluginCompletedDownload, PluginDownloadClientAddRequest, PluginDownloadClientAddResponse,
    PluginDownloadClientControlRequest, PluginDownloadClientMarkImportedRequest,
    PluginDownloadClientStatus, PluginDownloadItem, PluginDownloadScopedListRequest,
    PluginDownloadScopedListResponse, PluginDownloadScopedRecentCompletedRequest,
    PluginNotificationRequest, PluginNotificationResponse, PluginResult, PluginSearchRequest,
    PluginSearchResponse, SubtitlePluginDownloadRequest, SubtitlePluginDownloadResponse,
    SubtitlePluginGenerateRequest, SubtitlePluginGenerateResponse, SubtitlePluginSearchRequest,
    SubtitlePluginSearchResponse, SubtitlePluginValidateConfigRequest,
    SubtitlePluginValidateConfigResponse, SubtitleSyncPluginProcessRequest,
    SubtitleSyncPluginProcessResponse,
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
// Command envelopes are short-lived serialization values; boxing variants would be a
// source-compatible API break without a meaningful runtime benefit.
#[allow(clippy::large_enum_variant)]
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
#[allow(clippy::large_enum_variant)]
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
#[allow(clippy::large_enum_variant)]
pub enum PluginDownloadClientCommand {
    Add(PluginDownloadClientAddRequest),
    ListQueue,
    ListQueueScoped(PluginDownloadScopedListRequest),
    ListHistory,
    ListHistoryScoped(PluginDownloadScopedListRequest),
    ListCompleted,
    ListCompletedScoped(PluginDownloadScopedListRequest),
    ListRecentCompleted(crate::PluginDownloadListRecentCompletedRequest),
    ListRecentCompletedScoped(PluginDownloadScopedRecentCompletedRequest),
    GetCompleted(PluginDownloadGetCompletedRequest),
    Control(PluginDownloadClientControlRequest),
    MarkImported(PluginDownloadClientMarkImportedRequest),
    MarkImportedNonDestructive(PluginDownloadClientMarkImportedRequest),
    Status,
    TestConnection,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum PluginDownloadClientCommandResult {
    Add(PluginResult<PluginDownloadClientAddResponse>),
    ListQueue(PluginResult<Vec<PluginDownloadItem>>),
    ListQueueScoped(PluginResult<PluginDownloadScopedListResponse<PluginDownloadItem>>),
    ListHistory(PluginResult<Vec<PluginCompletedDownload>>),
    ListHistoryScoped(PluginResult<PluginDownloadScopedListResponse<PluginCompletedDownload>>),
    ListCompleted(PluginResult<Vec<PluginCompletedDownload>>),
    ListCompletedScoped(PluginResult<PluginDownloadScopedListResponse<PluginCompletedDownload>>),
    ListRecentCompleted(PluginResult<Vec<PluginCompletedDownload>>),
    ListRecentCompletedScoped(
        PluginResult<PluginDownloadScopedListResponse<PluginCompletedDownload>>,
    ),
    GetCompleted(PluginResult<Option<PluginCompletedDownload>>),
    Control(PluginResult<()>),
    MarkImported(PluginResult<()>),
    MarkImportedNonDestructive(PluginResult<()>),
    Status(PluginResult<PluginDownloadClientStatus>),
    TestConnection(PluginResult<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
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
#[allow(clippy::large_enum_variant)]
pub enum PluginSubtitleCommand {
    ValidateConfig(SubtitlePluginValidateConfigRequest),
    Search(SubtitlePluginSearchRequest),
    Download(SubtitlePluginDownloadRequest),
    Generate(SubtitlePluginGenerateRequest),
    /// Subtitle-*sync* alignment, probing and window decoding.
    ///
    /// Alignment was never part of this envelope: it rode its own
    /// [`SubtitleSyncPluginProcessRequest`] over a stdin/stdout transport the
    /// host no longer has. The payload here is that request type
    /// **verbatim** — the same `Align` / `Probe` / `DecodeWindow` operations,
    /// the same nested request and response structs — so a migrating sync
    /// plugin keeps its types and its dispatch `match` and only changes how
    /// the bytes arrive, exactly as every other family did.
    ///
    /// It is a subtitle operation rather than a family of its own because a
    /// sync plugin already ships a `ProviderDescriptor::Subtitle` (with
    /// `SubtitleCapabilities::mode == Sync`), and the
    /// `scryer:subtitle/subtitle-provider@1.0.0` world's payload is opaque
    /// UTF-8 JSON. Adding the operation here therefore needs no WIT revision,
    /// no new world, and no new component host.
    Sync(SubtitleSyncPluginProcessRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum PluginSubtitleCommandResult {
    ValidateConfig(PluginResult<SubtitlePluginValidateConfigResponse>),
    Search(PluginResult<SubtitlePluginSearchResponse>),
    Download(PluginResult<SubtitlePluginDownloadResponse>),
    Generate(PluginResult<SubtitlePluginGenerateResponse>),
    /// The answer to [`PluginSubtitleCommand::Sync`], wrapped in the same
    /// [`PluginResult`] every other operation uses so a catalog-only provider
    /// asked to align can refuse in-band rather than by trapping.
    Sync(PluginResult<SubtitleSyncPluginProcessResponse>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PluginDownloadFeedbackScope;

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

    #[test]
    fn legacy_unscoped_download_command_shape_is_unchanged() {
        let request = PluginCommandRequest::new(PluginCommand::DownloadClient(
            PluginDownloadClientCommand::ListQueue,
        ));
        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(
            value,
            serde_json::json!({
                "abi_version": COMMAND_ABI_VERSION,
                "command": {
                    "family": "download_client",
                    "command": {
                        "operation": "list_queue"
                    }
                }
            })
        );
    }

    #[test]
    fn category_scoped_download_command_round_trips() {
        let request = PluginCommandRequest::new(PluginCommand::DownloadClient(
            PluginDownloadClientCommand::ListRecentCompletedScoped(
                PluginDownloadScopedRecentCompletedRequest {
                    limit: 25,
                    scope: PluginDownloadFeedbackScope {
                        categories: vec!["Movies".to_string(), "TV / Anime".to_string()],
                    },
                },
            ),
        ));
        let json = serde_json::to_string(&request).expect("serialize request");
        let decoded: PluginCommandRequest =
            serde_json::from_str(&json).expect("deserialize request");
        let PluginCommand::DownloadClient(PluginDownloadClientCommand::ListRecentCompletedScoped(
            request,
        )) = decoded.command
        else {
            panic!("expected scoped recent-completed command");
        };
        assert_eq!(request.limit, 25);
        assert_eq!(request.scope.categories, vec!["Movies", "TV / Anime"]);
    }

    /// The whole point of the sync variant: the payload is the original sync
    /// request type verbatim, so an align job survives the envelope unchanged.
    #[test]
    fn a_subtitle_sync_align_command_round_trips_verbatim() {
        let request = PluginCommandRequest::new(PluginCommand::Subtitle(
            PluginSubtitleCommand::Sync(SubtitleSyncPluginProcessRequest {
                operation: crate::SubtitleSyncPluginOperation::Align {
                    request: Box::new(crate::SubtitleSyncCommandAlignRequest {
                        input: crate::SubtitleSyncCommandInputFile {
                            path: "/input/movie.mkv".into(),
                        },
                        subtitle: crate::SubtitleSyncCommandSubtitleFile {
                            path: "/subtitle/original.srt".into(),
                            format: "srt".to_string(),
                            file_name: Some("original.srt".to_string()),
                            encoding_hint: None,
                        },
                        reference_subtitle: None,
                        output: crate::SubtitleSyncCommandOutputTarget {
                            path: "/output/rewritten.srt".into(),
                            format: "srt".to_string(),
                        },
                        scratch_dir: "/scratch".into(),
                        media_metadata: None,
                        subtitle_spans: Vec::new(),
                        max_offset_seconds: 60,
                        sync_options: None,
                        selector: None,
                        expected_codec: None,
                    }),
                },
            }),
        ));
        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(value["command"]["family"], "subtitle");
        assert_eq!(value["command"]["command"]["operation"], "sync");
        assert_eq!(
            value["command"]["command"]["request"]["operation"]["kind"],
            "align"
        );

        let decoded: PluginCommandRequest =
            serde_json::from_value(value).expect("deserialize request");
        let PluginCommand::Subtitle(PluginSubtitleCommand::Sync(request)) = decoded.command else {
            panic!("expected a subtitle sync command");
        };
        let crate::SubtitleSyncPluginOperation::Align { request } = request.operation else {
            panic!("expected an align operation");
        };
        assert_eq!(request.max_offset_seconds, 60);
        assert_eq!(
            request.output.path,
            std::path::Path::new("/output/rewritten.srt")
        );
    }

    /// The result half, including the in-band refusal a catalog-only provider
    /// answers with when it is handed an align it cannot serve.
    #[test]
    fn a_subtitle_sync_result_round_trips_both_arms() {
        let ok = PluginCommandResponse::new(PluginCommandResult::Subtitle(
            PluginSubtitleCommandResult::Sync(crate::PluginResult::Ok(
                SubtitleSyncPluginProcessResponse {
                    response: crate::SubtitleSyncPluginResponse::Probe {
                        response: crate::SubtitleSyncProbeResponse {
                            codec: None,
                            supported: false,
                            backend: "fixture".to_string(),
                            confidence: 0.0,
                            sample_rate_hz: None,
                            notes: Vec::new(),
                        },
                    },
                },
            )),
        ));
        let json = serde_json::to_string(&ok).expect("serialize response");
        let decoded: PluginCommandResponse =
            serde_json::from_str(&json).expect("deserialize response");
        assert!(matches!(
            decoded.response,
            PluginCommandResult::Subtitle(PluginSubtitleCommandResult::Sync(
                crate::PluginResult::Ok(_)
            ))
        ));

        let refused = PluginCommandResponse::new(PluginCommandResult::Subtitle(
            PluginSubtitleCommandResult::Sync(crate::PluginResult::Err(crate::PluginError {
                code: crate::PluginErrorCode::Unsupported,
                public_message: "this provider does not align subtitles".to_string(),
                debug_message: None,
                retry_after_seconds: None,
                details: None,
            })),
        ));
        let json = serde_json::to_string(&refused).expect("serialize refusal");
        let decoded: PluginCommandResponse =
            serde_json::from_str(&json).expect("deserialize refusal");
        let PluginCommandResult::Subtitle(PluginSubtitleCommandResult::Sync(
            crate::PluginResult::Err(error),
        )) = decoded.response
        else {
            panic!("expected an in-band sync refusal");
        };
        assert_eq!(error.code, crate::PluginErrorCode::Unsupported);
    }
}
