//! Read budget for one title walk. Every scope of a title used to re-read the
//! title's whole episode list, its collections and its files; a
//! [`TitleCatalogReads`] shared across the walk reads each once.

use super::*;
use crate::acquisition::title_reads::TitleCatalogReads;
use crate::quality::canonical_context::SubjectIntent;

struct SeededSeries {
    app: AppUseCase,
    title: Title,
    collection_id: String,
    episode_ids: Vec<String>,
    shows: Arc<MockShowRepo>,
    media_files: Arc<MockMediaFileRepo>,
    user: User,
}

impl SeededSeries {
    /// Give `episode_ids` one primary file of `quality`, linked to every one
    /// of them, and return its id.
    async fn seed_primary_file(&self, quality: &str, episode_ids: &[&str]) -> String {
        let name = format!("Read.Budget.Series.{quality}.WEB-DL.H.264-GRP");
        let file_id = self
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: self.title.id.clone(),
                file_path: format!("/fixture/{name}-{}.mkv", Id::new().0),
                size_bytes: 1_500_000_000,
                role: MediaFileRole::Primary,
                quality_label: Some(quality.to_string()),
                scene_name: Some(name),
                ..Default::default()
            })
            .await
            .expect("insert media file");
        {
            let mut store = self.media_files.store.lock().await;
            let row = store
                .iter_mut()
                .find(|entry| entry.id == file_id)
                .expect("inserted row");
            row.episode_id = Some(episode_ids[0].to_string());
            row.role = MediaFileRole::Primary;
        }
        for episode_id in &episode_ids[1..] {
            self.media_files
                .link_additional_episode(&file_id, episode_id)
                .await;
        }
        file_id
    }

    fn scopes(&self) -> Vec<crate::SubmissionScope> {
        let mut scopes = self
            .episode_ids
            .iter()
            .map(|episode_id| crate::SubmissionScope::Episode {
                episode_id: episode_id.clone(),
            })
            .collect::<Vec<_>>();
        scopes.push(crate::SubmissionScope::Collection {
            collection_id: self.collection_id.clone(),
        });
        scopes
    }

    fn store_reads(&self) -> [usize; 5] {
        [
            self.shows.title_episode_reads.load(Ordering::SeqCst),
            self.shows.title_collection_reads.load(Ordering::SeqCst),
            self.shows.collection_episode_reads.load(Ordering::SeqCst),
            self.media_files.title_file_reads.load(Ordering::SeqCst),
            self.media_files.scoped_file_reads.load(Ordering::SeqCst),
        ]
    }

    /// Resolve admission and membership for every scope, the way a walk does
    /// for each due target, and return what they resolved to.
    async fn resolve_all(&self, reads: Option<&TitleCatalogReads>) -> Vec<String> {
        let profile = test_quality_profile("read-budget");
        let context = self
            .app
            .resolve_canonical_scoring_context(&self.title, &profile)
            .await;
        let mut resolved = Vec::new();
        for scope in self.scopes() {
            let (subject, membership) = match reads {
                Some(reads) => (
                    self.app
                        .admission_subject_for_scope_with_reads(
                            &self.title,
                            &scope,
                            &context,
                            None,
                            SubjectIntent::Grab,
                            reads,
                        )
                        .await,
                    self.app
                        .scope_membership_for_with_reads(&self.title, &scope, reads)
                        .await,
                ),
                None => (
                    self.app
                        .admission_subject_for_scope(
                            &self.title,
                            &scope,
                            &context,
                            None,
                            SubjectIntent::Grab,
                        )
                        .await,
                    self.app.scope_membership_for(&self.title, &scope).await,
                ),
            };
            resolved.push(format!("{subject:?} {membership:?}"));
        }
        resolved
    }
}

async fn seed_series() -> SeededSeries {
    let (app, user) = bootstrap();
    let media_files = Arc::new(MockMediaFileRepo::default());
    let mut app =
        app.with_test_overrides(|services| services.with_media_files(media_files.clone()));
    let shows = Arc::new(MockShowRepo::default());
    app.services.catalog.shows = shows.clone();

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Read Budget Series".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("4".into()),
        )
        .await
        .expect("create collection");
    let mut episode_ids = Vec::new();
    for number in 1..=4 {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some((Utc::now() - chrono::Duration::days(30 - number)).to_rfc3339()),
                Some(1_320),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    SeededSeries {
        app,
        title,
        collection_id: collection.id,
        episode_ids,
        shows,
        media_files,
        user,
    }
}

/// What a scoped read answers, in a form two answers can be compared by.
fn scoped_answer(files: &[EpisodeScopedMediaFile]) -> Vec<String> {
    files
        .iter()
        .map(|file| {
            format!(
                "{} {:?} {:?} {:?}",
                file.media_file.id,
                file.media_file.role,
                file.episode_ids,
                file.primary_episode_ids
            )
        })
        .collect()
}

#[tokio::test]
async fn primed_episode_sets_answer_like_fresh_reads_from_one_store_read() {
    let series = seed_series().await;
    let [e1, e2, e3, e4] = std::array::from_fn(|index| series.episode_ids[index].as_str());
    series.seed_primary_file("1080p", &[e1]).await;
    series.seed_primary_file("720p", &[e2, e3]).await;
    let sets = vec![
        vec![e1.to_string()],
        vec![e2.to_string(), e3.to_string()],
        vec![e3.to_string()],
        vec![e4.to_string()],
        vec![e1.to_string(), e2.to_string()],
    ];

    let mut fresh = Vec::new();
    for set in &sets {
        let reads = TitleCatalogReads::new(&series.title.id);
        fresh.push(scoped_answer(
            &reads
                .live_media_files_for_episode_ids(&series.app, set)
                .await
                .expect("fresh read"),
        ));
    }

    let reads = TitleCatalogReads::new(&series.title.id);
    let before = series.media_files.scoped_file_reads.load(Ordering::SeqCst);
    reads
        .prime_live_media_files_for_episode_sets(&series.app, &sets)
        .await
        .expect("prime");
    assert_eq!(
        series.media_files.scoped_file_reads.load(Ordering::SeqCst) - before,
        1,
        "every set is answered from one read of their union"
    );
    for (set, expected) in sets.iter().zip(&fresh) {
        let primed = reads
            .live_media_files_for_episode_ids(&series.app, set)
            .await
            .expect("primed read");
        assert_eq!(&scoped_answer(&primed), expected, "set {set:?}");
    }
    assert_eq!(
        series.media_files.scoped_file_reads.load(Ordering::SeqCst) - before,
        1,
        "primed sets cost no further reads"
    );
}

/// A manual-import selection of `files` (name, size) for the seeded series,
/// with the built-in 4K profile as the global profile when `with_profile`.
async fn seed_verdict_selection(
    series: &SeededSeries,
    files: &[(&str, Option<i64>)],
    with_profile: bool,
) -> (AppUseCase, String) {
    let import_repo = Arc::new(TrackingImportRepo::default());
    let mut app = series
        .app
        .with_test_overrides(|services| services.with_imports(import_repo.clone()));
    if with_profile {
        let profile = crate::builtin_4k_profile();
        let profiles = Arc::new(StoredQualityProfileRepo::default());
        profiles.set_profiles(vec![profile.clone()]).await;
        app.services.config.quality_profiles = profiles;
        app.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                None,
                serde_json::to_string(&profile.id).expect("profile id json"),
                "test",
                None,
            )
            .await
            .expect("select the global profile");
    }
    let selection = crate::ManualImportSelection {
        id: Id::new().0,
        actor_user_id: series.user.id.clone(),
        title_id: series.title.id.clone(),
        source_identity: ClientJobLocator {
            client_id: Some("qbittorrent-primary".to_string()),
            client_type: "qbittorrent".to_string(),
            item_id: "verdict-selection".to_string(),
        },
        canonical_download_id: None,
        release_evidence_json: Some(
            serde_json::to_string(
                &crate::import_workflow::ReleaseEvidence::DownloaderObservation {
                    release_name: None,
                },
            )
            .expect("evidence json"),
        ),
        trusted_source_root: "/fixture/download".to_string(),
        archive_workspace_root: None,
        candidates: files
            .iter()
            .enumerate()
            .map(
                |(index, (name, size_bytes))| crate::ManualImportSelectionCandidate {
                    id: format!("candidate-{index}"),
                    canonical_path: format!("/fixture/download/{name}.mkv"),
                    size_bytes: *size_bytes,
                },
            )
            .collect(),
    };
    let selection_id = selection.id.clone();
    *import_repo.manual_import_selection.lock().await = Some(selection);
    (app, selection_id)
}

fn verdict_request(candidate: usize, episode_ids: &[&str]) -> crate::ManualImportVerdictRequest {
    crate::ManualImportVerdictRequest {
        candidate_id: format!("candidate-{candidate}"),
        episode_ids: episode_ids.iter().map(|id| id.to_string()).collect(),
    }
}

#[tokio::test]
async fn manual_import_verdicts_compare_each_mapping_with_what_its_episodes_hold() {
    let series = seed_series().await;
    let [e1, e2, e3, _] = std::array::from_fn(|index| series.episode_ids[index].as_str());
    series.seed_primary_file("2160p", &[e1]).await;
    series.seed_primary_file("480p", &[e2]).await;
    let size = Some(1_500_000_000);
    let (app, selection_id) = seed_verdict_selection(
        &series,
        &[
            ("Read.Budget.Series.S01E01.1080p.WEB-DL.H.264-GRP", size),
            ("Read.Budget.Series.S01E02.1080p.WEB-DL.H.264-GRP", size),
            ("Read.Budget.Series.S01E03.1080p.WEB-DL.H.264-GRP", size),
            ("Read.Budget.Series.S01E01.1080p.WEB-DL.H.264-OLD", None),
            ("Read.Budget.Series.S01E01E02.1080p.WEB-DL.H.264-GRP", size),
        ],
        true,
    )
    .await;

    let before = series.media_files.scoped_file_reads.load(Ordering::SeqCst);
    let rows = crate::manual_import_verdicts(
        &app,
        &series.user,
        &selection_id,
        vec![
            verdict_request(0, &[e1]),
            verdict_request(1, &[e2]),
            verdict_request(2, &[e3]),
            verdict_request(3, &[e1]),
            verdict_request(4, &[e1, e2]),
            verdict_request(1, &["not-an-episode-of-this-title"]),
            verdict_request(9, &[e2]),
        ],
    )
    .await
    .expect("verdicts");
    assert_eq!(
        series.media_files.scoped_file_reads.load(Ordering::SeqCst) - before,
        1,
        "the whole request reads incumbents once"
    );

    use crate::ManualImportVerdict::*;
    let verdicts = rows.iter().map(|row| row.verdict).collect::<Vec<_>>();
    assert_eq!(
        verdicts,
        vec![
            Some(Downgrade),
            Some(Upgrade),
            Some(NoCurrentFile),
            // A selection row without a stored size is not estimated.
            None,
            // A span is judged against everything it would displace, so the
            // better file on one episode decides it.
            Some(Downgrade),
            None,
            None,
        ]
    );
    assert_eq!(
        rows[0]
            .current_file
            .as_ref()
            .and_then(|file| file.quality.as_deref()),
        Some("2160p")
    );
    assert!(rows[2].current_file.is_none());
    assert_eq!(rows[4].episode_ids, vec![e1.to_string(), e2.to_string()]);
    assert!(
        rows[5].episode_ids.is_empty(),
        "episodes of another title are dropped, not read"
    );
}

#[tokio::test]
async fn manual_import_verdicts_refuse_an_oversized_request_and_another_members_selection() {
    let series = seed_series().await;
    let e1 = series.episode_ids[0].as_str();
    let (app, selection_id) = seed_verdict_selection(
        &series,
        &[("Read.Budget.Series.S01E01.1080p.WEB-DL.H.264-GRP", Some(1))],
        false,
    )
    .await;

    // Without a profile to score against there is no estimate, and the
    // preview is not refused for it.
    let rows = crate::manual_import_verdicts(
        &app,
        &series.user,
        &selection_id,
        vec![verdict_request(0, &[e1])],
    )
    .await
    .expect("no profile leaves the rows unestimated");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verdict, None);

    let too_many_rows = (0..=crate::MANUAL_IMPORT_VERDICT_MAX_ROWS)
        .map(|_| verdict_request(0, &[e1]))
        .collect();
    let too_many_episodes = vec![crate::ManualImportVerdictRequest {
        candidate_id: "candidate-0".to_string(),
        episode_ids: (0..=crate::MANUAL_IMPORT_VERDICT_MAX_EPISODES_PER_ROW)
            .map(|index| format!("episode-{index}"))
            .collect(),
    }];
    for requests in [too_many_rows, too_many_episodes] {
        let error = crate::manual_import_verdicts(&app, &series.user, &selection_id, requests)
            .await
            .expect_err("oversized request");
        assert_eq!(
            error.validation_reason(),
            Some(crate::error_reasons::import::MANUAL_IMPORT_VERDICT_REQUEST_TOO_LARGE)
        );
    }

    let mut stranger = series.user.clone();
    stranger.id = Id::new().0;
    let error = crate::manual_import_verdicts(
        &app,
        &stranger,
        &selection_id,
        vec![verdict_request(0, &[e1])],
    )
    .await
    .expect_err("another member's selection");
    assert_eq!(
        error.validation_reason(),
        Some(crate::error_reasons::integration::MANUAL_IMPORT_SELECTION_UNAVAILABLE)
    );
}

#[tokio::test]
async fn one_memo_reads_each_catalog_list_once_across_every_scope_of_a_walk() {
    let series = seed_series().await;

    // The unshared path: each scope pays for its own reads. This is the
    // baseline the memo has to beat, and the answers it has to match.
    let before = series.store_reads();
    let unshared = series.resolve_all(None).await;
    let unshared_reads = series.store_reads();
    let unshared_episode_list_reads = unshared_reads[0] - before[0];
    assert!(
        unshared_episode_list_reads >= series.scopes().len(),
        "every unshared scope re-reads the title's episodes; saw {unshared_episode_list_reads}"
    );

    let reads = TitleCatalogReads::new(&series.title.id);
    let shared = series.resolve_all(Some(&reads)).await;
    assert_eq!(
        shared, unshared,
        "a shared memo must resolve every scope exactly as fresh reads do"
    );
    let first_pass = series.store_reads();
    let [episodes, collections, collection_episodes, _, _] =
        std::array::from_fn::<usize, 5, _>(|index| first_pass[index] - unshared_reads[index]);
    assert_eq!(
        episodes, 1,
        "the title's episode list is read once per walk"
    );
    assert!(
        collections <= 1,
        "the title's collections are read at most once per walk; saw {collections}"
    );
    assert!(
        collection_episodes <= 1,
        "the one season's episode list is read at most once per walk; saw {collection_episodes}"
    );

    // A second pass over the same scopes, as a walk makes when a target is
    // re-evaluated, is served entirely from the memo.
    assert_eq!(series.resolve_all(Some(&reads)).await, unshared);
    assert_eq!(
        series.store_reads(),
        first_pass,
        "a repeated scope must cost no store reads within one walk"
    );
}

#[tokio::test]
async fn a_memo_for_another_title_never_answers_for_this_one() {
    let series = seed_series().await;
    let foreign = TitleCatalogReads::new("some-other-title");
    let scope = crate::SubmissionScope::Collection {
        collection_id: series.collection_id.clone(),
    };

    let before = series.store_reads();
    let first = series
        .app
        .scope_membership_for_with_reads(&series.title, &scope, &foreign)
        .await;
    let second = series
        .app
        .scope_membership_for_with_reads(&series.title, &scope, &foreign)
        .await;
    let after = series.store_reads();

    let mut expected = series.episode_ids.clone();
    expected.sort();
    let mut resolved = first.episode_ids.clone();
    resolved.sort();
    assert_eq!(
        resolved, expected,
        "membership comes from this title's rows"
    );
    assert_eq!(first.episode_ids, second.episode_ids);
    assert_eq!(
        after[2] - before[2],
        2,
        "a mismatched memo falls back to a fresh read each call instead of caching"
    );
}

#[tokio::test]
async fn numbering_bridge_memo_retries_failed_reads_and_caches_absence() {
    let series = seed_series().await;
    let reads = TitleCatalogReads::new(&series.title.id);
    *series.shows.fail_anime_bridge.lock().await = true;
    assert!(reads.anime_numbering_bridge(&series.app).await.is_err());
    *series.shows.fail_anime_bridge.lock().await = false;
    assert!(
        reads
            .anime_numbering_bridge(&series.app)
            .await
            .unwrap()
            .is_none()
    );
    let after = series.shows.anime_bridge_reads.load(Ordering::SeqCst);
    assert!(
        reads
            .anime_numbering_bridge(&series.app)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        series.shows.anime_bridge_reads.load(Ordering::SeqCst),
        after
    );
}

#[test]
fn compact_cutoff_facts_preserve_every_scope_decision() {
    use crate::acquisition::decision_helpers::{CutoffScope, analyzed_cutoff_quality_for_scope};
    let files = vec![
        TitleMediaFile {
            episode_id: Some("e1".into()),
            quality_label: Some("1080p".into()),
            created_at: "2026-01-01".into(),
            ..Default::default()
        },
        TitleMediaFile {
            episode_id: Some("e1".into()),
            quality_label: Some("2160p".into()),
            role: crate::MediaFileRole::Additional,
            ..Default::default()
        },
        TitleMediaFile {
            episode_id: Some("e2".into()),
            quality_label: Some("720p".into()),
            ..Default::default()
        },
        TitleMediaFile {
            series_movie_link_ids: vec!["movie-link".into()],
            quality_label: Some("2160p".into()),
            ..Default::default()
        },
        TitleMediaFile {
            quality_label: Some("1080p".into()),
            ..Default::default()
        },
    ];
    let compact: Vec<_> = files
        .iter()
        .filter(|file| file.role.is_primary())
        .map(crate::MediaFileCutoffSummary::from)
        .collect();
    let criteria = crate::QualityProfileCriteria::default();
    for scope in [
        CutoffScope::Title,
        CutoffScope::Episode("e1".into()),
        CutoffScope::Episode("missing".into()),
        CutoffScope::SeriesMovieLink("movie-link".into()),
        CutoffScope::Episodes(vec!["e1".into(), "e2".into()]),
        CutoffScope::Episodes(vec!["e1".into(), "missing".into()]),
        CutoffScope::Episodes(vec![]),
    ] {
        assert_eq!(
            analyzed_cutoff_quality_for_scope(&files, &scope, &criteria),
            analyzed_cutoff_quality_for_scope(&compact, &scope, &criteria)
        );
    }
}

#[test]
fn cutoff_falls_back_to_the_scanned_resolution_when_a_file_has_no_quality() {
    use crate::acquisition::decision_helpers::{CutoffScope, analyzed_cutoff_quality_for_scope};
    let file = |episode: &str, quality: Option<&str>, resolution: Option<&str>| TitleMediaFile {
        episode_id: Some(episode.into()),
        quality_label: quality.map(Into::into),
        resolution: resolution.map(Into::into),
        created_at: "2026-01-01".into(),
        ..Default::default()
    };
    let files = vec![
        // Scanned, never labelled: judged at its resolution.
        file("scanned", None, Some("1080p")),
        file("blank-label", Some("  "), Some(" 720p ")),
        // Labelled: the label decides, as before.
        file("labelled", Some("720p"), Some("2160p")),
        // Neither: no quality reached, as before.
        file("unknown", None, None),
        file("blank", Some(""), Some(" ")),
        // A resolution the profile does not rank leaves no quality, as before.
        file("unranked", None, Some("1080i")),
        file("unparsed", None, Some("odd-scan")),
    ];
    let compact: Vec<_> = files
        .iter()
        .map(crate::MediaFileCutoffSummary::from)
        .collect();
    let criteria = crate::QualityProfileCriteria::default();
    for (episode, expected) in [
        ("scanned", Some("1080p")),
        ("blank-label", Some("720p")),
        ("labelled", Some("720p")),
        ("unknown", None),
        ("blank", None),
        ("unranked", None),
        ("unparsed", None),
    ] {
        let scope = CutoffScope::Episode(episode.into());
        assert_eq!(
            analyzed_cutoff_quality_for_scope(&files, &scope, &criteria),
            expected,
            "{episode}"
        );
        assert_eq!(
            analyzed_cutoff_quality_for_scope(&compact, &scope, &criteria),
            expected,
            "{episode}: compact rows"
        );
    }
}
