use std::fmt;

use serde_json::Value;

const FORMAT_ZSTD_DICTIONARY_V1: u8 = 1;
const COMPRESSION_LEVEL: i32 = 3;
const MAX_EXPLANATION_BYTES: usize = 64 * 1024;
const DICTIONARY_V1: &[u8] = include_bytes!("release_decision_explanation_v1.dict");

#[derive(Debug)]
pub struct ReleaseDecisionExplanationCodecError(String);

impl fmt::Display for ReleaseDecisionExplanationCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub fn encode_release_decision_explanation(
    explanation_json: Option<&str>,
) -> Result<Option<Vec<u8>>, ReleaseDecisionExplanationCodecError> {
    let Some(explanation_json) = explanation_json else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(explanation_json)
        .map_err(|error| codec_error(format!("explanation is not valid JSON: {error}")))?;
    let canonical = serde_json::to_vec(&value).map_err(|error| {
        codec_error(format!("failed to canonicalize explanation JSON: {error}"))
    })?;
    if canonical.len() > MAX_EXPLANATION_BYTES {
        return Err(codec_error(format!(
            "explanation expands to {} bytes, exceeding the {MAX_EXPLANATION_BYTES}-byte limit",
            canonical.len()
        )));
    }

    let mut compressor = zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
        .map_err(|error| codec_error(format!("failed to initialize zstd compressor: {error}")))?;
    let compressed = compressor
        .compress(&canonical)
        .map_err(|error| codec_error(format!("failed to compress explanation JSON: {error}")))?;
    let mut encoded = Vec::with_capacity(1 + compressed.len());
    encoded.push(FORMAT_ZSTD_DICTIONARY_V1);
    encoded.extend_from_slice(&compressed);
    Ok(Some(encoded))
}

pub fn decode_release_decision_explanation(
    encoded: Option<&[u8]>,
) -> Result<Option<String>, ReleaseDecisionExplanationCodecError> {
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    let Some((&format, compressed)) = encoded.split_first() else {
        return Err(codec_error("explanation payload is empty"));
    };
    if format != FORMAT_ZSTD_DICTIONARY_V1 {
        return Err(codec_error(format!(
            "unsupported explanation storage format {format}"
        )));
    }

    let mut decompressor = zstd::bulk::Decompressor::with_dictionary(DICTIONARY_V1)
        .map_err(|error| codec_error(format!("failed to initialize zstd decompressor: {error}")))?;
    let decoded = decompressor
        .decompress(compressed, MAX_EXPLANATION_BYTES + 1)
        .map_err(|error| codec_error(format!("failed to decompress explanation JSON: {error}")))?;
    if decoded.len() > MAX_EXPLANATION_BYTES {
        return Err(codec_error(format!(
            "explanation expands beyond the {MAX_EXPLANATION_BYTES}-byte limit"
        )));
    }
    let value: Value = serde_json::from_slice(&decoded).map_err(|error| {
        codec_error(format!(
            "decompressed explanation is not valid JSON: {error}"
        ))
    })?;
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|error| codec_error(format!("failed to serialize explanation JSON: {error}")))
}

fn codec_error(message: impl Into<String>) -> ReleaseDecisionExplanationCodecError {
    ReleaseDecisionExplanationCodecError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    mod dictionary_training {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/support/release_decision_dictionary_training.rs"
        ));
    }

    const EXPECTED_DICTIONARY_BLAKE3: &str =
        "3c837a8f2ff701b4ba29266fd4a3f2e7e0648f4265def397cd253fd44a0af084";

    fn representative_explanation() -> String {
        serde_json::to_string(&json!({
            "candidate": {
                "source": "synthetic-indexer-2-Torznab",
                "source_kind": "torrent",
                "guid": "synthetic-guid-held-out",
                "download_url_present": true,
                "link_present": true,
                "external_id_conflicts": null,
            },
            "auto_decision": {
                "eligible": false,
                "code": "episode_mismatch",
                "summary": "Synthetic episode mismatch decision",
            },
            "quality_profile_decision": {
                "allowed": true,
                "block_codes": [],
                "release_score": 1150,
                "preference_score": 75,
                "scoring_log": [
                    {"code": "quality_tier", "delta": 1000},
                    {"code": "preferred_protocol", "delta": 50},
                    {"code": "release_group", "delta": 25},
                    {"code": "revision", "delta": 75},
                ],
            },
            "parsed": {
                "raw_title": "Synthetic.Show.S04E11.WEBDL-1080p.x265-PARITY",
                "normalized_title": "synthetic show",
                "normalized_title_variants": ["synthetic show", "synthetic show alternate"],
                "year": null,
                "quality": "WEBDL-1080p",
                "source": "Web",
                "release_group": "PARITY",
                "disposition": "Parsed",
                "parse_family": "Episode",
                "parse_confidence": 0.97,
                "is_ambiguous": false,
                "parse_hints": ["synthetic exact identity"],
            },
        }))
        .expect("representative explanation should serialize")
    }

    #[test]
    fn dictionary_identity_is_pinned() {
        assert_eq!(DICTIONARY_V1.len(), 8 * 1024);
        assert_eq!(
            blake3::hash(DICTIONARY_V1).to_hex().as_str(),
            EXPECTED_DICTIONARY_BLAKE3
        );
    }

    #[test]
    fn dictionary_regenerates_exactly_and_improves_held_out_compression() {
        let trained = dictionary_training::train_dictionary().unwrap();
        assert_eq!(trained.bytes, DICTIONARY_V1);
        assert_eq!(trained.training_samples, 807);
        assert_eq!(trained.held_out.len(), 74);
        let (raw, plain, dictionary) =
            dictionary_training::held_out_compression_totals(&trained.bytes, &trained.held_out)
                .unwrap();
        assert!(raw > 0);
        assert!(
            dictionary < plain,
            "dictionary compression should beat plain level-3 zstd across held-out samples ({dictionary} >= {plain})"
        );
    }

    #[test]
    fn explanation_round_trips_and_meets_compression_target() {
        let explanation = representative_explanation();
        let encoded = encode_release_decision_explanation(Some(&explanation))
            .expect("explanation should encode")
            .expect("explanation should be present");
        let decoded = decode_release_decision_explanation(Some(&encoded))
            .expect("explanation should decode")
            .expect("explanation should be present");

        assert_eq!(decoded, explanation);
        assert_eq!(
            serde_json::from_str::<Value>(&decoded).expect("decoded explanation should be JSON"),
            serde_json::from_str::<Value>(&explanation).expect("source explanation should be JSON")
        );
        let without_dictionary = zstd::bulk::compress(explanation.as_bytes(), COMPRESSION_LEVEL)
            .expect("dictionary-free compression should succeed");
        assert!(
            encoded.len() - 1 < without_dictionary.len(),
            "dictionary compression should beat plain level-3 zstd ({} >= {})",
            encoded.len() - 1,
            without_dictionary.len()
        );
    }

    #[test]
    fn null_explanation_round_trips() {
        assert_eq!(encode_release_decision_explanation(None).unwrap(), None);
        assert_eq!(decode_release_decision_explanation(None).unwrap(), None);
    }

    #[test]
    fn invalid_and_oversized_json_is_rejected() {
        assert!(encode_release_decision_explanation(Some("not-json")).is_err());
        let oversized = serde_json::to_string(&"x".repeat(MAX_EXPLANATION_BYTES))
            .expect("oversized explanation should serialize");
        assert!(encode_release_decision_explanation(Some(&oversized)).is_err());
    }

    #[test]
    fn unknown_empty_and_corrupt_payloads_are_rejected() {
        assert!(decode_release_decision_explanation(Some(&[])).is_err());
        assert!(decode_release_decision_explanation(Some(&[2, 0])).is_err());
        assert!(decode_release_decision_explanation(Some(&[1, 0, 1, 2, 3])).is_err());
    }

    #[test]
    fn invalid_decoded_json_is_rejected() {
        let mut compressor =
            zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
                .expect("compressor should initialize");
        let mut encoded = vec![FORMAT_ZSTD_DICTIONARY_V1];
        encoded.extend(
            compressor
                .compress(b"not-json")
                .expect("invalid JSON should still compress"),
        );

        assert!(decode_release_decision_explanation(Some(&encoded)).is_err());
    }

    #[test]
    fn oversized_decoded_json_is_rejected() {
        let oversized = serde_json::to_vec(&"x".repeat(MAX_EXPLANATION_BYTES))
            .expect("oversized explanation should serialize");
        let mut compressor =
            zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
                .expect("compressor should initialize");
        let mut encoded = vec![FORMAT_ZSTD_DICTIONARY_V1];
        encoded.extend(
            compressor
                .compress(&oversized)
                .expect("oversized explanation should compress"),
        );
        assert!(decode_release_decision_explanation(Some(&encoded)).is_err());
    }
}
