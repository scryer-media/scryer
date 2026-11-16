use std::fmt;
use std::sync::LazyLock;

use serde_json::Value;
use zstd::dict::DecoderDictionary;

use crate::json_shape::{ValueParseShape, check_value_parse};

pub const DOMAIN_EVENT_PAYLOAD_FORMAT_V1: u8 = 1;
pub const DOMAIN_EVENT_PAYLOAD_MAX_BYTES: usize = 256 * 1024;
const COMPRESSION_LEVEL: i32 = 3;
const DICTIONARY_V1: &[u8] = include_bytes!("domain_event_payload_v1.dict");

// The decoder dictionary is digested once per process. The encoder still
// loads the raw dictionary per payload: a prepared encoder dictionary
// compresses large payloads to different bytes.
static DECODER_DICTIONARY_V1: LazyLock<Result<DecoderDictionary<'static>, String>> =
    LazyLock::new(|| DecoderDictionary::try_copy(DICTIONARY_V1).map_err(|error| error.to_string()));

#[derive(Debug)]
pub struct DomainEventPayloadCodecError(String);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DomainEventProjections {
    pub import_status: Option<String>,
    /// An import rejection's skip reason, so history can tell a rule's
    /// refusal from a failure or a plain skip without decoding the payload.
    pub import_skip_reason: Option<String>,
    pub media_file_delete_reason: Option<String>,
    pub download_id: Option<String>,
    /// The request a submission event is about, so one title's history can
    /// read the submissions of its own requests by key instead of decoding
    /// every submission on the instance.
    pub media_request_id: Option<String>,
}

impl fmt::Display for DomainEventPayloadCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DomainEventPayloadCodecError {}

pub fn derive_domain_event_projections(
    event_type: &str,
    payload: &Value,
) -> DomainEventProjections {
    let data = payload.get("data");
    let field = |name: &str| {
        data.and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    DomainEventProjections {
        import_status: (event_type == "import_rejected")
            .then(|| field("status"))
            .flatten(),
        import_skip_reason: (event_type == "import_rejected")
            .then(|| field("skip_reason"))
            .flatten(),
        media_file_delete_reason: (event_type == "media_file_deleted")
            .then(|| field("reason"))
            .flatten(),
        download_id: matches!(
            event_type,
            "release_grabbed" | "download_failed" | "release_blocklisted"
        )
        .then(|| field("download_id"))
        .flatten(),
        media_request_id: (event_type == "media_request_submitted")
            .then(|| field("request_id"))
            .flatten(),
    }
}

pub fn encode_domain_event_payload(
    payload: &Value,
) -> Result<Vec<u8>, DomainEventPayloadCodecError> {
    DomainEventPayloadEncoder::new().encode(payload)
}

/// Encodes a payload and also returns the JSON its stored bytes decode to, so
/// a writer can describe the event it just stored without decompressing what
/// it compressed. The second value is exactly what
/// [`decode_domain_event_payload`] returns for the first: compression is
/// lossless, so both parse the same compact bytes.
pub fn encode_domain_event_payload_with_stored_json(
    payload: &Value,
) -> Result<(Vec<u8>, Value), DomainEventPayloadCodecError> {
    let compact = compact_payload(payload)?;
    let encoded = DomainEventPayloadEncoder::new().compress_compact(&compact)?;
    Ok((encoded, parse_decoded_payload(&compact)?))
}

/// Encodes a run of payloads through one zstd context, so the raw dictionary
/// is loaded once for the run instead of once per payload. Each payload
/// encodes to exactly the bytes [`encode_domain_event_payload`] produces for
/// it alone: a zstd frame starts from the dictionary, never from the frame
/// before it.
#[derive(Default)]
pub struct DomainEventPayloadEncoder {
    compressor: Option<zstd::bulk::Compressor<'static>>,
}

impl DomainEventPayloadEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn encode(&mut self, payload: &Value) -> Result<Vec<u8>, DomainEventPayloadCodecError> {
        self.compress_compact(&compact_payload(payload)?)
    }

    /// [`encode_domain_event_payload_with_stored_json`], taking the payload
    /// by value. The compact bytes are serialized once. When a parse of those
    /// bytes reads them back as the payload they were written from — every
    /// object, array, string and number as written — the payload itself is
    /// the stored JSON and the bytes are only checked, not parsed into a
    /// second value; otherwise they are parsed as before.
    pub fn encode_with_stored_json(
        &mut self,
        payload: Value,
    ) -> Result<(Vec<u8>, Value), DomainEventPayloadCodecError> {
        let compact = compact_payload(&payload)?;
        let encoded = self.compress_compact(&compact)?;
        let stored = match check_value_parse(&compact).map_err(invalid_decoded_payload)? {
            ValueParseShape::Plain => payload,
            ValueParseShape::ExpandsRawValue => parse_decoded_payload(&compact)?,
        };
        Ok((encoded, stored))
    }

    fn compress_compact(
        &mut self,
        compact: &[u8],
    ) -> Result<Vec<u8>, DomainEventPayloadCodecError> {
        let compressor = match &mut self.compressor {
            Some(compressor) => compressor,
            None => self.compressor.insert(
                zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
                    .map_err(|error| codec_error(format!("failed to initialize zstd: {error}")))?,
            ),
        };
        let compressed = match compressor.compress(compact) {
            Ok(compressed) => compressed,
            Err(error) => {
                // Start the next payload from a fresh context rather than
                // trust one that just failed.
                self.compressor = None;
                return Err(codec_error(format!(
                    "failed to compress domain event JSON: {error}"
                )));
            }
        };
        let mut encoded = Vec::with_capacity(compressed.len() + 1);
        encoded.push(DOMAIN_EVENT_PAYLOAD_FORMAT_V1);
        encoded.extend_from_slice(&compressed);
        Ok(encoded)
    }
}

fn compact_payload(payload: &Value) -> Result<Vec<u8>, DomainEventPayloadCodecError> {
    let compact = serde_json::to_vec(payload)
        .map_err(|error| codec_error(format!("failed to compact domain event JSON: {error}")))?;
    if compact.len() > DOMAIN_EVENT_PAYLOAD_MAX_BYTES {
        return Err(codec_error(format!(
            "domain event payload is {} bytes, exceeding the {}-byte limit",
            compact.len(),
            DOMAIN_EVENT_PAYLOAD_MAX_BYTES
        )));
    }
    Ok(compact)
}

fn parse_decoded_payload(decoded: &[u8]) -> Result<Value, DomainEventPayloadCodecError> {
    serde_json::from_slice(decoded).map_err(invalid_decoded_payload)
}

fn invalid_decoded_payload(error: serde_json::Error) -> DomainEventPayloadCodecError {
    codec_error(format!(
        "decoded domain event payload is invalid JSON: {error}"
    ))
}

pub fn decode_domain_event_payload(encoded: &[u8]) -> Result<Value, DomainEventPayloadCodecError> {
    let Some((&format, compressed)) = encoded.split_first() else {
        return Err(codec_error("domain event payload is empty"));
    };
    if format != DOMAIN_EVENT_PAYLOAD_FORMAT_V1 {
        return Err(codec_error(format!(
            "unsupported domain event payload format {format}"
        )));
    }
    let dictionary = (*DECODER_DICTIONARY_V1)
        .as_ref()
        .map_err(|error| codec_error(format!("failed to initialize zstd: {error}")))?;
    let mut decompressor = zstd::bulk::Decompressor::with_prepared_dictionary(dictionary)
        .map_err(|error| codec_error(format!("failed to initialize zstd: {error}")))?;
    let decoded = decompressor
        .decompress(compressed, DOMAIN_EVENT_PAYLOAD_MAX_BYTES + 1)
        .map_err(|error| codec_error(format!("failed to decompress domain event JSON: {error}")))?;
    if decoded.len() > DOMAIN_EVENT_PAYLOAD_MAX_BYTES {
        return Err(codec_error(format!(
            "domain event payload expands beyond the {}-byte limit",
            DOMAIN_EVENT_PAYLOAD_MAX_BYTES
        )));
    }
    parse_decoded_payload(&decoded)
}

pub fn compact_legacy_domain_event_json(
    legacy: &[u8],
) -> Result<Value, DomainEventPayloadCodecError> {
    serde_json::from_slice(legacy).map_err(|error| {
        codec_error(format!(
            "legacy domain event payload is invalid JSON: {error}"
        ))
    })
}

fn codec_error(message: impl Into<String>) -> DomainEventPayloadCodecError {
    DomainEventPayloadCodecError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    mod dictionary_training {
        use crate as scryer_infrastructure_sql;

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/support/domain_event_dictionary_training.rs"
        ));
    }

    #[test]
    fn dictionary_identity_is_pinned() {
        assert_eq!(DICTIONARY_V1.len(), 8 * 1024);
        assert_eq!(
            blake3::hash(DICTIONARY_V1).to_hex().as_str(),
            "9ea5863fd17f94ff47c512f32c113bf15df343621d61d1ab88acb99a6e57e48d"
        );
    }

    #[test]
    fn dictionary_regenerates_exactly_and_improves_held_out_compression() {
        let trained = dictionary_training::train_dictionary().unwrap();
        assert_eq!(trained.bytes, DICTIONARY_V1);
        assert_eq!(trained.training_samples, 4_152);
        assert_eq!(trained.held_out.len(), 14);
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
    fn projection_derivation_covers_queryable_event_fields() {
        let import = derive_domain_event_projections(
            "import_rejected",
            &json!({"data": {"status": "failed"}}),
        );
        assert_eq!(import.import_status.as_deref(), Some("failed"));
        assert_eq!(import.import_skip_reason, None);

        let rule_rejection = derive_domain_event_projections(
            "import_rejected",
            &json!({"data": {"status": "failed", "skip_reason": "post_download_rule_blocked"}}),
        );
        assert_eq!(
            rule_rejection.import_skip_reason.as_deref(),
            Some("post_download_rule_blocked")
        );
        assert_eq!(
            derive_domain_event_projections(
                "import_completed",
                &json!({"data": {"skip_reason": "post_download_rule_blocked"}}),
            )
            .import_skip_reason,
            None
        );

        let submission = derive_domain_event_projections(
            "media_request_submitted",
            &json!({"data": {"request_id": "request-emberfall"}}),
        );
        assert_eq!(
            submission.media_request_id.as_deref(),
            Some("request-emberfall")
        );
        assert_eq!(
            derive_domain_event_projections(
                "media_request_approved",
                &json!({"data": {"request_id": "request-emberfall"}}),
            )
            .media_request_id,
            None
        );

        let deletion = derive_domain_event_projections(
            "media_file_deleted",
            &json!({"data": {"reason": "upgrade_cleanup"}}),
        );
        assert_eq!(
            deletion.media_file_delete_reason.as_deref(),
            Some("upgrade_cleanup")
        );

        for event_type in ["release_grabbed", "download_failed", "release_blocklisted"] {
            let projection = derive_domain_event_projections(
                event_type,
                &json!({"data": {"download_id": "download-1"}}),
            );
            assert_eq!(projection.download_id.as_deref(), Some("download-1"));
        }
    }

    #[test]
    fn representative_payload_round_trips() {
        let payload = json!({
            "type": "import_completed",
            "data": {
                "title_id": "<title-id:medium>",
                "download_id": "<download-id:long>",
                "status": "completed",
                "paths": ["<absolute-path:long>"]
            }
        });
        let encoded = encode_domain_event_payload(&payload).expect("payload should encode");
        assert_eq!(encoded[0], DOMAIN_EVENT_PAYLOAD_FORMAT_V1);
        assert_eq!(decode_domain_event_payload(&encoded).unwrap(), payload);
        let compact = serde_json::to_vec(&payload).unwrap();
        let without_dictionary = zstd::bulk::compress(&compact, COMPRESSION_LEVEL).unwrap();
        assert!(
            encoded.len() - 1 < without_dictionary.len(),
            "dictionary compression should beat plain level-3 zstd"
        );
    }

    /// The encoding stored rows were written with: the raw dictionary loaded
    /// into a fresh context for every payload.
    fn encode_with_per_call_dictionary(payload: &Value) -> Vec<u8> {
        let compact = serde_json::to_vec(payload).unwrap();
        let mut compressor =
            zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1).unwrap();
        let mut encoded = vec![DOMAIN_EVENT_PAYLOAD_FORMAT_V1];
        encoded.extend_from_slice(&compressor.compress(&compact).unwrap());
        encoded
    }

    /// Payloads from a few bytes up to the size limit, compressible and not.
    fn codec_sample_payloads() -> Vec<Value> {
        let mut noise = 0x9e37_79b9_7f4a_7c15_u64;
        let mut incompressible = |len: usize| -> String {
            (0..len)
                .map(|_| {
                    noise ^= noise << 13;
                    noise ^= noise >> 7;
                    noise ^= noise << 17;
                    char::from(b'a' + (noise % 26) as u8)
                })
                .collect()
        };
        let mut payloads = vec![
            json!(null),
            json!({}),
            json!({"type": "import_completed", "data": {"title_id": "title-emberfall"}}),
            json!({
                "type": "download_queue_item_updated",
                "data": {"ratio": 0.5, "seed_ratio": 2.25, "count": u64::MAX,
                         "negative": i64::MIN, "paths": ["/library/a", "/library/b"]}
            }),
        ];
        for len in [1usize, 100, 4_000, 20_000, 130_000, 250_000] {
            payloads.push(json!({"type": "import_rejected", "data": {"detail": "x".repeat(len)}}));
            payloads
                .push(json!({"type": "import_rejected", "data": {"detail": incompressible(len)}}));
        }
        payloads
    }

    #[test]
    fn encoded_bytes_match_a_fresh_per_call_dictionary() {
        for payload in codec_sample_payloads() {
            let encoded = encode_domain_event_payload(&payload).expect("payload should encode");
            assert_eq!(
                blake3::hash(&encoded),
                blake3::hash(&encode_with_per_call_dictionary(&payload)),
                "encoded bytes changed for a {}-byte payload",
                serde_json::to_vec(&payload).unwrap().len()
            );
            assert_eq!(decode_domain_event_payload(&encoded).unwrap(), payload);
        }
    }

    #[test]
    fn stored_json_is_what_decoding_the_encoded_bytes_returns() {
        for payload in codec_sample_payloads() {
            let (encoded, stored) = encode_domain_event_payload_with_stored_json(&payload)
                .expect("payload should encode");
            assert_eq!(encoded, encode_domain_event_payload(&payload).unwrap());
            assert_eq!(stored, decode_domain_event_payload(&encoded).unwrap());
        }
        assert!(
            encode_domain_event_payload_with_stored_json(&Value::String(
                "x".repeat(DOMAIN_EVENT_PAYLOAD_MAX_BYTES + 1)
            ))
            .is_err()
        );
    }

    /// Payloads whose numbers, strings and nesting stress what a parse of the
    /// compact bytes reads back.
    fn round_trip_edge_payloads() -> Vec<Value> {
        let deep = (0..120).fold(json!("leaf"), |inner, _| json!({"k": inner}));
        let too_deep = (0..140).fold(json!("leaf"), |inner, _| json!([inner]));
        vec![
            json!({"data": {"f32": f64::from(0.1_f32), "neg_zero": -0.0, "one": 1.0,
                            "tiny": 5e-324, "max": f64::MAX, "int_like": 1e21,
                            "u": u64::MAX, "i": i64::MIN, "zero": 0}}),
            json!({"data": {"text": "quote \" backslash \\ ctl \u{1} \u{7f} é 😀 /"}}),
            json!({"z": 1, "a": {"y": [], "b": {}}, "m": [null, true, false]}),
            json!({"$serde_json::private::RawValue": "{\"x\": 1}"}),
            json!({"data": {"$serde_json::private::RawValue": "[1, 2]"}}),
            json!({"$serde_json::private::RawValue": 7}),
            deep,
            too_deep,
        ]
    }

    #[test]
    fn a_reused_encoder_matches_per_payload_encoding() {
        let mut payloads = codec_sample_payloads();
        payloads.extend(round_trip_edge_payloads());
        payloads.push(Value::String(
            "x".repeat(DOMAIN_EVENT_PAYLOAD_MAX_BYTES + 1),
        ));
        let mut encoder = DomainEventPayloadEncoder::new();
        // Twice over, so every payload is also encoded after a larger and a
        // smaller one through the same context.
        for payload in payloads.iter().chain(payloads.iter().rev()) {
            let reused = encoder.encode(payload);
            let single = encode_domain_event_payload(payload);
            match (&reused, &single) {
                (Ok(reused), Ok(single)) => {
                    assert_eq!(reused, single);
                    assert_eq!(*reused, encode_with_per_call_dictionary(payload));
                }
                (Err(reused), Err(single)) => assert_eq!(reused.to_string(), single.to_string()),
                _ => panic!("reused and single encodes disagree"),
            }

            let owned = encoder.encode_with_stored_json(payload.clone());
            let before = encode_domain_event_payload_with_stored_json(payload);
            match (owned, before) {
                (Ok((owned_bytes, owned_json)), Ok((bytes, json))) => {
                    assert_eq!(owned_bytes, bytes);
                    assert_eq!(owned_json, json);
                    assert_eq!(
                        serde_json::to_vec(&owned_json).unwrap(),
                        serde_json::to_vec(&json).unwrap()
                    );
                }
                (Err(owned), Err(before)) => assert_eq!(owned.to_string(), before.to_string()),
                (owned, before) => panic!("stored JSON disagrees: {owned:?} / {before:?}"),
            }
        }
    }

    #[test]
    fn rejects_unknown_corrupt_and_oversized_payloads() {
        assert!(decode_domain_event_payload(&[]).is_err());
        assert!(decode_domain_event_payload(&[99, 1, 2, 3]).is_err());
        assert!(decode_domain_event_payload(&[DOMAIN_EVENT_PAYLOAD_FORMAT_V1, 1, 2, 3]).is_err());
        assert!(
            encode_domain_event_payload(&Value::String(
                "x".repeat(DOMAIN_EVENT_PAYLOAD_MAX_BYTES + 1)
            ))
            .is_err()
        );

        let encode_raw = |bytes: &[u8]| {
            let mut compressor =
                zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1).unwrap();
            let mut encoded = vec![DOMAIN_EVENT_PAYLOAD_FORMAT_V1];
            encoded.extend_from_slice(&compressor.compress(bytes).unwrap());
            encoded
        };
        assert!(decode_domain_event_payload(&encode_raw(b"not json")).is_err());
        assert!(
            decode_domain_event_payload(&encode_raw(&vec![
                b'x';
                DOMAIN_EVENT_PAYLOAD_MAX_BYTES + 1
            ]))
            .is_err()
        );
    }
}
