use chrono::Utc;
use scryer_domain::{CompletedDownload, download_identity::DownloadId};

use crate::{
    AppUseCase, ClientJobLocator, DownloadSourceKind, DownloadSubmission,
    DownloadSubmissionIdentity, ObservationResolution, ObservedClientJob, SubmissionScope,
    extract_magnet_info_hash,
};

pub const DOWNLOAD_ID_PARAMETER: &str = "*scryer_download_id";

pub struct AcceptedDownloadIdentityInput<'a> {
    pub initial_download_id: Option<&'a str>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_hint: Option<&'a str>,
    pub info_hash_hint: Option<&'a str>,
    pub client_type: Option<&'a str>,
    pub client_item_id: Option<&'a str>,
    pub accepted_info_hash: Option<&'a str>,
}

pub struct ObservedDownloadIdentityInput<'a> {
    pub download_id: Option<&'a str>,
    pub parameters: &'a [(String, String)],
    pub info_hash_hint: Option<&'a str>,
}

pub(crate) fn coalesce_download_submissions_by_release_attempt(
    submissions: &[DownloadSubmission],
) -> Option<DownloadSubmission> {
    let first = submissions.first()?;
    let first_key = download_submission_release_attempt_key(first);
    submissions
        .iter()
        .all(|submission| download_submission_release_attempt_key(submission) == first_key)
        .then(|| first.clone())
}

pub(crate) fn coalesce_completed_downloads_by_release_observation(
    downloads: &[CompletedDownload],
) -> Option<CompletedDownload> {
    let first = downloads.first()?;
    let first_key = completed_download_release_observation_key(first);
    downloads
        .iter()
        .all(|download| completed_download_release_observation_key(download) == first_key)
        .then(|| first.clone())
}

pub fn observed_download_identity(
    input: ObservedDownloadIdentityInput<'_>,
) -> DownloadSubmissionIdentity {
    let download_id = normalize_token(input.download_id)
        .or_else(|| observed_identity_parameter(input.parameters, DOWNLOAD_ID_PARAMETER))
        .or_else(|| download_id_from_info_hash(input.info_hash_hint));

    DownloadSubmissionIdentity { download_id }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ObservedClientJobResolution {
    Resolved(DownloadId),
    Conflict,
    Unavailable,
}

pub(crate) fn legacy_binding_predates_delete(
    binding: &crate::DownloadClientBindingRecord,
    command_created_at: chrono::DateTime<Utc>,
) -> bool {
    binding.last_seen_at.unwrap_or(binding.created_at) <= command_created_at
}

fn completed_delete_proves_stale_binding(
    command: &crate::DownloadQueueCommandRecord,
    binding: &crate::DownloadClientBindingRecord,
    locator: &ClientJobLocator,
) -> bool {
    if command.status != scryer_domain::DownloadQueueDeleteStatus::Completed {
        return false;
    }
    let Ok(created_at) = chrono::DateTime::parse_from_rfc3339(&command.created_at) else {
        return false;
    };
    let Some(finished_at) = command
        .finished_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return false;
    };
    if finished_at < created_at {
        return false;
    }

    match command.canonical_download_id {
        Some(download_id) => download_id == binding.download_id,
        None => {
            ClientJobLocator::new(
                command.client_id.as_deref(),
                &command.client_type,
                &command.download_client_item_id,
            ) == *locator
                && legacy_binding_predates_delete(binding, created_at.with_timezone(&Utc))
        }
    }
}

async fn heal_binding_after_completed_delete(
    app: &AppUseCase,
    observation: &ObservedClientJob,
    conflicting_download_id: DownloadId,
) -> bool {
    let binding = match app
        .services
        .workflow
        .download_registry
        .find_active_binding_by_locator(&observation.locator)
        .await
    {
        Ok(Some(binding)) if binding.download_id == conflicting_download_id => binding,
        Ok(_) | Err(_) => return false,
    };
    let sources = [
        (
            observation.locator.client_id.clone(),
            observation.locator.client_type.clone(),
            observation.locator.item_id.clone(),
            false,
        ),
        (
            observation.locator.client_id.clone(),
            observation.locator.client_type.clone(),
            observation.locator.item_id.clone(),
            true,
        ),
    ];
    let commands = match app
        .services
        .workflow
        .download_queue_commands
        .list_latest_delete_commands_for_sources(&sources, true)
        .await
    {
        Ok(commands) => commands,
        Err(_) => return false,
    };
    if !commands.iter().any(|command| {
        completed_delete_proves_stale_binding(command, &binding, &observation.locator)
    }) {
        return false;
    }

    if app
        .services
        .workflow
        .download_registry
        .end_binding(&binding.download_id)
        .await
        .is_err()
    {
        return false;
    }

    tracing::info!(
        target: "download_identity_resolver",
        download_id = %binding.download_id,
        config_id = observation.locator.client_id.as_deref().unwrap_or(""),
        client_type = %observation.locator.client_type,
        native_item_id = %observation.locator.item_id,
        "retired stale binding proven by completed delete"
    );
    true
}

/// Resolve an observation to the sole workflow identity.
pub(crate) async fn resolve_observed_client_job(
    app: &AppUseCase,
    observation: ObservedClientJob,
) -> ObservedClientJobResolution {
    let valid_token = observation
        .wire_token
        .as_deref()
        .and_then(scryer_domain::download_identity::DownloadId::from_wire);
    let token = observation.wire_token.as_deref().unwrap_or("");
    let config_id = observation.locator.client_id.as_deref().unwrap_or("");
    let client_type = observation.locator.client_type.as_str();
    let native_item_id = observation.locator.item_id.as_str();

    let mut resolution = app
        .services
        .workflow
        .download_registry
        .resolve_observation(&observation)
        .await;
    if let Ok(ObservationResolution::Conflict {
        binding_download_id,
        ..
    }) = &resolution
        && heal_binding_after_completed_delete(app, &observation, *binding_download_id).await
    {
        resolution = app
            .services
            .workflow
            .download_registry
            .resolve_observation(&observation)
            .await;
    }

    match resolution {
        Ok(ObservationResolution::Resolved {
            download_id,
            newly_foreign,
            attached,
        }) => {
            if valid_token.is_some() && newly_foreign {
                tracing::debug!(
                    target: "download_identity_resolver",
                    token,
                    config_id,
                    client_type,
                    native_item_id,
                    "unknown valid token adopted as foreign"
                );
            }
            if valid_token.is_none() && attached {
                tracing::debug!(
                    target: "download_identity_resolver",
                    config_id,
                    client_type,
                    native_item_id,
                    "ambiguous locator attached"
                );
            }
            ObservedClientJobResolution::Resolved(download_id)
        }
        Ok(ObservationResolution::Conflict {
            token_id,
            binding_download_id,
        }) => {
            tracing::warn!(
                target: "download_identity_resolver",
                token,
                config_id,
                client_type,
                native_item_id,
                token_id = %token_id,
                binding_download_id = %binding_download_id,
                "conflicting canonical download identity observation"
            );
            ObservedClientJobResolution::Conflict
        }
        Err(error) => {
            tracing::warn!(
                target: "download_identity_resolver",
                token,
                config_id,
                client_type,
                native_item_id,
                error = %error,
                "failed to resolve client observation"
            );
            ObservedClientJobResolution::Unavailable
        }
    }
}

pub(crate) fn observed_queue_item_job(
    item: &scryer_domain::DownloadQueueItem,
) -> ObservedClientJob {
    ObservedClientJob {
        locator: ClientJobLocator::new(
            Some(item.client_id.as_str()),
            item.client_type.as_str(),
            item.download_client_item_id.as_str(),
        ),
        wire_token: item
            .download_id
            .as_deref()
            .and_then(DownloadId::from_wire)
            .map(|id| id.to_wire()),
        observed_name: (!item.title_name.trim().is_empty()).then(|| item.title_name.clone()),
        observed_at: Utc::now(),
    }
}

pub(crate) fn observed_completed_job(item: &CompletedDownload) -> ObservedClientJob {
    let observed_name = item
        .release_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| (!item.name.trim().is_empty()).then(|| item.name.clone()));
    ObservedClientJob {
        locator: ClientJobLocator::new(
            Some(item.client_id.as_str()),
            item.client_type.as_str(),
            item.download_client_item_id.as_str(),
        ),
        wire_token: item
            .download_id
            .as_deref()
            .and_then(DownloadId::from_wire)
            .map(|id| id.to_wire()),
        observed_name,
        observed_at: Utc::now(),
    }
}

pub fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
}

pub fn download_id_from_info_hash(info_hash_hint: Option<&str>) -> Option<String> {
    normalize_torrent_info_hash(info_hash_hint)
}

pub fn normalize_torrent_info_hash(raw: Option<&str>) -> Option<String> {
    let mut value = normalize_lower(raw)?;
    if let Some(stripped) = value.strip_prefix("urn:btih:") {
        value = stripped.to_string();
    }
    let normalized = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    matches!(normalized.len(), 40 | 64).then_some(normalized)
}

pub fn accepted_download_submission_identity(
    input: AcceptedDownloadIdentityInput<'_>,
) -> DownloadSubmissionIdentity {
    let expected_info_hash = normalize_torrent_info_hash(input.info_hash_hint)
        .or_else(|| normalize_magnet_info_hash(input.source_hint));
    let actual_info_hash = normalize_torrent_info_hash(input.accepted_info_hash).or_else(|| {
        if source_kind_is_torrent(input.source_kind) || expected_info_hash.is_some() {
            normalize_torrent_info_hash(input.client_item_id)
        } else {
            None
        }
    });

    if let (Some(expected), Some(actual)) = (&expected_info_hash, &actual_info_hash)
        && expected != actual
    {
        tracing::debug!(
            initial_download_id = input.initial_download_id.unwrap_or(""),
            client_type = input.client_type.unwrap_or(""),
            expected_info_hash = expected.as_str(),
            actual_info_hash = actual.as_str(),
            "download_torrent_info_hash_mismatch"
        );
    }

    let download_id = actual_info_hash
        .or(expected_info_hash)
        .or_else(|| {
            client_round_trips_download_id(input.client_type)
                .then(|| normalize_token(input.initial_download_id))
                .flatten()
        })
        .or_else(|| normalize_token(input.client_item_id))
        .or_else(|| normalize_token(input.initial_download_id));

    DownloadSubmissionIdentity { download_id }
}

fn client_round_trips_download_id(client_type: Option<&str>) -> bool {
    matches!(
        normalize_lower(client_type).as_deref(),
        Some("nzbget" | "weaver")
    )
}

fn normalize_token(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn observed_identity_parameter(parameters: &[(String, String)], key: &str) -> Option<String> {
    parameters
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| normalize_token(Some(value)))
}

fn normalize_lower(raw: Option<&str>) -> Option<String> {
    normalize_token(raw).map(|value| value.to_ascii_lowercase())
}

fn download_submission_release_attempt_key(submission: &DownloadSubmission) -> String {
    let scope = match &submission.scope {
        SubmissionScope::Episode { episode_id } => format!("episode:{}", episode_id.trim()),
        SubmissionScope::EpisodeSet { episode_ids } => {
            let mut ids = episode_ids
                .iter()
                .map(|episode_id| episode_id.trim().to_string())
                .filter(|episode_id| !episode_id.is_empty())
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            format!("episodes:{}", ids.join(","))
        }
        SubmissionScope::Collection { collection_id } => {
            format!("collection:{}", collection_id.trim())
        }
        SubmissionScope::Title => "title".to_string(),
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => format!("series_movie:{series_movie_link_id}"),
        SubmissionScope::Orphan => "orphan".to_string(),
    };
    format!(
        "{}\u{1f}{}\u{1f}{}",
        submission.title_id.trim(),
        submission.facet.trim().to_ascii_lowercase(),
        scope
    )
}

fn completed_download_release_observation_key(completed: &CompletedDownload) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:?}",
        normalize_observed_identity_field(&completed.client_type),
        normalize_observed_identity_field(&completed.name),
        normalize_observed_identity_field(&completed.dest_dir),
        completed
            .category
            .as_deref()
            .map(normalize_observed_identity_field)
            .unwrap_or_default(),
        completed.size_bytes
    )
}

fn normalize_observed_identity_field(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_magnet_info_hash(raw: Option<&str>) -> Option<String> {
    raw.and_then(extract_magnet_info_hash)
}

fn source_kind_is_torrent(source_kind: Option<DownloadSourceKind>) -> bool {
    matches!(
        source_kind,
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_torrent_hash_is_download_id() {
        let accepted_hash = "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD";
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::TorrentFile),
            source_hint: Some("https://indexer.example/release.torrent"),
            info_hash_hint: Some("0123456789abcdef0123456789abcdef01234567"),
            client_type: Some("qbittorrent"),
            client_item_id: Some("job-1"),
            accepted_info_hash: Some(accepted_hash),
        });

        assert_eq!(
            identity.download_id.as_deref(),
            Some("abcdefabcdefabcdefabcdefabcdefabcdefabcd")
        );
    }

    #[test]
    fn nzbget_keeps_round_trip_download_id() {
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: Some("https://indexer.example/release.nzb"),
            info_hash_hint: None,
            client_type: Some("nzbget"),
            client_item_id: Some("10010"),
            accepted_info_hash: None,
        });

        assert_eq!(identity.download_id.as_deref(), Some("scryer-download:1"));
    }

    #[test]
    fn sab_uses_client_download_id() {
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: Some("https://indexer.example/release.nzb"),
            info_hash_hint: None,
            client_type: Some("sabnzbd"),
            client_item_id: Some("SABnzbd_nzo_abc"),
            accepted_info_hash: None,
        });

        assert_eq!(identity.download_id.as_deref(), Some("SABnzbd_nzo_abc"));
    }

    #[test]
    fn completed_delete_evidence_requires_exact_identity_and_safe_time_ordering() {
        let command_at = chrono::DateTime::parse_from_rfc3339("2026-08-29T12:00:00Z")
            .expect("valid command timestamp")
            .with_timezone(&Utc);
        let locator = ClientJobLocator::new(Some("client-1"), "weaver", "job-1");
        let binding_id = DownloadId::new();
        let binding = crate::DownloadClientBindingRecord {
            download_id: binding_id,
            client_config_id: locator.client_id.clone(),
            client_type_snapshot: Some(locator.client_type.clone()),
            client_name_snapshot: None,
            native_item_id: Some(locator.item_id.clone()),
            created_at: command_at - chrono::Duration::minutes(2),
            last_seen_at: Some(command_at - chrono::Duration::minutes(1)),
            ended_at: None,
        };
        let command = crate::DownloadQueueCommandRecord {
            id: "delete-1".to_string(),
            action: scryer_domain::DownloadQueueCommandAction::Delete,
            canonical_download_id: None,
            client_id: locator.client_id.clone(),
            client_type: locator.client_type.clone(),
            download_client_item_id: locator.item_id.clone(),
            is_history: false,
            status: scryer_domain::DownloadQueueDeleteStatus::Completed,
            error_text: None,
            requested_by_user_id: None,
            started_at: Some(command_at.to_rfc3339()),
            finished_at: Some((command_at + chrono::Duration::seconds(1)).to_rfc3339()),
            created_at: command_at.to_rfc3339(),
            updated_at: command_at.to_rfc3339(),
        };
        assert!(completed_delete_proves_stale_binding(
            &command, &binding, &locator
        ));
        let binding_at_boundary = crate::DownloadClientBindingRecord {
            last_seen_at: Some(command_at),
            ..binding.clone()
        };
        assert!(completed_delete_proves_stale_binding(
            &command,
            &binding_at_boundary,
            &locator
        ));
        let binding_seen_later = crate::DownloadClientBindingRecord {
            last_seen_at: Some(command_at + chrono::Duration::seconds(1)),
            ..binding.clone()
        };
        assert!(!completed_delete_proves_stale_binding(
            &command,
            &binding_seen_later,
            &locator
        ));

        let wrong_locator = ClientJobLocator::new(Some("client-1"), "weaver", "job-2");
        let mut altered = command.clone();
        altered.canonical_download_id = Some(binding_id);
        assert!(completed_delete_proves_stale_binding(
            &altered,
            &binding_seen_later,
            &wrong_locator
        ));
        altered.canonical_download_id = Some(DownloadId::new());
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
        assert!(!completed_delete_proves_stale_binding(
            &command,
            &binding,
            &wrong_locator
        ));

        altered = command.clone();
        altered.status = scryer_domain::DownloadQueueDeleteStatus::Failed;
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
        altered = command.clone();
        altered.finished_at = None;
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
        altered = command.clone();
        altered.created_at = "not-a-timestamp".to_string();
        assert!(!completed_delete_proves_stale_binding(
            &altered, &binding, &locator
        ));
    }
}
