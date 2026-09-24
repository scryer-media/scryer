//! The background cutoff pass: which scopes it scores, how many catalog reads
//! a page costs, and the cross-cycle landed-bar memo that lets an idle library
//! skip re-scoring. Every memoised answer is checked against a fresh
//! derivation — the memo may only ever save work, never change a number.

use super::*;
use crate::acquisition_workflow::LandedBarScope;
use std::collections::BTreeSet;

struct Fixture {
    app: AppUseCase,
    user: User,
    settings: Arc<StoredSettingsRepo>,
    quality_profiles: Arc<StoredQualityProfileRepo>,
    media_files: Arc<MockMediaFileRepo>,
    titles: Arc<MockTitleRepo>,
    shows: Arc<MockShowRepo>,
    libraries: Arc<MockLibraryRepo>,
}

async fn fixture(profile: QualityProfile) -> Fixture {
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (mut app, user, titles) = bootstrap_with_cutoff_projection_state(
        settings.clone(),
        quality_profiles.clone(),
        media_files.clone(),
    );
    let shows = Arc::new(MockShowRepo {
        titles: Some(titles.clone()),
        ..MockShowRepo::default()
    });
    app.services.catalog.shows = shows.clone();
    let libraries = Arc::new(MockLibraryRepo::default());
    app.services.catalog.libraries = libraries.clone();
    let fixture = Fixture {
        app,
        user,
        settings,
        quality_profiles,
        media_files,
        titles,
        shows,
        libraries,
    };
    fixture
        .settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            &format!("\"{}\"", profile.id),
        )
        .await;
    fixture.quality_profiles.set_profiles(vec![profile]).await;
    fixture
}

impl Fixture {
    fn memo(&self) -> &crate::quality::landed_bar_memo::LandedBarMemo {
        &self.app.runtime.acquisition.landed_bar_memo
    }

    async fn add_movie(&self, name: &str, scene_name: &str, size_bytes: i64) -> Title {
        let title = self
            .app
            .add_title(
                &self.user,
                NewTitle {
                    name: name.into(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    year: Some(2024),
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        let quality = if scene_name.contains("720p") {
            "720P"
        } else {
            "1080P"
        };
        self.media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!("/library/{name}/{scene_name}.mkv"),
                size_bytes,
                quality_label: Some(quality.into()),
                scene_name: Some(scene_name.into()),
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("insert media file");
        title
    }

    async fn target_titles(&self) -> BTreeSet<String> {
        self.app
            .derive_acquisition_targets(&Utc::now())
            .await
            .expect("derive targets")
            .into_iter()
            .map(|target| target.title_id)
            .collect()
    }

    async fn edit_file(&self, title_id: &str, edit: impl Fn(&mut TitleMediaFile)) {
        let mut store = self.media_files.store.lock().await;
        for row in store.iter_mut().filter(|row| row.title_id == title_id) {
            edit(row);
        }
    }

    async fn edit_title(&self, title_id: &str, edit: impl Fn(&mut Title)) {
        let mut store = self.titles.store.lock().await;
        for title in store.iter_mut().filter(|title| title.id == title_id) {
            edit(title);
        }
    }

    /// A memoised derivation after an input changed: it must miss, fill, and
    /// return exactly what a fresh derivation returns — then hit on repeat.
    async fn assert_misses_then_hits(&self, scopes: &[LandedBarScope], change: &str) {
        let fresh = self.app.landed_bars_for_scopes(scopes).await;
        assert!(
            fresh.iter().all(Option::is_some),
            "{change}: every scope is occupied"
        );
        let (hits, fills) = self.memo().counters();
        let memoised = self.app.landed_bars_for_scopes_memoized(scopes).await;
        assert_eq!(
            memoised, fresh,
            "{change}: memoised bar must equal a fresh one"
        );
        assert_eq!(
            self.memo().counters(),
            (hits, fills + scopes.len() as u64),
            "{change}: a changed input must miss the memo"
        );
        assert_eq!(
            self.app.landed_bars_for_scopes_memoized(scopes).await,
            fresh,
            "{change}: the repeat must return the same bar"
        );
        assert_eq!(
            self.memo().counters(),
            (hits + scopes.len() as u64, fills + scopes.len() as u64),
            "{change}: unchanged inputs must hit"
        );
    }
}

fn movie_scope(title: &Title) -> LandedBarScope {
    LandedBarScope {
        title_id: title.id.clone(),
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
    }
}

fn score_cutoff_profile(cutoff_score: Option<i32>) -> QualityProfile {
    let mut profile = cutoff_projection_test_profile("memo", "1080P");
    profile.criteria.cutoff_score = cutoff_score;
    profile
}

#[tokio::test]
async fn the_score_pass_skips_tier_targets_and_its_memo_never_changes_the_target_set() {
    let fixture = fixture(score_cutoff_profile(None)).await;
    // Source and codec weights come from the bundled rules in production.
    fixture.app.swap_user_rules_engine(
        AppUseCase::build_user_rules_engine(
            crate::rules::builtin_trash::baseline_rule_sets(),
            Vec::new(),
        )
        .expect("bundled scoring rules compile"),
    );
    let below_tier = fixture
        .add_movie(
            "Tier Short",
            "Tier.Short.2024.720p.WEB-DL.x264-GRP",
            2_000_000_000,
        )
        .await;
    let below_score = fixture
        .add_movie(
            "Score Short",
            "Score.Short.2024.1080p.HDTV.x264-GRP",
            1_500_000_000,
        )
        .await;
    let at_score = fixture
        .add_movie(
            "Score Met",
            "Score.Met.2024.1080p.BluRay.x265.DTS-HD.MA.5.1-GRP",
            12_000_000_000,
        )
        .await;

    let bars = fixture
        .app
        .landed_bars_for_scopes(&[movie_scope(&below_score), movie_scope(&at_score)])
        .await;
    let (Some(short_bar), Some(met_bar)) = (bars[0], bars[1]) else {
        panic!("both 1080p files must score: {bars:?}");
    };
    assert!(
        short_bar < met_bar,
        "the fixture needs one file below the other: {short_bar} vs {met_bar}"
    );
    fixture
        .quality_profiles
        .set_profiles(vec![score_cutoff_profile(Some(met_bar))])
        .await;

    // Below tier (quality sweep), at tier but below score (score pass); the
    // file sitting exactly at `cutoff_score` has reached it.
    let expected = BTreeSet::from([below_tier.id.clone(), below_score.id.clone()]);
    assert_eq!(fixture.target_titles().await, expected);
    assert_eq!(
        fixture.memo().len(),
        2,
        "the tier-short scope is a target whatever it scores, so it is never scored"
    );
    assert_eq!(fixture.memo().counters(), (0, 2));

    // An idle cycle: nothing changed, nothing is re-scored.
    assert_eq!(fixture.target_titles().await, expected);
    assert_eq!(fixture.memo().counters(), (2, 0));

    // Memo off (as while an engine reads the clock): the same targets.
    fixture.memo().reset_for_engine(true);
    assert_eq!(fixture.target_titles().await, expected);
    assert_eq!(fixture.memo().len(), 0);
    fixture.memo().reset_for_engine(false);
    assert_eq!(fixture.target_titles().await, expected);
    assert_eq!(fixture.memo().counters(), (0, 2));

    // A re-scanned file is a new key; the completed pass drops the old one.
    fixture
        .edit_file(&at_score.id, |row| row.size_bytes += 1)
        .await;
    let fresh = fixture
        .app
        .landed_bars_for_scopes(&[movie_scope(&at_score)])
        .await[0]
        .expect("still scored");
    let mut expected = expected;
    if fresh < met_bar {
        expected.insert(at_score.id.clone());
    }
    assert_eq!(fixture.target_titles().await, expected);
    assert_eq!(fixture.memo().counters(), (1, 1));
    assert_eq!(
        fixture.memo().len(),
        2,
        "the superseded entry is swept, not kept beside its replacement"
    );
}

#[tokio::test]
async fn every_scoring_input_change_misses_the_memo() {
    let fixture = fixture(score_cutoff_profile(None)).await;
    let title = fixture
        .add_movie(
            "Memo Inputs",
            "Memo.Inputs.2024.1080p.WEB-DL.DDP5.1.H.264-GRP",
            4_000_000_000,
        )
        .await;
    let scopes = [movie_scope(&title)];

    fixture
        .assert_misses_then_hits(&scopes, "first derivation")
        .await;

    fixture
        .edit_file(&title.id, |row| {
            row.scene_name = Some("Memo.Inputs.2024.1080p.BluRay.REMUX.AVC.TrueHD-GRP".into());
        })
        .await;
    fixture
        .assert_misses_then_hits(&scopes, "file analysis")
        .await;

    let mut profile = score_cutoff_profile(None);
    profile.criteria.prefer_remux = true;
    fixture
        .quality_profiles
        .set_profiles(vec![profile.clone()])
        .await;
    fixture
        .assert_misses_then_hits(&scopes, "ranking-only profile field")
        .await;

    profile.criteria.required_audio_languages = vec!["ja".into()];
    fixture
        .quality_profiles
        .set_profiles(vec![profile.clone()])
        .await;
    fixture
        .assert_misses_then_hits(&scopes, "required audio languages")
        .await;

    fixture
        .settings
        .set_value(SETTINGS_SCOPE_SYSTEM, SCORING_PERSONA_KEY, "\"Compatible\"")
        .await;
    fixture.assert_misses_then_hits(&scopes, "persona").await;

    fixture
        .edit_title(&title.id, |title| title.tags.push("synthetic-tag".into()))
        .await;
    fixture.assert_misses_then_hits(&scopes, "title tags").await;

    fixture
        .edit_title(&title.id, |title| title.runtime_minutes = Some(95))
        .await;
    fixture.assert_misses_then_hits(&scopes, "runtime").await;

    {
        let mut libraries = fixture.libraries.libraries.lock().await;
        for library in libraries
            .iter_mut()
            .filter(|library| library.id == title.library_id)
        {
            library.name = "Renamed Library".into();
        }
    }
    fixture
        .assert_misses_then_hits(&scopes, "library rename")
        .await;

    fixture.app.swap_user_rules_engine(
        AppUseCase::build_user_rules_engine(
            crate::rules::builtin_trash::baseline_rule_sets(),
            Vec::new(),
        )
        .expect("bundled scoring rules compile"),
    );
    assert_eq!(fixture.memo().len(), 0, "a rules swap clears the memo");
    fixture.assert_misses_then_hits(&scopes, "rules swap").await;
}

#[tokio::test]
async fn an_engine_that_reads_the_clock_is_never_memoised() {
    let fixture = fixture(score_cutoff_profile(None)).await;
    let title = fixture
        .add_movie(
            "Clock Rules",
            "Clock.Rules.2024.1080p.WEB-DL.H.264-GRP",
            4_000_000_000,
        )
        .await;
    let policy = scryer_rules::UserPolicy {
        id: "clock_rule".to_string(),
        name: "Clock rule".to_string(),
        rego_source: scryer_rules::rewrite_package_declaration(
            "score_entry[\"late\"] := 0 if {\n    time.now_ns() > 0\n}\n",
            "clock_rule",
        ),
        origin: scryer_rules::PolicyOrigin::User,
        applied_facets: vec![],
    };
    let engine = scryer_rules::UserRulesEngine::build(&[policy]).expect("clock rule compiles");
    assert!(engine.reads_clock());
    fixture.app.swap_user_rules_engine(engine);

    let scopes = [movie_scope(&title)];
    let fresh = fixture.app.landed_bars_for_scopes(&scopes).await;
    for _ in 0..2 {
        assert_eq!(
            fixture.app.landed_bars_for_scopes_memoized(&scopes).await,
            fresh
        );
    }
    assert_eq!(fixture.memo().len(), 0);
    assert_eq!(fixture.memo().counters(), (0, 0));
}

#[tokio::test]
async fn a_page_of_series_scopes_reads_episodes_once_and_a_changed_span_misses() {
    let fixture = fixture(score_cutoff_profile(None)).await;
    let mut scopes = Vec::new();
    let mut spans = Vec::new();
    for name in ["Batch One", "Batch Two", "Batch Three"] {
        let title = fixture
            .app
            .add_title(
                &fixture.user,
                NewTitle {
                    name: name.into(),
                    facet: MediaFacet::Series,
                    monitored: true,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        let collection = fixture
            .app
            .create_collection(
                &fixture.user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("2".into()),
            )
            .await
            .expect("create collection");
        let mut episode_ids = Vec::new();
        for number in 1..=2 {
            let episode = fixture
                .app
                .create_episode(
                    &fixture.user,
                    title.id.clone(),
                    Some(collection.id.clone()),
                    "standard".into(),
                    Some(number.to_string()),
                    Some("1".into()),
                    None,
                    Some(format!("Episode {number}")),
                    Some((Utc::now() - chrono::Duration::days(30)).to_rfc3339()),
                    Some(1_440),
                    false,
                    false,
                )
                .await
                .expect("create episode");
            episode_ids.push(episode.id);
        }
        let dotted = name.replace(' ', ".");
        let file_id = fixture
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!("/library/{name}/Season 01/{name} - S01E01.mkv"),
                size_bytes: 1_400_000_000,
                quality_label: Some("1080P".into()),
                scene_name: Some(format!("{dotted}.S01E01.1080p.WEB-DL.H.264-GRP")),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        fixture
            .media_files
            .link_file_to_episode(&file_id, &episode_ids[0])
            .await
            .expect("link episode");
        scopes.push(LandedBarScope {
            title_id: title.id.clone(),
            episode_id: Some(episode_ids[0].clone()),
            collection_id: None,
            series_movie_link_id: None,
        });
        spans.push((file_id, episode_ids));
    }

    let reads = || {
        (
            fixture.shows.titles_episode_reads.load(Ordering::SeqCst),
            fixture.shows.title_episode_reads.load(Ordering::SeqCst),
        )
    };
    let before = reads();
    fixture
        .assert_misses_then_hits(&scopes, "first derivation")
        .await;
    let after = reads();
    // `assert_misses_then_hits` derives three times (fresh, miss, hit).
    assert_eq!(
        (after.0 - before.0, after.1 - before.1),
        (3, 0),
        "each derivation reads every title's episodes in one batched query"
    );

    // The first file now also covers E02, the way the join emits a second
    // link: a longer span is a different size basis, so a different key.
    {
        let (file_id, episode_ids) = &spans[0];
        let mut store = fixture.media_files.store.lock().await;
        let mut second = store
            .iter()
            .find(|row| &row.id == file_id)
            .cloned()
            .expect("seeded file");
        second.episode_id = Some(episode_ids[1].clone());
        store.push(second);
    }
    let (hits, fills) = fixture.memo().counters();
    let fresh = fixture.app.landed_bars_for_scopes(&scopes).await;
    assert_eq!(
        fixture.app.landed_bars_for_scopes_memoized(&scopes).await,
        fresh
    );
    assert_eq!(
        fixture.memo().counters(),
        (hits + 2, fills + 1),
        "only the scope whose episode span changed is re-scored"
    );
}
