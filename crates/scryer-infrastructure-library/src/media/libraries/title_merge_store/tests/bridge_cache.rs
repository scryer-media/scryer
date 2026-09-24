//! A merge retires its source title, and the source's numbering bridge row
//! cascades with it. The show store's bridge cache has to drop the source,
//! and must leave the surviving destination's cached bridge alone.

use super::*;
use crate::media::shows::store::ShowStore;
use scryer_application::ShowRepository;
use scryer_domain::{AnimeCommunitySeason, AnimeNumberingBridge};

fn bridge(tag: &str) -> AnimeNumberingBridge {
    AnimeNumberingBridge {
        generated_on: tag.to_string(),
        seasons: vec![AnimeCommunitySeason {
            index: 1,
            anidb_id: None,
            anilist_id: None,
            mal_id: None,
            titles: vec![format!("Synthetic Cour {tag}")],
            ranges: Vec::new(),
            absolute_start: Some(1),
            episode_count: Some(12),
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn a_merge_drops_the_retired_source_bridge_from_the_cache() {
    let (_, datastore) = test_store().await;
    insert_title_with_facet(&datastore, SOURCE, "library-a", &[], "anime").await;
    insert_title_with_facet(&datastore, DESTINATION, "library-b", &[], "anime").await;
    let shows = ShowStore::new(datastore.clone());
    let store = TitleMergeStore::new(datastore.clone())
        .with_anime_numbering_bridge_cache(shows.anime_numbering_bridge_cache());
    for (id, tag) in [(SOURCE, "source"), (DESTINATION, "destination")] {
        shows
            .replace_anime_numbering_bridge(id, Some(&bridge(tag)))
            .await
            .unwrap();
        assert_eq!(
            shows.get_anime_numbering_bridge(id).await.unwrap(),
            Some(bridge(tag))
        );
    }

    let plan = plan_for(&store).await;
    store.execute_title_merge(&plan).await.unwrap();

    assert_eq!(
        shows.get_anime_numbering_bridge(SOURCE).await.unwrap(),
        None,
        "the source's row cascaded away with it"
    );
    let cache = shows.anime_numbering_bridge_cache();
    assert_eq!(
        cache.lookup(DESTINATION),
        Some(Some(bridge("destination"))),
        "the destination keeps its row, so its cached value stays"
    );
}
