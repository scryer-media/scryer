use super::*;
use chrono::Utc;

fn fixture_title(id: &str, name: &str, year: Option<i32>) -> Title {
    Title {
        id: id.into(),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        name: name.into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        root_folder_id: String::new(),
        created_by: None,
        created_at: Utc::now(),
        year,
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
        popularity: Some(1.0),
        canonical_tags: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: Some("eng".into()),
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        movie_release_dates: None,
        folder_path: None,
    }
}

#[test]
fn catalog_order_keeps_missing_years_last_in_both_directions() {
    for (direction, expected) in [
        (
            SortDirection::Asc,
            ["early", "later-a", "later-b", "missing"],
        ),
        (
            SortDirection::Desc,
            ["later-b", "later-a", "early", "missing"],
        ),
    ] {
        let mut titles = vec![
            fixture_title("missing", "Éclair", None),
            fixture_title("later-b", "Éclair", Some(2020)),
            fixture_title("early", "Éclair", Some(1990)),
            fixture_title("later-a", "Éclair", Some(2020)),
        ];
        sort_titles_for_catalog(
            &mut titles,
            &TitleCatalogSort {
                key: TitleCatalogSortKey::Title,
                direction,
                ..Default::default()
            },
        );
        assert_eq!(
            titles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn catalog_order_secondary_sort_is_ascending_for_other_sort_directions() {
    for key in [
        TitleCatalogSortKey::Monitored,
        TitleCatalogSortKey::Popularity,
    ] {
        for direction in [SortDirection::Asc, SortDirection::Desc] {
            let mut titles = vec![
                fixture_title("b", "Beta", None),
                fixture_title("a", "Alpha", None),
            ];
            sort_titles_for_catalog(
                &mut titles,
                &TitleCatalogSort {
                    key,
                    direction,
                    ..Default::default()
                },
            );
            assert_eq!(
                titles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
                ["a", "b"]
            );
        }
    }
}

#[test]
fn catalog_order_normalizes_unicode_width_and_case_without_stripping_tie_articles() {
    let mut titles = vec![
        fixture_title("article", "The Meridian", None),
        fixture_title("plain", "Meridian", None),
        fixture_title("width", "Ｍｅｒｉｄｉａｎ", None),
        fixture_title("accent-b", "éclair", None),
        fixture_title("accent-a", "ÉCLAIR", None),
    ];
    sort_titles_for_catalog(&mut titles, &TitleCatalogSort::default());
    assert_eq!(
        titles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["accent-a", "accent-b", "plain", "width", "article"]
    );
}
