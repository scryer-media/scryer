//! Media-server watch signals (RFC 137 §7.3, WP-M).
//!
//! Two layers are covered here:
//!
//! * the pure identity mapping — movie by TMDB id, episode by series plus
//!   season/episode numbers, and every case that must stay *unmapped* rather
//!   than be guessed onto a subject (RFC identity-mapping rule 4);
//! * the sweep itself — that a participant's failure does not blank their
//!   stored history, that a disabled connection is recorded as disabled, and
//!   that unmapped observations are still written.

use super::*;

use std::collections::BTreeMap;

use crate::media_server_signals::reasons;
use crate::media_server_signals::{
    EpisodeNumberIndex, ScryerLibraryPaths, TitleExternalIdIndex, resolve_episode, resolve_subject,
    resolve_title,
};
use crate::ports::{
    MediaServerConnectionRepository, MediaServerSignalReader, MediaServerSignalRepository,
    MediaServerSignalSource, ProviderPlayedItem, SignalReadFailure, UserExternalAccountRepository,
};
use scryer_domain::{
    AppPermissionMask, ExternalAccountProvider, ExternalAccountStatus, ExternalId, LibraryGrant,
    MediaFacet, MediaServerConnection, MediaServerPlaybackEntityKind, MediaServerPlaybackItem,
    MediaServerProvider, MediaServerSignalKind, MediaServerSignalSyncState, NewUserMediaSignal,
    User, UserExternalAccount, UserMediaSignal,
};

// ── Fixtures ────────────────────────────────────────────────────────────────

pub(super) const CONNECTION_ID: &str = "conn-jellyfin";
const PARTICIPANT_EXTERNAL_ID: &str = "jf-user-alpha";
const PARTICIPANT_SCRYER_ID: &str = "scryer-user-alpha";

pub(super) fn jellyfin_connection(enabled: bool) -> MediaServerConnection {
    named_jellyfin_connection(CONNECTION_ID, enabled)
}

/// The same connection under a caller-chosen id, so a test can stand up more
/// than one server and prove they are judged independently.
pub(super) fn named_jellyfin_connection(id: &str, enabled: bool) -> MediaServerConnection {
    MediaServerConnection {
        id: id.to_string(),
        provider: MediaServerProvider::Jellyfin,
        display_name: "Example Jellyfin".to_string(),
        base_url: "http://jellyfin.example".to_string(),
        external_url: None,
        enabled,
        login_enabled: true,
        linking_enabled: true,
        auto_add_enabled: false,
        default_app_permissions: AppPermissionMask::default(),
        default_library_grants: Vec::new(),
        machine_id: None,
        api_key: Some("api-key".to_string()),
        emby_server_id: None,
        emby_connect_enabled: false,
        path_mappings: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn verified_account() -> UserExternalAccount {
    UserExternalAccount {
        id: "link-1".to_string(),
        user_id: PARTICIPANT_SCRYER_ID.to_string(),
        provider: ExternalAccountProvider::Jellyfin,
        connection_id: CONNECTION_ID.to_string(),
        external_user_id: Some(PARTICIPANT_EXTERNAL_ID.to_string()),
        username: "viewer-one".to_string(),
        display_name: None,
        avatar_url: None,
        status: ExternalAccountStatus::Active,
        verified_at: Some(Utc::now()),
        last_login_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn played_movie(item_id: &str, tmdb: &str) -> ProviderPlayedItem {
    ProviderPlayedItem {
        provider_item_id: item_id.to_string(),
        kind: MediaServerSignalKind::Movie,
        name: Some("Example Feature".to_string()),
        external_ids: BTreeMap::from([("tmdb".to_string(), tmdb.to_string())]),
        series_external_ids: BTreeMap::new(),
        series_provider_item_id: None,
        season_number: None,
        episode_number: None,
        played: true,
        play_count: 2,
        last_played_at: Some(Utc::now()),
        is_favorite: false,
        file_paths: Vec::new(),
    }
}

fn played_episode(
    item_id: &str,
    series_tvdb: &str,
    season: i64,
    episode: i64,
) -> ProviderPlayedItem {
    ProviderPlayedItem {
        provider_item_id: item_id.to_string(),
        kind: MediaServerSignalKind::Episode,
        name: Some("Pilot".to_string()),
        external_ids: BTreeMap::new(),
        series_external_ids: BTreeMap::from([("tvdb".to_string(), series_tvdb.to_string())]),
        series_provider_item_id: Some("jf-series-1".to_string()),
        season_number: Some(season),
        episode_number: Some(episode),
        played: true,
        play_count: 1,
        last_played_at: Some(Utc::now()),
        is_favorite: false,
        file_paths: Vec::new(),
    }
}

fn title_index(entries: Vec<(&str, &str, &str, MediaFacet)>) -> TitleExternalIdIndex {
    let mut index = TitleExternalIdIndex::new();
    for (source, external_id, title_id, facet) in entries {
        index
            .entry((source.to_string(), external_id.to_string()))
            .or_default()
            .push((title_id.to_string(), facet));
    }
    index
}

fn episode_index(entries: Vec<(&str, i64, i64, &str)>) -> EpisodeNumberIndex {
    let mut index = EpisodeNumberIndex::new();
    for (title_id, season, number, episode_id) in entries {
        index
            .entry((title_id.to_string(), season, number))
            .or_default()
            .push(episode_id.to_string());
    }
    index
}

// ── Mapping ─────────────────────────────────────────────────────────────────

#[test]
fn a_movie_maps_through_its_tmdb_id() {
    let index = title_index(vec![("tmdb", "603", "title-movie", MediaFacet::Movie)]);
    let subject = resolve_subject(
        &played_movie("jf-movie-1", "603"),
        &index,
        &EpisodeNumberIndex::new(),
    );

    assert_eq!(subject.title_id.as_deref(), Some("title-movie"));
    assert!(subject.episode_id.is_none());
}

#[test]
fn a_movie_never_maps_onto_a_series_that_shares_the_id() {
    // Facet is part of RFC identity-mapping rule 3, not an afterthought.
    let index = title_index(vec![("tmdb", "603", "title-series", MediaFacet::Series)]);
    let subject = resolve_subject(
        &played_movie("jf-movie-1", "603"),
        &index,
        &EpisodeNumberIndex::new(),
    );

    assert!(!subject.is_mapped());
}

#[test]
fn an_episode_maps_through_its_series_plus_season_and_episode_numbers() {
    let titles = title_index(vec![("tvdb", "999", "title-series", MediaFacet::Series)]);
    let episodes = episode_index(vec![("title-series", 2, 5, "episode-25")]);

    let subject = resolve_subject(&played_episode("jf-ep-1", "999", 2, 5), &titles, &episodes);

    assert_eq!(subject.title_id.as_deref(), Some("title-series"));
    assert_eq!(subject.episode_id.as_deref(), Some("episode-25"));
}

#[test]
fn an_episode_maps_under_the_anime_facet_too() {
    let titles = title_index(vec![("tvdb", "999", "title-anime", MediaFacet::Anime)]);
    let episodes = episode_index(vec![("title-anime", 1, 1, "episode-11")]);

    let subject = resolve_subject(&played_episode("jf-ep-1", "999", 1, 1), &titles, &episodes);

    assert_eq!(subject.episode_id.as_deref(), Some("episode-11"));
}

#[test]
fn an_episode_whose_series_resolves_but_whose_number_does_not_stays_fully_unmapped() {
    // Half-mapping it to the series would be a show-level rollup, which this
    // wave stores nowhere.
    let titles = title_index(vec![("tvdb", "999", "title-series", MediaFacet::Series)]);
    let episodes = episode_index(vec![("title-series", 2, 5, "episode-25")]);

    let subject = resolve_subject(&played_episode("jf-ep-9", "999", 2, 99), &titles, &episodes);

    assert!(subject.title_id.is_none());
    assert!(subject.episode_id.is_none());
}

#[test]
fn an_ambiguous_external_id_resolves_to_nothing() {
    let index = title_index(vec![
        ("tmdb", "603", "title-a", MediaFacet::Movie),
        ("tmdb", "603", "title-b", MediaFacet::Movie),
    ]);

    assert!(resolve_title(&played_movie("jf-movie-1", "603"), &index).is_none());
}

#[test]
fn a_duplicate_row_for_one_title_is_not_ambiguity() {
    // The same title reached through the same id twice is one answer, not two.
    let index = title_index(vec![
        ("tmdb", "603", "title-a", MediaFacet::Movie),
        ("tmdb", "603", "title-a", MediaFacet::Movie),
    ]);

    assert_eq!(
        resolve_title(&played_movie("jf-movie-1", "603"), &index).as_deref(),
        Some("title-a")
    );
}

#[test]
fn an_episode_without_coordinates_has_nothing_to_join_on() {
    let mut item = played_episode("jf-ep-1", "999", 1, 1);
    item.episode_number = None;
    let episodes = episode_index(vec![("title-series", 1, 1, "episode-11")]);

    assert!(resolve_episode(&item, "title-series", &episodes).is_none());
}

#[test]
fn two_episodes_at_the_same_coordinates_stay_unmapped() {
    let episodes = episode_index(vec![
        ("title-series", 1, 1, "episode-a"),
        ("title-series", 1, 1, "episode-b"),
    ]);

    assert!(
        resolve_episode(
            &played_episode("jf-ep-1", "999", 1, 1),
            "title-series",
            &episodes
        )
        .is_none()
    );
}

// ── Doubles for the sweep ───────────────────────────────────────────────────

#[derive(Default)]
pub(super) struct StubSignalConnections {
    pub(super) connections: Vec<MediaServerConnection>,
    pub(super) fail_list: bool,
    /// The library mapping: each connection's server item for a title or
    /// episode.
    pub(super) playback_items: std::sync::Mutex<Vec<MediaServerPlaybackItem>>,
}

#[async_trait]
impl MediaServerConnectionRepository for StubSignalConnections {
    async fn list(
        &self,
        provider: Option<MediaServerProvider>,
    ) -> AppResult<Vec<MediaServerConnection>> {
        if self.fail_list {
            return Err(AppError::Repository("connection list unreadable".into()));
        }
        Ok(self
            .connections
            .iter()
            .filter(|connection| {
                provider
                    .as_ref()
                    .is_none_or(|provider| &connection.provider == provider)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<MediaServerConnection>> {
        Ok(self
            .connections
            .iter()
            .find(|connection| connection.id == id)
            .cloned())
    }

    async fn create(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        Ok(connection)
    }

    async fn update(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        Ok(connection)
    }

    async fn list_playback_items_for_entity(
        &self,
        entity_kind: MediaServerPlaybackEntityKind,
        entity_id: &str,
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        Ok(self
            .playback_items
            .lock()
            .expect("playback items")
            .iter()
            .filter(|item| item.entity_kind == entity_kind && item.entity_id == entity_id)
            .cloned()
            .collect())
    }

    async fn replace_playback_items_for_connection(
        &self,
        _: &str,
        _: Vec<MediaServerPlaybackItem>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn has_external_accounts(&self, _: &str) -> AppResult<bool> {
        Ok(true)
    }

    async fn has_notification_channels(&self, _: &str) -> AppResult<bool> {
        Ok(false)
    }
}

#[derive(Default)]
pub(super) struct StubExternalAccounts {
    pub(super) accounts: Vec<UserExternalAccount>,
}

#[async_trait]
impl UserExternalAccountRepository for StubExternalAccounts {
    async fn create(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
        Ok(account)
    }

    async fn create_or_get_by_provider_identity(
        &self,
        account: UserExternalAccount,
    ) -> AppResult<UserExternalAccount> {
        Ok(account)
    }

    async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserExternalAccount>> {
        Ok(self
            .accounts
            .iter()
            .filter(|account| account.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserExternalAccount>> {
        Ok(self.accounts.iter().find(|a| a.id == id).cloned())
    }

    async fn get_by_provider_identity(
        &self,
        _: ExternalAccountProvider,
        _: &str,
        _: &str,
    ) -> AppResult<Option<UserExternalAccount>> {
        Ok(None)
    }

    async fn get_pending_claim_by_provider_username(
        &self,
        _: ExternalAccountProvider,
        _: &str,
        _: &str,
    ) -> AppResult<Option<UserExternalAccount>> {
        Ok(None)
    }

    async fn list_verified_by_connection(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
    ) -> AppResult<Vec<UserExternalAccount>> {
        Ok(self
            .accounts
            .iter()
            .filter(|account| {
                account.provider == provider
                    && account.connection_id == connection_id
                    && account.status == ExternalAccountStatus::Active
                    && account.verified_at.is_some()
                    && account
                        .external_user_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
            })
            .cloned()
            .collect())
    }

    async fn update(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
        Ok(account)
    }

    async fn create_auto_added_user_with_account(
        &self,
        user: User,
        _: AppPermissionMask,
        _: Vec<LibraryGrant>,
        account: UserExternalAccount,
    ) -> AppResult<(User, UserExternalAccount)> {
        Ok((user, account))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

/// Replays a fixed played-item list per participant, or fails for them.
/// `open_failure` fails the whole connection before any participant is read.
#[derive(Default)]
struct StubSignalSource {
    items: HashMap<String, Vec<ProviderPlayedItem>>,
    /// The audience read's answer; `None` keeps the reader's default refusal.
    audience: Option<Result<Vec<crate::ports::ProviderAudiencePlay>, SignalReadFailure>>,
    failing: HashSet<String>,
    open_failure: Option<SignalReadFailure>,
    opens: AtomicUsize,
    calls: Arc<AtomicUsize>,
}

struct StubSignalReader {
    items: HashMap<String, Vec<ProviderPlayedItem>>,
    audience: Option<Result<Vec<crate::ports::ProviderAudiencePlay>, SignalReadFailure>>,
    failing: HashSet<String>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl MediaServerSignalSource for StubSignalSource {
    async fn open(
        &self,
        _: &MediaServerConnection,
    ) -> Result<Box<dyn MediaServerSignalReader>, SignalReadFailure> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        if let Some(failure) = &self.open_failure {
            return Err(failure.clone());
        }
        Ok(Box::new(StubSignalReader {
            items: self.items.clone(),
            audience: self.audience.clone(),
            failing: self.failing.clone(),
            calls: self.calls.clone(),
        }))
    }
}

#[async_trait]
impl MediaServerSignalReader for StubSignalReader {
    async fn fetch_played_items(
        &self,
        external_user_id: &str,
    ) -> Result<Vec<ProviderPlayedItem>, SignalReadFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.contains(external_user_id) {
            return Err(SignalReadFailure::new(reasons::ACCESS_DENIED, "status 401"));
        }
        Ok(self
            .items
            .get(external_user_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn fetch_audience_plays(
        &self,
    ) -> Result<Vec<crate::ports::ProviderAudiencePlay>, SignalReadFailure> {
        match &self.audience {
            Some(answer) => answer.clone(),
            None => Err(SignalReadFailure::new(
                reasons::SOURCE_UNAVAILABLE,
                "no audience in this fixture",
            )),
        }
    }
}

/// Mirrors the SQL store's replace-generation contract closely enough that a
/// sync bug cannot hide behind a permissive double.
#[derive(Default)]
pub(super) struct InMemorySignalRepo {
    fail_state_reads: std::sync::atomic::AtomicBool,
    /// Stands in for a store failure while removing an audience read marker.
    fail_audience_clear: std::sync::atomic::AtomicBool,
    rows: Mutex<Vec<UserMediaSignal>>,
    states: Mutex<Vec<MediaServerSignalSyncState>>,
    generations: Mutex<HashMap<(String, String), i64>>,
    roster: Mutex<Vec<crate::ports::SignalRosterEntry>>,
    pub(super) audience: Mutex<Vec<crate::ports::AudiencePlay>>,
    pub(super) audience_reads: Mutex<Vec<crate::ports::AudienceRead>>,
}

impl InMemorySignalRepo {
    async fn rows_for(&self, external_user_id: &str) -> Vec<UserMediaSignal> {
        self.rows
            .lock()
            .await
            .iter()
            .filter(|row| row.external_user_id == external_user_id)
            .cloned()
            .collect()
    }

    /// A store whose roster already records `accounts` as read by a sweep,
    /// for fixtures that seed signals directly instead of sweeping.
    pub(super) fn swept(accounts: &[UserExternalAccount]) -> Self {
        let roster = accounts
            .iter()
            .map(|account| crate::ports::SignalRosterEntry {
                connection_id: account.connection_id.clone(),
                external_user_id: account.external_user_id.clone().unwrap_or_default(),
                link: crate::ports::SignalRosterLink {
                    link_id: account.id.clone(),
                    scryer_user_id: account.user_id.clone(),
                    link_verified_at: account.verified_at,
                    sweep_started_at: Utc::now(),
                },
            })
            .collect();
        Self {
            roster: Mutex::new(roster),
            ..Self::default()
        }
    }

    async fn state_for(&self, connection_id: &str) -> Option<MediaServerSignalSyncState> {
        self.states
            .lock()
            .await
            .iter()
            .find(|state| state.connection_id == connection_id)
            .cloned()
    }
}

#[async_trait]
impl MediaServerSignalRepository for InMemorySignalRepo {
    async fn retain_connection_participants(
        &self,
        connection_id: &str,
        participants: &[(String, String)],
    ) -> AppResult<u64> {
        let mut rows = self.rows.lock().await;
        let before = rows.len();
        rows.retain(|row| {
            row.connection_id != connection_id
                || participants.iter().any(|(external, user)| {
                    row.external_user_id == *external
                        && row.scryer_user_id.as_deref() == Some(user.as_str())
                })
        });
        self.roster.lock().await.retain(|entry| {
            entry.connection_id != connection_id
                || participants.iter().any(|(external, user)| {
                    entry.external_user_id == *external && entry.link.scryer_user_id == *user
                })
        });
        Ok((before - rows.len()) as u64)
    }

    async fn replace_participant_signals(
        &self,
        connection_id: &str,
        external_user_id: &str,
        signals: &[NewUserMediaSignal],
        read_through: Option<&crate::ports::SignalRosterLink>,
    ) -> AppResult<u64> {
        if let Some(link) = read_through {
            let mut roster = self.roster.lock().await;
            roster.retain(|entry| {
                !(entry.connection_id == connection_id && entry.link.link_id == link.link_id)
            });
            roster.push(crate::ports::SignalRosterEntry {
                connection_id: connection_id.to_string(),
                external_user_id: external_user_id.to_string(),
                link: link.clone(),
            });
        }
        let key = (connection_id.to_string(), external_user_id.to_string());
        let generation = {
            let mut generations = self.generations.lock().await;
            let next = generations.get(&key).copied().unwrap_or(0) + 1;
            generations.insert(key, next);
            next
        };

        let mut rows = self.rows.lock().await;
        rows.retain(|row| {
            !(row.connection_id == connection_id && row.external_user_id == external_user_id)
        });
        for signal in signals {
            rows.push(UserMediaSignal {
                id: Id::new().0,
                connection_id: connection_id.to_string(),
                provider: signal.provider.clone(),
                external_user_id: external_user_id.to_string(),
                scryer_user_id: signal.scryer_user_id.clone(),
                provider_item_id: signal.provider_item_id.clone(),
                kind: signal.kind,
                scryer_title_id: signal.scryer_title_id.clone(),
                scryer_episode_id: signal.scryer_episode_id.clone(),
                played: signal.played,
                play_count: signal.play_count,
                last_played_at: signal.last_played_at,
                is_favorite: signal.is_favorite,
                observed_at: signal.observed_at,
                sync_generation: generation,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            });
        }
        Ok(rows
            .iter()
            .filter(|row| {
                row.connection_id == connection_id && row.external_user_id == external_user_id
            })
            .count() as u64)
    }

    async fn movie_signals_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<UserMediaSignal>>> {
        Ok(group_by_title(
            &self.rows.lock().await,
            title_ids,
            MediaServerSignalKind::Movie,
        ))
    }

    async fn episode_signals_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<UserMediaSignal>>> {
        Ok(group_by_title(
            &self.rows.lock().await,
            title_ids,
            MediaServerSignalKind::Episode,
        ))
    }

    async fn signal_roster(&self) -> AppResult<Vec<crate::ports::SignalRosterEntry>> {
        Ok(self.roster.lock().await.clone())
    }

    async fn replace_connection_audience(
        &self,
        connection_id: &str,
        plays: &[crate::ports::AudiencePlay],
        sweep_started_at: chrono::DateTime<Utc>,
    ) -> AppResult<u64> {
        let mut audience = self.audience.lock().await;
        audience.retain(|play| play.connection_id != connection_id);
        audience.extend(plays.iter().cloned());
        let mut reads = self.audience_reads.lock().await;
        reads.retain(|read| read.connection_id != connection_id);
        reads.push(crate::ports::AudienceRead {
            connection_id: connection_id.to_string(),
            sweep_started_at,
            read_at: Utc::now(),
            play_count: plays.len() as i64,
        });
        Ok(plays.len() as u64)
    }

    async fn clear_audience_read(&self, connection_id: &str) -> AppResult<()> {
        if self
            .fail_audience_clear
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(AppError::Repository("audience marker write failed".into()));
        }
        self.audience_reads
            .lock()
            .await
            .retain(|read| read.connection_id != connection_id);
        Ok(())
    }

    async fn audience_reads(&self) -> AppResult<Vec<crate::ports::AudienceRead>> {
        Ok(self.audience_reads.lock().await.clone())
    }

    async fn audience_plays_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<crate::ports::AudiencePlay>>> {
        let mut grouped: HashMap<String, Vec<crate::ports::AudiencePlay>> = HashMap::new();
        for play in self.audience.lock().await.iter() {
            if let Some(title_id) = play
                .scryer_title_id
                .as_ref()
                .filter(|title_id| title_ids.contains(title_id))
            {
                grouped
                    .entry(title_id.clone())
                    .or_default()
                    .push(play.clone());
            }
        }
        Ok(grouped)
    }

    async fn signal_sync_states(&self) -> AppResult<Vec<MediaServerSignalSyncState>> {
        if self
            .fail_state_reads
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(AppError::Repository("sync state read failed".into()));
        }
        Ok(self.states.lock().await.clone())
    }

    async fn upsert_signal_sync_state(&self, state: &MediaServerSignalSyncState) -> AppResult<()> {
        let mut states = self.states.lock().await;
        states.retain(|existing| existing.connection_id != state.connection_id);
        states.push(state.clone());
        Ok(())
    }
}

fn group_by_title(
    rows: &[UserMediaSignal],
    title_ids: &[String],
    kind: MediaServerSignalKind,
) -> HashMap<String, Vec<UserMediaSignal>> {
    let mut grouped: HashMap<String, Vec<UserMediaSignal>> = HashMap::new();
    for row in rows {
        let Some(title_id) = row.scryer_title_id.clone() else {
            continue;
        };
        if row.kind == kind && title_ids.contains(&title_id) {
            grouped.entry(title_id).or_default().push(row.clone());
        }
    }
    grouped
}

struct SyncFixture {
    app: AppUseCase,
    signals: Arc<InMemorySignalRepo>,
    source: Arc<StubSignalSource>,
}

fn sync_app(
    connections: Vec<MediaServerConnection>,
    accounts: Vec<UserExternalAccount>,
    source: StubSignalSource,
) -> SyncFixture {
    let (app, _user) = bootstrap();
    let signals = Arc::new(InMemorySignalRepo::default());
    let source = Arc::new(source);
    let app = app.with_test_overrides(|services| {
        services
            .with_media_server_connection_store(Arc::new(StubSignalConnections {
                connections,
                fail_list: false,
                ..Default::default()
            }))
            .with_external_account_store(Arc::new(StubExternalAccounts { accounts }))
            .with_media_server_signal_source(source.clone())
            .with_media_server_signal_store(signals.clone())
    });
    SyncFixture {
        app,
        signals,
        source,
    }
}

/// Seeds a movie title with a TMDB id so the sweep has something to map onto.
async fn seed_movie_title(app: &AppUseCase, title_id: &str, tmdb: &str) {
    let mut title = make_due_hydration_title(title_id, MediaFacet::Movie, 1);
    title.external_ids = vec![ExternalId::new("tmdb".to_string(), tmdb.to_string())];
    app.services
        .catalog
        .titles
        .create_or_get_existing(title)
        .await
        .expect("seed title");
}

// ── The sweep ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_sweep_stores_a_mapped_movie_signal() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(
                PARTICIPANT_EXTERNAL_ID.to_string(),
                vec![played_movie("jf-movie-1", "603")],
            )]),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert_eq!(report.connections_synced, 1);
    assert_eq!(report.participants_synced, 1);
    assert_eq!(report.signals_written, 1);
    assert_eq!(report.signals_unmapped, 0);

    let rows = fixture.signals.rows_for(PARTICIPANT_EXTERNAL_ID).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].scryer_title_id.as_deref(), Some("title-movie"));
    assert_eq!(
        rows[0].scryer_user_id.as_deref(),
        Some(PARTICIPANT_SCRYER_ID)
    );
    assert_eq!(rows[0].kind, MediaServerSignalKind::Movie);
    assert!(rows[0].played);

    let state = fixture
        .signals
        .state_for(CONNECTION_ID)
        .await
        .expect("sync state");
    assert!(state.enabled);
    assert!(state.last_success_at.is_some());
    assert!(state.last_error.is_none());
    assert_eq!(state.participant_count, 1);
    assert_eq!(state.signal_count, 1);
}

#[tokio::test]
async fn an_unmapped_observation_is_retained_with_no_subject() {
    // RFC identity-mapping rule 4: retained, never guessed onto a subject.
    // With no external id at all, nothing proves the movie is not a Scryer
    // title the server failed to identify, so the read is incomplete.
    let mut unidentified = played_movie("jf-movie-unknown", "999999");
    unidentified.external_ids.clear();
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(PARTICIPANT_EXTERNAL_ID.to_string(), vec![unidentified])]),
            ..Default::default()
        },
    );

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert_eq!(report.signals_written, 1);
    assert_eq!(report.signals_unmapped, 1);

    let rows = fixture.signals.rows_for(PARTICIPANT_EXTERNAL_ID).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].scryer_title_id.is_none());
    assert!(rows[0].scryer_episode_id.is_none());
    assert_eq!(rows[0].provider_item_id, "jf-movie-unknown");
    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert!(state.last_error.is_some());
    assert_eq!(
        state.last_error_code.as_deref(),
        Some(reasons::UNMAPPED_PLAYED_MOVIES)
    );
    assert!(state.last_success_at.is_none());
    // A recent successful sweep must not make a later incomplete answer safe.
    let mut state = state;
    state.last_success_at = Some(Utc::now());
    fixture
        .signals
        .upsert_signal_sync_state(&state)
        .await
        .unwrap();
    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert!(matches!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::SIGNAL_SYNC_INCOMPLETE
        )
    ));
}

/// A connection that was turned off and back on has not swept since it came
/// back, so its stored success timestamp describes a picture from before the
/// gap. Reporting that as fresh would hand a rule a watch picture up to the
/// whole freshness window old, and those facts gate destructive actions.
#[tokio::test]
async fn a_re_enabled_connection_reports_never_swept_until_it_sweeps_again() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    let swept = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert!(swept.last_success_at.is_some());
    assert!(
        matches!(
            fixture
                .app
                .maintenance_watch_context()
                .await
                .unwrap()
                .freshness,
            crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
        ),
        "a connection that has swept cleanly is the baseline this test moves away from"
    );

    // What a disable sweep records: the connection's state as of that sweep,
    // with the prior success carried forward. The connection itself is enabled
    // again, but nothing has swept since.
    let mut disabled = swept.clone();
    disabled.enabled = false;
    fixture
        .signals
        .upsert_signal_sync_state(&disabled)
        .await
        .unwrap();

    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert!(
        matches!(
            context.freshness,
            crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
                crate::maintenance_rules::facts::unknown_reason::SIGNAL_SYNC_NEVER_SUCCEEDED
            )
        ),
        "a pre-disable success is not evidence that the re-enabled connection has swept: {:?}",
        context.freshness
    );
}

/// The same store, judged by a gate that sees `accounts`: what the watch gate
/// reads after links changed since the sweep that filled the store.
fn gate_app(accounts: Vec<UserExternalAccount>, signals: Arc<InMemorySignalRepo>) -> AppUseCase {
    let (app, _user) = bootstrap();
    app.with_test_overrides(|services| {
        services
            .with_media_server_connection_store(Arc::new(StubSignalConnections {
                connections: vec![jellyfin_connection(true)],
                fail_list: false,
                ..Default::default()
            }))
            .with_external_account_store(Arc::new(StubExternalAccounts { accounts }))
            .with_media_server_signal_store(signals)
    })
}

fn late_link() -> UserExternalAccount {
    let mut late = verified_account();
    late.id = "link-late".to_string();
    late.user_id = "scryer-user-late".to_string();
    late.external_user_id = Some("jf-user-late".to_string());
    late
}

#[tokio::test]
async fn a_sweep_records_each_link_it_read_in_the_roster() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let roster = fixture.signals.signal_roster().await.unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].connection_id, CONNECTION_ID);
    assert_eq!(roster[0].external_user_id, PARTICIPANT_EXTERNAL_ID);
    assert_eq!(roster[0].link.link_id, "link-1");
    assert_eq!(roster[0].link.scryer_user_id, PARTICIPANT_SCRYER_ID);
}

/// The commit-order race: a link stamped before the sweep started, but
/// written after the sweep listed its participants, was never read. Its
/// timestamps say "swept"; only the roster knows better. Its plays are
/// unknown, so no watch fact is an answer until a sweep reads it.
#[tokio::test]
async fn a_link_the_sweep_did_not_read_is_unobserved_whatever_its_timestamp() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let mut late = late_link();
    late.verified_at = Some(Utc::now() - chrono::Duration::hours(1));
    let context = gate_app(vec![verified_account(), late], fixture.signals.clone())
        .maintenance_watch_context()
        .await
        .unwrap();

    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::PARTICIPANTS_NOT_YET_SWEPT
        )
    );
    assert!(context.linked_user_ids.contains(PARTICIPANT_SCRYER_ID));
    assert!(!context.linked_user_ids.contains("scryer-user-late"));
    assert!(context.unswept_user_ids.contains("scryer-user-late"));
}

/// A login refreshes `verified_at` but reads the same server account, so a
/// link the sweep read stays observed until the next sweep.
#[tokio::test]
async fn a_read_link_stays_observed_after_a_login_refreshes_it() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let mut logged_in = verified_account();
    logged_in.verified_at = Some(Utc::now() + chrono::Duration::hours(1));
    let context = gate_app(vec![logged_in], fixture.signals.clone())
        .maintenance_watch_context()
        .await
        .unwrap();

    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );
    assert!(context.linked_user_ids.contains(PARTICIPANT_SCRYER_ID));
    assert!(context.unswept_user_ids.is_empty());
}

/// The same link pointed at a different server account reads someone else's
/// plays; the roster row describes the old account, so it no longer counts.
#[tokio::test]
async fn a_link_repointed_at_another_server_account_is_unobserved() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let mut repointed = verified_account();
    repointed.external_user_id = Some("jf-user-replacement".to_string());
    let context = gate_app(vec![repointed], fixture.signals.clone())
        .maintenance_watch_context()
        .await
        .unwrap();

    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::PARTICIPANTS_NOT_YET_SWEPT
        )
    );
}

/// A clean sweep state with no roster at all is what an instance looks like
/// right after the upgrade that added the roster: nobody has been read under
/// it yet, so nobody is observed.
#[tokio::test]
async fn a_clean_sweep_state_without_a_roster_observes_nobody() {
    let signals = Arc::new(InMemorySignalRepo::default());
    signals
        .upsert_signal_sync_state(&MediaServerSignalSyncState {
            connection_id: CONNECTION_ID.to_string(),
            provider: MediaServerProvider::Jellyfin,
            enabled: true,
            last_started_at: Some(Utc::now()),
            last_success_at: Some(Utc::now()),
            last_error: None,
            last_error_code: None,
            participant_count: 1,
            signal_count: 0,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();

    let context = gate_app(vec![verified_account()], signals)
        .maintenance_watch_context()
        .await
        .unwrap();

    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::PARTICIPANTS_NOT_YET_SWEPT
        )
    );
    assert!(context.unswept_user_ids.contains(PARTICIPANT_SCRYER_ID));
}

/// A clean sweep over nobody, then links made afterwards: no one is read, and
/// the reason says to wait for a sweep rather than to link an account.
#[tokio::test]
async fn links_made_after_a_sweep_over_nobody_are_not_yet_swept() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        Vec::new(),
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let nobody = gate_app(Vec::new(), fixture.signals.clone())
        .maintenance_watch_context()
        .await
        .unwrap();
    assert_eq!(
        nobody.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::NO_LINKED_PARTICIPANTS
        )
    );

    let context = gate_app(vec![late_link()], fixture.signals.clone())
        .maintenance_watch_context()
        .await
        .unwrap();
    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::PARTICIPANTS_NOT_YET_SWEPT
        )
    );
}

#[tokio::test]
async fn unreadable_prior_sync_state_preserves_the_last_success() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .unwrap();
    let before = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert!(before.last_success_at.is_some());
    fixture
        .signals
        .fail_state_reads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        fixture
            .app
            .run_media_server_signal_sync_job()
            .await
            .is_err()
    );
    let after = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert_eq!(after.last_success_at, before.last_success_at);
    assert_eq!(after.last_started_at, before.last_started_at);
}

#[tokio::test]
async fn a_participant_read_failure_leaves_their_stored_history_alone() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(
                PARTICIPANT_EXTERNAL_ID.to_string(),
                vec![played_movie("jf-movie-1", "603")],
            )]),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("first sweep");
    assert_eq!(
        fixture
            .signals
            .rows_for(PARTICIPANT_EXTERNAL_ID)
            .await
            .len(),
        1
    );

    // Second sweep: the same participant is now unreadable.
    let failing = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            failing: HashSet::from([PARTICIPANT_EXTERNAL_ID.to_string()]),
            ..Default::default()
        },
    );
    let report = failing
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep completes");

    assert_eq!(report.participants_failed, 1);
    assert_eq!(report.connections_failed, 1);
    assert_eq!(report.signals_written, 0);
    // No replace was attempted, so nothing was deleted.
    assert!(
        failing
            .signals
            .rows_for(PARTICIPANT_EXTERNAL_ID)
            .await
            .is_empty()
    );

    let state = failing
        .signals
        .state_for(CONNECTION_ID)
        .await
        .expect("sync state");
    assert!(state.last_error.is_some());
    // Freshness is only claimed for a sweep that read everything.
    assert!(state.last_success_at.is_none());
}

#[tokio::test]
async fn a_disabled_connection_is_recorded_as_disabled_and_never_read() {
    let fixture = sync_app(
        vec![jellyfin_connection(false)],
        vec![verified_account()],
        StubSignalSource::default(),
    );

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert_eq!(report.connections_considered, 1);
    assert_eq!(report.connections_skipped_disabled, 1);
    assert_eq!(report.connections_synced, 0);
    assert_eq!(fixture.source.calls.load(Ordering::SeqCst), 0);

    let state = fixture
        .signals
        .state_for(CONNECTION_ID)
        .await
        .expect("sync state");
    assert!(!state.enabled);
    assert!(state.last_success_at.is_none());
    assert!(state.last_error.is_none());
}

#[tokio::test]
async fn a_connection_with_no_verified_participants_syncs_nothing_and_stays_clean() {
    let mut pending = verified_account();
    pending.status = ExternalAccountStatus::PendingClaim;
    pending.verified_at = None;

    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![pending],
        StubSignalSource::default(),
    );

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert_eq!(report.participants_considered, 0);
    assert_eq!(report.connections_synced, 1);
    assert_eq!(fixture.source.calls.load(Ordering::SeqCst), 0);

    let state = fixture
        .signals
        .state_for(CONNECTION_ID)
        .await
        .expect("sync state");
    assert!(state.last_error.is_none());
    assert!(state.last_success_at.is_some());
}

#[tokio::test]
async fn a_connection_without_a_credential_fails_that_connection_alone() {
    let mut connection = jellyfin_connection(true);
    connection.api_key = None;

    let fixture = sync_app(
        vec![connection],
        vec![verified_account()],
        StubSignalSource::default(),
    );

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep completes");

    assert_eq!(report.connections_failed, 1);
    assert_eq!(fixture.source.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .signals
            .state_for(CONNECTION_ID)
            .await
            .expect("sync state")
            .last_error
            .as_deref(),
        Some("no credential stored")
    );
}

#[tokio::test]
async fn an_assembly_with_no_connections_does_nothing_at_all() {
    let fixture = sync_app(Vec::new(), Vec::new(), StubSignalSource::default());

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert_eq!(report.connections_considered, 0);
    assert_eq!(report.connections_failed, 0);
    assert!(
        fixture
            .signals
            .signal_sync_states()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_successful_sync_retires_unlinked_or_unverified_watchers() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(
                PARTICIPANT_EXTERNAL_ID.into(),
                vec![played_movie("movie", "603")],
            )]),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .unwrap();
    assert_eq!(
        fixture
            .signals
            .rows_for(PARTICIPANT_EXTERNAL_ID)
            .await
            .len(),
        1
    );
    let mut unverified = verified_account();
    unverified.verified_at = None;
    let app = fixture.app.with_test_overrides(|services| {
        services.with_external_account_store(Arc::new(StubExternalAccounts {
            accounts: vec![unverified],
        }))
    });
    let report = app.run_media_server_signal_sync_job().await.unwrap();
    assert_eq!(report.connections_synced, 1);
    assert_eq!(report.participants_considered, 0);
    assert!(
        fixture
            .signals
            .rows_for(PARTICIPANT_EXTERNAL_ID)
            .await
            .is_empty()
    );
    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert_eq!(state.participant_count, 0);
    assert_eq!(state.signal_count, 0);
    assert!(state.last_error.is_none());
}

/// Emby and Plex connections are swept beside Jellyfin's. A connection whose
/// participant read was incomplete gets no clean-sweep time and keeps its
/// reason; a connection that read cleanly gets one, and a favourite flag
/// reported by the adapter is stored with the play.
#[tokio::test]
async fn emby_and_plex_connections_are_swept_and_only_a_clean_one_records_a_clean_sweep() {
    let mut emby = named_jellyfin_connection("conn-emby", true);
    emby.provider = MediaServerProvider::Emby;
    let mut plex = named_jellyfin_connection("conn-plex", true);
    plex.provider = MediaServerProvider::Plex;

    let mut emby_viewer = verified_account();
    emby_viewer.id = "link-emby".to_string();
    emby_viewer.provider = ExternalAccountProvider::Emby;
    emby_viewer.connection_id = "conn-emby".to_string();
    emby_viewer.external_user_id = Some("emby-user-alpha".to_string());
    let mut plex_viewer = verified_account();
    plex_viewer.id = "link-plex".to_string();
    plex_viewer.provider = ExternalAccountProvider::Plex;
    plex_viewer.connection_id = "conn-plex".to_string();
    plex_viewer.external_user_id = Some("2002".to_string());

    let mut favourite = played_movie("emby-movie-1", "603");
    favourite.is_favorite = true;
    let fixture = sync_app(
        vec![emby, plex],
        vec![emby_viewer, plex_viewer],
        StubSignalSource {
            items: HashMap::from([
                ("emby-user-alpha".to_string(), vec![favourite]),
                ("2002".to_string(), vec![played_movie("101", "603")]),
            ]),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;

    // First sweep: both read cleanly.
    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    assert_eq!(report.connections_considered, 2);
    assert_eq!(report.connections_synced, 2);
    let emby_rows = fixture.signals.rows_for("emby-user-alpha").await;
    assert_eq!(emby_rows.len(), 1);
    assert!(emby_rows[0].is_favorite);
    assert_eq!(emby_rows[0].provider, MediaServerProvider::Emby);
    let plex_rows = fixture.signals.rows_for("2002").await;
    assert_eq!(plex_rows.len(), 1);
    assert!(!plex_rows[0].is_favorite);
    let first_plex = fixture
        .signals
        .state_for("conn-plex")
        .await
        .expect("plex state");
    assert!(first_plex.last_success_at.is_some());
    assert!(first_plex.last_error.is_none());
}

#[tokio::test]
async fn an_incomplete_emby_read_leaves_the_connection_without_a_clean_sweep() {
    let mut emby = named_jellyfin_connection("conn-emby", true);
    emby.provider = MediaServerProvider::Emby;
    let mut viewer = verified_account();
    viewer.provider = ExternalAccountProvider::Emby;
    viewer.connection_id = "conn-emby".to_string();
    viewer.external_user_id = Some("emby-user-alpha".to_string());

    let fixture = sync_app(
        vec![emby],
        vec![viewer],
        StubSignalSource {
            failing: HashSet::from(["emby-user-alpha".to_string()]),
            ..Default::default()
        },
    );

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    assert_eq!(report.connections_failed, 1);
    assert_eq!(report.participants_failed, 1);
    let state = fixture
        .signals
        .state_for("conn-emby")
        .await
        .expect("emby state");
    assert!(state.last_success_at.is_none(), "no clean sweep yet");
    assert!(state.last_error.is_some());
    assert_eq!(
        state.last_error_code.as_deref(),
        Some(reasons::ACCESS_DENIED)
    );
    assert_eq!(state.provider, MediaServerProvider::Emby);
}

/// A played movie whose external ids no Scryer title claims is something
/// Scryer does not manage. It is kept, unmapped, and does not hold the sweep
/// incomplete: a whole watched library full of home videos and films outside
/// every Scryer library must not freeze the watch gate.
#[tokio::test]
async fn a_played_movie_whose_ids_match_no_title_is_foreign_and_the_sweep_stays_clean() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(
                PARTICIPANT_EXTERNAL_ID.to_string(),
                vec![
                    played_movie("jf-movie-managed", "603"),
                    played_movie("jf-movie-foreign", "999999"),
                ],
            )]),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    assert_eq!(report.signals_written, 2);
    assert_eq!(report.signals_unmapped, 1);
    assert_eq!(report.connections_synced, 1);

    let rows = fixture.signals.rows_for(PARTICIPANT_EXTERNAL_ID).await;
    let foreign = rows
        .iter()
        .find(|row| row.provider_item_id == "jf-movie-foreign")
        .expect("the foreign movie is retained");
    assert!(foreign.scryer_title_id.is_none());
    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert!(state.last_error.is_none());
    assert!(state.last_error_code.is_none());
    assert!(state.last_success_at.is_some());
}

/// A connection that cannot be opened (no server selected, say) fails before
/// any participant is read: nobody's stored history is touched, and the reason
/// is stored as its code beside the sentence.
#[tokio::test]
async fn a_connection_that_cannot_be_opened_records_its_code_and_reads_nobody() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            open_failure: Some(SignalReadFailure::new(
                reasons::PLEX_SERVER_NOT_SELECTED,
                "no Plex server selected",
            )),
            ..Default::default()
        },
    );

    let report = fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    assert_eq!(report.connections_failed, 1);
    assert_eq!(report.participants_failed, 1);
    assert_eq!(fixture.source.calls.load(Ordering::SeqCst), 0);
    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert_eq!(
        state.last_error_code.as_deref(),
        Some(reasons::PLEX_SERVER_NOT_SELECTED)
    );
    assert!(state.last_success_at.is_none());
}

/// Whatever a provider reads once per connection is read once per sweep, not
/// once per participant.
#[tokio::test]
async fn a_connection_is_opened_once_for_all_of_its_participants() {
    let mut second = verified_account();
    second.id = "link-2".to_string();
    second.user_id = "scryer-user-beta".to_string();
    second.external_user_id = Some("jf-user-beta".to_string());
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account(), second],
        StubSignalSource::default(),
    );

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    assert_eq!(fixture.source.opens.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.source.calls.load(Ordering::SeqCst), 2);
}

/// The settings surface reads a clean sweep older than the freshness window as
/// stale, the same way the watch gate does, and a fresh one as current.
#[test]
fn a_clean_sweep_outside_the_freshness_window_reads_as_stale() {
    let now = Utc::now();
    let state = |hours_ago: i64, enabled: bool| MediaServerSignalSyncState {
        connection_id: CONNECTION_ID.to_string(),
        provider: MediaServerProvider::Jellyfin,
        enabled,
        last_started_at: None,
        last_success_at: Some(now - chrono::Duration::hours(hours_ago)),
        last_error: None,
        last_error_code: None,
        participant_count: 1,
        signal_count: 1,
        updated_at: now,
    };
    let window = crate::maintenance_rules::facts::WATCH_SIGNAL_FRESHNESS_HOURS;
    let status = crate::media_server_signals::MediaServerSignalSyncStatus::at;
    assert!(!status(state(1, true), now).clean_sweep_stale);
    assert!(status(state(window + 1, true), now).clean_sweep_stale);
    // Recorded while the connection was off: not a sweep the gate accepts.
    assert!(status(state(1, false), now).clean_sweep_stale);
    let mut never = state(1, true);
    never.last_success_at = None;
    assert!(!status(never, now).clean_sweep_stale);
}

/// The sweep state sits beside the connection listing as a diagnostic; a store
/// that cannot read it leaves the listing without it instead of failing it.
#[tokio::test]
async fn an_unreadable_sweep_state_is_listed_as_none_rather_than_failing() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource::default(),
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .unwrap();
    let admin = User::new_admin("signal-state-viewer");
    assert_eq!(
        fixture
            .app
            .list_media_server_signal_sync_states(&admin)
            .await
            .unwrap()
            .len(),
        1
    );

    fixture
        .signals
        .fail_state_reads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let statuses = fixture
        .app
        .list_media_server_signal_sync_states(&admin)
        .await
        .expect("an unreadable state does not fail the listing");
    assert!(statuses.is_empty());
}

/// A Scryer library root, as the test app's library store holds it, and a
/// connection whose server sees that root under a different mount.
async fn mapped_root_connection(app: &AppUseCase) -> (String, MediaServerConnection) {
    let root = app
        .services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("libraries")
        .into_iter()
        .flat_map(|library| library.roots)
        .map(|root| root.path)
        .next()
        .expect("the test app has a library root");
    let mut connection = jellyfin_connection(true);
    connection.path_mappings = vec![scryer_domain::MediaServerPathMapping {
        source_path: root.clone(),
        destination_path: "/mnt/server-library".to_string(),
        sort_order: 0,
    }];
    (root, connection)
}

async fn sweep_one_unmatched_movie(file_paths: Vec<String>) -> MediaServerSignalSyncState {
    let mut movie = played_movie("jf-movie-unmatched", "999999");
    movie.file_paths = file_paths;
    let probe = sync_app(Vec::new(), Vec::new(), StubSignalSource::default());
    let (_, connection) = mapped_root_connection(&probe.app).await;
    let fixture = sync_app(
        vec![connection],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(PARTICIPANT_EXTERNAL_ID.to_string(), vec![movie])]),
            ..Default::default()
        },
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    fixture.signals.state_for(CONNECTION_ID).await.unwrap()
}

/// The server identified Scryer's own file as a film Scryer does not have. Its
/// ids match nothing, but its file is in a Scryer library once the server's
/// mount is mapped back, so it may be a watched Scryer title: incomplete.
#[tokio::test]
async fn an_unmatched_movie_whose_file_is_in_a_library_root_keeps_the_sweep_incomplete() {
    let state = sweep_one_unmatched_movie(vec![
        "/mnt/server-library/Example Feature (2001)/feature.mkv".to_string(),
    ])
    .await;
    assert_eq!(
        state.last_error_code.as_deref(),
        Some(reasons::UNMAPPED_PLAYED_MOVIES)
    );
    assert!(state.last_success_at.is_none());
}

/// Ids that match nothing and a file outside every library root: something
/// Scryer does not manage, so the sweep stays clean.
#[tokio::test]
async fn an_unmatched_movie_whose_file_is_outside_every_root_is_foreign() {
    let state =
        sweep_one_unmatched_movie(vec!["/mnt/home-videos/example-clip.mkv".to_string()]).await;
    assert!(state.last_error_code.is_none());
    assert!(state.last_success_at.is_some());
}

/// With no file path reported, the ids alone decide, as before.
#[tokio::test]
async fn an_unmatched_movie_with_no_file_path_falls_back_to_its_ids() {
    let state = sweep_one_unmatched_movie(Vec::new()).await;
    assert!(state.last_error_code.is_none());
    assert!(state.last_success_at.is_some());
}

#[test]
fn library_paths_map_server_paths_back_before_comparing() {
    let mappings = vec![scryer_domain::MediaServerPathMapping {
        source_path: "/srv/scryer/movies".to_string(),
        destination_path: "/mnt/server-library".to_string(),
        sort_order: 0,
    }];
    let library = ScryerLibraryPaths::new(Some(vec!["/srv/scryer/movies".to_string()]), &mappings);
    assert!(library.may_hold("/mnt/server-library/Example Feature/feature.mkv"));
    assert!(library.may_hold("/srv/scryer/movies/Example Feature/feature.mkv"));
    assert!(!library.may_hold("/mnt/server-library-other/feature.mkv"));
    assert!(!library.may_hold("/mnt/home-videos/example-clip.mkv"));
    // Roots that could not be read prove nothing is outside them.
    assert!(ScryerLibraryPaths::new(None, &mappings).may_hold("/anywhere/feature.mkv"));
}

// ── The audience read ───────────────────────────────────────────────────────

fn audience_play(account: &str, item: ProviderPlayedItem) -> crate::ports::ProviderAudiencePlay {
    crate::ports::ProviderAudiencePlay {
        external_account_id: account.to_string(),
        item,
    }
}

#[tokio::test]
async fn a_sweep_stores_every_accounts_mapped_plays_and_marks_the_audience_read() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            audience: Some(Ok(vec![
                // An account nobody linked, and one whose movie is not a
                // Scryer title but carries ids that prove it foreign.
                audience_play("jf-user-unlinked", played_movie("jf-movie-1", "603")),
                audience_play("jf-user-unlinked", played_movie("jf-movie-2", "424242")),
            ])),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let audience = fixture.signals.audience.lock().await.clone();
    assert_eq!(audience.len(), 1, "only the mapped play is stored");
    assert_eq!(audience[0].external_account_id, "jf-user-unlinked");
    assert_eq!(audience[0].scryer_title_id.as_deref(), Some("title-movie"));
    let reads = fixture.signals.audience_reads.lock().await.clone();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].connection_id, CONNECTION_ID);
    // The participant read is unaffected by the audience read.
    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert!(state.last_error.is_none());
}

#[tokio::test]
async fn a_failed_audience_read_keeps_the_rows_but_clears_the_read_marker() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            audience: Some(Err(SignalReadFailure::new(
                reasons::ACCESS_DENIED,
                "status 401",
            ))),
            ..Default::default()
        },
    );
    let earlier = crate::ports::AudiencePlay {
        connection_id: CONNECTION_ID.to_string(),
        external_account_id: "jf-user-unlinked".to_string(),
        provider_item_id: "jf-movie-1".to_string(),
        kind: MediaServerSignalKind::Movie,
        scryer_title_id: Some("title-movie".to_string()),
        scryer_episode_id: None,
        play_count: 1,
        last_played_at: Some(Utc::now()),
    };
    fixture
        .signals
        .replace_connection_audience(CONNECTION_ID, std::slice::from_ref(&earlier), Utc::now())
        .await
        .unwrap();

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert_eq!(fixture.signals.audience.lock().await.clone(), vec![earlier]);
    assert!(fixture.signals.audience_reads.lock().await.is_empty());
    // Only the anonymous facts depend on it: the connection swept cleanly.
    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert!(state.last_error.is_none());
    assert!(state.last_success_at.is_some());
}

#[tokio::test]
async fn an_audience_play_of_an_unidentified_movie_leaves_the_audience_unread() {
    let mut unidentified = played_movie("jf-movie-unknown", "999999");
    unidentified.external_ids.clear();
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            audience: Some(Ok(vec![audience_play("jf-user-unlinked", unidentified)])),
            ..Default::default()
        },
    );

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    assert!(fixture.signals.audience_reads.lock().await.is_empty());
    assert!(fixture.signals.audience.lock().await.is_empty());
}

#[tokio::test]
async fn the_audience_gate_opens_only_when_every_enabled_connection_was_read() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            audience: Some(Ok(Vec::new())),
            ..Default::default()
        },
    );
    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");
    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );
    assert_eq!(
        context.audience,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );

    fixture
        .signals
        .clear_audience_read(CONNECTION_ID)
        .await
        .unwrap();
    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );
    assert_eq!(
        context.audience,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::AUDIENCE_NOT_READ
        )
    );
}

/// The audience is every account on the server, so it is read even when no
/// Scryer user is linked: the anonymous facts stand without one.
#[tokio::test]
async fn a_sweep_with_nobody_linked_still_reads_the_audience() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        Vec::new(),
        StubSignalSource {
            audience: Some(Ok(vec![audience_play(
                "jf-user-unlinked",
                played_movie("jf-movie-1", "603"),
            )])),
            ..Default::default()
        },
    );
    seed_movie_title(&fixture.app, "title-movie", "603").await;

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let audience = fixture.signals.audience.lock().await.clone();
    assert_eq!(audience.len(), 1);
    assert_eq!(audience[0].external_account_id, "jf-user-unlinked");
    assert_eq!(audience[0].scryer_title_id.as_deref(), Some("title-movie"));
    assert_eq!(fixture.signals.audience_reads.lock().await.len(), 1);
    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert_eq!(
        context.audience,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );
    assert_eq!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Unavailable(
            crate::maintenance_rules::facts::unknown_reason::NO_LINKED_PARTICIPANTS
        )
    );
}

/// A failed audience read whose marker cannot be removed would leave the
/// last good read standing for it, so the sweep records the connection as
/// failed and every watch fact holds.
#[tokio::test]
async fn an_audience_marker_that_cannot_be_cleared_fails_the_sweep() {
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            audience: Some(Err(SignalReadFailure::new(
                reasons::REQUEST_FAILED,
                "connection reset",
            ))),
            ..Default::default()
        },
    );
    // An earlier good read left its marker.
    fixture
        .signals
        .replace_connection_audience(CONNECTION_ID, &[], Utc::now())
        .await
        .unwrap();
    fixture
        .signals
        .fail_audience_clear
        .store(true, std::sync::atomic::Ordering::SeqCst);

    fixture
        .app
        .run_media_server_signal_sync_job()
        .await
        .expect("sweep");

    let state = fixture.signals.state_for(CONNECTION_ID).await.unwrap();
    assert_eq!(
        state.last_error_code.as_deref(),
        Some(reasons::STORE_FAILED)
    );
    let context = fixture.app.maintenance_watch_context().await.unwrap();
    assert_ne!(
        context.freshness,
        crate::maintenance_rules::facts::WatchSignalFreshness::Fresh
    );
}
