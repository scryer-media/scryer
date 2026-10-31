use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, NotificationChannelRepository, NotificationSubscriptionRepository,
};
use scryer_domain::{
    ChannelType, NotificationChannelConfig, NotificationEventType, NotificationSubscription,
    NotificationTargetKind,
};

use crate::config_store::{current_encryption_key, decrypt_value, maybe_encrypt_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const CHANNEL_COLUMNS: &str = "id, name, channel_type, config_json, media_server_connection_id, is_enabled, created_at, updated_at";

const CHANNEL_INSERT_SQL: &str = "INSERT INTO notification_channels (
    id, name, channel_type, config_json, media_server_connection_id, is_enabled, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}
)";

const SUBSCRIPTION_COLUMNS: &str = "id, channel_id, target_kind, target_id, event_type, scope, scope_id, is_enabled, created_at, updated_at";

const SUBSCRIPTION_INSERT_SQL: &str = "INSERT INTO notification_subscriptions (
    id, channel_id, target_kind, target_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

#[derive(Clone)]
pub struct NotificationStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl NotificationStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl NotificationChannelRepository for NotificationStore {
    async fn list_channels(&self) -> AppResult<Vec<NotificationChannelConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_channels(
            self.datastore.read_exec(),
            &format!(
                "SELECT {CHANNEL_COLUMNS} FROM notification_channels ORDER BY created_at DESC"
            ),
            &[],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_channel(&self, id: &str) -> AppResult<Option<NotificationChannelConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_channel(
            self.datastore.read_exec(),
            &format!("SELECT {CHANNEL_COLUMNS} FROM notification_channels WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        let encryption_key = self.encryption_key()?;
        let args = channel_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(&self.datastore, "create_notification_channel", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), CHANNEL_INSERT_SQL, &args).await?;
                Ok(config)
            })
        })
        .await
    }

    async fn update_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        let encryption_key = self.encryption_key()?;
        let stored_config = channel_config_arg(encryption_key.as_ref(), &config.config_json)?;
        let updated_at = Utc::now();
        let args = vec![
            SqlArg::Text(config.name.clone()),
            stored_config,
            SqlArg::OptText(config.media_server_connection_id.clone()),
            SqlArg::Bool(config.is_enabled),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(config.id.clone()),
        ];
        let id = config.id.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "update_notification_channel", move |tx| {
            let args = args.clone();
            let id = id.clone();
            let encryption_key = encryption_key.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE notification_channels
                     SET name = {}, config_json = {}, media_server_connection_id = {}, is_enabled = {}, updated_at = {}
                     WHERE id = {}",
                    &args,
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::not_found_reason(crate::error_reasons::NOTIFICATION_CHANNEL_NOT_FOUND, format!("notification channel {id}")));
                }
                fetch_optional_channel(
                    SqlExec::Tx(tx),
                    &format!("SELECT {CHANNEL_COLUMNS} FROM notification_channels WHERE id = {{}}"),
                    &[SqlArg::Text(id.clone())],
                    encryption_key.as_ref(),
                )
                .await?
                .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::NOTIFICATION_CHANNEL_NOT_FOUND, format!("notification channel {id}")))
            })
        })
        .await
    }

    async fn delete_channel(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_notification_channel", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM notification_channels WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::not_found_reason(
                        crate::error_reasons::NOTIFICATION_CHANNEL_NOT_FOUND,
                        format!("notification channel {id}"),
                    ));
                }
                Ok(())
            })
        })
        .await
    }
}

#[async_trait]
impl NotificationSubscriptionRepository for NotificationStore {
    async fn list_subscriptions(&self) -> AppResult<Vec<NotificationSubscription>> {
        fetch_subscriptions(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBSCRIPTION_COLUMNS} FROM notification_subscriptions ORDER BY created_at DESC"
            ),
            &[],
        )
        .await
    }

    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<NotificationSubscription>> {
        fetch_subscriptions(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBSCRIPTION_COLUMNS} FROM notification_subscriptions WHERE channel_id = {{}} ORDER BY created_at DESC"
            ),
            &[SqlArg::Text(channel_id.to_string())],
        )
        .await
    }

    async fn list_subscriptions_for_target(
        &self,
        target_kind: NotificationTargetKind,
        target_id: &str,
    ) -> AppResult<Vec<NotificationSubscription>> {
        fetch_subscriptions(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBSCRIPTION_COLUMNS} FROM notification_subscriptions WHERE target_kind = {{}} AND target_id = {{}} ORDER BY created_at DESC"
            ),
            &[
                SqlArg::Text(target_kind.as_str().to_string()),
                SqlArg::Text(target_id.to_string()),
            ],
        )
        .await
    }

    async fn list_subscriptions_for_event(
        &self,
        event_type: NotificationEventType,
    ) -> AppResult<Vec<NotificationSubscription>> {
        fetch_subscriptions(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SUBSCRIPTION_COLUMNS} FROM notification_subscriptions WHERE event_type = {{}} ORDER BY created_at DESC"
            ),
            &[SqlArg::Text(event_type.as_str().to_string())],
        )
        .await
    }

    async fn create_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        let args = subscription_insert_args(&sub);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_notification_subscription",
            move |tx| {
                let sub = sub.clone();
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), SUBSCRIPTION_INSERT_SQL, &args).await?;
                    Ok(sub)
                })
            },
        )
        .await
    }

    async fn update_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        let updated_at = Utc::now();
        let args = vec![
            SqlArg::OptText(sub.channel_id.clone()),
            SqlArg::Text(sub.target_kind.as_str().to_string()),
            SqlArg::Text(sub.target_id.clone()),
            SqlArg::Text(sub.event_type.as_str().to_string()),
            SqlArg::Text(sub.scope.clone()),
            SqlArg::OptText(sub.scope_id.clone()),
            SqlArg::Bool(sub.is_enabled),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(sub.id.clone()),
        ];
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_notification_subscription",
            move |tx| {
                let sub = sub.clone();
                let args = args.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE notification_subscriptions
                         SET channel_id = {}, target_kind = {}, target_id = {}, event_type = {}, scope = {}, scope_id = {}, is_enabled = {}, updated_at = {}
                         WHERE id = {}",
                        &args,
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::not_found_reason(crate::error_reasons::NOTIFICATION_SUBSCRIPTION_NOT_FOUND, format!(
                            "notification subscription {}",
                            sub.id
                        )));
                    }
                    Ok(sub)
                })
            },
        )
        .await
    }

    async fn delete_subscription(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_notification_subscription",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM notification_subscriptions WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::not_found_reason(
                            crate::error_reasons::NOTIFICATION_SUBSCRIPTION_NOT_FOUND,
                            format!("notification subscription {id}"),
                        ));
                    }
                    Ok(())
                })
            },
        )
        .await
    }
}

fn channel_insert_args(
    config: &NotificationChannelConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.channel_type.as_str().to_string()),
        channel_config_arg(encryption_key, &config.config_json)?,
        SqlArg::OptText(config.media_server_connection_id.clone()),
        SqlArg::Bool(config.is_enabled),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ])
}

fn channel_config_arg(
    encryption_key: Option<&EncryptionKey>,
    config_json: &str,
) -> AppResult<SqlArg> {
    let stored_config = maybe_encrypt_value(encryption_key, config_json)?;
    Ok(SqlArg::Text(stored_config))
}

fn subscription_insert_args(sub: &NotificationSubscription) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(sub.id.clone()),
        SqlArg::OptText(sub.channel_id.clone()),
        SqlArg::Text(sub.target_kind.as_str().to_string()),
        SqlArg::Text(sub.target_id.clone()),
        SqlArg::Text(sub.event_type.as_str().to_string()),
        SqlArg::Text(sub.scope.clone()),
        SqlArg::OptText(sub.scope_id.clone()),
        SqlArg::Bool(sub.is_enabled),
        SqlArg::Timestamp(sub.created_at),
        SqlArg::Timestamp(sub.updated_at),
    ]
}

async fn fetch_channels(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<NotificationChannelConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_channel(&row, encryption_key))
        .collect()
}

async fn fetch_optional_channel(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<NotificationChannelConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_channel(&row, encryption_key))
        .transpose()
}

fn row_to_channel(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<NotificationChannelConfig> {
    let channel_type_raw = row.text("channel_type")?;
    let channel_type = ChannelType::parse(&channel_type_raw)
        .ok_or_else(|| AppError::Repository(format!("invalid channel_type: {channel_type_raw}")))?;
    Ok(NotificationChannelConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        channel_type,
        config_json: channel_config_json_from_row(row, encryption_key)?,
        media_server_connection_id: row.opt_text("media_server_connection_id")?,
        is_enabled: row.bool("is_enabled")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn channel_config_json_from_row(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<String> {
    let raw = row.text("config_json")?;
    decrypt_value(encryption_key, raw, "config_json", false)
}

async fn fetch_subscriptions(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<NotificationSubscription>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_subscription(&row))
        .collect()
}

fn row_to_subscription(row: &SqlRow) -> AppResult<NotificationSubscription> {
    let event_type_raw = row.text("event_type")?;
    let event_type = NotificationEventType::parse(&event_type_raw)
        .ok_or_else(|| AppError::Repository(format!("unknown event_type: {event_type_raw}")))?;
    let target_kind_raw = row.text("target_kind")?;
    let target_kind = NotificationTargetKind::parse(&target_kind_raw)
        .ok_or_else(|| AppError::Repository(format!("unknown target_kind: {target_kind_raw}")))?;
    Ok(NotificationSubscription {
        id: row.text("id")?,
        channel_id: row.opt_text("channel_id")?,
        target_kind,
        target_id: row.text("target_id")?,
        event_type,
        scope: row.text("scope")?,
        scope_id: row.opt_text("scope_id")?,
        is_enabled: row.bool("is_enabled")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_infrastructure_sql::runtime::repo_err;
    use sqlx::postgres::PgPoolOptions;

    #[tokio::test]
    async fn postgres_notification_store_smoke_from_env() -> AppResult<()> {
        let Some(database_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL notification store smoke; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .map_err(repo_err)?;
        sqlx::query(
            "CREATE TEMP TABLE notification_channels (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                channel_type TEXT NOT NULL,
                config_json TEXT NOT NULL,
                media_server_connection_id TEXT,
                is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            ) ON COMMIT PRESERVE ROWS",
        )
        .execute(&pool)
        .await
        .map_err(repo_err)?;
        sqlx::query(
            "CREATE TEMP TABLE notification_subscriptions (
                id TEXT PRIMARY KEY,
                channel_id TEXT REFERENCES notification_channels(id) ON DELETE CASCADE,
                target_kind TEXT NOT NULL,
                target_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                scope TEXT NOT NULL,
                scope_id TEXT,
                is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            ) ON COMMIT PRESERVE ROWS",
        )
        .execute(&pool)
        .await
        .map_err(repo_err)?;

        let store = NotificationStore::new(
            StoreDatastore::Postgres { pool: pool.clone() },
            Arc::new(RwLock::new(None)),
        );
        let now = Utc::now();
        let channel = NotificationChannelConfig {
            id: "pg-channel-1".to_string(),
            name: "Postgres Webhook".to_string(),
            channel_type: ChannelType::parse("webhook").expect("channel type"),
            config_json: r#"{"url":"https://example.com/webhook"}"#.to_string(),
            media_server_connection_id: None,
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };

        NotificationChannelRepository::create_channel(&store, channel.clone()).await?;
        let fetched = NotificationChannelRepository::get_channel(&store, &channel.id)
            .await?
            .expect("channel should exist");
        assert_eq!(fetched.config_json, channel.config_json);

        let subscription = NotificationSubscription {
            id: "pg-subscription-1".to_string(),
            channel_id: Some(channel.id.clone()),
            target_kind: NotificationTargetKind::PluginChannel,
            target_id: channel.id.clone(),
            event_type: NotificationEventType::ImportComplete,
            scope: "global".to_string(),
            scope_id: None,
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        NotificationSubscriptionRepository::create_subscription(&store, subscription.clone())
            .await?;
        let updated_subscription = NotificationSubscription {
            is_enabled: false,
            updated_at: Utc::now(),
            ..subscription.clone()
        };
        NotificationSubscriptionRepository::update_subscription(&store, updated_subscription)
            .await?;
        let by_event = NotificationSubscriptionRepository::list_subscriptions_for_event(
            &store,
            NotificationEventType::ImportComplete,
        )
        .await?;
        assert_eq!(by_event.len(), 1);
        assert!(!by_event[0].is_enabled);

        NotificationSubscriptionRepository::delete_subscription(&store, &subscription.id).await?;
        NotificationChannelRepository::delete_channel(&store, &channel.id).await?;
        pool.close().await;
        Ok(())
    }
}
