use std::path::PathBuf;

use serde_json::json;

pub const DICTIONARY_BYTES: usize = 8 * 1024;

pub struct TrainedDictionary {
    pub bytes: Vec<u8>,
    pub training_samples: usize,
    pub held_out: Vec<Vec<u8>>,
}

pub fn train_dictionary() -> Result<TrainedDictionary, String> {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/corpora/domain_event_payload_sanitized.jsonl");
    let samples = scryer_infrastructure_sql::sanitized_corpus::load_sanitized_jsonl(&corpus)?;
    let split = scryer_infrastructure_sql::sanitized_corpus::split_sanitized_samples(samples)?;
    let mut training = split.training;
    training.extend(synthetic_samples());
    let refs = training.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let bytes = zstd::dict::from_samples(&refs, DICTIONARY_BYTES)
        .map_err(|error| format!("failed to train domain-event dictionary: {error}"))?;
    if bytes.len() != DICTIONARY_BYTES {
        return Err(format!(
            "trained domain-event dictionary is {} bytes, expected {DICTIONARY_BYTES}",
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
        .map_err(|error| format!("failed to initialize domain-event compressor: {error}"))?;
    for sample in held_out {
        raw_bytes += sample.len();
        plain_bytes += zstd::bulk::compress(sample, 3)
            .map_err(|error| format!("failed to compress held-out domain event: {error}"))?
            .len();
        dictionary_bytes += compressor
            .compress(sample)
            .map_err(|error| {
                format!("failed to dictionary-compress held-out domain event: {error}")
            })?
            .len();
    }
    Ok((raw_bytes, plain_bytes, dictionary_bytes))
}

fn synthetic_samples() -> Vec<Vec<u8>> {
    let event_types = [
        "release_grabbed",
        "import_completed",
        "import_rejected",
        "media_file_deleted",
        "media_file_upgraded",
        "download_failed",
        "job_run_started",
        "job_run_completed",
        "library_scan_started",
        "library_scan_completed",
        "seeding_started",
        "seeding_completed",
    ];
    (0..4096)
        .map(|index| {
            let client_type = ["usenet", "torrent", "weaver", "qbittorrent"][index % 4];
            let quality = ["WEBDL-1080p", "Bluray-1080p", "HDTV-720p", "WEBDL-2160p"][index % 4];
            let items = (0..(index % 6))
                .map(|item| {
                    let state = ["queued", "downloading", "completed", "failed"][item % 4];
                    json!({
                        "id": format!("<item-id:{}>", item),
                        "state": state,
                        "progress": (index + item) % 101,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::to_vec(&json!({
                "type": event_types[index % event_types.len()],
                "data": {
                    "title_id": format!("<title-id:{}>", index % 4),
                    "download_id": format!("<download-id:{}>", index % 6),
                    "status": if index % 3 == 0 { "completed" } else { "failed" },
                    "reason": if index % 5 == 0 { "upgrade_cleanup" } else { "<message:medium>" },
                    "source_path": "<absolute-path:long>",
                    "destination_path": "<absolute-path:long>",
                    "episode_ids": ["<episode-id:medium>"],
                    "collection_id": format!("<collection-id:{}>", index % 11),
                    "client_id": format!("<client-id:{}>", index % 13),
                    "client_type": client_type,
                    "quality": quality,
                    "source_title": format!("<release-title:{}>", index % 17),
                    "source_provider": format!("<provider-name:{}>", index % 19),
                    "correlation_id": format!("<correlation-id:{}>", index % 23),
                    "items": items,
                    "size_bytes": (index % 8) * 1024,
                    "successful": index % 3 == 0,
                }
            }))
            .expect("synthetic event should serialize")
        })
        .collect()
}
