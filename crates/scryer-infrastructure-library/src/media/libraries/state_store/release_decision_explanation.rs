use std::fmt;
use std::sync::LazyLock;

use scryer_application::ReleaseDecisionExplanation;
use scryer_infrastructure_sql::json_shape::{ValueParseShape, check_value_parse};
use serde_json::Value;
use zstd::dict::DecoderDictionary;

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
    ReleaseDecisionExplanationEncoder::new().encode(explanation_json)
}

/// Encodes a run of explanations through one zstd context, so the dictionary
/// is loaded and digested once for the run instead of once per explanation.
/// Each explanation encodes to exactly the bytes
/// [`encode_release_decision_explanation`] produces for it alone.
#[derive(Default)]
pub struct ReleaseDecisionExplanationEncoder {
    compressor: Option<zstd::bulk::Compressor<'static>>,
}

impl ReleaseDecisionExplanationEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn encode(
        &mut self,
        explanation_json: Option<&str>,
    ) -> Result<Option<Vec<u8>>, ReleaseDecisionExplanationCodecError> {
        let Some(explanation_json) = explanation_json else {
            return Ok(None);
        };
        let canonical = canonical_explanation(explanation_json)?;
        self.compress_canonical(&canonical).map(Some)
    }

    /// Encodes a decision's explanation. Text this process serialized is
    /// compressed as it is; any other text is canonicalized first, exactly as
    /// [`Self::encode`] does.
    pub fn encode_explanation(
        &mut self,
        explanation: Option<&ReleaseDecisionExplanation>,
    ) -> Result<Option<Vec<u8>>, ReleaseDecisionExplanationCodecError> {
        let Some(explanation) = explanation else {
            return Ok(None);
        };
        match explanation.canonical_text() {
            Some(canonical) => self.compress_canonical(canonical.as_bytes()).map(Some),
            None => self.encode(Some(explanation.as_str())),
        }
    }

    fn compress_canonical(
        &mut self,
        canonical: &[u8],
    ) -> Result<Vec<u8>, ReleaseDecisionExplanationCodecError> {
        if canonical.len() > MAX_EXPLANATION_BYTES {
            return Err(codec_error(format!(
                "explanation expands to {} bytes, exceeding the {MAX_EXPLANATION_BYTES}-byte limit",
                canonical.len()
            )));
        }
        let compressor = match &mut self.compressor {
            Some(compressor) => compressor,
            None => self.compressor.insert(
                zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1).map_err(
                    |error| codec_error(format!("failed to initialize zstd compressor: {error}")),
                )?,
            ),
        };
        let compressed = match compressor.compress(canonical) {
            Ok(compressed) => compressed,
            Err(error) => {
                // Start the next explanation from a fresh context rather than
                // trust one that just failed.
                self.compressor = None;
                return Err(codec_error(format!(
                    "failed to compress explanation JSON: {error}"
                )));
            }
        };
        let mut encoded = Vec::with_capacity(1 + compressed.len());
        encoded.push(FORMAT_ZSTD_DICTIONARY_V1);
        encoded.extend_from_slice(&compressed);
        Ok(encoded)
    }
}

fn canonical_explanation(
    explanation_json: &str,
) -> Result<Vec<u8>, ReleaseDecisionExplanationCodecError> {
    let value: Value = serde_json::from_str(explanation_json)
        .map_err(|error| codec_error(format!("explanation is not valid JSON: {error}")))?;
    serde_json::to_vec(&value)
        .map_err(|error| codec_error(format!("failed to canonicalize explanation JSON: {error}")))
}

pub fn decode_release_decision_explanation(
    encoded: Option<&[u8]>,
) -> Result<Option<String>, ReleaseDecisionExplanationCodecError> {
    Ok(decode_release_decision_explanation_record(encoded)?
        .map(ReleaseDecisionExplanation::into_string))
}

/// Decodes a stored explanation into the form a release decision carries.
///
/// Every writer stores the compact text serde_json writes for a parsed value,
/// and that text parses back to a value that writes it again byte for byte,
/// so the stored text is returned as it is rather than parsed and written a
/// second time. It is still checked to be JSON exactly as a parse into a
/// value would check it. Only text carrying serde_json's raw-value marker is
/// parsed and rewritten; any other stored text is trusted to be canonical.
pub(crate) fn decode_release_decision_explanation_record(
    encoded: Option<&[u8]>,
) -> Result<Option<ReleaseDecisionExplanation>, ReleaseDecisionExplanationCodecError> {
    let Some(decoded) = decompress_explanation(encoded)? else {
        return Ok(None);
    };
    match check_value_parse(&decoded).map_err(invalid_decoded_json)? {
        ValueParseShape::Plain => {}
        ValueParseShape::ExpandsRawValue => {
            let value: Value = serde_json::from_slice(&decoded).map_err(invalid_decoded_json)?;
            return Ok(Some(ReleaseDecisionExplanation::from_value(&value)));
        }
    }
    // A value parse accepted the bytes, so they are UTF-8.
    let text = String::from_utf8(decoded).map_err(|error| {
        codec_error(format!(
            "decompressed explanation is not valid JSON: {error}"
        ))
    })?;
    Ok(Some(ReleaseDecisionExplanation::stored_canonical(text)))
}

fn invalid_decoded_json(error: serde_json::Error) -> ReleaseDecisionExplanationCodecError {
    codec_error(format!(
        "decompressed explanation is not valid JSON: {error}"
    ))
}

// Digested once per process, as the domain-event payload decoder's is.
static DECODER_DICTIONARY_V1: LazyLock<Result<DecoderDictionary<'static>, String>> =
    LazyLock::new(|| DecoderDictionary::try_copy(DICTIONARY_V1).map_err(|error| error.to_string()));

fn decompress_explanation(
    encoded: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, ReleaseDecisionExplanationCodecError> {
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

    let dictionary = (*DECODER_DICTIONARY_V1)
        .as_ref()
        .map_err(|error| codec_error(format!("failed to initialize zstd decompressor: {error}")))?;
    let mut decompressor = zstd::bulk::Decompressor::with_prepared_dictionary(dictionary)
        .map_err(|error| codec_error(format!("failed to initialize zstd decompressor: {error}")))?;
    let decoded = decompressor
        .decompress(compressed, MAX_EXPLANATION_BYTES + 1)
        .map_err(|error| codec_error(format!("failed to decompress explanation JSON: {error}")))?;
    if decoded.len() > MAX_EXPLANATION_BYTES {
        return Err(codec_error(format!(
            "explanation expands beyond the {MAX_EXPLANATION_BYTES}-byte limit"
        )));
    }
    Ok(Some(decoded))
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

    /// The per-explanation encoder as it stood before explanations shared a
    /// zstd context: a fresh compressor, and its dictionary, for every call.
    fn fresh_context_encode(explanation_json: Option<&str>) -> Option<Vec<u8>> {
        let explanation_json = explanation_json?;
        let value: Value = serde_json::from_str(explanation_json).ok()?;
        let canonical = serde_json::to_vec(&value).ok()?;
        if canonical.len() > MAX_EXPLANATION_BYTES {
            return None;
        }
        let mut compressor =
            zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
                .expect("compressor should initialize");
        let compressed = compressor
            .compress(&canonical)
            .expect("explanation should compress");
        let mut encoded = vec![FORMAT_ZSTD_DICTIONARY_V1];
        encoded.extend_from_slice(&compressed);
        Some(encoded)
    }

    fn mixed_explanations() -> Vec<Option<String>> {
        let representative = representative_explanation();
        let oversized = serde_json::to_string(&"x".repeat(MAX_EXPLANATION_BYTES))
            .expect("oversized explanation should serialize");
        let mut explanations = vec![
            Some(representative.clone()),
            None,
            Some("not-json".to_string()),
            Some(format!("  {}\n", representative.replace(',', ", "))),
            Some(oversized),
            Some(r#"{"b":1,"a":{"z":[1,2.50,"é"],"y":null},"b":2}"#.to_string()),
            Some(representative.clone()),
        ];
        for index in 0..40 {
            let mut value: Value = serde_json::from_str(&representative).unwrap();
            value["parsed"]["raw_title"] =
                Value::String(format!("Synthetic.Show.S01E{index:02}.WEBDL-1080p-PARITY"));
            value["quality_profile_decision"]["release_score"] = Value::from(index * 37);
            let hints = (0..index)
                .map(|hint| Value::String(format!("synthetic hint {hint}")))
                .collect::<Vec<_>>();
            value["parsed"]["parse_hints"] = Value::Array(hints);
            explanations.push(Some(serde_json::to_string(&value).unwrap()));
        }
        explanations
    }

    #[test]
    fn shared_context_encodes_byte_identically_to_a_fresh_context() {
        let explanations = mixed_explanations();
        let mut encoder = ReleaseDecisionExplanationEncoder::new();
        for explanation in &explanations {
            let shared = encoder.encode(explanation.as_deref());
            let single = encode_release_decision_explanation(explanation.as_deref());
            let fresh = fresh_context_encode(explanation.as_deref());
            match shared {
                Ok(shared) => {
                    assert_eq!(shared, fresh, "shared context must match a fresh context");
                    assert_eq!(shared, single.expect("single-row encode should agree"));
                    if let (Some(encoded), Some(source)) = (&shared, explanation) {
                        let decoded = decode_release_decision_explanation(Some(encoded))
                            .expect("explanation should decode")
                            .expect("explanation should be present");
                        assert_eq!(
                            serde_json::from_str::<Value>(&decoded).unwrap(),
                            serde_json::from_str::<Value>(source).unwrap()
                        );
                    }
                }
                Err(_) => {
                    assert!(fresh.is_none(), "a rejected explanation stays rejected");
                    assert!(single.is_err(), "single-row encode should reject it too");
                }
            }
        }
    }

    #[test]
    fn serialized_explanations_skip_reparsing_and_encode_byte_identically() {
        let mut encoder = ReleaseDecisionExplanationEncoder::new();
        for explanation in mixed_explanations().into_iter().flatten() {
            let Ok(value) = serde_json::from_str::<Value>(&explanation) else {
                continue;
            };
            let serialized = ReleaseDecisionExplanation::from_value(&value);
            assert!(serialized.canonical_text().is_some());
            let direct = encoder.encode_explanation(Some(&serialized));
            let reparsed = encode_release_decision_explanation(Some(serialized.as_str()));
            match direct {
                Ok(direct) => {
                    assert_eq!(direct, reparsed.expect("reparsed encode should agree"));
                    assert_eq!(direct, fresh_context_encode(Some(serialized.as_str())));
                    let decoded =
                        decode_release_decision_explanation_record(direct.as_deref()).unwrap();
                    assert_eq!(decoded, Some(serialized));
                }
                Err(_) => assert!(reparsed.is_err(), "both paths reject it"),
            }
        }
    }

    #[test]
    fn untrusted_explanations_keep_canonicalization() {
        let mut encoder = ReleaseDecisionExplanationEncoder::new();
        for explanation in mixed_explanations().into_iter().flatten() {
            let untrusted = ReleaseDecisionExplanation::untrusted(explanation.clone());
            assert!(untrusted.canonical_text().is_none());
            let encoded = encoder.encode_explanation(Some(&untrusted));
            let old = encode_release_decision_explanation(Some(&explanation));
            match (encoded, old) {
                (Ok(encoded), Ok(old)) => assert_eq!(encoded, old),
                (Err(encoded), Err(old)) => assert_eq!(encoded.to_string(), old.to_string()),
                _ => panic!("untrusted text must encode exactly as before: {explanation}"),
            }
        }
        assert_eq!(encoder.encode_explanation(None).unwrap(), None);
    }

    #[test]
    fn oversized_serialized_explanation_is_rejected() {
        let value = Value::String("x".repeat(MAX_EXPLANATION_BYTES));
        let serialized = ReleaseDecisionExplanation::from_value(&value);
        let mut encoder = ReleaseDecisionExplanationEncoder::new();
        assert_eq!(
            encoder
                .encode_explanation(Some(&serialized))
                .unwrap_err()
                .to_string(),
            encode_release_decision_explanation(Some(serialized.as_str()))
                .unwrap_err()
                .to_string()
        );
    }

    /// The decoder as it stood before stored text was carried as it is: a
    /// fresh raw-dictionary context per row, a parse into a value, and the
    /// value written out again.
    fn reparsing_decode(encoded: &[u8]) -> Result<String, ()> {
        let (&format, compressed) = encoded.split_first().ok_or(())?;
        if format != FORMAT_ZSTD_DICTIONARY_V1 {
            return Err(());
        }
        let mut decompressor =
            zstd::bulk::Decompressor::with_dictionary(DICTIONARY_V1).map_err(|_| ())?;
        let decoded = decompressor
            .decompress(compressed, MAX_EXPLANATION_BYTES + 1)
            .map_err(|_| ())?;
        if decoded.len() > MAX_EXPLANATION_BYTES {
            return Err(());
        }
        let value: Value = serde_json::from_slice(&decoded).map_err(|_| ())?;
        serde_json::to_string(&value).map_err(|_| ())
    }

    fn compress_raw(bytes: &[u8]) -> Vec<u8> {
        let mut compressor =
            zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
                .expect("compressor should initialize");
        let mut encoded = vec![FORMAT_ZSTD_DICTIONARY_V1];
        encoded.extend(compressor.compress(bytes).expect("bytes should compress"));
        encoded
    }

    fn assert_decodes_as_before(encoded: &[u8]) {
        let before = reparsing_decode(encoded);
        let text = decode_release_decision_explanation(Some(encoded));
        let record = decode_release_decision_explanation_record(Some(encoded));
        match before {
            Ok(before) => {
                assert_eq!(
                    text.expect("decode should agree").as_deref(),
                    Some(&*before)
                );
                let record = record.expect("record decode should agree").unwrap();
                assert_eq!(record.as_str(), before);
                assert_eq!(record.canonical_text(), Some(before.as_str()));
            }
            Err(()) => {
                assert!(text.is_err(), "rejected before, rejected now: {encoded:?}");
                assert!(record.is_err());
            }
        }
    }

    #[test]
    fn stored_text_decodes_byte_identically_to_a_reparse() {
        let mut encoder = ReleaseDecisionExplanationEncoder::new();
        let mut values = mixed_explanations()
            .into_iter()
            .flatten()
            .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
            .collect::<Vec<_>>();
        values.extend([
            json!({"floats": [0.1, -0.0, 1e300, 5e-324, 1.0, 123456.789, f64::MAX, f64::MIN_POSITIVE]}),
            json!({"ints": [0, -1, u64::MAX, i64::MIN], "nested": [[[{"k": null}]]]}),
            json!({"escapes": "quote \" backslash \\ tab \t nl \n ctl \u{1} del \u{7f} slash / é 😀"}),
            json!({"z": 1, "a": 2, "m": {"y": [], "b": {}}}),
            json!([]),
            json!("bare string"),
            json!(null),
        ]);
        for value in values {
            let explanation = ReleaseDecisionExplanation::from_value(&value);
            let Ok(Some(encoded)) = encoder.encode_explanation(Some(&explanation)) else {
                continue;
            };
            assert_decodes_as_before(&encoded);
            assert_eq!(
                decode_release_decision_explanation(Some(&encoded))
                    .unwrap()
                    .as_deref(),
                Some(explanation.as_str())
            );
        }
    }

    /// Text no writer stores: accepted and rejected exactly as before, and
    /// when accepted it is the same JSON value. Whitespace, duplicate keys and
    /// other non-canonical spellings come back as stored rather than
    /// rewritten; text in canonical form comes back byte for byte.
    fn assert_decodes_equivalently(encoded: &[u8]) {
        let before = reparsing_decode(encoded);
        let text = decode_release_decision_explanation(Some(encoded));
        let record = decode_release_decision_explanation_record(Some(encoded));
        match before {
            Ok(before) => {
                let text = text.expect("accepted before, accepted now").unwrap();
                assert_eq!(record.unwrap().unwrap().as_str(), text);
                assert_eq!(
                    serde_json::from_str::<Value>(&text).unwrap(),
                    serde_json::from_str::<Value>(&before).unwrap()
                );
                let stored = &encoded[1..];
                let mut decompressor =
                    zstd::bulk::Decompressor::with_dictionary(DICTIONARY_V1).unwrap();
                let raw = decompressor
                    .decompress(stored, MAX_EXPLANATION_BYTES + 1)
                    .unwrap();
                if raw == before.as_bytes() {
                    assert_eq!(text, before);
                }
            }
            Err(()) => {
                assert!(text.is_err(), "rejected before, rejected now: {encoded:?}");
                assert!(record.is_err());
            }
        }
    }

    #[test]
    fn raw_value_token_objects_decode_exactly_as_a_reparse() {
        for sample in [
            &br#"{"$serde_json::private::RawValue":"{\"x\": 1}"}"#[..],
            br#"{"$serde_json::private::RawValue":7}"#,
            br#"{"a":{"$serde_json::private::RawValue":"[1, 2]"}}"#,
            br#"[{"$serde_json::private::RawValue":"{\"b\":2,  \"a\":1}"}]"#,
        ] {
            assert_decodes_as_before(&compress_raw(sample));
        }
    }

    #[test]
    fn foreign_and_malformed_stored_text_decodes_as_a_reparse_does() {
        let deep_ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
        let too_deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        let samples: Vec<Vec<u8>> = vec![
            br#"{"a": 1 , "b" : [1, 2]}"#.to_vec(),
            br#"{"a":1,"a":2}"#.to_vec(),
            br#"{"$serde_json::private::RawValue":"{\"x\": 1}"}"#.to_vec(),
            br#"{"$serde_json::private::RawValue":7}"#.to_vec(),
            br#"{"a":{"$serde_json::private::RawValue":"[1, 2]"}}"#.to_vec(),
            br#"{"a":1,"$serde_json::private::RawValue":"[1]"}"#.to_vec(),
            br#"[1e400]"#.to_vec(),
            br#"["\ud800"]"#.to_vec(),
            [&br#"[""#[..], br"\", b"ud83d", br"\", b"ude00", br#""]"#].concat(),
            br#"{"a":1} trailing"#.to_vec(),
            br#"{"a":1}  "#.to_vec(),
            br#"18446744073709551616"#.to_vec(),
            br#"1E2"#.to_vec(),
            b"[\"\xff\"]".to_vec(),
            b"\xef\xbb\xbf{}".to_vec(),
            b"".to_vec(),
            b"not-json".to_vec(),
            deep_ok.into_bytes(),
            too_deep.into_bytes(),
        ];
        for sample in samples {
            assert_decodes_equivalently(&compress_raw(&sample));
        }
        let oversized = serde_json::to_vec(&"x".repeat(MAX_EXPLANATION_BYTES)).unwrap();
        assert_decodes_equivalently(&compress_raw(&oversized));
        for corrupt in [&[][..], &[2, 0], &[1, 0, 1, 2, 3]] {
            assert_decodes_equivalently(corrupt);
        }
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
