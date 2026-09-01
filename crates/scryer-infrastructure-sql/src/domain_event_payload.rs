use std::fmt;

use serde_json::Value;

pub const DOMAIN_EVENT_PAYLOAD_FORMAT_V1: u8 = 1;
pub const DOMAIN_EVENT_PAYLOAD_MAX_BYTES: usize = 256 * 1024;
const COMPRESSION_LEVEL: i32 = 3;
const DICTIONARY_V1: &[u8] = include_bytes!("domain_event_payload_v1.dict");

#[derive(Debug)]
pub struct DomainEventPayloadCodecError(String);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DomainEventProjections {
    pub import_status: Option<String>,
    pub media_file_delete_reason: Option<String>,
    pub download_id: Option<String>,
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
        media_file_delete_reason: (event_type == "media_file_deleted")
            .then(|| field("reason"))
            .flatten(),
        download_id: matches!(
            event_type,
            "release_grabbed" | "download_failed" | "release_blocklisted"
        )
        .then(|| field("download_id"))
        .flatten(),
    }
}

pub fn encode_domain_event_payload(
    payload: &Value,
) -> Result<Vec<u8>, DomainEventPayloadCodecError> {
    let compact = serde_json::to_vec(payload)
        .map_err(|error| codec_error(format!("failed to compact domain event JSON: {error}")))?;
    if compact.len() > DOMAIN_EVENT_PAYLOAD_MAX_BYTES {
        return Err(codec_error(format!(
            "domain event payload is {} bytes, exceeding the {}-byte limit",
            compact.len(),
            DOMAIN_EVENT_PAYLOAD_MAX_BYTES
        )));
    }
    let mut compressor = zstd::bulk::Compressor::with_dictionary(COMPRESSION_LEVEL, DICTIONARY_V1)
        .map_err(|error| codec_error(format!("failed to initialize zstd: {error}")))?;
    let compressed = compressor
        .compress(&compact)
        .map_err(|error| codec_error(format!("failed to compress domain event JSON: {error}")))?;
    let mut encoded = Vec::with_capacity(compressed.len() + 1);
    encoded.push(DOMAIN_EVENT_PAYLOAD_FORMAT_V1);
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
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
    let mut decompressor = zstd::bulk::Decompressor::with_dictionary(DICTIONARY_V1)
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
    serde_json::from_slice(&decoded).map_err(|error| {
        codec_error(format!(
            "decoded domain event payload is invalid JSON: {error}"
        ))
    })
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
