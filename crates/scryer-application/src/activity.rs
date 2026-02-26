use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::Id;

pub const ACTIVITY_EVENT_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum ActivityKind {
    SettingSaved,
    MovieFetched,
    MovieAdded,
    MovieDownloaded,
    SeriesEpisodeImported,
    AcquisitionSearchCompleted,
    AcquisitionCandidateAccepted,
    AcquisitionCandidateRejected,
    AcquisitionDownloadFailed,
    #[default]
    SystemNotice,
}


#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(Default)]
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
            Self::MovieDownloaded => "movie_downloaded",
            Self::SeriesEpisodeImported => "series_episode_imported",
            Self::AcquisitionSearchCompleted => "acquisition_search_completed",
            Self::AcquisitionCandidateAccepted => "acquisition_candidate_accepted",
            Self::AcquisitionCandidateRejected => "acquisition_candidate_rejected",
            Self::AcquisitionDownloadFailed => "acquisition_download_failed",
            Self::SystemNotice => "system_notice",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActivityEvent {
    pub id: String,
    pub kind: ActivityKind,
    pub severity: ActivitySeverity,
    pub channels: Vec<ActivityChannel>,
    pub actor_user_id: Option<String>,
    pub title_id: Option<String>,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
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
            message,
            occurred_at: Utc::now(),
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
