//! Safety preconditions for lifecycle actions.
//!
//! Two read-only observations the action executor must consult before a due
//! lifecycle action touches media:
//!
//! 1. **Live playback.** A current playback session on a supported
//!    Plex/Jellyfin/Emby source holds every action on the title it is playing.
//! 2. **Active acquisition.** A title with a grab or import in flight must not
//!    be deleted, unmonitored, or otherwise disturbed underneath the pipeline
//!    that is currently writing to it.
//!
//! # The playback hold is per title
//!
//! The probe reports what each live session is playing, normalized the way a
//! watch-signal item is. Each session is placed on a Scryer title with the
//! evidence the watch-signal sync and the playback mapping already keep, all
//! read in batches for the whole snapshot:
//!
//! * the connection's playback mapping (`media_server_playback_items`), from
//!   the session's own provider item id and, for an episode, its series'
//!   provider item id;
//! * the item's external ids, through the same `title_external_ids` join and
//!   facet rule the watch-signal mapping uses;
//! * the item's file paths, read back through the connection's path mappings
//!   and matched against Scryer's media files.
//!
//! A session playing a title holds every action on that title, whatever its
//! kind. For a series, any of its episodes playing holds every subject of the
//! series: the title, each season and each episode. Other titles proceed.
//!
//! # What holds and what does not
//!
//! * A session that cannot be placed on any title holds **nothing**. Holding
//!   every title's work because one stream could not be matched would let any
//!   unmatched item stop the whole library. The session is logged by name and
//!   counted as unmatched in the pass report, so the gap in the mapping can be
//!   found and fixed.
//! * An enabled connection that cannot be queried is *unknown*, not idle. What
//!   it plays cannot be seen at all, so it holds **destructive** work on every
//!   title until it answers again; reversible work proceeds. The one case that
//!   is genuinely clear rather than unknown is having no enabled connections:
//!   nothing can be playing on servers Scryer does not know about.
//!
//! # One snapshot per pass
//!
//! The executor asks about many candidates in one pass. It reads one picture
//! for them all: the first candidate that reaches the playback check probes
//! and attributes, and later candidates look their title up in that picture.
//! Each action pass starts with a fresh one, and a picture older than
//! [`PLAYBACK_PICTURE_MAX_AGE`] is read again, so a long pass still sees a
//! session that starts while it runs. A probe that fails is never kept.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::contracts::AcquisitionScopeStatesQuery;
use crate::media_server_signals::{ScryerLibraryPaths, TitleExternalIdIndex, is_foreign_item};
use crate::ports::{PlaybackActivitySnapshot, PlaybackProbeStatus, PlayingMedia};
use crate::types::AcquisitionScopeStatus;
use crate::{AppResult, AppUseCase};
use scryer_domain::{ImportRecord, MediaServerPlaybackEntityKind};

/// Client parameter Scryer stamps on its own grabs so a completed download can
/// be attributed back to the title it was grabbed for. Mirrors the private
/// `SCRYER_TITLE_ID_PARAM` in `import::workflow::completed`; kept as a literal
/// here rather than widening that module's visibility for a read-only probe.
const SCRYER_TITLE_ID_PARAM: &str = "*scryer_title_id";

/// How long one playback picture answers for a pass before it is read again.
pub const PLAYBACK_PICTURE_MAX_AGE: Duration = Duration::from_secs(30);

/// One live session, as a hold names it: the connection it plays on and the
/// item's name as the server gave it. Never a path or a credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayingSession {
    pub connection_id: String,
    pub name: Option<String>,
}

impl std::fmt::Display for PlayingSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{name:?} on {}", self.connection_id),
            None => write!(f, "an unnamed item on {}", self.connection_id),
        }
    }
}

/// Whether live playback holds a due lifecycle action on one title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecyclePlaybackHold {
    /// Nothing that answered is playing this title, and nothing else stops
    /// this kind of action. (Also the answer when there are no enabled
    /// connections to ask.)
    Clear,
    /// These sessions are playing the title. Holds every action kind.
    Hold { sessions: Vec<PlayingSession> },
    /// At least one enabled connection could not be asked, so playback of the
    /// title could not be ruled out. Holds destructive work only.
    Unknown { reason: String },
}

impl LifecyclePlaybackHold {
    /// Whether the action must be held. The picture already decided whether
    /// the action's kind matters, so every answer but `Clear` holds.
    pub fn holds(&self) -> bool {
        !matches!(self, Self::Clear)
    }

    /// The stable code the executor stores for this hold.
    pub fn reason_code(&self) -> Option<&'static str> {
        use super::action_execution::execution_reason;
        match self {
            Self::Clear => None,
            Self::Hold { .. } => Some(execution_reason::PLAYBACK_HOLD),
            Self::Unknown { .. } => Some(execution_reason::PLAYBACK_UNKNOWN),
        }
    }

    /// A sentence for the log naming what is playing.
    pub fn describe(&self) -> String {
        let list = |sessions: &[PlayingSession]| {
            sessions
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        };
        match self {
            Self::Clear => "no playback holds this title".to_string(),
            Self::Hold { sessions } => format!("playing now: {}", list(sessions)),
            Self::Unknown { reason } => reason.clone(),
        }
    }
}

/// Where one session was placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionAttribution {
    /// The session plays these titles. More than one only when the evidence
    /// disagrees, and then every one of them holds.
    Titles(BTreeSet<String>),
    /// The session is shown not to be playing Scryer media.
    NotScryerMedia,
    /// The session could not be placed on any title. It holds nothing; the
    /// pass logs it and counts it as unmatched.
    Unattributed,
}

/// What every enabled connection is playing, placed on Scryer titles, as of
/// one probe. Looked up by title for each candidate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecyclePlaybackPicture {
    playing: HashMap<String, Vec<PlayingSession>>,
    unattributed: Vec<PlayingSession>,
    unreachable: Vec<String>,
}

impl LifecyclePlaybackPicture {
    /// The picture of a probe that could not run at all.
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self {
            unreachable: vec![reason.into()],
            ..Self::default()
        }
    }

    /// Whether this picture came from a probe that answered for every
    /// connection it could list (one worth keeping for the pass).
    fn probe_failed(&self) -> bool {
        self.playing.is_empty() && self.unattributed.is_empty() && !self.unreachable.is_empty()
    }

    /// Sessions playing something no title could be matched to. They hold
    /// nothing; the pass reports them.
    pub fn unmatched_sessions(&self) -> &[PlayingSession] {
        &self.unattributed
    }

    /// The hold for an action on `title_id`. `destructive` is whether the
    /// action can delete, which is the only kind an unreachable connection
    /// holds.
    ///
    /// Precedence, highest first: a session playing this title; for
    /// destructive work, a connection that could not be asked; clear.
    pub fn hold_for_title(&self, title_id: &str, destructive: bool) -> LifecyclePlaybackHold {
        if let Some(sessions) = self.playing.get(title_id) {
            return LifecyclePlaybackHold::Hold {
                sessions: sessions.clone(),
            };
        }
        if destructive && !self.unreachable.is_empty() {
            return LifecyclePlaybackHold::Unknown {
                reason: format!(
                    "playback could not be ruled out on {}: {}",
                    self.unreachable.len(),
                    self.unreachable.join("; ")
                ),
            };
        }
        LifecyclePlaybackHold::Clear
    }
}

/// Folds a playback snapshot and each session's attribution into the
/// picture. `attributions` is positionally aligned with the snapshot: one
/// list per connection, one entry per playing session. A session without an
/// entry is unattributed.
pub fn fold_playback_picture(
    snapshot: &PlaybackActivitySnapshot,
    attributions: &[Vec<SessionAttribution>],
) -> LifecyclePlaybackPicture {
    let mut picture = LifecyclePlaybackPicture::default();
    for (position, connection) in snapshot.connections.iter().enumerate() {
        match &connection.status {
            PlaybackProbeStatus::Idle => {}
            PlaybackProbeStatus::Unreachable(reason) => picture
                .unreachable
                .push(format!("{} ({reason})", connection.connection_id)),
            PlaybackProbeStatus::Playing(sessions) => {
                for (index, media) in sessions.iter().enumerate() {
                    let session = PlayingSession {
                        connection_id: connection.connection_id.clone(),
                        name: playing_media_name(media),
                    };
                    match attributions
                        .get(position)
                        .and_then(|sessions| sessions.get(index))
                        .unwrap_or(&SessionAttribution::Unattributed)
                    {
                        SessionAttribution::Titles(titles) if !titles.is_empty() => {
                            for title_id in titles {
                                picture
                                    .playing
                                    .entry(title_id.clone())
                                    .or_default()
                                    .push(session.clone());
                            }
                        }
                        SessionAttribution::NotScryerMedia => {}
                        SessionAttribution::Titles(_) | SessionAttribution::Unattributed => {
                            picture.unattributed.push(session)
                        }
                    }
                }
            }
        }
    }
    picture
}

fn playing_media_name(media: &PlayingMedia) -> Option<String> {
    match media {
        PlayingMedia::Video(item) => item.name.clone(),
        PlayingMedia::Other { name, .. } => name.clone(),
        PlayingMedia::Audio => None,
    }
}

/// Everything the batched reads found, keyed for the per-session pass.
#[derive(Default)]
pub(crate) struct PlaybackEvidence {
    /// `(connection id, provider item id)` to the mapped entities.
    pub(crate) mapped: HashMap<(String, String), Vec<(MediaServerPlaybackEntityKind, String)>>,
    /// Episode id to its title id.
    pub(crate) episode_titles: HashMap<String, String>,
    /// External ids to titles, as the watch-signal mapping builds it.
    pub(crate) external_ids: TitleExternalIdIndex,
    /// A session's file path, in the server's spelling, per connection, to
    /// the title of the Scryer media file at it.
    pub(crate) file_titles: HashMap<(String, String), String>,
    /// Scryer's library roots, `None` when they could not be read.
    pub(crate) library_roots: Option<Vec<String>>,
    /// A batched read failed, so nothing can be shown to be foreign.
    pub(crate) incomplete: bool,
}

/// Places every playing session of `snapshot` with `evidence`. Pure; the
/// positional shape [`fold_playback_picture`] reads.
pub(crate) fn attribute_playback_sessions(
    snapshot: &PlaybackActivitySnapshot,
    evidence: &PlaybackEvidence,
) -> Vec<Vec<SessionAttribution>> {
    snapshot
        .connections
        .iter()
        .map(|connection| {
            let PlaybackProbeStatus::Playing(sessions) = &connection.status else {
                return Vec::new();
            };
            let library =
                ScryerLibraryPaths::new(evidence.library_roots.clone(), &connection.path_mappings);
            sessions
                .iter()
                .map(|media| {
                    attribute_session(&connection.connection_id, media, evidence, &library)
                })
                .collect()
        })
        .collect()
}

fn attribute_session(
    connection_id: &str,
    media: &PlayingMedia,
    evidence: &PlaybackEvidence,
    library: &ScryerLibraryPaths,
) -> SessionAttribution {
    let (item_ids, file_paths, video) = match media {
        PlayingMedia::Audio => return SessionAttribution::NotScryerMedia,
        PlayingMedia::Video(item) => (
            [
                Some(item.provider_item_id.as_str()),
                item.series_provider_item_id.as_deref(),
            ],
            item.file_paths.as_slice(),
            Some(item),
        ),
        PlayingMedia::Other {
            provider_item_id,
            file_paths,
            ..
        } => (
            [provider_item_id.as_deref(), None],
            file_paths.as_slice(),
            None,
        ),
    };

    let mut titles = BTreeSet::new();
    for item_id in item_ids.into_iter().flatten() {
        let key = (connection_id.to_string(), item_id.to_string());
        for (kind, entity_id) in evidence.mapped.get(&key).into_iter().flatten() {
            match kind {
                MediaServerPlaybackEntityKind::Title => {
                    titles.insert(entity_id.clone());
                }
                MediaServerPlaybackEntityKind::Episode => {
                    if let Some(title_id) = evidence.episode_titles.get(entity_id) {
                        titles.insert(title_id.clone());
                    }
                }
            }
        }
    }
    if let Some(item) = video
        && let Some(title_id) =
            crate::media_server_signals::resolve_title(item, &evidence.external_ids)
    {
        titles.insert(title_id);
    }
    for path in file_paths {
        if let Some(title_id) = evidence
            .file_titles
            .get(&(connection_id.to_string(), path.clone()))
        {
            titles.insert(title_id.clone());
        }
    }

    if !titles.is_empty() {
        return SessionAttribution::Titles(titles);
    }
    match video {
        Some(item)
            if !evidence.incomplete && is_foreign_item(item, &evidence.external_ids, library) =>
        {
            SessionAttribution::NotScryerMedia
        }
        _ => SessionAttribution::Unattributed,
    }
}

/// Whether a title currently has acquisition work in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleActivityCheck {
    /// A grab or an import for this title is in flight.
    Active,
    /// Every signal answered, and none of them shows work in flight.
    Clear,
    /// A signal could not be read, or in-flight work could not be attributed to
    /// a title. Holds destructive work.
    Unknown { reason: String },
}

impl LifecycleActivityCheck {
    /// Whether a destructive action must be held. Both `Active` and `Unknown`
    /// hold; only `Clear` releases.
    pub fn holds_destructive_work(&self) -> bool {
        !matches!(self, Self::Clear)
    }
}

type CachedPlaybackPicture = (tokio::time::Instant, Arc<LifecyclePlaybackPicture>);

/// The pass's playback picture, shared by every candidate that asks.
#[derive(Clone, Default)]
pub(crate) struct LifecyclePlaybackCache {
    picture: Arc<tokio::sync::Mutex<Option<CachedPlaybackPicture>>>,
}

impl LifecyclePlaybackCache {
    /// Forget the kept picture, so the next ask probes again.
    pub(crate) async fn clear(&self) {
        *self.picture.lock().await = None;
    }

    /// How many sessions the kept picture could not match to a title, without
    /// probing. Zero when no picture is kept.
    pub(crate) async fn kept_unmatched_sessions(&self) -> usize {
        self.picture
            .lock()
            .await
            .as_ref()
            .map_or(0, |(_, picture)| picture.unattributed.len())
    }
}

/// The title a queued import will land in, when the queued row says so.
///
/// `imports` carries no title column, and its payload is one of three shapes,
/// so attribution is a payload read rather than a query:
///
/// * a manual import stores the operator's chosen `title_id`;
/// * a completed-download import stores `target_title_id` (legacy alias
///   `manual_title_id`) when the release evidence carries no Scryer identity;
/// * a download Scryer grabbed carries `*scryer_title_id` in the completed
///   download's client parameters, at `completed.parameters` in the current
///   payload shape and at `parameters` in the legacy one.
///
/// `None` means the row exists but names no title — a foreign import whose
/// title is resolved from release evidence at execution time. The caller must
/// treat that as unknown, never as "not this title".
pub(crate) fn queued_import_title_id(record: &ImportRecord) -> Option<String> {
    let payload = serde_json::from_str::<serde_json::Value>(&record.payload_json).ok()?;

    let direct = ["title_id", "target_title_id", "manual_title_id"]
        .into_iter()
        .filter_map(|key| payload.get(key))
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .find(|value| !value.is_empty());
    if let Some(title_id) = direct {
        return Some(title_id.to_string());
    }

    [
        payload.pointer("/completed/parameters"),
        payload.get("parameters"),
    ]
    .into_iter()
    .flatten()
    .filter_map(serde_json::Value::as_array)
    .flatten()
    .filter_map(serde_json::Value::as_array)
    .filter_map(|pair| {
        // Client parameters serialize as two-element `[key, value]` arrays.
        let mut entry = pair.iter();
        let key = entry.next()?.as_str()?;
        if key != SCRYER_TITLE_ID_PARAM {
            return None;
        }
        entry.next()?.as_str()
    })
    .map(str::trim)
    .find(|value| !value.is_empty())
    .map(str::to_string)
}

impl AppUseCase {
    /// Whether live playback holds an action on `title_id`. `destructive` is
    /// whether the action can delete. Reads the pass's playback picture; see
    /// the module docs.
    pub async fn lifecycle_playback_hold_for_title(
        &self,
        title_id: &str,
        destructive: bool,
    ) -> LifecyclePlaybackHold {
        self.lifecycle_playback_picture()
            .await
            .hold_for_title(title_id, destructive)
    }

    /// The pass's playback picture: the kept one while it is fresh, else a
    /// new probe. The lock is held across the probe so candidates asking at
    /// once share one read.
    pub(crate) async fn lifecycle_playback_picture(&self) -> Arc<LifecyclePlaybackPicture> {
        let cache = &self.runtime.integrations.lifecycle_playback;
        let mut kept = cache.picture.lock().await;
        if let Some((read_at, picture)) = kept.as_ref()
            && read_at.elapsed() < PLAYBACK_PICTURE_MAX_AGE
        {
            return picture.clone();
        }
        let picture = Arc::new(self.observe_lifecycle_playback().await);
        *kept = (!picture.probe_failed()).then(|| (tokio::time::Instant::now(), picture.clone()));
        picture
    }

    /// Probes live playback and places each session on a title.
    ///
    /// A failure to read the connection list itself is a picture in which
    /// playback could not be ruled out, rather than an `Err`: the caller's
    /// question is "may I touch media?", and the fail-closed answer to a
    /// broken probe is the same as to an unreachable server.
    async fn observe_lifecycle_playback(&self) -> LifecyclePlaybackPicture {
        let snapshot = match self
            .services
            .integrations
            .media_server_playback_probe
            .active_playback()
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return LifecyclePlaybackPicture::unknown(format!(
                    "playback probe unavailable: {error}"
                ));
            }
        };
        let evidence = self.lifecycle_playback_evidence(&snapshot).await;
        let picture = fold_playback_picture(
            &snapshot,
            &attribute_playback_sessions(&snapshot, &evidence),
        );
        if !picture.unattributed.is_empty() {
            tracing::info!(
                unmatched = picture.unattributed.len(),
                sessions = %picture
                    .unattributed
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
                "lifecycle playback: sessions matched no title and hold nothing"
            );
        }
        picture
    }

    /// Every read the attribution needs, each one batched over the whole
    /// snapshot. Nothing is read when nothing plays.
    async fn lifecycle_playback_evidence(
        &self,
        snapshot: &PlaybackActivitySnapshot,
    ) -> PlaybackEvidence {
        let mut evidence = PlaybackEvidence::default();
        let mut provider_items = Vec::new();
        let mut video_items = Vec::new();
        let mut scryer_paths: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for connection in &snapshot.connections {
            let PlaybackProbeStatus::Playing(sessions) = &connection.status else {
                continue;
            };
            let library = ScryerLibraryPaths::new(None, &connection.path_mappings);
            for media in sessions {
                let (ids, paths): (Vec<&str>, &[String]) = match media {
                    PlayingMedia::Audio => continue,
                    PlayingMedia::Video(item) => {
                        video_items.push(item.clone());
                        (
                            std::iter::once(item.provider_item_id.as_str())
                                .chain(item.series_provider_item_id.as_deref())
                                .collect(),
                            &item.file_paths,
                        )
                    }
                    PlayingMedia::Other {
                        provider_item_id,
                        file_paths,
                        ..
                    } => (
                        provider_item_id.as_deref().into_iter().collect(),
                        file_paths,
                    ),
                };
                for id in ids.into_iter().map(str::trim).filter(|id| !id.is_empty()) {
                    provider_items.push((connection.connection_id.clone(), id.to_string()));
                }
                for path in paths {
                    scryer_paths
                        .entry(library.to_scryer_path(path))
                        .or_default()
                        .push((connection.connection_id.clone(), path.clone()));
                }
            }
        }
        if provider_items.is_empty() && video_items.is_empty() && scryer_paths.is_empty() {
            return evidence;
        }

        match self
            .services
            .integrations
            .media_server_connections
            .list_playback_items_for_provider_items(&provider_items)
            .await
        {
            Ok(items) => {
                for item in items {
                    evidence
                        .mapped
                        .entry((item.connection_id, item.provider_item_id))
                        .or_default()
                        .push((item.entity_kind, item.entity_id));
                }
            }
            Err(_) => evidence.incomplete = true,
        }
        let episode_ids = evidence
            .mapped
            .values()
            .flatten()
            .filter(|(kind, _)| *kind == MediaServerPlaybackEntityKind::Episode)
            .map(|(_, id)| id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if !episode_ids.is_empty() {
            match self
                .services
                .catalog
                .shows
                .get_episodes_by_ids(&episode_ids)
                .await
            {
                Ok(episodes) => {
                    evidence.episode_titles = episodes
                        .into_iter()
                        .map(|episode| (episode.id, episode.title_id))
                        .collect();
                }
                Err(_) => evidence.incomplete = true,
            }
        }
        match self.signal_title_index(&video_items).await {
            Ok(index) => evidence.external_ids = index,
            Err(_) => evidence.incomplete = true,
        }
        if !scryer_paths.is_empty() {
            let paths = scryer_paths.keys().cloned().collect::<Vec<_>>();
            match self
                .services
                .library
                .media_files
                .list_media_files_by_paths(&paths)
                .await
            {
                Ok(files) => {
                    for (path, file) in files {
                        for key in scryer_paths.get(&path).into_iter().flatten() {
                            evidence
                                .file_titles
                                .insert(key.clone(), file.title_id.clone());
                        }
                    }
                }
                Err(_) => evidence.incomplete = true,
            }
        }
        evidence.library_roots = match self.services.catalog.libraries.list(None).await {
            Ok(libraries) => Some(
                libraries
                    .into_iter()
                    .flat_map(|library| library.roots.into_iter().map(|root| root.path))
                    .collect(),
            ),
            Err(_) => None,
        };
        evidence
    }

    /// Whether `title_id` currently has a download, import, or acquisition
    /// workflow in flight.
    ///
    /// # Signals, and why these
    ///
    /// * **Acquisition scope states in `grabbed`**
    ///   ([`crate::ports::AcquisitionScopeStateRepository::count_acquisition_scope_states`]
    ///   filtered by `title_id`). This is the authoritative per-title in-flight
    ///   marker: a scope enters `grabbed` when a release is grabbed for it and
    ///   leaves only when the import lands (or an operator completes/reopens
    ///   it), so one indexed count covers the whole grab → download → import
    ///   lifetime of every scope under the title, movie and episode alike.
    /// * **The queued import set** ([`crate::ports::ImportRepository::list_pending_imports`],
    ///   i.e. rows in `queued`/`pending`/`running`/`processing`). This catches
    ///   the import of a download Scryer never grabbed, which has no scope
    ///   state to observe. It is a bounded read of the live work queue, not a
    ///   history scan.
    ///
    /// Ports deliberately *not* used: `DownloadRegistryRepository` and
    /// `DownloadSubmissionRepository` expose no per-title lookup at all
    /// (`DownloadRecord` has no title column, and the submission lookups key on
    /// an info hash or a normalized release name), and
    /// `WorkflowOperationRepository` exposes only `create_workflow_operation` —
    /// there is nothing to read. Answering from those would mean an unbounded
    /// table scan for a check that runs per candidate.
    ///
    /// # Fail closed
    ///
    /// Any repository read that fails is [`LifecycleActivityCheck::Unknown`],
    /// never `Clear`. A queued import row that names no title is also
    /// `Unknown`, not "not this title": `imports` has no title column, so an
    /// unattributed row could be for this title. That over-holds while a
    /// foreign import is in flight — the safe direction, and the reason a
    /// per-title import index is the follow-up this check wants.
    pub async fn title_has_active_acquisition(
        &self,
        title_id: &str,
    ) -> AppResult<LifecycleActivityCheck> {
        let title_id = title_id.trim();
        if title_id.is_empty() {
            return Ok(LifecycleActivityCheck::Unknown {
                reason: "activity check requires a title id".to_string(),
            });
        }

        let grabbed_scopes = self
            .services
            .workflow
            .acquisition_scope_states
            .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: vec![AcquisitionScopeStatus::Grabbed.as_str().to_string()],
                title_id: Some(title_id.to_string()),
                ..AcquisitionScopeStatesQuery::default()
            })
            .await;

        // Only read the import queue when the grab signal left the answer
        // open: a grab in flight already decides `Active`, and an unreadable
        // scope table already decides `Unknown`. The decision itself stays in
        // the fold — this match only skips work the fold would ignore.
        let queued_import_titles = match &grabbed_scopes {
            Ok(count) if *count == 0 => self
                .services
                .workflow
                .imports
                .list_pending_imports()
                .await
                .map(|records| records.iter().map(queued_import_title_id).collect()),
            Ok(_) | Err(_) => Ok(Vec::new()),
        };

        Ok(fold_acquisition_activity(
            title_id,
            grabbed_scopes,
            queued_import_titles,
        ))
    }
}

/// Folds the two acquisition signals into the activity answer.
///
/// Precedence, highest first:
///
/// 1. a grabbed scope for this title, or a queued import naming it → `Active`;
/// 2. a signal that could not be read, or a queued import naming no title at
///    all → `Unknown`;
/// 3. both signals answered and neither shows work → `Clear`.
///
/// `queued_import_titles` is one entry per queued import row: `Some(title_id)`
/// when the row names the title it will land in, `None` when it names none.
fn fold_acquisition_activity(
    title_id: &str,
    grabbed_scopes: AppResult<i64>,
    queued_import_titles: AppResult<Vec<Option<String>>>,
) -> LifecycleActivityCheck {
    match grabbed_scopes {
        Ok(count) if count > 0 => return LifecycleActivityCheck::Active,
        Ok(_) => {}
        Err(error) => {
            return LifecycleActivityCheck::Unknown {
                reason: format!("acquisition scope states unreadable: {error}"),
            };
        }
    }

    let queued = match queued_import_titles {
        Ok(queued) => queued,
        Err(error) => {
            return LifecycleActivityCheck::Unknown {
                reason: format!("queued imports unreadable: {error}"),
            };
        }
    };

    let mut unattributed = 0usize;
    for queued_title_id in &queued {
        match queued_title_id.as_deref() {
            Some(queued_title_id) if queued_title_id == title_id => {
                return LifecycleActivityCheck::Active;
            }
            Some(_) => {}
            None => unattributed += 1,
        }
    }
    if unattributed > 0 {
        return LifecycleActivityCheck::Unknown {
            reason: format!("{unattributed} queued import(s) name no title"),
        };
    }

    LifecycleActivityCheck::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{ConnectionPlaybackActivity, ProviderPlayedItem};
    use chrono::Utc;
    use scryer_domain::{ImportStatus, ImportType, MediaServerProvider, MediaServerSignalKind};

    fn connection(id: &str, status: PlaybackProbeStatus) -> ConnectionPlaybackActivity {
        ConnectionPlaybackActivity {
            connection_id: id.to_string(),
            provider: MediaServerProvider::Jellyfin,
            path_mappings: Vec::new(),
            status,
        }
    }

    fn snapshot(connections: Vec<ConnectionPlaybackActivity>) -> PlaybackActivitySnapshot {
        PlaybackActivitySnapshot {
            connections,
            observed_at: Utc::now(),
        }
    }

    fn movie(item_id: &str, name: &str) -> PlayingMedia {
        PlayingMedia::Video(ProviderPlayedItem {
            provider_item_id: item_id.to_string(),
            kind: MediaServerSignalKind::Movie,
            name: Some(name.to_string()),
            ..ProviderPlayedItem::default()
        })
    }

    fn episode(item_id: &str, series_item_id: Option<&str>) -> PlayingMedia {
        PlayingMedia::Video(ProviderPlayedItem {
            provider_item_id: item_id.to_string(),
            kind: MediaServerSignalKind::Episode,
            name: Some("Episode".to_string()),
            series_provider_item_id: series_item_id.map(str::to_string),
            season_number: Some(1),
            episode_number: Some(2),
            ..ProviderPlayedItem::default()
        })
    }

    fn playing(id: &str, sessions: Vec<PlayingMedia>) -> ConnectionPlaybackActivity {
        connection(id, PlaybackProbeStatus::Playing(sessions))
    }

    fn mapped(
        entries: &[(&str, &str, MediaServerPlaybackEntityKind, &str)],
    ) -> HashMap<(String, String), Vec<(MediaServerPlaybackEntityKind, String)>> {
        let mut map: HashMap<_, Vec<_>> = HashMap::new();
        for (connection_id, item_id, kind, entity_id) in entries {
            map.entry((connection_id.to_string(), item_id.to_string()))
                .or_default()
                .push((*kind, entity_id.to_string()));
        }
        map
    }

    /// Attribute and fold in one step, as the use case does.
    fn picture(
        snapshot: &PlaybackActivitySnapshot,
        evidence: &PlaybackEvidence,
    ) -> LifecyclePlaybackPicture {
        fold_playback_picture(snapshot, &attribute_playback_sessions(snapshot, evidence))
    }

    fn titles(ids: &[&str]) -> SessionAttribution {
        SessionAttribution::Titles(ids.iter().map(|id| id.to_string()).collect())
    }

    fn import_record(payload_json: &str) -> ImportRecord {
        ImportRecord {
            id: "import-1".to_string(),
            source_client_id: None,
            source_system: "test".to_string(),
            source_ref: "ref".to_string(),
            import_type: ImportType::MovieDownload,
            status: ImportStatus::Pending,
            payload_json: payload_json.to_string(),
            result_json: None,
            download_id: None,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // ── The fold ────────────────────────────────────────────────────────────

    #[test]
    fn no_configured_connections_is_clear() {
        // Nothing can be playing on servers Scryer does not know about.
        let folded = fold_playback_picture(&snapshot(Vec::new()), &[]);
        assert_eq!(
            folded.hold_for_title("title-a", true),
            LifecyclePlaybackHold::Clear
        );
    }

    #[test]
    fn every_connection_idle_is_clear() {
        let folded = fold_playback_picture(
            &snapshot(vec![
                connection("plex", PlaybackProbeStatus::Idle),
                connection("jellyfin", PlaybackProbeStatus::Idle),
            ]),
            &[],
        );
        assert!(!folded.hold_for_title("title-a", true).holds());
    }

    #[test]
    fn a_session_on_title_a_holds_a_and_not_b() {
        let snapshot = snapshot(vec![playing("jellyfin", vec![movie("jf-a", "Alpha")])]);
        let folded = fold_playback_picture(&snapshot, &[vec![titles(&["title-a"])]]);

        for destructive in [true, false] {
            let LifecyclePlaybackHold::Hold { sessions } =
                folded.hold_for_title("title-a", destructive)
            else {
                panic!("title-a must hold");
            };
            assert_eq!(
                sessions,
                vec![PlayingSession {
                    connection_id: "jellyfin".to_string(),
                    name: Some("Alpha".to_string()),
                }]
            );
            assert_eq!(
                folded.hold_for_title("title-b", destructive),
                LifecyclePlaybackHold::Clear
            );
        }
        assert_eq!(
            folded.hold_for_title("title-a", true).reason_code(),
            Some(super::super::action_execution::execution_reason::PLAYBACK_HOLD)
        );
        assert!(
            folded
                .hold_for_title("title-a", true)
                .describe()
                .contains("Alpha")
        );
    }

    #[test]
    fn an_unmatched_session_holds_nothing_and_is_reported() {
        let snapshot = snapshot(vec![playing("plex", vec![movie("px-1", "Mystery")])]);
        let folded = fold_playback_picture(&snapshot, &[vec![SessionAttribution::Unattributed]]);

        for title in ["title-a", "title-b"] {
            for destructive in [true, false] {
                assert_eq!(
                    folded.hold_for_title(title, destructive),
                    LifecyclePlaybackHold::Clear
                );
            }
        }
        assert_eq!(
            folded.unmatched_sessions(),
            [PlayingSession {
                connection_id: "plex".to_string(),
                name: Some("Mystery".to_string()),
            }]
        );
    }

    #[test]
    fn a_session_without_an_attribution_entry_is_unmatched() {
        let snapshot = snapshot(vec![playing("plex", vec![movie("px-1", "Mystery")])]);
        let folded = fold_playback_picture(&snapshot, &[]);
        assert_eq!(
            folded.hold_for_title("title-a", true),
            LifecyclePlaybackHold::Clear
        );
        assert_eq!(folded.unmatched_sessions().len(), 1);
    }

    #[test]
    fn a_session_shown_not_to_be_scryer_media_holds_nothing() {
        let snapshot = snapshot(vec![playing("plex", vec![PlayingMedia::Audio])]);
        let folded = fold_playback_picture(&snapshot, &[vec![SessionAttribution::NotScryerMedia]]);
        assert_eq!(
            folded.hold_for_title("title-a", true),
            LifecyclePlaybackHold::Clear
        );
    }

    #[test]
    fn an_unreachable_connection_holds_destructive_work_only() {
        let folded = fold_playback_picture(
            &snapshot(vec![
                connection("plex", PlaybackProbeStatus::Idle),
                connection(
                    "jellyfin",
                    PlaybackProbeStatus::Unreachable("status 401".to_string()),
                ),
            ]),
            &[],
        );
        let LifecyclePlaybackHold::Unknown { reason } = folded.hold_for_title("title-a", true)
        else {
            panic!("expected unknown");
        };
        assert!(reason.contains("jellyfin"), "{reason}");
        assert!(reason.contains("status 401"), "{reason}");
        assert_eq!(
            folded.hold_for_title("title-a", false),
            LifecyclePlaybackHold::Clear
        );
    }

    #[test]
    fn a_session_on_this_title_beats_an_unreachable_peer() {
        // A live session is a fact; it does not become less certain because a
        // different server was down.
        let snapshot = snapshot(vec![
            playing("plex", vec![movie("px-a", "Alpha")]),
            connection(
                "jellyfin",
                PlaybackProbeStatus::Unreachable("timeout".to_string()),
            ),
        ]);
        let folded = fold_playback_picture(&snapshot, &[vec![titles(&["title-a"])], Vec::new()]);
        assert!(matches!(
            folded.hold_for_title("title-a", false),
            LifecyclePlaybackHold::Hold { .. }
        ));
        assert!(matches!(
            folded.hold_for_title("title-b", true),
            LifecyclePlaybackHold::Unknown { .. }
        ));
        assert_eq!(
            folded.hold_for_title("title-b", false),
            LifecyclePlaybackHold::Clear
        );
    }

    #[test]
    fn a_failed_probe_is_unknown_and_never_kept() {
        let folded = LifecyclePlaybackPicture::unknown("playback probe unavailable");
        assert!(matches!(
            folded.hold_for_title("title-a", true),
            LifecyclePlaybackHold::Unknown { .. }
        ));
        assert!(folded.probe_failed());
    }

    // ── Attribution ─────────────────────────────────────────────────────────

    #[test]
    fn a_playing_episode_holds_its_whole_series_through_the_series_mapping() {
        // The episode itself is not mapped; its series item is.
        let snapshot = snapshot(vec![playing(
            "jellyfin",
            vec![episode("jf-ep", Some("jf-series"))],
        )]);
        let evidence = PlaybackEvidence {
            mapped: mapped(&[(
                "jellyfin",
                "jf-series",
                MediaServerPlaybackEntityKind::Title,
                "series-title",
            )]),
            ..PlaybackEvidence::default()
        };
        let folded = picture(&snapshot, &evidence);
        assert!(matches!(
            folded.hold_for_title("series-title", false),
            LifecyclePlaybackHold::Hold { .. }
        ));
        assert_eq!(
            folded.hold_for_title("another-series", true),
            LifecyclePlaybackHold::Clear
        );
    }

    #[test]
    fn a_playing_episode_mapped_on_its_own_reaches_its_series_title() {
        let snapshot = snapshot(vec![playing("jellyfin", vec![episode("jf-ep", None)])]);
        let evidence = PlaybackEvidence {
            mapped: mapped(&[(
                "jellyfin",
                "jf-ep",
                MediaServerPlaybackEntityKind::Episode,
                "episode-1",
            )]),
            episode_titles: [("episode-1".to_string(), "series-title".to_string())]
                .into_iter()
                .collect(),
            ..PlaybackEvidence::default()
        };
        assert_eq!(
            attribute_playback_sessions(&snapshot, &evidence),
            vec![vec![titles(&["series-title"])]]
        );
    }

    #[test]
    fn a_mapping_on_another_connection_does_not_place_the_session() {
        let snapshot = snapshot(vec![playing("plex", vec![movie("item-1", "Alpha")])]);
        let evidence = PlaybackEvidence {
            mapped: mapped(&[(
                "jellyfin",
                "item-1",
                MediaServerPlaybackEntityKind::Title,
                "title-a",
            )]),
            ..PlaybackEvidence::default()
        };
        assert_eq!(
            attribute_playback_sessions(&snapshot, &evidence),
            vec![vec![SessionAttribution::Unattributed]]
        );
    }

    #[test]
    fn a_movie_is_placed_by_its_external_ids() {
        let mut item = ProviderPlayedItem {
            provider_item_id: "px-1".to_string(),
            kind: MediaServerSignalKind::Movie,
            ..ProviderPlayedItem::default()
        };
        item.external_ids
            .insert("tmdb".to_string(), "603".to_string());
        let snapshot = snapshot(vec![playing("plex", vec![PlayingMedia::Video(item)])]);
        let mut evidence = PlaybackEvidence::default();
        evidence.external_ids.insert(
            ("tmdb".to_string(), "603".to_string()),
            vec![("title-a".to_string(), scryer_domain::MediaFacet::Movie)],
        );
        assert_eq!(
            attribute_playback_sessions(&snapshot, &evidence),
            vec![vec![titles(&["title-a"])]]
        );
    }

    #[test]
    fn a_session_is_placed_by_its_file_path() {
        let snapshot = snapshot(vec![playing(
            "plex",
            vec![PlayingMedia::Other {
                provider_item_id: None,
                name: None,
                file_paths: vec!["/srv/movies/a.mkv".to_string()],
            }],
        )]);
        let evidence = PlaybackEvidence {
            file_titles: [(
                ("plex".to_string(), "/srv/movies/a.mkv".to_string()),
                "title-a".to_string(),
            )]
            .into_iter()
            .collect(),
            ..PlaybackEvidence::default()
        };
        assert_eq!(
            attribute_playback_sessions(&snapshot, &evidence),
            vec![vec![titles(&["title-a"])]]
        );
    }

    #[test]
    fn audio_is_not_scryer_media_and_a_foreign_video_is_let_go_only_on_complete_evidence() {
        let mut foreign = ProviderPlayedItem {
            provider_item_id: "px-foreign".to_string(),
            kind: MediaServerSignalKind::Movie,
            file_paths: vec!["/elsewhere/home-video.mkv".to_string()],
            ..ProviderPlayedItem::default()
        };
        foreign
            .external_ids
            .insert("tmdb".to_string(), "999".to_string());
        let snapshot = snapshot(vec![playing(
            "plex",
            vec![PlayingMedia::Audio, PlayingMedia::Video(foreign)],
        )]);
        let complete = PlaybackEvidence {
            library_roots: Some(vec!["/media/movies".to_string()]),
            ..PlaybackEvidence::default()
        };
        assert_eq!(
            attribute_playback_sessions(&snapshot, &complete),
            vec![vec![
                SessionAttribution::NotScryerMedia,
                SessionAttribution::NotScryerMedia
            ]]
        );

        // A read that failed shows nothing to be foreign.
        let incomplete = PlaybackEvidence {
            incomplete: true,
            ..complete
        };
        assert_eq!(
            attribute_playback_sessions(&snapshot, &incomplete),
            vec![vec![
                SessionAttribution::NotScryerMedia,
                SessionAttribution::Unattributed
            ]]
        );
    }

    #[test]
    fn an_item_with_no_ids_or_file_in_a_library_stays_unattributed() {
        let snapshot = snapshot(vec![playing(
            "plex",
            vec![
                movie("px-bare", "Bare"),
                PlayingMedia::Other {
                    provider_item_id: Some("px-clip".to_string()),
                    name: Some("Clip".to_string()),
                    file_paths: vec!["/elsewhere/clip.mkv".to_string()],
                },
            ],
        )]);
        let evidence = PlaybackEvidence {
            library_roots: Some(vec!["/media/movies".to_string()]),
            ..PlaybackEvidence::default()
        };
        assert_eq!(
            attribute_playback_sessions(&snapshot, &evidence),
            vec![vec![
                SessionAttribution::Unattributed,
                SessionAttribution::Unattributed
            ]]
        );
    }

    // ── Queued-import attribution ───────────────────────────────────────────

    #[test]
    fn manual_import_payload_names_its_title() {
        let record = import_record(r#"{"title_id":"title-1","client_type":"sabnzbd"}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-1"));
    }

    #[test]
    fn completed_import_payload_names_its_target_title() {
        let record =
            import_record(r#"{"completed":{"parameters":[]},"target_title_id":"title-2"}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-2"));
    }

    #[test]
    fn legacy_completed_import_payload_alias_is_read() {
        let record = import_record(r#"{"manual_title_id":"title-3"}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-3"));
    }

    #[test]
    fn a_scryer_grab_is_attributed_through_its_client_parameter() {
        let record = import_record(
            r#"{"completed":{"parameters":[["*scryer_category","movies"],["*scryer_title_id","title-4"]]}}"#,
        );
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-4"));
    }

    #[test]
    fn a_legacy_top_level_completed_payload_is_attributed_too() {
        let record = import_record(r#"{"parameters":[["*scryer_title_id","title-5"]]}"#);
        assert_eq!(queued_import_title_id(&record).as_deref(), Some("title-5"));
    }

    #[test]
    fn a_foreign_import_names_no_title() {
        let record = import_record(r#"{"completed":{"parameters":[["*scryer_category","tv"]]}}"#);
        assert_eq!(queued_import_title_id(&record), None);
    }

    #[test]
    fn a_blank_title_id_is_not_an_attribution() {
        let record = import_record(r#"{"title_id":"   "}"#);
        assert_eq!(queued_import_title_id(&record), None);
    }

    #[test]
    fn an_unparseable_payload_names_no_title() {
        let record = import_record("not json");
        assert_eq!(queued_import_title_id(&record), None);
    }

    // ── The acquisition fold ────────────────────────────────────────────────

    #[test]
    fn a_grabbed_scope_is_active() {
        assert_eq!(
            fold_acquisition_activity("title-1", Ok(1), Ok(Vec::new())),
            LifecycleActivityCheck::Active
        );
    }

    #[test]
    fn no_grab_and_no_queued_imports_is_clear() {
        assert_eq!(
            fold_acquisition_activity("title-1", Ok(0), Ok(Vec::new())),
            LifecycleActivityCheck::Clear
        );
    }

    #[test]
    fn a_queued_import_for_this_title_is_active() {
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(0),
                Ok(vec![
                    Some("title-2".to_string()),
                    Some("title-1".to_string())
                ])
            ),
            LifecycleActivityCheck::Active
        );
    }

    #[test]
    fn queued_imports_for_other_titles_are_clear() {
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(0),
                Ok(vec![
                    Some("title-2".to_string()),
                    Some("title-3".to_string())
                ])
            ),
            LifecycleActivityCheck::Clear
        );
    }

    #[test]
    fn an_unattributed_queued_import_is_unknown_not_clear() {
        let folded = fold_acquisition_activity(
            "title-1",
            Ok(0),
            Ok(vec![Some("title-2".to_string()), None]),
        );
        let LifecycleActivityCheck::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains('1'), "{reason}");
    }

    #[test]
    fn an_attributed_match_beats_an_unattributed_peer() {
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(0),
                Ok(vec![None, Some("title-1".to_string())])
            ),
            LifecycleActivityCheck::Active
        );
    }

    #[test]
    fn an_unreadable_scope_table_is_unknown() {
        let folded = fold_acquisition_activity(
            "title-1",
            Err(crate::AppError::Repository("datastore offline".into())),
            Ok(Vec::new()),
        );
        let LifecycleActivityCheck::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains("acquisition scope states"), "{reason}");
    }

    #[test]
    fn an_unreadable_import_queue_is_unknown() {
        let folded = fold_acquisition_activity(
            "title-1",
            Ok(0),
            Err(crate::AppError::Repository("datastore offline".into())),
        );
        let LifecycleActivityCheck::Unknown { reason } = &folded else {
            panic!("expected unknown, got {folded:?}");
        };
        assert!(reason.contains("queued imports"), "{reason}");
    }

    #[test]
    fn a_grab_beats_an_unreadable_import_queue() {
        // The grab is already conclusive; the queue read cannot make it less so.
        assert_eq!(
            fold_acquisition_activity(
                "title-1",
                Ok(3),
                Err(crate::AppError::Repository("datastore offline".into()))
            ),
            LifecycleActivityCheck::Active
        );
    }

    #[test]
    fn activity_check_holds_unless_clear() {
        assert!(LifecycleActivityCheck::Active.holds_destructive_work());
        assert!(
            LifecycleActivityCheck::Unknown {
                reason: "unreadable".to_string()
            }
            .holds_destructive_work()
        );
        assert!(!LifecycleActivityCheck::Clear.holds_destructive_work());
    }
}
