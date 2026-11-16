//! Checking JSON text the way a parse into a [`serde_json::Value`] checks it,
//! without building the value.
//!
//! A writer that already holds a value, or text it wrote from one, can learn
//! whether a value parse of that text would succeed — and so skip the parse —
//! with no allocation for strings, arrays or objects.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// What a value parse makes of JSON text that it accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueParseShape {
    /// A value parse reads every object, array, string and number as it is
    /// written.
    Plain,
    /// Some object's first key is serde_json's raw-value token, which a value
    /// parse expands in place (and may reject). Only a full parse can say what
    /// such text becomes.
    ExpandsRawValue,
}

/// Walks `json` with the tokenizer and the string, number, nesting-depth and
/// trailing-data checks a `serde_json::from_slice::<Value>` call runs, and
/// fails exactly where that call fails — except for text it reports as
/// [`ValueParseShape::ExpandsRawValue`], which needs that call to decide.
pub fn check_value_parse(json: &[u8]) -> Result<ValueParseShape, serde_json::Error> {
    serde_json::from_slice::<Walked>(json).map(|walked| {
        if walked.expands_raw_value {
            ValueParseShape::ExpandsRawValue
        } else {
            ValueParseShape::Plain
        }
    })
}

/// serde_json's private marker for a raw value. A value parse treats an
/// object whose first key is this string as an embedded raw value when the
/// `raw_value` feature is enabled anywhere in the build.
const RAW_VALUE_TOKEN: &str = "$serde_json::private::RawValue";

struct Walked {
    expands_raw_value: bool,
}

impl Walked {
    const PLAIN: Self = Self {
        expands_raw_value: false,
    };
}

impl<'de> Deserialize<'de> for Walked {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(WalkVisitor)
    }
}

/// An object key, read as a value parse reads it.
struct WalkedKey {
    is_raw_value_token: bool,
}

impl<'de> Deserialize<'de> for WalkedKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl Visitor<'_> for KeyVisitor {
            type Value = WalkedKey;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string key")
            }
            fn visit_str<E: de::Error>(self, key: &str) -> Result<WalkedKey, E> {
                Ok(WalkedKey {
                    is_raw_value_token: key == RAW_VALUE_TOKEN,
                })
            }
        }
        deserializer.deserialize_str(KeyVisitor)
    }
}

struct WalkVisitor;

impl<'de> Visitor<'de> for WalkVisitor {
    type Value = Walked;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<Walked, E> {
        Ok(Walked::PLAIN)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Walked, E> {
        Ok(Walked::PLAIN)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Walked, E> {
        Ok(Walked::PLAIN)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Walked, E> {
        Ok(Walked::PLAIN)
    }

    fn visit_str<E>(self, _: &str) -> Result<Walked, E> {
        Ok(Walked::PLAIN)
    }

    fn visit_unit<E>(self) -> Result<Walked, E> {
        Ok(Walked::PLAIN)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Walked, A::Error> {
        let mut expands_raw_value = false;
        while let Some(element) = seq.next_element::<Walked>()? {
            expands_raw_value |= element.expands_raw_value;
        }
        Ok(Walked { expands_raw_value })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Walked, A::Error> {
        let mut expands_raw_value = false;
        let mut first = true;
        while let Some(key) = map.next_key::<WalkedKey>()? {
            expands_raw_value |= first && key.is_raw_value_token;
            first = false;
            expands_raw_value |= map.next_value::<Walked>()?.expands_raw_value;
        }
        Ok(Walked { expands_raw_value })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn accepts_and_rejects_exactly_what_a_value_parse_does() {
        let deep_ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
        let too_deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        let deep_objects = format!("{}1{}", r#"{"k":"#.repeat(127), "}".repeat(127));
        let too_deep_objects = format!("{}1{}", r#"{"k":"#.repeat(129), "}".repeat(129));
        let samples: Vec<Vec<u8>> = vec![
            br#"{"a":1,"b":[true,false,null],"c":{"d":"e"}}"#.to_vec(),
            br#"{"a": 1 , "b" : [1, 2]}"#.to_vec(),
            br#"{"a":1,"a":2}"#.to_vec(),
            br#"[1e400]"#.to_vec(),
            br#"[-1e400]"#.to_vec(),
            br#"[1e-400]"#.to_vec(),
            br#"["\ud800"]"#.to_vec(),
            br#"["\udc00"]"#.to_vec(),
            [&br#"[""#[..], br"\", b"ud83d", br"\", b"ude00", br#""]"#].concat(),
            br#"["\x"]"#.to_vec(),
            br#"["\u12"]"#.to_vec(),
            b"[\"tab\tinside\"]".to_vec(),
            br#"{"a":1} trailing"#.to_vec(),
            br#"{"a":1}  "#.to_vec(),
            br#"{"a":1,}"#.to_vec(),
            br#"[1,]"#.to_vec(),
            br#"{1:2}"#.to_vec(),
            br#"18446744073709551616"#.to_vec(),
            br#"-9223372036854775809"#.to_vec(),
            br#"01"#.to_vec(),
            br#"1E2"#.to_vec(),
            br#"-0"#.to_vec(),
            b"[\"\xff\"]".to_vec(),
            b"[\"\xc3\xa9\"]".to_vec(),
            b"\xef\xbb\xbf{}".to_vec(),
            b"".to_vec(),
            b"   ".to_vec(),
            b"not-json".to_vec(),
            b"nul".to_vec(),
            deep_ok.into_bytes(),
            too_deep.into_bytes(),
            deep_objects.into_bytes(),
            too_deep_objects.into_bytes(),
        ];
        for sample in samples {
            let value = serde_json::from_slice::<Value>(&sample);
            let walked = check_value_parse(&sample);
            match (&value, &walked) {
                (Ok(_), Ok(ValueParseShape::Plain)) => {}
                (Err(value), Err(walked)) => {
                    assert_eq!(value.to_string(), walked.to_string(), "{sample:?}")
                }
                _ => panic!("{sample:?}: value parse {value:?}, walk {walked:?}"),
            }
        }
    }

    #[test]
    fn raw_value_token_objects_are_left_to_a_full_parse() {
        for sample in [
            &br#"{"$serde_json::private::RawValue":"{\"x\": 1}"}"#[..],
            br#"{"$serde_json::private::RawValue":7}"#,
            br#"{"a":{"$serde_json::private::RawValue":"[1, 2]"}}"#,
            br#"[{"$serde_json::private::RawValue":"{}"}]"#,
        ] {
            assert_eq!(
                check_value_parse(sample).unwrap(),
                ValueParseShape::ExpandsRawValue
            );
        }
        // Only the first key of an object is classified.
        assert_eq!(
            check_value_parse(br#"{"a":1,"$serde_json::private::RawValue":"[1]"}"#).unwrap(),
            ValueParseShape::Plain
        );
    }
}
