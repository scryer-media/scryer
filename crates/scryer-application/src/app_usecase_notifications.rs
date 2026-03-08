use chrono::Utc;
use scryer_domain::{
    Id, NotificationChannelConfig, NotificationEventType, NotificationSubscription,
};
use std::collections::HashMap;
use tracing::{info, warn};

use crate::{AppError, AppResult, AppUseCase};

impl AppUseCase {
    pub async fn list_notification_channels(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationChannelConfig>> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;
        repo.list_channels().await
    }

    pub async fn get_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<Option<NotificationChannelConfig>> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
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
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        if name.trim().is_empty() {
            return Err(AppError::Validation("channel name must not be empty".into()));
        }
        if channel_type.trim().is_empty() {
            return Err(AppError::Validation("channel_type must not be empty".into()));
        }

        let now = Utc::now();
        let config = NotificationChannelConfig {
            id: Id::new().0,
            name,
            channel_type,
            config_json,
            is_enabled,
            created_at: now,
            updated_at: now,
        };

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
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;

        let mut channel = repo
            .get_channel(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {id}")))?;

        if let Some(n) = name {
            channel.name = n;
        }
        if let Some(c) = config_json {
            channel.config_json = c;
        }
        if let Some(e) = is_enabled {
            channel.is_enabled = e;
        }
        channel.updated_at = Utc::now();

        repo.update_channel(channel).await
    }

    pub async fn delete_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;
        repo.delete_channel(id).await
    }

    pub async fn test_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        let repo = self.notification_channels()?;
        let channel = repo
            .get_channel(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {id}")))?;

        let provider = self.services.notification_provider.as_ref().ok_or_else(|| {
            AppError::Repository("notification plugin provider is not configured".into())
        })?;

        let client = provider
            .client_for_channel(&channel)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "no notification plugin for channel type '{}'",
                    channel.channel_type
                ))
            })?;

        let metadata = HashMap::new();
        client
            .send_notification(
                NotificationEventType::Test.as_str(),
                "Scryer Test Notification",
                "This is a test notification from Scryer.",
                &metadata,
            )
            .await
    }

    pub async fn list_notification_subscriptions(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationSubscription>> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
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
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        if NotificationEventType::from_str(&event_type).is_none() {
            return Err(AppError::Validation(format!(
                "unknown notification event type: {event_type}"
            )));
        }

        // Validate channel exists
        let ch_repo = self.notification_channels()?;
        ch_repo
            .get_channel(&channel_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {channel_id}")))?;

        let now = Utc::now();
        let sub = NotificationSubscription {
            id: Id::new().0,
            channel_id,
            event_type,
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
        scope_id: Option<Option<String>>,
        is_enabled: Option<bool>,
    ) -> AppResult<NotificationSubscription> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_subscriptions()?;

        // Find all subscriptions and locate ours
        let all = repo.list_subscriptions().await?;
        let mut sub = all
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::NotFound(format!("notification subscription {id}")))?;

        if let Some(et) = event_type {
            if NotificationEventType::from_str(&et).is_none() {
                return Err(AppError::Validation(format!(
                    "unknown notification event type: {et}"
                )));
            }
            sub.event_type = et;
        }
        if let Some(s) = scope {
            sub.scope = s;
        }
        if let Some(si) = scope_id {
            sub.scope_id = si;
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
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_subscriptions()?;
        repo.delete_subscription(id).await
    }

    /// Dispatch a notification for a given event type. Finds matching
    /// subscriptions, resolves channels, and sends through plugins.
    pub async fn dispatch_notification(
        &self,
        event_type: &str,
        title: &str,
        message: &str,
        metadata: &HashMap<String, serde_json::Value>,
    ) {
        let sub_repo = match self.notification_subscriptions() {
            Ok(r) => r,
            Err(_) => return, // notifications not configured
        };
        let ch_repo = match self.notification_channels() {
            Ok(r) => r,
            Err(_) => return,
        };
        let provider = match self.services.notification_provider.as_ref() {
            Some(p) => p,
            None => return,
        };

        let subscriptions = match sub_repo.list_subscriptions_for_event(event_type).await {
            Ok(subs) => subs,
            Err(e) => {
                warn!(error = %e, event_type, "failed to list notification subscriptions");
                return;
            }
        };

        for sub in subscriptions {
            if !sub.is_enabled {
                continue;
            }

            let channel = match ch_repo.get_channel(&sub.channel_id).await {
                Ok(Some(ch)) if ch.is_enabled => ch,
                _ => continue,
            };

            let client = match provider.client_for_channel(&channel) {
                Some(c) => c,
                None => {
                    warn!(
                        channel_type = channel.channel_type.as_str(),
                        channel_name = channel.name.as_str(),
                        "no notification plugin available for channel type"
                    );
                    continue;
                }
            };

            match client
                .send_notification(event_type, title, message, metadata)
                .await
            {
                Ok(()) => {
                    info!(
                        event_type,
                        channel = channel.name.as_str(),
                        "notification dispatched"
                    );
                }
                Err(e) => {
                    warn!(
                        event_type,
                        channel = channel.name.as_str(),
                        error = %e,
                        "notification dispatch failed"
                    );
                }
            }
        }
    }

    pub fn available_notification_provider_types(&self) -> Vec<String> {
        self.services
            .notification_provider
            .as_ref()
            .map(|p| p.available_provider_types())
            .unwrap_or_default()
    }

    pub fn notification_provider_config_fields(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.services
            .notification_provider
            .as_ref()
            .map(|p| p.config_fields_for_provider(provider_type))
            .unwrap_or_default()
    }

    pub fn notification_provider_name(
        &self,
        provider_type: &str,
    ) -> Option<String> {
        self.services
            .notification_provider
            .as_ref()
            .and_then(|p| p.plugin_name_for_provider(provider_type))
    }

    // Helper to get notification channel repository
    fn notification_channels(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationChannelRepository>> {
        self.services.notification_channels.as_ref().ok_or_else(|| {
            AppError::Repository("notification channel repository is not configured".into())
        })
    }

    // Helper to get notification subscription repository
    fn notification_subscriptions(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationSubscriptionRepository>> {
        self.services
            .notification_subscriptions
            .as_ref()
            .ok_or_else(|| {
                AppError::Repository(
                    "notification subscription repository is not configured".into(),
                )
            })
    }
}
