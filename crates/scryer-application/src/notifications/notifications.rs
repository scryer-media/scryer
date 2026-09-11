use chrono::Utc;
use scryer_domain::{
    Id, MediaServerConnection, MediaServerProvider, NotificationChannelConfig,
    NotificationEventType, NotificationSubscription, NotificationTarget, NotificationTargetKind,
};
use url::Url;

use crate::ports::NOTIFICATION_REQUEST_SCHEMA_VERSION;
use crate::{
    AppError, AppResult, AppUseCase, NotificationAppPayload, NotificationPayload,
    NotificationScopeIdUpdate,
};

fn parse_subscribable_notification_event_type(
    event_type: &str,
) -> AppResult<NotificationEventType> {
    let parsed = NotificationEventType::parse(event_type).ok_or_else(|| {
        AppError::Validation(format!("unknown notification event type: {event_type}"))
    })?;

    if crate::notifications::dispatcher::supported_notification_event_types()
        .iter()
        .any(|candidate| candidate == &parsed)
    {
        Ok(parsed)
    } else {
        Err(AppError::Validation(format!(
            "notification event type is not subscribable: {event_type}"
        )))
    }
}

fn is_media_server_notification_provider(provider_type: &str) -> bool {
    MediaServerProvider::parse(provider_type).is_some()
}

struct NotificationChannelCreateInternal {
    name: String,
    channel_type: String,
    config_json: String,
    media_server_connection_id: Option<String>,
    is_enabled: bool,
    reject_media_provider: bool,
}

struct NotificationChannelUpdateInternal {
    id: String,
    name: Option<String>,
    config_json: Option<String>,
    media_server_connection_id: Option<Option<String>>,
    is_enabled: Option<bool>,
    reject_media_provider: bool,
}

pub struct NotificationSubscriptionTargetCreate {
    pub channel_id: Option<String>,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
}

pub struct NotificationSubscriptionTargetUpdate {
    pub id: String,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub event_type: Option<String>,
    pub scope: Option<String>,
    pub scope_id: NotificationScopeIdUpdate,
    pub is_enabled: Option<bool>,
}

impl AppUseCase {
    pub fn subscribable_notification_event_types(&self) -> &'static [NotificationEventType] {
        crate::notifications::dispatcher::supported_notification_event_types()
    }

    pub async fn list_notification_channels(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationChannelConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_channels()?;
        Ok(repo
            .list_channels()
            .await?
            .into_iter()
            .filter(|channel| !is_media_server_notification_provider(channel.channel_type.as_str()))
            .collect())
    }

    pub async fn list_notification_targets(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationTarget>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let mut targets = Vec::new();
        for channel in self.notification_channels()?.list_channels().await? {
            if is_media_server_notification_provider(channel.channel_type.as_str()) {
                continue;
            }
            targets.push(NotificationTarget {
                id: channel.id,
                target_kind: NotificationTargetKind::PluginChannel,
                name: channel.name,
                provider_type: channel.channel_type.as_str().to_string(),
                media_server_provider: None,
                media_server_connection_id: None,
                is_enabled: channel.is_enabled,
            });
        }

        for connection in self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?
        {
            let provider_type = connection.provider.as_str().to_string();
            targets.push(NotificationTarget {
                id: connection.id.clone(),
                target_kind: NotificationTargetKind::MediaServerConnection,
                name: connection.display_name,
                provider_type,
                media_server_provider: Some(connection.provider),
                media_server_connection_id: Some(connection.id),
                is_enabled: connection.enabled,
            });
        }

        Ok(targets)
    }

    pub async fn get_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<Option<NotificationChannelConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_channels()?;
        repo.get_channel(id).await
    }

    pub async fn create_notification_channel(
        &self,
        actor: &scryer_domain::User,
        name: String,
        channel_type: String,
        config_json: String,
        is_enabled: bool,
    ) -> AppResult<NotificationChannelConfig> {
        self.create_notification_channel_internal(
            actor,
            NotificationChannelCreateInternal {
                name,
                channel_type,
                config_json,
                media_server_connection_id: None,
                is_enabled,
                reject_media_provider: true,
            },
        )
        .await
    }

    pub async fn create_notification_channel_with_media_server_connection_id(
        &self,
        actor: &scryer_domain::User,
        name: String,
        channel_type: String,
        config_json: String,
        media_server_connection_id: Option<String>,
        is_enabled: bool,
    ) -> AppResult<NotificationChannelConfig> {
        self.create_notification_channel_internal(
            actor,
            NotificationChannelCreateInternal {
                name,
                channel_type,
                config_json,
                media_server_connection_id,
                is_enabled,
                reject_media_provider: true,
            },
        )
        .await
    }

    async fn create_notification_channel_internal(
        &self,
        actor: &scryer_domain::User,
        input: NotificationChannelCreateInternal,
    ) -> AppResult<NotificationChannelConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let NotificationChannelCreateInternal {
            name,
            channel_type,
            config_json,
            media_server_connection_id,
            is_enabled,
            reject_media_provider,
        } = input;

        if name.trim().is_empty() {
            return Err(AppError::Validation(
                "channel name must not be empty".into(),
            ));
        }
        let channel_type = scryer_domain::ChannelType::parse(channel_type.trim())
            .ok_or_else(|| AppError::Validation(format!("invalid channel type: {channel_type}")))?;
        if reject_media_provider && is_media_server_notification_provider(channel_type.as_str()) {
            return Err(AppError::Validation(
                "media server notification targets must be managed in Media Servers".into(),
            ));
        }
        if normalize_notification_media_server_connection_id(media_server_connection_id.clone())
            .is_some()
        {
            return Err(AppError::Validation(
                "notification channels cannot reference media server connections".into(),
            ));
        }

        let now = Utc::now();
        let config = NotificationChannelConfig {
            id: Id::new().0,
            name,
            channel_type,
            config_json,
            media_server_connection_id: normalize_notification_media_server_connection_id(
                media_server_connection_id,
            ),
            is_enabled,
            created_at: now,
            updated_at: now,
        };

        self.validate_notification_provider_required_config(
            config.channel_type.as_str(),
            &config.config_json,
        )?;

        let repo = self.notification_channels()?;
        repo.create_channel(config).await
    }

    pub async fn update_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: String,
        name: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
    ) -> AppResult<NotificationChannelConfig> {
        self.update_notification_channel_internal(
            actor,
            NotificationChannelUpdateInternal {
                id,
                name,
                config_json,
                media_server_connection_id: None,
                is_enabled,
                reject_media_provider: true,
            },
        )
        .await
    }

    pub async fn update_notification_channel_with_media_server_connection_id(
        &self,
        actor: &scryer_domain::User,
        id: String,
        name: Option<String>,
        config_json: Option<String>,
        media_server_connection_id: Option<Option<String>>,
        is_enabled: Option<bool>,
    ) -> AppResult<NotificationChannelConfig> {
        self.update_notification_channel_internal(
            actor,
            NotificationChannelUpdateInternal {
                id,
                name,
                config_json,
                media_server_connection_id,
                is_enabled,
                reject_media_provider: true,
            },
        )
        .await
    }

    async fn update_notification_channel_internal(
        &self,
        actor: &scryer_domain::User,
        input: NotificationChannelUpdateInternal,
    ) -> AppResult<NotificationChannelConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_channels()?;

        let NotificationChannelUpdateInternal {
            id,
            name,
            config_json,
            media_server_connection_id,
            is_enabled,
            reject_media_provider,
        } = input;

        let mut channel = repo
            .get_channel(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {id}")))?;

        if let Some(n) = name {
            if n.trim().is_empty() {
                return Err(AppError::Validation(
                    "channel name must not be empty".into(),
                ));
            }
            channel.name = n;
        }
        if let Some(c) = config_json {
            channel.config_json = c;
        }
        if let Some(media_server_connection_id) = media_server_connection_id {
            if normalize_notification_media_server_connection_id(media_server_connection_id)
                .is_some()
            {
                return Err(AppError::Validation(
                    "notification channels cannot reference media server connections".into(),
                ));
            }
            channel.media_server_connection_id = None;
        }
        if reject_media_provider
            && is_media_server_notification_provider(channel.channel_type.as_str())
        {
            return Err(AppError::Validation(
                "media server notification targets must be managed in Media Servers".into(),
            ));
        }
        if let Some(e) = is_enabled {
            channel.is_enabled = e;
        }
        self.validate_notification_provider_required_config(
            channel.channel_type.as_str(),
            &channel.config_json,
        )?;
        channel.updated_at = Utc::now();

        repo.update_channel(channel).await
    }

    pub async fn delete_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_channels()?;
        repo.delete_channel(id).await
    }

    pub async fn test_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let repo = self.notification_channels()?;
        let channel = repo
            .get_channel(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {id}")))?;

        let provider = self
            .services
            .notifications
            .notification_provider()
            .ok_or_else(|| {
                AppError::Repository("notification plugin provider is not configured".into())
            })?;

        // Checked before the capability probe: a blocked plugin is absent from
        // the provider, so the probe would report it as not supporting tests.
        self.ensure_provider_plugin_not_blocked(channel.channel_type.as_str())
            .await?;

        if !provider.supports_test_for_provider(channel.channel_type.as_str()) {
            return Err(AppError::Validation(format!(
                "notification provider '{}' does not support test notifications",
                channel.channel_type.as_str()
            )));
        }

        let channel = self
            .notification_channel_with_resolved_media_server_config(channel)
            .await?;
        let client = provider.client_for_channel(&channel).ok_or_else(|| {
            AppError::NotFound(format!(
                "no notification plugin for channel type '{}'",
                channel.channel_type.as_str()
            ))
        })?;

        client
            .send_notification(&NotificationPayload {
                schema_version: NOTIFICATION_REQUEST_SCHEMA_VERSION,
                event_type: NotificationEventType::Test,
                event_id: None,
                occurred_at: None,
                correlation_id: None,
                actor: None,
                severity: None,
                is_test: true,
                summary_title: "Scryer Test Notification".to_string(),
                summary_message: "This is a test notification from Scryer.".to_string(),
                app: NotificationAppPayload {
                    name: "Scryer".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
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
            })
            .await
    }

    pub async fn list_notification_subscriptions(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationSubscription>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_subscriptions()?;
        repo.list_subscriptions().await
    }

    pub async fn create_notification_subscription(
        &self,
        actor: &scryer_domain::User,
        channel_id: String,
        event_type: String,
        scope: String,
        scope_id: Option<String>,
        is_enabled: bool,
    ) -> AppResult<NotificationSubscription> {
        self.create_notification_subscription_for_target(
            actor,
            NotificationSubscriptionTargetCreate {
                channel_id: Some(channel_id),
                target_kind: None,
                target_id: None,
                event_type,
                scope,
                scope_id,
                is_enabled,
            },
        )
        .await
    }

    pub async fn create_notification_subscription_for_target(
        &self,
        actor: &scryer_domain::User,
        input: NotificationSubscriptionTargetCreate,
    ) -> AppResult<NotificationSubscription> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let NotificationSubscriptionTargetCreate {
            channel_id,
            target_kind,
            target_id,
            event_type,
            scope,
            scope_id,
            is_enabled,
        } = input;

        let parsed_event_type = parse_subscribable_notification_event_type(&event_type)?;
        let (target_kind, target_id) =
            normalize_notification_target(channel_id.as_deref(), target_kind, target_id)?;
        let channel_id = self
            .validate_notification_subscription_target(target_kind, &target_id, parsed_event_type)
            .await?;

        let now = Utc::now();
        let sub = NotificationSubscription {
            id: Id::new().0,
            channel_id,
            target_kind,
            target_id,
            event_type: parsed_event_type,
            scope,
            scope_id,
            is_enabled,
            created_at: now,
            updated_at: now,
        };

        let repo = self.notification_subscriptions()?;
        repo.create_subscription(sub).await
    }

    pub async fn update_notification_subscription(
        &self,
        actor: &scryer_domain::User,
        id: String,
        event_type: Option<String>,
        scope: Option<String>,
        scope_id: NotificationScopeIdUpdate,
        is_enabled: Option<bool>,
    ) -> AppResult<NotificationSubscription> {
        self.update_notification_subscription_target(
            actor,
            NotificationSubscriptionTargetUpdate {
                id,
                target_kind: None,
                target_id: None,
                event_type,
                scope,
                scope_id,
                is_enabled,
            },
        )
        .await
    }

    pub async fn update_notification_subscription_target(
        &self,
        actor: &scryer_domain::User,
        input: NotificationSubscriptionTargetUpdate,
    ) -> AppResult<NotificationSubscription> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_subscriptions()?;

        let NotificationSubscriptionTargetUpdate {
            id,
            target_kind,
            target_id,
            event_type,
            scope,
            scope_id,
            is_enabled,
        } = input;

        // Find all subscriptions and locate ours
        let all = repo.list_subscriptions().await?;
        let mut sub = all
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::NotFound(format!("notification subscription {id}")))?;

        if target_kind.is_some() || target_id.is_some() {
            let (next_kind, next_id) =
                normalize_notification_target(sub.channel_id.as_deref(), target_kind, target_id)?;
            let channel_id = self
                .validate_notification_subscription_target(next_kind, &next_id, sub.event_type)
                .await?;
            sub.target_kind = next_kind;
            sub.target_id = next_id;
            sub.channel_id = channel_id;
        }
        if let Some(et) = event_type {
            let parsed = parse_subscribable_notification_event_type(&et)?;
            let channel_id = self
                .validate_notification_subscription_target(sub.target_kind, &sub.target_id, parsed)
                .await?;
            sub.event_type = parsed;
            sub.channel_id = channel_id;
        }
        if let Some(s) = scope {
            sub.scope = s;
        }
        match scope_id {
            NotificationScopeIdUpdate::NoChange => {}
            NotificationScopeIdUpdate::Clear => sub.scope_id = None,
            NotificationScopeIdUpdate::Set(scope_id) => sub.scope_id = Some(scope_id),
        }
        if let Some(e) = is_enabled {
            sub.is_enabled = e;
        }
        sub.updated_at = Utc::now();

        repo.update_subscription(sub).await
    }

    pub async fn delete_notification_subscription(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let repo = self.notification_subscriptions()?;
        repo.delete_subscription(id).await
    }

    pub fn available_notification_provider_types(&self) -> Vec<String> {
        self.services
            .notifications
            .notification_provider()
            .map(|p| p.available_provider_types())
            .unwrap_or_default()
    }

    pub fn notification_provider_config_fields(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.services
            .notifications
            .notification_provider()
            .map(|p| p.config_fields_for_provider(provider_type))
            .unwrap_or_default()
    }

    pub fn notification_provider_name(&self, provider_type: &str) -> Option<String> {
        self.services
            .notifications
            .notification_provider()
            .and_then(|p| p.plugin_name_for_provider(provider_type))
    }

    pub fn notification_provider_supported_events(
        &self,
        provider_type: &str,
    ) -> Vec<NotificationEventType> {
        self.services
            .notifications
            .notification_provider()
            .map(|p| p.supported_events_for_provider(provider_type))
            .unwrap_or_default()
    }

    pub fn notification_provider_supports_test(&self, provider_type: &str) -> bool {
        self.services
            .notifications
            .notification_provider()
            .is_some_and(|p| p.supports_test_for_provider(provider_type))
    }

    async fn validate_notification_subscription_target(
        &self,
        target_kind: NotificationTargetKind,
        target_id: &str,
        event_type: NotificationEventType,
    ) -> AppResult<Option<String>> {
        let provider_type = match target_kind {
            NotificationTargetKind::PluginChannel => {
                let channel = self
                    .notification_channels()?
                    .get_channel(target_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("notification channel {target_id}"))
                    })?;
                if !channel.is_enabled {
                    return Err(AppError::Validation(
                        "notification channel is disabled".into(),
                    ));
                }
                if is_media_server_notification_provider(channel.channel_type.as_str()) {
                    return Err(AppError::Validation(
                        "media server notification providers must be targeted through media server connections".into(),
                    ));
                }
                self.validate_notification_provider_required_config(
                    channel.channel_type.as_str(),
                    &channel.config_json,
                )?;
                channel.channel_type.as_str().to_string()
            }
            NotificationTargetKind::MediaServerConnection => {
                let connection = self
                    .services
                    .integrations
                    .media_server_connections
                    .get_by_id(target_id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound(format!("media server connection {target_id}"))
                    })?;
                if !connection.enabled {
                    return Err(AppError::Validation(
                        "media server connection is disabled".into(),
                    ));
                }
                let provider_type = connection.provider.as_str().to_string();
                self.notification_channel_for_media_server_connection(connection)
                    .await?;
                provider_type
            }
        };

        let provider = self
            .services
            .notifications
            .notification_provider()
            .ok_or_else(|| {
                AppError::Repository("notification plugin provider is not configured".into())
            })?;
        if provider.plugin_name_for_provider(&provider_type).is_none()
            && !provider
                .available_provider_types()
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&provider_type))
        {
            return Err(AppError::Validation(format!(
                "notification provider '{provider_type}' is not available"
            )));
        }
        let supported_events = provider.supported_events_for_provider(&provider_type);
        if !supported_events.is_empty() && !supported_events.contains(&event_type) {
            return Err(AppError::Validation(format!(
                "notification provider '{provider_type}' does not support event '{}'",
                event_type.as_str()
            )));
        }

        Ok(match target_kind {
            NotificationTargetKind::PluginChannel => Some(target_id.to_string()),
            NotificationTargetKind::MediaServerConnection => None,
        })
    }

    pub(crate) async fn notification_channel_for_media_server_target(
        &self,
        connection_id: &str,
    ) -> AppResult<NotificationChannelConfig> {
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("media server connection {connection_id}"))
            })?;
        self.notification_channel_for_media_server_connection(connection)
            .await
    }

    async fn notification_channel_for_media_server_connection(
        &self,
        connection: MediaServerConnection,
    ) -> AppResult<NotificationChannelConfig> {
        let channel_type = scryer_domain::ChannelType::parse(connection.provider.as_str())
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "invalid media server provider '{}'",
                    connection.provider.as_str()
                ))
            })?;
        let mut channel = NotificationChannelConfig {
            id: format!("media-server:{}", connection.id),
            name: connection.display_name.clone(),
            channel_type,
            config_json: "{}".to_string(),
            media_server_connection_id: Some(connection.id.clone()),
            is_enabled: connection.enabled,
            created_at: connection.created_at,
            updated_at: connection.updated_at,
        };
        channel.config_json = self.media_server_notification_config_json(&connection, None)?;
        self.validate_notification_provider_required_config(
            connection.provider.as_str(),
            &channel.config_json,
        )?;
        Ok(channel)
    }

    pub(crate) async fn notification_channel_with_resolved_media_server_config(
        &self,
        mut channel: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        let Some(connection_id) = channel.media_server_connection_id.clone() else {
            self.validate_notification_provider_required_config(
                channel.channel_type.as_str(),
                &channel.config_json,
            )?;
            return Ok(channel);
        };
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(&connection_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("media server connection {connection_id}"))
            })?;
        if connection.provider.as_str() != channel.channel_type.as_str() {
            return Err(AppError::Validation(
                "notification channel media server connection provider does not match channel type"
                    .into(),
            ));
        }
        channel.config_json =
            self.media_server_notification_config_json(&connection, Some(&channel.config_json))?;
        self.validate_notification_provider_required_config(
            connection.provider.as_str(),
            &channel.config_json,
        )?;
        Ok(channel)
    }

    fn media_server_notification_config_json(
        &self,
        connection: &MediaServerConnection,
        existing_config_json: Option<&str>,
    ) -> AppResult<String> {
        let existing_config = existing_config_json
            .and_then(|config_json| serde_json::from_str::<serde_json::Value>(config_json).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let mut config = serde_json::Map::new();
        for (key, value) in existing_config {
            config.insert(key, value);
        }
        config.insert(
            "base_url".to_string(),
            serde_json::Value::String(connection.base_url.clone()),
        );
        if connection.provider == MediaServerProvider::Plex {
            let parsed = Url::parse(&connection.base_url).map_err(|err| {
                AppError::Validation(format!(
                    "invalid Plex media server base URL '{}': {err}",
                    connection.base_url
                ))
            })?;
            if let Some(host) = parsed
                .host_str()
                .map(str::trim)
                .filter(|host| !host.is_empty())
            {
                config.insert(
                    "host".to_string(),
                    serde_json::Value::String(host.to_string()),
                );
            } else {
                config.remove("host");
            }
            let port = parsed
                .port_or_known_default()
                .map(|port| port.to_string())
                .unwrap_or_else(|| "32400".to_string());
            config.insert("port".to_string(), serde_json::Value::String(port));
            config.insert(
                "use_ssl".to_string(),
                serde_json::Value::String((parsed.scheme() == "https").to_string()),
            );
            config.insert(
                "update_library".to_string(),
                serde_json::Value::String("true".into()),
            );
        }
        if let Some(api_key) = connection.api_key.as_deref() {
            config.insert(
                "api_key".to_string(),
                serde_json::Value::String(api_key.to_string()),
            );
            config.insert(
                "auth_token".to_string(),
                serde_json::Value::String(api_key.to_string()),
            );
        } else {
            config.remove("api_key");
            config.remove("auth_token");
        }
        if let Some(machine_id) = connection.machine_id.as_deref() {
            config.insert(
                "machine_id".to_string(),
                serde_json::Value::String(machine_id.to_string()),
            );
        } else {
            config.remove("machine_id");
        }
        if !connection.path_mappings.is_empty() {
            let rendered = connection
                .path_mappings
                .iter()
                .map(|mapping| format!("{} => {}", mapping.destination_path, mapping.source_path))
                .collect::<Vec<_>>()
                .join("\n");
            config.insert(
                "path_mappings".to_string(),
                serde_json::Value::String(rendered),
            );
        }
        Ok(serde_json::Value::Object(config).to_string())
    }

    fn validate_notification_provider_required_config(
        &self,
        provider_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        let config = serde_json::from_str::<serde_json::Value>(config_json).map_err(|error| {
            AppError::Validation(format!(
                "notification configuration is not valid JSON: {error}"
            ))
        })?;
        let config = config.as_object().ok_or_else(|| {
            AppError::Validation("notification configuration must be a JSON object".into())
        })?;
        for field in self.notification_provider_config_fields(provider_type) {
            if !field.required {
                continue;
            }
            let present = config
                .get(&field.key)
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty());
            if !present {
                return Err(AppError::Validation(format!(
                    "notification provider '{provider_type}' requires '{}'",
                    field.label
                )));
            }
        }
        Ok(())
    }

    pub fn notification_channels_repo(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationChannelRepository>> {
        self.services
            .notifications
            .notification_channels()
            .ok_or_else(|| {
                AppError::Repository("notification channel repository is not configured".into())
            })
    }

    pub fn notification_subscriptions_repo(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationSubscriptionRepository>> {
        self.services
            .notifications
            .notification_subscriptions()
            .ok_or_else(|| {
                AppError::Repository(
                    "notification subscription repository is not configured".into(),
                )
            })
    }

    // Helper to get notification channel repository
    fn notification_channels(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationChannelRepository>> {
        self.notification_channels_repo()
    }

    // Helper to get notification subscription repository
    fn notification_subscriptions(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationSubscriptionRepository>> {
        self.notification_subscriptions_repo()
    }
}

fn normalize_notification_media_server_connection_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_notification_target(
    channel_id: Option<&str>,
    target_kind: Option<String>,
    target_id: Option<String>,
) -> AppResult<(NotificationTargetKind, String)> {
    let target_kind = target_kind
        .as_deref()
        .map(|value| {
            NotificationTargetKind::parse(value).ok_or_else(|| {
                AppError::Validation(format!("invalid notification target kind: {value}"))
            })
        })
        .transpose()?;

    let target_id = target_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let channel_id = channel_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    match (target_kind, target_id, channel_id) {
        (Some(kind), Some(id), _) => Ok((kind, id)),
        (None, None, Some(id)) => Ok((NotificationTargetKind::PluginChannel, id)),
        (Some(NotificationTargetKind::PluginChannel), None, Some(id)) => {
            Ok((NotificationTargetKind::PluginChannel, id))
        }
        (Some(kind), None, _) => Err(AppError::Validation(format!(
            "notification target id is required for target kind '{}'",
            kind.as_str()
        ))),
        (None, Some(_), _) => Err(AppError::Validation(
            "notification target kind is required when target id is provided".into(),
        )),
        (None, None, None) => Err(AppError::Validation(
            "notification subscription target is required".into(),
        )),
    }
}
