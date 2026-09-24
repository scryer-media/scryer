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
}

impl SeededSeries {
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
    }
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
