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

use crate::media_server_signals::{
    EpisodeNumberIndex, TitleExternalIdIndex, resolve_episode, resolve_subject, resolve_title,
};
use crate::ports::{
    MediaServerConnectionRepository, MediaServerSignalRepository, MediaServerSignalSource,
    ProviderPlayedItem, UserExternalAccountRepository,
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
        _: MediaServerPlaybackEntityKind,
        _: &str,
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        Ok(Vec::new())
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
#[derive(Default)]
struct StubSignalSource {
    items: HashMap<String, Vec<ProviderPlayedItem>>,
    failing: HashSet<String>,
    calls: AtomicUsize,
}

#[async_trait]
impl MediaServerSignalSource for StubSignalSource {
    async fn fetch_played_items(
        &self,
        _: &MediaServerConnection,
        external_user_id: &str,
    ) -> AppResult<Vec<ProviderPlayedItem>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.contains(external_user_id) {
            return Err(AppError::Repository("status 401".into()));
        }
        Ok(self
            .items
            .get(external_user_id)
            .cloned()
            .unwrap_or_default())
    }
}

/// Mirrors the SQL store's replace-generation contract closely enough that a
/// sync bug cannot hide behind a permissive double.
#[derive(Default)]
pub(super) struct InMemorySignalRepo {
    rows: Mutex<Vec<UserMediaSignal>>,
    states: Mutex<Vec<MediaServerSignalSyncState>>,
    generations: Mutex<HashMap<(String, String), i64>>,
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
    async fn replace_participant_signals(
        &self,
        connection_id: &str,
        external_user_id: &str,
        signals: &[NewUserMediaSignal],
    ) -> AppResult<u64> {
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

    async fn signal_sync_states(&self) -> AppResult<Vec<MediaServerSignalSyncState>> {
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
    title.external_ids = vec![ExternalId {
        source: "tmdb".to_string(),
        value: tmdb.to_string(),
    }];
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
    let fixture = sync_app(
        vec![jellyfin_connection(true)],
        vec![verified_account()],
        StubSignalSource {
            items: HashMap::from([(
                PARTICIPANT_EXTERNAL_ID.to_string(),
                vec![played_movie("jf-movie-unknown", "999999")],
            )]),
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
