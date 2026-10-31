//! The RSS cycle matches all of its new feed items in one batched pass.
//!
//! A batch shares the index reads, never the answers: every item must resolve
//! to the same title, or stay ambiguous or unmatched, as it would when matched
//! on its own. And the reads a cycle makes must not grow with the number of
//! items it carries.

use super::*;
use crate::IndexerResponseAttributes;
use crate::acquisition::rss::{
    rss_title_id_for_asserted_release, rss_title_id_for_release_read_item_by_item,
    rss_title_ids_for_cycle,
};
use crate::import_title_resolution::MonitoredTitleMatcher;
use chrono::TimeZone;
use scryer_domain::ExternalId;

fn title(
    id: &str,
    name: &str,
    facet: MediaFacet,
    year: i32,
    external_ids: &[(&str, &str)],
) -> Title {
    Title {
        id: id.to_string(),
        library_id: "library".to_string(),
        name: name.to_string(),
        facet,
        monitored: true,
        tags: Vec::new(),
        canonical_tags: vec![],
        external_ids: external_ids
            .iter()
            .map(|(source, value)| ExternalId::new(source.to_string(), value.to_string()))
            .collect(),
        root_folder_id: "root".to_string(),
        created_by: None,
        created_at: Utc.timestamp_opt(0, 0).unwrap(),
        year: Some(year),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: Vec::new(),
        tagged_aliases: Vec::new(),
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        movie_release_dates: None,
        folder_path: None,
    }
}

fn movie(id: &str, name: &str, year: i32) -> Title {
    title(id, name, MediaFacet::Movie, year, &[])
}

fn asserting_tvdb(tvdb_id: &str) -> IndexerResponseAttributes {
    IndexerResponseAttributes {
        tvdb_id: Some(tvdb_id.to_string()),
        ..IndexerResponseAttributes::default()
    }
}

/// A library with one title per lookup lane, two series sharing a name that
/// only an asserted id or a year can tell apart, and an unmonitored title
/// nothing may match.
async fn library() -> Arc<MockTitleRepo> {
    let repo = Arc::new(MockTitleRepo::default());
    let mut alias_title = movie("alias", "Corvane Tessel", 2016);
    alias_title.aliases = vec!["Brindle Ostravel (2016)".to_string()];
    let mut spelling_title = title("spelling", "Quessel Harbor", MediaFacet::Series, 2019, &[]);
    spelling_title.tagged_aliases = vec![scryer_domain::TaggedAlias {
        name: "Kväll Brönnöviken".into(),
        language: "swe".into(),
        country: None,
    }];
    let mut unmonitored = movie("unmonitored", "Pellucid Varrow Sound", 2020);
    unmonitored.monitored = false;
    repo.store.lock().await.extend([
        movie("exact", "Vellor Dunmarch", 2018),
        alias_title,
        spelling_title,
        movie("colour", "Thessel Colour Works", 2019),
        title(
            "twin-a",
            "Marrow Glen",
            MediaFacet::Series,
            2023,
            &[("tvdb", "910001")],
        ),
        title(
            "twin-b",
            "Marrow Glen",
            MediaFacet::Anime,
            1999,
            &[("tvdb", "910002")],
        ),
        unmonitored,
    ]);
    repo
}

/// One item per lane: an exact name, a year-suffixed alias reached by its
/// year-stripped shape, two spelling forms, a name two titles share (bare,
/// then with an asserted id for each, then with a year), a name nothing
/// carries, and an unmonitored title's name.
fn mixed_cycle() -> Vec<(&'static str, IndexerResponseAttributes)> {
    let bare = IndexerResponseAttributes::default;
    vec![
        ("Vellor.Dunmarch.2018.1080p.WEB.H264-GRP", bare()),
        ("Brindle.Ostravel.2016.1080p.BluRay.x264-GRP", bare()),
        (
            "Kvall.Bronnoviken.S01E02.SWEDiSH.1080p.WEB.h264-GRP",
            bare(),
        ),
        ("Thessel.Color.Works.2019.1080p.WEB.H264-GRP", bare()),
        ("Marrow.Glen.S02E01.1080p.WEB-DL.x264-GRP", bare()),
        (
            "Marrow.Glen.S02E01.1080p.WEB-DL.x264-GRP",
            asserting_tvdb("910002"),
        ),
        (
            "Marrow.Glen.S02E01.720p.WEB-DL.x264-GRP",
            asserting_tvdb("910001"),
        ),
        ("Marrow.Glen.2023.S02E01.1080p.WEB-DL.x264-GRP", bare()),
        ("Zephyrine.Klatt.Odyssey.2020.1080p.WEB.H264-GRP", bare()),
        ("Pellucid.Varrow.Sound.2020.1080p.WEB.H264-GRP", bare()),
        // The first item again, after everything else: a batch answers it
        // the same wherever it sits.
        ("Vellor.Dunmarch.2018.1080p.WEB.H264-GRP", bare()),
    ]
}

fn matcher(repo: &Arc<MockTitleRepo>) -> MonitoredTitleMatcher {
    MonitoredTitleMatcher::new(repo.clone())
}

#[tokio::test]
async fn a_batched_cycle_answers_every_item_as_it_would_alone() {
    let repo = library().await;
    let cycle = mixed_cycle();

    let mut alone = Vec::with_capacity(cycle.len());
    let mut item_by_item = Vec::with_capacity(cycle.len());
    for (release_title, attributes) in &cycle {
        alone.push(
            rss_title_id_for_asserted_release(matcher(&repo), release_title, attributes)
                .await
                .expect("match one item"),
        );
        item_by_item.push(
            rss_title_id_for_release_read_item_by_item(matcher(&repo), release_title, attributes)
                .await
                .expect("match one item with single-item reads"),
        );
    }
    let batched = rss_title_ids_for_cycle(matcher(&repo), &cycle)
        .await
        .expect("match the cycle");

    assert_eq!(batched, item_by_item);
    assert_eq!(alone, item_by_item);
    let expected = [
        Some("exact"),
        Some("alias"),
        Some("spelling"),
        Some("colour"),
        None,
        Some("twin-b"),
        Some("twin-a"),
        Some("twin-a"),
        None,
        None,
        Some("exact"),
    ]
    .map(|id| id.map(str::to_string));
    assert_eq!(batched, expected);
}

/// Each item's evidence is its own: an ambiguous item batched beside an
/// item that resolves one twin stays ambiguous, and the resolved item stays
/// resolved, in either order.
#[tokio::test]
async fn an_item_in_a_batch_never_borrows_another_items_evidence() {
    let repo = library().await;
    let bare = (
        "Marrow.Glen.S02E01.1080p.WEB-DL.x264-GRP",
        IndexerResponseAttributes::default(),
    );
    let asserted = (
        "Marrow.Glen.S02E01.1080p.WEB-DL.x264-GRP",
        asserting_tvdb("910001"),
    );

    let forward = rss_title_ids_for_cycle(matcher(&repo), &[bare.clone(), asserted.clone()])
        .await
        .expect("match the cycle");
    let reverse = rss_title_ids_for_cycle(matcher(&repo), &[asserted, bare])
        .await
        .expect("match the cycle");

    assert_eq!(forward, vec![None, Some("twin-a".to_string())]);
    assert_eq!(reverse, vec![Some("twin-a".to_string()), None]);
}

#[tokio::test]
async fn a_cycle_reads_the_index_a_bounded_number_of_times_whatever_its_size() {
    let repo = library().await;
    let reads = || repo.matching_reads.load(Ordering::SeqCst);

    let small = mixed_cycle();
    let before = reads();
    rss_title_ids_for_cycle(matcher(&repo), &small)
        .await
        .expect("match the small cycle");
    let small_reads = reads() - before;

    let large = (0..20).flat_map(|_| mixed_cycle()).collect::<Vec<_>>();
    let before = reads();
    let large_matches = rss_title_ids_for_cycle(matcher(&repo), &large)
        .await
        .expect("match the large cycle");
    let large_reads = reads() - before;

    // One read per lane: key shapes, asserted ids, spelling buckets, the
    // spelling candidates' rows, the candidates' index names and their
    // collision keys.
    assert!(small_reads <= 6, "a cycle took {small_reads} index reads");
    assert_eq!(
        large_reads, small_reads,
        "twenty times the items must not cost more reads"
    );
    assert_eq!(large_matches.len(), large.len());

    // Read item by item, the same items cost reads for every item and every
    // candidate.
    let before = reads();
    for (release_title, attributes) in &small {
        rss_title_id_for_release_read_item_by_item(matcher(&repo), release_title, attributes)
            .await
            .expect("match one item");
    }
    let single_reads = reads() - before;
    assert!(
        single_reads > small_reads * 2,
        "one at a time took {single_reads} reads, the batch {small_reads}"
    );
}
