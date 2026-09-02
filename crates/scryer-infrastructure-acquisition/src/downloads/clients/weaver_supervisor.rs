//! Runtime lifecycle supervisor for the Weaver subscription bridge.
//!
//! Bridge eligibility used to be decided exactly once, at process startup
//! (`resolve_weaver_subscription_bridge_client` in `main.rs`), and the download
//! queue poller's `excluded_client_types` was frozen from that same snapshot.
//! On a fresh install the download-client table is empty at boot, so adding or
//! promoting a Weaver client afterwards never started the bridge: every such
//! deployment silently ran on 1s queue / 30s history interval polling until the
//! next restart. Weaver completes small jobs in well under a second and its
//! `queueItems` facade is active-only, so interval polling routinely misses the
//! entire active window and completions hang on the 30s history poll.
//!
//! The supervisor re-resolves the primary enabled download client on an
//! interval and converges the bridge to match:
//!
//! - Weaver becomes primary → start the bridge, then mark `weaver` as
//!   bridge-covered so the generic poller stands down for it.
//! - Weaver stops being primary (removed, disabled, outprioritized) → un-cover
//!   first so the poller resumes, then stop the bridge.
//! - The primary Weaver's config changes (URL, credentials, name, mappings) →
//!   restart the bridge against the new config.
//!
//! Ordering is deliberate in both directions: coverage flips only while a
//! bridge is running, so any error window degrades to double observation
//! (harmless upserts — the tracked runtime is idempotent and the poller's
//! prune won't run against a covered client) rather than to a gap where no one
//! is watching Weaver at all.

use std::time::Duration;

use scryer_application::{
    AppUseCase,
    tracked_downloads::{BridgedClientTypesHandle, TrackedDownloadSnapshotIngestHandle},
};
use scryer_domain::DownloadClientConfig;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::weaver_subscription::{
    WeaverSubscriptionBridgeClient, start_weaver_subscription_bridge,
};

/// How often the supervisor re-resolves the primary download client.
///
/// The router already re-reads client configs from the datastore on every
/// feedback poll, so config changes reach polling paths within a tick; this
/// interval only bounds how long a NEW or RE-CONFIGURED Weaver primary waits
/// for its realtime bridge. Override via
/// `SCRYER_WEAVER_BRIDGE_SUPERVISOR_INTERVAL_SECS`; the floor is 1s.
const SUPERVISOR_INTERVAL_SECS: u64 = 15;
const SUPERVISOR_INTERVAL_ENV: &str = "SCRYER_WEAVER_BRIDGE_SUPERVISOR_INTERVAL_SECS";

const WEAVER_CLIENT_TYPE: &str = "weaver";

fn supervisor_interval() -> Duration {
    let secs = std::env::var(SUPERVISOR_INTERVAL_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|secs| *secs >= 1)
        .unwrap_or(SUPERVISOR_INTERVAL_SECS);
    Duration::from_secs(secs)
}

/// Identity of a bridge-worthy client config. Two equal fingerprints mean the
/// running bridge is already correct; any difference forces a restart.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BridgeFingerprint {
    client_id: String,
    client_name: String,
    config_json: String,
    /// `id@updated_at` of the assigned proxy, so editing or reassigning the
    /// proxy restarts the bridge the same way editing the client does.
    proxy_revision: Option<String>,
}

impl BridgeFingerprint {
    fn from_config(
        config: &DownloadClientConfig,
        proxy_config: Option<&scryer_domain::ProxyConfig>,
    ) -> Self {
        Self {
            client_id: config.id.clone(),
            client_name: config.name.clone(),
            config_json: config.config_json.clone(),
            proxy_revision: proxy_config
                .map(scryer_application::transport_proxy::transport_proxy_revision),
        }
    }
}

/// What the supervisor should do to converge the running bridge (if any)
/// toward the desired one (if any).
#[derive(Clone, Debug, PartialEq, Eq)]
enum BridgeAction {
    None,
    Start,
    Stop,
    Restart,
}

fn evaluate_bridge_action(
    running: Option<&BridgeFingerprint>,
    desired: Option<&BridgeFingerprint>,
) -> BridgeAction {
    match (running, desired) {
        (None, None) => BridgeAction::None,
        (None, Some(_)) => BridgeAction::Start,
        (Some(_), None) => BridgeAction::Stop,
        (Some(current), Some(target)) => {
            if current == target {
                BridgeAction::None
            } else {
                BridgeAction::Restart
            }
        }
    }
}

/// The bridge client and fingerprint for a primary config, when it is a
/// bridgeable Weaver.
fn desired_bridge_from_primary(
    primary: Option<&DownloadClientConfig>,
    proxy_config: Option<&scryer_domain::ProxyConfig>,
) -> Option<(WeaverSubscriptionBridgeClient, BridgeFingerprint)> {
    let config = primary?;
    if config.client_type != WEAVER_CLIENT_TYPE {
        return None;
    }
    match WeaverSubscriptionBridgeClient::from_config_with_proxy(config, proxy_config) {
        Ok(client) => Some((client, BridgeFingerprint::from_config(config, proxy_config))),
        Err(error) => {
            // A malformed weaver primary polls like any other client rather
            // than knocking out download tracking entirely.
            warn!(
                client_id = %config.id,
                client = %config.name,
                error = %error,
                "primary weaver client is not bridgeable; leaving generic polling in place"
            );
            None
        }
    }
}

struct RunningBridge {
    fingerprint: BridgeFingerprint,
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

/// Supervise the Weaver subscription bridge for the life of the process.
///
/// Owns the bridge task and the poller's bridge-coverage handle; see the module
/// docs for the convergence and ordering rules.
pub async fn start_weaver_bridge_supervisor(
    token: CancellationToken,
    app: AppUseCase,
    ingest: TrackedDownloadSnapshotIngestHandle,
    bridged_client_types: BridgedClientTypesHandle,
) {
    let mut interval = tokio::time::interval(supervisor_interval());
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut running: Option<RunningBridge> = None;

    info!(
        interval_secs = supervisor_interval().as_secs(),
        "weaver bridge supervisor started"
    );

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("weaver bridge supervisor shutting down");
                bridged_client_types.clear();
                if let Some(bridge) = running.take() {
                    bridge.cancel.cancel();
                }
                return;
            }
            _ = interval.tick() => {}
        }

        // A bridge that died on its own (it only exits via cancellation, so
        // this is strictly defensive) must not keep Weaver excluded from
        // polling: resume generic coverage and let the normal convergence
        // below decide whether to start a replacement.
        if running
            .as_ref()
            .is_some_and(|bridge| bridge.join.is_finished())
        {
            warn!("weaver subscription bridge task exited unexpectedly; resuming generic polling");
            bridged_client_types.clear();
            running = None;
        }

        let primary = match app.primary_enabled_download_client_config().await {
            Ok(primary) => primary,
            Err(error) => {
                // Leave the current state alone: a transient config-read
                // failure is not evidence the primary changed.
                warn!(error = %error, "weaver bridge supervisor failed to resolve primary download client");
                continue;
            }
        };
        // A proxy the primary references but that cannot be resolved is not a
        // reason to bridge unproxied: leave the current state alone and let the
        // router's own (proxied) polling cover the client.
        let proxy_config = match primary.as_ref() {
            Some(config) => match app.proxy_config_for_download_client(config).await {
                Ok(proxy_config) => proxy_config,
                Err(error) => {
                    warn!(
                        client_id = %config.id,
                        client = %config.name,
                        error = %error,
                        "weaver bridge supervisor could not resolve the primary client's proxy"
                    );
                    continue;
                }
            },
            None => None,
        };
        let desired = desired_bridge_from_primary(primary.as_ref(), proxy_config.as_ref());

        let action = evaluate_bridge_action(
            running.as_ref().map(|bridge| &bridge.fingerprint),
            desired.as_ref().map(|(_, fingerprint)| fingerprint),
        );

        match action {
            BridgeAction::None => {}
            BridgeAction::Start => {
                let Some((client, fingerprint)) = desired else {
                    continue;
                };
                running = Some(spawn_bridge(&token, client, fingerprint, &ingest));
                // Cover only after the bridge task exists: the error window is
                // double observation, never a coverage gap.
                bridged_client_types.set(vec![WEAVER_CLIENT_TYPE.to_string()]);
            }
            BridgeAction::Stop => {
                // Resume polling before stopping the bridge, for the same
                // reason: overlap over gap.
                bridged_client_types.clear();
                if let Some(bridge) = running.take() {
                    info!(
                        client_id = %bridge.fingerprint.client_id,
                        "weaver is no longer the primary download client; stopping subscription bridge"
                    );
                    bridge.cancel.cancel();
                }
            }
            BridgeAction::Restart => {
                let Some((client, fingerprint)) = desired else {
                    continue;
                };
                if let Some(bridge) = running.take() {
                    info!(
                        client_id = %bridge.fingerprint.client_id,
                        "primary weaver config changed; restarting subscription bridge"
                    );
                    bridge.cancel.cancel();
                }
                running = Some(spawn_bridge(&token, client, fingerprint, &ingest));
                bridged_client_types.set(vec![WEAVER_CLIENT_TYPE.to_string()]);
            }
        }
    }
}

fn spawn_bridge(
    supervisor_token: &CancellationToken,
    client: WeaverSubscriptionBridgeClient,
    fingerprint: BridgeFingerprint,
    ingest: &TrackedDownloadSnapshotIngestHandle,
) -> RunningBridge {
    info!(
        client_id = %fingerprint.client_id,
        client = %fingerprint.client_name,
        "weaver is the primary download client; starting subscription bridge"
    );
    let cancel = supervisor_token.child_token();
    let join = tokio::spawn(start_weaver_subscription_bridge(
        cancel.clone(),
        client,
        ingest.clone(),
    ));
    RunningBridge {
        fingerprint,
        cancel,
        join,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(id: &str, name: &str, client_type: &str, config_json: &str) -> DownloadClientConfig {
        let now = chrono::Utc::now();
        DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: config_json.to_string(),
            client_priority: 0,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
            proxy_config_id: None,
        }
    }

    fn weaver_config_json() -> &'static str {
        r#"{"host":"weaver","port":9090,"use_ssl":false,"api_key":"k"}"#
    }

    #[test]
    fn no_bridge_is_desired_without_a_primary() {
        assert!(desired_bridge_from_primary(None, None).is_none());
    }

    #[test]
    fn no_bridge_is_desired_for_a_non_weaver_primary() {
        let sab = config("dc-1", "SAB", "sabnzbd", weaver_config_json());
        assert!(desired_bridge_from_primary(Some(&sab), None).is_none());
    }

    #[test]
    fn a_weaver_primary_yields_a_bridge_and_fingerprint() {
        let weaver = config("dc-1", "Weaver", "weaver", weaver_config_json());
        let (_, fingerprint) =
            desired_bridge_from_primary(Some(&weaver), None).expect("bridgeable weaver primary");
        assert_eq!(fingerprint, BridgeFingerprint::from_config(&weaver, None));
    }

    #[test]
    fn an_unbridgeable_weaver_primary_falls_back_to_polling() {
        // No resolvable base URL → from_config fails → no bridge, no panic.
        let broken = config("dc-1", "Weaver", "weaver", "{}");
        assert!(desired_bridge_from_primary(Some(&broken), None).is_none());
    }

    #[test]
    fn convergence_actions_cover_every_transition() {
        let a = BridgeFingerprint {
            client_id: "dc-1".into(),
            client_name: "Weaver".into(),
            config_json: weaver_config_json().into(),
            proxy_revision: None,
        };
        let mut b = a.clone();
        b.config_json = r#"{"host":"weaver-2","port":9090}"#.into();

        assert_eq!(evaluate_bridge_action(None, None), BridgeAction::None);
        assert_eq!(evaluate_bridge_action(None, Some(&a)), BridgeAction::Start);
        assert_eq!(evaluate_bridge_action(Some(&a), None), BridgeAction::Stop);
        assert_eq!(
            evaluate_bridge_action(Some(&a), Some(&a)),
            BridgeAction::None
        );
        assert_eq!(
            evaluate_bridge_action(Some(&a), Some(&b)),
            BridgeAction::Restart
        );
    }

    #[test]
    fn renaming_the_client_restarts_the_bridge() {
        // The bridge stamps client_name onto every queue item it publishes, so
        // a rename must rebuild it.
        let before = BridgeFingerprint {
            client_id: "dc-1".into(),
            client_name: "Weaver".into(),
            config_json: weaver_config_json().into(),
            proxy_revision: None,
        };
        let mut after = before.clone();
        after.client_name = "Weaver Prod".into();
        assert_eq!(
            evaluate_bridge_action(Some(&before), Some(&after)),
            BridgeAction::Restart
        );
    }

    #[test]
    fn assigning_or_editing_a_proxy_restarts_the_bridge() {
        // The bridge's polling fallback dials through the assigned proxy, so a
        // proxy change is a client change as far as the bridge is concerned.
        let unproxied = BridgeFingerprint {
            client_id: "dc-1".into(),
            client_name: "Weaver".into(),
            config_json: weaver_config_json().into(),
            proxy_revision: None,
        };
        let mut proxied = unproxied.clone();
        proxied.proxy_revision = Some("proxy-1@2026-09-01T00:00:00+00:00".to_string());
        assert_eq!(
            evaluate_bridge_action(Some(&unproxied), Some(&proxied)),
            BridgeAction::Restart
        );
        let mut edited = proxied.clone();
        edited.proxy_revision = Some("proxy-1@2026-09-02T00:00:00+00:00".to_string());
        assert_eq!(
            evaluate_bridge_action(Some(&proxied), Some(&edited)),
            BridgeAction::Restart
        );
        assert_eq!(
            evaluate_bridge_action(Some(&proxied), Some(&proxied)),
            BridgeAction::None
        );
    }
}
