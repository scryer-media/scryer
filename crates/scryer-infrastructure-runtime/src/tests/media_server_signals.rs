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
        // Only the movie fixture is a favourite, so a round trip proves the
        // flag is stored rather than defaulted.
        is_favorite: true,
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
        is_favorite: false,
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
            None,
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
    assert!(signal.is_favorite);
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
            None,
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
            None,
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
            None,
        )
        .await
        .expect("seed participant one");
    store
        .replace_participant_signals(
            CONNECTION_ID,
            "jf-user-beta",
            &[movie_signal("jf-movie-1", Some("title-movie"), 1)],
            None,
        )
        .await
        .expect("seed participant two");

    let remaining = store
        .replace_participant_signals(CONNECTION_ID, PARTICIPANT, &[], None)
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
            None,
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
            None,
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
            last_error_code: Some("WATCH_SYNC_ACCESS_DENIED".to_string()),
            participant_count: 2,
            signal_count: 0,
            updated_at: failed_at,
        })
        .await
        .expect("record the failure");

    let states = store.signal_sync_states().await.expect("read states");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].last_error.as_deref(), Some("status 401"));
    assert_eq!(
        states[0].last_error_code.as_deref(),
        Some("WATCH_SYNC_ACCESS_DENIED")
    );
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
            last_error_code: None,
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
    assert!(states[0].last_error_code.is_none());
    assert!(states[0].last_success_at.is_some());
    assert_eq!(states[0].signal_count, 7);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn retired_participant_cleanup_preserves_only_exact_current_account_pairs() {
    let (services, db) = temp_services("retired_signal_participants").await;
    seed_connection(&services).await;
    let store = signal_store(&services);
    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("kept", Some("movie"), 1)],
            None,
        )
        .await
        .unwrap();
    store
        .replace_participant_signals(
            CONNECTION_ID,
            "retired",
            &[movie_signal("removed", Some("movie"), 1)],
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .retain_connection_participants(
                CONNECTION_ID,
                &[(PARTICIPANT.into(), "scryer-user-alpha".into())]
            )
            .await
            .unwrap(),
        1
    );
    let rows = store
        .movie_signals_for_titles(&["movie".into()])
        .await
        .unwrap();
    assert_eq!(rows["movie"].len(), 1);
    assert_eq!(rows["movie"][0].provider_item_id, "kept");
    assert_eq!(
        store
            .retain_connection_participants(
                CONNECTION_ID,
                &[(PARTICIPANT.into(), "different-scryer-user".into())]
            )
            .await
            .unwrap(),
        1
    );
    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("empty-participants", Some("movie"), 1)],
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .retain_connection_participants(CONNECTION_ID, &[])
            .await
            .unwrap(),
        1
    );
    assert!(
        store
            .movie_signals_for_titles(&["movie".into()])
            .await
            .unwrap()
            .is_empty()
    );
    services.pool().close().await;
    let _ = std::fs::remove_file(db);
}

fn roster_link(link_id: &str) -> scryer_application::SignalRosterLink {
    scryer_application::SignalRosterLink {
        link_id: link_id.to_string(),
        scryer_user_id: "scryer-user-alpha".to_string(),
        link_verified_at: Some(Utc::now()),
        sweep_started_at: Utc::now(),
    }
}

#[tokio::test]
async fn a_swap_records_the_link_it_read_through_in_the_roster() {
    let (services, db) = temp_services("scryer_media_server_signal_roster_write").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-1", Some("title-movie"), 1)],
            Some(&roster_link("link-alpha")),
        )
        .await
        .expect("swap with roster");
    // A later read through the same link replaces its row rather than adding
    // a second one.
    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[],
            Some(&roster_link("link-alpha")),
        )
        .await
        .expect("second swap");

    let roster = store.signal_roster().await.expect("read roster");
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].connection_id, CONNECTION_ID);
    assert_eq!(roster[0].external_user_id, PARTICIPANT);
    assert_eq!(roster[0].link.link_id, "link-alpha");
    assert_eq!(roster[0].link.scryer_user_id, "scryer-user-alpha");
    assert!(roster[0].link.link_verified_at.is_some());

    let _ = std::fs::remove_file(db);
}

/// The roster row and the signal swap are one unit of work: when the roster
/// write is refused, the swap it would have vouched for is rolled back too, so
/// the store never holds a read the roster does not name, nor the reverse.
#[tokio::test]
async fn a_refused_roster_write_rolls_back_the_signal_swap() {
    let (services, db) = temp_services("scryer_media_server_signal_roster_atomic").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-kept", Some("title-movie"), 1)],
            Some(&roster_link("link-alpha")),
        )
        .await
        .expect("first swap");

    // An empty link id breaks the roster's CHECK constraint, after the swap
    // has already upserted and deleted inside the same transaction.
    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-new", Some("title-movie"), 1)],
            Some(&roster_link("")),
        )
        .await
        .expect_err("the roster refuses an empty link id");

    let rows = store
        .movie_signals_for_titles(&["title-movie".to_string()])
        .await
        .expect("read signals");
    let ids: Vec<_> = rows["title-movie"]
        .iter()
        .map(|row| row.provider_item_id.as_str())
        .collect();
    assert_eq!(ids, vec!["jf-movie-kept"], "the refused swap left no trace");
    let roster = store.signal_roster().await.expect("read roster");
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].link.link_id, "link-alpha");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn retiring_an_identity_removes_its_roster_entry_with_its_signals() {
    let (services, db) = temp_services("scryer_media_server_signal_roster_retire").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_participant_signals(
            CONNECTION_ID,
            PARTICIPANT,
            &[movie_signal("jf-movie-1", Some("title-movie"), 1)],
            Some(&roster_link("link-alpha")),
        )
        .await
        .expect("kept participant");
    let mut retired = roster_link("link-retired");
    retired.scryer_user_id = "scryer-user-retired".to_string();
    store
        .replace_participant_signals(CONNECTION_ID, "jf-user-retired", &[], Some(&retired))
        .await
        .expect("retired participant");

    store
        .retain_connection_participants(
            CONNECTION_ID,
            &[(PARTICIPANT.into(), "scryer-user-alpha".into())],
        )
        .await
        .expect("retain");

    let roster = store.signal_roster().await.expect("read roster");
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].link.link_id, "link-alpha");

    let _ = std::fs::remove_file(db);
}

fn audience_play(
    account: &str,
    item_id: &str,
    title_id: Option<&str>,
    play_count: i64,
) -> scryer_application::AudiencePlay {
    scryer_application::AudiencePlay {
        connection_id: CONNECTION_ID.to_string(),
        external_account_id: account.to_string(),
        provider_item_id: item_id.to_string(),
        kind: MediaServerSignalKind::Movie,
        scryer_title_id: title_id.map(str::to_string),
        scryer_episode_id: None,
        play_count,
        last_played_at: Some(Utc::now()),
    }
}

#[tokio::test]
async fn an_audience_read_replaces_the_connections_plays_and_marks_it_read() {
    let (services, db) = temp_services("scryer_media_server_audience_round_trip").await;
    seed_connection(&services).await;
    let store = signal_store(&services);
    let started = Utc::now();

    let written = store
        .replace_connection_audience(
            CONNECTION_ID,
            &[
                audience_play("account-a", "jf-movie-1", Some("title-movie"), 1),
                audience_play("account-b", "jf-movie-1", Some("title-movie"), 2),
                audience_play("account-b", "jf-movie-2", Some("title-other"), 1),
                // The same account and item twice: the last entry is kept.
                audience_play("account-a", "jf-movie-1", Some("title-movie"), 5),
            ],
            started,
        )
        .await
        .expect("first audience read");
    assert_eq!(written, 3);

    let reads = store.audience_reads().await.expect("reads");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].connection_id, CONNECTION_ID);
    assert_eq!(reads[0].play_count, 3);
    assert_eq!(
        reads[0].sweep_started_at.timestamp_micros(),
        started.timestamp_micros()
    );

    let by_title = store
        .audience_plays_for_titles(&["title-movie".to_string()])
        .await
        .expect("plays by title");
    assert_eq!(by_title.len(), 1, "only the titles asked for are read");
    let mut plays = by_title.get("title-movie").expect("grouped").clone();
    plays.sort_by(|a, b| a.external_account_id.cmp(&b.external_account_id));
    let counts = plays
        .iter()
        .map(|play| (play.external_account_id.as_str(), play.play_count))
        .collect::<Vec<_>>();
    assert_eq!(counts, vec![("account-a", 5), ("account-b", 2)]);
    assert!(plays.iter().all(|play| play.last_played_at.is_some()));

    // A later read replaces every row, including ones it did not report.
    let written = store
        .replace_connection_audience(
            CONNECTION_ID,
            &[audience_play(
                "account-c",
                "jf-movie-2",
                Some("title-other"),
                1,
            )],
            Utc::now(),
        )
        .await
        .expect("second audience read");
    assert_eq!(written, 1);
    assert!(
        store
            .audience_plays_for_titles(&["title-movie".to_string()])
            .await
            .expect("read")
            .is_empty()
    );
    let other = store
        .audience_plays_for_titles(&["title-other".to_string()])
        .await
        .expect("read");
    assert_eq!(other.get("title-other").map(Vec::len), Some(1));

    // A read that found nobody's plays is still a read.
    let written = store
        .replace_connection_audience(CONNECTION_ID, &[], Utc::now())
        .await
        .expect("an empty audience read");
    assert_eq!(written, 0);
    assert_eq!(
        store.audience_reads().await.expect("reads")[0].play_count,
        0
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn clearing_an_audience_read_keeps_the_plays_but_drops_the_marker() {
    let (services, db) = temp_services("scryer_media_server_audience_clear").await;
    seed_connection(&services).await;
    let store = signal_store(&services);

    store
        .replace_connection_audience(
            CONNECTION_ID,
            &[audience_play(
                "account-a",
                "jf-movie-1",
                Some("title-movie"),
                1,
            )],
            Utc::now(),
        )
        .await
        .expect("audience read");
    store
        .clear_audience_read(CONNECTION_ID)
        .await
        .expect("clear the marker");

    assert!(store.audience_reads().await.expect("reads").is_empty());
    assert_eq!(
        store
            .audience_plays_for_titles(&["title-movie".to_string()])
            .await
            .expect("read")
            .get("title-movie")
            .map(Vec::len),
        Some(1),
        "the plays stay; only the marker that vouches for them goes"
    );
    assert!(
        store
            .audience_plays_for_titles(&[])
            .await
            .expect("an empty read")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}
