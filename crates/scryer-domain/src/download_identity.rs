use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use uuid::Uuid;

const WIRE_PREFIX: &str = "scryer-download:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DownloadId(Uuid);

impl DownloadId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::parse_hyphenated(value.trim())
    }

    pub fn as_str(&self) -> String {
        self.0.to_string()
    }

    pub fn to_wire(&self) -> String {
        format!("{WIRE_PREFIX}{self}")
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        let value = value.trim().strip_prefix(WIRE_PREFIX)?;
        Self::parse_hyphenated(value)
    }

    fn parse_hyphenated(value: &str) -> Option<Self> {
        let is_hyphenated_uuid = value.len() == 36
            && value.bytes().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            });

        is_hyphenated_uuid
            .then_some(Uuid::parse_str(value).ok()?)
            .map(Self)
    }
}

impl fmt::Display for DownloadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.as_str())
    }
}

impl Serialize for DownloadId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_str())
    }
}

impl<'de> Deserialize<'de> for DownloadId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value)
            .ok_or_else(|| serde::de::Error::custom("expected a hyphenated UUID download ID"))
    }
}

#[cfg(test)]
mod tests {
    use super::DownloadId;

    const UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

    #[test]
    fn parse_accepts_hyphenated_uuid_in_any_case_and_canonicalizes() {
        let parsed = DownloadId::parse("  550E8400-E29B-41D4-A716-446655440000  ").unwrap();

        assert_eq!(parsed.as_str(), UUID);
        assert_eq!(parsed.to_string(), UUID);
        assert_eq!(DownloadId::parse(&parsed.to_string()), Some(parsed));
    }

    #[test]
    fn parse_rejects_non_download_id_forms() {
        let info_hash_40 = "a".repeat(40);
        let info_hash_64 = "b".repeat(64);
        let trailing_junk = format!("{UUID}x");
        let prefixed = format!("scryer-download:{UUID}");

        for value in [
            info_hash_40.as_str(),
            info_hash_64.as_str(),
            "SABnzbd_nzo_abc",
            "10010",
            prefixed.as_str(),
            "",
            " \t\n ",
            trailing_junk.as_str(),
        ] {
            assert_eq!(
                DownloadId::parse(value),
                None,
                "unexpectedly accepted {value:?}"
            );
        }
    }

    #[test]
    fn wire_form_round_trips_and_rejects_other_forms() {
        let download_id = DownloadId::parse(UUID).unwrap();
        let wire = format!("scryer-download:{UUID}");

        assert_eq!(download_id.to_wire(), wire);
        assert_eq!(DownloadId::from_wire(&wire), Some(download_id));
        assert_eq!(
            DownloadId::from_wire(&format!(" \t{wire}\n")),
            Some(download_id)
        );
        assert_eq!(DownloadId::from_wire(UUID), None);
        assert_eq!(
            DownloadId::from_wire(&format!("other-download:{UUID}")),
            None
        );
        assert_eq!(DownloadId::from_wire("scryer-download:not-a-uuid"), None);
    }

    #[test]
    fn serde_round_trips_as_a_plain_uuid_and_rejects_info_hashes() {
        let download_id = DownloadId::parse(UUID).unwrap();
        let serialized = serde_json::to_string(&download_id).unwrap();

        assert_eq!(serialized, format!("\"{UUID}\""));
        assert_eq!(
            serde_json::from_str::<DownloadId>(&serialized).unwrap(),
            download_id
        );

        let info_hash = serde_json::to_string(&"a".repeat(40)).unwrap();
        assert!(serde_json::from_str::<DownloadId>(&info_hash).is_err());
    }
}
