use crate::{AppError, AppResult, RateLimitCooldownAction};
use chrono::{DateTime, Utc};
use scryer_domain::IndexerConfig;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, Weak};

const CAPS_TTL: chrono::Duration = chrono::Duration::days(7);
const FETCHED_AT: &str = "cache_fetched_at";

pub(crate) fn connection_identity(config: &IndexerConfig) -> serde_json::Value {
    serde_json::json!({
        "provider": config.provider_type.trim().to_ascii_lowercase(),
        "endpoint": config.base_url.trim().trim_end_matches('/'),
        "config": config.config_json.as_deref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
        "secret": config.api_key_encrypted,
        "proxy": config.proxy_config_id,
    })
}

pub(crate) fn fresh_snapshot(config: &IndexerConfig, now: DateTime<Utc>) -> Option<String> {
    let raw = config.caps_snapshot_json.as_ref()?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let fetched = DateTime::parse_from_rfc3339(value.get(FETCHED_AT)?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    if fetched > now || now.signed_duration_since(fetched) >= CAPS_TTL {
        return None;
    }
    serde_json::from_value::<scryer_domain::IndexerCapsSnapshot>(value).ok()?;
    Some(raw.clone())
}

pub(crate) fn serialize_snapshot(
    snapshot: &scryer_domain::IndexerCapsSnapshot,
    now: DateTime<Utc>,
) -> AppResult<String> {
    let mut value =
        serde_json::to_value(snapshot).map_err(|error| AppError::Repository(error.to_string()))?;
    value[FETCHED_AT] = serde_json::Value::String(now.to_rfc3339());
    serde_json::to_string(&value).map_err(|error| AppError::Repository(error.to_string()))
}

#[derive(Clone)]
enum FetchFailure {
    Temporary(String, Option<std::time::Duration>, RateLimitCooldownAction),
    Quota(u16, String),
    Validation(String),
    Canceled(String),
    Plugin(String, crate::PluginFailure),
    Repository(String),
}

impl FetchFailure {
    fn from_error(error: AppError) -> Self {
        match error {
            AppError::TemporaryUnavailable {
                message,
                retry_after,
                rate_limit_cooldown,
            } => Self::Temporary(message, retry_after, rate_limit_cooldown),
            AppError::NewznabQuotaExceeded { code, message } => Self::Quota(code, message),
            AppError::Validation(message) => Self::Validation(message),
            AppError::Canceled(message) => Self::Canceled(message),
            AppError::PluginFailed { message, failure } => Self::Plugin(message, failure),
            AppError::Repository(message) => Self::Repository(message),
            other => Self::Repository(other.to_string()),
        }
    }

    fn into_error(self) -> AppError {
        match self {
            Self::Temporary(message, retry_after, rate_limit_cooldown) => {
                AppError::TemporaryUnavailable {
                    message,
                    retry_after,
                    rate_limit_cooldown,
                }
            }
            Self::Quota(code, message) => AppError::NewznabQuotaExceeded { code, message },
            Self::Validation(message) => AppError::Validation(message),
            Self::Canceled(message) => AppError::Canceled(message),
            Self::Plugin(message, failure) => AppError::PluginFailed { message, failure },
            Self::Repository(message) => AppError::Repository(message),
        }
    }
}

type SharedFetch = tokio::sync::Mutex<Option<Result<Option<String>, FetchFailure>>>;

/// Only overlapping callers share a result. Durable snapshots own the seven-day cache.
#[derive(Default)]
pub(crate) struct CapsRequestCache {
    flights: Mutex<HashMap<String, Weak<SharedFetch>>>,
}

impl CapsRequestCache {
    pub(crate) async fn fetch(
        &self,
        config: &IndexerConfig,
        request: impl Future<Output = AppResult<Option<String>>>,
    ) -> AppResult<Option<String>> {
        let identity = format!("{}:{}", config.id, connection_identity(config));
        let key = crate::helpers::blake3_identity_hex(
            crate::helpers::HashDomain::IndexerSecret,
            &identity,
        );
        let flight = {
            let mut flights = self.flights.lock().expect("caps request map lock poisoned");
            flights.retain(|_, flight| flight.strong_count() > 0);
            if let Some(flight) = flights.get(&key).and_then(Weak::upgrade) {
                flight
            } else {
                if flights.len() >= 128 {
                    return Err(AppError::TemporaryUnavailable {
                        message: "Too many concurrent indexer caps refreshes; try again later"
                            .into(),
                        retry_after: None,
                        rate_limit_cooldown: RateLimitCooldownAction::None,
                    });
                }
                let flight = Arc::new(tokio::sync::Mutex::new(None));
                flights.insert(key, Arc::downgrade(&flight));
                flight
            }
        };
        let mut result = flight.lock().await;
        if result.is_none() {
            *result = Some(request.await.map_err(FetchFailure::from_error));
        }
        result
            .as_ref()
            .expect("caps fetch completed")
            .clone()
            .map_err(FetchFailure::into_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Poll;

    fn config() -> IndexerConfig {
        serde_json::from_value(serde_json::json!({
            "id": "cache-test", "name": "Synthetic indexer",
            "provider_type": "newznab", "base_url": "https://indexer.example.test",
            "config_json": "{\"api_key\":\"synthetic\"}",
            "is_enabled": true, "enable_interactive_search": true, "enable_auto_search": true,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn freshness_is_seven_days_from_success_and_survives_serialization() {
        let mut config = config();
        let fetched = config.created_at;
        config.caps_snapshot_json = Some(
            serialize_snapshot(&scryer_domain::IndexerCapsSnapshot::default(), fetched).unwrap(),
        );
        let restored: IndexerConfig =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(fresh_snapshot(&restored, fetched).is_some());
        assert!(
            fresh_snapshot(
                &restored,
                fetched + CAPS_TTL - chrono::Duration::nanoseconds(1)
            )
            .is_some()
        );
        assert!(fresh_snapshot(&restored, fetched + CAPS_TTL).is_none());
        assert!(fresh_snapshot(&restored, fetched - chrono::Duration::nanoseconds(1)).is_none());
        for raw in ["{}", "invalid", r#"{"cache_fetched_at":"invalid"}"#] {
            config.caps_snapshot_json = Some(raw.into());
            assert!(fresh_snapshot(&config, fetched).is_none());
        }
    }

    #[test]
    fn cache_metadata_and_local_preferences_do_not_change_search_or_connection_identity() {
        let original = config();
        let mut updated = original.clone();
        updated.caps_snapshot_json = Some(
            serialize_snapshot(
                &scryer_domain::IndexerCapsSnapshot::default(),
                updated.created_at,
            )
            .unwrap(),
        );
        let mut later = updated.clone();
        later.caps_snapshot_json = Some(
            serialize_snapshot(
                &scryer_domain::IndexerCapsSnapshot::default(),
                updated.created_at + CAPS_TTL,
            )
            .unwrap(),
        );
        assert_eq!(
            crate::indexer_search_identity(&updated, None),
            crate::indexer_search_identity(&later, None)
        );
        updated.name = "Renamed".into();
        updated.is_enabled = false;
        updated.rate_limit_seconds = Some(60);
        updated.download_client_id = Some("client".into());
        assert_eq!(
            connection_identity(&original),
            connection_identity(&updated)
        );
        for change in 0..4 {
            let mut changed = original.clone();
            match change {
                0 => changed.provider_type = "other".into(),
                1 => changed.base_url = "https://other.example.test".into(),
                2 => changed.config_json = Some(r#"{"api_key":"changed"}"#.into()),
                _ => changed.proxy_config_id = Some("proxy".into()),
            }
            assert_ne!(
                connection_identity(&original),
                connection_identity(&changed)
            );
        }
    }

    #[tokio::test]
    async fn overlapping_fetches_share_results_but_changed_connections_do_not() {
        let cache = CapsRequestCache::default();
        let config = config();
        let (release, waiting) = tokio::sync::oneshot::channel::<()>();
        let first = cache.fetch(&config, async {
            waiting.await.unwrap();
            Ok(Some("snapshot".into()))
        });
        let second = cache.fetch(&config, async { panic!("duplicate outbound request") });
        tokio::pin!(first, second);
        assert!(matches!(
            first
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop())),
            Poll::Pending
        ));
        assert!(matches!(
            second
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop())),
            Poll::Pending
        ));
        let mut changed = config.clone();
        changed.proxy_config_id = Some("different-proxy".into());
        assert_eq!(
            cache
                .fetch(&changed, async { Ok(Some("other".into())) })
                .await
                .unwrap()
                .as_deref(),
            Some("other")
        );
        release.send(()).unwrap();
        let (first, second) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(first, second)
        })
        .await
        .unwrap();
        assert_eq!(first.unwrap(), second.unwrap());
    }

    #[tokio::test]
    async fn failed_shared_fetch_preserves_quota_error_and_allows_next_refresh() {
        let cache = CapsRequestCache::default();
        let config = config();
        let (release, waiting) = tokio::sync::oneshot::channel::<()>();
        let first = cache.fetch(&config, async {
            waiting.await.unwrap();
            Err(AppError::NewznabQuotaExceeded {
                code: 500,
                message: "quota".into(),
            })
        });
        let second = cache.fetch(&config, async { panic!("duplicate outbound request") });
        tokio::pin!(first, second);
        assert!(matches!(
            first
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop())),
            Poll::Pending
        ));
        assert!(matches!(
            second
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop())),
            Poll::Pending
        ));
        release.send(()).unwrap();
        let results = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(first, second)
        })
        .await
        .unwrap();
        for result in [results.0, results.1] {
            assert!(matches!(
                result,
                Err(AppError::NewznabQuotaExceeded { code: 500, .. })
            ));
        }
        assert_eq!(
            cache
                .fetch(&config, async { Ok(Some("recovered".into())) })
                .await
                .unwrap()
                .as_deref(),
            Some("recovered")
        );
    }
}
