use super::*;
use crate::IndexerResponseAttributes;
use serde_json::json;

fn result(extra: serde_json::Value) -> IndexerSearchResult {
    IndexerSearchResult {
        http: None,
        indexer_id: None,
        source: "source".into(),
        title: "Synthetic Release 2026".into(),
        link: None,
        download_url: None,
        source_kind: None,
        size_bytes: None,
        published_at: None,
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: None,
        quality_profile_decision: None,
        extra: serde_json::from_value(extra).unwrap(),
        response_attributes: IndexerResponseAttributes::default(),
        guid: None,
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        coverage_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
        release_listing_json: None,
    }
}

#[test]
fn typed_only_result_yields_every_fact() {
    let mut result = result(json!({
        "seeders": 12,
        "leechers": 3,
        "info_hash_v1": "0123456789abcdef0123456789abcdef01234567",
        "magnet_url": "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "download_volume_factor": 0.0,
        "upload_volume_factor": 2.0,
        "minimum_seed_ratio": 1.5,
        "minimum_seed_time_minutes": 2880,
        "indexer_flags": ["freeleech", "  ", "internal"],
        "files": "4",
        "year": "2026",
        "genre": "Synthetic",
        "coverurl": "https://images.example/cover.jpg",
        "external_ids": {
            "imdb_id": "tt0133093",
            "tvdb_id": "0",
            "trakt": "77",
            "tvmaze_id": 88,
            "tvrage_id": "99",
            "douban": "55"
        }
    }));
    result.indexer_languages = Some(vec!["English".into(), " ".into()]);
    result.indexer_subtitles = Some(vec!["French".into()]);
    result.response_attributes.tmdb_id = Some("1001".into());

    let facts = result.release_facts();
    assert_eq!(
        facts,
        ReleaseFacts {
            seeders: Some(12),
            leechers: Some(3),
            peers: Some(15),
            info_hash: Some("0123456789abcdef0123456789abcdef01234567".into()),
            magnet_url: Some("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".into()),
            download_volume_factor: Some(0.0),
            upload_volume_factor: Some(2.0),
            minimum_seed_ratio: Some(1.5),
            minimum_seed_time_minutes: Some(2880),
            freeleech: Some(true),
            indexer_flags: vec!["freeleech".into(), "internal".into()],
            languages: vec!["English".into()],
            subtitles: vec!["French".into()],
            files: Some(4),
            year: Some(2026),
            genre: Some("Synthetic".into()),
            poster_url: Some("https://images.example/cover.jpg".into()),
            external_ids: ReleaseExternalIds {
                tvdb: None,
                tmdb: Some("1001".into()),
                imdb: Some("0133093".into()),
                trakt: Some("77".into()),
                tvmaze: Some("88".into()),
                tvrage: Some("99".into()),
                douban: Some("55".into()),
            },
        }
    );
}

#[test]
fn ununderscored_factor_keys_are_read_and_explicit_freeleech_wins() {
    let facts = result(json!({
        "downloadvolumefactor": 0.5,
        "uploadvolumefactor": 1.0,
        "peers": 9,
        "seeders": 4,
        "leechers": 2
    }))
    .release_facts();
    assert_eq!(facts.download_volume_factor, Some(0.5));
    assert_eq!(facts.upload_volume_factor, Some(1.0));
    assert_eq!(facts.freeleech, Some(false));
    assert_eq!(
        facts.peers,
        Some(9),
        "a reported peer count is not recomputed"
    );

    let facts = result(json!({"downloadvolumefactor": 0.5, "freeleech": true})).release_facts();
    assert_eq!(facts.freeleech, Some(true));
}

#[test]
fn absent_and_invalid_values_are_none() {
    let mut result = result(json!({
        "seeders": -1,
        "leechers": 2,
        "download_volume_factor": "fast",
        "minimum_seed_ratio": -0.5,
        "year": 0,
        "genre": "  "
    }));
    result.response_attributes.imdb_id = Some("nm123".into());
    let facts = result.release_facts();
    assert_eq!(facts.seeders, None);
    assert_eq!(facts.leechers, Some(2));
    assert_eq!(facts.peers, None, "peers needs both seeders and leechers");
    assert_eq!(facts.download_volume_factor, None);
    assert_eq!(facts.freeleech, None);
    assert_eq!(facts.minimum_seed_ratio, None);
    assert_eq!(facts.year, None);
    assert_eq!(facts.genre, None);
    assert_eq!(facts.external_ids.imdb, None);
    assert!(facts.indexer_flags.is_empty());
}
