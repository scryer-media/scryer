use chrono::{DateTime, Utc};
use scryer_domain::{NotificationEventType, Title};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::Id;

pub const ACTIVITY_EVENT_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ActivityKind {
    SettingSaved,
    MovieFetched,
    MovieAdded,
    TitleUpdated,
    MetadataHydrationStarted,
    MetadataHydrationCompleted,
    MetadataHydrationFailed,
    MovieDownloaded,
    SeriesEpisodeImported,
    AcquisitionSearchCompleted,
    AcquisitionCandidateAccepted,
    AcquisitionCandidateRejected,
    AcquisitionDownloadFailed,
    PostProcessingCompleted,
    FileUpgraded,
    ImportRejected,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    #[default]
    SystemNotice,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ActivitySeverity {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivityChannel {
    WebUi,
    Toast,
}

impl ActivityChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WebUi => "web_ui",
            Self::Toast => "toast",
        }
    }
}

impl ActivitySeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl ActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SettingSaved => "setting_saved",
            Self::MovieFetched => "movie_fetched",
            Self::MovieAdded => "movie_added",
            Self::TitleUpdated => "title_updated",
            Self::MetadataHydrationStarted => "metadata_hydration_started",
            Self::MetadataHydrationCompleted => "metadata_hydration_completed",
            Self::MetadataHydrationFailed => "metadata_hydration_failed",
            Self::MovieDownloaded => "movie_downloaded",
            Self::SeriesEpisodeImported => "series_episode_imported",
            Self::AcquisitionSearchCompleted => "acquisition_search_completed",
            Self::AcquisitionCandidateAccepted => "acquisition_candidate_accepted",
            Self::AcquisitionCandidateRejected => "acquisition_candidate_rejected",
            Self::AcquisitionDownloadFailed => "acquisition_download_failed",
            Self::PostProcessingCompleted => "post_processing_completed",
            Self::FileUpgraded => "file_upgraded",
            Self::ImportRejected => "import_rejected",
            Self::SubtitleDownloaded => "subtitle_downloaded",
            Self::SubtitleSearchFailed => "subtitle_search_failed",
            Self::SystemNotice => "system_notice",
        }
    }
}

/// Envelope attached to an `ActivityEvent` to trigger notification dispatch.
///
/// Events that carry this envelope are automatically routed to notification
/// plugins by the background notification dispatcher. Events without it are
/// UI-only (existing behaviour preserved).
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationEnvelope {
    pub event_type: NotificationEventType,
    pub title: String,
    pub body: String,
    pub facet: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationMediaUpdate {
    pub path: String,
    pub update_type: &'static str,
}

impl NotificationMediaUpdate {
    pub fn created(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            update_type: "created",
        }
    }

    pub fn modified(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            update_type: "modified",
        }
    }

    pub fn deleted(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            update_type: "deleted",
        }
    }
}

pub(crate) fn build_lifecycle_notification_metadata(
    title: &Title,
    media_updates: impl IntoIterator<Item = NotificationMediaUpdate>,
) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    metadata.insert("title_name".to_string(), serde_json::json!(title.name));
    metadata.insert(
        "title_facet".to_string(),
        serde_json::json!(title.facet.as_str()),
    );
    if let Some(year) = title.year {
        metadata.insert("title_year".to_string(), serde_json::json!(year));
    }
    if let Some(ref poster) = title.poster_url {
        metadata.insert("poster_url".to_string(), serde_json::json!(poster));
    }

    let mut external_ids = serde_json::Map::new();
    for external_id in &title.external_ids {
        match external_id.source.as_str() {
            "imdb" => {
                external_ids.insert("imdb_id".to_string(), serde_json::json!(external_id.value));
            }
            "tmdb" => {
                external_ids.insert("tmdb_id".to_string(), serde_json::json!(external_id.value));
            }
            "tvdb" => {
                external_ids.insert("tvdb_id".to_string(), serde_json::json!(external_id.value));
            }
            _ => {}
        }
    }
    if !external_ids.contains_key("imdb_id")
        && let Some(ref imdb_id) = title.imdb_id
    {
        external_ids.insert("imdb_id".to_string(), serde_json::json!(imdb_id));
    }
    if !external_ids.is_empty() {
        metadata.insert(
            "external_ids".to_string(),
            serde_json::Value::Object(external_ids),
        );
    }

    let updates: Vec<serde_json::Value> = media_updates
        .into_iter()
        .map(|update| {
            serde_json::json!({
                "path": update.path,
                "update_type": update.update_type,
            })
        })
        .collect();
    if !updates.is_empty() {
        if let Some(first_path) = updates
            .first()
            .and_then(|value| value.get("path"))
            .and_then(serde_json::Value::as_str)
        {
            metadata.insert("file_path".to_string(), serde_json::json!(first_path));
        }
        metadata.insert(
            "media_updates".to_string(),
            serde_json::Value::Array(updates),
        );
    }

    metadata
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActivityEvent {
    pub id: String,
    pub kind: ActivityKind,
    pub severity: ActivitySeverity,
    pub channels: Vec<ActivityChannel>,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub facet: Option<String>,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
    pub notification: Option<NotificationEnvelope>,
}

impl ActivityEvent {
    pub fn new(
        kind: ActivityKind,
        actor_user_id: Option<String>,
        title_id: Option<String>,
        message: String,
        severity: ActivitySeverity,
        channels: Vec<ActivityChannel>,
    ) -> Self {
        Self {
            id: Id::new().0,
            kind,
            severity,
            channels,
            actor_user_id,
            title_id,
            facet: None,
            message,
            occurred_at: Utc::now(),
            notification: None,
        }
    }

    pub fn with_default_channels(
        kind: ActivityKind,
        actor_user_id: Option<String>,
        title_id: Option<String>,
        message: String,
    ) -> Self {
        Self::new(
            kind,
            actor_user_id,
            title_id,
            message,
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        )
    }

    pub fn with_facet(mut self, facet: String) -> Self {
        self.facet = Some(facet);
        self
    }

    pub fn with_notification(mut self, envelope: NotificationEnvelope) -> Self {
        self.notification = Some(envelope);
        self
    }
}

#[derive(Clone)]
pub struct ActivityStream {
    entries: Arc<Mutex<VecDeque<ActivityEvent>>>,
}

impl ActivityStream {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn push(&self, event: ActivityEvent) {
        let mut entries = self.entries.lock().await;
        entries.push_back(event);
        while entries.len() > ACTIVITY_EVENT_LIMIT {
            let _ = entries.pop_front();
        }
    }

    pub async fn list(&self, limit: i64, offset: i64) -> Vec<ActivityEvent> {
        let limit = if limit <= 0 {
            ACTIVITY_EVENT_LIMIT
        } else {
            limit as usize
        };
        let offset = if offset <= 0 { 0 } else { offset as usize };

        let entries = self.entries.lock().await;
        entries
            .iter()
            .rev()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod activity_tests;
