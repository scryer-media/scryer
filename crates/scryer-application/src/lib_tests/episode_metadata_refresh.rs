use super::*;

fn season_one() -> Vec<SeasonMetadata> {
    vec![SeasonMetadata {
        tvdb_id: 880_001,
        tmdb_id: None,
        number: 1,
        label: "Season 1".into(),
        episode_type: "official".into(),
    }]
}

fn full_episode() -> EpisodeMetadata {
    EpisodeMetadata {
        tvdb_id: 880_101,
        tmdb_id: Some(990_101),
        episode_number: 1,
        name: "The Lantern Wakes".into(),
        aired: "2026-02-01".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: "A lantern is lit.".into(),
        absolute_number: "1".into(),
        contiguous_absolute_number: None,
        season_number: 1,
        image_url: String::new(),
    }
}

async fn refresh_fixture(
    facet: MediaFacet,
    tags: Vec<String>,
) -> (AppUseCase, Arc<MockShowRepo>, Title) {
    let (app, user) = bootstrap();
    let shows = Arc::new(MockShowRepo::default());
    let app = app.with_test_overrides(|services| services.with_shows(shows.clone()));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Lantern Verge".into(),
                facet,
                monitored: true,
                tags,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    (app, shows, title)
}

async fn stored_episode(shows: &MockShowRepo) -> Episode {
    let episodes = shows.episodes.lock().await;
    assert_eq!(episodes.len(), 1, "one episode row");
    episodes[0].clone()
}

#[tokio::test]
async fn an_unchanged_refresh_sends_no_episode_update() {
    let (app, shows, title) = refresh_fixture(MediaFacet::Series, vec![]).await;
    let episodes = vec![full_episode()];

    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;
    shows.episode_updates.lock().await.clear();
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    assert!(shows.episode_updates.lock().await.is_empty());
}

/// Upstream is authoritative on refresh: a field it stops sending is cleared,
/// in exactly one write, and the next identical refresh writes nothing.
#[tokio::test]
async fn a_field_upstream_stops_sending_is_cleared_once() {
    type Drop = fn(&mut EpisodeMetadata);
    type Stored = fn(&Episode) -> Option<String>;
    let cases: [(&str, Drop, Stored); 6] = [
        ("title", |ep| ep.name.clear(), |ep| ep.title.clone()),
        (
            "overview",
            |ep| ep.overview.clear(),
            |ep| ep.overview.clone(),
        ),
        ("tvdb id", |ep| ep.tvdb_id = 0, |ep| ep.tvdb_id.clone()),
        ("tmdb id", |ep| ep.tmdb_id = None, |ep| ep.tmdb_id.clone()),
        ("air date", |ep| ep.aired.clear(), |ep| ep.air_date.clone()),
        (
            "absolute number",
            |ep| ep.absolute_number.clear(),
            |ep| ep.absolute_number.clone(),
        ),
    ];

    for (field, drop_field, stored_value) in cases {
        let (app, shows, title) = refresh_fixture(MediaFacet::Series, vec![]).await;
        let mut episodes = vec![full_episode()];
        app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
            .await;
        assert!(
            stored_value(&stored_episode(&shows).await).is_some(),
            "{field} stored on first hydration"
        );
        shows.episode_updates.lock().await.clear();

        drop_field(&mut episodes[0]);
        app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
            .await;
        let updates = shows.episode_updates.lock().await.clone();
        assert_eq!(updates.len(), 1, "{field}: one write clears it");
        assert!(
            updates[0].has_changes(),
            "{field}: the write carries a change"
        );
        assert_eq!(
            stored_value(&stored_episode(&shows).await),
            None,
            "{field} cleared"
        );

        shows.episode_updates.lock().await.clear();
        app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
            .await;
        assert!(
            shows.episode_updates.lock().await.is_empty(),
            "{field}: an identical refresh writes nothing"
        );
    }
}

#[tokio::test]
async fn a_cleared_title_also_clears_the_episode_label() {
    let (app, shows, title) = refresh_fixture(MediaFacet::Series, vec![]).await;
    let mut episodes = vec![full_episode()];
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    episodes[0].name.clear();
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    let stored = stored_episode(&shows).await;
    assert_eq!(stored.title, None);
    assert_eq!(stored.episode_label, None);
}

#[tokio::test]
async fn a_raw_absolute_number_correction_reaches_an_existing_episode() {
    let (app, shows, title) = refresh_fixture(MediaFacet::Anime, vec![]).await;
    let mut episodes = vec![EpisodeMetadata {
        episode_number: 2,
        absolute_number: "3".into(),
        contiguous_absolute_number: Some(2),
        ..full_episode()
    }];
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    episodes[0].absolute_number = "2".into();
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    let stored = stored_episode(&shows).await;
    assert_eq!(stored.absolute_number.as_deref(), Some("2"));
    assert_eq!(stored.contiguous_absolute_number, Some(2));
}

#[tokio::test]
async fn an_episode_becoming_filler_is_unmonitored_when_the_show_skips_filler() {
    let (app, shows, title) = refresh_fixture(
        MediaFacet::Anime,
        vec!["scryer:filler-policy:skip_filler".to_string()],
    )
    .await;
    let mut episodes = vec![full_episode()];
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;
    assert!(stored_episode(&shows).await.monitored);

    episodes[0].is_filler = true;
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    let stored = stored_episode(&shows).await;
    assert!(stored.is_filler);
    assert!(!stored.monitored);
}

#[tokio::test]
async fn an_episode_becoming_filler_stays_monitored_when_the_show_keeps_filler() {
    let (app, shows, title) = refresh_fixture(MediaFacet::Anime, vec![]).await;
    let mut episodes = vec![full_episode()];
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    episodes[0].is_filler = true;
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    let stored = stored_episode(&shows).await;
    assert!(stored.is_filler);
    assert!(stored.monitored);
}

#[tokio::test]
async fn an_episode_becoming_recap_is_unmonitored_when_the_show_skips_recaps() {
    let (app, shows, title) = refresh_fixture(
        MediaFacet::Anime,
        vec!["scryer:recap-policy:skip_recap".to_string()],
    )
    .await;
    let mut episodes = vec![full_episode()];
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    episodes[0].is_recap = true;
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;

    let stored = stored_episode(&shows).await;
    assert!(stored.is_recap);
    assert!(!stored.monitored);
}

/// The skip policy acts on the flip only. An operator who re-monitors a filler
/// episode keeps it monitored through later refreshes, and an episode that
/// stops being filler is not re-monitored on its own.
#[tokio::test]
async fn a_remonitored_filler_episode_stays_monitored_on_later_refreshes() {
    let (app, shows, title) = refresh_fixture(
        MediaFacet::Anime,
        vec!["scryer:filler-policy:skip_filler".to_string()],
    )
    .await;
    let mut episodes = vec![full_episode()];
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;
    episodes[0].is_filler = true;
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;
    let episode_id = stored_episode(&shows).await.id;
    assert!(!stored_episode(&shows).await.monitored);

    shows
        .update_episode(
            &episode_id,
            EpisodeUpdate {
                monitored: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("operator re-monitors the episode");
    shows.episode_updates.lock().await.clear();
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;
    assert!(stored_episode(&shows).await.monitored);
    assert!(shows.episode_updates.lock().await.is_empty());

    shows
        .update_episode(
            &episode_id,
            EpisodeUpdate {
                monitored: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("operator unmonitors the episode again");
    episodes[0].is_filler = false;
    app.create_series_seasons_and_episodes(&title, &season_one(), &episodes, &[], &[])
        .await;
    let stored = stored_episode(&shows).await;
    assert!(!stored.is_filler);
    assert!(!stored.monitored, "leaving filler does not re-monitor");
}
