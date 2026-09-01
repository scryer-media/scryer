use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, NotificationClient, NotificationEpisodePayload,
    NotificationMediaUpdateTypePayload, NotificationPayload, NotificationSeverityPayload,
};
use scryer_domain::NotificationEventType as DomainNotificationEventType;
use scryer_plugin_sdk::PluginResult;
use scryer_plugin_sdk::command::{
    PluginActionRequest, PluginCommand, PluginCommandRequest, PluginCommandResult,
    PluginNotificationCommand, PluginNotificationCommandResult,
};
use tracing::warn;

use crate::blocking::run_blocking_plugin_call;
use crate::legacy_runtime::LegacyPlugin;
use crate::runtime_backing::PluginInstanceSpec;
use crate::socket_host::SocketHost;
use crate::types::{
    EXPORT_NOTIFICATION_ACTION, EXPORT_NOTIFICATION_SEND, NotificationEventType, PluginDescriptor,
    PluginNotificationActor, PluginNotificationApp, PluginNotificationApplicationUpdate,
    PluginNotificationDownload, PluginNotificationEpisode, PluginNotificationExternalIds,
    PluginNotificationFile, PluginNotificationHealth, PluginNotificationImport,
    PluginNotificationManualInteraction, PluginNotificationMediaFile,
    PluginNotificationMediaRequest, PluginNotificationMediaUpdate, PluginNotificationRequest,
    PluginNotificationResponse, PluginNotificationTitle, decode_plugin_result,
};
use crate::wasmtime_host::{NotificationComponentInvocation, process_notification_component};

const NOTIFICATION_PLUGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Which runtime serves this channel's operations.
///
/// The legacy reactor is instantiated once and re-entered under a mutex; the
/// component is instance-per-request and so retains only the spec it will
/// instantiate from. What the two arms deliberately do *not* differ in is
/// authority: both are built from the same [`crate::legacy_runtime::LegacyPluginSpec`],
/// so the config, the allowed hosts, the timeout and — the part that matters for
/// this family — the same `SocketHost` and `ProcessHost` reach the guest either
/// way. See `create_notification_client` in the loader, where that single
/// construction lives.
enum NotificationRuntime {
    Legacy(Arc<Mutex<LegacyPlugin>>),
    Component(NotificationComponent),
}

/// State for a `scryer:notification/notification@1.0.0` component channel.
struct NotificationComponent {
    spec: PluginInstanceSpec,
    /// One invocation at a time.
    ///
    /// The component is instance-per-request, but the channel's socket handle
    /// table is not: it lives in the `CommandHost` and is torn down by
    /// `SocketHost::cleanup` after every send. Two concurrent sends would
    /// therefore close each other's sockets. The legacy reactor serializes for
    /// its own reason (one instance, one mutex); this keeps the observable
    /// behaviour the same.
    invocation_lock: tokio::sync::Mutex<()>,
}

pub struct WasmNotificationClient {
    runtime: NotificationRuntime,
    descriptor: PluginDescriptor,
    channel_name: String,
    socket_host: Option<SocketHost>,
}

impl WasmNotificationClient {
    pub fn new(
        plugin: LegacyPlugin,
        descriptor: PluginDescriptor,
        channel_name: String,
        socket_host: Option<SocketHost>,
    ) -> Self {
        Self {
            runtime: NotificationRuntime::Legacy(Arc::new(Mutex::new(plugin))),
            descriptor,
            channel_name,
            socket_host,
        }
    }

    /// A `scryer:notification/notification@1.0.0` component channel.
    ///
    /// Deliberately takes the same `socket_host` the legacy constructor does,
    /// and a spec re-projected from the same `LegacyPluginSpec`: a channel that
    /// migrates its transport must not also change its authority or its
    /// per-send socket cleanup.
    pub fn new_component(
        spec: PluginInstanceSpec,
        descriptor: PluginDescriptor,
        channel_name: String,
        socket_host: Option<SocketHost>,
    ) -> Self {
        Self {
            runtime: NotificationRuntime::Component(NotificationComponent {
                spec,
                invocation_lock: tokio::sync::Mutex::new(()),
            }),
            descriptor,
            channel_name,
            socket_host,
        }
    }

    /// The component state behind this channel, or `None` on a legacy artifact.
    fn component(&self) -> Option<&NotificationComponent> {
        match &self.runtime {
            NotificationRuntime::Component(component) => Some(component),
            NotificationRuntime::Legacy(_) => None,
        }
    }

    fn legacy_plugin(&self) -> AppResult<Arc<Mutex<LegacyPlugin>>> {
        match &self.runtime {
            NotificationRuntime::Legacy(plugin) => Ok(Arc::clone(plugin)),
            NotificationRuntime::Component(_) => Err(AppError::Repository(format!(
                "notification plugin {} is a component and has no legacy exports",
                self.descriptor.id
            ))),
        }
    }

    /// Run one `PluginNotificationCommand` through the component world.
    ///
    /// The socket table is released after every invocation exactly as the
    /// legacy path releases it after every export call — the component's own
    /// `Store` is gone by then, but the handles are the channel's, not the
    /// store's.
    async fn invoke_component(
        &self,
        component: &NotificationComponent,
        command: PluginNotificationCommand,
        operation: &'static str,
    ) -> AppResult<PluginNotificationCommandResult> {
        let _guard = component.invocation_lock.lock().await;
        let response = process_notification_component(
            &component.spec,
            &PluginCommandRequest::new(PluginCommand::Notification(command)),
            NotificationComponentInvocation {
                plugin_id: &self.descriptor.id,
                plugin_version: &self.descriptor.version,
                operation,
            },
        )
        .await;
        if let Some(socket_host) = &self.socket_host {
            socket_host.cleanup();
        }
        match response?.response {
            PluginCommandResult::Notification(result) => Ok(result),
            _ => Err(AppError::Repository(format!(
                "notification plugin {} returned a response for another plugin family",
                self.descriptor.id
            ))),
        }
    }

    #[allow(dead_code)]
    pub async fn notification_action(
        &self,
        action: &str,
        query: BTreeMap<String, String>,
    ) -> AppResult<Option<serde_json::Value>> {
        if let Some(component) = self.component() {
            let result = self
                .invoke_component(
                    component,
                    PluginNotificationCommand::Action(PluginActionRequest {
                        action: action.to_string(),
                        payload: serde_json::json!({ "query": query }),
                    }),
                    "notification_action",
                )
                .await?;
            let PluginNotificationCommandResult::Action(result) = result else {
                return Err(AppError::Repository(format!(
                    "notification plugin {} answered an action command with another operation",
                    self.descriptor.id
                )));
            };
            return match result {
                PluginResult::Ok(response) => Ok(Some(response.payload)),
                PluginResult::Err(error) => Err(AppError::Repository(format!(
                    "notification {EXPORT_NOTIFICATION_ACTION} failed: plugin error {:?}: {}",
                    error.code, error.public_message
                ))),
            };
        }

        let request = serde_json::json!({
            "action": action,
            "query": query,
        });
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!(
                "failed to serialize notification action request: {e}"
            ))
        })?;

        let plugin = self.legacy_plugin()?;
        let socket_host = self.socket_host.clone();
        let output = run_blocking_plugin_call(
            NOTIFICATION_PLUGIN_TIMEOUT,
            "notification plugin",
            move || {
                let mut guard = plugin
                    .lock()
                    .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
                if !guard.function_exists(EXPORT_NOTIFICATION_ACTION) {
                    if let Some(socket_host) = socket_host {
                        socket_host.cleanup();
                    }
                    return Ok(None);
                }

                let output = guard.call_string(EXPORT_NOTIFICATION_ACTION, &input);
                if let Some(socket_host) = socket_host {
                    socket_host.cleanup();
                }
                output.map(Some).map_err(|e| {
                    AppError::Repository(format!(
                        "plugin {EXPORT_NOTIFICATION_ACTION}() failed: {e}"
                    ))
                })
            },
        )
        .await?;

        output
            .map(|output| decode_plugin_result(&output, EXPORT_NOTIFICATION_ACTION))
            .transpose()
    }
}

#[async_trait]
impl NotificationClient for WasmNotificationClient {
    async fn send_notification(&self, payload: &NotificationPayload) -> AppResult<()> {
        let request = PluginNotificationRequest {
            schema_version: payload.schema_version,
            event_type: map_event_type(payload.event_type),
            event_id: payload.event_id.clone(),
            occurred_at: payload.occurred_at.clone(),
            correlation_id: payload.correlation_id.clone(),
            actor: payload.actor.as_ref().map(|actor| PluginNotificationActor {
                user_id: actor.user_id.clone(),
            }),
            severity: payload.severity.map(map_severity),
            is_test: payload.is_test,
            summary_title: payload.summary_title.clone(),
            summary_message: payload.summary_message.clone(),
            app: PluginNotificationApp {
                name: payload.app.name.clone(),
                version: payload.app.version.clone(),
            },
            title: payload.title.as_ref().map(|title| PluginNotificationTitle {
                id: title.id.clone(),
                name: title.name.clone(),
                facet: title.facet.clone(),
                year: title.year,
                slug: title.slug.clone(),
                path: title.path.clone(),
                overview: title.overview.clone(),
                sort_title: title.sort_title.clone(),
                background_url: title.background_url.clone(),
                poster_url: title.poster_url.clone(),
                tags: title.tags.clone(),
                aliases: title.aliases.clone(),
                original_language: title.original_language.clone(),
                original_country: title.original_country.clone(),
                external_ids: PluginNotificationExternalIds {
                    tmdb_id: title.external_ids.tmdb_id.clone(),
                    imdb_id: title.external_ids.imdb_id.clone(),
                    tvdb_id: title.external_ids.tvdb_id.clone(),
                    anidb_id: title.external_ids.anidb_id.clone(),
                    tvmaze_id: title.external_ids.tvmaze_id.clone(),
                    anilist_ids: title.external_ids.anilist_ids.clone(),
                    mal_ids: title.external_ids.mal_ids.clone(),
                    kitsu_ids: title.external_ids.kitsu_ids.clone(),
                    by_source: title.external_ids.by_source.clone(),
                },
            }),
            episode: payload.episode.as_ref().map(map_episode),
            episodes: payload.episodes.iter().map(map_episode).collect(),
            release: payload.release.as_ref().map(|release| {
                crate::types::PluginNotificationRelease {
                    source_title: release.source_title.clone(),
                    source_hint: release.source_hint.clone(),
                    quality: release.quality.clone(),
                    provider: release.provider.clone(),
                    language: release.language.clone(),
                    release_group: release.release_group.clone(),
                    protocol: release.protocol.clone(),
                    indexer: release.indexer.clone(),
                    languages: release.languages.clone(),
                    custom_scores: release.custom_scores.clone(),
                }
            }),
            download: payload
                .download
                .as_ref()
                .map(|download| PluginNotificationDownload {
                    download_id: download.download_id.clone(),
                    client_id: download.client_id.clone(),
                    client_name: download.client_name.clone(),
                    client_type: download.client_type.clone(),
                    title: download.title.clone(),
                    status: download.status.clone(),
                    status_message: download.status_message.clone(),
                    size_bytes: download.size_bytes,
                    progress_percent: download.progress_percent,
                    output_path: download.output_path.clone(),
                }),
            import: payload
                .import
                .as_ref()
                .map(|import| PluginNotificationImport {
                    import_id: import.import_id.clone(),
                    source_system: import.source_system.clone(),
                    source_ref: import.source_ref.clone(),
                    source_title: import.source_title.clone(),
                    source_path: import.source_path.clone(),
                    dest_path: import.dest_path.clone(),
                    imported_count: import.imported_count,
                    status: import.status.clone(),
                    skipped_count: import.skipped_count,
                    rejected_count: import.rejected_count,
                    upgrade: import.upgrade,
                    deleted_paths: import.deleted_paths.clone(),
                    replaced_paths: import.replaced_paths.clone(),
                }),
            health: payload
                .health
                .as_ref()
                .map(|health| PluginNotificationHealth {
                    status: health.status.clone(),
                    message: health.message.clone(),
                    severity: health.severity.clone(),
                    code: health.code.clone(),
                    details: health.details.clone(),
                }),
            file: payload.file.as_ref().map(|file| PluginNotificationFile {
                primary_path: file.primary_path.clone(),
                media_updates: file
                    .media_updates
                    .iter()
                    .map(|update| PluginNotificationMediaUpdate {
                        path: update.path.clone(),
                        update_type: match update.update_type {
                            NotificationMediaUpdateTypePayload::Created => {
                                crate::types::NotificationMediaUpdateType::Created
                            }
                            NotificationMediaUpdateTypePayload::Modified => {
                                crate::types::NotificationMediaUpdateType::Modified
                            }
                            NotificationMediaUpdateTypePayload::Deleted => {
                                crate::types::NotificationMediaUpdateType::Deleted
                            }
                        },
                    })
                    .collect(),
            }),
            media_files: payload
                .media_files
                .iter()
                .map(|media_file| PluginNotificationMediaFile {
                    id: media_file.id.clone(),
                    path: media_file.path.clone(),
                    previous_path: media_file.previous_path.clone(),
                    recycle_bin_path: media_file.recycle_bin_path.clone(),
                    size_bytes: media_file.size_bytes,
                    quality: media_file.quality.clone(),
                    release_group: media_file.release_group.clone(),
                    scene_name: media_file.scene_name.clone(),
                    audio_languages: media_file.audio_languages.clone(),
                    subtitle_languages: media_file.subtitle_languages.clone(),
                    video_codec: media_file.video_codec.clone(),
                    audio_codec: media_file.audio_codec.clone(),
                    audio_channels: media_file.audio_channels.clone(),
                    video_width: media_file.video_width,
                    video_height: media_file.video_height,
                    video_bit_depth: media_file.video_bit_depth,
                    video_hdr_format: media_file.video_hdr_format.clone(),
                    video_frame_rate: media_file.video_frame_rate.clone(),
                    container_format: media_file.container_format.clone(),
                    edition: media_file.edition.clone(),
                })
                .collect(),
            application_update: payload.application_update.as_ref().map(|update| {
                PluginNotificationApplicationUpdate {
                    current_version: update.current_version.clone(),
                    target_version: update.target_version.clone(),
                    status: update.status.clone(),
                    summary: update.summary.clone(),
                }
            }),
            manual_interaction: payload.manual_interaction.as_ref().map(|manual| {
                PluginNotificationManualInteraction {
                    kind: manual.kind.clone(),
                    reason: manual.reason.clone(),
                    link: manual.link.clone(),
                }
            }),
            media_request: payload.media_request.as_ref().map(|request| {
                PluginNotificationMediaRequest {
                    request_id: request.request_id.clone(),
                    library_id: request.library_id.clone(),
                    status: request.status.clone(),
                    facet: request.facet.clone(),
                    requested_quality_profile_id: request.requested_quality_profile_id.clone(),
                    requested_quality_profile_name: request.requested_quality_profile_name.clone(),
                    requested_monitor_type: request.requested_monitor_type.clone(),
                    approved_quality_profile_id: request.approved_quality_profile_id.clone(),
                    approved_quality_profile_name: request.approved_quality_profile_name.clone(),
                    created_title_id: request.created_title_id.clone(),
                }
            }),
        };

        let plugin_name = self.descriptor.name.clone();
        let channel_name = self.channel_name.clone();

        // Both transports carry the identical `PluginNotificationRequest`
        // document; only the envelope around it differs, which is the whole
        // point of the world. Everything past this `if` — the warning log, the
        // failure projection — is therefore shared.
        let response: PluginNotificationResponse = if let Some(component) = self.component() {
            let result = self
                .invoke_component(
                    component,
                    PluginNotificationCommand::Send(request),
                    "notification_send",
                )
                .await?;
            let PluginNotificationCommandResult::Send(result) = result else {
                return Err(AppError::Repository(format!(
                    "notification plugin {} answered a send command with another operation",
                    self.descriptor.id
                )));
            };
            match result {
                PluginResult::Ok(response) => response,
                PluginResult::Err(error) => {
                    return Err(AppError::Repository(format!(
                        "notification {EXPORT_NOTIFICATION_SEND} failed: plugin error {:?}: {}",
                        error.code, error.public_message
                    )));
                }
            }
        } else {
            let input = serde_json::to_string(&request).map_err(|e| {
                AppError::Repository(format!("failed to serialize notification request: {e}"))
            })?;
            let plugin = self.legacy_plugin()?;
            let socket_host = self.socket_host.clone();
            let output = run_blocking_plugin_call(
                NOTIFICATION_PLUGIN_TIMEOUT,
                "notification plugin",
                move || {
                    let mut guard = plugin
                        .lock()
                        .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
                    let output = guard.call_string(EXPORT_NOTIFICATION_SEND, &input);
                    if let Some(socket_host) = socket_host {
                        socket_host.cleanup();
                    }
                    output.map_err(|e| {
                        AppError::Repository(format!(
                            "plugin {EXPORT_NOTIFICATION_SEND}() failed: {e}"
                        ))
                    })
                },
            )
            .await?;
            decode_plugin_result(&output, EXPORT_NOTIFICATION_SEND)?
        };

        for warning_message in &response.warnings {
            warn!(
                plugin = plugin_name.as_str(),
                channel = channel_name.as_str(),
                warning = warning_message.as_str(),
                "notification plugin returned warning"
            );
        }

        if !response.success {
            let err_msg = match (&response.error, &response.provider_status) {
                (Some(error), Some(provider_status)) => {
                    format!("{error} ({provider_status})")
                }
                (Some(error), None) => error.clone(),
                (None, Some(provider_status)) => provider_status.clone(),
                (None, None) => "unknown error".to_string(),
            };
            warn!(
                plugin = plugin_name.as_str(),
                channel = channel_name.as_str(),
                error = err_msg.as_str(),
                delivery_id = ?response.delivery_id,
                retry_after_seconds = ?response.retry_after_seconds,
                "notification plugin reported failure"
            );
            return Err(AppError::Repository(format!(
                "notification failed: {err_msg}"
            )));
        }

        Ok(())
    }
}

fn map_episode(episode: &NotificationEpisodePayload) -> PluginNotificationEpisode {
    PluginNotificationEpisode {
        id: episode.id.clone(),
        episode_ids: episode.episode_ids.clone(),
        media_file_id: episode.media_file_id.clone(),
        media_file_path: episode.media_file_path.clone(),
        display: episode.display.clone(),
        collection_id: episode.collection_id.clone(),
        season_number: episode.season_number.clone(),
        episode_number: episode.episode_number.clone(),
        absolute_number: episode.absolute_number.clone(),
        title: episode.title.clone(),
        overview: episode.overview.clone(),
        air_date: episode.air_date.clone(),
        air_date_utc: episode.air_date_utc.clone(),
        episode_type: episode.episode_type.clone(),
        finale_type: episode.finale_type.clone(),
        tvdb_id: episode.tvdb_id.clone(),
    }
}

fn map_severity(severity: NotificationSeverityPayload) -> crate::types::NotificationSeverity {
    match severity {
        NotificationSeverityPayload::Info => crate::types::NotificationSeverity::Info,
        NotificationSeverityPayload::Warning => crate::types::NotificationSeverity::Warning,
        NotificationSeverityPayload::Error => crate::types::NotificationSeverity::Error,
    }
}

fn map_event_type(event_type: DomainNotificationEventType) -> NotificationEventType {
    match event_type {
        DomainNotificationEventType::Grab => NotificationEventType::Grab,
        DomainNotificationEventType::Download => NotificationEventType::Download,
        DomainNotificationEventType::Upgrade => NotificationEventType::Upgrade,
        DomainNotificationEventType::ImportComplete => NotificationEventType::ImportComplete,
        DomainNotificationEventType::ImportRejected => NotificationEventType::ImportRejected,
        DomainNotificationEventType::Rename => NotificationEventType::Rename,
        DomainNotificationEventType::TitleAdded => NotificationEventType::TitleAdded,
        DomainNotificationEventType::TitleDeleted => NotificationEventType::TitleDeleted,
        DomainNotificationEventType::FileDeleted => NotificationEventType::FileDeleted,
        DomainNotificationEventType::FileDeletedForUpgrade => {
            NotificationEventType::FileDeletedForUpgrade
        }
        DomainNotificationEventType::PostProcessingCompleted => {
            NotificationEventType::PostProcessingCompleted
        }
        DomainNotificationEventType::SubtitleDownloaded => {
            NotificationEventType::SubtitleDownloaded
        }
        DomainNotificationEventType::SubtitleSearchFailed => {
            NotificationEventType::SubtitleSearchFailed
        }
        DomainNotificationEventType::MediaRequestSubmitted => {
            NotificationEventType::MediaRequestSubmitted
        }
        DomainNotificationEventType::MediaRequestApproved => {
            NotificationEventType::MediaRequestApproved
        }
        DomainNotificationEventType::MediaRequestRejected => {
            NotificationEventType::MediaRequestRejected
        }
        DomainNotificationEventType::MediaRequestCanceled => {
            NotificationEventType::MediaRequestCanceled
        }
        DomainNotificationEventType::HealthIssue => NotificationEventType::HealthIssue,
        DomainNotificationEventType::HealthRestored => NotificationEventType::HealthRestored,
        DomainNotificationEventType::ApplicationUpdate => NotificationEventType::ApplicationUpdate,
        DomainNotificationEventType::ManualInteractionRequired => {
            NotificationEventType::ManualInteractionRequired
        }
        DomainNotificationEventType::Test => NotificationEventType::Test,
    }
}

#[cfg(test)]
mod component_routing_tests {
    use super::*;
    use crate::legacy_runtime::LegacyPluginSpec;
    use crate::process_host::ProcessHost;
    use crate::wasmtime_host::command_host::CommandHost;
    use crate::wasmtime_host::notification_component_host::tests::{
        echo_listener, loopback_permission, notification_descriptor, socket_fixture_component,
    };
    use scryer_application::{NotificationAppPayload, NotificationPayload};

    /// The channel a component notifier is built from, wired exactly as the
    /// loader wires it: one `SocketHost` reaching both the `CommandHost` service
    /// arms and (on a reactor) the legacy registrations.
    fn component_channel(
        descriptor: &PluginDescriptor,
        port: u16,
    ) -> (WasmNotificationClient, SocketHost) {
        let socket_host = SocketHost::from_descriptor(descriptor, None);
        let command_host = CommandHost::for_notification(
            descriptor.id.clone(),
            std::collections::BTreeMap::new(),
            Vec::new(),
            NOTIFICATION_PLUGIN_TIMEOUT,
            None,
            None,
            socket_host.clone(),
            ProcessHost::disabled(),
        );
        let spec = PluginInstanceSpec {
            wasm: Arc::new(socket_fixture_component(descriptor, port)),
            preopens: Vec::new(),
            timeout: NOTIFICATION_PLUGIN_TIMEOUT,
            memory_max_bytes: None,
            command_host,
        };
        (
            WasmNotificationClient::new_component(
                spec,
                descriptor.clone(),
                "Fixture Channel".to_string(),
                Some(socket_host.clone()),
            ),
            socket_host,
        )
    }

    fn test_payload() -> NotificationPayload {
        NotificationPayload {
            schema_version: 1,
            event_type: scryer_domain::NotificationEventType::Test,
            event_id: None,
            occurred_at: None,
            correlation_id: None,
            actor: None,
            severity: None,
            is_test: true,
            summary_title: "fixture".to_string(),
            summary_message: "fixture".to_string(),
            app: NotificationAppPayload {
                name: "scryer".to_string(),
                version: "0.0.0".to_string(),
            },
            title: None,
            episode: None,
            episodes: Vec::new(),
            release: None,
            download: None,
            import: None,
            health: None,
            file: None,
            media_files: Vec::new(),
            application_update: None,
            manual_interaction: None,
            media_request: None,
        }
    }

    /// A component channel answers a real `NotificationClient` trait call
    /// through the component host, and the socket authority the loader gave it
    /// reaches the guest.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_component_channel_sends_through_the_component_host() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)]);
        let (client, _socket_host) = component_channel(&descriptor, port);

        client
            .send_notification(&test_payload())
            .await
            .expect("the component channel must complete a send");
        listener.join().ok();
    }

    /// The per-send socket teardown the reactor path has always done happens on
    /// the component path too: the channel's handle table is empty once the
    /// send returns, even though the guest never closed its socket.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_component_send_releases_the_channels_sockets() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)]);
        let (client, socket_host) = component_channel(&descriptor, port);

        client
            .send_notification(&test_payload())
            .await
            .expect("the component channel must complete a send");
        listener.join().ok();

        assert_eq!(
            socket_host.open_socket_count(),
            0,
            "a completed send must leave no socket open on the channel",
        );
    }

    /// The component path is additive: a core-module artifact still builds a
    /// legacy reactor and never reaches the component host.
    #[test]
    fn a_core_module_artifact_still_selects_the_legacy_runtime() {
        let core_module = wat::parse_str("(module (memory (export \"memory\") 1))")
            .expect("core module WAT must parse");
        let spec = LegacyPluginSpec::new(core_module, "fixture-notification".to_string());
        let plugin = LegacyPlugin::instantiate(spec).expect("legacy reactor must instantiate");

        let client = WasmNotificationClient::new(
            plugin,
            notification_descriptor(Vec::new()),
            "Fixture Channel".to_string(),
            None,
        );

        assert!(
            client.component().is_none(),
            "a non-component notifier must keep the legacy path"
        );
        client
            .legacy_plugin()
            .expect("the legacy reactor must be reachable");
    }

    /// And symmetrically: a component channel has no legacy exports to reach.
    #[test]
    fn a_component_channel_has_no_legacy_exports() {
        let descriptor = notification_descriptor(Vec::new());
        let (client, _socket_host) = component_channel(&descriptor, 1);

        assert!(client.component().is_some());
        assert!(client.legacy_plugin().is_err());
    }
}
