use super::*;

#[test]
fn every_anchor_vocabulary_key_builds_a_keyed_subject() {
    for (target_key, facet, canonical) in [
        ("tvdb:series:81189", MediaFacet::Series, "tvdb:series:81189"),
        ("tmdb:movie:603", MediaFacet::Movie, "tmdb:movie:603"),
        ("tmdb:series:1396", MediaFacet::Anime, "tmdb:series:1396"),
        (
            "imdb:title:tt0133093",
            MediaFacet::Movie,
            "imdb:title:tt0133093",
        ),
        ("mal:anime:1", MediaFacet::Anime, "mal:anime:1"),
        ("anidb:anime:23", MediaFacet::Anime, "anidb:anime:23"),
        ("anilist:anime:1", MediaFacet::Anime, "anilist:anime:1"),
        (" TMDB:Movie:603 ", MediaFacet::Movie, "tmdb:movie:603"),
    ] {
        let (key, subject) =
            discovery_neighbor_subject(target_key, &facet).expect("vocabulary key is accepted");
        assert_eq!(key, canonical);
        assert_eq!(subject.key.as_deref(), Some(canonical));
        assert_eq!(subject.facet.as_deref(), Some(facet.as_str()));
        assert_eq!(subject.external_ids.len(), 1);
    }
}

#[test]
fn numeric_anchor_ids_fill_the_matching_subject_id() {
    let (_, tvdb) = discovery_neighbor_subject("tvdb:series:81189", &MediaFacet::Series).unwrap();
    assert_eq!(tvdb.tvdb_id, Some(81189));
    let (_, mal) = discovery_neighbor_subject("mal:anime:1", &MediaFacet::Anime).unwrap();
    assert_eq!(mal.mal_id, Some(1));
    let (_, imdb) = discovery_neighbor_subject("imdb:title:tt0133093", &MediaFacet::Movie).unwrap();
    assert_eq!(imdb.tmdb_id, None);
    assert_eq!(imdb.external_ids[0].source, "imdb");
}

#[test]
fn keys_outside_the_vocabulary_are_refused_with_a_code() {
    for target_key in [
        "",
        "tmdb",
        "tmdb:movie",
        "tmdb:movie:0",
        "tmdb:movie:603:extra",
        "tmdb:anime:603",
        "mal:series:1",
        "imdb:title:nm0000206x",
        "local:movie:603",
    ] {
        let error = discovery_neighbor_subject(target_key, &MediaFacet::Movie)
            .expect_err("key outside the vocabulary is refused");
        assert_eq!(
            error.validation_reason(),
            Some(error_reasons::discovery::NEIGHBOR_TARGET_KEY_UNSUPPORTED),
            "{target_key:?}"
        );
    }
}

#[test]
fn movie_and_series_keys_must_agree_with_the_facet() {
    for (target_key, facet) in [
        ("tmdb:movie:603", MediaFacet::Series),
        ("tvdb:series:81189", MediaFacet::Movie),
    ] {
        let error =
            discovery_neighbor_subject(target_key, &facet).expect_err("facet mismatch is refused");
        assert_eq!(
            error.validation_reason(),
            Some(error_reasons::discovery::NEIGHBOR_TARGET_KEY_FACET_MISMATCH)
        );
    }
}

#[test]
fn anime_takes_movie_and_series_keys_alike() {
    for target_key in ["tmdb:movie:129", "tvdb:movie:129", "tmdb:series:1396"] {
        let (key, subject) = discovery_neighbor_subject(target_key, &MediaFacet::Anime)
            .expect("an anime anchor may be a film or a series");
        assert_eq!(key, target_key);
        assert_eq!(subject.facet.as_deref(), Some("anime"));
    }
}

fn neighbor_group(id: i64, size: i64) -> crate::DiscoveryRelatedGroup {
    crate::DiscoveryRelatedGroup {
        id,
        label_key: None,
        label: format!("Synthetic Group {id}"),
        medoid: format!("tmdb:movie:{}", 700 + id),
        size,
    }
}

#[test]
fn served_groups_keep_only_groups_with_served_items_sized_by_them() {
    let mut items = [
        (1, Some(2)),
        (2, Some(0)),
        (3, Some(2)),
        (4, Some(9)),
        (5, None),
    ]
    .into_iter()
    .map(|(id, group_id)| DiscoveryNeighborItemRecord {
        item: crate::discovery::tests::test_discovery_item(
            &format!("tmdb:movie:{id}"),
            "movie",
            Some("movie"),
        ),
        group_id,
    })
    .collect::<Vec<_>>();
    let groups = vec![
        neighbor_group(0, 6),
        neighbor_group(1, 4),
        neighbor_group(2, 3),
    ];

    let served = discovery_neighbor_served_groups(&mut items, groups);

    // Gateway order is kept, group 1 has no served member, and sizes count
    // the served members only.
    assert_eq!(served, vec![neighbor_group(0, 1), neighbor_group(2, 2)]);
    // An id the gateway never returned is dropped rather than left dangling.
    assert_eq!(
        items
            .iter()
            .map(|neighbor| neighbor.group_id)
            .collect::<Vec<_>>(),
        vec![Some(2), Some(0), Some(2), None, None]
    );
}

#[test]
fn served_groups_are_empty_when_the_gateway_did_not_group() {
    let mut items = vec![DiscoveryNeighborItemRecord {
        item: crate::discovery::tests::test_discovery_item("tmdb:movie:1", "movie", Some("movie")),
        group_id: None,
    }];
    assert!(discovery_neighbor_served_groups(&mut items, Vec::new()).is_empty());
    assert_eq!(items[0].group_id, None);
}
