use std::path::PathBuf;

use serde_json::json;

pub const DICTIONARY_BYTES: usize = 8 * 1024;
const SYNTHETIC_SAMPLE_COUNT: usize = 512;

pub struct TrainedDictionary {
    pub bytes: Vec<u8>,
    pub training_samples: usize,
    pub held_out: Vec<Vec<u8>>,
}

pub fn train_dictionary() -> Result<TrainedDictionary, String> {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/corpora/release_decision_explanation_sanitized.jsonl");
    let samples = scryer_infrastructure_sql::sanitized_corpus::load_sanitized_jsonl(&corpus)?;
    let split = scryer_infrastructure_sql::sanitized_corpus::split_sanitized_samples(samples)?;
    let mut training = split.training;
    training.extend(synthetic_samples());
    let refs = training.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let bytes = zstd::dict::from_samples(&refs, DICTIONARY_BYTES)
        .map_err(|error| format!("failed to train release-decision dictionary: {error}"))?;
    if bytes.len() != DICTIONARY_BYTES {
        return Err(format!(
            "trained release-decision dictionary is {} bytes, expected {DICTIONARY_BYTES}",
            bytes.len()
        ));
    }
    Ok(TrainedDictionary {
        bytes,
        training_samples: training.len(),
        held_out: split.held_out,
    })
}

pub fn held_out_compression_totals(
    dictionary: &[u8],
    held_out: &[Vec<u8>],
) -> Result<(usize, usize, usize), String> {
    let mut raw_bytes = 0;
    let mut plain_bytes = 0;
    let mut dictionary_bytes = 0;
    let mut compressor = zstd::bulk::Compressor::with_dictionary(3, dictionary)
        .map_err(|error| format!("failed to initialize release-decision compressor: {error}"))?;
    for sample in held_out {
        raw_bytes += sample.len();
        plain_bytes += zstd::bulk::compress(sample, 3)
            .map_err(|error| format!("failed to compress held-out release decision: {error}"))?
            .len();
        dictionary_bytes += compressor
            .compress(sample)
            .map_err(|error| {
                format!("failed to dictionary-compress held-out release decision: {error}")
            })?
            .len();
    }
    Ok((raw_bytes, plain_bytes, dictionary_bytes))
}

fn synthetic_samples() -> Vec<Vec<u8>> {
    let release_stems = [
        "Synthetic.Movie.2026",
        "Synthetic.Show.S03E07",
        "Synthetic.Show.S03.1080p.Pack",
        "Synthetic.Anime.042",
        "Synthetic.Daily.2026.08.28",
        "Synthetic.Multi.S01E03-E05",
        "Synthetic.Specials.S00E04",
        "Synthetic.Movie.Extended.Cut.2024",
    ];
    let decisions = [
        ("eligible", true),
        ("title_mismatch", false),
        ("episode_mismatch", false),
        ("category_mismatch", false),
        ("ambiguous_identity", false),
        ("quality_blocked", false),
        ("minimum_seeders", false),
        ("pack_below_missing_threshold", false),
        ("upgrade_rejected", false),
        ("pending_delay", false),
        ("minimum_age", false),
        ("protocol_disabled", false),
        ("download_client_unavailable", false),
        ("queued_better_or_equal", false),
    ];
    let qualities = ["HDTV-720p", "WEBDL-1080p", "Bluray-1080p", "WEBDL-2160p"];
    let sources = ["Torznab", "Newznab"];
    let groups = ["SYNTH", "EXAMPLE", "FIXTURE", "TESTGROUP"];

    (0..SYNTHETIC_SAMPLE_COUNT)
        .map(|index| {
            let stem = release_stems[index % release_stems.len()];
            let (decision_code, eligible) = decisions[index % decisions.len()];
            let quality = qualities[index % qualities.len()];
            let source = sources[index % sources.len()];
            let group = groups[index % groups.len()];
            let raw_title = format!(
                "{stem}.{quality}.x265.{}-{group}",
                if index % 3 == 0 { "PROPER" } else { "AAC" }
            );
            let scoring_log = [
                json!({"code": "quality_tier", "delta": 800 + (index % 4) * 100}),
                json!({"code": "preferred_protocol", "delta": if index % 2 == 0 { 50 } else { 0 }}),
                json!({"code": "release_group", "delta": (index % 7) as i32 - 3}),
                json!({"code": "revision", "delta": if index % 3 == 0 { 25 } else { 0 }}),
            ];
            let parsed = (index % 11 != 0).then(|| {
                json!({
                    "raw_title": raw_title,
                    "normalized_title": stem.replace('.', " ").to_lowercase(),
                    "normalized_title_variants": [
                        stem.replace('.', " ").to_lowercase(),
                        format!("{} alternate", stem.replace('.', " ").to_lowercase()),
                    ],
                    "year": if stem.contains("Movie") { Some(2026) } else { None },
                    "quality": quality,
                    "source": if quality.contains("Bluray") { "BluRay" } else { "Web" },
                    "release_group": group,
                    "disposition": if index % 5 == 0 { "NeedsReview" } else { "Parsed" },
                    "parse_family": if stem.contains("Movie") { "Movie" } else { "Episode" },
                    "parse_confidence": 0.70 + ((index % 30) as f64 / 100.0),
                    "is_ambiguous": index % 5 == 0,
                    "parse_hints": if index % 5 == 0 {
                        vec!["synthetic ambiguous numbering", "synthetic title alias"]
                    } else {
                        vec!["synthetic exact identity"]
                    },
                })
            });
            serde_json::to_vec(&json!({
                "candidate": {
                    "source": format!("synthetic-indexer-{}-{source}", index % 6),
                    "source_kind": if index % 2 == 0 { "torrent" } else { "usenet" },
                    "guid": format!("synthetic-guid-{index:04}"),
                    "download_url_present": index % 7 != 0,
                    "link_present": index % 9 != 0,
                    "external_id_conflicts": if index % 13 == 0 {
                        Some(json!([{"source": "tvdb", "expected": "100", "found": "200"}]))
                    } else {
                        None
                    },
                },
                "auto_decision": {
                    "eligible": eligible,
                    "code": decision_code,
                    "summary": format!("Synthetic {decision_code} decision"),
                },
                "quality_profile_decision": {
                    "allowed": decision_code != "quality_blocked",
                    "block_codes": if decision_code == "quality_blocked" {
                        vec!["quality_not_allowed", "below_cutoff"]
                    } else {
                        Vec::<&str>::new()
                    },
                    "release_score": 900 + (index % 400) as i32,
                    "preference_score": (index % 175) as i32 - 25,
                    "scoring_log": scoring_log,
                },
                "parsed": parsed,
            }))
            .expect("synthetic explanation should serialize")
        })
        .collect()
}
