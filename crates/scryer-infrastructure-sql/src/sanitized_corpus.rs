use std::{fs, path::Path};

use serde_json::Value;

const NUMBER_BUCKETS: &[i64] = &[0, 1, 10, 100, 1_000, 1_000_000];
const SAFE_VALUES: &[&str] = &[
    "movie",
    "series",
    "anime",
    "torrent",
    "usenet",
    "completed",
    "failed",
    "skipped",
    "queued",
    "running",
    "eligible",
    "blocked",
    "import_rejected",
    "import_completed",
    "release_grabbed",
    "release_blocklisted",
    "download_failed",
    "media_file_deleted",
    "media_file_upgraded",
    "upgrade_cleanup",
    "deleted",
    "missing_on_disk",
    "recycle_bin_purged",
    "system",
    "global",
    "title",
    "job_run",
    "library_scan",
    "download_queue_item",
    "WEBDL-1080p",
    "WEBDL-2160p",
    "Bluray-1080p",
    "HDTV-720p",
    "Parsed",
    "NeedsReview",
    "Episode",
    "Movie",
    "Web",
    "BluRay",
    "pending_delay",
    "minimum_age",
    "protocol_disabled",
    "quality_blocked",
    "episode_mismatch",
    "title_mismatch",
    "category_mismatch",
    "ambiguous_identity",
    "download_client_unavailable",
    "quality_tier",
    "preferred_protocol",
    "revision",
];

pub fn load_sanitized_jsonl(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "failed to read sanitized corpus {}: {error}",
            path.display()
        )
    })?;
    let mut samples = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            format!(
                "sanitized corpus line {} is invalid JSON: {error}",
                index + 1
            )
        })?;
        audit_sanitized_value(&value, "$").map_err(|error| {
            format!("sanitized corpus line {} failed audit: {error}", index + 1)
        })?;
        samples.push(serde_json::to_vec(&value).map_err(|error| error.to_string())?);
    }
    if samples.is_empty() {
        return Err("sanitized corpus is empty".to_string());
    }
    Ok(samples)
}

pub struct SanitizedCorpusSplit {
    pub training: Vec<Vec<u8>>,
    pub held_out: Vec<Vec<u8>>,
}

pub fn split_sanitized_samples(mut samples: Vec<Vec<u8>>) -> Result<SanitizedCorpusSplit, String> {
    samples.sort_by(|left, right| {
        stable_content_hash(left)
            .cmp(&stable_content_hash(right))
            .then_with(|| left.cmp(right))
    });
    let training_len = samples.len() * 4 / 5;
    if training_len == 0 || training_len == samples.len() {
        return Err("sanitized corpus content-hash split produced an empty partition".to_string());
    }
    let held_out = samples.split_off(training_len);
    let training = samples;
    Ok(SanitizedCorpusSplit { training, held_out })
}

fn stable_content_hash(sample: &[u8]) -> u64 {
    sample.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn audit_sanitized_value(value: &Value, path: &str) -> Result<(), String> {
    match value {
        Value::Null | Value::Bool(_) => Ok(()),
        Value::Number(number) => number
            .as_i64()
            .filter(|number| NUMBER_BUCKETS.contains(number))
            .map(|_| ())
            .ok_or_else(|| format!("{path} contains a non-bucketed number")),
        Value::String(value) => {
            if SAFE_VALUES.contains(&value.as_str()) || is_placeholder(value) {
                Ok(())
            } else {
                Err(format!("{path} contains a non-allowlisted string"))
            }
        }
        Value::Array(values) => {
            if values.len() > 8 {
                return Err(format!("{path} exceeds the bounded collection size"));
            }
            for (index, value) in values.iter().enumerate() {
                audit_sanitized_value(value, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, value) in values {
                audit_sanitized_value(value, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
    }
}

fn is_placeholder(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    else {
        return false;
    };
    let Some((field, bucket)) = inner.rsplit_once(':') else {
        return false;
    };
    !field.is_empty()
        && field
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && matches!(bucket, "short" | "medium" | "long" | "very-long")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sensitive_shapes_and_unbucketed_identifiers() {
        for value in [
            "https://example.invalid/query",
            "/volume/media/title.mkv",
            "550e8400-e29b-41d4-a716-446655440000",
            "0123456789abcdef0123456789abcdef01234567",
            "Example.Show.S01E01.1080p-GROUP",
            "provider-person-name",
        ] {
            assert!(audit_sanitized_value(&Value::String(value.into()), "$").is_err());
        }
        assert!(audit_sanitized_value(&Value::from(42), "$").is_err());
    }

    #[test]
    fn accepts_allowlisted_values_placeholders_and_buckets() {
        let value = serde_json::json!({
            "type": "import_completed",
            "title": "<source-title:long>",
            "count": 10,
            "items": [true, null, "torrent"]
        });
        audit_sanitized_value(&value, "$").unwrap();
    }

    #[test]
    fn content_hash_split_is_exact_and_input_order_independent() {
        let samples = (0..10)
            .map(|index| format!("sample-{index}").into_bytes())
            .collect::<Vec<_>>();
        let expected = split_sanitized_samples(samples.clone()).unwrap();
        let actual = split_sanitized_samples(samples.into_iter().rev().collect()).unwrap();

        assert_eq!(expected.training.len(), 8);
        assert_eq!(expected.held_out.len(), 2);
        assert_eq!(actual.training, expected.training);
        assert_eq!(actual.held_out, expected.held_out);
    }

    #[test]
    fn checked_in_sanitized_corpora_are_pinned_and_audited() {
        let sql_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cases = [
            (
                sql_root.join("examples/corpora/domain_event_payload_sanitized.jsonl"),
                "878cc0df6db956eb79a35589765855fc914da0113e722f9807dbbaf1969388a5",
            ),
            (
                sql_root.join("../scryer-infrastructure-library/examples/corpora/release_decision_explanation_sanitized.jsonl"),
                "6445c720811438288ecd9f5ef2279b883226ab48a4d8e1110463decd213a5208",
            ),
        ];
        for (path, expected_hash) in cases {
            let bytes = fs::read(&path).expect("sanitized corpus should be readable");
            assert_eq!(blake3::hash(&bytes).to_hex().as_str(), expected_hash);
            load_sanitized_jsonl(&path).expect("sanitized corpus should pass audit");
        }
    }
}
