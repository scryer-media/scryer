//! Store round-trips for the media-server watch-signal tables (RFC 137 §7.3).
//!
//! The invariant under test is the generation swap: a sweep's rows survive, and
//! anything the sweep did not report disappears. That behaviour is the only
//! reason "no longer played" is expressible at all, so it is asserted from both
//! ends — the item that stays keeps its identity, and the item that vanishes is
//! actually gone rather than merely stale.

use super::*;
use scryer_application::MediaServerSignalRepository;
use scryer_domain::{
    AppPermissionMask, MediaServerProvider, MediaServerSignalKind, MediaServerSignalSyncState,
    NewUserMediaSignal,
};

const CONNECTION_ID: &str = "conn-jellyfin";
const PARTICIPANT: &str = "jf-user-alpha";

fn signal_store(services: &SqliteServices) -> crate::MediaServerSignalStore {
    crate::MediaServerSignalStore::new(services.datastore())
}

/// Signals cascade from a media-server connection, so every test needs one.
async fn seed_connection(services: &SqliteServices) {
    use scryer_application::MediaServerConnectionRepository;
    let now = Utc::now();
    crate::MediaServerConnectionStore::new(services.datastore(), services.encryption_key_state())
        .create(scryer_domain::MediaServerConnection {
            id: CONNECTION_ID.to_string(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Example Jellyfin".to_string(),
            base_url: "http://jellyfin.example".to_string(),
            external_url: None,
            enabled: true,
            login_enabled: true,
            linking_enabled: true,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: None,
            api_key: Some("api-key".to_string()),
            emby_server_id: None,
            emby_connect_enabled: false,
            path_mappings: Vec::new(),
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("seed media server connection");
}

fn movie_signal(item_id: &str, title_id: Option<&str>, play_count: i64) -> NewUserMediaSignal {
    NewUserMediaSignal {
        provider: MediaServerProvider::Jellyfin,
        scryer_user_id: Some("scryer-user-alpha".to_string()),
        provider_item_id: item_id.to_string(),
        kind: MediaServerSignalKind::Movie,
        scryer_title_id: title_id.map(str::to_string),
        scryer_episode_id: None,
        played: true,
        play_count,
        last_played_at: Some(Utc::now()),
        observed_at: Utc::now(),
    }
}

fn episode_signal(item_id: &str, title_id: &str, episode_id: &str) -> NewUserMediaSignal {
    NewUserMediaSignal {
        provider: MediaServerProvider::Jellyfin,
        scryer_user_id: Some("scryer-user-alpha".to_string()),
        provider_item_id: item_id.to_string(),
        kind: MediaServerSignalKind::Episode,
        scryer_title_id: Some(title_id.to_string()),
        scryer_episode_id: Some(episode_id.to_string()),
        played: true,
        play_count: 1,
        last_played_at: Some(Utc::now()),
        observed_at: Utc::now(),
    }
}

#[tokio::test]
async fn signals_round_trip_with_their_subject_and_play_facts() {
    let (services, db) = temp_services("scryer_media_server_signals_round_trip").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    let written = store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-1", Some("title-movie"), 3)],
        )
        .await
        .expect("first sweep");
    assert_eq!(written, 1);

    let grouped = store
        .movie_signals_for_titles(&["title-movie".to_string()])
        .await
        .expect("read movie signals");
    let signals = grouped.get("title-movie").expect("grouped under its title");
    assert_eq!(signals.len(), 1);
    let signal = &signals[0];
    assert_eq!(signal.connection_id, CONNECTION_ID);
    assert_eq!(signal.external_user_id, PARTICIPANT);
    assert_eq!(signal.provider, MediaServerProvider::Jellyfin);
    assert_eq!(signal.kind, MediaServerSignalKind::Movie);
    assert_eq!(signal.provider_item_id, "jf-movie-1");
    assert_eq!(signal.play_count, 3);
    assert!(signal.played);
    assert!(signal.last_played_at.is_some());
    assert!(signal.scryer_episode_id.is_none());
    assert_eq!(signal.sync_generation, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_second_sweep_advances_the_generation_and_drops_what_it_did_not_report() {
    let (services, db) = temp_services("scryer_media_server_signals_generation").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[
                movie_signal("jf-movie-1", Some("title-movie"), 1),
                movie_signal("jf-movie-2", Some("title-other"), 1),
            ],
        )
        .await
        .expect("first sweep");

    let first_id = store
        .movie_signals_for_titles(&["title-movie".to_string()])
        .await
        .expect("read")
        .get("title-movie")
        .expect("present")[0]
        .id
        .clone();

    // The second sweep no longer reports jf-movie-2: the person unplayed it.
    let written = store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-1", Some("title-movie"), 5)],
        )
        .await
        .expect("second sweep");
    assert_eq!(written, 1);

    let surviving = store
        .movie_signals_for_titles(&["title-movie".to_string(), "title-other".to_string()])
        .await
        .expect("read");
    // Gone entirely, not left behind at an older generation.
    assert!(!surviving.contains_key("title-other"));

    let kept = &surviving.get("title-movie").expect("still played")[0];
    assert_eq!(kept.sync_generation, 2);
    assert_eq!(kept.play_count, 5, "the surviving row was updated in place");
    assert_eq!(
        kept.id, first_id,
        "an upsert keeps the row's identity across sweeps"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn an_empty_sweep_clears_that_participant_and_leaves_others_alone() {
    let (services, db) = temp_services("scryer_media_server_signals_empty_sweep").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-1", Some("title-movie"), 1)],
        )
        .await
        .expect("seed participant one");
    store
        .replace_participant_signals(
            CONNECTION_ID,
            "jf-user-beta",
            &[movie_signal("jf-movie-1", Some("title-movie"), 1)],
        )
        .await
        .expect("seed participant two");

    let remaining = store
        .replace_participant_signals(CONNECTION_ID, PARTICIPANT, &[])
        .await
        .expect("empty sweep is a real write");
    assert_eq!(remaining, 0);

    let grouped = store
        .movie_signals_for_titles(&["title-movie".to_string()])
        .await
        .expect("read");
    let signals = grouped.get("title-movie").expect("the other participant");
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].external_user_id, "jf-user-beta");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn movie_and_episode_reads_are_grouped_by_owning_title_and_never_mix_kinds() {
    let (services, db) = temp_services("scryer_media_server_signals_grouping").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[
                movie_signal("jf-movie-1", Some("title-movie"), 1),
                episode_signal("jf-ep-1", "title-series", "episode-1"),
                episode_signal("jf-ep-2", "title-series", "episode-2"),
            ],
        )
        .await
        .expect("sweep");

    let title_ids = vec!["title-movie".to_string(), "title-series".to_string()];

    let movies = store
        .movie_signals_for_titles(&title_ids)
        .await
        .expect("movie read");
    assert_eq!(movies.len(), 1);
    assert_eq!(movies.get("title-movie").expect("movie bucket").len(), 1);
    assert!(!movies.contains_key("title-series"));

    let episodes = store
        .episode_signals_for_titles(&title_ids)
        .await
        .expect("episode read");
    assert_eq!(episodes.len(), 1);
    let series = episodes.get("title-series").expect("series bucket");
    assert_eq!(series.len(), 2);
    assert!(
        series
            .iter()
            .all(|signal| signal.kind == MediaServerSignalKind::Episode)
    );
    assert!(
        series
            .iter()
            .all(|signal| signal.scryer_episode_id.is_some())
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn an_unmapped_observation_is_stored_and_stays_out_of_every_title_read() {
    let (services, db) = temp_services("scryer_media_server_signals_unmapped").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    let written = store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-unknown", None, 1)],
        )
        .await
        .expect("sweep");
    // Retained (RFC identity-mapping rule 4) …
    assert_eq!(written, 1);
    // … but attributed to nothing, so no subject read can pick it up.
    assert!(
        store
            .movie_signals_for_titles(&["title-movie".to_string()])
            .await
            .expect("read")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_title_read_with_no_ids_asks_the_database_nothing() {
    let (services, db) = temp_services("scryer_media_server_signals_empty_read").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    assert!(
        store
            .movie_signals_for_titles(&[])
            .await
            .expect("read")
            .is_empty()
    );
    assert!(
        store
            .episode_signals_for_titles(&[])
            .await
            .expect("read")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sync_state_upserts_in_place_and_keeps_its_error_until_the_next_success() {
    let (services, db) = temp_services("scryer_media_server_signal_sync_state").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    let failed_at = Utc::now();
    store
        .upsert_signal_sync_state(&MediaServerSignalSyncState {
            connection_id: CONNECTION_ID.to_string(),
            provider: MediaServerProvider::Jellyfin,
            enabled: true,
            last_started_at: Some(failed_at),
            last_success_at: None,
            last_error: Some("status 401".to_string()),
            participant_count: 2,
            signal_count: 0,
            updated_at: failed_at,
        })
        .await
        .expect("record the failure");

    let states = store.signal_sync_states().await.expect("read states");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].last_error.as_deref(), Some("status 401"));
    assert!(states[0].last_success_at.is_none());
    assert!(states[0].enabled);
    assert_eq!(states[0].participant_count, 2);

    let succeeded_at = Utc::now();
    store
        .upsert_signal_sync_state(&MediaServerSignalSyncState {
            connection_id: CONNECTION_ID.to_string(),
            provider: MediaServerProvider::Jellyfin,
            enabled: true,
            last_started_at: Some(succeeded_at),
            last_success_at: Some(succeeded_at),
            last_error: None,
            participant_count: 2,
            signal_count: 7,
            updated_at: succeeded_at,
        })
        .await
        .expect("record the success");

    let states = store.signal_sync_states().await.expect("read states");
    // One row per connection: the upsert replaced, it did not accumulate.
    assert_eq!(states.len(), 1);
    assert!(states[0].last_error.is_none());
    assert!(states[0].last_success_at.is_some());
    assert_eq!(states[0].signal_count, 7);

    let _ = std::fs::remove_file(db);
}
